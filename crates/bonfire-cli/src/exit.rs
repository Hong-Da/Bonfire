//! 退出码约定（见 docs/cli.md）。

pub const OK: u8 = 0;
#[allow(dead_code)]
pub const GENERAL: u8 = 1;
/// 参数错误（clap 解析失败时也使用 2）。
#[allow(dead_code)]
pub const USAGE: u8 = 2;
pub const CONFIG: u8 = 3;
#[allow(dead_code)]
pub const NOT_FOUND: u8 = 4;
#[allow(dead_code)]
pub const EXTERNAL: u8 = 5;
#[allow(dead_code)]
pub const REPO_LOCKED: u8 = 6;
#[allow(dead_code)]
pub const GAME_RUNNING: u8 = 7;
