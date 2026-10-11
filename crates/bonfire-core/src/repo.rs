//! 仓库目录、仓库参数与写入锁（文档 6.1、6.5）。
//!
//! ```text
//! repo/
//! ├─ config.toml        仓库格式版本、压缩与分块参数
//! ├─ lock               写入锁（进程持有期间独占）
//! ├─ objects/
//! │   ├─ chunks/ab/cdef…   各类对象按类型分目录，前 2 位哈希为子目录
//! │   ├─ blobs/  trees/  snapshots/
//! │   └─ tmp/              写入中的临时文件，原子重命名后移走
//! ├─ refs/  notes/  logs/  （M1-3）
//! └─ bonfire.db         （M1-4）
//! ```

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, IoContext, Result};
use crate::object::Kind;

/// 本程序支持的仓库格式版本。2：加密仓库（`[encryption]`）。未加密的仓库仍写 1，旧版本可读。
pub const FORMAT_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoConfig {
    pub format_version: u32,
    pub compression: String,
    pub zstd_level: i32,
    /// 小于该大小的文件不分块，整个作为一个 Chunk。
    pub chunk_threshold: u32,
    pub chunk_min: u32,
    pub chunk_avg: u32,
    pub chunk_max: u32,
    /// 加密设置（M5-2）；未加密时没有这一节。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption: Option<crate::crypt::EncryptionConfig>,
}

impl Default for RepoConfig {
    fn default() -> Self {
        const MIB: u32 = 1024 * 1024;
        Self {
            format_version: 1,
            compression: "zstd".into(),
            zstd_level: 3,
            chunk_threshold: MIB,
            chunk_min: MIB / 4,
            chunk_avg: MIB,
            chunk_max: 4 * MIB,
            encryption: None,
        }
    }
}

impl RepoConfig {
    fn validate(&self) -> Result<()> {
        if self.format_version > FORMAT_VERSION {
            return Err(CoreError::UnsupportedVersion {
                found: self.format_version,
                supported: FORMAT_VERSION,
            });
        }
        if self.compression != "zstd" {
            return Err(CoreError::BadRepoConfig(format!(
                "不支持的压缩算法 {}",
                self.compression
            )));
        }
        // FastCDC 2020 的参数范围。
        let ok = (64..=1_048_576).contains(&self.chunk_min)
            && (256..=4_194_304).contains(&self.chunk_avg)
            && (1024..=16_777_216).contains(&self.chunk_max)
            && self.chunk_min <= self.chunk_avg
            && self.chunk_avg <= self.chunk_max;
        if !ok {
            return Err(CoreError::BadRepoConfig("分块参数超出范围".into()));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct Repo {
    root: PathBuf,
    config: RepoConfig,
    /// 加密仓库解锁后的数据密钥。
    cipher: std::sync::RwLock<Option<std::sync::Arc<crate::crypt::Cipher>>>,
}

impl Repo {
    /// 初始化新仓库；目录已是仓库时直接打开。
    pub fn init(root: &Path) -> Result<Self> {
        if root.join("config.toml").is_file() {
            return Self::open(root);
        }
        let config = RepoConfig::default();
        let repo = Self {
            root: root.to_path_buf(),
            config,
            cipher: Default::default(),
        };
        repo.create_dirs()?;
        let text = toml::to_string_pretty(&repo.config).expect("仓库配置序列化不会失败");
        crate::store::write_atomic(
            &repo.tmp_dir(),
            &root.join("config.toml"),
            text.as_bytes(),
            false,
        )?;
        Ok(repo)
    }

    pub fn open(root: &Path) -> Result<Self> {
        let cfg_path = root.join("config.toml");
        let text = match std::fs::read_to_string(&cfg_path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(CoreError::NotInitialized(root.into()));
            }
            Err(e) => return Err(CoreError::io(cfg_path, e)),
        };
        let config: RepoConfig =
            toml::from_str(&text).map_err(|e| CoreError::BadRepoConfig(e.to_string()))?;
        config.validate()?;
        let repo = Self {
            root: root.to_path_buf(),
            config,
            cipher: Default::default(),
        };
        // 补齐可能缺失的目录（例如从网盘同步过来的仓库）。
        repo.create_dirs()?;
        Ok(repo)
    }

    fn create_dirs(&self) -> Result<()> {
        let mut dirs = vec![
            self.tmp_dir(),
            self.root.join("refs"),
            self.root.join("notes"),
            self.root.join("logs"),
        ];
        dirs.extend(Kind::ALL.iter().map(|k| self.objects_dir().join(k.dir())));
        for d in dirs {
            std::fs::create_dir_all(&d).at(&d)?;
        }
        Ok(())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn config(&self) -> &RepoConfig {
        &self.config
    }

    /// 仓库是否加密。
    pub fn is_encrypted(&self) -> bool {
        self.config.encryption.is_some()
    }

    /// 加密仓库是否已解锁（未加密的仓库总是可读）。
    pub fn is_unlocked(&self) -> bool {
        !self.is_encrypted() || self.cipher.read().expect("锁未中毒").is_some()
    }

    /// 用密码解锁，返回数据密钥（本机记住密码时由调用方保存）。
    pub fn unlock(&self, password: &str) -> Result<std::sync::Arc<crate::crypt::Cipher>> {
        let cfg = self
            .config
            .encryption
            .as_ref()
            .ok_or_else(|| CoreError::BadRepoConfig("仓库没有加密".into()))?;
        let c = std::sync::Arc::new(crate::crypt::unlock(cfg, password)?);
        *self.cipher.write().expect("锁未中毒") = Some(c.clone());
        Ok(c)
    }

    /// 用记住的数据密钥解锁；不属于这个仓库时返回 false。
    pub fn unlock_with_key(&self, key: [u8; 32]) -> bool {
        let Some(cfg) = &self.config.encryption else {
            return false;
        };
        let c = crate::crypt::Cipher::from_key(key);
        if !c.verify(cfg) {
            return false;
        }
        *self.cipher.write().expect("锁未中毒") = Some(std::sync::Arc::new(c));
        true
    }

    /// 当前的数据密钥（加密仓库已解锁时）。
    pub fn cipher(&self) -> Option<std::sync::Arc<crate::crypt::Cipher>> {
        self.cipher.read().expect("锁未中毒").clone()
    }

    /// 写入时加密（加密仓库）；未加密的仓库原样返回。
    pub fn seal(&self, plain: Vec<u8>) -> Result<Vec<u8>> {
        if !self.is_encrypted() {
            return Ok(plain);
        }
        let c = self.cipher().ok_or(CoreError::NeedPassword)?;
        Ok(c.seal(&plain))
    }

    /// 读取时解密：加密格式的数据需要数据密钥，其余原样返回（加密或解密中途中断时新旧格式并存）。
    pub fn unseal(&self, data: Vec<u8>, what: &str) -> Result<Vec<u8>> {
        if !crate::crypt::is_sealed(&data) {
            return Ok(data);
        }
        let c = self.cipher().ok_or(CoreError::NeedPassword)?;
        c.open(&data, what)
    }

    /// 写入新的加密设置（`None` 为取消加密）。调用方之后重新打开仓库。
    pub fn write_encryption(
        &self,
        _lock: &RepoLock,
        enc: Option<crate::crypt::EncryptionConfig>,
    ) -> Result<()> {
        let mut cfg = self.config.clone();
        cfg.format_version = if enc.is_some() { 2 } else { 1 };
        cfg.encryption = enc;
        let text = toml::to_string_pretty(&cfg).expect("仓库配置序列化不会失败");
        crate::store::write_atomic(
            &self.tmp_dir(),
            &self.root.join("config.toml"),
            text.as_bytes(),
            false,
        )
    }

    pub fn objects_dir(&self) -> PathBuf {
        self.root.join("objects")
    }

    pub(crate) fn tmp_dir(&self) -> PathBuf {
        self.objects_dir().join("tmp")
    }

    /// 获取写入锁；已被其他进程持有时立即返回 [`CoreError::Locked`]。
    pub fn lock(&self) -> Result<RepoLock> {
        let path = self.root.join("lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .at(&path)?;
        match file.try_lock() {
            Ok(()) => Ok(RepoLock { _file: file }),
            Err(TryLockError::WouldBlock) => Err(CoreError::Locked(self.root.clone())),
            Err(TryLockError::Error(e)) => Err(CoreError::io(path, e)),
        }
    }
}

/// 写入锁，离开作用域时释放。
#[derive(Debug)]
pub struct RepoLock {
    _file: File,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_then_open() {
        let dir = tempfile::tempdir().unwrap();
        let r = Repo::init(dir.path()).unwrap();
        assert_eq!(r.config(), &RepoConfig::default());
        for k in Kind::ALL {
            assert!(dir.path().join("objects").join(k.dir()).is_dir());
        }
        let r2 = Repo::open(dir.path()).unwrap();
        assert_eq!(r2.config(), r.config());
        // 再次 init 不覆盖已有仓库。
        Repo::init(dir.path()).unwrap();
    }

    #[test]
    fn open_uninitialized() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            Repo::open(dir.path()),
            Err(CoreError::NotInitialized(_))
        ));
    }

    #[test]
    fn rejects_newer_format() {
        let dir = tempfile::tempdir().unwrap();
        Repo::init(dir.path()).unwrap();
        let p = dir.path().join("config.toml");
        let text = std::fs::read_to_string(&p)
            .unwrap()
            .replace("format_version = 1", "format_version = 99");
        std::fs::write(&p, text).unwrap();
        assert!(matches!(
            Repo::open(dir.path()),
            Err(CoreError::UnsupportedVersion { found: 99, .. })
        ));
    }

    #[test]
    fn rejects_bad_chunk_params() {
        let dir = tempfile::tempdir().unwrap();
        Repo::init(dir.path()).unwrap();
        let p = dir.path().join("config.toml");
        let text = std::fs::read_to_string(&p)
            .unwrap()
            .replace("chunk_min = 262144", "chunk_min = 8");
        std::fs::write(&p, text).unwrap();
        assert!(matches!(
            Repo::open(dir.path()),
            Err(CoreError::BadRepoConfig(_))
        ));
    }

    #[test]
    fn lock_is_exclusive_and_released_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let r = Repo::init(dir.path()).unwrap();
        let guard = r.lock().unwrap();
        assert!(matches!(r.lock(), Err(CoreError::Locked(_))));
        drop(guard);
        r.lock().unwrap();
    }
}
