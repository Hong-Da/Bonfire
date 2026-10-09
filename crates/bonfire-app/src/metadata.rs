//! 游戏信息服务（文档 4.3 MetadataService）：按优先级链式调用各提供者，合并结果并记录来源；
//! 图片统一转为 WebP 缓存到 `cache/images/<游戏>/`；获取失败或离线不阻塞扫描与备份。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bonfire_core::index::{GameMetadata, GameRecord};
use bonfire_discovery::http::Http;
use bonfire_discovery::metadata::{self, Candidate, Igdb, ImageSource, MetaPatch, MetaQuery, MetadataProvider, SteamGridDb, SteamLocal, SteamStore};
use image::GenericImageView;
use serde::Serialize;

use crate::app::App;
use crate::error::{AppError, Result};

/// 联网信息的有效期（文档 4.3：文字信息有效期 30 天）。
pub const TTL_DAYS: i64 = 30;

/// 可手动修改并加锁的字段。
pub const EDITABLE_FIELDS: [&str; 6] = ["name", "nameZh", "developer", "publisher", "releaseDate", "cover"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Img {
    Cover,
    Hero,
    Icon,
}

impl Img {
    fn file(self) -> &'static str {
        match self {
            Img::Cover => "cover",
            Img::Hero => "hero",
            Img::Icon => "icon",
        }
    }

    /// 缓存的最大尺寸（宽, 高）。
    fn max(self) -> (u32, u32) {
        match self {
            Img::Cover => (600, 900),
            Img::Hero => (1920, 620),
            Img::Icon => (256, 256),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataView {
    #[serde(flatten)]
    pub data: GameMetadata,
    /// 封面缩略图。
    pub cover_thumb_path: Option<String>,
}

impl App {
    fn steam_adapter(&self) -> Option<bonfire_discovery::platforms::SteamAdapter> {
        match &self.fake_root {
            Some(root) => {
                let s = root.join("Steam");
                s.is_dir().then_some(bonfire_discovery::platforms::SteamAdapter { root: s })
            }
            None => bonfire_discovery::platforms::SteamAdapter::detect(),
        }
    }

    /// 按优先级排列的提供者；关闭联网（或 `online` 为假）时只有本地提供者。
    fn providers(&self, online: bool) -> Vec<Box<dyn MetadataProvider>> {
        let mut v: Vec<Box<dyn MetadataProvider>> = Vec::new();
        if let Some(steam) = self.steam_adapter() {
            v.push(Box::new(SteamLocal { steam }));
        }
        if online && self.config.metadata.online {
            let http: Arc<dyn Http> = self.http.clone();
            v.push(Box::new(SteamStore::new(http.clone())));
            let m = &self.config.metadata;
            if !m.igdb_client_id.is_empty() && !m.igdb_client_secret.is_empty() {
                v.push(Box::new(Igdb::new(http.clone(), &m.igdb_client_id, &m.igdb_client_secret)));
            }
            if !m.steamgriddb_api_key.is_empty() {
                v.push(Box::new(SteamGridDb::new(http, &m.steamgriddb_api_key)));
            }
        }
        v
    }

    pub fn metadata(&self, game_id: &str) -> Result<Option<MetadataView>> {
        Ok(self.index.metadata(game_id)?.map(|data| {
            let cover_thumb_path = data.cover_path.as_ref().map(|p| thumb_path(Path::new(p)).to_string_lossy().into_owned()).filter(|p| Path::new(p).is_file());
            MetadataView { data, cover_thumb_path }
        }))
    }

    fn query(&self, game: &GameRecord, m: &GameMetadata) -> MetaQuery {
        let mut external_ids = m.external_ids.clone();
        if game.platform == "steam"
            && let Some(id) = &game.app_id
        {
            external_ids.entry("steam".into()).or_insert_with(|| id.clone());
        }
        MetaQuery { platform: game.platform.clone(), app_id: game.app_id.clone(), name: game.name.clone(), external_ids }
    }

    /// 刷新游戏信息。`force` 时忽略有效期；否则联网信息在有效期内只刷新本地字段（游玩时长等）。
    pub fn refresh_metadata(&mut self, game: &GameRecord, force: bool) -> Result<GameMetadata> {
        self.refresh_metadata_with(game, force, true)
    }

    /// 只用本地数据（Steam 本地缓存的封面、游玩时长等）补全，不联网：扫描页展示未加入的游戏时使用。
    pub fn refresh_metadata_offline(&mut self, game: &GameRecord) -> Result<GameMetadata> {
        self.refresh_metadata_with(game, false, false)
    }

    fn refresh_metadata_with(&mut self, game: &GameRecord, force: bool, online: bool) -> Result<GameMetadata> {
        let mut m = self.index.metadata(&game.id)?.unwrap_or_else(|| GameMetadata { game_id: game.id.clone(), ..Default::default() });
        let expired = force || m.fetched_at.as_deref().is_none_or(|t| age_days(t).is_none_or(|d| d >= TTL_DAYS));

        // 没有平台 ID 的游戏：先搜索，候选唯一且名称一致时自动关联（文档 4.3 匹配策略）。
        let providers = self.providers(online);
        let mut query = self.query(game, &m);
        if expired && query.steam_id().is_none() && !query.external_ids.contains_key("igdb") && !query.external_ids.contains_key("sgdb") {
            let candidates = search_all(&providers, &game.name);
            if let Some(c) = metadata::auto_pick(&metadata::search_name(&game.name), &candidates) {
                m.external_ids.insert(c.source.clone(), c.id.clone());
                query = self.query(game, &m);
            }
        }

        let mut fetched_online = false;
        for p in providers.iter().filter(|p| p.supports(&query) && (expired || !p.online())) {
            match p.fetch(&query) {
                Ok(patch) => {
                    fetched_online |= p.online();
                    self.merge(game, &mut m, p.id(), patch);
                }
                Err(e) => tracing::info!(game = %game.id, provider = p.id(), "获取游戏信息失败（不影响备份）：{e}"),
            }
        }
        // 只用本地数据时不记获取时间，之后加入游戏库会再联网补全。
        if fetched_online || (online && expired && providers.iter().all(|p| !p.online())) {
            m.fetched_at = Some(jiff::Timestamp::now().to_string());
        }
        self.index.save_metadata(&m)?;
        Ok(m)
    }

    /// 合并一个提供者的结果：已有值的字段不被低优先级来源覆盖，锁定字段不变；本地字段每次覆盖。
    fn merge(&self, game: &GameRecord, m: &mut GameMetadata, source: &str, p: MetaPatch) {
        let locked = |f: &str| m.locked_fields.iter().any(|x| x == f);
        let set_text = |field: &str, slot: &mut Option<String>, value: Option<String>, sources: &mut std::collections::BTreeMap<String, String>| {
            if let Some(v) = value
                && !locked(field)
                && (slot.is_none() || sources.get(field).is_some_and(|s| s == source))
            {
                *slot = Some(v);
                sources.insert(field.to_string(), source.to_string());
            }
        };
        let mut sources = std::mem::take(&mut m.field_sources);
        set_text("name", &mut m.name, p.name, &mut sources);
        set_text("nameZh", &mut m.name_zh, p.name_zh, &mut sources);
        set_text("developer", &mut m.developer, p.developer, &mut sources);
        set_text("publisher", &mut m.publisher, p.publisher, &mut sources);
        set_text("releaseDate", &mut m.release_date, p.release_date, &mut sources);
        if let Some(g) = p.genres
            && !g.is_empty()
            && (m.genres.is_empty() || sources.get("genres").is_some_and(|s| s == source))
        {
            m.genres = g;
            sources.insert("genres".into(), source.into());
        }
        // 本地字段：每次扫描重读。
        if p.playtime_min.is_some() {
            m.playtime_min = p.playtime_min;
        }
        if p.last_played.is_some() {
            m.last_played_at = p.last_played;
        }
        if p.cloud_save.is_some() {
            m.cloud_save = p.cloud_save;
        }
        for (k, v) in p.external_ids {
            m.external_ids.entry(k).or_insert(v);
        }
        for (kind, src, slot) in [(Img::Cover, p.cover, &mut m.cover_path), (Img::Hero, p.hero, &mut m.hero_path), (Img::Icon, p.icon, &mut m.icon_path)] {
            let field = kind.file();
            let has = slot.as_ref().is_some_and(|s| Path::new(s).is_file());
            if let Some(src) = src
                && !locked(field)
                && !has
            {
                match self.cache_image(&game.id, kind, &src) {
                    Ok(path) => {
                        *slot = Some(path.to_string_lossy().into_owned());
                        sources.insert(field.into(), source.into());
                    }
                    Err(e) => tracing::info!(game = %game.id, "图片获取失败：{e}"),
                }
            }
        }
        m.field_sources = sources;
    }

    fn images_dir(&self, game_id: &str) -> PathBuf {
        self.paths.cache_dir.join("images").join(game_id)
    }

    /// 读取图片（本地文件或联网），缩放到上限后保存为 WebP，并为封面生成缩略图。
    fn cache_image(&self, game_id: &str, kind: Img, src: &ImageSource) -> Result<PathBuf> {
        let bytes = match src {
            ImageSource::File(p) => std::fs::read(p).map_err(|e| AppError::io(p, e))?,
            ImageSource::Url(u) => {
                let r = self.http.get(u, &[]).map_err(AppError::Invalid)?;
                if r.status != 200 {
                    return Err(AppError::Invalid(format!("HTTP {}：{u}", r.status)));
                }
                r.body
            }
        };
        let dest = self.images_dir(game_id).join(format!("{}.webp", kind.file()));
        save_webp(&bytes, kind.max(), &dest, kind == Img::Cover)?;
        Ok(dest)
    }

    /// 手动修改字段并加锁（文档 4.3：手动修改的字段加锁，自动刷新不覆盖）。
    pub fn set_metadata_field(&mut self, game: &GameRecord, field: &str, value: &str) -> Result<GameMetadata> {
        let mut m = self.index.metadata(&game.id)?.unwrap_or_else(|| GameMetadata { game_id: game.id.clone(), ..Default::default() });
        let v = Some(value.trim().to_string()).filter(|s| !s.is_empty());
        match field {
            "name" => m.name = v,
            "nameZh" => m.name_zh = v,
            "developer" => m.developer = v,
            "publisher" => m.publisher = v,
            "releaseDate" => m.release_date = v,
            "cover" => {
                let src = PathBuf::from(value);
                if !src.is_file() {
                    return Err(AppError::PathNotFound(src));
                }
                m.cover_path = Some(self.cache_image(&game.id, Img::Cover, &ImageSource::File(src))?.to_string_lossy().into_owned());
            }
            _ => return Err(AppError::Invalid(format!("不可修改的字段 {field}（可选：{}）", EDITABLE_FIELDS.join("、")))),
        }
        if !m.locked_fields.iter().any(|f| f == field) {
            m.locked_fields.push(field.to_string());
        }
        m.field_sources.insert(field.to_string(), "user".into());
        self.index.save_metadata(&m)?;
        Ok(m)
    }

    pub fn unlock_field(&mut self, game: &GameRecord, field: &str) -> Result<GameMetadata> {
        let mut m = self.index.metadata(&game.id)?.unwrap_or_else(|| GameMetadata { game_id: game.id.clone(), ..Default::default() });
        m.locked_fields.retain(|f| f != field);
        self.index.save_metadata(&m)?;
        Ok(m)
    }

    /// 无平台 ID 的游戏：列出候选供用户选择。
    pub fn search_metadata(&self, game: &GameRecord) -> Vec<Candidate> {
        search_all(&self.providers(true), &game.name)
    }

    /// 选定候选：写入外部 ID 映射后刷新（之后不再重复询问）。
    pub fn pick_metadata(&mut self, game: &GameRecord, source: &str, id: &str) -> Result<GameMetadata> {
        let mut m = self.index.metadata(&game.id)?.unwrap_or_else(|| GameMetadata { game_id: game.id.clone(), ..Default::default() });
        m.external_ids.insert(source.to_string(), id.to_string());
        self.index.save_metadata(&m)?;
        self.refresh_metadata(game, true)
    }

    /// 快照时记录的游戏时长。
    pub(crate) fn playtime_of(&self, game_id: &str) -> Option<u64> {
        self.index.metadata(game_id).ok().flatten().and_then(|m| m.playtime_min)
    }
}

fn search_all(providers: &[Box<dyn MetadataProvider>], name: &str) -> Vec<Candidate> {
    let q = metadata::search_name(name);
    let mut out = Vec::new();
    for p in providers {
        match p.search(&q) {
            Ok(c) => out.extend(c),
            Err(e) => tracing::info!(provider = p.id(), "搜索失败：{e}"),
        }
    }
    out
}

fn age_days(rfc3339: &str) -> Option<i64> {
    let t: jiff::Timestamp = rfc3339.parse().ok()?;
    Some((jiff::Timestamp::now().as_second() - t.as_second()) / 86_400)
}

pub fn thumb_path(p: &Path) -> PathBuf {
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    p.with_file_name(format!("{stem}_thumb.webp"))
}

/// 解码图片，缩放到不超过上限，保存为无损 WebP（文档 4.3）；`thumb` 时另存 200 px 宽的缩略图。
pub fn save_webp(bytes: &[u8], max: (u32, u32), dest: &Path, thumb: bool) -> Result<()> {
    let img = image::load_from_memory(bytes).map_err(|e| AppError::Invalid(format!("无法解码图片：{e}")))?;
    let (w, h) = img.dimensions();
    let img = if w > max.0 || h > max.1 { img.resize(max.0, max.1, image::imageops::FilterType::Lanczos3) } else { img };
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, e))?;
    }
    let tmp = dest.with_extension("webp.tmp");
    img.to_rgba8().save_with_format(&tmp, image::ImageFormat::WebP).map_err(|e| AppError::Invalid(format!("保存图片失败：{e}")))?;
    std::fs::rename(&tmp, dest).map_err(|e| AppError::io(dest, e))?;
    if thumb {
        let t = img.thumbnail(200, 300);
        t.to_rgba8().save_with_format(thumb_path(dest), image::ImageFormat::WebP).map_err(|e| AppError::Invalid(format!("保存缩略图失败：{e}")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Env;
    use bonfire_discovery::http::FakeHttp;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_pixel(w, h, image::Rgb([200, 80, 10])).write_to(&mut buf, image::ImageFormat::Png).unwrap();
        buf.into_inner()
    }

    fn steam_game(env: &mut Env) -> GameRecord {
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

    fn store_http() -> FakeHttp {
        let body = r#"{"367520":{"success":true,"data":{"name":"空洞骑士","developers":["Team Cherry"],"publishers":["Team Cherry"],"release_date":{"date":"2017-02-24"},"genres":[{"description":"动作"}]}}}"#;
        FakeHttp::default()
            .route("https://store.steampowered.com/api/appdetails", 200, body)
            .route("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/367520/library_600x900", 200, png(1200, 1800))
            .route("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/367520/library_hero", 404, "")
    }

    #[test]
    fn fetches_merges_caches_images_and_respects_ttl() {
        let mut env = Env::new();
        let http = Arc::new(store_http());
        env.app.http = http.clone();
        let g = steam_game(&mut env);
        let m = env.app.refresh_metadata(&g, false).unwrap();
        assert_eq!(m.name_zh.as_deref(), Some("空洞骑士"));
        assert_eq!(m.developer.as_deref(), Some("Team Cherry"));
        assert_eq!(m.field_sources["developer"], "steam-store");
        let cover = PathBuf::from(m.cover_path.clone().unwrap());
        assert!(cover.ends_with("cover.webp"));
        assert_eq!(image::image_dimensions(&cover).unwrap(), (600, 900), "缩放到上限");
        assert!(thumb_path(&cover).is_file());
        assert!(m.hero_path.is_none(), "404 不阻塞");
        assert!(m.fetched_at.is_some());

        // 有效期内不再联网。
        let before = http.request_count();
        env.app.refresh_metadata(&g, false).unwrap();
        assert_eq!(http.request_count(), before);
        env.app.refresh_metadata(&g, true).unwrap();
        assert!(http.request_count() > before, "强制刷新");
    }

    #[test]
    fn locked_fields_survive_refresh() {
        let mut env = Env::new();
        env.app.http = Arc::new(store_http());
        let g = steam_game(&mut env);
        env.app.set_metadata_field(&g, "nameZh", "空洞骑士（我的）").unwrap();
        let m = env.app.refresh_metadata(&g, true).unwrap();
        assert_eq!(m.name_zh.as_deref(), Some("空洞骑士（我的）"));
        assert_eq!(m.field_sources["nameZh"], "user");
        let m = env.app.unlock_field(&g, "nameZh").unwrap();
        assert!(m.locked_fields.is_empty());
        assert!(matches!(env.app.set_metadata_field(&g, "playtime", "1"), Err(AppError::Invalid(_))));
    }

    #[test]
    fn offline_or_disabled_never_blocks() {
        let mut env = Env::new();
        env.app.http = Arc::new(bonfire_discovery::http::Offline);
        let g = steam_game(&mut env);
        let m = env.app.refresh_metadata(&g, false).unwrap();
        assert!(m.name_zh.is_none());
        env.app.config.metadata.online = false;
        env.app.http = Arc::new(store_http());
        let m = env.app.refresh_metadata(&g, true).unwrap();
        assert!(m.developer.is_none(), "关闭联网后不请求");
    }

    #[test]
    fn manual_cover_is_converted_and_locked() {
        let mut env = Env::new();
        let g = steam_game(&mut env);
        let src = env.dir.path().join("my-cover.png");
        std::fs::write(&src, png(300, 450)).unwrap();
        let m = env.app.set_metadata_field(&g, "cover", &src.to_string_lossy()).unwrap();
        assert!(m.cover_path.unwrap().ends_with("cover.webp"));
        assert_eq!(m.locked_fields, ["cover"]);
    }

    #[test]
    fn custom_game_auto_links_unique_candidate() {
        let mut env = Env::new();
        env.app.config.metadata.steamgriddb_api_key = "k".into();
        env.app.http = Arc::new(
            FakeHttp::default()
                .route("https://www.steamgriddb.com/api/v2/search/autocomplete/Celeste", 200, r#"{"data":[{"id":77,"name":"Celeste"},{"id":78,"name":"Celeste Classic"}]}"#)
                .route("https://www.steamgriddb.com/api/v2/grids/game/77", 200, r#"{"data":[]}"#),
        );
        let lock = env.app.repo.lock().unwrap();
        std::fs::create_dir_all(env.saves()).unwrap();
        let g = env.app.add_game(&lock, &crate::NewGame { name: "Celeste v1.4 (GOG)".into(), save_paths: vec![env.saves()], ..Default::default() }).unwrap();
        drop(lock);
        let m = env.app.refresh_metadata(&g, false).unwrap();
        assert_eq!(m.external_ids.get("sgdb").map(String::as_str), Some("77"));
        assert_eq!(env.app.search_metadata(&g).len(), 2);
    }
}
