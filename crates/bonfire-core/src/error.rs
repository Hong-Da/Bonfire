use std::path::PathBuf;

use crate::object::Kind;
use crate::ObjectId;

pub type Result<T, E = CoreError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("仓库未初始化：{0}")]
    NotInitialized(PathBuf),
    #[error("仓库格式版本 {found} 高于本程序支持的 {supported}，请升级篝火")]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("仓库配置无效：{0}")]
    BadRepoConfig(String),
    #[error("仓库正被另一个篝火实例写入：{0}")]
    Locked(PathBuf),
    #[error("找不到{kind}对象 {id}")]
    NotFound { kind: Kind, id: ObjectId },
    #[error("{kind}对象 {id} 已损坏：{reason}")]
    Corrupt { kind: Kind, id: ObjectId, reason: String },
}

impl CoreError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io { path: path.into(), source }
    }
}

/// 给 `std::io::Result` 附上路径。
pub(crate) trait IoContext<T> {
    fn at(self, path: impl Into<PathBuf>) -> Result<T>;
}

impl<T> IoContext<T> for std::io::Result<T> {
    fn at(self, path: impl Into<PathBuf>) -> Result<T> {
        self.map_err(|e| CoreError::io(path, e))
    }
}
