//! 自动备份（U-9c）：定时备份（每天 / 每周 / 每隔 N 小时）与“存档变化后自动备份”。
//!
//! - 由桌面程序每分钟调用 [`App::auto_backup_due`] 判断是否到期，到期时持锁调用 [`App::run_auto_backup`]；
//!   CLI `bonfire backup --auto` 供系统计划任务调用。
//! - 只备份有变化、且已经备份过（手动确认过存档位置）的游戏；只管理截图的游戏只收集截图。
//! - 变化后备份：存档停止写入 `quiet_minutes` 分钟以上才备份，避免游戏正在写入时备份到一半的存档。
//! - 上次执行的时间记录在缓存目录 `auto/` 中。

use std::path::PathBuf;

use bonfire_core::object::Trigger;
use bonfire_core::{ObjectId, RepoLock};
use serde::Serialize;

use crate::app::App;
use crate::backup::{BackupOptions, BackupResult, DetachedChoice, GameState};
use crate::config::{AutoBackup, ShotSchedule, parse_hm};
use crate::error::Result;

/// 自动备份的方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AutoKind {
    /// 定时备份。
    Scheduled,
    /// 存档变化后自动备份。
    OnChange,
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

    /// 执行一次自动备份。调用方持有仓库锁。
    pub fn run_auto_backup(&mut self, lock: &RepoLock, kind: AutoKind) -> Result<AutoReport> {
        let cfg: AutoBackup = self.config.auto_backup.clone();
        let now_ns = bonfire_core::scan::now_ns();
        let quiet_ns = i64::from(cfg.quiet_minutes) * 60 * 1_000_000_000;
        let mut report = AutoReport {
            kind,
            items: Vec::new(),
            waiting: 0,
            cleaned: 0,
        };
        for g in self.games()? {
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
        self.mark_auto(kind)?;
        if kind == AutoKind::Scheduled {
            // 定时备份同时算作一次检查。
            self.mark_auto(AutoKind::OnChange)?;
        }
        tracing::info!(
            ?kind,
            created = report.created(),
            waiting = report.waiting,
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
}
