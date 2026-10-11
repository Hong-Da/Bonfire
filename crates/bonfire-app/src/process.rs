//! 游戏运行检测（文档 F7，M4-2）：进程的可执行文件位于游戏安装目录下，
//! 或进程名是为游戏设置的“游戏程序”（如模拟器、手动添加的游戏）时，视为游戏在运行。
//! 一次刷新进程列表即可检测多个游戏。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bonfire_core::RepoLock;
use bonfire_core::index::GameRecord;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

use crate::error::{AppError, Result};

/// 一个游戏的检测依据。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Watch {
    pub game_id: String,
    /// 安装目录：可执行文件在其中的进程。
    pub dir: Option<PathBuf>,
    /// 游戏程序：进程名（不区分大小写，可省略 `.exe`）。
    pub names: Vec<String>,
}

impl Watch {
    pub fn is_empty(&self) -> bool {
        self.dir.is_none() && self.names.is_empty()
    }
}

/// 正在运行的进程：(进程名, 可执行文件路径)。
pub fn processes() -> Vec<(String, Option<PathBuf>)> {
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
    sys.processes()
        .values()
        .map(|p| {
            (
                p.name().to_string_lossy().into_owned(),
                p.exe().map(Path::to_path_buf),
            )
        })
        .collect()
}

/// 进程名规范化：小写、去掉 `.exe`。
pub fn process_key(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    lower.strip_suffix(".exe").unwrap_or(&lower).to_string()
}

/// 在给定的进程列表中检测：返回 (游戏 ID, 进程名) 列表，只含在运行的游戏。
pub fn match_running(
    watches: &[Watch],
    procs: &[(String, Option<PathBuf>)],
) -> Vec<(String, Vec<String>)> {
    let procs: Vec<(String, String, Option<PathBuf>)> = procs
        .iter()
        .map(|(n, exe)| (n.clone(), process_key(n), exe.as_deref().map(normalize)))
        .collect();
    watches
        .iter()
        .filter_map(|w| {
            let dir = w.dir.as_deref().map(normalize);
            let keys: Vec<String> = w.names.iter().map(|n| process_key(n)).collect();
            let mut names: Vec<String> = procs
                .iter()
                .filter(|(_, key, exe)| {
                    keys.contains(key)
                        || matches!((&dir, exe), (Some(d), Some(e)) if e.starts_with(d))
                })
                .map(|(n, _, _)| n.clone())
                .collect();
            names.sort();
            names.dedup();
            (!names.is_empty()).then(|| (w.game_id.clone(), names))
        })
        .collect()
}

/// 刷新进程列表并检测。没有检测依据时不刷新。
pub fn running(watches: &[Watch]) -> Vec<(String, Vec<String>)> {
    if watches.iter().all(Watch::is_empty) {
        return Vec::new();
    }
    match_running(watches, &processes())
}

impl crate::app::App {
    /// 游戏的检测依据：安装目录与游戏程序。
    pub fn watch_of(&self, g: &GameRecord) -> Result<Watch> {
        Ok(Watch {
            game_id: g.id.clone(),
            dir: g
                .install_dir
                .as_deref()
                .filter(|d| !d.is_empty())
                .map(PathBuf::from),
            names: self.index.processes(&g.id)?,
        })
    }

    /// 这些游戏中正在运行的：(游戏 ID, 进程名)。备份前提醒、恢复前阻止用。
    pub fn running_games(&self, ids: &[String]) -> Result<Vec<(String, Vec<String>)>> {
        let mut watches = Vec::new();
        for id in ids {
            watches.push(self.watch_of(&self.find_game(id)?)?);
        }
        Ok(running(&watches))
    }

    /// 游戏库中所有能检测的游戏的检测依据（停用的游戏除外）。
    pub fn all_watches(&self) -> Result<Vec<Watch>> {
        let mut out = Vec::new();
        for g in self.index.games()?.into_iter().filter(|g| g.enabled) {
            let w = self.watch_of(&g)?;
            if !w.is_empty() {
                out.push(w);
            }
        }
        Ok(out)
    }

    /// 游戏库中正在运行的游戏：游戏 ID → 进程名。
    pub fn running_now(&self) -> Result<HashMap<String, Vec<String>>> {
        Ok(running(&self.all_watches()?).into_iter().collect())
    }

    /// 设置游戏程序（运行检测用的进程名）：去掉路径只留文件名，去重，忽略空白。
    pub fn set_game_processes(
        &mut self,
        _lock: &RepoLock,
        game: &GameRecord,
        names: &[String],
    ) -> Result<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        for n in names {
            let name = Path::new(n.trim())
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            if name.contains(['*', '?', '<', '>', '|', '"']) {
                return Err(AppError::Invalid(format!(
                    "进程名「{name}」中有不允许的字符"
                )));
            }
            if !out.iter().any(|o| process_key(o) == process_key(&name)) {
                out.push(name);
            }
        }
        self.index.set_processes(&game.id, &out)?;
        self.save_games_file()?;
        Ok(out)
    }
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

    fn w(id: &str, dir: Option<&str>, names: &[&str]) -> Watch {
        Watch {
            game_id: id.into(),
            dir: dir.map(PathBuf::from),
            names: names.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn matches_by_install_dir_or_process_name() {
        let procs = vec![
            (
                "Game.exe".to_string(),
                Some(PathBuf::from(r"D:\Games\Alpha\bin\Game.exe")),
            ),
            (
                "retroarch.exe".to_string(),
                Some(PathBuf::from(r"C:\Emu\retroarch.exe")),
            ),
            ("explorer.exe".to_string(), None),
        ];
        let found = match_running(
            &[
                w("alpha", Some(r"d:\games\alpha"), &[]),
                w("emu", None, &["RetroArch"]),
                w("beta", Some(r"D:\Games\Beta"), &["beta.exe"]),
            ],
            &procs,
        );
        assert_eq!(
            found,
            vec![
                ("alpha".to_string(), vec!["Game.exe".to_string()]),
                ("emu".to_string(), vec!["retroarch.exe".to_string()]),
            ]
        );
        assert!(running(&[w("x", None, &[])]).is_empty(), "没有检测依据");
    }

    #[test]
    fn set_processes_keeps_file_names_and_dedups() {
        let mut env = crate::testutil::Env::new();
        let g = env.add_game("g");
        let lock = env.app.repo.lock().unwrap();
        let names = env
            .app
            .set_game_processes(
                &lock,
                &g,
                &[
                    r"C:\Emu\RetroArch.exe".into(),
                    "retroarch".into(),
                    " ".into(),
                    "Game.exe".into(),
                ],
            )
            .unwrap();
        assert_eq!(names, ["RetroArch.exe", "Game.exe"]);
        assert_eq!(env.app.watch_of(&g).unwrap().names, names);
        assert!(
            env.app
                .set_game_processes(&lock, &g, &["a*.exe".into()])
                .is_err()
        );
        // 记在 games.toml 中，索引丢失后能恢复。
        drop(lock);
        let db = env.app.repo.root().join("bonfire.db");
        let elsewhere = tempfile::tempdir().unwrap();
        env.app = crate::testutil::open(elsewhere.path());
        for f in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{f}", db.display()));
        }
        env.reopen();
        assert_eq!(env.app.index.processes(&g.id).unwrap(), names);
    }

    #[test]
    #[cfg(windows)]
    fn detects_real_process() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("FakeGame.exe");
        std::fs::copy(r"C:\Windows\System32\ping.exe", &exe).unwrap();
        let mut child = std::process::Command::new(&exe)
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let other = tempfile::tempdir().unwrap();
        let found = running(&[
            w("by-dir", Some(&dir.path().to_string_lossy()), &[]),
            w("by-name", None, &["fakegame"]),
            w(
                "none",
                Some(&other.path().to_string_lossy()),
                &["NoSuchGame.exe"],
            ),
        ]);
        child.kill().unwrap();
        let _ = child.wait();
        let ids: Vec<&str> = found.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["by-dir", "by-name"], "{found:?}");
    }
}
