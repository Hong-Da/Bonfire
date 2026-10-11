use std::path::PathBuf;

use crate::ObjectId;
use crate::object::Kind;

pub type Result<T, E = CoreError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("仓库未初始化：{0}")]
    NotInitialized(PathBuf),
    #[error("仓库格式版本 {found} 高于本程序支持的 {supported}，请升级篝火")]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("仓库配置无效：{0}")]
    BadRepoConfig(String),
    #[error("仓库正被另一个篝火实例写入：{0}")]
    Locked(PathBuf),
    #[error(
        "仓库已加密，请先输入密码（命令行可设置环境变量 BONFIRE_PASSWORD，或运行 bonfire repo unlock --remember 在本机记住）"
    )]
    NeedPassword,
    #[error("密码不对")]
    WrongPassword,
    #[error("找不到{kind}对象 {id}")]
    NotFound { kind: Kind, id: ObjectId },
    #[error("{kind}对象 {id} 已损坏：{reason}")]
    Corrupt {
        kind: Kind,
        id: ObjectId,
        reason: String,
    },
    #[error("名称无效「{name}」：{reason}")]
    InvalidName { name: String, reason: &'static str },
    #[error("找不到引用「{0}」")]
    RefNotFound(String),
    #[error("「{0}」已存在")]
    RefExists(String),
    #[error("引用「{name}」有歧义，可能是：{candidates}")]
    AmbiguousRef { name: String, candidates: String },
    #[error("引用 {0} 已被其他操作修改，请重试")]
    RefConflict(String),
    #[error("{0} 没有更早的快照")]
    NoParent(String),
    #[error("游戏 {0} 还没有快照")]
    NoSnapshots(String),
    #[error("{path} 格式错误：{reason}")]
    BadFile { path: PathBuf, reason: String },
    #[error("索引数据库错误：{0}")]
    Db(#[from] rusqlite::Error),
}

impl CoreError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
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
