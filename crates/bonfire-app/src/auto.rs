//! 自动备份（U-9c、M4-3）：定时备份（每天 / 每周 / 每隔 N 小时）、“存档变化后自动备份”与“退出游戏后自动备份”。
//!
//! - 由桌面程序每分钟调用 [`App::auto_backup_due`] 判断是否到期，到期时持锁调用 [`App::run_auto_backup`]；
//!   CLI `bonfire backup --auto` 供系统计划任务调用。
//! - 只备份有变化、且已经备份过（手动确认过存档位置）的游戏；只管理截图的游戏只收集截图。
//! - 变化后备份：存档停止写入 `quiet_minutes` 分钟以上才备份，避免游戏正在写入时备份到一半的存档。
//! - 游戏运行中不做自动备份（文档 10.1“自动备份在游戏运行中不做重 IO”）：定时与变化后备份跳过正在运行的游戏，
//!   等它退出后再备份；开启“退出游戏后自动备份”时，每个游戏退出后都检查一次。
//!   退出检测由桌面程序每隔几秒调用 [`App::exit_watch_tick`]，游戏退出 [`EXIT_DELAY`] 后备份。
//! - 上次执行的时间记录在缓存目录 `auto/` 中。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use bonfire_core::object::Trigger;
use bonfire_core::{ObjectId, RepoLock};
use serde::Serialize;

use crate::app::App;
use crate::backup::{BackupOptions, BackupResult, DetachedChoice, GameState};
use crate::config::{AutoBackup, ShotSchedule, parse_hm};
use crate::error::{AppError, Result};

/// 自动备份的方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AutoKind {
    /// 定时备份。
    Scheduled,
    /// 存档变化后自动备份。
    OnChange,
    /// 退出游戏后自动备份（含运行中被推迟的定时与变化后备份）。
    OnExit,
}

/// 游戏退出多久后备份（给游戏的收尾写入与云同步留出时间）。
pub const EXIT_DELAY: Duration = Duration::from_secs(10);

/// 退出检测的状态：上一次看到在运行的游戏、已退出等待备份的游戏、运行中被推迟备份的游戏。
#[derive(Debug, Default)]
pub struct ExitWatcher {
    running: HashSet<String>,
    exited: HashMap<String, Instant>,
    deferred: HashSet<String>,
}

impl ExitWatcher {
    /// 运行中被推迟、等退出后备份的游戏。
    pub fn defer(&mut self, ids: impl IntoIterator<Item = String>) {
        self.deferred.extend(ids);
    }

    /// 是否需要检测进程（开启了退出后备份，或有被推迟的游戏）。
    pub fn needed(&self, on_exit: bool) -> bool {
        on_exit || !self.deferred.is_empty()
    }

    /// 记录这一次看到的在运行的游戏，返回已退出满 `delay`、该备份的游戏。
    /// `on_exit` 关闭时只处理被推迟的游戏。
    pub fn observe(
        &mut self,
        now_running: HashSet<String>,
        now: Instant,
        on_exit: bool,
        delay: Duration,
    ) -> Vec<String> {
        for id in self.running.difference(&now_running) {
            if on_exit || self.deferred.contains(id) {
                self.exited.insert(id.clone(), now);
            }
        }
        // 被推迟时在运行、但之后一直没检测到（如程序刚启动）：视为已经退出。
        for id in &self.deferred {
            if !now_running.contains(id) && !self.running.contains(id) {
                self.exited.entry(id.clone()).or_insert(now);
            }
        }
        for id in &now_running {
            self.exited.remove(id);
        }
        self.running = now_running;
        let mut ready: Vec<String> = self
            .exited
            .iter()
            .filter(|(_, t)| now.duration_since(**t) >= delay)
            .map(|(id, _)| id.clone())
            .collect();
        ready.sort();
        for id in &ready {
            self.exited.remove(id);
            self.deferred.remove(id);
        }
        ready
    }
}

/// 一个游戏的自动备份结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoItem {
    pub game_id: String,
    pub name: String,
    pub snapshot: Option<ObjectId>,
    pub new_branch: Option<String>,
    pub error: Option<String>,
    /// 顺带收集的新截图数。
    pub shots: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoReport {
    pub kind: AutoKind,
    /// 只列出备份了、出错或收集到截图的游戏。
    pub items: Vec<AutoItem>,
    /// 有变化但存档还在写入、推迟到下次的游戏数。
    pub waiting: usize,
    /// 正在运行、等退出后再备份的游戏。
    pub deferred: Vec<String>,
    /// 备份后自动清理历史压掉的快照数（设置中开启自动清理时）。
    pub cleaned: usize,
}

impl AutoReport {
    pub fn created(&self) -> usize {
        self.items.iter().filter(|i| i.snapshot.is_some()).count()
    }
}

impl App {
    fn auto_marker(&self, kind: AutoKind) -> PathBuf {
        let name = match kind {
            AutoKind::Scheduled => "last-scheduled",
            AutoKind::OnChange => "last-check",
            AutoKind::OnExit => "last-exit",
        };
        self.paths.cache_dir.join("auto").join(name)
    }

    pub fn last_auto(&self, kind: AutoKind) -> Option<jiff::Timestamp> {
        std::fs::read_to_string(self.auto_marker(kind))
            .ok()
            .and_then(|t| t.trim().parse().ok())
    }

    fn mark_auto(&self, kind: AutoKind) -> Result<()> {
        let p = self.auto_marker(kind);
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| crate::error::AppError::io(dir, e))?;
        }
        std::fs::write(&p, jiff::Timestamp::now().to_string())
            .map_err(|e| crate::error::AppError::io(&p, e))
    }

    /// 现在该执行的自动备份（定时优先）。
    pub fn auto_backup_due(&self, now: &jiff::Zoned) -> Option<AutoKind> {
        let cfg = &self.config.auto_backup;
        if schedule_due(
            cfg.schedule,
            &cfg.at,
            cfg.weekday,
            cfg.every_hours,
            self.last_auto(AutoKind::Scheduled),
            now,
        ) {
            return Some(AutoKind::Scheduled);
        }
        if cfg.on_change {
            let every = i64::from(cfg.check_minutes.max(1)) * 60;
            let due = self
                .last_auto(AutoKind::OnChange)
                .is_none_or(|t| now.timestamp().as_second() - t.as_second() >= every);
            if due {
                return Some(AutoKind::OnChange);
            }
        }
        None
    }

    /// 执行一次自动备份（定时或变化后）。调用方持有仓库锁。正在运行的游戏推迟到退出后。
    pub fn run_auto_backup(&mut self, lock: &RepoLock, kind: AutoKind) -> Result<AutoReport> {
        let running = self.running_now().unwrap_or_default();
        self.run_auto(lock, kind, None, &running)
    }

    /// 退出检测：记录这一次在运行的游戏，返回该做退出后备份的游戏。
    /// `running` 为 [`App::running_now`] 的结果（调用方可以在不占用应用的情况下检测进程）。
    pub fn exit_watch_tick(&mut self, running: HashSet<String>, now: Instant) -> Vec<String> {
        let on_exit = self.config.auto_backup.on_exit;
        self.exit_watch.observe(running, now, on_exit, EXIT_DELAY)
    }

    /// 是否需要检测游戏进程（开启了退出后备份，或有被推迟的游戏）。
    pub fn exit_watch_needed(&self) -> bool {
        self.exit_watch.needed(self.config.auto_backup.on_exit)
    }

    /// 游戏退出后备份这些游戏。调用方持有仓库锁。
    pub fn run_exit_backup(&mut self, lock: &RepoLock, ids: &[String]) -> Result<AutoReport> {
        self.run_auto(lock, AutoKind::OnExit, Some(ids), &HashMap::new())
    }

    fn run_auto(
        &mut self,
        lock: &RepoLock,
        kind: AutoKind,
        only: Option<&[String]>,
        running: &HashMap<String, Vec<String>>,
    ) -> Result<AutoReport> {
        let cfg: AutoBackup = self.config.auto_backup.clone();
        let now_ns = bonfire_core::scan::now_ns();
        let quiet_ns = i64::from(cfg.quiet_minutes) * 60 * 1_000_000_000;
        let mut report = AutoReport {
            kind,
            items: Vec::new(),
            waiting: 0,
            deferred: Vec::new(),
            cleaned: 0,
        };
        for g in self.games()? {
            if only.is_some_and(|ids| !ids.contains(&g.id)) {
                continue;
            }
            if running.contains_key(&g.id) {
                report.deferred.push(g.id.clone());
                continue;
            }
            let mut item = AutoItem {
                game_id: g.id.clone(),
                name: g.name.clone(),
                snapshot: None,
                new_branch: None,
                error: None,
                shots: 0,
            };
            match self.status(&g) {
                Ok(st) if st.state == GameState::HasChanges => {
                    let writing = kind == AutoKind::OnChange
                        && st
                            .last_modified_ns
                            .is_some_and(|m| now_ns.saturating_sub(m) < quiet_ns);
                    if writing {
                        report.waiting += 1;
                    } else {
                        let opts = BackupOptions {
                            trigger: match kind {
                                AutoKind::Scheduled => Trigger::AutoTimer,
                                AutoKind::OnChange => Trigger::AutoChange,
                                AutoKind::OnExit => Trigger::AutoExit,
                            },
                            on_detached: DetachedChoice::NewBranch,
                            ..Default::default()
                        };
                        match self.backup(lock, &g, &opts) {
                            Ok(BackupResult::Created {
                                snapshot,
                                new_branch,
                                ..
                            }) => {
                                item.snapshot = Some(snapshot);
                                item.new_branch = new_branch;
                            }
                            Ok(BackupResult::NoChanges) => {}
                            // 存档被游戏占用：不算出错，退出后（或下次）再备份。
                            Err(AppError::FilesLocked(_)) => report.deferred.push(g.id.clone()),
                            Err(e) => item.error = Some(e.to_string()),
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => item.error = Some(e.to_string()),
            }
            match self.sync_shots(lock, &g) {
                Ok(r) => item.shots = r.added,
                Err(e) => tracing::warn!(game = %g.id, "收集截图失败：{e}"),
            }
            if item.snapshot.is_some() || item.error.is_some() || item.shots > 0 {
                report.items.push(item);
            }
        }
        if report.created() > 0 {
            report.cleaned = self.auto_clean(lock).map_or(0, |r| r.removed);
        }
        self.exit_watch.defer(report.deferred.iter().cloned());
        self.mark_auto(kind)?;
        if kind == AutoKind::Scheduled {
            // 定时备份同时算作一次检查。
            self.mark_auto(AutoKind::OnChange)?;
        }
        tracing::info!(
            ?kind,
            created = report.created(),
            waiting = report.waiting,
            deferred = report.deferred.len(),
            "自动备份完成"
        );
        Ok(report)
    }
}

/// 定时任务是否到期：`last` 为上次执行时间。
/// - 每 N 小时：距上次已满 N 小时（从未执行过时立即执行）；
/// - 每天：今天的设定时刻已过，且上次执行早于这个时刻；
/// - 每周：最近一次“设定的星期 + 时刻”已过，且上次执行早于它。
pub fn schedule_due(
    schedule: ShotSchedule,
    at: &str,
    weekday: u8,
    every_hours: u32,
    last: Option<jiff::Timestamp>,
    now: &jiff::Zoned,
) -> bool {
    let Some((h, m)) = parse_hm(at) else {
        return false;
    };
    let last_s = last.map(|t| t.as_second()).unwrap_or(i64::MIN);
    match schedule {
        ShotSchedule::Off => false,
        ShotSchedule::Hours => match last {
            None => true,
            Some(t) => now.timestamp().as_second() - t.as_second() >= i64::from(every_hours) * 3600,
        },
        ShotSchedule::Daily | ShotSchedule::Weekly => {
            let Ok(mut slot) = now.date().at(h, m, 0, 0).to_zoned(now.time_zone().clone()) else {
                return false;
            };
            if schedule == ShotSchedule::Weekly {
                let want = i16::from(weekday);
                let today = i16::from(now.date().weekday().to_monday_one_offset());
                let back = (today - want).rem_euclid(7);
                slot = match slot.checked_sub(jiff::Span::new().days(back)) {
                    Ok(s) => s,
                    Err(_) => return false,
                };
            }
            if slot.timestamp() > now.timestamp() {
                // 今天（本周）的时刻还没到：看上一个周期。
                let step = if schedule == ShotSchedule::Daily {
                    1
                } else {
                    7
                };
                slot = match slot.checked_sub(jiff::Span::new().days(step)) {
                    Ok(s) => s,
                    Err(_) => return false,
                };
            }
            last_s < slot.timestamp().as_second()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Env;

    #[test]
    fn scheduled_backup_only_touches_changed_games() {
        let mut env = Env::new();
        env.write("a.sav", b"1");
        let g = env.add_game("Game");
        let lock = env.app.repo.lock().unwrap();
        // 从未备份的游戏不自动备份。
        let r = env.app.run_auto_backup(&lock, AutoKind::Scheduled).unwrap();
        assert_eq!(r.created(), 0);
        env.app
            .backup(&lock, &g, &BackupOptions::default())
            .unwrap();
        let r = env.app.run_auto_backup(&lock, AutoKind::Scheduled).unwrap();
        assert_eq!(r.created(), 0, "没有变化时不备份");

        env.write("a.sav", b"2");
        let r = env.app.run_auto_backup(&lock, AutoKind::Scheduled).unwrap();
        assert_eq!(r.created(), 1);
        let head = env.app.repo.head_snapshot(&g.id).unwrap().unwrap();
        assert_eq!(
            env.app.repo.get_snapshot(&head).unwrap().trigger,
            Trigger::AutoTimer
        );
        assert!(env.app.last_auto(AutoKind::Scheduled).is_some());
    }

    #[test]
    fn on_change_waits_until_the_save_is_quiet() {
        let mut env = Env::new();
        env.write("a.sav", b"1");
        let g = env.add_game("Game");
        let lock = env.app.repo.lock().unwrap();
        env.app
            .backup(&lock, &g, &BackupOptions::default())
            .unwrap();
        env.write("a.sav", b"2");
        // 刚刚写入：等待。
        env.app.config.auto_backup.quiet_minutes = 2;
        let r = env.app.run_auto_backup(&lock, AutoKind::OnChange).unwrap();
        assert_eq!((r.created(), r.waiting), (0, 1));
        // 不要求安静时间：备份。
        env.app.config.auto_backup.quiet_minutes = 0;
        let r = env.app.run_auto_backup(&lock, AutoKind::OnChange).unwrap();
        assert_eq!(r.created(), 1);
        let head = env.app.repo.head_snapshot(&g.id).unwrap().unwrap();
        assert_eq!(
            env.app.repo.get_snapshot(&head).unwrap().trigger,
            Trigger::AutoChange
        );
    }

    #[test]
    fn due() {
        let tz = jiff::tz::TimeZone::UTC;
        let at = |s: &str| {
            s.parse::<jiff::civil::DateTime>()
                .unwrap()
                .to_zoned(tz.clone())
                .unwrap()
        };
        let mut env = Env::new();
        let now = at("2026-10-09T22:00");
        assert_eq!(env.app.auto_backup_due(&now), None, "默认都关闭");
        env.app.config.auto_backup.on_change = true;
        assert_eq!(env.app.auto_backup_due(&now), Some(AutoKind::OnChange));
        env.app.config.auto_backup.schedule = ShotSchedule::Daily;
        assert_eq!(env.app.auto_backup_due(&now), Some(AutoKind::Scheduled));
        // 每 N 小时：从未执行过时立即执行。
        assert!(schedule_due(ShotSchedule::Hours, "21:00", 7, 6, None, &now));
        assert!(!schedule_due(ShotSchedule::Off, "21:00", 7, 6, None, &now));
    }

    #[test]
    fn exit_watcher_backs_up_after_exit_and_deferred_games() {
        let t0 = Instant::now();
        let d = Duration::from_secs(10);
        let set = |ids: &[&str]| ids.iter().map(|s| s.to_string()).collect::<HashSet<_>>();
        let mut w = ExitWatcher::default();
        assert!(!w.needed(false));
        assert!(w.observe(set(&["a", "b"]), t0, true, d).is_empty());
        // a 退出：10 秒后才备份；期间重新启动则取消。
        assert!(w.observe(set(&["b"]), t0 + d / 2, true, d).is_empty());
        assert!(w.observe(set(&["a", "b"]), t0 + d, true, d).is_empty());
        assert!(w.observe(set(&["b"]), t0 + d * 2, true, d).is_empty());
        assert_eq!(w.observe(set(&["b"]), t0 + d * 3, true, d), ["a"]);
        assert!(
            w.observe(set(&["b"]), t0 + d * 4, true, d).is_empty(),
            "只备份一次"
        );
        // 关闭退出后备份时，只处理运行中被推迟的游戏。
        w.defer(["b".to_string()]);
        assert!(w.needed(false));
        assert!(w.observe(set(&[]), t0 + d * 5, false, d).is_empty());
        assert_eq!(w.observe(set(&[]), t0 + d * 6, false, d), ["b"]);
        assert!(!w.needed(false));
        // 推迟时检测到在运行、之后再没见到（如刚启动）：视为已退出。
        w.defer(["c".to_string()]);
        assert!(w.observe(set(&[]), t0 + d * 7, false, d).is_empty());
        assert_eq!(w.observe(set(&[]), t0 + d * 8, false, d), ["c"]);
    }

    #[test]
    fn exit_backup_uses_exit_trigger() {
        let mut env = Env::new();
        env.write("a.sav", b"1");
        let g = env.add_game("Game");
        let lock = env.app.repo.lock().unwrap();
        env.app
            .backup(&lock, &g, &BackupOptions::default())
            .unwrap();
        env.write("a.sav", b"after playing");
        let r = env
            .app
            .run_exit_backup(&lock, std::slice::from_ref(&g.id))
            .unwrap();
        assert_eq!(r.created(), 1);
        let head = env.app.repo.head_snapshot(&g.id).unwrap().unwrap();
        assert_eq!(
            env.app.repo.get_snapshot(&head).unwrap().trigger,
            Trigger::AutoExit
        );
    }
}
