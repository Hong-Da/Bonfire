use std::path::PathBuf;

use bonfire_core::CoreError;

pub type Result<T, E = AppError> = std::result::Result<T, E>;

/// 应用层错误。CLI 按 [`AppError::kind`] 决定退出码。
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error(transparent)]
    Config(#[from] crate::config::ConfigError),
    #[error("找不到游戏「{0}」")]
    GameNotFound(String),
    #[error("「{name}」对应多个游戏：{candidates}，请改用游戏 ID")]
    AmbiguousGame { name: String, candidates: String },
    #[error("路径不存在：{0}")]
    PathNotFound(PathBuf),
    #[error("游戏「{0}」没有存档位置")]
    NoLocations(String),
    #[error("备注不能超过 {max} 个字符（当前 {len}）")]
    CommentTooLong { len: usize, max: usize },
    #[error("文件在备份过程中持续被修改：{0}，请关闭游戏后重试")]
    FileUnstable(PathBuf),
    #[error("游戏正在运行（{processes}），恢复可能被游戏覆盖；请先关闭游戏，或使用 --force")]
    GameRunning { processes: String },
    #[error("游戏「{0}」没有可撤销的恢复")]
    NothingToUndo(String),
    #[error("找不到备份方案「{0}」")]
    ProfileNotFound(String),
    #[error("恢复失败，已回滚到恢复前的状态：{0}")]
    RolledBack(String),
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{0}")]
    Invalid(String),
}

/// 错误类别，对应 CLI 退出码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    General,
    Usage,
    Config,
    NotFound,
    External,
    Locked,
    GameRunning,
}

impl AppError {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io { path: path.into(), source }
    }

    pub fn kind(&self) -> ErrorKind {
        match self {
            AppError::Core(e) => match e {
                CoreError::Locked(_) => ErrorKind::Locked,
                CoreError::NotInitialized(_) | CoreError::UnsupportedVersion { .. } | CoreError::BadRepoConfig(_) => ErrorKind::Config,
                CoreError::RefNotFound(_) | CoreError::NoSnapshots(_) | CoreError::NoParent(_) => ErrorKind::NotFound,
                CoreError::InvalidName { .. } | CoreError::AmbiguousRef { .. } | CoreError::RefExists(_) => ErrorKind::Usage,
                CoreError::Db(_) => ErrorKind::External,
                _ => ErrorKind::General,
            },
            AppError::Config(_) => ErrorKind::Config,
            AppError::GameNotFound(_) | AppError::ProfileNotFound(_) | AppError::PathNotFound(_) | AppError::NothingToUndo(_) => ErrorKind::NotFound,
            AppError::AmbiguousGame { .. } | AppError::CommentTooLong { .. } | AppError::NoLocations(_) | AppError::Invalid(_) => ErrorKind::Usage,
            AppError::GameRunning { .. } => ErrorKind::GameRunning,
            _ => ErrorKind::General,
        }
    }
}
