//! 本机主存档与路线改名（U-41）。
//!
//! - 每台电脑可以为每个游戏指定一条“本机主存档”（存放在仓库的 `refs/<游戏>/primary/<电脑的键>`），
//!   仓库在几台电脑间共用时各自独立；没有设置时以 `main` 为主存档。
//! - 主存档只是“家”：备份仍接在当前路线上，不强行写进主存档；界面标出“不在主存档”，可一键回去。
//! - 任一电脑的主存档都不能直接删除，要先把别的路线设为主存档。
//! - 路线改名只换名字，快照不动；当前分支、各电脑的主存档指向旧名时一并更新。

use bonfire_core::RepoLock;
use bonfire_core::index::GameRecord;
use bonfire_core::refs::{DEFAULT_BRANCH, RefKind, RefName};
use serde::Serialize;

use crate::app::App;
use crate::error::{AppError, Result};

/// 一台电脑的主存档。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrimaryView {
    /// 电脑的键（本机标识）。
    pub key: String,
    pub branch: String,
    /// 电脑名（设置主存档时记下的）；不知道时为空。
    pub label: String,
    /// 是本机。
    pub mine: bool,
}

/// 设为本机主存档的结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPrimaryReport {
    /// 主存档最终的名字（整理名称后为 main）。
    pub branch: String,
    /// 改名：(原名, 新名)。
    pub renamed: Vec<(String, String)>,
}

fn sanitize(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "PC".into() } else { out }
}

impl App {
    /// 本机标识（U-41g）：第一次用到时随机生成，保存在数据目录的 `machine-id`（与 config.toml 同目录）。
    /// 打包迁移数据目录时随之迁移，新电脑继承原来的主存档设置。
    pub fn machine_key(&self) -> Result<String> {
        let path = self.paths.config_file.with_file_name("machine-id");
        if let Ok(t) = std::fs::read_to_string(&path) {
            let k = t.trim();
            if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Ok(k.to_string());
            }
        }
        let seed = format!(
            "{}\0{}\0{}",
            crate::util::host_name(),
            bonfire_core::scan::now_ns(),
            std::process::id()
        );
        let key = format!(
            "m-{}",
            bonfire_core::ObjectId::of(seed.as_bytes()).short(16)
        );
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, e))?;
        }
        std::fs::write(&path, format!("{key}\n")).map_err(|e| AppError::io(&path, e))?;
        Ok(key)
    }

    fn machine_label_path(&self, key: &str) -> std::path::PathBuf {
        self.repo.root().join("machines").join(key)
    }

    /// 电脑名（各电脑设置主存档时写入仓库，供其他电脑显示“某某的主存档”）。
    fn machine_label(&self, key: &str) -> String {
        std::fs::read_to_string(self.machine_label_path(key))
            .map(|t| t.trim().to_string())
            .unwrap_or_default()
    }

    /// 本机主存档：设置过且分支还在时为它；否则为 main（存在时）；再否则为当前分支。
    pub fn primary_branch(&self, game_id: &str) -> Result<String> {
        let exists = |b: &str| -> Result<bool> {
            Ok(self.repo.read_ref(game_id, &RefName::branch(b))?.is_some())
        };
        if let Some(b) = self.repo.primary_branch(game_id, &self.machine_key()?)?
            && exists(&b)?
        {
            return Ok(b);
        }
        if exists(DEFAULT_BRANCH)? {
            return Ok(DEFAULT_BRANCH.to_string());
        }
        Ok(self.repo.head(game_id)?.branch)
    }

    /// 各电脑的主存档（本机在前；本机没有设置时按默认列出）。
    pub fn primaries(&self, game_id: &str) -> Result<Vec<PrimaryView>> {
        let me = self.machine_key()?;
        let mut out: Vec<PrimaryView> = self
            .repo
            .primary_branches(game_id)?
            .into_iter()
            .filter(|(k, _)| !k.eq_ignore_ascii_case(&me))
            .map(|(key, branch)| PrimaryView {
                label: self.machine_label(&key),
                key,
                branch,
                mine: false,
            })
            .collect();
        out.insert(
            0,
            PrimaryView {
                branch: self.primary_branch(game_id)?,
                label: crate::util::host_name(),
                key: me,
                mine: true,
            },
        );
        Ok(out)
    }

    /// 设为本机主存档时可以“整理名称”：选的不是 main 且 main 存在（如迁移后导入的 main-旧电脑）。
    pub fn can_tidy_primary(&self, game_id: &str, branch: &str) -> Result<bool> {
        Ok(branch != DEFAULT_BRANCH
            && self
                .repo
                .read_ref(game_id, &RefName::branch(DEFAULT_BRANCH))?
                .is_some())
    }

    /// 设为本机主存档。`tidy` 时整理名称：原来的 main 改名为 main-本机电脑名，选中的路线改名为 main。
    /// 不改动磁盘上的存档（需要时再恢复它的最新快照）。
    pub fn set_primary(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        branch: &str,
        tidy: bool,
    ) -> Result<SetPrimaryReport> {
        if self
            .repo
            .read_ref(&game.id, &RefName::branch(branch))?
            .is_none()
        {
            return Err(AppError::Invalid(format!("找不到分支「{branch}」")));
        }
        let mut renamed = Vec::new();
        let mut target = branch.to_string();
        if tidy && self.can_tidy_primary(&game.id, branch)? {
            let taken: Vec<String> = self
                .repo
                .list_refs(&game.id)?
                .into_iter()
                .filter(|(r, _)| r.kind == RefKind::Branch)
                .map(|(r, _)| r.name.to_lowercase())
                .collect();
            let base = format!("{DEFAULT_BRANCH}-{}", sanitize(&crate::util::host_name()));
            let mut aside = base.clone();
            let mut i = 2;
            while taken.contains(&aside.to_lowercase()) {
                aside = format!("{base}-{i}");
                i += 1;
            }
            self.repo
                .rename_branch(lock, &game.id, DEFAULT_BRANCH, &aside)?;
            renamed.push((DEFAULT_BRANCH.to_string(), aside));
            self.repo
                .rename_branch(lock, &game.id, branch, DEFAULT_BRANCH)?;
            renamed.push((branch.to_string(), DEFAULT_BRANCH.to_string()));
            target = DEFAULT_BRANCH.to_string();
            self.index
                .set_refs(&game.id, &self.repo.list_refs(&game.id)?)?;
        }
        let key = self.machine_key()?;
        self.repo
            .set_primary_branch(lock, &game.id, &key, Some(&target))?;
        let label = self.machine_label_path(&key);
        if let Some(dir) = label.parent() {
            std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, e))?;
        }
        std::fs::write(&label, format!("{}\n", crate::util::host_name()))
            .map_err(|e| AppError::io(&label, e))?;
        tracing::info!(game = %game.id, branch = %target, "设为本机主存档");
        Ok(SetPrimaryReport {
            branch: target,
            renamed,
        })
    }

    /// 当前不在本机主存档上（在别的路线，或从旧快照恢复后还没备份）。还没有快照时为否。
    pub fn off_primary(&self, game_id: &str) -> Result<bool> {
        if self.repo.list_refs(game_id)?.is_empty() {
            return Ok(false);
        }
        let head = self.repo.head(game_id)?;
        Ok(head.detached.is_some() || head.branch != self.primary_branch(game_id)?)
    }

    /// 分支改名。
    pub(crate) fn rename_branch_raw(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        from: &str,
        to: &str,
    ) -> Result<()> {
        let to = to.trim();
        if self
            .repo
            .read_ref(&game.id, &RefName::branch(from))?
            .is_none()
        {
            return Err(AppError::Invalid(format!("找不到分支「{from}」")));
        }
        self.repo.rename_branch(lock, &game.id, from, to)?;
        self.index
            .set_refs(&game.id, &self.repo.list_refs(&game.id)?)?;
        tracing::info!(game = %game.id, from, to, "分支改名");
        Ok(())
    }

    /// 这条路线是哪些电脑的主存档（电脑的键）。
    pub fn primary_owners(&self, game_id: &str, branch: &str) -> Result<Vec<String>> {
        Ok(self
            .repo
            .primary_branches(game_id)?
            .into_iter()
            .filter(|(_, b)| b == branch)
            .map(|(k, _)| k)
            .collect())
    }

    /// 删除分支前检查：任一电脑的主存档都不能直接删除。
    pub(crate) fn check_not_primary(&self, game_id: &str, branch: &str) -> Result<()> {
        if self.primary_owners(game_id, branch)?.is_empty() {
            Ok(())
        } else {
            Err(AppError::Invalid(format!(
                "「{branch}」是主存档，不能删除。先把别的路线设为主存档，再删除它。"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::backup::BackupOptions;
    use crate::testutil::Env;

    #[test]
    fn rename_and_primary_cannot_be_deleted() {
        let mut env = Env::new();
        let g = env.add_game("Hollow Knight");
        env.write("0/user1.dat", b"one");
        let lock = env.app.repo.lock().unwrap();
        env.app
            .backup(&lock, &g, &BackupOptions::default())
            .unwrap();
        let tip = env.app.repo.head_snapshot(&g.id).unwrap().unwrap();
        env.app
            .create_ref(
                &lock,
                &g,
                bonfire_core::refs::RefKind::Branch,
                "side",
                Some(&tip.to_hex()),
            )
            .unwrap();
        env.app
            .rename_branch(&lock, &g, "side", "main-OLD")
            .unwrap();
        let names: Vec<String> = env
            .app
            .repo
            .list_refs(&g.id)
            .unwrap()
            .into_iter()
            .map(|(r, _)| r.name)
            .collect();
        assert!(names.contains(&"main-OLD".to_string()) && !names.contains(&"side".to_string()));
        assert!(env.app.rename_branch(&lock, &g, "missing", "x").is_err());

        env.app
            .repo
            .set_primary_branch(&lock, &g.id, "pc-a", Some("main-OLD"))
            .unwrap();
        let err = env
            .app
            .delete_branch(&lock, &g, "main-OLD", true)
            .unwrap_err();
        assert!(err.to_string().contains("主存档"));
        env.app
            .repo
            .set_primary_branch(&lock, &g.id, "pc-a", None)
            .unwrap();
        assert!(env.app.delete_branch(&lock, &g, "main-OLD", true).is_ok());
    }

    #[test]
    fn set_primary_with_tidy_and_machine_key() {
        let mut env = Env::new();
        let g = env.add_game("Hollow Knight");
        env.write("0/user1.dat", b"one");
        let lock = env.app.repo.lock().unwrap();
        env.app
            .backup(&lock, &g, &BackupOptions::default())
            .unwrap();
        let tip = env.app.repo.head_snapshot(&g.id).unwrap().unwrap();
        env.app
            .create_ref(
                &lock,
                &g,
                bonfire_core::refs::RefKind::Branch,
                "main-OLDPC",
                Some(&tip.to_hex()),
            )
            .unwrap();
        // 没有设置时以 main 为主存档，当前就在 main 上。
        assert_eq!(env.app.primary_branch(&g.id).unwrap(), "main");
        assert!(!env.app.off_primary(&g.id).unwrap());
        let key = env.app.machine_key().unwrap();
        assert!(key.starts_with("m-"));
        assert_eq!(env.app.machine_key().unwrap(), key);

        // 不整理名称：只移动标记。
        let r = env.app.set_primary(&lock, &g, "main-OLDPC", false).unwrap();
        assert_eq!(r.branch, "main-OLDPC");
        assert!(r.renamed.is_empty());
        assert!(env.app.off_primary(&g.id).unwrap());

        // 整理名称：原 main 改名为 main-本机电脑名，选中的改回 main，当前分支随原 main 改名。
        let r = env.app.set_primary(&lock, &g, "main-OLDPC", true).unwrap();
        assert_eq!(r.branch, "main");
        assert_eq!(r.renamed[1], ("main-OLDPC".to_string(), "main".to_string()));
        let aside = r.renamed[0].1.clone();
        assert!(aside.starts_with("main-"));
        assert_eq!(env.app.repo.head(&g.id).unwrap().branch, aside);
        assert_eq!(env.app.primary_branch(&g.id).unwrap(), "main");
        assert!(env.app.off_primary(&g.id).unwrap());
        let views = env.app.primaries(&g.id).unwrap();
        assert!(views[0].mine && views[0].branch == "main");
        // 主存档不能删除。
        assert!(env.app.delete_branch(&lock, &g, "main", true).is_err());
    }

    #[test]
    fn import_old_pc_main_as_primary() {
        use crate::transfer::{ArchivePick, PackageOptions};
        // 旧电脑：有自己的进度。
        let mut old = Env::new();
        let g1 = old.add_game("Hollow Knight");
        old.write("0/user1.dat", b"old progress");
        let out = old.dir.path().join("old.bonfire");
        {
            let lock = old.app.repo.lock().unwrap();
            old.app
                .backup(&lock, &g1, &BackupOptions::default())
                .unwrap();
            old.app
                .export_package(
                    std::slice::from_ref(&g1),
                    &out,
                    &PackageOptions {
                        saves: true,
                        shots: false,
                        shot_ids: Default::default(),
                    },
                )
                .unwrap();
        }
        // 新电脑（同样的电脑名）：加入后首次备份产生了自己的 main。
        let mut new = Env::new();
        let pkg = new.dir.path().join("old.bonfire");
        std::fs::copy(&out, &pkg).unwrap();
        let g2 = new.add_game("Hollow Knight");
        new.write("0/user1.dat", b"fresh start");
        let lock = new.app.repo.lock().unwrap();
        new.app
            .backup(&lock, &g2, &BackupOptions::default())
            .unwrap();

        let preview = new.app.inspect_archive(&pkg).unwrap();
        let key = preview.games[0].key.clone();
        let pick = |adopt: bool| ArchivePick {
            target: Some(g2.id.clone()),
            adopt_main: adopt,
            ..ArchivePick::all(key.clone())
        };
        // 预览：包内的 main 与本机分叉。
        let dry = new
            .app
            .import_archive(&lock, &pkg, &[pick(true)], true)
            .unwrap();
        let conflict = dry[0]
            .report
            .as_ref()
            .unwrap()
            .preview
            .main_conflict
            .clone()
            .unwrap();
        assert!(conflict.starts_with("main-"));
        assert!(dry[0].adopted_main.is_none());

        let items = new
            .app
            .import_archive(&lock, &pkg, &[pick(true)], false)
            .unwrap();
        assert_eq!(
            items[0].adopted_main.as_deref(),
            Some("main"),
            "{:?}",
            items[0].error
        );
        // 导入的旧电脑路线成为 main 与本机主存档；新电脑原来的 main 改名保留（同名电脑也不冲突）。
        assert_eq!(new.app.primary_branch(&g2.id).unwrap(), "main");
        let names: Vec<String> = new
            .app
            .repo
            .list_refs(&g2.id)
            .unwrap()
            .into_iter()
            .map(|(r, _)| r.name)
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.iter().any(|n| n != "main" && n.starts_with("main-")));
        assert!(new.app.off_primary(&g2.id).unwrap());
    }
}
