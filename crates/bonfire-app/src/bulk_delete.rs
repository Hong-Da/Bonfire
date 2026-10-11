//! 一次删除多个快照（详情页 Ctrl / Shift 多选删除，`bonfire rm` 多个快照）。
//!
//! 删除前先检查（预览）：
//! - 不能删除的快照及原因：带标签的；存档安全保护开启时本机备份的；撤销上次恢复要用到的。不能删的跳过，其余照常删除。
//! - 删除的影响：改接的后续快照数；指向被删快照的分支退回到哪个快照（没有可退回的快照时分支一并删除）；
//!   当前分支被删时切换到哪个分支；当前存档对应的快照是否被删除（删除后显示有未备份的更新）；能释放多少空间。
//!
//! 删除时子快照改接到被删快照之前留下的快照上（`App::squash_snapshots`），之后回收不再使用的空间。只改仓库，不碰存档目录。

use std::collections::{HashMap, HashSet};

use bonfire_core::index::GameRecord;
use bonfire_core::refs::RefKind;
use bonfire_core::{ObjectId, RepoLock, Snapshot};
use serde::Serialize;

use crate::app::App;
use crate::error::{AppError, Result};

/// 不能删除的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlockReason {
    /// 带标签（标签保护的存档点）。
    Tag,
    /// 存档安全保护：本机备份的快照。
    Protected,
    /// 撤销上次恢复要用到。
    Undo,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockedSnapshot {
    pub id: ObjectId,
    pub reason: BlockReason,
    /// 原因的补充（标签名）。
    pub detail: String,
}

/// 被删快照所在分支的去向。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchMove {
    pub name: String,
    /// 退回到的快照（删除前的 ID）；没有时分支一并删除。
    pub to: Option<ObjectId>,
    pub to_created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkDeleteReport {
    /// 删除（预览时为将删除）的快照。
    pub removed: Vec<ObjectId>,
    pub blocked: Vec<BlockedSnapshot>,
    /// 改接到新父快照上的后续快照数（内容不变，ID 改变）。
    pub rewritten: usize,
    pub branches: Vec<BranchMove>,
    /// 当前分支被删除时切换到的分支。
    pub head_branch: Option<String>,
    /// 当前存档对应的快照被删除。
    pub current_removed: bool,
    /// 释放（预览时为预计）的空间。
    pub bytes: u64,
    /// 释放空间的来源明细。
    pub sources: Vec<bonfire_core::gc::GcSource>,
    pub dry_run: bool,
}

impl App {
    /// 删除多个快照。`dry_run` 时只检查与预览；执行时跳过不能删除的快照，一个都不能删时报错。
    pub fn delete_snapshots(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        specs: &[String],
        dry_run: bool,
    ) -> Result<BulkDeleteReport> {
        let mut ids: Vec<ObjectId> = Vec::new();
        for s in specs {
            let id = self.repo.resolve(&game.id, s)?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        if ids.is_empty() {
            return Err(AppError::Invalid("没有选择快照".into()));
        }
        let snaps: HashMap<ObjectId, Snapshot> = self
            .index
            .snapshots_of(&game.id)?
            .into_iter()
            .filter_map(|r| self.repo.get_snapshot(&r.id).ok().map(|s| (r.id, s)))
            .collect();
        let refs = self.repo.list_refs(&game.id)?;
        let undo: HashSet<ObjectId> = self.undo_snapshots(&game.id)?.into_iter().collect();

        // 1. 检查。
        let mut blocked = Vec::new();
        let mut removed = Vec::new();
        for id in &ids {
            if !snaps.contains_key(id) {
                return Err(AppError::Invalid(format!(
                    "快照 {} 不属于「{}」",
                    id.short(12),
                    game.name
                )));
            }
            let block = if let Some((t, _)) = refs
                .iter()
                .find(|(n, target)| n.kind == RefKind::Tag && target == id)
            {
                Some((BlockReason::Tag, t.name.clone()))
            } else if self.local_among(&[*id])?.is_some() {
                Some((BlockReason::Protected, String::new()))
            } else if undo.contains(id) {
                Some((BlockReason::Undo, String::new()))
            } else {
                None
            };
            match block {
                Some((reason, detail)) => blocked.push(BlockedSnapshot {
                    id: *id,
                    reason,
                    detail,
                }),
                None => removed.push(*id),
            }
        }
        removed.sort();
        let gone: HashSet<ObjectId> = removed.iter().copied().collect();

        // 2. 影响：分支退回到之前留下的快照（沿第一个父快照找），当前分支与当前快照。
        let surviving = |mut id: ObjectId| -> Option<ObjectId> {
            loop {
                if !gone.contains(&id) {
                    return Some(id);
                }
                id = *snaps.get(&id)?.parents.first()?;
            }
        };
        let mut branches = Vec::new();
        for (n, target) in &refs {
            if n.kind == RefKind::Branch && gone.contains(target) {
                let to = surviving(*target);
                branches.push(BranchMove {
                    name: n.name.clone(),
                    to,
                    to_created_at: to.and_then(|t| snaps.get(&t)).map(|s| s.created_at.clone()),
                });
            }
        }
        let head = self.repo.head(&game.id)?;
        let deleted: Vec<&str> = branches
            .iter()
            .filter(|b| b.to.is_none())
            .map(|b| b.name.as_str())
            .collect();
        let head_branch = if deleted.contains(&head.branch.as_str()) {
            let remaining: Vec<_> = refs
                .iter()
                .filter(|(n, _)| !(n.kind == RefKind::Branch && deleted.contains(&n.name.as_str())))
                .cloned()
                .collect();
            crate::prune::other_branch(&remaining, &head.branch)
        } else {
            None
        };
        let current_removed = self
            .repo
            .head_snapshot(&game.id)?
            .is_some_and(|h| gone.contains(&h));
        let rewritten = self.descendants_of_pub(&game.id, &removed)?;

        if dry_run {
            let (gc, sources) = if removed.is_empty() {
                (0, Vec::new())
            } else {
                let r = self.collect_garbage(lock, true, gone.clone())?;
                (r.bytes, r.sources)
            };
            return Ok(BulkDeleteReport {
                removed,
                blocked,
                rewritten,
                branches,
                head_branch,
                current_removed,
                bytes: gc,
                sources,
                dry_run,
            });
        }
        if removed.is_empty() {
            return Err(AppError::Invalid(
                "所选的快照都不能删除（带标签、受存档安全保护或撤销恢复要用到）".into(),
            ));
        }
        let rewritten = self.squash_snapshots(lock, game, &removed)?;
        let gc = self.collect_garbage(lock, false, HashSet::new())?;
        tracing::info!(game = %game.id, removed = removed.len(), rewritten, bytes = gc.bytes, "删除多个快照");
        Ok(BulkDeleteReport {
            removed,
            blocked,
            rewritten,
            branches,
            head_branch: if head_branch.is_some() {
                Some(self.repo.head(&game.id)?.branch)
            } else {
                None
            },
            current_removed,
            bytes: gc.bytes,
            sources: gc.sources,
            dry_run,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::BackupOptions;
    use crate::testutil::Env;

    fn backups(env: &mut Env, game: &GameRecord, n: u8) -> Vec<ObjectId> {
        let lock = env.app.repo.lock().unwrap();
        let mut out = Vec::new();
        for i in 0..n {
            env.write("a.sav", &vec![b'a' + i; 4 + i as usize]);
            env.app
                .backup(&lock, game, &BackupOptions::default())
                .unwrap();
            out.push(env.app.repo.head_snapshot(&game.id).unwrap().unwrap());
        }
        out
    }

    #[test]
    fn checks_then_deletes_range_skipping_blocked() {
        let mut env = Env::new();
        env.app.config.backup.protect_local = false;
        let game = env.add_game("g");
        let s = backups(&mut env, &game, 6);
        let lock = env.app.repo.lock().unwrap();
        env.app
            .repo
            .create_tag(&lock, &game.id, "boss", s[2])
            .unwrap();
        // 选中 s1–s4（Shift 范围）：s2 带标签不能删。
        let pick: Vec<String> = s[1..5].iter().map(|i| i.to_string()).collect();
        let plan = env.app.delete_snapshots(&lock, &game, &pick, true).unwrap();
        assert_eq!(plan.removed.len(), 3);
        assert_eq!(plan.blocked.len(), 1);
        assert_eq!(plan.blocked[0].reason, BlockReason::Tag);
        assert_eq!(plan.blocked[0].detail, "boss");
        assert_eq!(plan.rewritten, 2, "标签快照与最新快照改接");
        assert!(plan.bytes > 0);
        assert!(!plan.current_removed);
        assert_eq!(
            env.app.index.snapshots_of(&game.id).unwrap().len(),
            6,
            "预览不改仓库"
        );

        let r = env
            .app
            .delete_snapshots(&lock, &game, &pick, false)
            .unwrap();
        assert_eq!(r.removed.len(), 3);
        let head = env.app.repo.head_snapshot(&game.id).unwrap().unwrap();
        let chain = env.app.repo.first_parent_chain(head, 100).unwrap();
        assert_eq!(chain.len(), 3, "s0、s2（标签）、s5");
        assert!(bonfire_core::fsck::fsck(&env.app.repo, true).unwrap().ok());
        assert!(!env.app.status(&game).unwrap().summary.has_changes());
    }

    #[test]
    fn deleting_branch_tip_moves_branch_back_and_reports_current() {
        let mut env = Env::new();
        env.app.config.backup.protect_local = false;
        let game = env.add_game("g");
        let s = backups(&mut env, &game, 3);
        let lock = env.app.repo.lock().unwrap();
        let pick = vec![s[1].to_string(), s[2].to_string()];
        let plan = env.app.delete_snapshots(&lock, &game, &pick, true).unwrap();
        assert!(plan.current_removed, "删的是当前存档对应的快照");
        assert_eq!(plan.branches.len(), 1);
        assert_eq!(plan.branches[0].name, "main");
        assert_eq!(plan.branches[0].to, Some(s[0]));
        env.app
            .delete_snapshots(&lock, &game, &pick, false)
            .unwrap();
        assert_eq!(env.app.repo.head_snapshot(&game.id).unwrap(), Some(s[0]));
        assert!(
            env.app.status(&game).unwrap().summary.has_changes(),
            "磁盘上的存档比快照新"
        );
        assert!(bonfire_core::fsck::fsck(&env.app.repo, true).unwrap().ok());
        // 只剩一个快照，删掉后分支一并删除。
        let plan = env
            .app
            .delete_snapshots(&lock, &game, &[s[0].to_string()], true)
            .unwrap();
        assert_eq!(plan.branches[0].to, None);
    }

    #[test]
    fn protected_local_snapshots_are_blocked() {
        let mut env = Env::new();
        let game = env.add_game("g");
        let s = backups(&mut env, &game, 2);
        let lock = env.app.repo.lock().unwrap();
        let pick = vec![s[0].to_string()];
        let plan = env.app.delete_snapshots(&lock, &game, &pick, true).unwrap();
        assert_eq!(plan.blocked[0].reason, BlockReason::Protected);
        assert!(plan.removed.is_empty());
        assert!(
            env.app
                .delete_snapshots(&lock, &game, &pick, false)
                .is_err()
        );
    }
}
