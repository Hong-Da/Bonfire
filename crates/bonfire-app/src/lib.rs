//! 应用服务层：扫描、备份、恢复、历史、清理等服务，以及配置、目录、日志与多语言。
//!
//! CLI 与 Tauri 命令层只调用本 crate，不直接实现业务逻辑。

pub mod app;
pub mod backup;
pub mod config;
pub mod error;
pub mod i18n;
pub mod logging;
pub mod paths;
pub mod util;

#[cfg(test)]
mod testutil;

pub use app::{App, NewGame};
pub use backup::{BackupOptions, BackupResult, DetachedChoice, GameState, GameStatus};
pub use config::Config;
pub use error::{AppError, ErrorKind};
pub use i18n::{Locale, t, tf};
pub use paths::AppPaths;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
