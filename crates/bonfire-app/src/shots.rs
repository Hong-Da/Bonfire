//! 截图备份（截图备份方案 S1–S6，U-9）：每个游戏一个“相册”，与存档快照分开。
//!
//! - 来源：Steam 截图（按 AppID）、Windows 游戏栏 / Xbox 应用的“捕获”文件夹（按文件名中的标题）、
//!   为游戏手动添加的截图文件夹。收集时来源目录只读。
//! - 原图原样（不压缩、不重新编码）存放在仓库的 `screenshots/<游戏>/`，可以直接在资源管理器中打开；
//!   同一游戏内按内容哈希去重。索引表 `screenshots` 加速查询，仓库中的 `albums/<游戏>.jsonl`
//!   是相册记录的事实来源，重建索引时据此恢复。
//! - 只增不减：磁盘上删掉的截图，备份中仍保留。在相册中删除先进入回收站（`screenshots/<游戏>/.trash/`），
//!   可恢复；彻底删除后只留下记录，磁盘上的原图不会被再次收集。彻底删除时用户可选同时删除磁盘上的原图
//!   （移到系统回收站，只删内容与备份一致的），这是除“写回”外唯一改动来源文件夹的操作。
//! - 收集时机：每次备份时顺带收集；另可按天 / 周 / 小时定时收集（[`shots_due`]）。
//! - 游戏可设为“只管理截图”：不备份、不检查存档。
//! - 缩略图放在缓存目录 `shots/<游戏>/`，可随时重建。

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use bonfire_core::index::{GameRecord, ShotRecord, ShotSettings};
use bonfire_core::{Kind, ObjectId, RepoLock};
use bonfire_discovery::platforms::PlatformAdapter;
use bonfire_discovery::shots::{self as src, ShotFile};
use serde::Serialize;

use crate::app::App;
#[cfg(test)]
use crate::config::ShotSchedule;
use crate::config::Shots;
use crate::error::{AppError, Result};
use crate::transfer::{ExportFormat, replace_file, unique_child, with_suffix, zip_err, zip_time};
use crate::util::{now_compact, now_local_rfc3339};

/// “捕获”文件夹在系统设置中的位置（Known Folder `Captures`）。
const CAPTURES_KEY: &str =
    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders";
const CAPTURES_VALUE: &str = "{EDC0FE71-98D8-4F4A-B920-C8DC133CB165}";
/// 仓库中存放截图原图的目录。
pub const SHOTS_DIR: &str = "screenshots";
/// 每个游戏截图目录下的回收站。
pub const TRASH_DIR: &str = ".trash";
/// 缩略图的最大尺寸（高分屏下网格中也清晰）。
const THUMB: u32 = 480;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotSyncReport {
    pub game_id: String,
    /// 新收集的截图数。
    pub added: usize,
    /// 新收集截图的总大小。
    pub bytes: u64,
    /// 游戏关闭了截图备份。
    pub disabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotExportReport {
    pub dest: PathBuf,
    pub files: u64,
    pub bytes: u64,
}

/// 详情页“截图”标签页用的相册视图。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumView {
    /// 相册中的截图（不含回收站）。
    pub shots: Vec<ShotRecord>,
    /// 回收站中的截图。
    pub trash: Vec<ShotRecord>,
    pub enabled: bool,
    /// 跟随全局设置（没有为这个游戏单独设置）。
    pub follows_global: bool,
    /// 全局设置是否收集截图。
    pub global_enabled: bool,
    pub dirs: Vec<String>,
    /// 磁盘上还没有收集的截图数。
    pub pending: usize,
    pub bytes: u64,
    pub trash_bytes: u64,
    /// 本机能找到的来源（用于提示截图从哪里来）。
    pub sources: Vec<String>,
    /// 备份中的原图所在的文件夹。
    pub dir: PathBuf,
    /// 只管理截图（不备份存档）。
    pub shots_only: bool,
}

/// 彻底删除截图的结果。
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PurgeReport {
    pub purged: usize,
    /// 同时删除的磁盘上的原图数（移到系统回收站）。
    pub originals_deleted: usize,
    /// 磁盘上的原图内容已变，未删除。
    pub originals_kept: usize,
    /// 删除失败（被占用等）。
    pub originals_failed: usize,
}

/// 有截图、但还不在游戏库中的游戏（发现页“有截图的游戏”）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotCandidate {
    /// `steam:<AppID>` 或 `capture:<窗口标题>`。
    pub key: String,
    pub name: String,
    /// `steam` / `capture`
    pub source: String,
    pub app_id: Option<String>,
    pub count: usize,
    /// 截图所在的文件夹。
    pub dir: PathBuf,
}

/// 游戏的管理方式（U-10）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ManageMode {
    /// 存档和截图。
    Both,
    /// 只管理存档（不收集截图）。
    Saves,
    /// 只管理截图（不备份、不检查存档）。
    Shots,
}

impl ManageMode {
    pub fn as_str(self) -> &'static str {
        match self {
            ManageMode::Both => "both",
            ManageMode::Saves => "saves",
            ManageMode::Shots => "shots",
        }
    }
}

impl std::str::FromStr for ManageMode {
    type Err = AppError;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "both" => Ok(ManageMode::Both),
            "saves" => Ok(ManageMode::Saves),
            "shots" => Ok(ManageMode::Shots),
            _ => Err(AppError::Invalid(format!(
                "管理方式应为 both / saves / shots：{s}"
            ))),
        }
    }
}

impl App {
    /// 游戏是否收集截图：游戏单独设置优先，否则跟随全局设置。
    pub fn shots_enabled(&self, game: &GameRecord) -> Result<bool> {
        Ok(self
            .index
            .shot_settings(&game.id)?
            .enabled
            .unwrap_or(self.config.shots.enabled))
    }

    /// 游戏是否只管理截图（不备份、不检查存档）。
    pub fn is_shots_only(&self, game_id: &str) -> Result<bool> {
        Ok(self.index.shot_settings(game_id)?.shots_only)
    }

    /// 设为只管理截图，或恢复管理存档。只管理截图时截图收集随之开启。
    pub fn set_shots_only(&mut self, game: &GameRecord, on: bool) -> Result<()> {
        let mut s = self.index.shot_settings(&game.id)?;
        s.shots_only = on;
        if on {
            s.enabled = self.enabled_on();
        }
        self.index.save_shot_settings(&game.id, &s)?;
        self.save_games_file()
    }

    /// 截图收集开启时的游戏设置：全局开启时跟随全局，否则为这个游戏单独开启。
    fn enabled_on(&self) -> Option<bool> {
        (!self.config.shots.enabled).then_some(true)
    }

    /// 游戏的管理方式：存档和截图 / 只管理存档 / 只管理截图。
    pub fn manage_mode(&self, game: &GameRecord) -> Result<ManageMode> {
        Ok(if self.is_shots_only(&game.id)? {
            ManageMode::Shots
        } else if self.shots_enabled(game)? {
            ManageMode::Both
        } else {
            ManageMode::Saves
        })
    }

    /// 切换管理方式。已有的快照与截图都保留，可以随时改回来。
    pub fn set_manage_mode(&mut self, game: &GameRecord, mode: ManageMode) -> Result<()> {
        let mut s = self.index.shot_settings(&game.id)?;
        s.shots_only = mode == ManageMode::Shots;
        s.enabled = match mode {
            ManageMode::Saves => Some(false),
            ManageMode::Both | ManageMode::Shots => self.enabled_on(),
        };
        self.index.save_shot_settings(&game.id, &s)?;
        self.save_games_file()
    }

    /// Steam 安装目录（伪造根目录下为 `<root>/Steam`）。
    pub fn steam_root(&self) -> Option<PathBuf> {
        match &self.fake_root {
            Some(root) => Some(root.join("Steam")).filter(|p| p.is_dir()),
            None => bonfire_discovery::platforms::SteamAdapter::detect().map(|s| s.root),
        }
    }

    /// 存在的公用截图文件夹（设置中添加的；伪造根目录下测试时同样按配置）。
    pub fn shared_dirs(&self) -> Vec<PathBuf> {
        self.config
            .shots
            .shared_dirs
            .iter()
            .filter(|d| d.is_dir())
            .cloned()
            .collect()
    }

    /// “捕获”文件夹：配置优先，其次系统设置中的实际位置，最后 `<用户目录>\Videos\Captures`。
    pub fn captures_dir(&self) -> Option<PathBuf> {
        if let Some(d) = &self.config.shots.captures_dir {
            return Some(d.clone());
        }
        if self.fake_root.is_none()
            && let Some(v) = bonfire_discovery::registry::read_string(CAPTURES_KEY, CAPTURES_VALUE)
        {
            return Some(PathBuf::from(expand_env(&v)));
        }
        self.known_dirs
            .home
            .as_ref()
            .map(|h| h.join("Videos").join("Captures"))
    }

    /// 游戏的各种名称（英文名、中文名），用于对应游戏栏截图的窗口标题。
    pub(crate) fn shot_names(&self, game: &GameRecord) -> Result<Vec<String>> {
        let mut names = vec![game.name.clone()];
        if let Some(z) = self.metadata(&game.id)?.and_then(|m| m.data.name_zh) {
            names.push(z);
        }
        Ok(names)
    }

    /// 磁盘上属于这个游戏的全部截图（只读）。
    pub fn shot_files(&self, game: &GameRecord) -> Result<Vec<ShotFile>> {
        let mut out = Vec::new();
        if game.platform == "steam"
            && let (Some(app_id), Some(root)) = (&game.app_id, self.steam_root())
        {
            out.extend(src::steam_shots(&root, app_id));
        }
        if let Some(dir) = self.captures_dir() {
            let names = self.shot_names(game)?;
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            // 其他游戏的名称：同一标题对应多个游戏时，只归到对应得最好的那个。
            let mut others = Vec::new();
            for g in self.games()?.iter().filter(|g| g.id != game.id) {
                others.extend(self.shot_names(g)?);
            }
            let other_refs: Vec<&str> = others.iter().map(String::as_str).collect();
            out.extend(src::capture_shots(&dir, &refs, &other_refs));
            for d in self.shared_dirs() {
                out.extend(src::shared_shots(&d, &refs, &other_refs));
            }
        }
        for d in self.index.shot_settings(&game.id)?.dirs {
            out.extend(src::folder_shots(Path::new(&d)));
        }
        Ok(out)
    }

    fn shot_sources(&self, game: &GameRecord) -> Result<Vec<String>> {
        let mut v = Vec::new();
        if game.platform == "steam" && self.steam_root().is_some() {
            v.push("steam".to_string());
        }
        if self.captures_dir().is_some_and(|d| d.is_dir()) {
            v.push("capture".to_string());
        }
        if !self.shared_dirs().is_empty() {
            v.push("shared".to_string());
        }
        if !self.index.shot_settings(&game.id)?.dirs.is_empty() {
            v.push("folder".to_string());
        }
        Ok(v)
    }

    /// 还没有收集的截图（按原路径与大小判断，不读文件内容）。
    /// 回收站中与彻底删除的截图也算已收集，不会再次收集。
    pub(crate) fn uncollected(&self, game: &GameRecord) -> Result<Vec<ShotFile>> {
        // 路径按规范化后比较（大小写、斜杠方向不同的同一文件算同一个）。
        let key = |p: &Path| crate::transfer::path_key(p);
        let known: HashSet<(String, u64)> = self
            .index
            .shots_of(&game.id)?
            .into_iter()
            .map(|s| (key(Path::new(&s.original_path)), s.size))
            .collect();
        Ok(self
            .shot_files(game)?
            .into_iter()
            .filter(|f| !known.contains(&(key(&f.path), f.size)))
            .collect())
    }

    /// 磁盘上还没有收集的截图数（游戏库“检查更新”时提示）；关闭截图备份的游戏为 0。
    pub fn pending_shots(&self, game: &GameRecord) -> Result<usize> {
        if !self.shots_enabled(game)? {
            return Ok(0);
        }
        Ok(self.uncollected(game)?.len())
    }

    /// 收集新截图：原样复制到仓库的截图目录。来源目录只读；内容与已收集的相同时不重复存。
    pub fn sync_shots(&mut self, _lock: &RepoLock, game: &GameRecord) -> Result<ShotSyncReport> {
        let mut report = ShotSyncReport {
            game_id: game.id.clone(),
            ..Default::default()
        };
        if !self.shots_enabled(game)? {
            report.disabled = true;
            return Ok(report);
        }
        // 先把“未识别的截图”中属于这个游戏的移过来，不重复收集。
        self.adopt_unsorted(game)?;
        let files = self.uncollected(game)?;
        self.collect_files(game, files, report)
    }

    /// 把磁盘上的截图原样收集到备份中（内容与已收集的相同时跳过）。
    pub(crate) fn collect_files(
        &mut self,
        game: &GameRecord,
        files: Vec<ShotFile>,
        mut report: ShotSyncReport,
    ) -> Result<ShotSyncReport> {
        if files.is_empty() {
            return Ok(report);
        }
        let mut known: HashSet<ObjectId> = self
            .index
            .shots_of(&game.id)?
            .into_iter()
            .map(|s| s.blob)
            .collect();
        for f in files {
            let data = match std::fs::read(&f.path) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!(path = %f.path.display(), "读取截图失败，跳过：{e}");
                    continue;
                }
            };
            let blob = ObjectId::of(&data);
            if !known.insert(blob) {
                continue;
            }
            // 按内容判断格式（扩展名可能与内容不符）。
            let (width, height) = image::ImageReader::new(std::io::Cursor::new(&data))
                .with_guessed_format()
                .ok()
                .and_then(|r| r.into_dimensions().ok())
                .map(|(w, h)| (Some(w), Some(h)))
                .unwrap_or((None, None));
            let file =
                self.store_shot_file(&game.id, "", &shot_file_name(&f.path, &blob), &data)?;
            let record = ShotRecord {
                game_id: game.id.clone(),
                blob,
                taken_at: taken_at(&f),
                source: f.source.as_str().to_string(),
                original_path: f.path.to_string_lossy().into_owned(),
                size: data.len() as u64,
                width,
                height,
                added_at: now_local_rfc3339(),
                file,
                deleted_at: None,
                purged: false,
            };
            if self.index.add_shot(&record)? {
                report.added += 1;
                report.bytes += record.size;
            }
        }
        if report.added > 0 {
            self.save_album(&game.id)?;
        }
        Ok(report)
    }

    /// 收集所有开启了截图备份的游戏（定时收集、CLI `shots sync` 不带游戏时）。
    pub fn sync_all_shots(&mut self, lock: &RepoLock) -> Result<Vec<ShotSyncReport>> {
        let mut out = Vec::new();
        for g in self.games()? {
            match self.sync_shots(lock, &g) {
                Ok(r) => out.push(r),
                Err(e) => tracing::warn!(game = %g.id, "收集截图失败：{e}"),
            }
        }
        // 对应不上任何游戏的截图放到“未识别的截图”。
        match self.sync_unsorted(lock) {
            Ok(r) if r.added > 0 => out.push(r),
            Ok(_) => {}
            Err(e) => tracing::warn!("收集未识别的截图失败：{e}"),
        }
        self.mark_shots_synced()?;
        Ok(out)
    }

    pub fn album(&self, game: &GameRecord) -> Result<AlbumView> {
        let settings = self.index.shot_settings(&game.id)?;
        let (trash, shots): (Vec<ShotRecord>, Vec<ShotRecord>) = self
            .index
            .shots_of(&game.id)?
            .into_iter()
            .filter(|s| !s.purged)
            .partition(|s| s.deleted_at.is_some());
        Ok(AlbumView {
            bytes: shots.iter().map(|s| s.size).sum(),
            trash_bytes: trash.iter().map(|s| s.size).sum(),
            shots,
            trash,
            enabled: self.shots_enabled(game)?,
            follows_global: settings.enabled.is_none(),
            global_enabled: self.config.shots.enabled,
            dirs: settings.dirs,
            pending: self.pending_shots(game)?,
            sources: self.shot_sources(game)?,
            dir: self.shot_dir(&game.id),
            shots_only: settings.shots_only,
        })
    }

    /// 相册中的截图数（不含回收站）。
    pub fn shot_count(&self, game_id: &str) -> Result<u64> {
        Ok(self
            .index
            .shots_of(game_id)?
            .iter()
            .filter(|s| !s.purged && s.deleted_at.is_none())
            .count() as u64)
    }

    /// 修改游戏的截图设置：`enabled` 为 None 时跟随全局设置。
    pub fn set_shots_enabled(&mut self, game: &GameRecord, enabled: Option<bool>) -> Result<()> {
        let mut s = self.index.shot_settings(&game.id)?;
        s.enabled = enabled;
        self.index.save_shot_settings(&game.id, &s)?;
        self.save_games_file()
    }

    /// 为游戏添加截图文件夹（必须存在，不能是仓库或数据目录）。
    pub fn add_shot_dir(&mut self, game: &GameRecord, dir: &Path) -> Result<String> {
        let (s, key) = self.check_shot_dir(dir)?;
        let mut settings = self.index.shot_settings(&game.id)?;
        if !settings
            .dirs
            .iter()
            .any(|d| crate::transfer::path_key(Path::new(d)) == key)
        {
            settings.dirs.push(s.clone());
            self.index.save_shot_settings(&game.id, &settings)?;
            self.save_games_file()?;
        }
        Ok(s)
    }

    /// 截图文件夹必须存在，且不能是仓库或篝火的数据目录。返回绝对路径与比较用的键。
    fn check_shot_dir(&self, dir: &Path) -> Result<(String, String)> {
        if !dir.is_dir() {
            return Err(AppError::PathNotFound(dir.to_path_buf()));
        }
        let abs = std::path::absolute(dir).map_err(|e| AppError::io(dir, e))?;
        let key = crate::transfer::path_key(&abs);
        let inside = |base: &Path| {
            let b = crate::transfer::path_key(base);
            key == b || key.starts_with(&format!("{b}/")) || b.starts_with(&format!("{key}/"))
        };
        let data_dir = self
            .paths
            .cache_dir
            .parent()
            .unwrap_or(&self.paths.cache_dir);
        if inside(self.repo.root()) || inside(data_dir) {
            return Err(AppError::Invalid(format!(
                "不能把仓库或篝火的数据目录作为截图文件夹：{}",
                abs.display()
            )));
        }
        Ok((abs.to_string_lossy().into_owned(), key))
    }

    /// 移除截图文件夹（已收集的截图保留）。
    pub fn remove_shot_dir(&mut self, game: &GameRecord, dir: &str) -> Result<bool> {
        let key = crate::transfer::path_key(Path::new(dir));
        let mut settings = self.index.shot_settings(&game.id)?;
        let before = settings.dirs.len();
        settings
            .dirs
            .retain(|d| crate::transfer::path_key(Path::new(d)) != key);
        let removed = settings.dirs.len() != before;
        if removed {
            self.index.save_shot_settings(&game.id, &settings)?;
            self.save_games_file()?;
        }
        Ok(removed)
    }

    // ---------- 删除、回收站 ----------

    /// 把截图移入回收站（只动备份，不碰磁盘上的原图）。返回移入的数量。
    pub fn delete_shots(
        &mut self,
        _lock: &RepoLock,
        game: &GameRecord,
        blobs: &[ObjectId],
    ) -> Result<usize> {
        let wanted: HashSet<&ObjectId> = blobs.iter().collect();
        let now = now_local_rfc3339();
        let mut n = 0;
        for mut s in self.index.shots_of(&game.id)? {
            if !wanted.contains(&s.blob) || s.purged || s.deleted_at.is_some() {
                continue;
            }
            s.file = self.move_shot_file(&s, TRASH_DIR)?;
            s.deleted_at = Some(now.clone());
            self.index.update_shot(&s)?;
            n += 1;
        }
        if n > 0 {
            self.save_album(&game.id)?;
        }
        Ok(n)
    }

    /// 从回收站恢复截图。返回恢复的数量。
    pub fn restore_shots(
        &mut self,
        _lock: &RepoLock,
        game: &GameRecord,
        blobs: &[ObjectId],
    ) -> Result<usize> {
        let wanted: HashSet<&ObjectId> = blobs.iter().collect();
        let mut n = 0;
        for mut s in self.index.shots_of(&game.id)? {
            if !wanted.contains(&s.blob) || s.purged || s.deleted_at.is_none() {
                continue;
            }
            s.file = self.move_shot_file(&s, "")?;
            s.deleted_at = None;
            self.index.update_shot(&s)?;
            n += 1;
        }
        if n > 0 {
            self.save_album(&game.id)?;
        }
        Ok(n)
    }

    /// 彻底删除截图（`blobs` 为空时清空回收站）：删除备份中的原图，只留下记录，
    /// 磁盘上的原图不会再被收集。返回删除的数量。
    pub fn purge_shots(
        &mut self,
        lock: &RepoLock,
        game: &GameRecord,
        blobs: &[ObjectId],
    ) -> Result<usize> {
        Ok(self.purge_shots_with(lock, game, blobs, false)?.purged)
    }

    /// 彻底删除截图；`originals` 时同时删除磁盘上的原图（U-11）：
    /// 只删除内容与备份一致的原图（内容变了的保留），移到系统回收站（伪造根目录下直接删除）。
    pub fn purge_shots_with(
        &mut self,
        _lock: &RepoLock,
        game: &GameRecord,
        blobs: &[ObjectId],
        originals: bool,
    ) -> Result<PurgeReport> {
        let wanted: HashSet<&ObjectId> = blobs.iter().collect();
        let now = now_local_rfc3339();
        let mut report = PurgeReport::default();
        let mut n = 0;
        for mut s in self.index.shots_of(&game.id)? {
            let pick = if blobs.is_empty() {
                s.deleted_at.is_some()
            } else {
                wanted.contains(&s.blob)
            };
            if !pick || s.purged {
                continue;
            }
            if let Some(p) = self.shot_path(&s) {
                match std::fs::remove_file(&p) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(AppError::io(&p, e)),
                }
            }
            let _ = std::fs::remove_file(self.shot_thumb_path(&game.id, &s.blob));
            if originals {
                self.remove_original(&s, &mut report);
            }
            s.file = String::new();
            s.purged = true;
            s.deleted_at.get_or_insert(now.clone());
            self.index.update_shot(&s)?;
            n += 1;
        }
        if n > 0 {
            self.save_album(&game.id)?;
        }
        report.purged = n;
        if report.originals_deleted > 0 {
            tracing::info!(game = %game.id, n = report.originals_deleted, "已删除磁盘上的截图原图");
        }
        Ok(report)
    }

    /// 删除磁盘上的截图原图：不存在时跳过；内容与备份不一致时保留；移到系统回收站。
    fn remove_original(&self, s: &ShotRecord, report: &mut PurgeReport) {
        let p = PathBuf::from(&s.original_path);
        if !p.is_file() {
            return;
        }
        let same = std::fs::read(&p)
            .map(|d| ObjectId::of(&d) == s.blob)
            .unwrap_or(false);
        if !same {
            report.originals_kept += 1;
            return;
        }
        // 开发与测试（伪造根目录）时直接删除，不往真实的系统回收站里放东西。
        let r = if self.fake_root.is_some() {
            std::fs::remove_file(&p).map_err(|e| e.to_string())
        } else {
            trash::delete(&p).map_err(|e| e.to_string())
        };
        match r {
            Ok(()) => report.originals_deleted += 1,
            Err(e) => {
                tracing::warn!(path = %p.display(), "删除截图原图失败：{e}");
                report.originals_failed += 1;
            }
        }
    }

    /// 删除游戏的全部截图备份（相册、回收站与删除记录，U-15）：删除仓库中的截图文件夹与相册文件、缩略图缓存。
    /// 不留删除记录，之后重新加入游戏时磁盘上的原图会重新收集。磁盘上的原图不受影响。返回删除的截图数。
    pub fn delete_all_shots(&mut self, _lock: &RepoLock, game: &GameRecord) -> Result<usize> {
        let n = self
            .index
            .shots_of(&game.id)?
            .iter()
            .filter(|s| !s.purged)
            .count();
        for dir in [self.shot_dir(&game.id), self.shot_cache_dir(&game.id)] {
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(AppError::io(&dir, e)),
            }
        }
        self.index.replace_shots(&game.id, &[])?;
        let album = self.album_path(&game.id);
        match std::fs::remove_file(&album) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(AppError::io(&album, e)),
        }
        Ok(n)
    }

    /// 按前缀（≥ 6 位）找到相册（含回收站）中的截图。
    pub fn resolve_shots(&self, game: &GameRecord, specs: &[String]) -> Result<Vec<ObjectId>> {
        let shots: Vec<ShotRecord> = self
            .index
            .shots_of(&game.id)?
            .into_iter()
            .filter(|s| !s.purged)
            .collect();
        let mut out = Vec::new();
        for spec in specs {
            let p = spec.trim().trim_start_matches("b3:").to_lowercase();
            if p.len() < 6 {
                return Err(AppError::Invalid(format!("截图 ID 至少 6 位：{spec}")));
            }
            let hits: Vec<&ShotRecord> = shots
                .iter()
                .filter(|s| s.blob.to_hex().starts_with(&p))
                .collect();
            match hits.len() {
                0 => return Err(AppError::Invalid(format!("相册中没有截图 {spec}"))),
                1 => out.push(hits[0].blob),
                _ => {
                    return Err(AppError::Invalid(format!(
                        "截图 ID {spec} 不唯一，请多写几位"
                    )));
                }
            }
        }
        Ok(out)
    }

    // ---------- 原图文件 ----------

    /// 游戏的截图目录（备份中的原图）。
    pub fn shot_dir(&self, game_id: &str) -> PathBuf {
        self.repo.root().join(SHOTS_DIR).join(game_id)
    }

    /// 截图在备份中的原图；还在对象库中（早期版本）时为 None。
    pub fn shot_path(&self, s: &ShotRecord) -> Option<PathBuf> {
        (!s.file.is_empty()).then(|| self.shot_dir(&s.game_id).join(&s.file))
    }

    /// 把截图原样写入游戏截图目录下的 `sub`（"" 或回收站），重名时加序号。返回相对路径。
    fn store_shot_file(&self, game_id: &str, sub: &str, name: &str, data: &[u8]) -> Result<String> {
        let dir = self.shot_sub_dir(game_id, sub);
        std::fs::create_dir_all(&dir).map_err(|e| AppError::io(&dir, e))?;
        let dest = free_name(&dir, name);
        let tmp = with_suffix(&dest, ".tmp");
        let write = || -> std::io::Result<()> {
            let mut f = std::fs::File::create(&tmp)?;
            f.write_all(data)?;
            f.sync_all()
        };
        write().map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            AppError::io(&tmp, e)
        })?;
        replace_file(&tmp, &dest)?;
        Ok(rel_name(sub, &dest))
    }

    /// 导入的截图：原样写入游戏截图目录。返回相对路径。
    pub(crate) fn store_album_file(
        &self,
        game_id: &str,
        s: &ShotRecord,
        data: &[u8],
    ) -> Result<String> {
        self.store_shot_file(
            game_id,
            "",
            &shot_file_name(Path::new(&s.original_path), &s.blob),
            data,
        )
    }

    /// 在相册目录与回收站之间移动原图。原图不在时返回空（只改记录）。
    fn move_shot_file(&self, s: &ShotRecord, sub: &str) -> Result<String> {
        let Some(from) = self.shot_path(s).filter(|p| p.is_file()) else {
            return Ok(String::new());
        };
        let dir = self.shot_sub_dir(&s.game_id, sub);
        std::fs::create_dir_all(&dir).map_err(|e| AppError::io(&dir, e))?;
        let name = from
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| s.blob.short(12));
        let dest = free_name(&dir, &name);
        std::fs::rename(&from, &dest).map_err(|e| AppError::io(&dest, e))?;
        Ok(rel_name(sub, &dest))
    }

    fn shot_sub_dir(&self, game_id: &str, sub: &str) -> PathBuf {
        let d = self.shot_dir(game_id);
        if sub.is_empty() { d } else { d.join(sub) }
    }

    fn shot_cache_dir(&self, game_id: &str) -> PathBuf {
        self.paths.cache_dir.join("shots").join(game_id)
    }

    fn shot_thumb_path(&self, game_id: &str, blob: &ObjectId) -> PathBuf {
        self.shot_cache_dir(game_id)
            .join(format!("{}-{THUMB}.webp", blob.to_hex()))
    }

    fn shot_record(&self, game_id: &str, blob: &ObjectId) -> Result<ShotRecord> {
        self.index
            .shots_of(game_id)?
            .into_iter()
            .find(|s| s.blob == *blob && !s.purged)
            .ok_or_else(|| AppError::Invalid(format!("相册中没有截图 {}", blob.short(12))))
    }

    /// 截图内容：备份中的原图，早期版本从对象库中取出。
    pub(crate) fn shot_bytes(&self, s: &ShotRecord) -> Result<Vec<u8>> {
        if let Some(p) = self.shot_path(s) {
            return std::fs::read(&p).map_err(|e| AppError::io(&p, e));
        }
        let mut buf = Vec::new();
        self.repo
            .read_blob_to(&s.blob, &mut buf, self.repo.root())?;
        Ok(buf)
    }

    /// 缩略图（缓存中没有时由备份中的原图生成）。
    pub fn shot_thumb(&self, game_id: &str, blob: &ObjectId) -> Result<PathBuf> {
        let path = self.shot_thumb_path(game_id, blob);
        if !path.is_file() {
            let rec = self.shot_record(game_id, blob)?;
            let bytes = self.shot_bytes(&rec)?;
            let img = image::load_from_memory(&bytes)
                .map_err(|e| AppError::Invalid(format!("无法解码截图：{e}")))?
                .thumbnail(THUMB, THUMB);
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, e))?;
            }
            let tmp = path.with_extension("webp.tmp");
            img.to_rgba8()
                .save_with_format(&tmp, image::ImageFormat::WebP)
                .map_err(|e| AppError::Invalid(format!("保存缩略图失败：{e}")))?;
            std::fs::rename(&tmp, &path).map_err(|e| AppError::io(&path, e))?;
        }
        Ok(path)
    }

    /// 查看大图用的原图：备份中的原图文件（早期版本先取出到缓存目录）。
    pub fn shot_image(&self, game_id: &str, blob: &ObjectId) -> Result<PathBuf> {
        let rec = self.shot_record(game_id, blob)?;
        if let Some(p) = self.shot_path(&rec).filter(|p| p.is_file()) {
            return Ok(p);
        }
        let ext = Path::new(&rec.original_path)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_else(|| "png".into());
        let path = self
            .shot_cache_dir(game_id)
            .join("full")
            .join(format!("{}.{ext}", blob.to_hex()));
        if !path.is_file() {
            let bytes = self.shot_bytes(&rec)?;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, e))?;
            }
            let tmp = with_suffix(&path, ".tmp");
            std::fs::write(&tmp, &bytes).map_err(|e| AppError::io(&tmp, e))?;
            std::fs::rename(&tmp, &path).map_err(|e| AppError::io(&path, e))?;
        }
        Ok(path)
    }

    /// 早期版本存放在对象库中的截图：取出为普通文件（打开仓库时）。返回迁移的数量。
    pub(crate) fn migrate_shot_files(&mut self) -> Result<usize> {
        let mut games = HashSet::new();
        let mut n = 0;
        for mut s in self.index.shots_without_file()? {
            if !self.repo.has(Kind::Blob, &s.blob) {
                continue;
            }
            let data = match self.shot_bytes(&s) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!(game = %s.game_id, "迁移截图失败：{e}");
                    continue;
                }
            };
            let sub = if s.deleted_at.is_some() {
                TRASH_DIR
            } else {
                ""
            };
            s.file = self.store_shot_file(
                &s.game_id,
                sub,
                &shot_file_name(Path::new(&s.original_path), &s.blob),
                &data,
            )?;
            self.index.update_shot(&s)?;
            games.insert(s.game_id.clone());
            n += 1;
        }
        for g in games {
            self.save_album(&g)?;
        }
        if n > 0 {
            tracing::info!("已把 {n} 张截图迁移为普通文件");
        }
        Ok(n)
    }

    /// 导出截图（`blobs` 为空表示相册中全部）到 zip 或新文件夹，文件名用原文件名（重名加序号）。
    pub fn export_shots(
        &self,
        game: &GameRecord,
        blobs: &[ObjectId],
        dest: &Path,
        format: ExportFormat,
    ) -> Result<ShotExportReport> {
        let all = self.index.shots_of(&game.id)?;
        let wanted: HashSet<&ObjectId> = blobs.iter().collect();
        let mut chosen: Vec<&ShotRecord> = all
            .iter()
            .filter(|s| !s.purged)
            .filter(|s| {
                if wanted.is_empty() {
                    s.deleted_at.is_none()
                } else {
                    wanted.contains(&s.blob)
                }
            })
            .collect();
        if chosen.is_empty() {
            return Err(AppError::Invalid(format!(
                "「{}」没有要导出的截图",
                game.name
            )));
        }
        chosen.sort_by(|a, b| a.taken_at.cmp(&b.taken_at));
        let names = file_names(&chosen);
        self.check_export_target(dest)?;
        let mut bytes = 0;
        let out = match format {
            ExportFormat::Folder => {
                let dir = unique_child(
                    dest,
                    &format!("{}-截图-{}", sanitize(&game.name), now_compact()),
                );
                std::fs::create_dir_all(&dir).map_err(|e| AppError::io(&dir, e))?;
                for (s, name) in chosen.iter().zip(&names) {
                    let data = self.shot_bytes(s)?;
                    bytes += data.len() as u64;
                    let p = dir.join(name);
                    std::fs::write(&p, &data).map_err(|e| AppError::io(&p, e))?;
                }
                dir
            }
            ExportFormat::Zip => {
                let tmp = with_suffix(dest, ".bonfire-tmp");
                let result = (|| -> Result<u64> {
                    let file = std::fs::File::create(&tmp).map_err(|e| AppError::io(&tmp, e))?;
                    let mut zip = zip::ZipWriter::new(file);
                    let mut total = 0;
                    for (s, name) in chosen.iter().zip(&names) {
                        // 图片本身已压缩，原样存放。
                        let opts = zip::write::SimpleFileOptions::default()
                            .compression_method(zip::CompressionMethod::Stored)
                            .large_file(true)
                            .last_modified_time(zip_time(Some(&s.taken_at)));
                        let data = self.shot_bytes(s)?;
                        total += data.len() as u64;
                        zip.start_file(name.as_str(), opts)
                            .map_err(|e| zip_err(&tmp, e))?;
                        zip.write_all(&data).map_err(|e| AppError::io(&tmp, e))?;
                    }
                    let f = zip.finish().map_err(|e| zip_err(&tmp, e))?;
                    f.sync_all().map_err(|e| AppError::io(&tmp, e))?;
                    Ok(total)
                })();
                match result {
                    Ok(n) => bytes = n,
                    Err(e) => {
                        let _ = std::fs::remove_file(&tmp);
                        return Err(e);
                    }
                }
                replace_file(&tmp, dest)?;
                dest.to_path_buf()
            }
        };
        Ok(ShotExportReport {
            dest: out,
            files: chosen.len() as u64,
            bytes,
        })
    }

    /// 截图与存档快照的对应：拍摄之后最早的一个快照（“这次存档前拍的截图”）；之后还没有快照的为 None。
    /// 只包括相册中的截图（不含回收站）。
    pub fn shot_snapshots(&self, game: &GameRecord) -> Result<HashMap<ObjectId, Option<ObjectId>>> {
        let mut snaps: Vec<(i64, ObjectId)> = self
            .index
            .snapshots_of(&game.id)?
            .into_iter()
            .filter_map(|r| Some((parse_ts(&r.created_at)?, r.id)))
            .collect();
        snaps.sort();
        Ok(self
            .index
            .shots_of(&game.id)?
            .into_iter()
            .filter(|s| !s.purged && s.deleted_at.is_none())
            .map(|s| {
                let t = parse_ts(&s.taken_at).unwrap_or(i64::MAX);
                let i = snaps.partition_point(|(st, _)| *st < t);
                (s.blob, snaps.get(i).map(|(_, id)| *id))
            })
            .collect())
    }

    // ---------- 不在游戏库中的游戏 ----------

    /// 有截图、但还不在游戏库中（或已移出游戏库）的游戏：Steam 截图按 AppID，
    /// 游戏栏截图按窗口标题（与游戏库中游戏名对应的除外）。
    pub fn shot_candidates(&self) -> Result<Vec<ShotCandidate>> {
        let games = self.games()?;
        let mut out = Vec::new();
        if let Some(root) = self.steam_root() {
            let have: HashSet<&str> = games
                .iter()
                .filter(|g| g.platform == "steam")
                .filter_map(|g| g.app_id.as_deref())
                .collect();
            let apps = src::steam_shot_apps(&root);
            if apps.iter().any(|(id, _, _)| !have.contains(id.as_str())) {
                let installed: HashMap<String, String> = self
                    .steam_adapter()
                    .map(|s| {
                        s.list_games()
                            .into_iter()
                            .map(|g| (g.app_id, g.name))
                            .collect()
                    })
                    .unwrap_or_default();
                let rules = self.load_rules().ok().map(|(r, _)| r);
                for (id, count, dir) in apps {
                    if have.contains(id.as_str()) {
                        continue;
                    }
                    let name = installed
                        .get(&id)
                        .cloned()
                        .or_else(|| {
                            let n: u64 = id.parse().ok()?;
                            rules.as_ref()?.by_steam_id(n).map(|(r, _)| r.name.clone())
                        })
                        .or_else(|| {
                            self.metadata(&format!("steam-{id}"))
                                .ok()
                                .flatten()
                                .and_then(|m| m.data.name)
                        })
                        .unwrap_or_else(|| format!("Steam {id}"));
                    out.push(ShotCandidate {
                        key: format!("steam:{id}"),
                        name,
                        source: "steam".into(),
                        app_id: Some(id),
                        count,
                        dir,
                    });
                }
            }
        }
        if let Some(cap) = self.captures_dir().filter(|d| d.is_dir()) {
            let mut names = Vec::new();
            for g in &games {
                names.extend(self.shot_names(g)?);
            }
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            for (title, count) in src::capture_titles(&cap, &refs) {
                out.push(ShotCandidate {
                    key: format!("capture:{title}"),
                    name: title,
                    source: "capture".into(),
                    app_id: None,
                    count,
                    dir: cap.clone(),
                });
            }
        }
        let shared = self.shared_dirs();
        if !shared.is_empty() {
            let mut names = Vec::new();
            for g in &games {
                names.extend(self.shot_names(g)?);
            }
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            for d in shared {
                for (title, count) in src::shared_titles(&d, &refs) {
                    // 与游戏栏截图同名的合并为一个候选。
                    if let Some(c) = out
                        .iter_mut()
                        .find(|c| c.app_id.is_none() && c.name.eq_ignore_ascii_case(&title))
                    {
                        c.count += count;
                        continue;
                    }
                    out.push(ShotCandidate {
                        key: format!("shared:{title}"),
                        name: title,
                        source: "shared".into(),
                        app_id: None,
                        count,
                        dir: d.clone(),
                    });
                }
            }
        }
        out.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
        Ok(out)
    }

    /// 把游戏加入游戏库，只管理截图：`steam:<AppID>` 为 Steam 游戏（之前移出过的重新加入），
    /// 其他为手动游戏（`name` 与游戏栏截图的窗口标题对应）；`dirs` 为另外的截图文件夹。
    pub fn add_shots_game(
        &mut self,
        _lock: &RepoLock,
        key: Option<&str>,
        name: &str,
        dirs: &[PathBuf],
    ) -> Result<GameRecord> {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::Invalid("游戏名称不能为空".into()));
        }
        let mut checked = Vec::new();
        for d in dirs {
            checked.push(self.check_shot_dir(d)?.0);
        }
        let app_id = key.and_then(|k| k.strip_prefix("steam:")).map(String::from);
        let record = match &app_id {
            Some(id) => {
                let gid = format!("steam-{id}");
                let mut g = self.index.game(&gid)?.unwrap_or(GameRecord {
                    id: gid,
                    name: name.to_string(),
                    platform: "steam".into(),
                    app_id: Some(id.clone()),
                    install_dir: None,
                    enabled: true,
                    include_config: None,
                    custom_rule_json: None,
                });
                g.enabled = true;
                g
            }
            None => {
                let seed = format!(
                    "{name}\0{}\0{}",
                    bonfire_core::scan::now_ns(),
                    std::process::id()
                );
                GameRecord {
                    id: format!("custom-{}", ObjectId::of(seed.as_bytes()).short(12)),
                    name: name.to_string(),
                    platform: "custom".into(),
                    app_id: None,
                    install_dir: None,
                    enabled: true,
                    include_config: None,
                    custom_rule_json: None,
                }
            }
        };
        self.index.upsert_game(&record)?;
        let mut s = self.index.shot_settings(&record.id)?;
        s.shots_only = true;
        s.enabled = self.enabled_on();
        for d in checked {
            let key = crate::transfer::path_key(Path::new(&d));
            if !s
                .dirs
                .iter()
                .any(|x| crate::transfer::path_key(Path::new(x)) == key)
            {
                s.dirs.push(d);
            }
        }
        self.index.save_shot_settings(&record.id, &s)?;
        self.save_games_file()?;
        Ok(record)
    }

    // ---------- 相册文件（事实来源） ----------

    fn album_path(&self, game_id: &str) -> PathBuf {
        self.repo
            .root()
            .join("albums")
            .join(format!("{game_id}.jsonl"))
    }

    /// 把索引中的相册整体写入 `albums/<游戏>.jsonl`（先写临时文件再替换）。
    pub(crate) fn save_album(&self, game_id: &str) -> Result<()> {
        let path = self.album_path(game_id);
        let dir = path.parent().expect("相册路径有上级目录");
        std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, e))?;
        let mut shots = self.index.shots_of(game_id)?;
        shots.sort_by(|a, b| a.taken_at.cmp(&b.taken_at).then(a.blob.cmp(&b.blob)));
        let mut text = String::new();
        for s in &shots {
            text.push_str(&serde_json::to_string(s).expect("截图记录可以序列化"));
            text.push('\n');
        }
        let tmp = path.with_extension("jsonl.tmp");
        std::fs::write(&tmp, text).map_err(|e| AppError::io(&tmp, e))?;
        std::fs::rename(&tmp, &path).map_err(|e| AppError::io(&path, e))
    }

    /// 从相册文件重建截图索引（重建索引时）。原图已不在的记录跳过。返回恢复的截图数。
    pub(crate) fn reindex_albums(&mut self) -> Result<usize> {
        let dir = self.repo.root().join("albums");
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return Ok(0);
        };
        let mut total = 0;
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "jsonl") {
                continue;
            }
            let game = p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let text = std::fs::read_to_string(&p).map_err(|e| AppError::io(&p, e))?;
            let shot_dir = self.shot_dir(&game);
            let shots: Vec<ShotRecord> = text
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str::<ShotRecord>(l).ok())
                .filter(|s| {
                    s.purged
                        || (!s.file.is_empty() && shot_dir.join(&s.file).is_file())
                        || self.repo.has(Kind::Blob, &s.blob)
                })
                .map(|s| ShotRecord {
                    game_id: game.clone(),
                    ..s
                })
                .collect();
            total += shots.len();
            self.index.replace_shots(&game, &shots)?;
        }
        Ok(total)
    }

    /// 定时收集截图：上次收集的时间记录在缓存目录中。
    fn shots_marker(&self) -> PathBuf {
        self.paths.cache_dir.join("shots").join("last-sync")
    }

    pub fn last_shots_sync(&self) -> Option<jiff::Timestamp> {
        std::fs::read_to_string(self.shots_marker())
            .ok()
            .and_then(|t| t.trim().parse().ok())
    }

    fn mark_shots_synced(&self) -> Result<()> {
        let p = self.shots_marker();
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, e))?;
        }
        std::fs::write(&p, jiff::Timestamp::now().to_string()).map_err(|e| AppError::io(&p, e))
    }

    /// 现在是否该定时收集截图。
    pub fn shots_sync_due(&self) -> bool {
        shots_due(
            &self.config.shots,
            self.last_shots_sync(),
            &jiff::Zoned::now(),
        )
    }

    /// 游戏的截图设置（写入 games.toml 用）。
    pub(crate) fn shot_settings_of(&self, game_id: &str) -> Result<ShotSettings> {
        Ok(self.index.shot_settings(game_id)?)
    }
}

/// 定时收集是否到期：`last` 为上次收集时间（规则见 [`crate::auto::schedule_due`]）；
/// 开启“截图变化后自动收集”时，距上次收集满 `check_minutes` 分钟也到期。
pub fn shots_due(cfg: &Shots, last: Option<jiff::Timestamp>, now: &jiff::Zoned) -> bool {
    if !cfg.enabled {
        return false;
    }
    let changed = cfg.on_change
        && last.is_none_or(|t| {
            now.timestamp().as_second() - t.as_second() >= i64::from(cfg.check_minutes.max(1)) * 60
        });
    changed
        || crate::auto::schedule_due(
            cfg.schedule,
            &cfg.at,
            cfg.weekday,
            cfg.every_hours,
            last,
            now,
        )
}

/// 拍摄时间：文件名中的时间（本地时间），否则用文件修改时间。
fn taken_at(f: &ShotFile) -> String {
    let tz = jiff::tz::TimeZone::system();
    let zoned = f
        .taken_local
        .and_then(|(y, mo, d, h, mi, s)| {
            jiff::civil::date(y, mo, d)
                .at(h, mi, s, 0)
                .to_zoned(tz.clone())
                .ok()
        })
        .or_else(|| {
            jiff::Timestamp::from_nanosecond(i128::from(f.mtime_ns))
                .ok()
                .map(|t| t.to_zoned(tz.clone()))
        });
    match zoned {
        Some(z) => z.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        None => now_local_rfc3339(),
    }
}

fn parse_ts(s: &str) -> Option<i64> {
    s.parse::<jiff::Timestamp>().ok().map(|t| t.as_second())
}

/// 备份中原图的文件名：原文件名（去掉不能用的字符），没有时用内容哈希。
fn shot_file_name(original: &Path, blob: &ObjectId) -> String {
    let stem = original
        .file_stem()
        .map(|x| sanitize(&x.to_string_lossy()))
        .filter(|x| !x.is_empty())
        .unwrap_or_else(|| blob.short(12));
    let ext = original
        .extension()
        .map(|x| sanitize(&x.to_string_lossy()).to_lowercase())
        .filter(|x| !x.is_empty())
        .unwrap_or_else(|| "png".into());
    format!("{stem}.{ext}")
}

/// `dir` 下尚不存在的文件名：`a.png`、`a (2).png`……
fn free_name(dir: &Path, name: &str) -> PathBuf {
    let p = Path::new(name);
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = p
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let mut cand = dir.join(name);
    let mut i = 2;
    while cand.exists() || with_suffix(&cand, ".tmp").exists() {
        cand = dir.join(format!("{stem} ({i}){ext}"));
        i += 1;
    }
    cand
}

/// 相对游戏截图目录的路径（`a.png` 或 `.trash/a.png`）。
fn rel_name(sub: &str, dest: &Path) -> String {
    let n = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if sub.is_empty() {
        n
    } else {
        format!("{sub}/{n}")
    }
}

/// 导出时的文件名：原文件名，重名时加序号。
fn file_names(shots: &[&ShotRecord]) -> Vec<String> {
    let mut used: HashSet<String> = HashSet::new();
    shots
        .iter()
        .map(|s| {
            let p = Path::new(&s.original_path);
            let stem = p
                .file_stem()
                .map(|x| sanitize(&x.to_string_lossy()))
                .filter(|x| !x.is_empty())
                .unwrap_or_else(|| s.blob.short(12));
            let ext = p
                .extension()
                .map(|x| x.to_string_lossy().into_owned())
                .unwrap_or_else(|| "png".into());
            let mut name = format!("{stem}.{ext}");
            let mut i = 2;
            while !used.insert(name.to_lowercase()) {
                name = format!("{stem} ({i}).{ext}");
                i += 1;
            }
            name
        })
        .collect()
}

fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .trim_end_matches('.')
        .to_string()
}

/// 展开注册表值中的 `%USERPROFILE%` 等环境变量。
fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('%') {
            Some(j) => {
                let name = &after[..j];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[j + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Env;

    fn png(seed: u8) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(40, 30, image::Rgb([seed, 80, 10]))
            .write_to(&mut buf, image::ImageFormat::Png)
            .unwrap();
        buf.into_inner()
    }

    fn put(p: &Path, data: &[u8]) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    }

    /// 伪造的 Steam 截图与“捕获”文件夹（都在测试的伪造根目录下）。
    fn steam_game(env: &mut Env) -> GameRecord {
        let root = env.app.fake_root.clone().unwrap();
        let shots = root.join("Steam/userdata/111/760/remote/367520/screenshots");
        put(&shots.join("20261009123456_1.jpg"), &png(1));
        put(&shots.join("20261009133000_1.jpg"), &png(2));
        put(&root.join("Steam/steamapps/.keep"), b"");
        let cap = env
            .app
            .known_dirs
            .home
            .clone()
            .unwrap()
            .join("Videos/Captures");
        put(&cap.join("Hollow Knight 2026_10_9 21_05_00.png"), &png(3));
        put(&cap.join("Other Game 2026_10_9 21_05_00.png"), &png(4));
        let g = GameRecord {
            id: "steam-367520".into(),
            name: "Hollow Knight".into(),
            platform: "steam".into(),
            app_id: Some("367520".into()),
            install_dir: None,
            enabled: true,
            include_config: None,
            custom_rule_json: None,
        };
        env.app.index.upsert_game(&g).unwrap();
        g
    }

    #[test]
    fn sync_collects_once_and_keeps_shots_deleted_from_disk() {
        let mut env = Env::new();
        let g = steam_game(&mut env);
        assert_eq!(env.app.pending_shots(&g).unwrap(), 3);
        let lock = env.app.repo.lock().unwrap();
        let r = env.app.sync_shots(&lock, &g).unwrap();
        assert_eq!(r.added, 3);
        assert_eq!(env.app.pending_shots(&g).unwrap(), 0);
        assert_eq!(env.app.sync_shots(&lock, &g).unwrap().added, 0);

        let album = env.app.album(&g).unwrap();
        assert_eq!(album.shots.len(), 3);
        assert_eq!(album.sources, ["steam", "capture"]);
        let steam = album
            .shots
            .iter()
            .find(|s| s.original_path.contains("20261009123456"))
            .unwrap();
        assert!(
            steam.taken_at.starts_with("2026-10-09T12:34:56"),
            "{}",
            steam.taken_at
        );
        assert_eq!((steam.width, steam.height), (Some(40), Some(30)));
        // 原图原样存为普通文件（文件名用原文件名），不进对象库。
        assert_eq!(steam.file, "20261009123456_1.jpg");
        let stored = album.dir.join(&steam.file);
        assert_eq!(std::fs::read(&stored).unwrap(), png(1));
        assert!(!env.app.repo.has(Kind::Blob, &steam.blob));

        // 磁盘上删掉的截图，备份中仍保留；内容相同的新文件不重复收集。
        let root = env.app.fake_root.clone().unwrap();
        let shots = root.join("Steam/userdata/111/760/remote/367520/screenshots");
        std::fs::remove_file(shots.join("20261009123456_1.jpg")).unwrap();
        put(&shots.join("20261010000000_1.jpg"), &png(2));
        assert_eq!(env.app.sync_shots(&lock, &g).unwrap().added, 0);
        assert_eq!(env.app.album(&g).unwrap().shots.len(), 3);

        // 缩略图由备份中的原图生成；大图就是备份中的原图。
        let thumb = env.app.shot_thumb(&g.id, &steam.blob).unwrap();
        assert!(thumb.is_file());
        assert_eq!(env.app.shot_image(&g.id, &steam.blob).unwrap(), stored);
    }

    #[test]
    fn trash_restore_purge_and_reindex() {
        let mut env = Env::new();
        let g = steam_game(&mut env);
        let lock = env.app.repo.lock().unwrap();
        env.app.sync_shots(&lock, &g).unwrap();
        let first = env.app.album(&g).unwrap().shots[0].clone();
        let dir = env.app.shot_dir(&g.id);

        // 删除：进入回收站，原图移到 .trash；磁盘上的原图不动，也不会被再次收集。
        assert_eq!(env.app.delete_shots(&lock, &g, &[first.blob]).unwrap(), 1);
        assert!(
            Path::new(&first.original_path).exists(),
            "不删除磁盘上的原图"
        );
        let album = env.app.album(&g).unwrap();
        assert_eq!((album.shots.len(), album.trash.len()), (2, 1));
        assert!(album.trash[0].file.starts_with(".trash/"));
        assert!(dir.join(&album.trash[0].file).is_file());
        assert!(!dir.join(&first.file).exists());
        assert_eq!(env.app.pending_shots(&g).unwrap(), 0);

        // 恢复。
        assert_eq!(env.app.restore_shots(&lock, &g, &[first.blob]).unwrap(), 1);
        let album = env.app.album(&g).unwrap();
        assert_eq!((album.shots.len(), album.trash.len()), (3, 0));
        let back = album.shots.iter().find(|s| s.blob == first.blob).unwrap();
        assert!(dir.join(&back.file).is_file());

        // 清空回收站：原图删除，只留记录；不再收集。
        env.app.delete_shots(&lock, &g, &[first.blob]).unwrap();
        assert_eq!(env.app.purge_shots(&lock, &g, &[]).unwrap(), 1);
        let album = env.app.album(&g).unwrap();
        assert_eq!((album.shots.len(), album.trash.len()), (2, 0));
        assert!(!dir.join(TRASH_DIR).read_dir().unwrap().any(|_| true));
        assert_eq!(env.app.sync_shots(&lock, &g).unwrap().added, 0);
        assert_eq!(env.app.pending_shots(&g).unwrap(), 0);

        // 重建索引后一致。
        env.app.reindex(&lock).unwrap();
        let album = env.app.album(&g).unwrap();
        assert_eq!((album.shots.len(), album.trash.len()), (2, 0));
        assert_eq!(env.app.pending_shots(&g).unwrap(), 0);
    }

    #[test]
    fn purge_can_also_delete_originals_on_disk() {
        let mut env = Env::new();
        let g = steam_game(&mut env);
        let lock = env.app.repo.lock().unwrap();
        env.app.sync_shots(&lock, &g).unwrap();
        let shots = env.app.album(&g).unwrap().shots;
        let (a, b) = (shots[0].clone(), shots[1].clone());
        // b 的原图内容被改过：不删除。
        std::fs::write(&b.original_path, b"edited").unwrap();
        let r = env
            .app
            .purge_shots_with(&lock, &g, &[a.blob, b.blob], true)
            .unwrap();
        assert_eq!((r.purged, r.originals_deleted, r.originals_kept), (2, 1, 1));
        assert!(!Path::new(&a.original_path).exists());
        assert_eq!(std::fs::read(&b.original_path).unwrap(), b"edited");
        // 不选时磁盘上的原图保留。
        let c = env.app.album(&g).unwrap().shots[0].clone();
        let r = env
            .app
            .purge_shots_with(&lock, &g, &[c.blob], false)
            .unwrap();
        assert_eq!((r.purged, r.originals_deleted), (1, 0));
        assert!(Path::new(&c.original_path).exists());
    }

    #[test]
    fn old_shots_in_object_store_are_migrated_to_files() {
        let mut env = Env::new();
        let g = steam_game(&mut env);
        let data = png(7);
        let stats = env
            .app
            .repo
            .write_file(&data[..], Path::new("x.png"))
            .unwrap();
        let rec = ShotRecord {
            game_id: g.id.clone(),
            blob: stats.blob,
            taken_at: "2026-10-01T10:00:00+00:00".into(),
            source: "steam".into(),
            original_path: "C:/old/20261001100000_1.jpg".into(),
            size: data.len() as u64,
            width: None,
            height: None,
            added_at: now_local_rfc3339(),
            file: String::new(),
            deleted_at: None,
            purged: false,
        };
        env.app.index.add_shot(&rec).unwrap();
        // 早期版本的记录：缩略图、大图照常可用。
        assert!(env.app.shot_thumb(&g.id, &rec.blob).is_ok());
        assert_eq!(env.app.migrate_shot_files().unwrap(), 1);
        let s = env.app.album(&g).unwrap().shots[0].clone();
        assert_eq!(s.file, "20261001100000_1.jpg");
        assert_eq!(
            std::fs::read(env.app.shot_dir(&g.id).join(&s.file)).unwrap(),
            data
        );
        assert_eq!(env.app.migrate_shot_files().unwrap(), 0);
    }

    #[test]
    fn candidates_and_shots_only_games() {
        let mut env = Env::new();
        let root = env.app.fake_root.clone().unwrap();
        put(
            &root.join("Steam/userdata/111/760/remote/367520/screenshots/20261009123456_1.jpg"),
            &png(1),
        );
        put(
            &root.join("Steam/userdata/111/760/remote/4242/screenshots/20261009123456_1.jpg"),
            &png(2),
        );
        put(&root.join("Steam/steamapps/.keep"), b"");
        let cap = env
            .app
            .known_dirs
            .home
            .clone()
            .unwrap()
            .join("Videos/Captures");
        put(&cap.join("Minishoot 2026_7_18 0_02_36.png"), &png(3));
        put(&cap.join("Minishoot 2026_7_18 0_05_00.png"), &png(4));
        let c = env.app.shot_candidates().unwrap();
        let keys: Vec<&str> = c.iter().map(|x| x.key.as_str()).collect();
        assert_eq!(
            keys,
            ["capture:Minishoot", "steam:367520", "steam:4242"],
            "{c:?}"
        );
        assert_eq!(c[0].count, 2);

        // 加入为只管理截图的游戏：不再是候选；状态为只管理截图，不能备份存档。
        let lock = env.app.repo.lock().unwrap();
        let g = env
            .app
            .add_shots_game(&lock, Some("capture:Minishoot"), "Minishoot", &[])
            .unwrap();
        let st = env.app.status(&g).unwrap();
        assert_eq!(st.state, crate::backup::GameState::ShotsOnly);
        assert!(
            env.app
                .backup(&lock, &g, &crate::BackupOptions::default())
                .is_err()
        );
        assert_eq!(env.app.sync_shots(&lock, &g).unwrap().added, 2);
        let s = env
            .app
            .add_shots_game(&lock, Some("steam:4242"), "Steam 4242", &[])
            .unwrap();
        assert_eq!(s.id, "steam-4242");
        let keys: Vec<String> = env
            .app
            .shot_candidates()
            .unwrap()
            .into_iter()
            .map(|x| x.key)
            .collect();
        assert_eq!(keys, ["steam:367520"]);
        // 设置保存在 games.toml 中，重建索引后仍在。
        env.app.reindex(&lock).unwrap();
        assert!(env.app.is_shots_only(&g.id).unwrap());
        env.app.set_shots_only(&g, false).unwrap();
        assert_ne!(
            env.app.status(&g).unwrap().state,
            crate::backup::GameState::ShotsOnly
        );
    }

    #[test]
    fn per_game_switch_dirs_and_export() {
        let mut env = Env::new();
        let g = steam_game(&mut env);
        let own = env.dir.path().join("MyShots");
        put(&own.join("a.png"), &png(9));
        env.app.add_shot_dir(&g, &own).unwrap();
        let repo_root = env.app.repo.root().to_path_buf();
        assert!(env.app.add_shot_dir(&g, &repo_root).is_err());
        assert_eq!(env.app.pending_shots(&g).unwrap(), 4);
        env.app.set_shots_enabled(&g, Some(false)).unwrap();
        let lock = env.app.repo.lock().unwrap();
        assert!(env.app.sync_shots(&lock, &g).unwrap().disabled);
        env.app.set_shots_enabled(&g, None).unwrap();
        assert_eq!(env.app.sync_shots(&lock, &g).unwrap().added, 4);
        assert!(env.app.remove_shot_dir(&g, &own.to_string_lossy()).unwrap());

        let out = env.dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        let r = env
            .app
            .export_shots(&g, &[], &out.join("shots.zip"), ExportFormat::Zip)
            .unwrap();
        assert_eq!(r.files, 4);
        let r = env
            .app
            .export_shots(&g, &[], &out, ExportFormat::Folder)
            .unwrap();
        assert!(r.dest.join("a.png").is_file());
        // 不能导出到截图来源以外的受保护位置：仓库中。
        assert!(
            env.app
                .export_shots(
                    &g,
                    &[],
                    &env.app.repo.root().join("x.zip"),
                    ExportFormat::Zip
                )
                .is_err()
        );
    }

    #[test]
    fn shots_map_to_the_next_snapshot() {
        let mut env = Env::new();
        env.write("a.sav", b"1");
        let g = env.add_game("Game");
        let lock = env.app.repo.lock().unwrap();
        env.app
            .backup(&lock, &g, &crate::BackupOptions::default())
            .unwrap();
        let head = env.app.repo.head_snapshot(&g.id).unwrap().unwrap();
        let before = ShotRecord {
            game_id: g.id.clone(),
            blob: ObjectId::of(b"before"),
            taken_at: "2000-01-01T00:00:00+00:00".into(),
            source: "folder".into(),
            original_path: "x.png".into(),
            size: 1,
            width: None,
            height: None,
            added_at: now_local_rfc3339(),
            file: String::new(),
            deleted_at: None,
            purged: false,
        };
        let after = ShotRecord {
            blob: ObjectId::of(b"after"),
            taken_at: "2100-01-01T00:00:00+00:00".into(),
            ..before.clone()
        };
        env.app.index.add_shot(&before).unwrap();
        env.app.index.add_shot(&after).unwrap();
        let m = env.app.shot_snapshots(&g).unwrap();
        assert_eq!(m[&before.blob], Some(head));
        assert_eq!(m[&after.blob], None);
    }

    #[test]
    fn schedule() {
        let tz = jiff::tz::TimeZone::UTC;
        let at = |s: &str| {
            s.parse::<jiff::civil::DateTime>()
                .unwrap()
                .to_zoned(tz.clone())
                .unwrap()
        };
        let ts = |s: &str| at(s).timestamp();
        let mut c = Shots::default();
        assert!(!shots_due(&c, None, &at("2026-10-09T22:00")));
        // 截图变化后自动收集：满检查间隔才到期。
        c.on_change = true;
        c.check_minutes = 10;
        assert!(shots_due(&c, None, &at("2026-10-09T22:00")));
        assert!(!shots_due(
            &c,
            Some(ts("2026-10-09T21:55")),
            &at("2026-10-09T22:00")
        ));
        assert!(shots_due(
            &c,
            Some(ts("2026-10-09T21:50")),
            &at("2026-10-09T22:00")
        ));
        c.enabled = false;
        assert!(!shots_due(&c, None, &at("2026-10-09T22:00")));
        c.enabled = true;
        c.on_change = false;
        c.schedule = ShotSchedule::Daily;
        c.at = "21:00".into();
        assert!(shots_due(
            &c,
            Some(ts("2026-10-09T08:00")),
            &at("2026-10-09T22:00")
        ));
        assert!(!shots_due(
            &c,
            Some(ts("2026-10-09T21:30")),
            &at("2026-10-09T22:00")
        ));
        assert!(!shots_due(
            &c,
            Some(ts("2026-10-08T22:00")),
            &at("2026-10-09T20:00")
        ));
        // 2026-10-09 是周五；每周日 21:00。
        c.schedule = ShotSchedule::Weekly;
        c.weekday = 7;
        assert!(!shots_due(
            &c,
            Some(ts("2026-10-04T21:30")),
            &at("2026-10-09T22:00")
        ));
        assert!(shots_due(
            &c,
            Some(ts("2026-10-04T20:00")),
            &at("2026-10-09T22:00")
        ));
        c.weekday = 5;
        assert!(shots_due(
            &c,
            Some(ts("2026-10-08T22:00")),
            &at("2026-10-09T22:00")
        ));
        c.schedule = ShotSchedule::Hours;
        c.every_hours = 6;
        assert!(!shots_due(
            &c,
            Some(ts("2026-10-09T17:00")),
            &at("2026-10-09T22:00")
        ));
        assert!(shots_due(
            &c,
            Some(ts("2026-10-09T15:00")),
            &at("2026-10-09T22:00")
        ));
        assert!(shots_due(&c, None, &at("2026-10-09T22:00")));
        c.enabled = false;
        assert!(!shots_due(&c, None, &at("2026-10-09T22:00")));
    }

    #[test]
    fn env_vars_expand() {
        // SAFETY: 测试中只读取已有的环境变量。
        let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME"));
        if let Ok(h) = home {
            let key = if std::env::var("USERPROFILE").is_ok() {
                "USERPROFILE"
            } else {
                "HOME"
            };
            assert_eq!(
                expand_env(&format!("%{key}%\\Videos")),
                format!("{h}\\Videos")
            );
        }
        assert_eq!(expand_env("%NO_SUCH_VAR_X%\\a"), "%NO_SUCH_VAR_X%\\a");
    }
}
