//! 截图备份（截图备份方案 S1–S6）：每个游戏一个“相册”，与存档快照分开。
//!
//! - 来源：Steam 截图（按 AppID）、Windows 游戏栏 / Xbox 应用的“捕获”文件夹（按文件名中的标题）、
//!   为游戏手动添加的截图文件夹。来源目录只读，绝不写入。
//! - 原图作为普通文件对象入库（内容寻址去重）；索引表 `screenshots` 加速查询，
//!   仓库中的 `albums/<游戏>.jsonl` 是相册的事实来源，重建索引时据此恢复。
//! - 只增不减：磁盘上删掉的截图，备份中仍保留；可在界面中手动删除备份中的截图。
//! - 收集时机：每次备份时顺带收集；另可按天 / 周 / 小时定时收集（[`shots_due`]）。
//! - 缩略图与查看用的原图放在缓存目录 `shots/<游戏>/`，可随时重建。

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use bonfire_core::index::{GameRecord, ShotRecord, ShotSettings};
use bonfire_core::{ObjectId, RepoLock};
use bonfire_discovery::shots::{self as src, ShotFile};
use serde::Serialize;

use crate::app::App;
use crate::config::{ShotSchedule, Shots};
use crate::error::{AppError, Result};
use crate::transfer::{ExportFormat, replace_file, unique_child, with_suffix, zip_err, zip_time};
use crate::util::{now_compact, now_local_rfc3339};

/// “捕获”文件夹在系统设置中的位置（Known Folder `Captures`）。
const CAPTURES_KEY: &str =
    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders";
const CAPTURES_VALUE: &str = "{EDC0FE71-98D8-4F4A-B920-C8DC133CB165}";
/// 缩略图的最大尺寸。
const THUMB: (u32, u32) = (320, 320);

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
    pub shots: Vec<ShotRecord>,
    pub enabled: bool,
    /// 跟随全局设置（没有为这个游戏单独设置）。
    pub follows_global: bool,
    /// 全局设置是否收集截图。
    pub global_enabled: bool,
    pub dirs: Vec<String>,
    /// 磁盘上还没有收集的截图数。
    pub pending: usize,
    pub bytes: u64,
    /// 本机能找到的来源（用于提示截图从哪里来）。
    pub sources: Vec<String>,
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

    /// Steam 安装目录（伪造根目录下为 `<root>/Steam`）。
    fn steam_root(&self) -> Option<PathBuf> {
        match &self.fake_root {
            Some(root) => Some(root.join("Steam")).filter(|p| p.is_dir()),
            None => bonfire_discovery::platforms::SteamAdapter::detect().map(|s| s.root),
        }
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

    /// 磁盘上属于这个游戏的全部截图（只读）。
    pub fn shot_files(&self, game: &GameRecord) -> Result<Vec<ShotFile>> {
        let mut out = Vec::new();
        if game.platform == "steam"
            && let (Some(app_id), Some(root)) = (&game.app_id, self.steam_root())
        {
            out.extend(src::steam_shots(&root, app_id));
        }
        if let Some(dir) = self.captures_dir() {
            let zh = self.metadata(&game.id)?.and_then(|m| m.data.name_zh);
            let mut names = vec![game.name.as_str()];
            if let Some(z) = zh.as_deref() {
                names.push(z);
            }
            out.extend(src::capture_shots(&dir, &names));
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
        if !self.index.shot_settings(&game.id)?.dirs.is_empty() {
            v.push("folder".to_string());
        }
        Ok(v)
    }

    /// 还没有收集的截图（按原路径与大小判断，不读文件内容）。
    fn uncollected(&self, game: &GameRecord) -> Result<Vec<ShotFile>> {
        let known: HashSet<(String, u64)> = self
            .index
            .shots_of(&game.id)?
            .into_iter()
            .map(|s| (s.original_path, s.size))
            .collect();
        Ok(self
            .shot_files(game)?
            .into_iter()
            .filter(|f| !known.contains(&(f.path.to_string_lossy().into_owned(), f.size)))
            .collect())
    }

    /// 磁盘上还没有收集的截图数（游戏库“检查更新”时提示）；关闭截图备份的游戏为 0。
    pub fn pending_shots(&self, game: &GameRecord) -> Result<usize> {
        if !self.shots_enabled(game)? {
            return Ok(0);
        }
        Ok(self.uncollected(game)?.len())
    }

    /// 收集新截图入库。来源目录只读；内容与已收集的相同时只更新不重复存。
    pub fn sync_shots(&mut self, _lock: &RepoLock, game: &GameRecord) -> Result<ShotSyncReport> {
        let mut report = ShotSyncReport {
            game_id: game.id.clone(),
            ..Default::default()
        };
        if !self.shots_enabled(game)? {
            report.disabled = true;
            return Ok(report);
        }
        let files = self.uncollected(game)?;
        if files.is_empty() {
            return Ok(report);
        }
        let mut changed = false;
        for f in files {
            let file = match std::fs::File::open(&f.path) {
                Ok(h) => h,
                Err(e) => {
                    tracing::warn!(path = %f.path.display(), "读取截图失败，跳过：{e}");
                    continue;
                }
            };
            let stats = self.repo.write_file(&file, &f.path)?;
            // 按内容判断格式（扩展名可能与内容不符）。
            let (width, height) = image::ImageReader::open(&f.path)
                .ok()
                .and_then(|r| r.with_guessed_format().ok())
                .and_then(|r| r.into_dimensions().ok())
                .map(|(w, h)| (Some(w), Some(h)))
                .unwrap_or((None, None));
            let record = ShotRecord {
                game_id: game.id.clone(),
                blob: stats.blob,
                taken_at: taken_at(&f),
                source: f.source.as_str().to_string(),
                original_path: f.path.to_string_lossy().into_owned(),
                size: stats.size,
                width,
                height,
                added_at: now_local_rfc3339(),
            };
            if self.index.add_shot(&record)? {
                report.added += 1;
                report.bytes += stats.size;
                changed = true;
            }
        }
        if changed {
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
        self.mark_shots_synced()?;
        Ok(out)
    }

    pub fn album(&self, game: &GameRecord) -> Result<AlbumView> {
        let settings = self.index.shot_settings(&game.id)?;
        let shots = self.index.shots_of(&game.id)?;
        Ok(AlbumView {
            bytes: shots.iter().map(|s| s.size).sum(),
            shots,
            enabled: self.shots_enabled(game)?,
            follows_global: settings.enabled.is_none(),
            global_enabled: self.config.shots.enabled,
            dirs: settings.dirs,
            pending: self.pending_shots(game)?,
            sources: self.shot_sources(game)?,
        })
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
        let s = abs.to_string_lossy().into_owned();
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

    /// 从相册中删除截图（只删备份，不碰磁盘上的原图；对象由 GC 清理）。
    pub fn delete_shots(
        &mut self,
        _lock: &RepoLock,
        game: &GameRecord,
        blobs: &[ObjectId],
    ) -> Result<usize> {
        let n = self.index.delete_shots(&game.id, blobs)?;
        if n > 0 {
            self.save_album(&game.id)?;
            for b in blobs {
                let _ = std::fs::remove_file(self.shot_thumb_path(&game.id, b));
            }
        }
        Ok(n)
    }

    /// 按前缀（≥ 6 位）找到相册中的截图。
    pub fn resolve_shots(&self, game: &GameRecord, specs: &[String]) -> Result<Vec<ObjectId>> {
        let shots = self.index.shots_of(&game.id)?;
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

    fn shot_cache_dir(&self, game_id: &str) -> PathBuf {
        self.paths.cache_dir.join("shots").join(game_id)
    }

    fn shot_thumb_path(&self, game_id: &str, blob: &ObjectId) -> PathBuf {
        self.shot_cache_dir(game_id)
            .join(format!("{}.webp", blob.to_hex()))
    }

    fn shot_record(&self, game_id: &str, blob: &ObjectId) -> Result<ShotRecord> {
        self.index
            .shots_of(game_id)?
            .into_iter()
            .find(|s| s.blob == *blob)
            .ok_or_else(|| AppError::Invalid(format!("相册中没有截图 {}", blob.short(12))))
    }

    fn shot_bytes(&self, blob: &ObjectId) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        self.repo.read_blob_to(blob, &mut buf, self.repo.root())?;
        Ok(buf)
    }

    /// 缩略图（缓存中没有时由备份中的原图生成）。
    pub fn shot_thumb(&self, game_id: &str, blob: &ObjectId) -> Result<PathBuf> {
        let path = self.shot_thumb_path(game_id, blob);
        if !path.is_file() {
            self.shot_record(game_id, blob)?;
            let bytes = self.shot_bytes(blob)?;
            let img = image::load_from_memory(&bytes)
                .map_err(|e| AppError::Invalid(format!("无法解码截图：{e}")))?
                .thumbnail(THUMB.0, THUMB.1);
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

    /// 查看大图用的原图（从备份中取出到缓存目录）。
    pub fn shot_image(&self, game_id: &str, blob: &ObjectId) -> Result<PathBuf> {
        let rec = self.shot_record(game_id, blob)?;
        let ext = Path::new(&rec.original_path)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_else(|| "png".into());
        let path = self
            .shot_cache_dir(game_id)
            .join("full")
            .join(format!("{}.{ext}", blob.to_hex()));
        if !path.is_file() {
            let bytes = self.shot_bytes(blob)?;
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, e))?;
            }
            let tmp = with_suffix(&path, ".tmp");
            std::fs::write(&tmp, &bytes).map_err(|e| AppError::io(&tmp, e))?;
            std::fs::rename(&tmp, &path).map_err(|e| AppError::io(&path, e))?;
        }
        Ok(path)
    }

    /// 导出截图（`blobs` 为空表示全部）到 zip 或新文件夹，文件名用原文件名（重名加序号）。
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
            .filter(|s| wanted.is_empty() || wanted.contains(&s.blob))
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
                    let data = self.shot_bytes(&s.blob)?;
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
                        let data = self.shot_bytes(&s.blob)?;
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
            .map(|s| {
                let t = parse_ts(&s.taken_at).unwrap_or(i64::MAX);
                let i = snaps.partition_point(|(st, _)| *st < t);
                (s.blob, snaps.get(i).map(|(_, id)| *id))
            })
            .collect())
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

    /// 从相册文件重建截图索引（重建索引时）。返回恢复的截图数。
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
            let shots: Vec<ShotRecord> = text
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str::<ShotRecord>(l).ok())
                .filter(|s| self.repo.has(bonfire_core::Kind::Blob, &s.blob))
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

/// 定时收集是否到期：`last` 为上次收集时间。
/// - 每 N 小时：距上次已满 N 小时（从未收集过时立即收集）；
/// - 每天：今天的设定时刻已过，且上次收集早于这个时刻；
/// - 每周：最近一次“设定的星期 + 时刻”已过，且上次收集早于它。
pub fn shots_due(cfg: &Shots, last: Option<jiff::Timestamp>, now: &jiff::Zoned) -> bool {
    if !cfg.enabled {
        return false;
    }
    let Some((h, m)) = cfg.at_hm() else {
        return false;
    };
    let last_s = last.map(|t| t.as_second()).unwrap_or(i64::MIN);
    match cfg.schedule {
        ShotSchedule::Off => false,
        ShotSchedule::Hours => match last {
            None => true,
            Some(t) => {
                now.timestamp().as_second() - t.as_second() >= i64::from(cfg.every_hours) * 3600
            }
        },
        ShotSchedule::Daily | ShotSchedule::Weekly => {
            let Ok(mut slot) = now.date().at(h, m, 0, 0).to_zoned(now.time_zone().clone()) else {
                return false;
            };
            if cfg.schedule == ShotSchedule::Weekly {
                let want = i16::from(cfg.weekday);
                let today = i16::from(now.date().weekday().to_monday_one_offset());
                let back = (today - want).rem_euclid(7);
                slot = match slot.checked_sub(jiff::Span::new().days(back)) {
                    Ok(s) => s,
                    Err(_) => return false,
                };
            }
            if slot.timestamp() > now.timestamp() {
                // 今天（本周）的时刻还没到：看上一个周期。
                let step = if cfg.schedule == ShotSchedule::Daily {
                    1
                } else {
                    7
                };
                slot = match slot.checked_sub(jiff::Span::new().days(step)) {
                    Ok(s) => s,
                    Err(_) => return false,
                };
            }
            last_s < slot.timestamp().as_second()
        }
    }
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

        // 磁盘上删掉的截图，备份中仍保留；内容相同的新文件不重复收集。
        let root = env.app.fake_root.clone().unwrap();
        let shots = root.join("Steam/userdata/111/760/remote/367520/screenshots");
        std::fs::remove_file(shots.join("20261009123456_1.jpg")).unwrap();
        put(&shots.join("20261010000000_1.jpg"), &png(2));
        assert_eq!(env.app.sync_shots(&lock, &g).unwrap().added, 0);
        assert_eq!(env.app.album(&g).unwrap().shots.len(), 3);

        // 缩略图与原图可以从备份中取出。
        let thumb = env.app.shot_thumb(&g.id, &steam.blob).unwrap();
        assert!(thumb.is_file());
        let full = env.app.shot_image(&g.id, &steam.blob).unwrap();
        assert_eq!(std::fs::read(full).unwrap(), png(1));
    }

    #[test]
    fn album_survives_reindex_and_delete_is_backup_only() {
        let mut env = Env::new();
        let g = steam_game(&mut env);
        let lock = env.app.repo.lock().unwrap();
        env.app.sync_shots(&lock, &g).unwrap();
        let first = env.app.album(&g).unwrap().shots[0].clone();
        assert_eq!(env.app.delete_shots(&lock, &g, &[first.blob]).unwrap(), 1);
        assert!(
            Path::new(&first.original_path).exists(),
            "不删除磁盘上的原图"
        );
        env.app.reindex(&lock).unwrap();
        assert_eq!(env.app.album(&g).unwrap().shots.len(), 2);
        // 删除后磁盘上的原图仍在：会被当作新截图再次提示。
        assert_eq!(env.app.pending_shots(&g).unwrap(), 1);
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
