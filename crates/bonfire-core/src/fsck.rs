//! 仓库校验（文档 10.1 可靠性）：校验所有对象的哈希与引用完整性。

use std::collections::HashSet;

use serde::Serialize;

use crate::error::Result;
use crate::object::Kind;
use crate::refs::{RefKind, RefName};
use crate::{CoreError, ObjectId, Repo};

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsckReport {
    pub chunks: u64,
    pub blobs: u64,
    pub trees: u64,
    pub snapshots: u64,
    pub refs: u64,
    /// 损坏或缺失的对象、指向不存在快照的引用等。非空即校验失败。
    pub errors: Vec<String>,
    /// 不被任何引用（含 HEAD 与 reflog）指向的快照，可由 GC 清理，不算错误。
    pub unreachable_snapshots: u64,
    /// 写入中断留下的临时文件。
    pub tmp_files: u64,
}

impl FsckReport {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// 校验整个仓库。`deep` 时还原每个文件并校验整个文件的哈希。
pub fn fsck(repo: &Repo, deep: bool) -> Result<FsckReport> {
    let mut r = FsckReport::default();

    let chunks: HashSet<ObjectId> = repo.list_objects(Kind::Chunk)?.into_iter().collect();
    for id in &chunks {
        r.chunks += 1;
        if let Err(e) = repo.get_chunk(id) {
            err(&mut r, e);
        }
    }

    let blobs: HashSet<ObjectId> = repo.list_objects(Kind::Blob)?.into_iter().collect();
    for id in &blobs {
        r.blobs += 1;
        match repo.get_blob(id) {
            Ok(b) => {
                if let Some(missing) = b.chunks.iter().find(|c| !chunks.contains(c)) {
                    r.errors.push(format!("文件对象 {id} 引用的分块 {missing} 不存在"));
                } else if deep && let Err(e) = repo.read_blob_to(id, std::io::sink(), repo.root()) {
                    err(&mut r, e);
                }
            }
            Err(e) => err(&mut r, e),
        }
    }

    let trees: HashSet<ObjectId> = repo.list_objects(Kind::Tree)?.into_iter().collect();
    for id in &trees {
        r.trees += 1;
        match repo.get_tree(id) {
            Ok(t) => {
                for e in t.entries.iter().filter(|e| !blobs.contains(&e.blob)) {
                    r.errors.push(format!("清单 {id} 中的文件 {} 引用的对象 {} 不存在", e.path, e.blob));
                }
            }
            Err(e) => err(&mut r, e),
        }
    }

    let snapshots: HashSet<ObjectId> = repo.list_objects(Kind::Snapshot)?.into_iter().collect();
    for id in &snapshots {
        r.snapshots += 1;
        match repo.get_snapshot(id) {
            Ok(s) => {
                if !trees.contains(&s.tree) {
                    r.errors.push(format!("快照 {id} 的清单 {} 不存在", s.tree));
                }
                for p in s.parents.iter().filter(|p| !snapshots.contains(p)) {
                    r.errors.push(format!("快照 {id} 的父快照 {p} 不存在"));
                }
            }
            Err(e) => err(&mut r, e),
        }
    }

    // 引用：分支、标签、HEAD 与 reflog 中出现的快照都视为可达。
    let mut reachable: Vec<ObjectId> = Vec::new();
    for game in repo.games()? {
        let refs = match repo.list_refs(&game) {
            Ok(v) => v,
            Err(e) => {
                err(&mut r, e);
                continue;
            }
        };
        for (name, id) in refs {
            r.refs += 1;
            check_ref_target(repo, &game, &name.to_string(), &id, &snapshots, &mut r);
            reachable.push(id);
        }
        match repo.head(&game) {
            Ok(h) => {
                if let Some(d) = h.detached {
                    check_ref_target(repo, &game, "HEAD", &d, &snapshots, &mut r);
                    reachable.push(d);
                }
                if repo.read_ref(&game, &RefName { kind: RefKind::Branch, name: h.branch.clone() })?.is_none() && h.detached.is_none() {
                    let any_branch = repo.list_refs(&game)?.iter().any(|(n, _)| n.kind == RefKind::Branch);
                    if any_branch {
                        r.errors.push(format!("游戏 {game} 的 HEAD 指向不存在的分支 {}", h.branch));
                    }
                }
            }
            Err(e) => err(&mut r, e),
        }
        for entry in repo.reflog(&game)? {
            reachable.extend(entry.new.iter().chain(entry.old.iter()).copied().filter(|id| snapshots.contains(id)));
        }
    }

    // 从可达快照沿父链标记。
    let mut seen: HashSet<ObjectId> = HashSet::new();
    while let Some(id) = reachable.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let Ok(s) = repo.get_snapshot(&id) {
            reachable.extend(s.parents.iter().copied().filter(|p| snapshots.contains(p)));
        }
    }
    r.unreachable_snapshots = snapshots.iter().filter(|id| !seen.contains(id)).count() as u64;

    r.tmp_files = std::fs::read_dir(repo.tmp_dir()).map(|rd| rd.count() as u64).unwrap_or(0);
    Ok(r)
}

fn err(r: &mut FsckReport, e: CoreError) {
    r.errors.push(e.to_string());
}

fn check_ref_target(repo: &Repo, game: &str, name: &str, id: &ObjectId, snapshots: &HashSet<ObjectId>, r: &mut FsckReport) {
    if !snapshots.contains(id) {
        r.errors.push(format!("游戏 {game} 的引用 {name} 指向不存在的快照 {id}"));
        return;
    }
    if let Ok(s) = repo.get_snapshot(id)
        && s.game != game
    {
        r.errors.push(format!("游戏 {game} 的引用 {name} 指向其他游戏（{}）的快照 {id}", s.game));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{FileTag, Snapshot, SnapshotStats, Tree, TreeEntry, Trigger};
    use std::path::Path;

    fn snap(repo: &Repo, parents: Vec<ObjectId>, data: &[u8]) -> ObjectId {
        let blob = repo.write_file(data, Path::new("x")).unwrap().blob;
        let tree = repo.put_tree(&Tree::new(vec![TreeEntry { path: "0/x".into(), blob, size: data.len() as u64, mtime_ns: 0, tag: FileTag::Save }])).unwrap();
        repo.put_snapshot(&Snapshot {
            game: "custom-g".into(),
            tree,
            parents,
            created_at: "2026-10-09T12:00:00+08:00".into(),
            trigger: Trigger::Manual,
            host: "PC".into(),
            comment: String::new(),
            stats: SnapshotStats::default(),
            playtime_min: None,
        })
        .unwrap()
    }

    #[test]
    fn healthy_repo_passes() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repo::init(dir.path()).unwrap();
        let lock = repo.lock().unwrap();
        let a = snap(&repo, vec![], b"1");
        let b = snap(&repo, vec![a], b"2");
        repo.update_ref(&lock, "custom-g", &RefName::branch("main"), None, b, "backup", "").unwrap();
        let _orphan = snap(&repo, vec![], b"orphan");
        let r = fsck(&repo, true).unwrap();
        assert!(r.ok(), "{:?}", r.errors);
        assert_eq!((r.snapshots, r.refs, r.unreachable_snapshots), (3, 1, 1));
    }

    #[test]
    fn detects_missing_and_corrupt_objects() {
        let dir = tempfile::tempdir().unwrap();
        let repo = Repo::init(dir.path()).unwrap();
        let lock = repo.lock().unwrap();
        let a = snap(&repo, vec![], b"first");
        repo.update_ref(&lock, "custom-g", &RefName::branch("main"), None, a, "backup", "").unwrap();
        // 删除唯一的分块。
        let chunk = repo.list_objects(Kind::Chunk).unwrap()[0];
        let hex = chunk.to_hex();
        std::fs::remove_file(repo.objects_dir().join("chunks").join(&hex[..2]).join(&hex[2..])).unwrap();
        // 引用指向不存在的快照。
        repo.update_ref(&lock, "custom-g", &RefName::branch("ghost"), None, ObjectId::of(b"ghost"), "branch", "").unwrap();
        let r = fsck(&repo, false).unwrap();
        assert!(!r.ok());
        assert_eq!(r.errors.len(), 2, "{:?}", r.errors);
    }
}
