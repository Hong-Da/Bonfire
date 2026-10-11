//! 垃圾回收（文档 6.6）：从所有分支、标签、HEAD 与 30 天内 reflog 中出现的快照出发，
//! 沿父快照 → 清单 → 文件 → 分块标记，删除没有被标记的对象。
//!
//! - 调用方持有写入锁，期间不会有新对象写入；另外按 `grace` 跳过最近写入的对象，多一层保险。
//! - 删除顺序为快照 → 清单 → 文件 → 分块：中途失败时留下的都是“被引用的对象还在”的状态，fsck 仍通过。
//! - 只删对象库中的文件；索引中的快照记录与备注由调用方随后清理（见 [`GcReport::snapshots`]）。

use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use serde::Serialize;

use crate::error::{IoContext, Result};
use crate::object::Kind;
use crate::{ObjectId, Repo, RepoLock};

/// reflog 中的快照保留天数（文档 6.4）。
pub const REFLOG_DAYS: i64 = 30;

#[derive(Debug, Clone)]
pub struct GcOptions {
    pub dry_run: bool,
    /// 修改时间在这之内的对象不删除。
    pub grace: Duration,
    /// 除引用之外还要保留的快照（如可撤销的恢复操作记录的快照）。
    pub extra_roots: Vec<ObjectId>,
    /// 视为已删除的快照（预览历史清理能释放多少空间时用）：不作为起点，也不沿它们标记。
    pub assume_removed: HashSet<ObjectId>,
}

impl Default for GcOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            grace: Duration::from_secs(3600),
            extra_roots: Vec::new(),
            assume_removed: HashSet::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GcReport {
    /// 删除（预览时为将删除）的各类对象数。
    pub chunks: u64,
    pub blobs: u64,
    pub trees: u64,
    /// 删除的快照（不含 `assume_removed` 中的）。
    pub snapshots: Vec<ObjectId>,
    /// 释放的磁盘空间（对象文件大小之和）。
    pub bytes: u64,
    /// 删除的写入中断留下的临时文件。
    pub tmp_files: u64,
    pub dry_run: bool,
}

impl GcReport {
    pub fn objects(&self) -> u64 {
        self.chunks + self.blobs + self.trees + self.snapshots.len() as u64
    }
}

/// 起点快照：所有游戏的分支、标签、HEAD 与 `REFLOG_DAYS` 天内 reflog 中出现的快照。
pub fn roots(repo: &Repo) -> Result<Vec<ObjectId>> {
    let cutoff = jiff::Timestamp::now() - jiff::SignedDuration::from_hours(24 * REFLOG_DAYS);
    let mut games: HashSet<String> = repo.games()?.into_iter().collect();
    let logs = repo.root().join("logs");
    if let Ok(rd) = std::fs::read_dir(&logs) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(g) = name.strip_suffix(".jsonl") {
                games.insert(g.to_string());
            }
        }
    }
    let mut out = Vec::new();
    for game in &games {
        if repo.root().join("refs").join(game).is_dir() {
            out.extend(repo.list_refs(game)?.into_iter().map(|(_, id)| id));
            out.extend(repo.head(game)?.detached);
        }
        for e in repo.reflog(game)? {
            if e.time >= cutoff {
                out.extend(e.old);
                out.extend(e.new);
            }
        }
    }
    Ok(out)
}

/// 执行（或预览）垃圾回收。
pub fn gc(repo: &Repo, _lock: &RepoLock, opts: &GcOptions) -> Result<GcReport> {
    let all_snaps: HashSet<ObjectId> = repo.list_objects(Kind::Snapshot)?.into_iter().collect();

    // 1. 快照：沿父链标记。视为已删除的快照不标记，但它的父快照仍要保留（子快照会改接过去）。
    let mut snaps: HashSet<ObjectId> = HashSet::new();
    let mut passed: HashSet<ObjectId> = HashSet::new();
    let mut stack: Vec<ObjectId> = roots(repo)?;
    stack.extend(opts.extra_roots.iter().copied());
    while let Some(id) = stack.pop() {
        if !all_snaps.contains(&id) {
            continue;
        }
        let first = if opts.assume_removed.contains(&id) {
            passed.insert(id)
        } else {
            snaps.insert(id)
        };
        if first {
            stack.extend(repo.get_snapshot(&id)?.parents);
        }
    }

    // 2. 清单、文件、分块。
    let mut trees = HashSet::new();
    for id in &snaps {
        trees.insert(repo.get_snapshot(id)?.tree);
    }
    let mut blobs = HashSet::new();
    for id in &trees {
        if let Ok(t) = repo.get_tree(id) {
            blobs.extend(t.entries.into_iter().map(|e| e.blob));
        }
    }
    let mut chunks = HashSet::new();
    for id in &blobs {
        if let Ok(b) = repo.get_blob(id) {
            chunks.extend(b.chunks);
        }
    }

    // 3. 删除未标记的对象。
    let mut report = GcReport {
        dry_run: opts.dry_run,
        ..Default::default()
    };
    let now = SystemTime::now();
    for (kind, keep) in [
        (Kind::Snapshot, &snaps),
        (Kind::Tree, &trees),
        (Kind::Blob, &blobs),
        (Kind::Chunk, &chunks),
    ] {
        for id in repo.list_objects(kind)? {
            if keep.contains(&id) {
                continue;
            }
            let assumed = kind == Kind::Snapshot && opts.assume_removed.contains(&id);
            let path = repo.object_path(kind, &id);
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let recent = meta
                .modified()
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .is_some_and(|age| age < opts.grace);
            if recent && !assumed {
                continue;
            }
            report.bytes += meta.len();
            match kind {
                Kind::Snapshot if assumed => {}
                Kind::Snapshot => report.snapshots.push(id),
                Kind::Tree => report.trees += 1,
                Kind::Blob => report.blobs += 1,
                Kind::Chunk => report.chunks += 1,
            }
            if !opts.dry_run && !assumed {
                match std::fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(crate::CoreError::io(path, e)),
                }
            }
        }
    }

    // 4. 写入中断留下的临时文件。
    let tmp = repo.tmp_dir();
    if let Ok(rd) = std::fs::read_dir(&tmp) {
        for e in rd.flatten() {
            let old = e
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|m| now.duration_since(m).ok())
                .is_some_and(|age| age >= opts.grace);
            if old {
                report.tmp_files += 1;
                if !opts.dry_run {
                    std::fs::remove_file(e.path()).at(e.path())?;
                }
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{Snapshot, SnapshotStats, Tree, TreeEntry, Trigger};
    use crate::refs::RefName;

    fn now_opts() -> GcOptions {
        GcOptions {
            grace: Duration::ZERO,
            ..Default::default()
        }
    }

    fn snap(repo: &Repo, game: &str, data: &[u8], parents: Vec<ObjectId>) -> ObjectId {
        let stats = repo.write_file(data, std::path::Path::new("x")).unwrap();
        let tree = repo
            .put_tree(&Tree::new(vec![TreeEntry {
                path: "save/x".into(),
                blob: stats.blob,
                size: stats.size,
                mtime_ns: 0,
                tag: crate::FileTag::Save,
            }]))
            .unwrap();
        repo.put_snapshot(&Snapshot {
            game: game.into(),
            tree,
            parents,
            created_at: "2026-10-11T00:00:00+08:00".into(),
            trigger: Trigger::Manual,
            host: "pc".into(),
            comment: String::new(),
            stats: SnapshotStats::default(),
            playtime_min: None,
        })
        .unwrap()
    }

    #[test]
    fn keeps_referenced_history_and_removes_the_rest() {
        let d = tempfile::tempdir().unwrap();
        let repo = Repo::init(d.path()).unwrap();
        let lock = repo.lock().unwrap();
        let a = snap(&repo, "g", b"one", vec![]);
        let b = snap(&repo, "g", b"two", vec![a]);
        repo.update_ref(&lock, "g", &RefName::branch("main"), None, b, "backup", "")
            .unwrap();
        // 没有任何引用的快照与它独有的内容（如中断的备份）。
        let orphan = snap(&repo, "g", b"lost", vec![b]);
        // reflog 中删掉的旧快照不会被找回，但 reflog 指向的快照在 30 天内保留：这里直接写一个无记录的对象。
        let stray = repo.put_chunk(b"stray chunk").unwrap().0;

        let preview = gc(
            &repo,
            &lock,
            &GcOptions {
                dry_run: true,
                ..now_opts()
            },
        )
        .unwrap();
        assert_eq!(preview.snapshots, vec![orphan]);
        assert!(repo.has(Kind::Snapshot, &orphan), "预览不删除");

        let r = gc(&repo, &lock, &now_opts()).unwrap();
        assert_eq!(r.snapshots, vec![orphan]);
        assert_eq!((r.trees, r.blobs, r.chunks), (1, 1, 2));
        assert!(r.bytes > 0);
        assert!(!repo.has(Kind::Chunk, &stray));
        assert!(repo.has(Kind::Snapshot, &a) && repo.has(Kind::Snapshot, &b));
        assert!(crate::fsck::fsck(&repo, true).unwrap().ok());
        // 再运行一次没有可删的。
        assert_eq!(gc(&repo, &lock, &now_opts()).unwrap().objects(), 0);
    }

    #[test]
    fn grace_period_and_extra_roots() {
        let d = tempfile::tempdir().unwrap();
        let repo = Repo::init(d.path()).unwrap();
        let lock = repo.lock().unwrap();
        let a = snap(&repo, "g", b"one", vec![]);
        // 刚写入的对象在保留期内不删除。
        assert_eq!(
            gc(&repo, &lock, &GcOptions::default()).unwrap().objects(),
            0
        );
        // 额外保留的快照。
        let r = gc(
            &repo,
            &lock,
            &GcOptions {
                extra_roots: vec![a],
                ..now_opts()
            },
        )
        .unwrap();
        assert_eq!(r.objects(), 0);
        assert!(repo.has(Kind::Snapshot, &a));
    }

    #[test]
    fn preview_with_assumed_removed_snapshot_keeps_its_parent() {
        let d = tempfile::tempdir().unwrap();
        let repo = Repo::init(d.path()).unwrap();
        let lock = repo.lock().unwrap();
        let a = snap(&repo, "g", b"one", vec![]);
        let b = snap(&repo, "g", b"two", vec![a]);
        let c = snap(&repo, "g", b"three", vec![b]);
        repo.update_ref(&lock, "g", &RefName::branch("main"), None, c, "backup", "")
            .unwrap();
        let r = gc(
            &repo,
            &lock,
            &GcOptions {
                dry_run: true,
                assume_removed: [b].into_iter().collect(),
                ..now_opts()
            },
        )
        .unwrap();
        // b 的清单、文件与分块可释放；a 仍是 c 的祖先。
        assert!(r.snapshots.is_empty());
        assert_eq!((r.trees, r.blobs, r.chunks), (1, 1, 1));
        assert!(r.bytes > 0);
    }
}
