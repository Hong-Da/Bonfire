//! 游戏运行检测（简化版，M4 完善）：可执行文件位于游戏安装目录下的进程视为游戏在运行。

use std::path::{Path, PathBuf};

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// 正在运行的、可执行文件位于任一目录下的进程，返回 (进程名, 路径)。
pub fn running_under(dirs: &[PathBuf]) -> Vec<(String, PathBuf)> {
    if dirs.is_empty() {
        return Vec::new();
    }
    let dirs: Vec<PathBuf> = dirs.iter().map(|d| normalize(d)).collect();
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
    let mut out: Vec<(String, PathBuf)> = sys
        .processes()
        .values()
        .filter_map(|p| {
            let exe = p.exe()?;
            let n = normalize(exe);
            dirs.iter()
                .any(|d| n.starts_with(d))
                .then(|| (p.name().to_string_lossy().into_owned(), exe.to_path_buf()))
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Windows 路径不区分大小写：统一转为小写比较。
fn normalize(p: &Path) -> PathBuf {
    let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    if cfg!(windows) {
        PathBuf::from(abs.to_string_lossy().to_lowercase())
    } else {
        abs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_dirs_means_nothing() {
        assert!(running_under(&[]).is_empty());
    }

    #[test]
    #[cfg(windows)]
    fn detects_process_running_from_dir() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("FakeGame.exe");
        std::fs::copy(r"C:\Windows\System32\ping.exe", &exe).unwrap();
        let mut child = std::process::Command::new(&exe)
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let found = running_under(&[dir.path().to_path_buf()]);
        child.kill().unwrap();
        let _ = child.wait();
        assert!(
            found
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("FakeGame.exe")),
            "{found:?}"
        );
        let other = tempfile::tempdir().unwrap();
        assert!(running_under(&[other.path().to_path_buf()]).is_empty());
    }
}
