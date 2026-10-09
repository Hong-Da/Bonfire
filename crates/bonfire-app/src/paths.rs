//! 应用目录解析：标准模式与便携模式。
//!
//! - 标准模式：配置在 `%APPDATA%\Bonfire\`，数据（仓库、日志、缓存）在 `%LOCALAPPDATA%\Bonfire\`。
//! - 便携模式：程序目录下存在 `portable` 标记文件时，全部放在程序目录旁的 `data\` 下。
//! - 环境变量 `BONFIRE_HOME` 指定一个根目录，效果同便携模式（测试与自动化使用）。
//! - 环境变量 `BONFIRE_CONFIG` 单独指定配置文件路径。

use std::path::{Path, PathBuf};

pub const APP_DIR_NAME: &str = "Bonfire";
pub const PORTABLE_MARKER: &str = "portable";
pub const ENV_HOME: &str = "BONFIRE_HOME";
pub const ENV_CONFIG: &str = "BONFIRE_CONFIG";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub config_file: PathBuf,
    /// 未在配置中指定仓库位置时使用的默认仓库目录。
    pub default_repo_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub portable: bool,
}

impl AppPaths {
    /// 按环境变量、便携标记、系统目录的顺序解析。
    pub fn resolve() -> Self {
        let mut paths = if let Some(home) = std::env::var_os(ENV_HOME).filter(|v| !v.is_empty()) {
            Self::under_root(Path::new(&home), true)
        } else if let Some(root) = portable_root() {
            Self::under_root(&root, true)
        } else {
            Self::standard()
        };
        if let Some(cfg) = std::env::var_os(ENV_CONFIG).filter(|v| !v.is_empty()) {
            paths.config_file = PathBuf::from(cfg);
        }
        paths
    }

    /// 所有内容放在同一个根目录下。
    pub fn under_root(root: &Path, portable: bool) -> Self {
        Self {
            config_file: root.join("config.toml"),
            default_repo_dir: root.join("repo"),
            logs_dir: root.join("logs"),
            cache_dir: root.join("cache"),
            portable,
        }
    }

    fn standard() -> Self {
        // 两个系统目录都取不到时（极少见）退回当前目录，保证程序仍能启动。
        let config_root = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join(APP_DIR_NAME);
        let data_root = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join(APP_DIR_NAME);
        Self {
            config_file: config_root.join("config.toml"),
            default_repo_dir: data_root.join("repo"),
            logs_dir: data_root.join("logs"),
            cache_dir: data_root.join("cache"),
            portable: false,
        }
    }
}

/// 程序目录下有 `portable` 标记文件时，返回 `<程序目录>\data`。
fn portable_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    dir.join(PORTABLE_MARKER).is_file().then(|| dir.join("data"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_root_puts_everything_in_root() {
        let p = AppPaths::under_root(Path::new("D:/x"), true);
        assert_eq!(p.config_file, Path::new("D:/x/config.toml"));
        assert_eq!(p.default_repo_dir, Path::new("D:/x/repo"));
        assert_eq!(p.logs_dir, Path::new("D:/x/logs"));
        assert_eq!(p.cache_dir, Path::new("D:/x/cache"));
        assert!(p.portable);
    }

    #[test]
    fn standard_uses_app_dir_name() {
        let p = AppPaths::standard();
        assert!(p.config_file.ends_with("Bonfire/config.toml"));
        assert!(p.default_repo_dir.ends_with("Bonfire/repo"));
        assert!(!p.portable);
    }
}
