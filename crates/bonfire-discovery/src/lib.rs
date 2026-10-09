//! 引擎·存档发现（文档 §4）：平台适配器 + 存档规则库两层，以及游戏信息获取。
//!
//! 适配器回答“装了哪些游戏、装在哪、用户 ID 是什么”，规则库回答“这个游戏的存档在哪”。
//! 本 crate 对磁盘只读（导出注册表到缓存目录除外）。

pub mod http;
pub mod manifest;
pub mod metadata;
pub mod paths;
pub mod platforms;
pub mod registry;
pub mod resolve;
pub mod shots;
pub mod vdf;

pub use manifest::{ManifestPaths, RuleIndex};
pub use paths::KnownDirs;
pub use platforms::{InstalledGame, PlatformAdapter};
pub use resolve::{Discovered, FoundKind, FoundLocation, LocationSource};
