//! 文件夹目标：本机目录、局域网共享（`\\NAS\…`）、网盘客户端的同步文件夹。

use std::path::{Path, PathBuf};

use super::Remote;
use crate::error::{AppError, Result};

pub struct Folder {
    root: PathBuf,
}

impl Folder {
    pub fn new(path: &str) -> Self {
        Self {
            root: PathBuf::from(path.trim()),
        }
    }

    fn at(&self, rel: &str) -> PathBuf {
        rel.split('/').fold(self.root.clone(), |p, s| p.join(s))
    }
}

impl Remote for Folder {
    fn test(&mut self) -> Result<String> {
        std::fs::create_dir_all(&self.root).map_err(|e| AppError::io(&self.root, e))?;
        let probe = self.root.join(".bonfire-write-test");
        std::fs::write(&probe, b"ok").map_err(|e| AppError::io(&probe, e))?;
        let _ = std::fs::remove_file(&probe);
        Ok(format!("可以写入 {}", self.root.display()))
    }

    fn put(&mut self, rel: &str, data: &[u8]) -> Result<()> {
        let dest = self.at(rel);
        let dir = dest.parent().unwrap_or(&self.root).to_path_buf();
        std::fs::create_dir_all(&dir).map_err(|e| AppError::io(&dir, e))?;
        // 先写临时文件再改名：网盘客户端不会同步到写了一半的文件。
        let tmp = dir.join(format!(
            ".{}.bonfire-tmp",
            dest.file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default()
        ));
        std::fs::write(&tmp, data).map_err(|e| AppError::io(&tmp, e))?;
        std::fs::rename(&tmp, &dest).map_err(|e| AppError::io(&dest, e))
    }

    fn delete(&mut self, rel: &str) -> Result<()> {
        let p = self.at(rel);
        match std::fs::remove_file(&p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(AppError::io(&p, e)),
        }
        // 删掉变空的上级目录（不超出根目录）。
        let mut d = p.parent().map(Path::to_path_buf);
        while let Some(dir) = d {
            if dir == self.root || std::fs::remove_dir(&dir).is_err() {
                break;
            }
            d = dir.parent().map(Path::to_path_buf);
        }
        Ok(())
    }

    fn list(&mut self) -> Result<Vec<(String, u64)>> {
        let mut out = Vec::new();
        for e in walkdir::WalkDir::new(&self.root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if e.file_type().is_file() {
                let rel = e
                    .path()
                    .strip_prefix(&self.root)
                    .unwrap_or(e.path())
                    .to_string_lossy()
                    .replace('\\', "/");
                if rel.ends_with(".bonfire-tmp") {
                    continue;
                }
                out.push((rel, e.metadata().map(|m| m.len()).unwrap_or(0)));
            }
        }
        Ok(out)
    }

    fn get(&mut self, rel: &str) -> Result<Vec<u8>> {
        let p = self.at(rel);
        std::fs::read(&p).map_err(|e| AppError::io(&p, e))
    }
}
