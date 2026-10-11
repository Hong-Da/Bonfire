//! 内容寻址对象库（文档 6.2、6.5）。
//!
//! - 每个对象文件是一个带校验和的 zstd 帧；同一 ID 的对象只写一次（全局去重）。
//! - 写入先写 `objects/tmp/` 下的临时文件并落盘，再原子重命名到目标位置；
//!   崩溃最多留下临时文件或无引用对象，不会出现写了一半的对象。
//! - 读取时校验：Chunk、Tree、Snapshot 校验 ID；Blob 在还原文件时校验整个文件的哈希。

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{CoreError, IoContext, Result};
use crate::fault::{self, points};
use crate::object::{Blob, Kind, Snapshot, Tree, canonical_json};
use crate::{ObjectId, Repo, RepoLock};

/// 写入一个文件后的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteStats {
    pub blob: ObjectId,
    pub size: u64,
    pub chunks: u64,
    /// 本次新写入（之前不存在）的 Chunk 数与其原始字节数。
    pub new_chunks: u64,
    pub new_bytes: u64,
}

impl Repo {
    pub(crate) fn object_path(&self, kind: Kind, id: &ObjectId) -> PathBuf {
        let hex = id.to_hex();
        self.objects_dir()
            .join(kind.dir())
            .join(&hex[..2])
            .join(&hex[2..])
    }

    /// 对象文件在磁盘上的大小（不存在为 0）。
    pub fn object_size(&self, kind: Kind, id: &ObjectId) -> u64 {
        std::fs::metadata(self.object_path(kind, id)).map_or(0, |m| m.len())
    }

    pub fn has(&self, kind: Kind, id: &ObjectId) -> bool {
        self.object_path(kind, id).is_file()
    }

    /// 某类对象的全部 ID（遍历目录；不在对象目录规则内的文件被忽略）。
    pub fn list_objects(&self, kind: Kind) -> Result<Vec<ObjectId>> {
        let root = self.objects_dir().join(kind.dir());
        let mut out = Vec::new();
        for sub in std::fs::read_dir(&root).at(&root)?.flatten() {
            let prefix = sub.file_name().to_string_lossy().into_owned();
            if prefix.len() != 2 || !sub.path().is_dir() {
                continue;
            }
            for f in std::fs::read_dir(sub.path()).at(sub.path())?.flatten() {
                if let Ok(id) = format!("{prefix}{}", f.file_name().to_string_lossy()).parse() {
                    out.push(id);
                }
            }
        }
        out.sort();
        Ok(out)
    }

    /// 写入原始字节（压缩后存放）。已存在时不写，返回 false。
    fn put_raw(&self, kind: Kind, id: &ObjectId, raw: &[u8]) -> Result<bool> {
        let path = self.object_path(kind, id);
        if path.is_file() {
            return Ok(false);
        }
        let compressed = compress(raw, self.config().zstd_level).at(&path)?;
        let stored = self.seal(compressed)?;
        let dir = path.parent().expect("对象路径有上级目录");
        std::fs::create_dir_all(dir).at(dir)?;
        write_atomic(&self.tmp_dir(), &path, &stored, true)?;
        Ok(true)
    }

    fn get_raw(&self, kind: Kind, id: &ObjectId) -> Result<Vec<u8>> {
        let path = self.object_path(kind, id);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(CoreError::NotFound { kind, id: *id });
            }
            Err(e) => return Err(CoreError::io(path, e)),
        };
        let bytes = self.unseal(bytes, &format!("{kind}对象 {id}"))?;
        zstd::stream::decode_all(&bytes[..]).map_err(|e| CoreError::Corrupt {
            kind,
            id: *id,
            reason: e.to_string(),
        })
    }

    fn put_json<T: Serialize>(&self, kind: Kind, v: &T) -> Result<ObjectId> {
        let raw = canonical_json(v);
        let id = ObjectId::of(&raw);
        self.put_raw(kind, &id, &raw)?;
        Ok(id)
    }

    /// 读取 JSON 对象并校验其 ID。
    fn get_json<T: DeserializeOwned>(&self, kind: Kind, id: &ObjectId) -> Result<T> {
        let raw = self.get_raw(kind, id)?;
        if ObjectId::of(&raw) != *id {
            return Err(CoreError::Corrupt {
                kind,
                id: *id,
                reason: "内容哈希不匹配".into(),
            });
        }
        serde_json::from_slice(&raw).map_err(|e| CoreError::Corrupt {
            kind,
            id: *id,
            reason: e.to_string(),
        })
    }

    /// 对象文件中存放的字节（压缩后，加密仓库中解密后），用于导出历史包时原样复制。
    pub fn read_object_file(&self, kind: Kind, id: &ObjectId) -> Result<Vec<u8>> {
        let path = self.object_path(kind, id);
        match std::fs::read(&path) {
            Ok(b) => self.unseal(b, &format!("{kind}对象 {id}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(CoreError::NotFound { kind, id: *id })
            }
            Err(e) => Err(CoreError::io(path, e)),
        }
    }

    /// 导入别处导出的对象文件：解压并校验后原样写入，已存在时不写（返回 false）。
    /// Chunk、Tree、Snapshot 校验 ID；Blob 只校验格式，整个文件的哈希在还原时（或导入后）校验。
    pub fn import_object_file(&self, kind: Kind, id: &ObjectId, stored: &[u8]) -> Result<bool> {
        let path = self.object_path(kind, id);
        if path.is_file() {
            return Ok(false);
        }
        let raw = zstd::stream::decode_all(stored).map_err(|e| CoreError::Corrupt {
            kind,
            id: *id,
            reason: e.to_string(),
        })?;
        let ok = match kind {
            Kind::Blob => serde_json::from_slice::<Blob>(&raw).is_ok(),
            _ => ObjectId::of(&raw) == *id,
        };
        if !ok {
            return Err(CoreError::Corrupt {
                kind,
                id: *id,
                reason: "内容与 ID 不符".into(),
            });
        }
        let sealed = self.seal(stored.to_vec())?;
        let dir = path.parent().expect("对象路径有上级目录");
        std::fs::create_dir_all(dir).at(dir)?;
        write_atomic(&self.tmp_dir(), &path, &sealed, true)?;
        Ok(true)
    }

    /// 加密或解密整个仓库时改写一个对象文件（内容不变，只换存放格式）。返回是否改写。
    pub fn reseal_object(
        &self,
        _lock: &RepoLock,
        kind: Kind,
        id: &ObjectId,
        encrypt: bool,
    ) -> Result<bool> {
        let path = self.object_path(kind, id);
        let data = std::fs::read(&path).at(&path)?;
        let sealed = crate::crypt::is_sealed(&data);
        if sealed == encrypt {
            return Ok(false);
        }
        let c = self.cipher().ok_or(CoreError::NeedPassword)?;
        let out = if encrypt {
            c.seal(&data)
        } else {
            c.open(&data, &format!("{kind}对象 {id}"))?
        };
        write_atomic(&self.tmp_dir(), &path, &out, false)?;
        Ok(true)
    }

    /// 写入一个分块，返回 (ID, 是否新写入)。
    pub fn put_chunk(&self, data: &[u8]) -> Result<(ObjectId, bool)> {
        let id = ObjectId::of(data);
        let new = self.put_raw(Kind::Chunk, &id, data)?;
        Ok((id, new))
    }

    pub fn get_chunk(&self, id: &ObjectId) -> Result<Vec<u8>> {
        let data = self.get_raw(Kind::Chunk, id)?;
        if ObjectId::of(&data) != *id {
            return Err(CoreError::Corrupt {
                kind: Kind::Chunk,
                id: *id,
                reason: "内容哈希不匹配".into(),
            });
        }
        Ok(data)
    }

    pub fn get_blob(&self, id: &ObjectId) -> Result<Blob> {
        let raw = self.get_raw(Kind::Blob, id)?;
        serde_json::from_slice(&raw).map_err(|e| CoreError::Corrupt {
            kind: Kind::Blob,
            id: *id,
            reason: e.to_string(),
        })
    }

    pub fn put_tree(&self, tree: &Tree) -> Result<ObjectId> {
        let id = self.put_json(Kind::Tree, tree)?;
        fault::point(points::AFTER_TREE);
        Ok(id)
    }

    pub fn get_tree(&self, id: &ObjectId) -> Result<Tree> {
        self.get_json(Kind::Tree, id)
    }

    pub fn put_snapshot(&self, snap: &Snapshot) -> Result<ObjectId> {
        let id = self.put_json(Kind::Snapshot, snap)?;
        fault::point(points::AFTER_SNAPSHOT);
        Ok(id)
    }

    pub fn get_snapshot(&self, id: &ObjectId) -> Result<Snapshot> {
        self.get_json(Kind::Snapshot, id)
    }

    /// 删除快照对象（用户删除快照或分支时）。调用方须先确认没有引用、子快照指向它；
    /// 它引用的清单与文件不删除，由 GC 清理。不存在时返回 false。
    pub fn remove_snapshot(&self, _lock: &RepoLock, id: &ObjectId) -> Result<bool> {
        let path = self.object_path(Kind::Snapshot, id);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(CoreError::io(path, e)),
        }
    }

    /// 读取整个文件内容并入库：小文件整体作为一个 Chunk，大文件用 FastCDC 分块。
    /// `source` 只用于错误信息中的路径。
    pub fn write_file(&self, mut reader: impl Read, source: &Path) -> Result<WriteStats> {
        let cfg = self.config();
        let mut hasher = blake3::Hasher::new();
        let mut chunks = Vec::new();
        let mut stats = WriteStats {
            blob: ObjectId::of(b""),
            size: 0,
            chunks: 0,
            new_chunks: 0,
            new_bytes: 0,
        };

        let mut add = |data: &[u8], stats: &mut WriteStats| -> Result<()> {
            hasher.update(data);
            let (id, new) = self.put_chunk(data)?;
            if new {
                stats.new_chunks += 1;
                stats.new_bytes += data.len() as u64;
                fault::point(points::AFTER_CHUNK);
            }
            stats.size += data.len() as u64;
            chunks.push(id);
            Ok(())
        };

        // 先读到阈值大小，判断是否需要分块。
        let threshold = cfg.chunk_threshold as usize;
        let mut head = Vec::with_capacity(threshold.min(1 << 20));
        (&mut reader)
            .take(threshold as u64)
            .read_to_end(&mut head)
            .at(source)?;
        if head.len() < threshold {
            if !head.is_empty() {
                add(&head, &mut stats)?;
            }
        } else {
            let stream = std::io::Cursor::new(head).chain(reader);
            let cdc = fastcdc::v2020::StreamCDC::new(
                stream,
                cfg.chunk_min as usize,
                cfg.chunk_avg as usize,
                cfg.chunk_max as usize,
            );
            for chunk in cdc {
                let chunk = chunk
                    .map_err(|e| CoreError::io(source, std::io::Error::other(e.to_string())))?;
                add(&chunk.data, &mut stats)?;
            }
        }

        stats.blob = ObjectId::from_hash(hasher.finalize());
        stats.chunks = chunks.len() as u64;
        let blob = Blob {
            size: stats.size,
            chunks,
        };
        if self.put_raw(Kind::Blob, &stats.blob, &canonical_json(&blob))? {
            fault::point(points::AFTER_BLOB);
        }
        Ok(stats)
    }

    /// 还原文件内容到 `out`，并校验大小与整个文件的哈希。返回写出的字节数。
    pub fn read_blob_to(&self, id: &ObjectId, mut out: impl Write, dest: &Path) -> Result<u64> {
        let blob = self.get_blob(id)?;
        let mut hasher = blake3::Hasher::new();
        let mut written = 0u64;
        for c in &blob.chunks {
            let data = self.get_chunk(c)?;
            hasher.update(&data);
            out.write_all(&data).at(dest)?;
            written += data.len() as u64;
        }
        if written != blob.size || ObjectId::from_hash(hasher.finalize()) != *id {
            return Err(CoreError::Corrupt {
                kind: Kind::Blob,
                id: *id,
                reason: "还原后的内容与记录不符".into(),
            });
        }
        Ok(written)
    }
}

fn compress(raw: &[u8], level: i32) -> std::io::Result<Vec<u8>> {
    let mut enc = zstd::stream::Encoder::new(Vec::with_capacity(raw.len() / 2 + 64), level)?;
    enc.include_checksum(true)?;
    enc.write_all(raw)?;
    enc.finish()
}

/// 唯一的临时文件名：进程号 + 进程内序号。
fn tmp_name() -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

/// 原子写入：写临时文件 → 落盘 → 重命名到目标位置。
/// `content_addressed` 为 true 时，目标已存在即视为成功（同一 ID 的内容必然相同）。
pub(crate) fn write_atomic(
    tmp_dir: &Path,
    dest: &Path,
    bytes: &[u8],
    content_addressed: bool,
) -> Result<()> {
    std::fs::create_dir_all(tmp_dir).at(tmp_dir)?;
    let tmp = tmp_dir.join(tmp_name());
    let result = (|| {
        let mut f = std::fs::File::create(&tmp).at(&tmp)?;
        f.write_all(bytes).at(&tmp)?;
        f.sync_all().at(&tmp)?;
        drop(f);
        fault::point(points::BEFORE_RENAME);
        match std::fs::rename(&tmp, dest) {
            Ok(()) => Ok(()),
            // 并发写入同一对象时，对方已经放好了同样的内容（Windows 上替换正被读取的文件会失败）。
            Err(_) if content_addressed && dest.is_file() => Ok(()),
            Err(e) => Err(CoreError::io(dest, e)),
        }
    })();
    if tmp.exists() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{FileTag, SnapshotStats, TreeEntry, Trigger};

    /// 固定种子的伪随机数据（xorshift64），不引入 rand 依赖。
    fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
        let mut x = seed.max(1);
        let mut v = Vec::with_capacity(len + 8);
        while v.len() < len {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            v.extend_from_slice(&x.to_le_bytes());
        }
        v.truncate(len);
        v
    }

    fn repo() -> (tempfile::TempDir, Repo) {
        let dir = tempfile::tempdir().unwrap();
        let r = Repo::init(dir.path()).unwrap();
        (dir, r)
    }

    fn roundtrip(r: &Repo, id: &ObjectId) -> Vec<u8> {
        let mut out = Vec::new();
        r.read_blob_to(id, &mut out, Path::new("mem")).unwrap();
        out
    }

    #[test]
    fn chunk_roundtrip_and_dedupe() {
        let (_d, r) = repo();
        let (id, new) = r.put_chunk(b"save data").unwrap();
        assert!(new);
        assert_eq!(r.get_chunk(&id).unwrap(), b"save data");
        let (id2, new2) = r.put_chunk(b"save data").unwrap();
        assert_eq!(id, id2);
        assert!(!new2);
    }

    #[test]
    fn missing_object_is_not_found() {
        let (_d, r) = repo();
        let id = ObjectId::of(b"nope");
        assert!(matches!(
            r.get_chunk(&id),
            Err(CoreError::NotFound {
                kind: Kind::Chunk,
                ..
            })
        ));
        assert!(matches!(
            r.get_tree(&id),
            Err(CoreError::NotFound {
                kind: Kind::Tree,
                ..
            })
        ));
    }

    #[test]
    fn small_file_is_single_chunk_with_file_hash() {
        let (_d, r) = repo();
        let data = "存档 ã ç é".repeat(100).into_bytes();
        let s = r.write_file(&data[..], Path::new("a.sav")).unwrap();
        assert_eq!(s.blob, ObjectId::of(&data));
        assert_eq!((s.chunks, s.new_chunks, s.size), (1, 1, data.len() as u64));
        assert_eq!(roundtrip(&r, &s.blob), data);
    }

    #[test]
    fn empty_file() {
        let (_d, r) = repo();
        let s = r.write_file(&b""[..], Path::new("empty")).unwrap();
        assert_eq!(s.blob, ObjectId::of(b""));
        assert_eq!(s.chunks, 0);
        assert_eq!(roundtrip(&r, &s.blob), b"");
    }

    #[test]
    fn large_file_dedupes_unchanged_chunks() {
        let (_d, r) = repo();
        let mut data = pseudo_random(12 << 20, 42);
        let first = r.write_file(&data[..], Path::new("big")).unwrap();
        assert!(
            first.chunks >= 4,
            "12 MiB 应分为多个块，实际 {}",
            first.chunks
        );
        assert_eq!(first.blob, ObjectId::of(&data));
        assert_eq!(roundtrip(&r, &first.blob), data);

        // 同样内容再写一次：没有新块。
        let again = r.write_file(&data[..], Path::new("big")).unwrap();
        assert_eq!((again.blob, again.new_chunks), (first.blob, 0));

        // 修改中间 100 字节：只新增改动附近的少数块。
        for b in &mut data[6 << 20..(6 << 20) + 100] {
            *b ^= 0xff;
        }
        let changed = r.write_file(&data[..], Path::new("big")).unwrap();
        assert_ne!(changed.blob, first.blob);
        assert!(
            changed.new_chunks <= 2,
            "只应新增 1–2 个块，实际 {}",
            changed.new_chunks
        );
        assert!(changed.new_bytes < (data.len() as u64) / 2);
        assert_eq!(roundtrip(&r, &changed.blob), data);
    }

    #[test]
    fn exactly_threshold_sized_file() {
        let (_d, r) = repo();
        let data = pseudo_random(r.config().chunk_threshold as usize, 7);
        let s = r.write_file(&data[..], Path::new("t")).unwrap();
        assert_eq!(roundtrip(&r, &s.blob), data);
    }

    #[test]
    fn tree_and_snapshot_roundtrip() {
        let (_d, r) = repo();
        let blob = r.write_file(&b"x"[..], Path::new("x")).unwrap().blob;
        let tree = Tree::new(vec![TreeEntry {
            path: "0/存档.sav".into(),
            blob,
            size: 1,
            mtime_ns: 123,
            tag: FileTag::Save,
        }]);
        let tid = r.put_tree(&tree).unwrap();
        assert_eq!(r.get_tree(&tid).unwrap(), tree);
        assert_eq!(r.put_tree(&tree).unwrap(), tid, "同样的清单得到同样的 ID");

        let snap = Snapshot {
            game: "custom-1".into(),
            tree: tid,
            parents: vec![],
            created_at: "2026-10-09T12:00:00+08:00".into(),
            trigger: Trigger::Manual,
            host: "PC".into(),
            comment: "第一次".into(),
            stats: SnapshotStats {
                files: 1,
                bytes: 1,
                changed: 1,
            },
            playtime_min: Some(42 * 60),
        };
        let sid = r.put_snapshot(&snap).unwrap();
        assert_eq!(r.get_snapshot(&sid).unwrap(), snap);
    }

    /// 找到对象文件并改写其内容。
    fn tamper(r: &Repo, kind: Kind, id: &ObjectId, f: impl FnOnce(Vec<u8>) -> Vec<u8>) {
        let p = r.object_path(kind, id);
        let bytes = std::fs::read(&p).unwrap();
        std::fs::write(&p, f(bytes)).unwrap();
    }

    #[test]
    fn corrupted_bytes_are_detected() {
        let (_d, r) = repo();
        let (id, _) = r.put_chunk(&pseudo_random(4096, 1)).unwrap();
        tamper(&r, Kind::Chunk, &id, |mut b| {
            let n = b.len() / 2;
            b[n] ^= 0x55;
            b
        });
        assert!(matches!(r.get_chunk(&id), Err(CoreError::Corrupt { .. })));
    }

    #[test]
    fn swapped_object_content_is_detected() {
        // 文件完好但内容属于另一个 ID（例如被手工复制错位置）。
        let (_d, r) = repo();
        let (a, _) = r.put_chunk(b"aaaa").unwrap();
        let (b, _) = r.put_chunk(b"bbbb").unwrap();
        let other = std::fs::read(r.object_path(Kind::Chunk, &b)).unwrap();
        tamper(&r, Kind::Chunk, &a, |_| other);
        assert!(matches!(r.get_chunk(&a), Err(CoreError::Corrupt { .. })));
    }

    #[test]
    fn missing_chunk_breaks_blob_read() {
        let (_d, r) = repo();
        let s = r.write_file(&b"hello"[..], Path::new("h")).unwrap();
        std::fs::remove_file(r.object_path(Kind::Chunk, &s.blob)).unwrap();
        let mut out = Vec::new();
        assert!(matches!(
            r.read_blob_to(&s.blob, &mut out, Path::new("h")),
            Err(CoreError::NotFound {
                kind: Kind::Chunk,
                ..
            })
        ));
    }

    #[test]
    fn no_tmp_files_left_behind() {
        let (_d, r) = repo();
        r.write_file(&pseudo_random(3 << 20, 3)[..], Path::new("f"))
            .unwrap();
        assert_eq!(std::fs::read_dir(r.tmp_dir()).unwrap().count(), 0);
    }

    // ---------- 崩溃一致性：子进程在注入点中止，父进程检查仓库 ----------

    const CHILD_REPO: &str = "BONFIRE_TEST_CHILD_REPO";

    /// 子进程入口：仅在父测试设置了环境变量时执行写入。
    #[test]
    fn crash_child() {
        let Some(dir) = std::env::var_os(CHILD_REPO) else {
            return;
        };
        let r = Repo::open(Path::new(&dir)).unwrap();
        r.write_file(&pseudo_random(8 << 20, 11)[..], Path::new("big"))
            .unwrap();
        let blob = r
            .write_file(&b"small"[..], Path::new("small"))
            .unwrap()
            .blob;
        let tree = Tree::new(vec![TreeEntry {
            path: "0/small".into(),
            blob,
            size: 5,
            mtime_ns: 0,
            tag: FileTag::Save,
        }]);
        r.put_tree(&tree).unwrap();
    }

    /// 检查仓库中每个对象文件都完整、ID 正确。
    fn assert_all_objects_valid(r: &Repo) -> usize {
        let mut n = 0;
        for kind in Kind::ALL {
            for sub in std::fs::read_dir(r.objects_dir().join(kind.dir()))
                .unwrap()
                .flatten()
            {
                for f in std::fs::read_dir(sub.path()).unwrap().flatten() {
                    let id: ObjectId = format!(
                        "{}{}",
                        sub.file_name().to_string_lossy(),
                        f.file_name().to_string_lossy()
                    )
                    .parse()
                    .unwrap();
                    match kind {
                        Kind::Chunk => drop(r.get_chunk(&id).unwrap()),
                        Kind::Blob => drop(
                            r.read_blob_to(&id, std::io::sink(), Path::new("x"))
                                .unwrap(),
                        ),
                        Kind::Tree => drop(r.get_tree(&id).unwrap()),
                        Kind::Snapshot => drop(r.get_snapshot(&id).unwrap()),
                    }
                    n += 1;
                }
            }
        }
        n
    }

    fn crash_at(point: &str) -> (tempfile::TempDir, Repo) {
        let (dir, r) = repo();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "store::tests::crash_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_REPO, dir.path())
            .env(fault::ENV, point)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(!status.success(), "子进程应在 {point} 处中止");
        (dir, r)
    }

    #[test]
    fn crash_before_rename_leaves_only_tmp() {
        let (_d, r) = crash_at("before-rename:3");
        assert_eq!(assert_all_objects_valid(&r), 2, "只有前两个分块完整写入");
        // 临时文件可能残留，但不会出现在对象目录中；下次写入同一对象仍然成功。
        r.write_file(&pseudo_random(8 << 20, 11)[..], Path::new("big"))
            .unwrap();
        assert_all_objects_valid(&r);
    }

    #[test]
    fn crash_mid_file_leaves_no_blob() {
        let (_d, r) = crash_at("after-chunk:2");
        let n = assert_all_objects_valid(&r);
        assert_eq!(n, 2);
        assert!(
            std::fs::read_dir(r.objects_dir().join("blobs"))
                .unwrap()
                .next()
                .is_none()
        );
    }

    #[test]
    fn crash_after_blob_keeps_complete_file() {
        let (_d, r) = crash_at("after-blob");
        assert!(assert_all_objects_valid(&r) >= 2);
        let blob = ObjectId::of(&pseudo_random(8 << 20, 11));
        assert!(r.has(Kind::Blob, &blob));
    }

    #[test]
    fn concurrent_writes_of_same_chunk() {
        let (_d, r) = repo();
        let data = pseudo_random(64 * 1024, 9);
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| r.put_chunk(&data).unwrap());
            }
        });
        assert_eq!(r.get_chunk(&ObjectId::of(&data)).unwrap(), data);
    }
}
