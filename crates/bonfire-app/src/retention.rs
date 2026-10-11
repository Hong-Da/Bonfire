//! 历史清理（文档 F9、6.6，M4-4）：按保留策略压掉多余的快照，再做垃圾回收释放空间。
//!
//! - 保留策略（设置 `[retention]`）：最近 `keep_last` 个、最近 `keep_daily` 天每天最新的 1 个、
//!   最近 `keep_weekly` 周每周最新的 1 个；只在“可清理”的快照中计算——默认只有自动生成的快照
//!   （定时、存档变化后、退出游戏后、恢复前保护）可清理，`scope = "all"` 时所有快照都可清理，
//!   但存档安全保护开启时本机手动备份的快照仍保留。
//! - 永远保留：带标签的、各分支最新的、当前所在的快照，以及可撤销的恢复用到的快照。
//! - 压掉：子快照改接到被删快照的父快照上（得到新 ID），分支、标签、备注、HEAD 与恢复记录随之更新；
//!   多个快照一起删除时一次改写，每个子孙快照只重写一次。
//! - 之后从所有引用出发做标记-清除，删除不再使用的清单、文件与分块（`bonfire_core::gc`）。
//!
//! 只改仓库，不碰存档目录。

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::time::Duration;

use bonfire_core::gc::{self, GcOptions, GcReport};
use bonfire_core::index::GameRecord;
use bonfire_core::refs::{Head, RefKind};
use bonfire_core::{ObjectId, RepoLock, Snapshot, Trigger};
use serde::Serialize;

use crate::app::App;
use crate::config::{Retention, RetentionScope};
use crate::error::Result;
use crate::prune::{changed_between, other_branch};
use crate::util::host_name;

/// 一个游戏的清理结果。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameCleanup {
    pub game_id: String,
    pub name: String,
    /// 清理前的快照数。
    pub snapshots: usize,
    /// 压掉的快照。
    pub removed: Vec<ObjectId>,
    /// 改接到新父快照上的子孙快照数。
    pub rewritten: usize,
    /// 压掉的快照的时间、触发方式与备注（预览中列出）。
    pub items: Vec<CleanupItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupItem {
    pub id: ObjectId,
    pub created_at: String,
    pub trigger: Trigger,
    pub comment: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupReport {
    /// 有快照要压掉的游戏。
    pub games: Vec<GameCleanup>,
    pub removed: usize,
    /// 垃圾回收（预览时为预计）：释放的对象数与空间。
    pub gc: GcReport,
    pub dry_run: bool,
}

/// 清理范围。
#[derive(Debug, Clone, Default)]
pub struct CleanupOptions {
    pub dry_run: bool,
    /// 只清理这些游戏的历史（为空时全部游戏）；垃圾回收总是作用于整个仓库。
    pub games: Vec<String>,
    /// 只做垃圾回收，不按保留策略删除快照。
    pub gc_only: bool,
    /// 没有压掉快照时不做垃圾回收（自动清理时用，避免每次都遍历对象库）。
    pub gc_if_removed: bool,
}

/// 可以由保留策略清理的触发方式。
fn is_auto(t: Trigger) -> bool {
    matches!(
        t,
        Trigger::AutoExit | Trigger::AutoTimer | Trigger::AutoChange | Trigger::PreRestore
    )
}

/// 每个快照在历史中的深度（到最早快照的最长距离）。
fn depth_in_history(
    ids: &HashSet<ObjectId>,
    parents: impl Fn(&ObjectId) -> Vec<ObjectId>,
) -> HashMap<ObjectId, usize> {
    let ps: HashMap<ObjectId, Vec<ObjectId>> = ids
        .iter()
        .map(|id| {
            (
                *id,
                parents(id)
                    .into_iter()
                    .filter(|p| ids.contains(p))
                    .collect(),
            )
        })
        .collect();
    let mut depth: HashMap<ObjectId, usize> = HashMap::new();
    for id in ids {
        let mut stack = vec![*id];
        while let Some(cur) = stack.pop() {
            if depth.contains_key(&cur) {
                continue;
            }
            let pending: Vec<ObjectId> = ps[&cur]
                .iter()
                .filter(|p| !depth.contains_key(*p))
                .copied()
                .collect();
            if pending.is_empty() {
                let d = ps[&cur].iter().map(|p| depth[p] + 1).max().unwrap_or(0);
                depth.insert(cur, d);
            } else {
                stack.push(cur);
                stack.extend(pending);
            }
        }
    }
    depth
}

fn monday_of(d: jiff::civil::Date) -> jiff::civil::Date {
    let back = d.weekday().to_monday_zero_offset() as i64;
    d.checked_sub(jiff::Span::new().days(back)).unwrap_or(d)
}

/// `from` 到 `to` 相隔的天数（`from` 在后时为负）。
fn days_between(from: jiff::civil::Date, to: jiff::civil::Date) -> i64 {
    to.since(from).map(|s| s.get_days() as i64).unwrap_or(0)
}

/// 按保留策略从 `candidates`（可清理的快照，按新到旧排列：ID 与创建时间）中选出要保留的。
pub fn keep_by_policy(
    policy: &Retention,
    candidates: &[(ObjectId, jiff::Zoned)],
    now: &jiff::Zoned,
) -> HashSet<ObjectId> {
    let sorted: Vec<&(ObjectId, jiff::Zoned)> = candidates.iter().collect();
    let mut keep: HashSet<ObjectId> = sorted
        .iter()
        .take(policy.keep_last as usize)
        .map(|(id, _)| *id)
        .collect();
    let today = now.date();
    let this_monday = monday_of(today);
    let mut days = HashSet::new();
    let mut weeks = HashSet::new();
    for (id, at) in &sorted {
        let d = at.date();
        // 今天算第 1 天、本周算第 1 周。
        if days_between(d, today) < policy.keep_daily as i64 && days.insert(d) {
            keep.insert(*id);
        }
        let monday = monday_of(d);
        if days_between(monday, this_monday) / 7 < policy.keep_weekly as i64 && weeks.insert(monday)
        {
            keep.insert(*id);
        }
    }
    keep
}

impl App {
    /// 按保留策略选出一个游戏要压掉的快照（不改仓库）。
    pub(crate) fn retention_plan(
        &self,
        game: &GameRecord,
        now: &jiff::Zoned,
    ) -> Result<(usize, Vec<ObjectId>)> {
        let policy = &self.config.retention;
        let refs = self.repo.list_refs(&game.id)?;
        let head = self.repo.head(&game.id)?;
        let mut roots: Vec<ObjectId> = refs.iter().map(|(_, id)| *id).collect();
        roots.extend(head.detached);
        let all = self.ancestors(&roots, &HashMap::new());

        let mut protected: HashSet<ObjectId> = roots.iter().copied().collect();
        protected.extend(self.repo.head_snapshot(&game.id)?);
        protected.extend(self.undo_snapshots(&game.id)?);

        let host = host_name();
        let protect_local = self.config.backup.protect_local;
        let tz = now.time_zone().clone();
        let mut candidates = Vec::new();
        for id in &all {
            let s = self.repo.get_snapshot(id)?;
            let eligible = match policy.scope {
                RetentionScope::Auto => is_auto(s.trigger),
                RetentionScope::All => {
                    is_auto(s.trigger) || !(protect_local && s.host.eq_ignore_ascii_case(&host))
                }
            };
            if !eligible {
                continue;
            }
            let Ok(ts) = s.created_at.parse::<jiff::Timestamp>() else {
                continue;
            };
            candidates.push((*id, ts.to_zoned(tz.clone())));
        }
        // 新到旧；时间相同（同一秒内）时后代在前，保证改写 ID 后再算结果不变。
        let depth = depth_in_history(&all, |id| {
            self.repo
                .get_snapshot(id)
                .map(|s| s.parents)
                .unwrap_or_default()
        });
        candidates.sort_by(|a, b| b.1.cmp(&a.1).then(depth[&b.0].cmp(&depth[&a.0])));
        let keep = keep_by_policy(policy, &candidates, now);
        let mut removed: Vec<ObjectId> = candidates
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| !keep.contains(id) && !protected.contains(id))
            .collect();
        removed.sort();
        Ok((all.len(), removed))
    }

    /// 清理历史：按保留策略压掉多余的快照，再做垃圾回收。`dry_run` 时只预览。
    pub fn clean_history(
        &mut self,
        lock: &RepoLock,
        opts: &CleanupOptions,
    ) -> Result<CleanupReport> {
        let now = jiff::Zoned::now();
        let mut games = Vec::new();
        if !opts.gc_only {
            for game in self.index.games()? {
                if !opts.games.is_empty() && !opts.games.contains(&game.id) {
                    continue;
                }
                let (snapshots, removed) = self.retention_plan(&game, &now)?;
                if removed.is_empty() {
                    continue;
                }
                let mut items = Vec::new();
                for id in &removed {
                    let snap = self.repo.get_snapshot(id)?;
                    items.push(CleanupItem {
                        id: *id,
                        comment: self.repo.effective_comment(id, &snap)?,
                        created_at: snap.created_at,
                        trigger: snap.trigger,
                    });
                }
                items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
                let rewritten = if opts.dry_run {
                    self.descendants_of(&game.id, &removed)?.len()
                } else {
                    self.squash_snapshots(lock, &game, &removed)?
                };
                games.push(GameCleanup {
                    game_id: game.id.clone(),
                    name: game.name.clone(),
                    snapshots,
                    removed,
                    rewritten,
                    items,
                });
            }
        }
        let removed: usize = games.iter().map(|g| g.removed.len()).sum();
        let gc = if opts.gc_if_removed && removed == 0 {
            GcReport {
                dry_run: opts.dry_run,
                ..Default::default()
            }
        } else {
            let planned = games
                .iter()
                .flat_map(|g| g.removed.iter().copied())
                .collect();
            self.collect_garbage(lock, opts.dry_run, planned)?
        };
        Ok(CleanupReport {
            games,
            removed,
            gc,
            dry_run: opts.dry_run,
        })
    }

    /// 垃圾回收：删除不再被任何快照使用的对象；顺带清理被回收快照的索引记录与备注。
    pub(crate) fn collect_garbage(
        &mut self,
        lock: &RepoLock,
        dry_run: bool,
        assume_removed: HashSet<ObjectId>,
    ) -> Result<GcReport> {
        let mut extra = Vec::new();
        for g in self.index.games()? {
            extra.extend(self.undo_snapshots(&g.id)?);
        }
        let opts = GcOptions {
            dry_run,
            grace: if cfg!(test) {
                Duration::ZERO
            } else {
                Duration::from_secs(3600)
            },
            extra_roots: extra,
            shot_blobs: self.index.all_shot_blobs()?.into_iter().collect(),
            assume_removed: if dry_run {
                assume_removed
            } else {
                HashSet::new()
            },
        };
        let report = gc::gc(&self.repo, lock, &opts)?;
        if !dry_run && !report.snapshots.is_empty() {
            self.index.delete_snapshots(&report.snapshots)?;
            for id in &report.snapshots {
                self.repo.delete_note(lock, id)?;
            }
        }
        tracing::info!(
            dry_run,
            objects = report.objects(),
            bytes = report.bytes,
            "垃圾回收"
        );
        Ok(report)
    }

    /// 自动备份后按保留策略清理（设置中开启自动清理时）。失败只记日志。
    pub fn auto_clean(&mut self, lock: &RepoLock) -> Option<CleanupReport> {
        if !self.config.retention.enabled {
            return None;
        }
        let opts = CleanupOptions {
            gc_if_removed: true,
            ..Default::default()
        };
        match self.clean_history(lock, &opts) {
            Ok(r) => Some(r),
            Err(e) => {
                tracing::warn!("自动清理历史失败：{e}");
                None
            }
        }
    }

    /// `ids` 的子孙快照（不含 `ids` 自身）。
    fn descendants_of(&self, game_id: &str, ids: &[ObjectId]) -> Result<HashSet<ObjectId>> {
        let snaps = self.game_snapshots(game_id)?;
        let mut children: HashMap<ObjectId, Vec<ObjectId>> = HashMap::new();
        for (id, s) in &snaps {
            for p in &s.parents {
                children.entry(*p).or_default().push(*id);
            }
        }
        let gone: HashSet<ObjectId> = ids.iter().copied().collect();
        let mut seen: HashSet<ObjectId> = gone.clone();
        let mut queue: VecDeque<ObjectId> = ids.iter().copied().collect();
        while let Some(id) = queue.pop_front() {
            for c in children.get(&id).into_iter().flatten() {
                if seen.insert(*c) {
                    queue.push_back(*c);
                }
            }
        }
        Ok(seen.difference(&gone).copied().collect())
    }

    /// 改接的后续快照数（预览用）。
    pub(crate) fn descendants_of_pub(&self, game_id: &str, ids: &[ObjectId]) -> Result<usize> {
        Ok(self.descendants_of(game_id, ids)?.len())
    }

    fn game_snapshots(&self, game_id: &str) -> Result<HashMap<ObjectId, Snapshot>> {
        let mut snaps = HashMap::new();
        for row in self.index.snapshots_of(game_id)? {
            if let Ok(s) = self.repo.get_snapshot(&row.id) {
                snaps.insert(row.id, s);
            }
        }
        Ok(snaps)
    }

    /// 一次压掉多个快照（不能带标签）。返回改写的子孙快照数。
    /// 指向被删快照的分支退回到之前留下的快照（没有时删除分支），当前所在的快照被删时同样退回。
    /// 顺序与删除单个快照相同：先写新快照，再改引用，最后删旧快照，中途失败只会多留快照。
    pub(crate) fn squash_snapshots(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        ids: &[ObjectId],
    ) -> Result<usize> {
        let gone: HashSet<ObjectId> = ids.iter().copied().collect();
        let snaps = self.game_snapshots(&game.id)?;
        let rows = self.index.snapshots_of(&game.id)?;
        let descendants = self.descendants_of(&game.id, ids)?;
        // 按在历史中的深度排列：祖先一定在前（中间隔着被删的快照时也是），改写子快照时父快照已有新 ID。
        let all: HashSet<ObjectId> = snaps.keys().copied().collect();
        let depth = depth_in_history(&all, |id| {
            snaps.get(id).map(|s| s.parents.clone()).unwrap_or_default()
        });
        let mut order: Vec<ObjectId> = descendants.into_iter().collect();
        order.sort_by_key(|id| (depth[id], *id));
        let before = self.head_tree(&game.id)?;

        // 被删快照的父快照（越过连续被删的快照）。
        fn resolve(
            p: ObjectId,
            gone: &HashSet<ObjectId>,
            snaps: &HashMap<ObjectId, Snapshot>,
            mapping: &HashMap<ObjectId, ObjectId>,
            out: &mut Vec<ObjectId>,
        ) {
            if gone.contains(&p) {
                for q in snaps.get(&p).map(|s| s.parents.clone()).unwrap_or_default() {
                    resolve(q, gone, snaps, mapping, out);
                }
            } else {
                let m = mapping.get(&p).copied().unwrap_or(p);
                if !out.contains(&m) {
                    out.push(m);
                }
            }
        }

        // 1. 新快照。
        let mut mapping: HashMap<ObjectId, ObjectId> = HashMap::new();
        let mut added = Vec::new();
        for old in &order {
            let mut s = snaps[old].clone();
            let mut parents = Vec::new();
            for p in &s.parents {
                resolve(*p, &gone, &snaps, &mapping, &mut parents);
            }
            s.parents = parents;
            s.stats.changed = match s.parents.first() {
                Some(p) => changed_between(
                    &self.repo.get_tree(&self.repo.get_snapshot(p)?.tree)?,
                    &self.repo.get_tree(&s.tree)?,
                ),
                None => s.stats.files,
            };
            let new_id = self.repo.put_snapshot(&s)?;
            mapping.insert(*old, new_id);
            added.push((new_id, s));
        }

        // 2. 分支、标签、备注、HEAD 与恢复记录。指向被删快照的分支退回到它之前留下的快照，没有时分支一并删除。
        let message = format!("删除 {} 个快照", ids.len());
        let surviving = |id: ObjectId, mapping: &HashMap<ObjectId, ObjectId>| {
            let mut out = Vec::new();
            resolve(id, &gone, &snaps, mapping, &mut out);
            out.first().copied()
        };
        let refs = self.repo.list_refs(&game.id)?;
        let mut deleted_branches = Vec::new();
        for (n, id) in &refs {
            let new = if gone.contains(id) {
                surviving(*id, &mapping)
            } else {
                match mapping.get(id) {
                    Some(m) => Some(*m),
                    None => continue,
                }
            };
            match (n.kind, new) {
                (RefKind::Branch, Some(new)) => self.repo.update_ref(
                    lock,
                    &game.id,
                    n,
                    Some(*id),
                    new,
                    "delete-snapshot",
                    &message,
                )?,
                (RefKind::Branch, None) => {
                    self.repo.delete_ref(lock, &game.id, n)?;
                    deleted_branches.push(n.name.clone());
                }
                (RefKind::Tag, Some(new)) if !gone.contains(id) => {
                    self.repo.delete_ref(lock, &game.id, n)?;
                    self.repo.create_tag(lock, &game.id, &n.name, new)?;
                }
                (RefKind::Tag, _) => {
                    return Err(crate::error::AppError::Invalid(format!(
                        "快照带有标签「{}」，标签保护的快照不能删除",
                        n.name
                    )));
                }
            }
        }
        for (old, new) in &mapping {
            if let Some(note) = self.repo.note(old)? {
                self.repo.set_note(lock, new, &note.comment)?;
            }
        }
        let head = self.repo.head(&game.id)?;
        let detached = match head.detached {
            Some(d) if gone.contains(&d) => surviving(d, &mapping),
            Some(d) => Some(mapping.get(&d).copied().unwrap_or(d)),
            None => None,
        };
        let branch = if deleted_branches.contains(&head.branch) {
            let remaining = self.repo.list_refs(&game.id)?;
            other_branch(&remaining, &head.branch).unwrap_or_else(|| head.branch.clone())
        } else {
            head.branch.clone()
        };
        if detached != head.detached || branch != head.branch {
            self.repo
                .set_head(lock, &game.id, &Head { branch, detached })?;
        }
        self.remap_undo(&game.id, &mapping)?;

        // 3. 索引（沿用原来的显示备注），删除旧快照。
        let comments: BTreeMap<ObjectId, String> =
            rows.iter().map(|r| (r.id, r.comment.clone())).collect();
        let reverse: HashMap<ObjectId, ObjectId> = mapping.iter().map(|(o, n)| (*n, *o)).collect();
        for (id, s) in &added {
            let comment = comments
                .get(&reverse[id])
                .cloned()
                .unwrap_or_else(|| s.comment.clone());
            self.index.add_snapshot_as(id, s, &comment, &game.id)?;
        }
        let mut drop: Vec<ObjectId> = ids.to_vec();
        drop.extend(order.iter().copied());
        for id in ids {
            self.repo.delete_note(lock, id)?;
        }
        self.drop_snapshots(lock, game, &drop, before)?;
        Ok(order.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::BackupOptions;
    use crate::testutil::Env;

    fn at(s: &str) -> jiff::Zoned {
        s.parse().unwrap()
    }

    fn id(n: u8) -> ObjectId {
        ObjectId::of(&[n])
    }

    #[test]
    fn policy_keeps_last_daily_and_weekly() {
        let now = at("2026-10-11T12:00:00+08:00[Asia/Shanghai]");
        let policy = Retention {
            enabled: true,
            keep_last: 2,
            keep_daily: 3,
            keep_weekly: 2,
            scope: RetentionScope::Auto,
        };
        let c = vec![
            (id(1), at("2026-10-11T11:00:00+08:00[Asia/Shanghai]")),
            (id(2), at("2026-10-11T10:00:00+08:00[Asia/Shanghai]")),
            (id(3), at("2026-10-11T09:00:00+08:00[Asia/Shanghai]")),
            (id(4), at("2026-10-10T09:00:00+08:00[Asia/Shanghai]")),
            (id(5), at("2026-10-10T08:00:00+08:00[Asia/Shanghai]")),
            (id(6), at("2026-10-08T08:00:00+08:00[Asia/Shanghai]")),
            (id(7), at("2026-10-04T08:00:00+08:00[Asia/Shanghai]")),
            (id(8), at("2026-10-03T08:00:00+08:00[Asia/Shanghai]")),
            (id(9), at("2026-09-20T08:00:00+08:00[Asia/Shanghai]")),
        ];
        let keep = keep_by_policy(&policy, &c, &now);
        // 最近 2 个：1、2；每天（10-11、10-10、10-09）：1、4；每周（本周 10-05 起、上周 9-28 起）：1、7。
        let mut k: Vec<ObjectId> = keep.into_iter().collect();
        k.sort();
        let mut want = vec![id(1), id(2), id(4), id(7)];
        want.sort();
        assert_eq!(k, want);
    }

    #[test]
    fn cleanup_squashes_auto_snapshots_keeps_manual_and_tags_and_gc_frees_space() {
        let mut env = Env::new();
        let game = env.add_game("g");
        let lock = env.app.repo.lock().unwrap();
        let backup = |env: &mut Env, trigger| {
            env.app
                .backup(
                    &lock,
                    &game,
                    &BackupOptions {
                        trigger,
                        ..Default::default()
                    },
                )
                .unwrap();
        };
        // 一个手动快照，之后 5 个自动快照（每个内容不同），中间一个打标签。
        env.write("a.sav", b"manual");
        backup(&mut env, Trigger::Manual);
        for i in 0..5u8 {
            env.write("a.sav", &vec![b'x' + i; 10 + i as usize]);
            backup(&mut env, Trigger::AutoTimer);
        }
        let id = &game.id;
        assert_eq!(env.app.index.snapshots_of(id).unwrap().len(), 6);
        let tagged = env.app.repo.resolve(id, "HEAD~2").unwrap();
        env.app.repo.create_tag(&lock, id, "boss", tagged).unwrap();
        env.app.config.retention = Retention {
            enabled: true,
            keep_last: 1,
            keep_daily: 0,
            keep_weekly: 0,
            scope: RetentionScope::Auto,
        };

        let preview = env
            .app
            .clean_history(
                &lock,
                &CleanupOptions {
                    dry_run: true,
                    ..Default::default()
                },
            )
            .unwrap();
        // 分支最新的与带标签的保留，手动快照不在清理范围内；另按“最近 1 个”保留一个。
        assert!((2..=3).contains(&preview.removed), "{}", preview.removed);
        assert!(preview.gc.bytes > 0, "预览算出可释放的空间");
        assert_eq!(
            env.app.index.snapshots_of(id).unwrap().len(),
            6,
            "预览不改仓库"
        );

        let r = env
            .app
            .clean_history(&lock, &CleanupOptions::default())
            .unwrap();
        assert_eq!(r.removed, preview.removed);
        assert_eq!(r.gc.chunks, preview.gc.chunks, "预览与实际回收的一致");
        assert!(r.gc.chunks >= 2, "被压掉快照独有的内容被回收");
        let rows = env.app.index.snapshots_of(id).unwrap();
        assert_eq!(rows.len(), 6 - r.removed);
        assert!(rows.iter().any(|r| r.trigger == "manual"), "手动快照保留");
        let tag = env
            .app
            .repo
            .read_ref(id, &bonfire_core::RefName::tag("boss"))
            .unwrap()
            .unwrap();
        assert!(rows.iter().any(|r| r.id == tag), "标签指向改写后的快照");
        // 历史仍是一条链，从最新一直到手动快照。
        let head = env.app.repo.head_snapshot(id).unwrap().unwrap();
        let chain = env.app.repo.first_parent_chain(head, 100).unwrap();
        assert_eq!(chain.len(), rows.len());
        assert!(bonfire_core::fsck::fsck(&env.app.repo, true).unwrap().ok());
        // 状态仍是“已是最新”。
        assert!(!env.app.status(&game).unwrap().summary.has_changes());
        // 再清理一次没有可删的。
        let again = env
            .app
            .clean_history(&lock, &CleanupOptions::default())
            .unwrap();
        assert_eq!((again.removed, again.gc.objects()), (0, 0));
    }

    #[test]
    fn cleanup_after_restore_keeps_undo_working() {
        let mut env = Env::new();
        let game = env.add_game("g");
        let lock = env.app.repo.lock().unwrap();
        for i in 0..4u8 {
            env.write("a.sav", &vec![b'a' + i; 5 + i as usize]);
            env.app
                .backup(
                    &lock,
                    &game,
                    &BackupOptions {
                        trigger: Trigger::AutoChange,
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        let first = env.app.repo.resolve(&game.id, "HEAD~3").unwrap();
        env.write("a.sav", b"unsaved");
        env.app
            .restore(
                &lock,
                &game,
                &first.to_string(),
                &crate::restore::RestoreOptions {
                    full: true,
                    ..Default::default()
                },
            )
            .unwrap();
        env.app.config.retention.keep_last = 1;
        env.app.config.retention.keep_daily = 0;
        env.app.config.retention.keep_weekly = 0;
        let r = env
            .app
            .clean_history(&lock, &CleanupOptions::default())
            .unwrap();
        assert!(r.removed >= 1, "中间的自动快照被压掉");
        assert!(bonfire_core::fsck::fsck(&env.app.repo, true).unwrap().ok());
        // 撤销恢复仍能回到恢复前的状态。
        env.app.undo_restore(&lock, &game).unwrap();
        assert_eq!(env.read("a.sav").unwrap(), b"unsaved");
    }

    #[test]
    fn squashes_snapshots_separated_by_kept_ones() {
        // 自动、手动交替：被删的快照之间隔着保留的快照，改写顺序必须祖先在前。
        let mut env = Env::new();
        let game = env.add_game("g");
        let lock = env.app.repo.lock().unwrap();
        let triggers = [
            Trigger::AutoTimer,
            Trigger::Manual,
            Trigger::AutoTimer,
            Trigger::Manual,
            Trigger::AutoTimer,
            Trigger::Manual,
            Trigger::AutoTimer,
        ];
        for (i, trigger) in triggers.into_iter().enumerate() {
            env.write("a.sav", &vec![b'a' + i as u8; 3 + i]);
            env.app
                .backup(
                    &lock,
                    &game,
                    &BackupOptions {
                        trigger,
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        env.app.config.retention.keep_last = 1;
        env.app.config.retention.keep_daily = 0;
        env.app.config.retention.keep_weekly = 0;
        let r = env
            .app
            .clean_history(&lock, &CleanupOptions::default())
            .unwrap();
        assert_eq!(r.removed, 3, "最新的自动快照是分支最新，其余 3 个压掉");
        let head = env.app.repo.head_snapshot(&game.id).unwrap().unwrap();
        let chain = env.app.repo.first_parent_chain(head, 100).unwrap();
        let kinds: Vec<Trigger> = chain.iter().map(|(_, s)| s.trigger).collect();
        assert_eq!(
            kinds,
            [
                Trigger::AutoTimer,
                Trigger::Manual,
                Trigger::Manual,
                Trigger::Manual
            ]
        );
        assert!(bonfire_core::fsck::fsck(&env.app.repo, true).unwrap().ok());
    }
}
