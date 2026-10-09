//! 变更检测（文档 §5）：先比较文件元数据，元数据变化的文件再计算内容哈希确认。
//!
//! 单个文件判定：
//! 1. 索引中无此路径 → 新增。
//! 2. 大小与 mtime 都未变 → 未变化（不读内容），除非处于 racy 窗口。
//! 3. 任一变化 → 计算 BLAKE3；与索引不同 → 已修改，相同 → 未变化（调用方据此刷新索引中的 mtime）。
//! 4. 索引中有、磁盘上无 → 已删除。
//!
//! racy：mtime 与写入索引的时间相差不到 2 秒时（FAT32、网络盘的 mtime 精度为 2 秒），
//! 即使元数据相同也重算哈希，与 Git 的 racy-git 处理一致。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rayon::prelude::*;
use serde::Serialize;

use crate::error::{CoreError, IoContext, Result};
use crate::index::IndexEntry;
use crate::object::FileTag;
use crate::ObjectId;

pub const RACY_WINDOW_NS: i64 = 2_000_000_000;

/// 磁盘上的一个存档文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskFile {
    /// 清单中的相对路径：`<存档位置序号>/<相对路径>`，`/` 分隔。
    pub path: String,
    pub abs: PathBuf,
    pub size: u64,
    pub mtime_ns: i64,
    pub tag: FileTag,
}

/// 一个存档位置：序号作为清单路径前缀。
#[derive(Debug, Clone)]
pub struct ScanRoot {
    pub ord: usize,
    pub path: PathBuf,
    /// true 为单个文件，false 为目录（递归）。
    pub is_file: bool,
    pub tag: FileTag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    pub status: FileStatus,
    /// 磁盘上的大小；已删除的文件为索引中的大小。
    pub size: u64,
    /// 已知的内容哈希：计算过的、或元数据未变时取索引中的值；新增文件未计算时为空。
    pub hash: Option<ObjectId>,
    /// 内容未变但 mtime 变化，调用方应刷新索引。
    #[serde(skip)]
    pub refresh_index: bool,
}

pub fn mtime_ns(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

pub fn now_ns() -> i64 {
    std::time::SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos() as i64).unwrap_or(0)
}

/// 计算文件内容的 BLAKE3（即 Blob ID）。以共享读方式打开，不阻止游戏写入。
pub fn hash_file(path: &Path) -> Result<ObjectId> {
    let f = std::fs::File::open(path).at(path)?;
    let mut h = blake3::Hasher::new();
    h.update_reader(f).at(path)?;
    Ok(ObjectId::from_hash(h.finalize()))
}

/// 列出存档位置下的全部文件。不存在的位置视为没有文件；无法访问的子项跳过并记录日志。
pub fn collect_files(roots: &[ScanRoot]) -> Result<Vec<DiskFile>> {
    let mut out = Vec::new();
    for root in roots {
        if root.is_file {
            match std::fs::metadata(&root.path) {
                Ok(m) if m.is_file() => {
                    let name = root.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    out.push(DiskFile { path: format!("{}/{name}", root.ord), abs: root.path.clone(), size: m.len(), mtime_ns: mtime_ns(&m), tag: root.tag });
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(CoreError::io(&root.path, e)),
            }
            continue;
        }
        if !root.path.is_dir() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&root.path).follow_links(false).sort_by_file_name() {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    tracing::warn!(root = %root.path.display(), "跳过无法访问的项：{e}");
                    continue;
                }
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let meta = entry.metadata().map_err(|e| CoreError::io(entry.path(), std::io::Error::other(e.to_string())))?;
            let rel = entry.path().strip_prefix(&root.path).expect("walkdir 返回的路径在根目录下");
            let rel = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            out.push(DiskFile { path: format!("{}/{rel}", root.ord), abs: entry.into_path(), size: meta.len(), mtime_ns: mtime_ns(&meta), tag: root.tag });
        }
    }
    Ok(out)
}

/// 对比磁盘与索引，结果按路径排序。`force_verify` 时忽略元数据，全部重算哈希。
/// 哈希计算在线程池中并行进行。
pub fn detect(disk: &[DiskFile], index: &HashMap<String, IndexEntry>, force_verify: bool) -> Result<Vec<FileChange>> {
    let mut changes: Vec<FileChange> = disk
        .par_iter()
        .map(|f| -> Result<FileChange> {
            let Some(prev) = index.get(&f.path) else {
                return Ok(FileChange { path: f.path.clone(), status: FileStatus::Added, size: f.size, hash: None, refresh_index: false });
            };
            let racy = (f.mtime_ns - prev.indexed_at_ns).abs() < RACY_WINDOW_NS;
            let same_meta = f.size == prev.size && f.mtime_ns == prev.mtime_ns;
            if same_meta && !racy && !force_verify {
                return Ok(FileChange { path: f.path.clone(), status: FileStatus::Unchanged, size: f.size, hash: Some(prev.hash), refresh_index: false });
            }
            if f.size != prev.size {
                // 大小不同必然已修改，哈希留到备份时计算。
                return Ok(FileChange { path: f.path.clone(), status: FileStatus::Modified, size: f.size, hash: None, refresh_index: false });
            }
            let hash = hash_file(&f.abs)?;
            let status = if hash == prev.hash { FileStatus::Unchanged } else { FileStatus::Modified };
            let refresh = status == FileStatus::Unchanged && (f.mtime_ns != prev.mtime_ns || racy);
            Ok(FileChange { path: f.path.clone(), status, size: f.size, hash: Some(hash), refresh_index: refresh })
        })
        .collect::<Result<Vec<_>>>()?;

    let on_disk: std::collections::HashSet<&str> = disk.iter().map(|f| f.path.as_str()).collect();
    changes.extend(index.values().filter(|e| !on_disk.contains(e.path.as_str())).map(|e| FileChange {
        path: e.path.clone(),
        status: FileStatus::Deleted,
        size: e.size,
        hash: Some(e.hash),
        refresh_index: false,
    }));
    changes.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(changes)
}

/// 变化汇总。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSummary {
    pub added: u64,
    pub modified: u64,
    pub deleted: u64,
    pub unchanged: u64,
    /// 新增与修改文件的字节数。
    pub changed_bytes: u64,
}

impl ChangeSummary {
    pub fn of(changes: &[FileChange]) -> Self {
        let mut s = Self::default();
        for c in changes {
            match c.status {
                FileStatus::Added => {
                    s.added += 1;
                    s.changed_bytes += c.size;
                }
                FileStatus::Modified => {
                    s.modified += 1;
                    s.changed_bytes += c.size;
                }
                FileStatus::Deleted => s.deleted += 1,
                FileStatus::Unchanged => s.unchanged += 1,
            }
        }
        s
    }

    pub fn changed_files(&self) -> u64 {
        self.added + self.modified + self.deleted
    }

    pub fn has_changes(&self) -> bool {
        self.changed_files() > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, data: &[u8]) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    }

    fn set_mtime(p: &Path, ns: i64) {
        let t = UNIX_EPOCH + std::time::Duration::from_nanos(ns as u64);
        std::fs::File::options().write(true).open(p).unwrap().set_modified(t).unwrap();
    }

    /// 1 小时前，避开 racy 窗口。
    fn old_ns() -> i64 {
        now_ns() - 3_600_000_000_000
    }

    fn index_of(files: &[DiskFile], indexed_at_ns: i64) -> HashMap<String, IndexEntry> {
        files
            .iter()
            .map(|f| (f.path.clone(), IndexEntry { path: f.path.clone(), size: f.size, mtime_ns: f.mtime_ns, hash: hash_file(&f.abs).unwrap(), indexed_at_ns }))
            .collect()
    }

    fn roots(dir: &Path) -> Vec<ScanRoot> {
        vec![
            ScanRoot { ord: 0, path: dir.join("saves"), is_file: false, tag: FileTag::Save },
            ScanRoot { ord: 1, path: dir.join("options.ini"), is_file: true, tag: FileTag::Config },
            ScanRoot { ord: 2, path: dir.join("missing"), is_file: false, tag: FileTag::Save },
        ]
    }

    fn status_of(changes: &[FileChange]) -> Vec<(String, FileStatus)> {
        changes.iter().map(|c| (c.path.clone(), c.status)).collect()
    }

    #[test]
    fn collects_files_with_prefixes() {
        let d = tempfile::tempdir().unwrap();
        write(&d.path().join("saves/slot1.sav"), b"1");
        write(&d.path().join("saves/子目录/存档 ã.sav"), b"2");
        write(&d.path().join("options.ini"), b"x=1");
        let files = collect_files(&roots(d.path())).unwrap();
        let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["0/slot1.sav", "0/子目录/存档 ã.sav", "1/options.ini"]);
        assert_eq!(files[2].tag, FileTag::Config);
    }

    #[test]
    fn detects_all_statuses() {
        let d = tempfile::tempdir().unwrap();
        let p = |s: &str| d.path().join(s);
        for (f, data) in [("saves/keep", "k"), ("saves/modify", "m"), ("saves/delete", "d"), ("saves/touch", "t")] {
            write(&p(f), data.as_bytes());
            set_mtime(&p(f), old_ns());
        }
        let before = collect_files(&roots(d.path())).unwrap();
        let index = index_of(&before, now_ns());

        write(&p("saves/modify"), b"M");
        set_mtime(&p("saves/modify"), old_ns() + 1_000_000_000_000);
        std::fs::remove_file(p("saves/delete")).unwrap();
        write(&p("saves/new"), b"n");
        set_mtime(&p("saves/touch"), old_ns() + 5_000_000_000);

        let changes = detect(&collect_files(&roots(d.path())).unwrap(), &index, false).unwrap();
        assert_eq!(
            status_of(&changes),
            [
                ("0/delete".to_string(), FileStatus::Deleted),
                ("0/keep".to_string(), FileStatus::Unchanged),
                ("0/modify".to_string(), FileStatus::Modified),
                ("0/new".to_string(), FileStatus::Added),
                ("0/touch".to_string(), FileStatus::Unchanged),
            ]
        );
        let touch = changes.iter().find(|c| c.path == "0/touch").unwrap();
        assert!(touch.refresh_index, "只有 mtime 变化的文件需要刷新索引");
        let s = ChangeSummary::of(&changes);
        assert_eq!((s.added, s.modified, s.deleted, s.unchanged), (1, 1, 1, 2));
        assert!(s.has_changes());
    }

    #[test]
    fn same_size_same_mtime_change_is_caught_only_when_racy_or_forced() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("saves/a");
        write(&f, b"aaaa");
        let mtime = old_ns();
        set_mtime(&f, mtime);
        let before = collect_files(&roots(d.path())).unwrap();

        // 内容改变但大小与 mtime 都被还原（例如 2 秒精度的文件系统）。
        write(&f, b"bbbb");
        set_mtime(&f, mtime);
        let after = collect_files(&roots(d.path())).unwrap();

        let indexed_long_after = index_of(&before, mtime + 10 * RACY_WINDOW_NS);
        // 元数据一致且不在 racy 窗口：按设计不读内容。
        let hashes_from = |idx: &HashMap<String, IndexEntry>| {
            let mut idx = idx.clone();
            idx.get_mut("0/a").unwrap().hash = ObjectId::of(b"aaaa");
            idx
        };
        let idx = hashes_from(&indexed_long_after);
        assert_eq!(detect(&after, &idx, false).unwrap()[0].status, FileStatus::Unchanged);
        assert_eq!(detect(&after, &idx, true).unwrap()[0].status, FileStatus::Modified, "强制校验");

        // 索引写入时间与 mtime 相差不到 2 秒：racy，重算哈希。
        let idx = hashes_from(&index_of(&before, mtime + RACY_WINDOW_NS / 2));
        assert_eq!(detect(&after, &idx, false).unwrap()[0].status, FileStatus::Modified);
    }

    #[test]
    fn empty_index_means_all_added() {
        let d = tempfile::tempdir().unwrap();
        write(&d.path().join("saves/a"), b"a");
        let changes = detect(&collect_files(&roots(d.path())).unwrap(), &HashMap::new(), false).unwrap();
        assert_eq!(changes[0].status, FileStatus::Added);
        assert!(changes[0].hash.is_none());
    }

    #[test]
    fn many_files_hash_in_parallel() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..200 {
            write(&d.path().join(format!("saves/f{i:03}")), format!("{i}").as_bytes());
        }
        let files = collect_files(&roots(d.path())).unwrap();
        let changes = detect(&files, &HashMap::new(), true).unwrap();
        assert_eq!(changes.len(), 200);
    }
}
