//! 恢复出厂设置（0.4.2）：清除本机的篝火设置与数据，下次启动进入首次运行向导。
//!
//! - 默认清除：设置（`config.toml`）、自定义规则、界面状态、本机记住的仓库密码与同步令牌（`keys`）、
//!   缓存（封面、缩略图、同步记录、加密仓库的索引）、操作记录、日志、窗口数据（`webview`）。
//! - 保留：本机标识（`machine-id`，备份中“本机的主存档”靠它识别这台电脑）；可选是否同时删除备份仓库。
//! - 不碰：游戏存档文件、远程同步的备份。
//! - 删除备份仓库时只删除篝火仓库自己的文件（对象、refs、截图等），文件夹中其他文件保留。
//! - 桌面程序运行时部分文件被占用，所以只写下待办标记并重启，在下次启动、打开任何文件之前执行（[`run_pending`]）。
//! - 必须输入确认文字（[`phrase`]）才执行，界面与命令层都会核对。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::{AppPaths, Config, Locale};

/// 待办标记（数据目录中）。
pub const MARKER: &str = "factory-reset.pending";

/// 篝火仓库自己的文件与文件夹（删除仓库时只删这些）。
const REPO_ENTRIES: &[&str] = &[
    "objects",
    "refs",
    "logs",
    "notes",
    "screenshots",
    "albums",
    "config.toml",
    "games.toml",
    "lock",
    "bonfire.db",
    "bonfire.db-wal",
    "bonfire.db-shm",
    crate::sync::SETTINGS_FILE,
];

/// 需要输入的确认文字。
pub fn phrase(locale: Locale, delete_repo: bool) -> &'static str {
    match (locale, delete_repo) {
        (Locale::En, false) => "RESET",
        (Locale::En, true) => "DELETE ALL BACKUPS",
        (_, false) => "恢复出厂设置",
        (_, true) => "删除全部备份",
    }
}

/// 核对确认文字（去掉首尾空白）。
pub fn check_phrase(locale: Locale, delete_repo: bool, typed: &str) -> Result<()> {
    if typed.trim() == phrase(locale, delete_repo) {
        Ok(())
    } else {
        Err(AppError::Invalid(format!(
            "确认文字不对，请输入“{}”",
            phrase(locale, delete_repo)
        )))
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResetItem {
    /// `settings`、`rules`、`ui`、`keys`、`cache`、`activity`、`logs`、`webview`、`repo`。
    pub kind: String,
    pub path: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetPlan {
    pub items: Vec<ResetItem>,
    /// 备份仓库位置（不删除时也显示，说明会保留）。
    pub repo_dir: String,
    pub repo_bytes: u64,
    pub delete_repo: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetReport {
    pub removed: usize,
    /// 没能删除的（路径与原因）。
    pub failed: Vec<(String, String)>,
}

#[derive(Serialize, Deserialize)]
struct Pending {
    delete_repo: bool,
    repo_dir: PathBuf,
}

fn data_dir(paths: &AppPaths) -> PathBuf {
    paths
        .config_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn size(p: &Path) -> u64 {
    if p.is_file() {
        return std::fs::metadata(p).map_or(0, |m| m.len());
    }
    walkdir::WalkDir::new(p)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// 当前配置中的仓库位置（配置读不出时为默认位置）。
fn repo_dir(paths: &AppPaths) -> PathBuf {
    Config::load(&paths.config_file)
        .map(|c| c.repo_dir(&paths.default_repo_dir))
        .unwrap_or_else(|_| paths.default_repo_dir.clone())
}

/// 本机数据（不含备份仓库）。
fn local_items(paths: &AppPaths) -> Vec<(&'static str, PathBuf)> {
    let data = data_dir(paths);
    vec![
        ("settings", paths.config_file.clone()),
        ("rules", data.join("custom-rules.yaml")),
        ("ui", crate::ui_state::ui_state_path(paths)),
        ("keys", data.join("keys")),
        ("cache", paths.cache_dir.clone()),
        ("activity", data.join("activity.jsonl")),
        ("logs", paths.logs_dir.clone()),
        ("webview", paths.webview_dir()),
    ]
}

/// 要清除的内容与大小（界面上列出）。
pub fn plan(paths: &AppPaths, delete_repo: bool) -> ResetPlan {
    let mut items: Vec<ResetItem> = local_items(paths)
        .into_iter()
        .filter(|(_, p)| p.exists())
        .map(|(kind, p)| ResetItem {
            kind: kind.into(),
            bytes: size(&p),
            path: p.to_string_lossy().into_owned(),
        })
        .collect();
    let repo = repo_dir(paths);
    let repo_bytes: u64 = REPO_ENTRIES.iter().map(|e| size(&repo.join(e))).sum();
    if delete_repo {
        items.push(ResetItem {
            kind: "repo".into(),
            path: repo.to_string_lossy().into_owned(),
            bytes: repo_bytes,
        });
    }
    ResetPlan {
        items,
        repo_dir: repo.to_string_lossy().into_owned(),
        repo_bytes,
        delete_repo,
    }
}

/// 写下待办标记，下次启动时执行（桌面程序用）。
pub fn schedule(paths: &AppPaths, delete_repo: bool) -> Result<()> {
    let p = data_dir(paths).join(MARKER);
    let body = serde_json::to_vec(&Pending {
        delete_repo,
        repo_dir: repo_dir(paths),
    })
    .expect("可以序列化");
    std::fs::write(&p, body).map_err(|e| AppError::io(&p, e))
}

/// 启动时执行待办的恢复出厂设置（在打开日志、仓库与窗口之前调用）。没有待办时为 None。
pub fn run_pending(paths: &AppPaths) -> Option<ResetReport> {
    let marker = data_dir(paths).join(MARKER);
    let pending: Pending = serde_json::from_slice(&std::fs::read(&marker).ok()?).ok()?;
    let report = perform_at(paths, pending.delete_repo, &pending.repo_dir);
    let _ = std::fs::remove_file(&marker);
    Some(report)
}

/// 立即清除（命令行用；桌面程序运行时部分文件可能被占用，会列在 `failed` 中）。
pub fn perform(paths: &AppPaths, delete_repo: bool) -> ResetReport {
    perform_at(paths, delete_repo, &repo_dir(paths))
}

fn remove(p: &Path, report: &mut ResetReport) {
    if !p.exists() {
        return;
    }
    let r = if p.is_dir() {
        std::fs::remove_dir_all(p)
    } else {
        std::fs::remove_file(p)
    };
    match r {
        Ok(()) => report.removed += 1,
        Err(e) => report
            .failed
            .push((p.to_string_lossy().into_owned(), e.to_string())),
    }
}

fn perform_at(paths: &AppPaths, delete_repo: bool, repo: &Path) -> ResetReport {
    let mut report = ResetReport::default();
    for (_, p) in local_items(paths) {
        remove(&p, &mut report);
    }
    if delete_repo {
        for e in REPO_ENTRIES {
            remove(&repo.join(e), &mut report);
        }
        // 文件夹空了才删除（里面有别的文件时保留）。
        let _ = std::fs::remove_dir(repo);
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(root: &Path) -> AppPaths {
        let paths = AppPaths::under_root(root, true);
        for (name, body) in [
            ("config.toml", "[general]\n"),
            ("ui-state.json", "{}"),
            ("activity.jsonl", "{}\n"),
            ("custom-rules.yaml", "x: 1\n"),
            ("machine-id", "pc-1"),
        ] {
            std::fs::write(root.join(name), body).unwrap();
        }
        for d in [
            "keys",
            "cache/images",
            "logs",
            "webview",
            "repo/objects/chunks",
            "repo/refs",
            "repo/screenshots/g",
        ] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("keys/sync-1.key"), b"secret").unwrap();
        std::fs::write(root.join("cache/images/a.webp"), b"img").unwrap();
        std::fs::write(root.join("repo/config.toml"), b"repo").unwrap();
        std::fs::write(root.join("repo/objects/chunks/x"), vec![1u8; 100]).unwrap();
        std::fs::write(root.join("repo/screenshots/g/1.png"), b"png").unwrap();
        // 用户放在仓库文件夹中的其他文件：删除仓库时也保留。
        std::fs::write(root.join("repo/我的笔记.txt"), b"note").unwrap();
        paths
    }

    #[test]
    fn phrases_must_match() {
        assert!(check_phrase(Locale::ZhCn, false, " 恢复出厂设置 ").is_ok());
        assert!(check_phrase(Locale::ZhCn, false, "恢复出厂").is_err());
        assert!(
            check_phrase(Locale::ZhCn, true, "恢复出厂设置").is_err(),
            "删除仓库要输入另一句"
        );
        assert!(check_phrase(Locale::ZhCn, true, "删除全部备份").is_ok());
        assert!(check_phrase(Locale::En, false, "RESET").is_ok());
    }

    #[test]
    fn keeps_repo_by_default() {
        let d = tempfile::tempdir().unwrap();
        let paths = setup(d.path());
        let p = plan(&paths, false);
        let kinds: Vec<&str> = p.items.iter().map(|i| i.kind.as_str()).collect();
        assert_eq!(
            kinds,
            [
                "settings", "rules", "ui", "keys", "cache", "activity", "logs", "webview"
            ]
        );
        assert!(p.repo_bytes >= 100);
        let r = perform(&paths, false);
        assert!(r.failed.is_empty());
        for gone in [
            "config.toml",
            "ui-state.json",
            "activity.jsonl",
            "custom-rules.yaml",
            "keys",
            "cache",
            "logs",
            "webview",
        ] {
            assert!(!d.path().join(gone).exists(), "{gone} 已删除");
        }
        assert!(d.path().join("machine-id").exists(), "保留本机标识");
        assert!(
            d.path().join("repo/objects/chunks/x").exists(),
            "保留备份仓库"
        );
    }

    #[test]
    fn deletes_only_repo_files_and_runs_when_pending() {
        let d = tempfile::tempdir().unwrap();
        let paths = setup(d.path());
        assert!(run_pending(&paths).is_none(), "没有待办");
        schedule(&paths, true).unwrap();
        let p = plan(&paths, true);
        assert_eq!(p.items.last().unwrap().kind, "repo");
        let r = run_pending(&paths).unwrap();
        assert!(r.failed.is_empty(), "{:?}", r.failed);
        assert!(!d.path().join(MARKER).exists());
        assert!(
            !d.path().join("repo/objects").exists() && !d.path().join("repo/screenshots").exists()
        );
        assert!(
            d.path().join("repo/我的笔记.txt").exists(),
            "仓库文件夹中别的文件保留"
        );
        assert!(d.path().join("repo").exists(), "文件夹不空时保留");
        assert!(d.path().join("machine-id").exists());
    }
}
