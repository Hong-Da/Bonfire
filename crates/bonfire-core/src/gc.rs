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
    /// 截图相册中的文件 ID：不被快照使用、但与截图相同的文件对象归为“早期版本留下的截图”。
    pub shot_blobs: HashSet<ObjectId>,
}

impl Default for GcOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            grace: Duration::from_secs(3600),
            extra_roots: Vec::new(),
            assume_removed: HashSet::new(),
            shot_blobs: HashSet::new(),
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
    /// 释放的空间按来源分类（只列出有内容的来源）。
    pub sources: Vec<GcSource>,
    pub dry_run: bool,
}

/// 释放空间的来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
    /// 这次清理（按保留规则或选中）删除的快照独有的内容。
    Planned,
    /// 不在任何路线上、引用日志也已超过 30 天的快照（如“移动当前分支”后留下的）。
    Unreachable,
    /// 之前删除的快照、分支或游戏历史留下的内容（删除时只删记录，空间留到清理时回收）。
    Deleted,
    /// 早期版本存放在对象库中的截图（截图已改为普通文件）。
    Shots,
    /// 其他：中断的备份等没有快照使用的内容。
    Other,
    /// 写入中断留下的临时文件。
    Tmp,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GcSource {
    pub kind: SourceKind,
    pub objects: u64,
    pub bytes: u64,
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
    let (trees, blobs, chunks) = mark_content(repo, &snaps)?;
    // 不假设删除时的可达内容：两者之差就是这次清理删除的快照独有的内容。
    let baseline = if opts.assume_removed.is_empty() {
        None
    } else {
        let mut all = snaps.clone();
        all.extend(passed.iter().copied());
        let (t, b, c) = mark_content(repo, &all)?;
        Some(Marked {
            snaps: all,
            trees: t,
            blobs: b,
            chunks: c,
        })
    };

    // 3. 找出未标记的对象（跳过最近写入的）并记下大小。
    let mut report = GcReport {
        dry_run: opts.dry_run,
        ..Default::default()
    };
    let now = SystemTime::now();
    let mut freed: Vec<Freed> = Vec::new();
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
            freed.push((kind, id, path, meta.len()));
        }
    }

    // 4. 按来源分类（每个对象只算一次）。
    report.sources = classify(repo, &freed, baseline.as_ref(), opts)?;

    // 5. 删除：快照 → 清单 → 文件 → 分块。
    if !opts.dry_run {
        for (kind, id, path, _) in &freed {
            if *kind == Kind::Snapshot && opts.assume_removed.contains(id) {
                continue;
            }
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(crate::CoreError::io(path.clone(), e)),
            }
        }
    }

    // 6. 写入中断留下的临时文件。
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
                let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                report.bytes += size;
                add_source(&mut report.sources, SourceKind::Tmp, size);
                if !opts.dry_run {
                    std::fs::remove_file(e.path()).at(e.path())?;
                }
            }
        }
    }
    Ok(report)
}

type Content = (HashSet<ObjectId>, HashSet<ObjectId>, HashSet<ObjectId>);

/// 快照、清单、文件、分块的标记集合。
struct Marked {
    snaps: HashSet<ObjectId>,
    trees: HashSet<ObjectId>,
    blobs: HashSet<ObjectId>,
    chunks: HashSet<ObjectId>,
}

/// 将释放的对象：类型、ID、文件路径、大小。
type Freed = (Kind, ObjectId, std::path::PathBuf, u64);

/// 快照用到的清单、文件与分块。
fn mark_content(repo: &Repo, snaps: &HashSet<ObjectId>) -> Result<Content> {
    let mut trees = HashSet::new();
    for id in snaps {
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
    Ok((trees, blobs, chunks))
}

fn add_source(sources: &mut Vec<GcSource>, kind: SourceKind, bytes: u64) {
    match sources.iter_mut().find(|s| s.kind == kind) {
        Some(s) => {
            s.objects += 1;
            s.bytes += bytes;
        }
        None => sources.push(GcSource {
            kind,
            objects: 1,
            bytes,
        }),
    }
}

/// 释放的对象按来源归类：这次删除的快照 → 不在路线上的快照 → 之前删除的快照留下的清单 → 早期版本的截图 → 其他。
fn classify(
    repo: &Repo,
    freed: &[Freed],
    baseline: Option<&Marked>,
    opts: &GcOptions,
) -> Result<Vec<GcSource>> {
    use std::collections::HashMap;
    let size: HashMap<(Kind, ObjectId), u64> =
        freed.iter().map(|(k, id, _, n)| ((*k, *id), *n)).collect();
    let mut owner: HashMap<(Kind, ObjectId), SourceKind> = HashMap::new();

    // 从一个对象出发，把它和它用到的、尚未归类的被释放对象归到 `kind`。
    let claim = |start: (Kind, ObjectId),
                 kind: SourceKind,
                 owner: &mut HashMap<(Kind, ObjectId), SourceKind>|
     -> Result<()> {
        let mut stack = vec![start];
        while let Some((k, id)) = stack.pop() {
            if !size.contains_key(&(k, id)) || owner.contains_key(&(k, id)) {
                continue;
            }
            owner.insert((k, id), kind);
            match k {
                Kind::Snapshot => {
                    if let Ok(s) = repo.get_snapshot(&id) {
                        stack.push((Kind::Tree, s.tree));
                    }
                }
                Kind::Tree => {
                    if let Ok(t) = repo.get_tree(&id) {
                        stack.extend(t.entries.into_iter().map(|e| (Kind::Blob, e.blob)));
                    }
                }
                Kind::Blob => {
                    if let Ok(b) = repo.get_blob(&id) {
                        stack.extend(b.chunks.into_iter().map(|c| (Kind::Chunk, c)));
                    }
                }
                Kind::Chunk => {}
            }
        }
        Ok(())
    };

    if let Some(Marked {
        snaps,
        trees,
        blobs,
        chunks,
    }) = baseline
    {
        for (k, id, _, _) in freed {
            let reachable = match k {
                Kind::Snapshot => snaps.contains(id),
                Kind::Tree => trees.contains(id),
                Kind::Blob => blobs.contains(id),
                Kind::Chunk => chunks.contains(id),
            };
            if reachable {
                owner.insert((*k, *id), SourceKind::Planned);
            }
        }
    }
    for (k, id, _, _) in freed.iter().filter(|(k, _, _, _)| *k == Kind::Snapshot) {
        claim((*k, *id), SourceKind::Unreachable, &mut owner)?;
    }
    for (k, id, _, _) in freed.iter().filter(|(k, _, _, _)| *k == Kind::Tree) {
        claim((*k, *id), SourceKind::Deleted, &mut owner)?;
    }
    for (k, id, _, _) in freed
        .iter()
        .filter(|(k, id, _, _)| *k == Kind::Blob && opts.shot_blobs.contains(id))
    {
        claim((*k, *id), SourceKind::Shots, &mut owner)?;
    }
    for (k, id, _, _) in freed {
        claim((*k, *id), SourceKind::Other, &mut owner)?;
    }

    let mut sources: Vec<GcSource> = Vec::new();
    for (k, id, _, n) in freed {
        add_source(&mut sources, owner[&(*k, *id)], *n);
    }
    let order = |k: SourceKind| k as u8;
    sources.sort_by_key(|s| order(s.kind));
    Ok(sources)
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
        // 来源：不在路线上的快照（含它独有的内容）与其他（没有快照使用的分块）。
        let kinds: Vec<(SourceKind, u64)> = r.sources.iter().map(|s| (s.kind, s.objects)).collect();
        assert_eq!(
            kinds,
            [(SourceKind::Unreachable, 4), (SourceKind::Other, 1)]
        );
        assert_eq!(r.sources.iter().map(|s| s.bytes).sum::<u64>(), r.bytes);
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
        assert_eq!(r.sources.len(), 1);
        assert_eq!(r.sources[0].kind, SourceKind::Planned);
    }
}
