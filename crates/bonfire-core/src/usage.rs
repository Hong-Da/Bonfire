//! 空间统计（总览页）：一组快照在仓库中实际占用的空间与原始大小。
//!
//! 占用按对象文件在磁盘上的大小计算（压缩、加密后），同一组快照内共用的对象只算一次；
//! 不同游戏之间共用的对象极少（内容不同），按游戏分别统计后相加会略大于实际。

use std::collections::HashSet;

use crate::error::Result;
use crate::object::Kind;
use crate::{ObjectId, Repo};

/// 一组快照的空间。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    pub snapshots: u64,
    /// 用到的对象在磁盘上的大小。
    pub stored: u64,
    /// 各快照中文件原始大小之和（不去重）。
    pub logical: u64,
}

/// 统计这些快照用到的快照、清单、文件与分块对象。读不到的对象跳过（由 fsck 报告）。
pub fn usage(repo: &Repo, snaps: &[ObjectId]) -> Result<Usage> {
    let mut u = Usage::default();
    let mut trees = HashSet::new();
    for id in snaps {
        let Ok(s) = repo.get_snapshot(id) else {
            continue;
        };
        u.snapshots += 1;
        u.logical += s.stats.bytes;
        u.stored += repo.object_size(Kind::Snapshot, id);
        trees.insert(s.tree);
    }
    let mut blobs = HashSet::new();
    for id in &trees {
        u.stored += repo.object_size(Kind::Tree, id);
        if let Ok(t) = repo.get_tree(id) {
            blobs.extend(t.entries.into_iter().map(|e| e.blob));
        }
    }
    let mut chunks = HashSet::new();
    for id in &blobs {
        u.stored += repo.object_size(Kind::Blob, id);
        if let Ok(b) = repo.get_blob(id) {
            chunks.extend(b.chunks);
        }
    }
    for id in &chunks {
        u.stored += repo.object_size(Kind::Chunk, id);
    }
    Ok(u)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{Snapshot, SnapshotStats, Tree, TreeEntry, Trigger};

    fn snap(repo: &Repo, data: &[u8], parents: Vec<ObjectId>) -> ObjectId {
        let stats = repo.write_file(data, std::path::Path::new("x")).unwrap();
        let tree = repo
            .put_tree(&Tree::new(vec![TreeEntry {
                path: "0/x".into(),
                blob: stats.blob,
                size: stats.size,
                mtime_ns: 0,
                tag: crate::FileTag::Save,
            }]))
            .unwrap();
        repo.put_snapshot(&Snapshot {
            game: "g".into(),
            tree,
            parents,
            created_at: "2026-10-11T00:00:00+08:00".into(),
            trigger: Trigger::Manual,
            host: "pc".into(),
            comment: String::new(),
            stats: SnapshotStats {
                files: 1,
                bytes: data.len() as u64,
                changed: 1,
            },
            playtime_min: None,
        })
        .unwrap()
    }

    #[test]
    fn shared_objects_count_once() {
        let d = tempfile::tempdir().unwrap();
        let repo = Repo::init(d.path()).unwrap();
        let data = vec![7u8; 100_000];
        let a = snap(&repo, &data, vec![]);
        let one = usage(&repo, &[a]).unwrap();
        assert_eq!((one.snapshots, one.logical), (1, 100_000));
        assert!(one.stored > 0 && one.stored < 100_000, "压缩后更小");
        // 内容相同的第二个快照：只多一个快照对象（清单与文件相同）。
        let b = snap(&repo, &data, vec![a]);
        let two = usage(&repo, &[a, b]).unwrap();
        assert_eq!((two.snapshots, two.logical), (2, 200_000));
        assert_eq!(
            two.stored,
            one.stored + repo.object_size(Kind::Snapshot, &b)
        );
        assert_eq!(usage(&repo, &[]).unwrap(), Usage::default());
    }
}
