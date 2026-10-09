//! 应用目录解析：标准模式与便携模式。
//!
//! - 标准模式：配置在 `%APPDATA%\Bonfire\`，日志与缓存在 `%LOCALAPPDATA%\Bonfire\`；
//!   默认仓库在程序目录下的 `data`（开发时为项目根目录下的 `data`），可在配置中修改。
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
        let base = std::env::current_exe().ok().and_then(|exe| exe.parent().map(program_root)).unwrap_or_else(|| PathBuf::from("."));
        Self {
            config_file: config_root.join("config.toml"),
            default_repo_dir: base.join(DATA_DIR),
            logs_dir: data_root.join("logs"),
            cache_dir: data_root.join("cache"),
            portable: false,
        }
    }
}

/// 默认仓库所在的目录名：项目（程序）目录下的 `data`。
pub const DATA_DIR: &str = "data";

/// 程序的根目录：开发时从 `<项目>\target\<debug|release>[\deps]` 运行，返回项目根目录；
/// 安装后返回程序所在目录。默认仓库为其下的 `data`。
pub fn program_root(exe_dir: &Path) -> PathBuf {
    let target = exe_dir.ancestors().take(3).find(|p| p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("target")));
    match target.and_then(Path::parent) {
        Some(project) if project.join("Cargo.toml").is_file() => project.to_path_buf(),
        _ => exe_dir.to_path_buf(),
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
    fn standard_uses_app_dir_name_and_project_data() {
        let p = AppPaths::standard();
        assert!(p.config_file.ends_with("Bonfire/config.toml"));
        // 测试程序位于 <项目>/target/debug/deps，默认仓库是项目根目录下的 data。
        let project = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
        assert_eq!(p.default_repo_dir, project.join("data"));
        assert!(!p.portable);
    }

    #[test]
    fn program_root_detection() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("Cargo.toml"), "").unwrap();
        for sub in ["target/debug", "target/release", "target/debug/deps"] {
            assert_eq!(program_root(&d.path().join(sub)), d.path(), "{sub}");
        }
        // 安装目录：没有 target 上级，就是程序所在目录。
        let install = d.path().join("Programs/Bonfire");
        assert_eq!(program_root(&install), install);
        // target 的上级不是 Cargo 项目：仍按程序目录处理。
        let other = tempfile::tempdir().unwrap();
        assert_eq!(program_root(&other.path().join("target/release")), other.path().join("target/release"));
    }
}
