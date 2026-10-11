//! 同步备份（REQ-015，0.2.0）：把仓库、最新存档、截图与篝火设置同步到远程。
//!
//! - 目标：GitHub、Gitee（内置 Git）、WebDAV、文件夹（本机目录、局域网共享、网盘客户端的同步文件夹）。
//!   可以同时设置多个目标，各自独立：内容、游戏范围、大小规则、同步时机。
//! - 内容：完整仓库（原样同步仓库文件，含历史，可从远程恢复；加密仓库远程也是加密的）/
//!   最新存档（每个游戏当前快照的文件，`<游戏名>/<文件>`，不加密）/ 只同步截图；另可选带截图、篝火设置。
//! - 每次同步先生成计划：要上传、要删除、被排除的文件（超过单文件上限）与同步后远程的大小；
//!   会超过总大小上限时不同步，给出原因。上传只传远程还没有的内容（按本机记录的同步状态比较）。
//! - 完整仓库按“对象 → 其他 → 引用”的顺序上传，中断时远程不会出现指向缺失对象的引用。
//! - 令牌与密码用 Windows 数据保护加密存在数据目录的 `keys/` 中，不写入配置文件与日志。

mod content;
mod folder;
pub mod target;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use content::{Item, Source};
pub use target::{SyncContent, SyncKind, SyncSchedule, SyncTarget};

use crate::app::App;
use crate::error::{AppError, Result};

/// 一个被排除的文件。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Excluded {
    pub path: String,
    pub size: u64,
    pub reason: String,
}

/// 远程大小与上限的关系。
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SizeLevel {
    Ok,
    /// 超过上限的 80%。
    Warn,
    /// 会超过上限：不同步。
    Over,
}

/// 同步计划（预览）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPlan {
    pub target_id: String,
    /// 要上传的文件数与大小。
    pub upload: usize,
    pub upload_bytes: u64,
    /// 远程要删除的文件（本机已不再有，如清理历史后）。
    pub delete: usize,
    /// 已经在远程、不需要再传的文件数。
    pub unchanged: usize,
    pub excluded: Vec<Excluded>,
    /// 同步后远程的大小（GitHub / Gitee 含历史：只增不减）。
    pub remote_bytes: u64,
    /// 总大小上限（0 为不限）。
    pub limit_bytes: u64,
    pub level: SizeLevel,
    /// 按内容分类的大小：存档、截图、设置。
    pub parts: Vec<(String, u64)>,
    /// 不能同步的原因（如会超过上限、存档分块超过单文件上限）。
    pub blocked: Option<String>,
}

/// 一次同步的结果。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncRun {
    pub at: String,
    pub uploaded: usize,
    pub uploaded_bytes: u64,
    pub deleted: usize,
    pub excluded: Vec<Excluded>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 每个目标在本机记录的同步状态：远程已有的文件（路径 → 指纹）与最近的同步记录。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncState {
    pub files: BTreeMap<String, String>,
    /// GitHub / Gitee 历史中累计推送的大小（远程只增不减，压缩远程后重置）。
    #[serde(default)]
    pub pushed_bytes: u64,
    #[serde(default)]
    pub runs: Vec<SyncRun>,
}

/// 目标的状态（同步页显示）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetStatus {
    pub target: SyncTarget,
    pub has_secret: bool,
    pub last: Option<SyncRun>,
    pub remote_files: usize,
}

/// 远程存储。
pub trait Remote {
    /// 测试连接，返回说明（如“可以写入”）。
    fn test(&mut self) -> Result<String>;
    fn put(&mut self, path: &str, data: &[u8]) -> Result<()>;
    fn delete(&mut self, path: &str) -> Result<()>;
    /// 一批改动结束（Git：提交并推送）。
    fn commit(&mut self, _message: &str) -> Result<()> {
        Ok(())
    }
    /// 每批最多上传多少字节后提交一次（0 为不分批）。
    fn batch_bytes(&self) -> u64 {
        0
    }
    /// 远程的全部文件（恢复时用）。
    fn list(&mut self) -> Result<Vec<(String, u64)>>;
    fn get(&mut self, path: &str) -> Result<Vec<u8>>;
}

const MB: u64 = 1024 * 1024;
/// 远程中的篝火设置文件。
pub const SETTINGS_FILE: &str = "bonfire-settings.toml";

impl App {
    fn sync_state_path(&self, id: &str) -> PathBuf {
        self.paths.cache_dir.join("sync").join(format!("{id}.json"))
    }

    pub fn sync_state(&self, id: &str) -> SyncState {
        std::fs::read(self.sync_state_path(id))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn save_sync_state(&self, id: &str, s: &SyncState) -> Result<()> {
        let p = self.sync_state_path(id);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| AppError::io(d, e))?;
        }
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(s).expect("可以序列化"))
            .map_err(|e| AppError::io(&tmp, e))?;
        std::fs::rename(&tmp, &p).map_err(|e| AppError::io(&p, e))
    }

    pub fn sync_target(&self, id: &str) -> Result<SyncTarget> {
        self.config
            .sync
            .targets
            .iter()
            .find(|t| t.id == id || t.name == id)
            .cloned()
            .ok_or_else(|| AppError::Invalid(format!("找不到同步目标「{id}」")))
    }

    /// 各目标的状态。
    pub fn sync_status(&self) -> Vec<TargetStatus> {
        self.config
            .sync
            .targets
            .iter()
            .map(|t| {
                let s = self.sync_state(&t.id);
                TargetStatus {
                    has_secret: self.sync_secret(&t.id).is_some(),
                    last: s.runs.last().cloned(),
                    remote_files: s.files.len(),
                    target: t.clone(),
                }
            })
            .collect()
    }

    /// 生成同步计划（不连接远程）。
    pub fn sync_plan(&self, t: &SyncTarget) -> Result<(SyncPlan, Vec<Item>, Vec<String>)> {
        let items = self.sync_items(t)?;
        let state = self.sync_state(&t.id);
        let max_file = t.max_file_mb * MB;
        let mut keep = Vec::new();
        let mut excluded = Vec::new();
        let mut blocked = None;
        let mut parts: BTreeMap<String, u64> = BTreeMap::new();
        for it in items {
            if max_file > 0 && it.size > max_file {
                if it.required {
                    blocked = Some(format!(
                        "仓库中的文件 {}（{}）超过单文件上限 {} MB；仓库文件不能跳过，请调高上限",
                        it.path,
                        crate::util::human_bytes(it.size),
                        t.max_file_mb
                    ));
                }
                excluded.push(Excluded {
                    path: it.path.clone(),
                    size: it.size,
                    reason: format!("超过单文件上限 {} MB", t.max_file_mb),
                });
                continue;
            }
            *parts.entry(it.part.to_string()).or_default() += it.size;
            keep.push(it);
        }
        let upload: Vec<Item> = keep
            .iter()
            .filter(|it| state.files.get(&it.path) != Some(&it.fingerprint))
            .cloned()
            .collect();
        let wanted: std::collections::HashSet<&str> =
            keep.iter().map(|i| i.path.as_str()).collect();
        let delete: Vec<String> = state
            .files
            .keys()
            .filter(|p| !wanted.contains(p.as_str()))
            .cloned()
            .collect();
        let upload_bytes: u64 = upload.iter().map(|i| i.size).sum();
        let current: u64 = keep.iter().map(|i| i.size).sum();
        // Git 远程保留历史：已推送的加上这次新增的。
        let remote_bytes = if t.kind.is_git() {
            state.pushed_bytes + upload_bytes
        } else {
            current
        };
        let limit_bytes = t.max_total_mb * MB;
        let level = if limit_bytes == 0 {
            SizeLevel::Ok
        } else if remote_bytes > limit_bytes {
            SizeLevel::Over
        } else if remote_bytes * 10 >= limit_bytes * 8 {
            SizeLevel::Warn
        } else {
            SizeLevel::Ok
        };
        if level == SizeLevel::Over && blocked.is_none() {
            blocked = Some(format!(
                "同步后远程约 {}，超过总大小上限 {} MB；可以不同步截图、只同步部分游戏、清理历史{}，或改用 WebDAV / 文件夹",
                crate::util::human_bytes(remote_bytes),
                t.max_total_mb,
                if t.kind.is_git() {
                    "后压缩远程仓库"
                } else {
                    ""
                }
            ));
        }
        let plan = SyncPlan {
            target_id: t.id.clone(),
            upload: upload.len(),
            upload_bytes,
            delete: delete.len(),
            unchanged: keep.len() - upload.len(),
            excluded,
            remote_bytes,
            limit_bytes,
            level,
            parts: parts.into_iter().collect(),
            blocked,
        };
        Ok((plan, upload, delete))
    }

    /// 打开目标的远程存储。
    pub fn sync_remote(&self, t: &SyncTarget) -> Result<Box<dyn Remote>> {
        match t.kind {
            SyncKind::Folder => Ok(Box::new(folder::Folder::new(&t.url))),
            _ => Err(AppError::Invalid(format!(
                "暂不支持的同步目标：{:?}",
                t.kind
            ))),
        }
    }

    /// 同步一个目标。`progress(已上传字节, 总字节)`。
    pub fn sync_push(&mut self, id: &str, progress: &mut dyn FnMut(u64, u64)) -> Result<SyncRun> {
        let t = self.sync_target(id)?;
        let (plan, upload, delete) = self.sync_plan(&t)?;
        let mut run = SyncRun {
            at: crate::util::now_local_rfc3339(),
            excluded: plan.excluded.clone(),
            ..Default::default()
        };
        let mut state = self.sync_state(&t.id);
        if let Some(why) = &plan.blocked {
            run.error = Some(why.clone());
            self.record_run(&t.id, &mut state, run.clone())?;
            return Err(AppError::Invalid(why.clone()));
        }
        let result = (|| -> Result<()> {
            let mut remote = self.sync_remote(&t)?;
            let batch = remote.batch_bytes();
            let mut in_batch = 0u64;
            let mut done = 0u64;
            for it in &upload {
                let data = self.item_bytes(it)?;
                remote.put(&it.path, &data)?;
                state.files.insert(it.path.clone(), it.fingerprint.clone());
                run.uploaded += 1;
                run.uploaded_bytes += it.size;
                done += it.size;
                in_batch += it.size;
                progress(done, plan.upload_bytes);
                if batch > 0 && in_batch >= batch {
                    remote.commit(&format!("篝火同步（{} 个文件）", run.uploaded))?;
                    state.pushed_bytes += in_batch;
                    in_batch = 0;
                    // 分批推送后立即记下进度，中断后不用重传。
                    self.save_sync_state(&t.id, &state)?;
                }
            }
            for p in &delete {
                remote.delete(p)?;
                state.files.remove(p);
                run.deleted += 1;
            }
            remote.commit(&format!(
                "篝火同步：上传 {} 个、删除 {} 个文件",
                run.uploaded, run.deleted
            ))?;
            state.pushed_bytes += in_batch;
            Ok(())
        })();
        if let Err(e) = &result {
            run.error = Some(e.to_string());
        }
        self.record_run(&t.id, &mut state, run.clone())?;
        result.map(|_| run)
    }

    fn record_run(&self, id: &str, state: &mut SyncState, run: SyncRun) -> Result<()> {
        state.runs.push(run);
        let n = state.runs.len();
        if n > 30 {
            state.runs.drain(..n - 30);
        }
        self.save_sync_state(id, state)
    }

    /// 测试连接。
    pub fn sync_test(&self, t: &SyncTarget) -> Result<String> {
        self.sync_remote(t)?.test()
    }

    /// 从远程恢复完整仓库到空文件夹 `dest`，返回恢复的文件数。之后可把仓库位置改为 `dest`。
    pub fn sync_restore(&self, id: &str, dest: &Path) -> Result<usize> {
        let t = self.sync_target(id)?;
        if t.content != SyncContent::Repo {
            return Err(AppError::Invalid("只有“完整仓库”的同步目标可以恢复".into()));
        }
        if dest.exists()
            && std::fs::read_dir(dest)
                .map_err(|e| AppError::io(dest, e))?
                .next()
                .is_some()
        {
            return Err(AppError::Invalid(format!(
                "{} 不是空文件夹",
                dest.display()
            )));
        }
        let mut remote = self.sync_remote(&t)?;
        let files = remote.list()?;
        if !files.iter().any(|(p, _)| p == "config.toml") {
            return Err(AppError::Invalid(
                "远程没有篝火仓库（缺少 config.toml）".into(),
            ));
        }
        let mut n = 0;
        for (p, _) in &files {
            let data = remote.get(p)?;
            let out = dest.join(p);
            if let Some(d) = out.parent() {
                std::fs::create_dir_all(d).map_err(|e| AppError::io(d, e))?;
            }
            std::fs::write(&out, data).map_err(|e| AppError::io(&out, e))?;
            n += 1;
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests;
