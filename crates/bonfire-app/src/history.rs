//! 历史服务：快照时间线、差异、备注修改、分支与标签（文档 §3 历史服务）。

use std::collections::{BTreeMap, HashMap};

use bonfire_core::index::GameRecord;
use bonfire_core::refs::{RefKind, RefName};
use bonfire_core::scan;
use bonfire_core::{ObjectId, RepoLock, TreeEntry};
use serde::Serialize;

use crate::app::App;
use crate::backup::check_comment;
use crate::error::Result;

/// 默认显示的快照数量。
pub const DEFAULT_LOG_LIMIT: usize = 50;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub id: ObjectId,
    pub created_at: String,
    pub trigger: String,
    /// 显示用备注（有修改时为修改后的备注）。
    pub comment: String,
    pub files: u64,
    pub bytes: u64,
    pub changed: u64,
    /// 指向该快照的分支与标签，如 `heads/main`、`tags/before-boss`。
    pub refs: Vec<String>,
    /// 是否为当前状态对应的快照。
    pub head: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffStatus {
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffEntry {
    pub path: String,
    pub status: DiffStatus,
    pub size_before: Option<u64>,
    pub size_after: Option<u64>,
}

impl App {
    /// 快照时间线：默认从当前状态沿第一父快照向前；指定分支时从分支最新快照开始；
    /// 指定 `grep` 时按备注搜索该游戏的全部快照。
    pub fn log(&self, game: &GameRecord, branch: Option<&str>, grep: Option<&str>, limit: usize) -> Result<Vec<LogEntry>> {
        let mut labels: HashMap<ObjectId, Vec<String>> = HashMap::new();
        for (name, id) in self.repo.list_refs(&game.id)? {
            labels.entry(id).or_default().push(name.to_string());
        }
        let head = self.repo.head_snapshot(&game.id)?;

        let ids: Vec<ObjectId> = match grep {
            Some(q) => self.index.search_comments(q, Some(&game.id))?.into_iter().take(limit).map(|r| r.id).collect(),
            None => {
                let start = match branch {
                    Some(b) => Some(self.repo.resolve(&game.id, &format!("heads/{b}"))?),
                    None => head,
                };
                match start {
                    Some(s) => self.repo.first_parent_chain(s, limit)?.into_iter().map(|(id, _)| id).collect(),
                    None => Vec::new(),
                }
            }
        };

        ids.into_iter()
            .map(|id| {
                let s = self.repo.get_snapshot(&id)?;
                Ok(LogEntry {
                    id,
                    created_at: s.created_at.clone(),
                    trigger: serde_json::to_value(s.trigger).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default(),
                    comment: self.repo.effective_comment(&id, &s)?,
                    files: s.stats.files,
                    bytes: s.stats.bytes,
                    changed: s.stats.changed,
                    refs: labels.remove(&id).unwrap_or_default(),
                    head: head == Some(id),
                })
            })
            .collect()
    }

    /// 快照 A 到快照 B 的差异；省略 B 时与当前磁盘比较。
    pub fn diff(&self, game: &GameRecord, a: &str, b: Option<&str>) -> Result<Vec<DiffEntry>> {
        let before = self.tree_map(&self.repo.resolve(&game.id, a)?)?;
        let mut out = Vec::new();
        match b {
            Some(b) => {
                let after = self.tree_map(&self.repo.resolve(&game.id, b)?)?;
                for (path, e) in &after {
                    match before.get(path) {
                        None => out.push(entry(path, DiffStatus::Added, None, Some(e.size))),
                        Some(p) if p.blob != e.blob => out.push(entry(path, DiffStatus::Modified, Some(p.size), Some(e.size))),
                        Some(_) => {}
                    }
                }
                for (path, p) in &before {
                    if !after.contains_key(path) {
                        out.push(entry(path, DiffStatus::Removed, Some(p.size), None));
                    }
                }
            }
            None => {
                let disk = scan::collect_files(&self.scan_roots(game)?)?;
                for f in &disk {
                    match before.get(&f.path) {
                        None => out.push(entry(&f.path, DiffStatus::Added, None, Some(f.size))),
                        Some(p) if p.size != f.size || scan::hash_file(&f.abs)? != p.blob => {
                            out.push(entry(&f.path, DiffStatus::Modified, Some(p.size), Some(f.size)))
                        }
                        Some(_) => {}
                    }
                }
                let on_disk: std::collections::HashSet<&str> = disk.iter().map(|f| f.path.as_str()).collect();
                for (path, p) in &before {
                    if !on_disk.contains(path.as_str()) {
                        out.push(entry(path, DiffStatus::Removed, Some(p.size), None));
                    }
                }
            }
        }
        out.sort_by(|x, y| x.path.cmp(&y.path));
        Ok(out)
    }

    fn tree_map(&self, snap: &ObjectId) -> Result<BTreeMap<String, TreeEntry>> {
        let tree = self.repo.get_tree(&self.repo.get_snapshot(snap)?.tree)?;
        Ok(tree.entries.into_iter().map(|e| (e.path.clone(), e)).collect())
    }

    /// 修改快照备注（写入注记，保留修改历史）。
    pub fn set_comment(&mut self, lock: &RepoLock, game: &GameRecord, spec: &str, comment: &str) -> Result<ObjectId> {
        check_comment(comment)?;
        let id = self.repo.resolve(&game.id, spec)?;
        self.repo.set_note(lock, &id, comment.trim())?;
        self.index.update_comment(&id, comment.trim())?;
        Ok(id)
    }

    /// 在指定快照（默认当前状态）上创建分支或标签。
    pub fn create_ref(&mut self, lock: &RepoLock, game: &GameRecord, kind: RefKind, name: &str, spec: Option<&str>) -> Result<ObjectId> {
        let id = self.repo.resolve(&game.id, spec.unwrap_or("HEAD"))?;
        match kind {
            RefKind::Branch => self.repo.create_branch(lock, &game.id, name, id)?,
            RefKind::Tag => self.repo.create_tag(lock, &game.id, name, id)?,
        }
        self.index.set_refs(&game.id, &self.repo.list_refs(&game.id)?)?;
        Ok(id)
    }

    pub fn refs(&self, game: &GameRecord) -> Result<Vec<(RefName, ObjectId)>> {
        Ok(self.repo.list_refs(&game.id)?)
    }

    pub fn fsck(&self, deep: bool) -> Result<bonfire_core::fsck::FsckReport> {
        Ok(bonfire_core::fsck::fsck(&self.repo, deep)?)
    }
}

fn entry(path: &str, status: DiffStatus, size_before: Option<u64>, size_after: Option<u64>) -> DiffEntry {
    DiffEntry { path: path.to_string(), status, size_before, size_after }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Env;
    use crate::{BackupOptions, BackupResult};

    fn backup(env: &mut Env, g: &GameRecord, comment: &str) -> ObjectId {
        let lock = env.app.repo.lock().unwrap();
        let opts = BackupOptions { comment: Some(comment.into()), ..Default::default() };
        match env.app.backup(&lock, g, &opts).unwrap() {
            BackupResult::Created { snapshot, .. } => snapshot,
            BackupResult::NoChanges => panic!("应生成快照"),
        }
    }

    #[test]
    fn log_diff_comment_and_refs() {
        let mut env = Env::new();
        env.write("a", b"1");
        env.write("b", b"1");
        let g = env.add_game("G");
        let s1 = backup(&mut env, &g, "第一章");
        env.write("a", b"22");
        std::fs::remove_file(env.saves().join("b")).unwrap();
        env.write("c", b"3");
        let s2 = backup(&mut env, &g, "第二章 Boss 前");

        let log = env.app.log(&g, None, None, 10).unwrap();
        assert_eq!(log.iter().map(|e| e.id).collect::<Vec<_>>(), [s2, s1]);
        assert!(log[0].head);
        assert_eq!(log[0].refs, ["heads/main"]);
        assert_eq!(env.app.log(&g, None, Some("Boss"), 10).unwrap()[0].id, s2);
        assert_eq!(env.app.log(&g, None, None, 1).unwrap().len(), 1);

        let d = env.app.diff(&g, &s1.to_string(), Some(&s2.to_string())).unwrap();
        let st: Vec<_> = d.iter().map(|e| (e.path.as_str(), e.status)).collect();
        assert_eq!(st, [("0/a", DiffStatus::Modified), ("0/b", DiffStatus::Removed), ("0/c", DiffStatus::Added)]);

        // 与磁盘比较。
        env.write("c", b"changed");
        let d = env.app.diff(&g, "HEAD", None).unwrap();
        assert_eq!(d.iter().map(|e| (e.path.as_str(), e.status)).collect::<Vec<_>>(), [("0/c", DiffStatus::Modified)]);

        let lock = env.app.repo.lock().unwrap();
        env.app.set_comment(&lock, &g, "HEAD~1", "第一章（改）").unwrap();
        env.app.create_ref(&lock, &g, RefKind::Tag, "ch1", Some("HEAD~1")).unwrap();
        env.app.create_ref(&lock, &g, RefKind::Branch, "二周目", None).unwrap();
        drop(lock);
        let log = env.app.log(&g, None, None, 10).unwrap();
        assert_eq!(log[1].comment, "第一章（改）");
        assert_eq!(log[1].refs, ["tags/ch1"]);
        assert_eq!(log[0].refs, ["heads/main", "heads/二周目"]);
        assert_eq!(env.app.log(&g, None, Some("改"), 10).unwrap().len(), 1);
        assert!(env.app.fsck(true).unwrap().ok());
    }
}
