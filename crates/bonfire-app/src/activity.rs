//! 本机操作记录（总览页“操作记录”，决策 OV-2）：`data\activity.jsonl`，每行一条，保留 180 天，不同步。
//!
//! - 备份、恢复、历史（分支 / 标签 / 删除快照）、同步、截图、导入导出、清理与维护都会记一条，失败的也记（带原因）。
//! - 记录在应用服务层完成：本模块包装各项操作（原实现改名为 `*_raw`），命令行与桌面程序都会记录。
//! - 第一次读取时，从仓库的分支记录（reflog）与同步记录补出以前的历史（只补一次，见缓存目录 `activity-backfilled`）。
//! - 写入失败只写日志，不影响操作本身。

use std::io::Write;
use std::path::{Path, PathBuf};

use bonfire_core::index::GameRecord;
use bonfire_core::{ObjectId, RefKind, RepoLock, Trigger};
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::backup::{BackupOptions, BackupResult};
use crate::error::{AppError, Result};

/// 保留天数。
pub const KEEP_DAYS: i64 = 180;

/// 一条操作记录。界面按 `action` 显示（`ui.activity.action.<action>`），参数见各字段。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Activity {
    /// RFC 3339，本地时区。
    pub at: String,
    pub action: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game: Option<String>,
    /// 游戏名或同步目标名。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<String>,
    /// 文件、快照或截图数。
    #[serde(skip_serializing_if = "is_zero")]
    pub n: u64,
    #[serde(skip_serializing_if = "is_zero")]
    pub bytes: u64,
    /// 补充说明：备注、分支名、触发方式、文件名等。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 失败原因分类（如 `locked`、`network`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue: Option<String>,
    /// 失败已处理：之后同一操作成功了，或用户标为已处理（读取时计算）。
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub resolved: bool,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl Activity {
    pub fn new(action: &str) -> Self {
        Self {
            at: crate::util::now_local_rfc3339(),
            action: action.into(),
            ok: true,
            ..Default::default()
        }
    }

    fn game(mut self, g: &GameRecord) -> Self {
        self.game = Some(g.id.clone());
        self.name = g.name.clone();
        self
    }

    fn n(mut self, n: impl TryInto<u64>) -> Self {
        self.n = n.try_into().unwrap_or(0);
        self
    }

    fn bytes(mut self, b: u64) -> Self {
        self.bytes = b;
        self
    }

    fn detail(mut self, d: impl Into<String>) -> Self {
        self.detail = d.into();
        self
    }

    fn failed(mut self, e: &AppError) -> Self {
        self.ok = false;
        self.error = Some(e.to_string());
        self.issue = Some(issue_of(e).into());
        self
    }

    /// 分类：备份、恢复、历史、同步、截图、导入导出、维护。
    pub fn category(&self) -> &'static str {
        category(&self.action)
    }
}

pub fn category(action: &str) -> &'static str {
    match action {
        a if a.starts_with("backup") => "backup",
        "restore" | "undo-restore" => "restore",
        "sync" => "sync",
        a if a.starts_with("shots") => "shots",
        a if a.starts_with("import") || a.starts_with("export") => "transfer",
        a if a.starts_with("branch")
            || a.starts_with("tag")
            || a == "delete-snapshots"
            || a == "primary" =>
        {
            "history"
        }
        _ => "maintain",
    }
}

/// 失败原因分类。
fn issue_of(e: &AppError) -> &'static str {
    match e {
        AppError::FilesLocked(_) => "locked",
        AppError::FileUnstable(_) => "unstable",
        AppError::GameRunning { .. } => "running",
        AppError::Network(_) => "network",
        AppError::Io { .. } | AppError::PathNotFound(_) => "io",
        AppError::RolledBack(_) => "rolled-back",
        _ => "other",
    }
}

/// 查询条件。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ActivityQuery {
    /// 分类（见 [`category`]）；空为全部。
    pub category: Option<String>,
    /// 只看失败的。
    pub failed: bool,
    pub game: Option<String>,
    /// 搜索游戏名、目标名、说明与错误。
    pub text: Option<String>,
    /// 最多返回几条（0 为 200）。
    pub limit: usize,
    /// 只返回早于这个时间的（翻页）。
    pub before: Option<String>,
    /// 只看最近几天（0 为全部保留的记录）。
    pub days: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPage {
    pub items: Vec<Activity>,
    /// 还有更早的记录。
    pub more: bool,
    /// 符合条件（不含失败筛选）中失败的条数。
    pub failed: usize,
}

fn ts(at: &str) -> Option<jiff::Timestamp> {
    at.parse::<jiff::Timestamp>().ok()
}

impl App {
    pub fn activity_file(&self) -> PathBuf {
        self.paths
            .logs_dir
            .parent()
            .map(|p| p.join("activity.jsonl"))
            .unwrap_or_else(|| PathBuf::from("activity.jsonl"))
    }

    fn backfill_marker(&self) -> PathBuf {
        self.paths.cache_dir.join("activity-backfilled")
    }

    /// 记一条操作。
    pub fn note(&self, a: Activity) {
        if let Err(e) = append(&self.activity_file(), std::slice::from_ref(&a)) {
            tracing::warn!("写入操作记录失败：{e}");
        }
    }

    /// 读取操作记录（新的在前）。第一次读取时补出以前的历史，并去掉超过保留天数的记录。
    pub fn activities(&self, q: &ActivityQuery) -> Result<ActivityPage> {
        let path = self.activity_file();
        let mut all = read_all(&path);
        let cutoff = jiff::Timestamp::now() - jiff::SignedDuration::from_hours(24 * KEEP_DAYS);
        let before = all.len();
        all.retain(|a| ts(&a.at).is_none_or(|t| t >= cutoff));
        if all.len() != before {
            let _ = rewrite(&path, &all);
        }
        if !self.backfill_marker().exists() {
            self.backfill_activity();
            all = read_all(&path);
        }
        // 同一秒内的按写入顺序：先倒序，再按时间稳定排序。
        all.reverse();
        all.sort_by_key(|a| std::cmp::Reverse(ts(&a.at)));
        self.mark_resolved(&mut all);
        let text = q
            .text
            .as_deref()
            .map(str::to_lowercase)
            .filter(|t| !t.trim().is_empty());
        let before_ts = q.before.as_deref().and_then(ts);
        let since = (q.days > 0).then(|| {
            jiff::Timestamp::now() - jiff::SignedDuration::from_hours(24 * i64::from(q.days))
        });
        let matched: Vec<Activity> = all
            .into_iter()
            .filter(|a| {
                q.category
                    .as_deref()
                    .is_none_or(|c| c.is_empty() || a.category() == c)
            })
            .filter(|a| {
                q.game
                    .as_deref()
                    .is_none_or(|g| a.game.as_deref() == Some(g))
            })
            .filter(|a| {
                text.as_deref().is_none_or(|t| {
                    [
                        a.name.as_str(),
                        a.detail.as_str(),
                        a.error.as_deref().unwrap_or(""),
                    ]
                    .iter()
                    .any(|s| s.to_lowercase().contains(t))
                })
            })
            .filter(|a| before_ts.is_none_or(|b| ts(&a.at).is_some_and(|t| t < b)))
            .filter(|a| since.is_none_or(|s| ts(&a.at).is_some_and(|t| t >= s)))
            .collect();
        let failed = matched.iter().filter(|a| !a.ok && !a.resolved).count();
        let limit = if q.limit == 0 { 200 } else { q.limit };
        let mut items: Vec<Activity> = matched
            .into_iter()
            .filter(|a| !q.failed || (!a.ok && !a.resolved))
            .collect();
        let more = items.len() > limit;
        items.truncate(limit);
        Ok(ActivityPage {
            items,
            more,
            failed,
        })
    }

    /// 失败记录的处理状态：之后同一操作（同一游戏或目标）成功了，或用户标为已处理。
    /// `all` 为新的在前。
    fn mark_resolved(&self, all: &mut [Activity]) {
        let manual = self.resolved_ids();
        let mut later_ok = std::collections::HashSet::new();
        for a in all.iter_mut() {
            let key = resolve_key(a);
            if a.ok {
                later_ok.insert(key);
            } else {
                a.resolved = later_ok.contains(&key) || manual.contains(&activity_id(a));
            }
        }
    }

    fn resolved_ids(&self) -> std::collections::HashSet<String> {
        crate::ui_state::load_ui_state(&crate::ui_state::ui_state_path(&self.paths))
            .get(RESOLVED_KEY)
            .and_then(|v| serde_json::from_str(v).ok())
            .unwrap_or_default()
    }

    /// 把一条失败记录标为已处理（`id` 见 [`activity_id`]）。
    pub fn resolve_activity(&self, id: &str) -> Result<()> {
        let mut ids = self.resolved_ids();
        ids.insert(id.to_string());
        // 只保留还在保留期内的记录对应的 ID。
        let live: std::collections::HashSet<String> = read_all(&self.activity_file())
            .iter()
            .map(activity_id)
            .collect();
        ids.retain(|i| live.contains(i));
        crate::ui_state::set_ui_state(
            &crate::ui_state::ui_state_path(&self.paths),
            RESOLVED_KEY,
            Some(serde_json::to_string(&ids).expect("可以序列化")),
        )
    }

    /// 还没解决的失败：每个游戏最近一次备份失败（之后没有成功的备份）、最近一次校验发现问题。
    /// 同步失败看同步目标的状态，不在这里。
    pub fn open_failures(&self) -> Result<Vec<Activity>> {
        let all = self
            .activities(&ActivityQuery {
                limit: usize::MAX,
                ..Default::default()
            })?
            .items;
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for a in all {
            let key = match a.category() {
                "backup" => format!("backup:{}", a.game.clone().unwrap_or_default()),
                _ if a.action == "fsck" => "fsck".to_string(),
                _ => continue,
            };
            // 新的在前：每类只看最近一条。
            if seen.insert(key) && !a.ok && !a.resolved {
                out.push(a);
            }
        }
        Ok(out)
    }

    /// 从分支记录与同步记录补出以前的历史（只补一次；加密仓库未解锁时稍后再补）。
    fn backfill_activity(&self) {
        let marker = self.backfill_marker();
        if marker.exists() || !self.repo.is_unlocked() {
            return;
        }
        let names: std::collections::HashMap<String, String> = self
            .all_games()
            .unwrap_or_default()
            .into_iter()
            .map(|g| (g.id, g.name))
            .collect();
        let mut out = Vec::new();
        for game in self.repo.games().unwrap_or_default() {
            for e in self.repo.reflog(&game).unwrap_or_default() {
                let deleting = e.new.is_none();
                let action = match e.action.as_str() {
                    "backup" => "backup",
                    "restore" => "restore",
                    "undo-restore" => "undo-restore",
                    "import" => "import",
                    "delete-snapshot" => "delete-snapshots",
                    "rename" => "branch-rename",
                    "primary" => "primary",
                    "tag" if deleting => "tag-delete",
                    "tag" => "tag-create",
                    "branch" if deleting => "branch-delete",
                    "branch" => "branch-create",
                    "delete" if e.reference.starts_with("tags/") => "tag-delete",
                    "delete" => "branch-delete",
                    _ => continue,
                };
                let at = e
                    .time
                    .to_zoned(jiff::tz::TimeZone::system())
                    .strftime("%Y-%m-%dT%H:%M:%S%:z")
                    .to_string();
                // 分支、标签：说明为名称；其余为当时的说明（备份为备注）。
                let short = e
                    .reference
                    .trim_start_matches("heads/")
                    .trim_start_matches("tags/")
                    .to_string();
                let detail = if action.starts_with("branch-create")
                    || action.starts_with("branch-delete")
                    || action.starts_with("tag-")
                {
                    short
                } else {
                    e.message
                };
                out.push(Activity {
                    at,
                    action: action.into(),
                    ok: true,
                    // 已移出游戏库的游戏没有名称，界面显示为“已移除的游戏”。
                    name: names.get(&game).cloned().unwrap_or_default(),
                    game: Some(game.clone()),
                    snapshot: e.new.map(|id| id.to_hex()),
                    detail,
                    ..Default::default()
                });
            }
        }
        for t in &self.config.sync.targets {
            for r in self.sync_state(&t.id).runs {
                out.push(Activity {
                    at: r.at,
                    action: "sync".into(),
                    ok: r.error.is_none(),
                    name: t.name.clone(),
                    n: r.uploaded as u64,
                    bytes: r.uploaded_bytes,
                    error: r.error,
                    issue: r.issue.map(|i| issue_key(i).into()),
                    ..Default::default()
                });
            }
        }
        let path = self.activity_file();
        let mut all = read_all(&path);
        // 只补已有记录之前的历史（之后的已经实时记录）。
        if let Some(first) = all.iter().filter_map(|a| ts(&a.at)).min() {
            out.retain(|a| ts(&a.at).is_some_and(|t| t < first));
        }
        all.extend(out);
        all.sort_by_key(|a| ts(&a.at));
        all.dedup();
        if rewrite(&path, &all).is_ok() {
            if let Some(d) = marker.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(&marker, b"");
        }
    }
}

/// 标为已处理的记录在界面状态中的键：值为 JSON 数组（记录 ID）。
pub const RESOLVED_KEY: &str = "activity.resolved";

/// 记录的 ID：时间、操作与名称。
pub fn activity_id(a: &Activity) -> String {
    format!("{}|{}|{}", a.at, a.action, a.name)
}

/// 判断“之后成功了”用的键：同一类操作、同一游戏或目标。
fn resolve_key(a: &Activity) -> String {
    let action = if a.action.starts_with("backup") {
        "backup"
    } else {
        a.action.as_str()
    };
    format!("{action}|{}|{}", a.game.as_deref().unwrap_or(""), a.name)
}

pub(crate) fn issue_key(i: crate::sync::SyncIssue) -> &'static str {
    use crate::sync::SyncIssue as S;
    match i {
        S::Network => "network",
        S::Auth => "auth",
        S::NotFound => "not-found",
        S::Public => "public",
        S::Size => "size",
        S::FileLimit => "file-limit",
        S::Local => "local",
        S::Excluded => "excluded",
        S::Other => "other",
    }
}

fn read_all(path: &Path) -> Vec<Activity> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn append(path: &Path, items: &[Activity]) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut buf = Vec::new();
    for a in items {
        serde_json::to_writer(&mut buf, a).expect("可以序列化");
        buf.push(b'\n');
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(&buf)
}

fn rewrite(path: &Path, items: &[Activity]) -> std::io::Result<()> {
    let tmp = path.with_extension("jsonl.tmp");
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let _ = std::fs::remove_file(&tmp);
    append(&tmp, items)?;
    std::fs::rename(&tmp, path)
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn trigger_key(t: Trigger) -> &'static str {
    match t {
        Trigger::Manual => "manual",
        Trigger::AutoExit => "auto-exit",
        Trigger::AutoTimer => "auto-timer",
        Trigger::AutoChange => "auto-change",
        Trigger::PreRestore => "pre-restore",
        Trigger::Import => "import",
    }
}

// ---------- 包装各项操作：调用原实现后记一条 ----------

impl App {
    /// 备份一个游戏。调用方持有仓库锁。恢复前的保护快照与导入不单独记录。
    pub fn backup(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        opts: &BackupOptions,
    ) -> Result<BackupResult> {
        let r = self.backup_raw(lock, game, opts);
        if matches!(opts.trigger, Trigger::PreRestore | Trigger::Import) {
            return r;
        }
        let action = if opts.trigger == Trigger::Manual {
            "backup"
        } else {
            "backup-auto"
        };
        let base = Activity::new(action).game(game);
        match &r {
            Ok(BackupResult::Created {
                snapshot,
                branch,
                comment,
                summary,
                ..
            }) => {
                let mut a = base
                    .n(summary.added + summary.modified + summary.deleted)
                    .bytes(summary.changed_bytes)
                    .detail(if opts.trigger == Trigger::Manual {
                        comment.clone()
                    } else {
                        trigger_key(opts.trigger).to_string()
                    });
                a.snapshot = Some(snapshot.to_hex());
                if branch != "main" {
                    a.detail = format!("{}|{branch}", a.detail);
                }
                self.note(a);
            }
            Ok(BackupResult::NoChanges) => {}
            Err(e) => self.note(base.detail(trigger_key(opts.trigger)).failed(e)),
        }
        r
    }

    pub fn restore(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        target_spec: &str,
        opts: &crate::restore::RestoreOptions,
    ) -> Result<crate::restore::RestoreReport> {
        let r = self.restore_raw(lock, game, target_spec, opts);
        match &r {
            Ok(rep) if !rep.dry_run => {
                let mut a = Activity::new("restore").game(game).n(rep.plan.files.len());
                a.snapshot = Some(rep.plan.target.to_hex());
                self.note(a);
            }
            Ok(_) => {}
            Err(e) => self.note(
                Activity::new("restore")
                    .game(game)
                    .detail(target_spec)
                    .failed(e),
            ),
        }
        r
    }

    pub fn undo_restore(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
    ) -> Result<crate::restore::RestoreReport> {
        let r = self.undo_restore_raw(lock, game);
        match &r {
            Ok(rep) => {
                let mut a = Activity::new("undo-restore")
                    .game(game)
                    .n(rep.plan.files.len());
                a.snapshot = Some(rep.plan.target.to_hex());
                self.note(a);
            }
            Err(e) => self.note(Activity::new("undo-restore").game(game).failed(e)),
        }
        r
    }

    pub fn delete_snapshots(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        specs: &[String],
        dry_run: bool,
    ) -> Result<crate::bulk_delete::BulkDeleteReport> {
        let r = self.delete_snapshots_raw(lock, game, specs, dry_run);
        if let Ok(rep) = &r
            && !dry_run
            && !rep.removed.is_empty()
        {
            self.note(
                Activity::new("delete-snapshots")
                    .game(game)
                    .n(rep.removed.len())
                    .bytes(rep.bytes),
            );
        }
        r
    }

    pub fn delete_branch(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        name: &str,
        dry_run: bool,
    ) -> Result<crate::prune::DeleteReport> {
        let r = self.delete_branch_raw(lock, game, name, dry_run);
        if let Ok(rep) = &r
            && !dry_run
        {
            self.note(
                Activity::new("branch-delete")
                    .game(game)
                    .n(rep.removed.len())
                    .detail(name),
            );
        }
        r
    }

    pub fn delete_tag(&mut self, lock: &RepoLock, game: &GameRecord, name: &str) -> Result<()> {
        let r = self.delete_tag_raw(lock, game, name);
        if r.is_ok() {
            self.note(Activity::new("tag-delete").game(game).detail(name));
        }
        r
    }

    pub fn rename_branch(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        from: &str,
        to: &str,
    ) -> Result<()> {
        let r = self.rename_branch_raw(lock, game, from, to);
        if r.is_ok() {
            self.note(
                Activity::new("branch-rename")
                    .game(game)
                    .detail(format!("{from}|{to}")),
            );
        }
        r
    }

    pub fn create_ref(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        kind: RefKind,
        name: &str,
        spec: Option<&str>,
    ) -> Result<ObjectId> {
        let r = self.create_ref_raw(lock, game, kind, name, spec);
        if let Ok(id) = &r {
            let action = if kind == RefKind::Tag {
                "tag-create"
            } else {
                "branch-create"
            };
            let mut a = Activity::new(action).game(game).detail(name);
            a.snapshot = Some(id.to_hex());
            self.note(a);
        }
        r
    }

    pub fn fsck(&self, deep: bool) -> Result<bonfire_core::fsck::FsckReport> {
        let r = self.fsck_raw(deep);
        match &r {
            Ok(rep) => {
                let mut a = Activity::new("fsck").n(rep.errors.len());
                a.ok = rep.errors.is_empty();
                if !a.ok {
                    a.issue = Some("damaged".into());
                    a.error = rep.errors.first().cloned();
                }
                self.note(a.detail(if deep { "deep" } else { "" }));
            }
            Err(e) => self.note(Activity::new("fsck").failed(e)),
        }
        r
    }

    pub fn clean_history(
        &mut self,
        lock: &RepoLock,
        opts: &crate::retention::CleanupOptions,
    ) -> Result<crate::retention::CleanupReport> {
        let r = self.clean_history_raw(lock, opts);
        match &r {
            Ok(rep) if !rep.dry_run && (rep.removed > 0 || rep.gc.bytes > 0) => {
                self.note(Activity::new("clean").n(rep.removed).bytes(rep.gc.bytes))
            }
            Ok(_) => {}
            Err(e) => self.note(Activity::new("clean").failed(e)),
        }
        r
    }

    pub fn sync_shots(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
    ) -> Result<crate::shots::ShotSyncReport> {
        let r = self.sync_shots_raw(lock, game);
        match &r {
            Ok(rep) if rep.added > 0 => self.note(
                Activity::new("shots-collect")
                    .game(game)
                    .n(rep.added)
                    .bytes(rep.bytes),
            ),
            Ok(_) => {}
            Err(e) => self.note(Activity::new("shots-collect").game(game).failed(e)),
        }
        r
    }

    pub fn write_back_shots(
        &self,
        game: &GameRecord,
        blobs: &[ObjectId],
        dest: Option<&Path>,
    ) -> Result<crate::gallery::WriteBackReport> {
        let r = self.write_back_shots_raw(game, blobs, dest);
        match &r {
            Ok(rep) if rep.written > 0 => self.note(
                Activity::new("shots-write-back")
                    .game(game)
                    .n(rep.written)
                    .bytes(rep.bytes),
            ),
            Ok(_) => {}
            Err(e) => self.note(Activity::new("shots-write-back").game(game).failed(e)),
        }
        r
    }

    pub fn sync_shots_to_disk(
        &mut self,
        game: &GameRecord,
        blobs: &[ObjectId],
    ) -> Result<crate::gallery::WriteBackReport> {
        let r = self.sync_shots_to_disk_raw(game, blobs);
        if let Ok(rep) = &r
            && rep.written > 0
        {
            self.note(
                Activity::new("shots-write-back")
                    .game(game)
                    .n(rep.written)
                    .bytes(rep.bytes),
            );
        }
        r
    }

    pub fn delete_shots(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        blobs: &[ObjectId],
    ) -> Result<usize> {
        let r = self.delete_shots_raw(lock, game, blobs);
        if let Ok(n) = &r
            && *n > 0
        {
            self.note(Activity::new("shots-delete").game(game).n(*n));
        }
        r
    }

    pub fn purge_shots_with(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        blobs: &[ObjectId],
        originals: bool,
    ) -> Result<crate::shots::PurgeReport> {
        let r = self.purge_shots_with_raw(lock, game, blobs, originals);
        if let Ok(rep) = &r
            && rep.purged > 0
        {
            self.note(Activity::new("shots-purge").game(game).n(rep.purged));
        }
        r
    }

    pub fn import_bundle(
        &mut self,
        lock: &RepoLock,
        path: &Path,
        target: Option<&str>,
        dry_run: bool,
    ) -> Result<crate::transfer::BundleImportReport> {
        let r = self.import_bundle_raw(lock, path, target, dry_run);
        if dry_run {
            return r;
        }
        match &r {
            Ok(rep) => {
                let mut a = Activity::new("import-bundle")
                    .n(rep.imported_snapshots)
                    .detail(file_name(path));
                a.name = rep.preview.game_name.clone();
                a.game = Some(rep.preview.game_id.clone());
                self.note(a);
            }
            Err(e) => self.note(
                Activity::new("import-bundle")
                    .detail(file_name(path))
                    .failed(e),
            ),
        }
        r
    }

    pub fn import_files(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        src: &Path,
        opts: &crate::transfer::ImportFilesOptions,
    ) -> Result<crate::transfer::ImportFilesReport> {
        let r = self.import_files_raw(lock, game, src, opts);
        match &r {
            Ok(rep) if !rep.dry_run && rep.snapshot.is_some() => {
                let mut a = Activity::new("import-files")
                    .game(game)
                    .detail(file_name(src));
                a.snapshot = rep.snapshot.map(|s| s.to_hex());
                self.note(a);
            }
            Ok(_) => {}
            Err(e) => self.note(
                Activity::new("import-files")
                    .game(game)
                    .detail(file_name(src))
                    .failed(e),
            ),
        }
        r
    }

    pub fn import_download(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        src: &Path,
        password: Option<&str>,
        req: &crate::downloads::DownloadImport,
    ) -> Result<crate::downloads::DownloadReport> {
        let r = self.import_download_raw(lock, game, src, password, req);
        match &r {
            Ok(rep) if rep.snapshot.is_some() => {
                let mut a = Activity::new("import-download")
                    .game(game)
                    .n(rep.files)
                    .detail(file_name(src));
                a.snapshot = rep.snapshot.map(|s| s.to_hex());
                self.note(a);
            }
            Ok(_) => {}
            Err(e) => self.note(
                Activity::new("import-download")
                    .game(game)
                    .detail(file_name(src))
                    .failed(e),
            ),
        }
        r
    }

    pub fn export_files(
        &self,
        game: &GameRecord,
        spec: Option<&str>,
        dest: &Path,
        format: crate::transfer::ExportFormat,
    ) -> Result<crate::transfer::ExportReport> {
        let r = self.export_files_raw(game, spec, dest, format);
        match &r {
            Ok(rep) => self.note(
                Activity::new("export-files")
                    .game(game)
                    .n(rep.files)
                    .bytes(rep.bytes)
                    .detail(rep.dest.to_string_lossy()),
            ),
            Err(e) => self.note(Activity::new("export-files").game(game).failed(e)),
        }
        r
    }

    pub fn export_bundle(
        &self,
        game: &GameRecord,
        scope: &crate::transfer::BundleScope,
        dest: &Path,
        include_shots: bool,
    ) -> Result<crate::transfer::BundleReport> {
        let r = self.export_bundle_raw(game, scope, dest, include_shots);
        match &r {
            Ok(rep) => self.note(
                Activity::new("export-bundle")
                    .game(game)
                    .n(rep.snapshots)
                    .bytes(rep.bytes)
                    .detail(rep.dest.to_string_lossy()),
            ),
            Err(e) => self.note(Activity::new("export-bundle").game(game).failed(e)),
        }
        r
    }

    pub fn export_package(
        &self,
        games: &[GameRecord],
        dest: &Path,
        opts: &crate::transfer::PackageOptions,
    ) -> Result<crate::transfer::CollectionReport> {
        let r = self.export_package_raw(games, dest, opts);
        self.note_package(&r, games.len());
        r
    }

    pub fn export_package_zip(
        &self,
        games: &[GameRecord],
        dest: &Path,
        opts: &crate::transfer::PackageOptions,
    ) -> Result<crate::transfer::CollectionReport> {
        let r = self.export_package_zip_raw(games, dest, opts);
        self.note_package(&r, games.len());
        r
    }

    fn note_package(&self, r: &Result<crate::transfer::CollectionReport>, n: usize) {
        match r {
            Ok(rep) => self.note(
                Activity::new("export-package")
                    .n(rep.games.len())
                    .bytes(rep.bytes)
                    .detail(rep.dest.to_string_lossy()),
            ),
            Err(e) => self.note(Activity::new("export-package").n(n).failed(e)),
        }
    }

    pub fn export_shots(
        &self,
        game: &GameRecord,
        blobs: &[ObjectId],
        dest: &Path,
        format: crate::transfer::ExportFormat,
    ) -> Result<crate::shots::ShotExportReport> {
        let r = self.export_shots_raw(game, blobs, dest, format);
        match &r {
            Ok(rep) => self.note(
                Activity::new("export-shots")
                    .game(game)
                    .n(rep.files)
                    .bytes(rep.bytes)
                    .detail(rep.dest.to_string_lossy()),
            ),
            Err(e) => self.note(Activity::new("export-shots").game(game).failed(e)),
        }
        r
    }

    pub fn scan_with(
        &mut self,
        lock: &RepoLock,
        adapters: &[Box<dyn bonfire_discovery::PlatformAdapter>],
    ) -> Result<crate::discover::ScanReport> {
        let r = self.scan_with_raw(lock, adapters);
        match &r {
            Ok(rep) => self.note(
                Activity::new("scan")
                    .n(rep.games.len())
                    .detail(rep.platforms.join(",")),
            ),
            Err(e) => self.note(Activity::new("scan").failed(e)),
        }
        r
    }

    pub fn update_rules(
        &self,
        url: &str,
        bundle_to: Option<&Path>,
    ) -> Result<bonfire_discovery::manifest::UpdateOutcome> {
        let r = self.update_rules_raw(url, bundle_to);
        match &r {
            Ok(bonfire_discovery::manifest::UpdateOutcome::Updated { games, bytes }) => {
                self.note(Activity::new("rules-update").n(*games).bytes(*bytes))
            }
            Ok(_) => {}
            Err(e) => self.note(Activity::new("rules-update").failed(e)),
        }
        r
    }

    pub fn encrypt_repo(&mut self, lock: &RepoLock, password: &str, remember: bool) -> Result<()> {
        let r = self.encrypt_repo_raw(lock, password, remember);
        if r.is_ok() {
            self.note(Activity::new("encrypt"));
        }
        r
    }

    pub fn decrypt_repo(&mut self, lock: &RepoLock, password: &str) -> Result<()> {
        let r = self.decrypt_repo_raw(lock, password);
        if r.is_ok() {
            self.note(Activity::new("decrypt"));
        }
        r
    }

    pub fn change_password(&mut self, lock: &RepoLock, old: &str, new: &str) -> Result<()> {
        let r = self.change_password_raw(lock, old, new);
        if r.is_ok() {
            self.note(Activity::new("password"));
        }
        r
    }

    pub fn reindex(&mut self, lock: &RepoLock) -> Result<bonfire_core::index::ReindexStats> {
        let r = self.reindex_raw(lock);
        if let Ok(s) = &r {
            self.note(Activity::new("reindex").n(s.snapshots));
        }
        r
    }

    /// 同步一个目标后记一条（由同步服务调用）。
    pub(crate) fn note_sync(&self, t: &crate::sync::SyncTarget, run: &crate::sync::SyncRun) {
        self.note(Activity {
            at: run.at.clone(),
            action: "sync".into(),
            ok: run.error.is_none(),
            name: t.name.clone(),
            n: run.uploaded as u64,
            bytes: run.uploaded_bytes,
            error: run.error.clone(),
            issue: run.issue.map(|i| issue_key(i).into()),
            ..Default::default()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Env;

    fn q() -> ActivityQuery {
        ActivityQuery::default()
    }

    #[test]
    fn records_operations_and_filters() {
        let mut env = Env::new();
        env.write("a.sav", b"one");
        let g = env.add_game("空洞骑士");
        let lock = env.app.repo.lock().unwrap();
        let opts = BackupOptions {
            comment: Some("打 Boss 前".into()),
            ..Default::default()
        };
        env.app.backup(&lock, &g, &opts).unwrap();
        env.write("a.sav", b"two");
        let auto = BackupOptions {
            trigger: Trigger::AutoChange,
            ..Default::default()
        };
        env.app.backup(&lock, &g, &auto).unwrap();
        // 没有变化不记录。
        env.app.backup(&lock, &g, &auto).unwrap();
        env.app
            .create_ref(&lock, &g, RefKind::Tag, "通关", None)
            .unwrap();
        let restore = crate::restore::RestoreOptions::default();
        assert!(
            env.app
                .restore(&lock, &g, "不存在的快照", &restore)
                .is_err()
        );
        drop(lock);

        let page = env.app.activities(&q()).unwrap();
        let actions: Vec<&str> = page.items.iter().map(|a| a.action.as_str()).collect();
        assert_eq!(
            actions,
            ["restore", "tag-create", "backup-auto", "backup"],
            "新的在前"
        );
        let manual = &page.items[3];
        assert_eq!(
            (manual.name.as_str(), manual.detail.as_str()),
            ("空洞骑士", "打 Boss 前")
        );
        assert!(manual.n > 0 && manual.snapshot.is_some());
        assert_eq!(page.items[2].detail, "auto-change");
        assert!(!page.items[0].ok && page.items[0].error.is_some());
        assert!(!page.items[0].resolved, "恢复失败之后没有成功的恢复");
        assert_eq!(page.failed, 1);

        let backups = env
            .app
            .activities(&ActivityQuery {
                category: Some("backup".into()),
                ..q()
            })
            .unwrap();
        assert_eq!(backups.items.len(), 2);
        let failed = env
            .app
            .activities(&ActivityQuery {
                failed: true,
                ..q()
            })
            .unwrap();
        assert_eq!(failed.items.len(), 1);
        let found = env
            .app
            .activities(&ActivityQuery {
                text: Some("boss".into()),
                ..q()
            })
            .unwrap();
        assert_eq!(found.items.len(), 1, "搜索备注，不区分大小写");
        let paged = env
            .app
            .activities(&ActivityQuery { limit: 3, ..q() })
            .unwrap();
        assert!(paged.more && paged.items.len() == 3);
        let older = env
            .app
            .activities(&ActivityQuery {
                before: Some(paged.items[1].at.clone()),
                ..q()
            })
            .unwrap();
        assert!(
            older
                .items
                .iter()
                .all(|a| ts(&a.at) < ts(&paged.items[1].at))
        );
        // 备份失败后又备份成功：失败记录显示为已处理。
        let mut fail = Activity::new("backup-auto");
        fail.game = Some(g.id.clone());
        fail.name = g.name.clone();
        fail.ok = false;
        env.app.note(fail);
        env.write("a.sav", b"three");
        let lock = env.app.repo.lock().unwrap();
        env.app
            .backup(&lock, &g, &BackupOptions::default())
            .unwrap();
        drop(lock);
        let page = env.app.activities(&q()).unwrap();
        let backup_fail = page
            .items
            .iter()
            .find(|a| a.action == "backup-auto" && !a.ok)
            .unwrap();
        assert!(backup_fail.resolved);
        // “只看失败的”不含已处理的。
        let failed = env
            .app
            .activities(&ActivityQuery {
                failed: true,
                ..q()
            })
            .unwrap();
        assert!(failed.items.iter().all(|a| !a.resolved));
        // 只看最近几天：10 天前的一条不在最近 7 天中。
        let mut old = Activity::new("scan");
        old.at = (jiff::Zoned::now() - jiff::SignedDuration::from_hours(240))
            .strftime("%Y-%m-%dT%H:%M:%S%:z")
            .to_string();
        env.app.note(old);
        let week = env
            .app
            .activities(&ActivityQuery { days: 7, ..q() })
            .unwrap();
        assert!(week.items.iter().all(|a| a.action != "scan"));
        let all = env
            .app
            .activities(&ActivityQuery { days: 0, ..q() })
            .unwrap();
        assert!(all.items.iter().any(|a| a.action == "scan"));
    }

    #[test]
    fn backfills_from_reflog_once_and_prunes_old() {
        let mut env = Env::new();
        env.write("a.sav", b"one");
        let g = env.add_game("Hades");
        let lock = env.app.repo.lock().unwrap();
        env.app
            .backup(&lock, &g, &BackupOptions::default())
            .unwrap();
        drop(lock);
        // 模拟升级前：没有操作记录，只有分支记录。
        std::fs::remove_file(env.app.activity_file()).unwrap();
        // 一条超过保留天数的旧记录。
        let old = Activity {
            at: "2020-01-01T00:00:00+08:00".into(),
            ..Activity::new("scan")
        };
        append(&env.app.activity_file(), &[old]).unwrap();
        let page = env.app.activities(&q()).unwrap();
        assert_eq!(page.items.len(), 1, "补出备份、去掉过期记录");
        assert_eq!(
            (page.items[0].action.as_str(), page.items[0].name.as_str()),
            ("backup", "Hades")
        );
        // 只补一次。
        let again = env.app.activities(&q()).unwrap();
        assert_eq!(again.items.len(), 1);
        assert_eq!(category("shots-collect"), "shots");
        assert_eq!(category("tag-delete"), "history");
        assert_eq!(category("import-bundle"), "transfer");
        assert_eq!(category("rules-update"), "maintain");
        // 最近一次备份成功：没有未解决的失败。
        assert!(env.app.open_failures().unwrap().is_empty());
        let mut fail = Activity::new("backup-auto");
        fail.game = Some(g.id.clone());
        fail.ok = false;
        fail.issue = Some("locked".into());
        env.app.note(fail);
        let open = env.app.open_failures().unwrap();
        assert_eq!((open.len(), open[0].issue.as_deref()), (1, Some("locked")));
        // 手动标为已处理：不再是未解决的失败，记录显示为已处理。
        env.app.resolve_activity(&activity_id(&open[0])).unwrap();
        assert!(env.app.open_failures().unwrap().is_empty());
        let page = env.app.activities(&q()).unwrap();
        assert!(page.items.iter().any(|a| !a.ok && a.resolved));
        assert_eq!(page.failed, 0);
    }
}
