//! 退出码约定（见 docs/cli.md）。

pub const OK: u8 = 0;
pub const GENERAL: u8 = 1;
/// 参数错误（clap 解析失败时也使用 2）。
pub const USAGE: u8 = 2;
pub const CONFIG: u8 = 3;
pub const NOT_FOUND: u8 = 4;
pub const EXTERNAL: u8 = 5;
pub const REPO_LOCKED: u8 = 6;
pub const GAME_RUNNING: u8 = 7;
