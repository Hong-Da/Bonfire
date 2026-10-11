//! 要同步的文件清单：按目标的内容设置列出路径、大小、指纹与来源。

use std::path::{Path, PathBuf};

use bonfire_core::ObjectId;

use super::SETTINGS_FILE;
use super::target::{SyncContent, SyncTarget};
use crate::app::App;
use crate::error::{AppError, Result};

/// 文件内容的来源。
#[derive(Debug, Clone)]
pub enum Source {
    File(PathBuf),
    /// 对象库中的文件（最新存档）。
    Blob(ObjectId),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct Item {
    /// 远程路径（`/` 分隔）。
    pub path: String,
    pub size: u64,
    /// 内容没变时不变：仓库对象用固定值（内容寻址，不会改变），其余用大小与修改时间或文件哈希。
    pub fingerprint: String,
    pub source: Source,
    /// 存档、截图、设置（预览中分类显示）。
    pub part: &'static str,
    /// 仓库文件不能被排除（否则远程仓库不完整）。
    pub required: bool,
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
}

fn file_fp(meta: &std::fs::Metadata) -> String {
    format!("{}:{}", meta.len(), bonfire_core::scan::mtime_ns(meta))
}

fn sanitize(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_control() || r#"<>:"/\|?*"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let out = out.trim().trim_end_matches('.').to_string();
    if out.is_empty() { "_".into() } else { out }
}

impl App {
    pub(crate) fn sync_items(&self, t: &SyncTarget) -> Result<Vec<Item>> {
        let mut out = Vec::new();
        let games: Vec<_> = self
            .games()?
            .into_iter()
            .filter(|g| t.games.is_empty() || t.games.contains(&g.id))
            .collect();
        match t.content {
            SyncContent::Repo => self.repo_items(t, &mut out)?,
            SyncContent::Latest => {
                for g in &games {
                    let Some(head) = self.repo.head_snapshot(&g.id)? else {
                        continue;
                    };
                    let tree = self.repo.get_tree(&self.repo.get_snapshot(&head)?.tree)?;
                    let dir = sanitize(&g.name);
                    for e in tree.entries {
                        out.push(Item {
                            path: format!("{dir}/{}", e.path),
                            size: e.size,
                            fingerprint: e.blob.to_hex(),
                            source: Source::Blob(e.blob),
                            part: "saves",
                            required: false,
                        });
                    }
                }
                if t.shots {
                    self.shot_items(&games, &mut out)?;
                }
            }
            SyncContent::Shots => self.shot_items(&games, &mut out)?,
        }
        if t.settings {
            let text = self.portable_settings();
            out.push(Item {
                path: SETTINGS_FILE.into(),
                size: text.len() as u64,
                fingerprint: blake3::hash(text.as_bytes()).to_hex().to_string(),
                source: Source::Bytes(text.into_bytes()),
                part: "settings",
                required: false,
            });
        }
        // 对象在前、引用在后：中断时远程不会出现指向缺失对象的引用。
        out.sort_by_key(|i| {
            let rank = if i.path.starts_with("objects/") {
                0
            } else if i.path.starts_with("refs/") {
                2
            } else {
                1
            };
            (rank, i.path.clone())
        });
        Ok(out)
    }

    /// 完整仓库：仓库目录中的文件，去掉写入锁、临时文件与可重建的索引；不带截图时去掉截图与相册。
    fn repo_items(&self, t: &SyncTarget, out: &mut Vec<Item>) -> Result<()> {
        let root = self.repo.root().to_path_buf();
        for e in walkdir::WalkDir::new(&root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !e.file_type().is_file() {
                continue;
            }
            let path = rel(&root, e.path());
            let top = path.split('/').next().unwrap_or("");
            let name = e.file_name().to_string_lossy();
            if path == "lock"
                || path.starts_with("objects/tmp/")
                || name.starts_with("bonfire.db")
                || name.ends_with(".tmp")
            {
                continue;
            }
            let shots = top == crate::shots::SHOTS_DIR || top == "albums";
            if shots && !t.shots {
                continue;
            }
            let meta = e
                .metadata()
                .map_err(|err| AppError::io(e.path(), err.into()))?;
            let object = path.starts_with("objects/");
            out.push(Item {
                fingerprint: if object { "o".into() } else { file_fp(&meta) },
                size: meta.len(),
                source: Source::File(e.path().to_path_buf()),
                part: if shots { "shots" } else { "saves" },
                required: !shots,
                path,
            });
        }
        Ok(())
    }

    /// 截图：`截图/<游戏名>/<文件名>`（相册中的，不含回收站）。
    fn shot_items(
        &self,
        games: &[bonfire_core::index::GameRecord],
        out: &mut Vec<Item>,
    ) -> Result<()> {
        for g in games {
            let dir = sanitize(&g.name);
            for s in self.index.shots_of(&g.id)? {
                if s.deleted_at.is_some() {
                    continue;
                }
                let Some(p) = self.shot_path(&s) else {
                    continue;
                };
                let Ok(meta) = std::fs::metadata(&p) else {
                    continue;
                };
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                out.push(Item {
                    path: format!("截图/{dir}/{name}"),
                    size: meta.len(),
                    fingerprint: s.blob.to_hex(),
                    source: Source::File(p),
                    part: "shots",
                    required: false,
                });
            }
        }
        Ok(())
    }

    /// 篝火设置：去掉密钥（IGDB、SteamGridDB）；同步目标的令牌与本机记住的密码本来就不在设置中。
    pub(crate) fn portable_settings(&self) -> String {
        let mut c = self.config.clone();
        c.metadata.igdb_client_id.clear();
        c.metadata.igdb_client_secret.clear();
        c.metadata.steamgriddb_api_key.clear();
        format!(
            "# 篝火 Bonfire 设置（同步备份生成；不含密钥、令牌与密码）\n\n{}",
            toml::to_string_pretty(&c).expect("配置序列化不会失败")
        )
    }

    pub(crate) fn item_bytes(&self, it: &Item) -> Result<Vec<u8>> {
        match &it.source {
            Source::File(p) => std::fs::read(p).map_err(|e| AppError::io(p, e)),
            Source::Blob(b) => {
                let mut v = Vec::with_capacity(it.size as usize);
                self.repo.read_blob_to(b, &mut v, Path::new(&it.path))?;
                Ok(v)
            }
            Source::Bytes(b) => Ok(b.clone()),
        }
    }
}
