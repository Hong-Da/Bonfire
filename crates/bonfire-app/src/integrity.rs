//! 篝火导出内容的校验值（U-48）：导出时对包内每个文件计算 blake3，按路径排序后再算一次，
//! 写入说明文件的 `contentHash`（`blake3:<十六进制>`）；说明文件本身与 readme.txt 不计入。
//! 导入时重新计算并比对：一致才按篝火导出的处理；不一致说明内容被手动修改过或混入了其他文件，
//! 按普通下载的存档处理并提示。早期导出的没有校验值，仍按篝火导出的处理。

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use serde::Serialize;

use crate::error::{AppError, Result};

/// 校验值的前缀（算法）。
pub const PREFIX: &str = "blake3:";

/// 存档文件校验不通过时的提示（界面据“校验不通过”引导改用“导入下载的存档”）。
pub const FILES_MODIFIED: &str = "这份篝火导出的存档校验不通过：内容与导出时不一致，可能被手动修改过或混入了其他文件。请改用“导入下载的存档”，按普通的存档文件导入";

/// 校验结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Check {
    /// 与导出时一致。
    Verified,
    /// 早期导出的，没有校验值。
    Unsigned,
    /// 与导出时不一致。
    Modified,
}

/// 包内文件的校验值：相对路径 → 文件内容的 blake3。
#[derive(Default)]
pub struct Digest(BTreeMap<String, blake3::Hash>);

impl Digest {
    pub fn add(&mut self, rel: &str, hash: blake3::Hash) {
        self.0.insert(rel.to_string(), hash);
    }

    pub fn add_bytes(&mut self, rel: &str, bytes: &[u8]) {
        self.add(rel, blake3::hash(bytes));
    }

    /// 整体的校验值：按路径排序，逐行“路径\0文件校验值\n”再算 blake3。
    pub fn finish(&self) -> String {
        let mut h = blake3::Hasher::new();
        for (rel, fh) in &self.0 {
            h.update(rel.as_bytes());
            h.update(b"\0");
            h.update(fh.to_hex().as_bytes());
            h.update(b"\n");
        }
        format!("{PREFIX}{}", h.finalize().to_hex())
    }
}

/// 一边写入一边计算 blake3。
pub struct HashWriter<W> {
    pub inner: W,
    pub hasher: blake3::Hasher,
}

impl<W> HashWriter<W> {
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            hasher: blake3::Hasher::new(),
        }
    }
}

impl<W: Write> Write for HashWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// 不计入校验值的文件：说明文件本身与 readme.txt（都在包的根目录）。
fn skipped(rel: &str, manifest: &str) -> bool {
    rel == manifest || rel.eq_ignore_ascii_case(crate::branding::README_NAME)
}

fn hash_reader(r: &mut impl Read) -> std::io::Result<blake3::Hash> {
    let mut h = blake3::Hasher::new();
    std::io::copy(r, &mut h)?;
    Ok(h.finalize())
}

/// 文件夹的校验值（`skip_dir` 为不计入的子文件夹相对路径，如解压时放嵌套压缩包的临时文件夹）。
pub fn dir_digest(root: &Path, manifest: &str, skip_dir: &dyn Fn(&str) -> bool) -> Result<String> {
    let mut d = Digest::default();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir)
            .map_err(|err| AppError::io(&dir, err))?
            .flatten()
        {
            let p = e.path();
            let rel = p
                .strip_prefix(root)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            let ft = e.file_type().map_err(|err| AppError::io(&p, err))?;
            if ft.is_dir() {
                if !skip_dir(&rel) {
                    stack.push(p);
                }
            } else if ft.is_file() && !skipped(&rel, manifest) {
                let mut f = std::fs::File::open(&p).map_err(|err| AppError::io(&p, err))?;
                d.add(
                    &rel,
                    hash_reader(&mut f).map_err(|err| AppError::io(&p, err))?,
                );
            }
        }
    }
    Ok(d.finish())
}

/// 压缩包中 `prefix` 目录下的校验值（`prefix` 为空表示整个压缩包）。
pub fn zip_digest(
    zip: &mut zip::ZipArchive<std::fs::File>,
    path: &Path,
    prefix: &str,
    manifest: &str,
) -> Result<String> {
    let mut d = Digest::default();
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| AppError::Invalid(format!("读取压缩包失败：{e}")))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry
            .name()
            .map_err(|e| AppError::Invalid(format!("读取压缩包失败：{e}")))?
            .replace('\\', "/");
        let Some(rel) = name.strip_prefix(prefix) else {
            continue;
        };
        if skipped(rel, manifest) {
            continue;
        }
        let h = hash_reader(&mut entry).map_err(|e| AppError::io(path, e))?;
        d.add(rel, h);
    }
    Ok(d.finish())
}

/// 比对：没有校验值为 `Unsigned`。
pub fn compare(expected: Option<&str>, actual: impl FnOnce() -> Result<String>) -> Result<Check> {
    match expected {
        None => Ok(Check::Unsigned),
        Some(e) => Ok(if actual()? == e {
            Check::Verified
        } else {
            Check::Modified
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_order_independent_and_detects_changes() {
        let mut a = Digest::default();
        a.add_bytes("b/x.dat", b"1");
        a.add_bytes("a.dat", b"2");
        let mut b = Digest::default();
        b.add_bytes("a.dat", b"2");
        b.add_bytes("b/x.dat", b"1");
        assert_eq!(a.finish(), b.finish());
        assert!(a.finish().starts_with(PREFIX));
        b.add_bytes("c.dat", b"3");
        assert_ne!(a.finish(), b.finish());
    }

    #[test]
    fn dir_digest_skips_manifest_and_readme() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join("Save")).unwrap();
        std::fs::write(t.path().join("Save/user1.dat"), b"data").unwrap();
        let before = dir_digest(t.path(), "m.json", &|_| false).unwrap();
        std::fs::write(t.path().join("m.json"), b"{}").unwrap();
        std::fs::write(t.path().join("README.TXT"), b"hi").unwrap();
        assert_eq!(dir_digest(t.path(), "m.json", &|_| false).unwrap(), before);
        std::fs::write(t.path().join("Save/user1.dat"), b"changed").unwrap();
        assert_ne!(dir_digest(t.path(), "m.json", &|_| false).unwrap(), before);
        assert_eq!(compare(None, || unreachable!()).unwrap(), Check::Unsigned);
    }
}
