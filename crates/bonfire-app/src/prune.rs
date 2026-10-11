//! 删除分支、标签与快照（文档 6.4、6.6）。
//!
//! - **删除分支**：去掉分支指针，只属于这个分支的快照（其他分支、标签与当前所在快照都到不了的）一并删除。
//! - **删除快照**：从历史中“压掉”——它的子快照改为接在它的父快照上（类似 rebase）。子孙快照的父指针变了，
//!   会得到新的 ID，指向它们的分支、标签、备注与“已从某快照恢复”的标记随之更新；分支的最新快照被删除时，分支退回上一个快照。
//! - 带标签的快照受保护：先删除标签再删快照。
//! - 存档安全保护（设置 `backup.protect_local`，默认开启）：本机备份的快照（快照记录的电脑名与本机相同）不能删除，
//!   会删掉这类快照的分支也不能删除；从别处导入的分支与快照不受影响。
//!
//! 被删除快照引用的清单与文件仍留在对象库中，由 GC（M4）回收空间。只改仓库，不碰存档目录。

use std::collections::{HashMap, HashSet, VecDeque};

use bonfire_core::index::{GameRecord, IndexEntry};
use bonfire_core::object::{Snapshot, Tree};
use bonfire_core::refs::{Head, RefKind, RefName};
use bonfire_core::{ObjectId, RepoLock};
use serde::Serialize;

use crate::app::App;
use crate::error::{AppError, Result};
use crate::util::host_name;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteReport {
    /// 被删除的快照（删除快照时不含被改接的子孙快照的旧 ID）。
    pub removed: Vec<ObjectId>,
    /// 改接到新父快照上的子孙快照数。
    pub rewritten: usize,
    /// 一并删除的分支（快照删除后没有快照可指向的分支）。
    pub branches_deleted: Vec<String>,
    /// 当前分支改为（删除的正是当前分支时）。
    pub head_branch: Option<String>,
    pub dry_run: bool,
}

impl App {
    /// 删除分支。只属于这个分支的快照一并删除；删除的是当前分支时切换到 main（或其他分支）。
    pub fn delete_branch(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        name: &str,
        dry_run: bool,
    ) -> Result<DeleteReport> {
        let r = RefName::branch(name);
        let tip = self
            .repo
            .read_ref(&game.id, &r)?
            .ok_or_else(|| AppError::Invalid(format!("找不到分支「{name}」")))?;
        self.check_not_primary(&game.id, name)?;
        let head = self.repo.head(&game.id)?;
        let refs = self.repo.list_refs(&game.id)?;
        let mut keep_roots: Vec<ObjectId> = refs
            .iter()
            .filter(|(n, _)| *n != r)
            .map(|(_, id)| *id)
            .collect();
        if head.branch != name {
            keep_roots.extend(head.detached);
        }
        let keep = self.ancestors(&keep_roots, &HashMap::new());
        let mut removed: Vec<ObjectId> = self
            .ancestors(&[tip], &HashMap::new())
            .into_iter()
            .filter(|id| !keep.contains(id))
            .collect();
        removed.sort();
        if let Some(local) = self.local_among(&removed)? {
            return Err(protected(&format!(
                "分支「{name}」中有本机备份的快照（如 {}）",
                local.short(12)
            )));
        }
        let new_head = (head.branch == name).then(|| other_branch(&refs, name));
        let report = DeleteReport {
            removed: removed.clone(),
            rewritten: 0,
            branches_deleted: vec![name.to_string()],
            head_branch: new_head.clone().flatten(),
            dry_run,
        };
        if dry_run {
            return Ok(report);
        }
        let before = self.head_tree(&game.id)?;
        self.repo.delete_ref(lock, &game.id, &r)?;
        if let Some(branch) = new_head {
            let detached = head.detached.filter(|d| !removed.contains(d));
            self.repo.set_head(
                lock,
                &game.id,
                &Head {
                    branch: branch.unwrap_or_else(|| name.to_string()),
                    detached,
                },
            )?;
        }
        self.drop_snapshots(lock, game, &removed, before)?;
        Ok(report)
    }

    /// 删除游戏的全部快照、分支与标签（移出游戏库时用户选择“同时删除备份的存档”，U-15）。
    /// 存档安全保护开启且其中有本机备份的快照时拒绝，什么都不删。只改仓库，不碰存档目录。
    pub fn delete_game_history(&mut self, lock: &RepoLock, game: &GameRecord) -> Result<usize> {
        let ids: Vec<ObjectId> = self
            .index
            .snapshots_of(&game.id)?
            .into_iter()
            .map(|r| r.id)
            .collect();
        if let Some(local) = self.local_among(&ids)? {
            return Err(protected(&format!(
                "「{}」中有本机备份的快照（如 {}）",
                game.name,
                local.short(12)
            )));
        }
        let before = self.head_tree(&game.id)?;
        for (r, _) in self.repo.list_refs(&game.id)? {
            self.repo.delete_ref(lock, &game.id, &r)?;
        }
        self.repo.set_head(
            lock,
            &game.id,
            &Head {
                branch: "main".into(),
                detached: None,
            },
        )?;
        self.drop_snapshots(lock, game, &ids, before)?;
        Ok(ids.len())
    }

    /// 删除标签（只删指针，快照保留）。
    pub fn delete_tag(&mut self, lock: &RepoLock, game: &GameRecord, name: &str) -> Result<()> {
        let r = RefName::tag(name);
        if self.repo.read_ref(&game.id, &r)?.is_none() {
            return Err(AppError::Invalid(format!("找不到标签「{name}」")));
        }
        self.repo.delete_ref(lock, &game.id, &r)?;
        self.index
            .set_refs(&game.id, &self.repo.list_refs(&game.id)?)?;
        Ok(())
    }

    /// 删除快照：子快照改接到它的父快照上。带标签的快照不能删除。
    pub fn delete_snapshot(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        spec: &str,
        dry_run: bool,
    ) -> Result<DeleteReport> {
        let target = self.repo.resolve(&game.id, spec)?;
        let refs = self.repo.list_refs(&game.id)?;
        if let Some((t, _)) = refs
            .iter()
            .find(|(n, id)| n.kind == RefKind::Tag && *id == target)
        {
            return Err(AppError::Invalid(format!(
                "快照带有标签「{}」，标签保护的快照不能删除；如确实要删，请先删除标签",
                t.name
            )));
        }
        let gone = self.repo.get_snapshot(&target)?;
        if self.local_among(&[target])?.is_some() {
            return Err(protected("这个快照是本机备份的"));
        }

        // 子孙快照：按父在前的顺序重写。
        let rows = self.index.snapshots_of(&game.id)?;
        let mut snaps: HashMap<ObjectId, Snapshot> = HashMap::new();
        for row in &rows {
            if let Ok(s) = self.repo.get_snapshot(&row.id) {
                snaps.insert(row.id, s);
            }
        }
        let mut children: HashMap<ObjectId, Vec<ObjectId>> = HashMap::new();
        for (id, s) in &snaps {
            for p in &s.parents {
                children.entry(*p).or_default().push(*id);
            }
        }
        let mut descendants = HashSet::new();
        let mut queue = VecDeque::from([target]);
        while let Some(id) = queue.pop_front() {
            for c in children.get(&id).into_iter().flatten() {
                if descendants.insert(*c) {
                    queue.push_back(*c);
                }
            }
        }
        let order = parents_first(&descendants, &snaps);

        // 分支：指向被删快照的退回其父快照；没有父快照时分支一并删除。
        let head = self.repo.head(&game.id)?;
        let fallback = gone.parents.first().copied();
        let mut branches_deleted = Vec::new();
        if fallback.is_none() {
            for (n, id) in &refs {
                if n.kind == RefKind::Branch && *id == target {
                    branches_deleted.push(n.name.clone());
                }
            }
        }
        let head_deleted = branches_deleted.contains(&head.branch);
        let remaining: Vec<(RefName, ObjectId)> = refs
            .iter()
            .filter(|(n, _)| !(n.kind == RefKind::Branch && branches_deleted.contains(&n.name)))
            .cloned()
            .collect();
        let new_head = head_deleted.then(|| other_branch(&remaining, &head.branch));
        if dry_run {
            return Ok(DeleteReport {
                removed: vec![target],
                rewritten: order.len(),
                branches_deleted,
                head_branch: new_head.flatten(),
                dry_run,
            });
        }

        // 1. 先写新快照，再改引用，最后删旧快照：中途失败只会留下多余的快照，不会丢历史。
        let before = self.head_tree(&game.id)?;
        let mut mapping: HashMap<ObjectId, ObjectId> = HashMap::new();
        let mut added = Vec::new();
        for old in &order {
            let mut s = snaps[old].clone();
            let mut parents = Vec::new();
            for p in &s.parents {
                let mapped: Vec<ObjectId> = if *p == target {
                    gone.parents.clone()
                } else {
                    vec![mapping.get(p).copied().unwrap_or(*p)]
                };
                for m in mapped {
                    if !parents.contains(&m) {
                        parents.push(m);
                    }
                }
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

        // 2. 分支、标签、备注与 HEAD。
        let message = format!("删除快照 {}", target.short(12));
        for (n, id) in &refs {
            match n.kind {
                RefKind::Branch if *id == target => match fallback {
                    Some(p) => self.repo.update_ref(
                        lock,
                        &game.id,
                        n,
                        Some(*id),
                        p,
                        "delete-snapshot",
                        &message,
                    )?,
                    None => self.repo.delete_ref(lock, &game.id, n)?,
                },
                RefKind::Branch if mapping.contains_key(id) => self.repo.update_ref(
                    lock,
                    &game.id,
                    n,
                    Some(*id),
                    mapping[id],
                    "delete-snapshot",
                    &message,
                )?,
                // 标签不可移动：删掉后在新快照上重建。
                RefKind::Tag if mapping.contains_key(id) => {
                    self.repo.delete_ref(lock, &game.id, n)?;
                    self.repo.create_tag(lock, &game.id, &n.name, mapping[id])?;
                }
                _ => {}
            }
        }
        for (old, new) in &mapping {
            if let Some(note) = self.repo.note(old)? {
                self.repo.set_note(lock, new, &note.comment)?;
            }
        }
        let detached = match head.detached {
            Some(d) if d == target => None,
            Some(d) => Some(mapping.get(&d).copied().unwrap_or(d)),
            None => None,
        };
        let branch = match new_head {
            Some(b) => b.unwrap_or(head.branch.clone()),
            None => head.branch.clone(),
        };
        if detached != head.detached || branch != head.branch {
            self.repo
                .set_head(lock, &game.id, &Head { branch, detached })?;
        }

        // 3. 索引：登记新快照（沿用原来的显示备注）。
        let comments: HashMap<ObjectId, String> =
            rows.iter().map(|r| (r.id, r.comment.clone())).collect();
        let reverse: HashMap<ObjectId, ObjectId> = mapping.iter().map(|(o, n)| (*n, *o)).collect();
        for (id, s) in &added {
            let comment = comments
                .get(&reverse[id])
                .cloned()
                .unwrap_or_else(|| s.comment.clone());
            self.index.add_snapshot_as(id, s, &comment, &game.id)?;
        }
        self.remap_undo(&game.id, &mapping)?;
        self.repo.delete_note(lock, &target)?;
        let mut removed_all = vec![target];
        removed_all.extend(order.iter().copied());
        self.drop_snapshots(lock, game, &removed_all, before)?;
        Ok(DeleteReport {
            removed: vec![target],
            rewritten: order.len(),
            branches_deleted,
            head_branch: if head_deleted {
                Some(self.repo.head(&game.id)?.branch)
            } else {
                None
            },
            dry_run,
        })
    }

    /// 变更检测缓存改为当前所在快照的清单（没有快照时清空）：之后的状态与这个快照比较。
    pub(crate) fn cache_head_tree(&mut self, game_id: &str) -> Result<()> {
        let entries: Vec<IndexEntry> = match self.repo.head_snapshot(game_id)? {
            Some(id) => self
                .repo
                .get_tree(&self.repo.get_snapshot(&id)?.tree)?
                .entries
                .iter()
                .map(|e| IndexEntry {
                    path: e.path.clone(),
                    size: e.size,
                    mtime_ns: e.mtime_ns,
                    hash: e.blob,
                    indexed_at_ns: 0,
                })
                .collect(),
            None => Vec::new(),
        };
        self.index.replace_scan_entries(game_id, &entries)?;
        Ok(())
    }

    /// 存档安全保护开启时，返回其中第一个本机备份的快照。
    pub(crate) fn local_among(&self, ids: &[ObjectId]) -> Result<Option<ObjectId>> {
        if !self.config.backup.protect_local {
            return Ok(None);
        }
        let host = host_name();
        for id in ids {
            if self.repo.get_snapshot(id)?.host.eq_ignore_ascii_case(&host) {
                return Ok(Some(*id));
            }
        }
        Ok(None)
    }

    /// 当前所在快照的清单。
    pub(crate) fn head_tree(&self, game_id: &str) -> Result<Option<ObjectId>> {
        Ok(match self.repo.head_snapshot(game_id)? {
            Some(id) => Some(self.repo.get_snapshot(&id)?.tree),
            None => None,
        })
    }

    /// 删除快照对象与索引记录，刷新引用镜像；当前所在快照变了时，变更检测缓存改为新的当前快照的清单
    /// （删掉的正是磁盘当前状态对应的快照时，界面会提示有未备份的更新）。
    pub(crate) fn drop_snapshots(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        ids: &[ObjectId],
        tree_before: Option<ObjectId>,
    ) -> Result<()> {
        for id in ids {
            self.repo.remove_snapshot(lock, id)?;
        }
        self.index.delete_snapshots(ids)?;
        self.index
            .set_refs(&game.id, &self.repo.list_refs(&game.id)?)?;
        if self.head_tree(&game.id)? != tree_before {
            self.cache_head_tree(&game.id)?;
        }
        Ok(())
    }
}

fn protected(what: &str) -> AppError {
    AppError::Invalid(format!(
        "{what}，存档安全保护开启时不能删除；如确实要删，请先在设置中关闭“保护本机的存档备份”"
    ))
}

/// 删除当前分支后切换到的分支：优先 main，其次按名称排第一的分支；没有其他分支时为 None（保留空分支名）。
pub(crate) fn other_branch(refs: &[(RefName, ObjectId)], except: &str) -> Option<String> {
    let mut names: Vec<&str> = refs
        .iter()
        .filter(|(n, _)| n.kind == RefKind::Branch && n.name != except)
        .map(|(n, _)| n.name.as_str())
        .collect();
    names.sort_by_key(|n| (*n != bonfire_core::refs::DEFAULT_BRANCH, n.to_lowercase()));
    names.first().map(|n| n.to_string())
}

/// 集合内的快照按父在前排序。
pub(crate) fn parents_first(
    set: &HashSet<ObjectId>,
    snaps: &HashMap<ObjectId, Snapshot>,
) -> Vec<ObjectId> {
    let mut out = Vec::new();
    let mut done = HashSet::new();
    let mut ids: Vec<ObjectId> = set.iter().copied().collect();
    ids.sort();
    for start in ids {
        let mut stack = vec![start];
        while let Some(&id) = stack.last() {
            if done.contains(&id) {
                stack.pop();
                continue;
            }
            let pending: Vec<ObjectId> = snaps[&id]
                .parents
                .iter()
                .filter(|p| set.contains(p) && !done.contains(*p))
                .copied()
                .collect();
            if pending.is_empty() {
                done.insert(id);
                out.push(id);
                stack.pop();
            } else {
                stack.extend(pending);
            }
        }
    }
    out
}

/// 两个清单之间变化的文件数（新增、修改与删除）。
pub(crate) fn changed_between(before: &Tree, after: &Tree) -> u64 {
    let old: HashMap<&str, ObjectId> = before
        .entries
        .iter()
        .map(|e| (e.path.as_str(), e.blob))
        .collect();
    let new: HashSet<&str> = after.entries.iter().map(|e| e.path.as_str()).collect();
    let changed = after
        .entries
        .iter()
        .filter(|e| old.get(e.path.as_str()) != Some(&e.blob))
        .count();
    let removed = old.keys().filter(|p| !new.contains(*p)).count();
    (changed + removed) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Env;
    use crate::{BackupOptions, BackupResult};

    #[test]
    fn remove_game_can_delete_backups_but_never_disk_files() {
        let mut env = Env::new();
        env.write("a.sav", b"1");
        let g = env.add_game("G");
        backup(&mut env, &g, "one");
        env.write("a.sav", b"2");
        backup(&mut env, &g, "two");
        {
            let lock = env.app.repo.lock().unwrap();
            env.app
                .create_ref(&lock, &g, RefKind::Tag, "t", None)
                .unwrap();
        }
        // 一张截图：放在为游戏添加的截图文件夹中（伪造目录）。
        let shots = env.dir.path().join("shots");
        std::fs::create_dir_all(&shots).unwrap();
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        std::fs::write(shots.join("s.png"), png.into_inner()).unwrap();
        env.app.add_shot_dir(&g, &shots).unwrap();
        let lock = env.app.repo.lock().unwrap();
        env.app.sync_shots(&lock, &g).unwrap();
        let shot_dir = env.app.shot_dir(&g.id);
        assert!(shot_dir.is_dir());

        // 保护开启：拒绝，什么都不改。
        env.app.config.backup.protect_local = true;
        let both = crate::app::RemoveOptions {
            snapshots: true,
            shots: true,
        };
        assert!(env.app.remove_game_with(&lock, &g.id, &both).is_err());
        assert_eq!(env.app.index.snapshots_of(&g.id).unwrap().len(), 2);
        assert!(env.app.find_game(&g.id).is_ok());
        assert!(shot_dir.is_dir());

        // 保护关闭：删除全部快照、分支、标签与截图备份；磁盘上的存档与截图原图不动。
        env.app.config.backup.protect_local = false;
        let r = env.app.remove_game_with(&lock, &g.id, &both).unwrap();
        assert_eq!((r.snapshots, r.shots), (2, 1));
        assert!(env.app.index.snapshots_of(&g.id).unwrap().is_empty());
        assert!(env.app.repo.list_refs(&g.id).unwrap().is_empty());
        assert!(env.app.index.shots_of(&g.id).unwrap().is_empty());
        assert!(!shot_dir.exists());
        assert_eq!(env.read("a.sav").unwrap(), b"2");
        assert!(shots.join("s.png").is_file());
    }

    #[test]
    fn remove_game_keeps_backups_by_default() {
        let mut env = Env::new();
        env.write("a.sav", b"1");
        let g = env.add_game("G");
        backup(&mut env, &g, "one");
        let lock = env.app.repo.lock().unwrap();
        let r = env
            .app
            .remove_game_with(&lock, &g.id, &crate::app::RemoveOptions::default())
            .unwrap();
        assert_eq!((r.snapshots, r.shots), (0, 0));
        assert_eq!(env.app.index.snapshots_of(&g.id).unwrap().len(), 1);
    }

    fn backup(env: &mut Env, g: &GameRecord, comment: &str) -> ObjectId {
        let lock = env.app.repo.lock().unwrap();
        match env
            .app
            .backup(
                &lock,
                g,
                &BackupOptions {
                    comment: Some(comment.into()),
                    ..Default::default()
                },
            )
            .unwrap()
        {
            BackupResult::Created { snapshot, .. } => snapshot,
            other => panic!("{other:?}"),
        }
    }

    fn switch(env: &mut Env, g: &GameRecord, branch: &str) {
        let lock = env.app.repo.lock().unwrap();
        env.app
            .repo
            .set_head(
                &lock,
                &g.id,
                &Head {
                    branch: branch.into(),
                    detached: None,
                },
            )
            .unwrap();
    }

    fn comments(env: &Env, g: &GameRecord) -> Vec<String> {
        let mut v: Vec<String> = env
            .app
            .history_view(g)
            .unwrap()
            .nodes
            .into_iter()
            .map(|n| n.comment)
            .collect();
        v.sort();
        v
    }

    /// main: A - B - C；alt 从 A 分出：A - D。测试删除逻辑时关闭存档安全保护（快照都是本机备份的）。
    fn forked(env: &mut Env) -> (GameRecord, [ObjectId; 4]) {
        env.app.config.backup.protect_local = false;
        env.write("a.sav", b"1");
        let g = env.add_game("Game");
        let a = backup(env, &g, "A");
        env.write("a.sav", b"2");
        let b = backup(env, &g, "B");
        env.write("a.sav", b"3");
        let c = backup(env, &g, "C");
        let lock = env.app.repo.lock().unwrap();
        env.app
            .create_ref(&lock, &g, RefKind::Branch, "alt", Some(&a.to_string()))
            .unwrap();
        drop(lock);
        switch(env, &g, "alt");
        env.write("a.sav", b"4");
        let d = backup(env, &g, "D");
        switch(env, &g, "main");
        (g, [a, b, c, d])
    }

    #[test]
    fn delete_branch_removes_only_its_own_snapshots() {
        let mut env = Env::new();
        let (g, [a, _b, _c, d]) = forked(&mut env);
        let lock = env.app.repo.lock().unwrap();
        let dry = env.app.delete_branch(&lock, &g, "alt", true).unwrap();
        assert_eq!(dry.removed, vec![d]);
        assert!(env.app.repo.has(bonfire_core::Kind::Snapshot, &d));

        let r = env.app.delete_branch(&lock, &g, "alt", false).unwrap();
        assert_eq!((r.removed, r.head_branch), (vec![d], None));
        assert!(!env.app.repo.has(bonfire_core::Kind::Snapshot, &d));
        assert!(env.app.repo.has(bonfire_core::Kind::Snapshot, &a));
        assert_eq!(comments(&env, &g), ["A", "B", "C"]);
        assert!(env.app.fsck(false).unwrap().ok());
        // 重建索引后被删的快照不会回来。
        env.app.reindex(&lock).unwrap();
        assert_eq!(comments(&env, &g), ["A", "B", "C"]);
    }

    #[test]
    fn deleting_current_branch_switches_to_main() {
        let mut env = Env::new();
        let (g, [_a, _b, c, _d]) = forked(&mut env);
        switch(&mut env, &g, "alt");
        let lock = env.app.repo.lock().unwrap();
        let r = env.app.delete_branch(&lock, &g, "alt", false).unwrap();
        assert_eq!(r.head_branch.as_deref(), Some("main"));
        assert_eq!(env.app.repo.head_snapshot(&g.id).unwrap(), Some(c));
        // 磁盘上是 D 的内容，D 已删除：显示有更新。
        let row = env.app.game_row(&g, false).unwrap();
        assert_eq!(row.state, crate::GameState::HasChanges);
        assert!(env.app.fsck(false).unwrap().ok());
    }

    #[test]
    fn delete_middle_snapshot_reparents_children_and_keeps_tags_and_notes() {
        let mut env = Env::new();
        let (g, [a, b, c, d]) = forked(&mut env);
        let lock = env.app.repo.lock().unwrap();
        env.app
            .create_ref(&lock, &g, RefKind::Tag, "boss", Some(&c.to_string()))
            .unwrap();
        env.app
            .set_comment(&lock, &g, &c.to_string(), "C 改过的备注")
            .unwrap();

        let err = env
            .app
            .delete_snapshot(&lock, &g, &c.to_string(), false)
            .unwrap_err();
        assert!(err.to_string().contains("boss"), "{err}");

        let r = env
            .app
            .delete_snapshot(&lock, &g, &b.to_string(), false)
            .unwrap();
        assert_eq!((r.removed, r.rewritten), (vec![b], 1));
        let main = env
            .app
            .repo
            .read_ref(&g.id, &RefName::branch("main"))
            .unwrap()
            .unwrap();
        assert_ne!(main, c);
        let snap = env.app.repo.get_snapshot(&main).unwrap();
        assert_eq!(snap.parents, vec![a]);
        assert_eq!(snap.stats.changed, 1);
        assert_eq!(
            env.app.repo.read_ref(&g.id, &RefName::tag("boss")).unwrap(),
            Some(main)
        );
        assert_eq!(
            env.app.repo.note(&main).unwrap().unwrap().comment,
            "C 改过的备注"
        );
        // alt 不受影响。
        assert_eq!(
            env.app
                .repo
                .read_ref(&g.id, &RefName::branch("alt"))
                .unwrap(),
            Some(d)
        );
        assert_eq!(comments(&env, &g), ["A", "C 改过的备注", "D"]);
        assert!(!env.app.repo.has(bonfire_core::Kind::Snapshot, &b));
        assert!(!env.app.repo.has(bonfire_core::Kind::Snapshot, &c));
        // 当前快照的内容没变，变更检测缓存不动，状态不变。
        assert_eq!(
            env.app.game_row(&g, false).unwrap().state,
            crate::GameState::UpToDate
        );
        assert!(env.app.fsck(false).unwrap().ok());
        env.app.reindex(&lock).unwrap();
        assert_eq!(comments(&env, &g), ["A", "C 改过的备注", "D"]);
    }

    #[test]
    fn delete_tip_moves_branch_back_and_root_only_snapshot_deletes_branch() {
        let mut env = Env::new();
        let (g, [a, b, c, d]) = forked(&mut env);
        let lock = env.app.repo.lock().unwrap();
        env.app
            .delete_snapshot(&lock, &g, &c.to_string(), false)
            .unwrap();
        assert_eq!(env.app.repo.head_snapshot(&g.id).unwrap(), Some(b));
        assert_eq!(
            env.app.game_row(&g, false).unwrap().state,
            crate::GameState::HasChanges
        );

        // 删除 A：B 与 D 都变成没有父快照的起点。
        let r = env
            .app
            .delete_snapshot(&lock, &g, &a.to_string(), false)
            .unwrap();
        assert_eq!(r.rewritten, 2);
        let view = env.app.history_view(&g).unwrap();
        assert_eq!(view.nodes.len(), 2);
        assert!(view.nodes.iter().all(|n| n.parents.is_empty()));
        assert!(!view.nodes.iter().any(|n| n.id == d));

        // 只有一个快照的分支：删掉快照即删掉分支。
        let alt = env
            .app
            .repo
            .read_ref(&g.id, &RefName::branch("alt"))
            .unwrap()
            .unwrap();
        let r = env
            .app
            .delete_snapshot(&lock, &g, &alt.to_string(), false)
            .unwrap();
        assert_eq!(r.branches_deleted, ["alt"]);
        assert!(env.app.fsck(false).unwrap().ok());
    }

    #[test]
    fn protection_keeps_local_backups() {
        let mut env = Env::new();
        let (g, [_a, b, _c, d]) = forked(&mut env);
        env.app.config.backup.protect_local = true;
        let lock = env.app.repo.lock().unwrap();
        let err = env.app.delete_branch(&lock, &g, "alt", true).unwrap_err();
        assert!(err.to_string().contains("存档安全保护"), "{err}");
        let err = env
            .app
            .delete_snapshot(&lock, &g, &b.to_string(), false)
            .unwrap_err();
        assert!(err.to_string().contains("本机备份"), "{err}");
        assert!(env.app.repo.has(bonfire_core::Kind::Snapshot, &d));
        // 只是指针、不会删除快照的操作不受影响：删除标签。
        env.app
            .create_ref(&lock, &g, RefKind::Tag, "t", Some(&b.to_string()))
            .unwrap();
        env.app.delete_tag(&lock, &g, "t").unwrap();
        // 别的电脑备份的快照可以删除。
        env.app.config.backup.protect_local = false;
        let lock2 = &lock;
        let mut s = env.app.repo.get_snapshot(&d).unwrap();
        s.host = "OTHER-PC".into();
        s.parents = vec![d];
        let other = env.app.repo.put_snapshot(&s).unwrap();
        env.app.index.add_snapshot(&other, &s, "x").unwrap();
        env.app
            .create_ref(
                lock2,
                &g,
                RefKind::Branch,
                "friend",
                Some(&other.to_string()),
            )
            .unwrap();
        env.app.config.backup.protect_local = true;
        let r = env.app.delete_branch(&lock, &g, "friend", false).unwrap();
        assert_eq!(r.removed, vec![other]);
    }
}
