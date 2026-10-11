//! 应用服务层：扫描、备份、恢复、历史、清理等服务，以及配置、目录、日志与多语言。
//!
//! CLI 与 Tauri 命令层只调用本 crate，不直接实现业务逻辑。

pub mod app;
pub mod auto;
pub mod backup;
pub mod config;
pub mod discover;
pub mod downloads;
pub mod error;
pub mod gallery;
pub mod history;
pub mod i18n;
pub mod library;
pub mod logging;
pub mod metadata;
pub mod paths;
pub mod primary;
pub mod process;
pub mod prune;
pub mod restore;
pub mod shots;
pub mod transfer;
pub mod ui_state;
pub mod unsorted;
pub mod util;

#[cfg(test)]
mod testutil;

pub use app::{App, NewGame};
pub use backup::{BackupOptions, BackupResult, DetachedChoice, GameState, GameStatus};
pub use config::Config;
pub use error::{AppError, ErrorKind};
pub use history::{DiffEntry, DiffStatus, LogEntry};
pub use i18n::{Locale, t, tf};
pub use paths::AppPaths;
pub use restore::{Action, RestoreOptions, RestorePlan, RestoreReport};

/// 引擎层，供 CLI 与 Tauri 命令层使用其中的数据类型。
pub use bonfire_core as core;
/// 存档发现层，供命令层使用其中的数据类型。
pub use bonfire_discovery as discovery;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
