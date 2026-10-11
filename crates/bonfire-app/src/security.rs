//! 仓库加密的应用层（M5-2）：解锁、加密 / 取消加密整个仓库、修改密码、在本机记住密码。
//!
//! - 加密仓库的索引数据库（含备注、路径）不放在仓库里，放在本机缓存目录（可随时从仓库重建）；
//!   `games.toml` 加密保存。截图不加密（相册直接显示图片文件）。
//! - 记住密码：数据密钥用 Windows 数据保护（DPAPI，与当前 Windows 账户绑定）加密后存在数据目录的 `keys/` 中，
//!   之后启动自动解锁；其他账户或其他电脑无法解开。命令行也可以用环境变量 `BONFIRE_PASSWORD` 提供密码。
//! - 忘记密码时任何备份都无法恢复。

use std::path::PathBuf;

use bonfire_core::crypt;
use bonfire_core::{Index, Repo, RepoLock};
use serde::Serialize;

use crate::app::App;
use crate::error::{AppError, Result};

pub const ENV_PASSWORD: &str = "BONFIRE_PASSWORD";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityStatus {
    pub encrypted: bool,
    pub unlocked: bool,
    /// 本机记住了密码。
    pub remembered: bool,
    /// 本机能否记住密码（Windows）。
    pub can_remember: bool,
}

/// 加密仓库的索引位置：本机缓存目录中按仓库标识存放。
pub(crate) fn index_path(paths: &crate::AppPaths, repo: &Repo) -> PathBuf {
    match &repo.config().encryption {
        Some(e) => paths
            .cache_dir
            .join("index")
            .join(format!("{}.db", e.repo_id)),
        None => repo.root().join("bonfire.db"),
    }
}

impl App {
    fn key_file(&self) -> Option<PathBuf> {
        let id = &self.repo.config().encryption.as_ref()?.repo_id;
        let data = self.paths.config_file.parent()?;
        Some(data.join("keys").join(format!("{id}.key")))
    }

    pub fn security_status(&self) -> SecurityStatus {
        SecurityStatus {
            encrypted: self.repo.is_encrypted(),
            unlocked: self.repo.is_unlocked(),
            remembered: self.key_file().is_some_and(|p| p.is_file()),
            can_remember: dpapi::available(),
        }
    }

    /// 打开加密仓库时自动解锁：先用环境变量中的密码，再用本机记住的密钥。
    pub(crate) fn auto_unlock(&self) {
        if self.repo.is_unlocked() {
            return;
        }
        if let Ok(pw) = std::env::var(ENV_PASSWORD)
            && !pw.is_empty()
            && self.repo.unlock(&pw).is_ok()
        {
            return;
        }
        if let Some(p) = self.key_file()
            && let Ok(sealed) = std::fs::read(&p)
            && let Some(key) = dpapi::unprotect(&sealed)
            && let Ok(key) = <[u8; 32]>::try_from(key.as_slice())
            && !self.repo.unlock_with_key(key)
        {
            tracing::warn!("本机记住的密钥与仓库不符，需要重新输入密码");
        }
    }

    /// 用密码解锁；`remember` 时在本机记住。解锁后补做打开仓库时跳过的步骤（恢复游戏列表、重建索引等）。
    pub fn unlock(&mut self, password: &str, remember: bool) -> Result<()> {
        if !self.repo.is_encrypted() {
            return Ok(());
        }
        self.repo.unlock(password)?;
        if remember {
            self.remember_key()?;
        }
        self.after_unlock()
    }

    /// 在本机记住当前的数据密钥（仓库已解锁时）。
    pub fn remember_key(&self) -> Result<()> {
        let (Some(path), Some(c)) = (self.key_file(), self.repo.cipher()) else {
            return Ok(());
        };
        let sealed = dpapi::protect(&c.key())
            .ok_or_else(|| AppError::Invalid("这台电脑不能记住密码".into()))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, e))?;
        }
        std::fs::write(&path, sealed).map_err(|e| AppError::io(&path, e))
    }

    /// 不再在本机记住密码（下次启动需要输入）。
    pub fn forget_password(&self) -> Result<()> {
        if let Some(p) = self.key_file() {
            match std::fs::remove_file(&p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(AppError::io(&p, e)),
            }
        }
        Ok(())
    }

    /// 加密整个仓库。索引移到本机缓存目录，仓库中的旧索引删除；`games.toml` 改为加密保存。
    pub fn encrypt_repo(&mut self, lock: &RepoLock, password: &str, remember: bool) -> Result<()> {
        check_password(password)?;
        let old_index = self.repo.root().join("bonfire.db");
        let repo = crypt::encrypt_repo(&self.repo, lock, password)?;
        self.switch_repo(repo, Some(old_index))?;
        if remember {
            self.remember_key()?;
        }
        self.save_games_file()?;
        tracing::info!("仓库已加密");
        Ok(())
    }

    /// 取消加密：数据改回明文，索引移回仓库，删除本机记住的密钥。
    pub fn decrypt_repo(&mut self, lock: &RepoLock, password: &str) -> Result<()> {
        self.repo.unlock(password)?;
        let old_index = index_path(&self.paths, &self.repo);
        let key = self.key_file();
        let repo = crypt::decrypt_repo(&self.repo, lock)?;
        self.switch_repo(repo, Some(old_index))?;
        self.save_games_file()?;
        if let Some(k) = key {
            let _ = std::fs::remove_file(k);
        }
        tracing::info!("仓库已取消加密");
        Ok(())
    }

    /// 修改密码；本机记住的密钥不变（数据密钥没变）。
    pub fn change_password(&mut self, lock: &RepoLock, old: &str, new: &str) -> Result<()> {
        check_password(new)?;
        let repo = crypt::change_password(&self.repo, lock, old, new)?;
        self.repo = repo;
        Ok(())
    }

    /// 换成重新打开的仓库，把索引数据库从 `old_index` 移到新位置。
    fn switch_repo(&mut self, repo: Repo, old_index: Option<PathBuf>) -> Result<()> {
        let new_index = index_path(&self.paths, &repo);
        // 先关闭当前的索引连接，才能移动数据库文件。
        self.index = Index::open(std::path::Path::new(":memory:"))?;
        if let Some(old) = old_index
            && old != new_index
            && old.is_file()
        {
            if let Some(dir) = new_index.parent() {
                std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, e))?;
            }
            for ext in ["", "-wal", "-shm"] {
                let from = PathBuf::from(format!("{}{ext}", old.display()));
                let to = PathBuf::from(format!("{}{ext}", new_index.display()));
                if from.is_file() {
                    std::fs::copy(&from, &to).map_err(|e| AppError::io(&from, e))?;
                    std::fs::remove_file(&from).map_err(|e| AppError::io(&from, e))?;
                }
            }
        }
        self.repo = repo;
        self.index = Index::open(&new_index)?;
        Ok(())
    }
}

fn check_password(p: &str) -> Result<()> {
    if p.chars().count() < 6 {
        return Err(AppError::Invalid("密码至少 6 个字符".into()));
    }
    Ok(())
}

/// Windows 数据保护（DPAPI）：与当前 Windows 账户绑定的加密。
mod dpapi {
    #[cfg(windows)]
    pub fn available() -> bool {
        true
    }

    #[cfg(windows)]
    fn call(data: &[u8], protect: bool) -> Option<Vec<u8>> {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        };
        let input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        // SAFETY: 输入缓冲区在调用期间有效；输出由系统分配，复制后用 LocalFree 释放。
        let ok = unsafe {
            if protect {
                CryptProtectData(
                    &input,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            } else {
                CryptUnprotectData(
                    &input,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                )
            }
        };
        if ok == 0 || out.pbData.is_null() {
            return None;
        }
        // SAFETY: 系统返回的缓冲区长度为 cbData。
        let v = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
        unsafe {
            LocalFree(out.pbData.cast());
        }
        Some(v)
    }

    #[cfg(windows)]
    pub fn protect(data: &[u8]) -> Option<Vec<u8>> {
        call(data, true)
    }

    #[cfg(windows)]
    pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
        call(data, false)
    }

    #[cfg(not(windows))]
    pub fn available() -> bool {
        false
    }

    #[cfg(not(windows))]
    pub fn protect(_: &[u8]) -> Option<Vec<u8>> {
        None
    }

    #[cfg(not(windows))]
    pub fn unprotect(_: &[u8]) -> Option<Vec<u8>> {
        None
    }

    #[cfg(all(test, windows))]
    mod tests {
        #[test]
        fn roundtrip() {
            let s = super::protect(b"key").unwrap();
            assert_ne!(s, b"key");
            assert_eq!(super::unprotect(&s).unwrap(), b"key");
            assert!(super::unprotect(b"garbage").is_none());
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::backup::BackupOptions;
    use crate::testutil::Env;

    #[test]
    fn encrypted_repo_backup_restore_lock_and_unlock() {
        bonfire_core::crypt::FAST_KDF.store(true, std::sync::atomic::Ordering::Relaxed);
        let mut env = Env::new();
        env.write("a.sav", b"secret progress");
        let g = env.add_game("Hollow Knight");
        {
            let lock = env.app.repo.lock().unwrap();
            env.app
                .backup(
                    &lock,
                    &g,
                    &BackupOptions {
                        comment: Some("打完螳螂领主".into()),
                        ..Default::default()
                    },
                )
                .unwrap();
            assert!(
                env.app.encrypt_repo(&lock, "123", false).is_err(),
                "密码太短"
            );
            env.app.encrypt_repo(&lock, "correct horse", false).unwrap();
        }
        let root = env.app.repo.root().to_path_buf();
        assert!(!root.join("bonfire.db").exists(), "仓库中不再有明文索引");
        let games = std::fs::read(root.join("games.toml")).unwrap();
        assert!(bonfire_core::crypt::is_sealed(&games), "游戏列表加密保存");
        assert!(env.app.security_status().encrypted && env.app.security_status().unlocked);

        // 加密后继续备份、查看历史。
        env.write("a.sav", b"more progress");
        let lock = env.app.repo.lock().unwrap();
        env.app
            .backup(&lock, &g, &BackupOptions::default())
            .unwrap();
        drop(lock);
        assert_eq!(env.app.history_view(&g).unwrap().nodes.len(), 2);

        // 重新打开：没有记住密码时是锁定状态；输入密码后一切照常。
        env.reopen();
        let s = env.app.security_status();
        assert!(s.encrypted && !s.unlocked && !s.remembered);
        assert!(env.app.unlock("wrong", false).is_err());
        env.app.unlock("correct horse", true).unwrap();
        assert!(env.app.status(&g).is_ok());
        let found = env.app.index.search_comments("螳螂", None).unwrap();
        assert_eq!(found.len(), 1);

        // 记住密码后重新打开自动解锁；忘记后又需要密码。
        env.reopen();
        assert!(env.app.security_status().unlocked, "本机记住的密码自动解锁");
        env.app.forget_password().unwrap();
        env.reopen();
        assert!(!env.app.security_status().unlocked);
        env.app.unlock("correct horse", false).unwrap();

        // 改密码、取消加密。
        let lock = env.app.repo.lock().unwrap();
        env.app
            .change_password(&lock, "correct horse", "battery staple")
            .unwrap();
        assert!(env.app.decrypt_repo(&lock, "correct horse").is_err());
        env.app.decrypt_repo(&lock, "battery staple").unwrap();
        drop(lock);
        assert!(root.join("bonfire.db").exists(), "索引移回仓库");
        env.reopen();
        assert!(!env.app.security_status().encrypted);
        assert_eq!(env.app.history_view(&g).unwrap().nodes.len(), 2);
        assert!(bonfire_core::fsck::fsck(&env.app.repo, true).unwrap().ok());
    }
}
