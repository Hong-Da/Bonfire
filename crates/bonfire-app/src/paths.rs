//! 应用目录解析。
//!
//! 所有数据（配置 `config.toml`、仓库 `repo`、日志 `logs`、缓存 `cache`、窗口数据 `webview`）
//! 都放在程序目录下的 `data` 中；开发时（从 `<项目>/target/debug|release` 运行）为项目根目录下的 `data`。
//! 仓库位置可在配置中修改。
//! - 程序目录下存在 `portable` 标记文件时同样使用 `<程序目录>\data`（与标准模式一致，标记仅用于界面提示）。
//! - 环境变量 `BONFIRE_HOME` 指定另一个数据根目录（测试与自动化使用）。
//! - 环境变量 `BONFIRE_CONFIG` 单独指定配置文件路径。

use std::path::{Path, PathBuf};

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

    /// 标准模式：配置、仓库、日志、缓存都在程序（开发时为项目）目录下的 `data` 中。
    fn standard() -> Self {
        let base = std::env::current_exe().ok().and_then(|exe| exe.parent().map(program_root)).unwrap_or_else(|| PathBuf::from("."));
        Self::under_root(&base.join(DATA_DIR), false)
    }

    /// 窗口（WebView）数据目录，与其他数据放在一起。
    pub fn webview_dir(&self) -> PathBuf {
        self.logs_dir.parent().map(|p| p.join("webview")).unwrap_or_else(|| PathBuf::from("webview"))
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
    fn standard_puts_everything_in_project_data() {
        let p = AppPaths::standard();
        // 测试程序位于 <项目>/target/debug/deps：全部数据在项目根目录下的 data。
        let data = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap().join("data");
        assert_eq!(p.config_file, data.join("config.toml"));
        assert_eq!(p.default_repo_dir, data.join("repo"));
        assert_eq!(p.logs_dir, data.join("logs"));
        assert_eq!(p.cache_dir, data.join("cache"));
        assert_eq!(p.webview_dir(), data.join("webview"));
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
