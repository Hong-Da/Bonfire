//! 应用服务层：扫描、备份、恢复、历史、清理等服务，以及配置、目录、日志与多语言。
//!
//! CLI 与 Tauri 命令层只调用本 crate，不直接实现业务逻辑。

pub mod config;
pub mod i18n;
pub mod logging;
pub mod paths;

pub use config::Config;
pub use i18n::{Locale, t};
pub use paths::AppPaths;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
