//! 游戏信息提供者（文档 4.3）：本地数据优先、联网数据补充；只发送游戏 ID 或名称。
//!
//! | 提供者 | 字段 |
//! |---|---|
//! | steam-local | 游玩时长、最后游玩、云存档、本地 librarycache 图片 |
//! | steam-store | 中文名、开发商、发行商、发行日期、类型、封面/横幅（CDN） |
//! | igdb | 名称、别名、开发商、发行商、日期、类型、封面（需要 Twitch 凭证） |
//! | sgdb | 封面、横幅、图标（需要 API Key） |

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::http::Http;
use crate::manifest::normalize;
use crate::platforms::{PlatformAdapter, SteamAdapter};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "value")]
pub enum ImageSource {
    File(PathBuf),
    Url(String),
}

/// 一个提供者给出的部分信息。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaPatch {
    pub name: Option<String>,
    pub name_zh: Option<String>,
    pub developer: Option<String>,
    pub publisher: Option<String>,
    pub release_date: Option<String>,
    pub genres: Option<Vec<String>>,
    pub cover: Option<ImageSource>,
    pub hero: Option<ImageSource>,
    pub icon: Option<ImageSource>,
    pub playtime_min: Option<u64>,
    pub last_played: Option<i64>,
    pub cloud_save: Option<bool>,
    /// 新发现的外部 ID，如 `igdb` → `1234`。
    pub external_ids: BTreeMap<String, String>,
}

/// 查询条件。
#[derive(Debug, Clone, Default)]
pub struct MetaQuery {
    pub platform: String,
    pub app_id: Option<String>,
    pub name: String,
    /// 已知的外部 ID：`steam`、`igdb`、`sgdb` …
    pub external_ids: BTreeMap<String, String>,
}

impl MetaQuery {
    pub fn steam_id(&self) -> Option<&str> {
        self.external_ids.get("steam").map(String::as_str).or(if self.platform == "steam" { self.app_id.as_deref() } else { None })
    }
}

/// 搜索候选（无平台 ID 的游戏由用户选择）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub source: String,
    pub id: String,
    pub name: String,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
}

pub trait MetadataProvider: Send + Sync {
    fn id(&self) -> &'static str;
    /// 是否联网（关闭联网时跳过）。
    fn online(&self) -> bool;
    fn supports(&self, q: &MetaQuery) -> bool;
    fn fetch(&self, q: &MetaQuery) -> Result<MetaPatch, String>;
    fn search(&self, _query: &str) -> Result<Vec<Candidate>, String> {
        Ok(Vec::new())
    }
}

fn json(resp: crate::http::HttpResponse) -> Result<Value, String> {
    if resp.status != 200 {
        return Err(format!("HTTP {}", resp.status));
    }
    serde_json::from_slice(&resp.body).map_err(|e| e.to_string())
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---------- Steam 本地 ----------

pub struct SteamLocal {
    pub steam: SteamAdapter,
}

impl SteamLocal {
    /// 按图片宽高比分类：竖版为封面，很宽的为横幅，接近正方形的小图为图标。
    fn classify(paths: &[PathBuf]) -> (Option<PathBuf>, Option<PathBuf>, Option<PathBuf>) {
        // 每类保存 (路径, 面积)。
        type Best = Option<(PathBuf, u32)>;
        let (mut cover, mut hero, mut icon): (Best, Best, Best) = (None, None, None);
        for p in paths {
            let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
            if name.contains("logo") {
                continue;
            }
            let Ok((w, h)) = image::image_dimensions(p) else { continue };
            if w == 0 || h == 0 {
                continue;
            }
            let ratio = w as f32 / h as f32;
            let slot = if ratio < 0.8 {
                &mut cover
            } else if ratio > 1.8 {
                &mut hero
            } else if w <= 256 {
                &mut icon
            } else {
                continue;
            };
            // 同类取面积最大的。
            if slot.as_ref().is_none_or(|(_, area)| w * h > *area) {
                *slot = Some((p.clone(), w * h));
            }
        }
        (cover.map(|c| c.0), hero.map(|c| c.0), icon.map(|c| c.0))
    }
}

impl MetadataProvider for SteamLocal {
    fn id(&self) -> &'static str {
        "steam-local"
    }

    fn online(&self) -> bool {
        false
    }

    fn supports(&self, q: &MetaQuery) -> bool {
        q.steam_id().is_some()
    }

    fn fetch(&self, q: &MetaQuery) -> Result<MetaPatch, String> {
        let id = q.steam_id().ok_or("没有 Steam ID")?;
        let game = self.steam.list_games().into_iter().find(|g| g.app_id == id);
        let (cover, hero, icon) = Self::classify(&self.steam.cached_images(id));
        Ok(MetaPatch {
            name: game.as_ref().map(|g| g.name.clone()),
            playtime_min: game.as_ref().and_then(|g| g.playtime_min),
            last_played: game.as_ref().and_then(|g| g.last_played),
            cloud_save: game.as_ref().map(|g| g.cloud_save),
            cover: cover.map(ImageSource::File),
            hero: hero.map(ImageSource::File),
            icon: icon.map(ImageSource::File),
            ..Default::default()
        })
    }
}

// ---------- Steam Store API ----------

pub struct SteamStore {
    pub http: Arc<dyn Http>,
    /// 默认 `https://store.steampowered.com`。
    pub base: String,
    pub cdn: String,
}

impl SteamStore {
    pub fn new(http: Arc<dyn Http>) -> Self {
        Self { http, base: "https://store.steampowered.com".into(), cdn: "https://shared.akamai.steamstatic.com/store_item_assets/steam/apps".into() }
    }
}

impl MetadataProvider for SteamStore {
    fn id(&self) -> &'static str {
        "steam-store"
    }

    fn online(&self) -> bool {
        true
    }

    fn supports(&self, q: &MetaQuery) -> bool {
        q.steam_id().is_some()
    }

    fn fetch(&self, q: &MetaQuery) -> Result<MetaPatch, String> {
        let id = q.steam_id().ok_or("没有 Steam ID")?;
        let v = json(self.http.get(&format!("{}/api/appdetails?appids={id}&l=schinese", self.base), &[])?)?;
        let entry = v.get(id).ok_or("响应中没有该游戏")?;
        if entry.get("success").and_then(Value::as_bool) != Some(true) {
            return Err("Steam 商店没有该游戏".into());
        }
        let d = &entry["data"];
        let s = |k: &str| d.get(k).and_then(Value::as_str).filter(|x| !x.is_empty()).map(String::from);
        let first = |k: &str| d.get(k).and_then(Value::as_array).and_then(|a| a.first()).and_then(Value::as_str).map(String::from);
        let name_zh = s("name").filter(|n| *n != q.name && n.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)));
        Ok(MetaPatch {
            name_zh,
            developer: first("developers"),
            publisher: first("publishers"),
            release_date: d.get("release_date").and_then(|r| r.get("date")).and_then(Value::as_str).filter(|x| !x.is_empty()).map(String::from),
            genres: d.get("genres").and_then(Value::as_array).map(|a| a.iter().filter_map(|g| g.get("description").and_then(Value::as_str).map(String::from)).collect()),
            cover: Some(ImageSource::Url(format!("{}/{id}/library_600x900_2x.jpg", self.cdn))),
            hero: Some(ImageSource::Url(format!("{}/{id}/library_hero.jpg", self.cdn))),
            icon: None,
            ..Default::default()
        })
    }
}

// ---------- IGDB ----------

pub struct Igdb {
    pub http: Arc<dyn Http>,
    pub client_id: String,
    pub client_secret: String,
    pub api: String,
    pub auth: String,
    token: Mutex<Option<String>>,
}

impl Igdb {
    pub fn new(http: Arc<dyn Http>, client_id: &str, client_secret: &str) -> Self {
        Self {
            http,
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            api: "https://api.igdb.com/v4".into(),
            auth: "https://id.twitch.tv/oauth2/token".into(),
            token: Mutex::new(None),
        }
    }

    fn token(&self) -> Result<String, String> {
        if let Some(t) = self.token.lock().unwrap().clone() {
            return Ok(t);
        }
        let url = format!("{}?client_id={}&client_secret={}&grant_type=client_credentials", self.auth, url_encode(&self.client_id), url_encode(&self.client_secret));
        let v = json(self.http.post(&url, &[], "")?)?;
        let t = v.get("access_token").and_then(Value::as_str).ok_or("Twitch 未返回令牌")?.to_string();
        *self.token.lock().unwrap() = Some(t.clone());
        Ok(t)
    }

    fn query(&self, body: &str) -> Result<Vec<Value>, String> {
        let token = self.token()?;
        let auth = format!("Bearer {token}");
        let v = json(self.http.post(&format!("{}/games", self.api), &[("Client-ID", &self.client_id), ("Authorization", &auth)], body)?)?;
        Ok(v.as_array().cloned().unwrap_or_default())
    }

    const FIELDS: &'static str = "fields name,first_release_date,cover.image_id,genres.name,involved_companies.company.name,involved_companies.developer,involved_companies.publisher,alternative_names.name;";

    fn to_patch(g: &Value) -> MetaPatch {
        let companies = g.get("involved_companies").and_then(Value::as_array).cloned().unwrap_or_default();
        let company = |role: &str| {
            companies.iter().find(|c| c.get(role).and_then(Value::as_bool) == Some(true)).and_then(|c| c["company"]["name"].as_str()).map(String::from)
        };
        let zh = g.get("alternative_names").and_then(Value::as_array).and_then(|a| {
            a.iter().filter_map(|n| n.get("name").and_then(Value::as_str)).find(|n| n.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c))).map(String::from)
        });
        let date = g.get("first_release_date").and_then(Value::as_i64).and_then(jiff_date);
        let mut ids = BTreeMap::new();
        if let Some(id) = g.get("id").and_then(Value::as_i64) {
            ids.insert("igdb".to_string(), id.to_string());
        }
        MetaPatch {
            name: g.get("name").and_then(Value::as_str).map(String::from),
            name_zh: zh,
            developer: company("developer"),
            publisher: company("publisher"),
            release_date: date,
            genres: g.get("genres").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x["name"].as_str().map(String::from)).collect()),
            cover: g["cover"]["image_id"].as_str().map(|i| ImageSource::Url(format!("https://images.igdb.com/igdb/image/upload/t_cover_big/{i}.jpg"))),
            external_ids: ids,
            ..Default::default()
        }
    }
}

/// Unix 秒 → `YYYY-MM-DD`（UTC）。
fn jiff_date(t: i64) -> Option<String> {
    let days = t.div_euclid(86_400);
    // 公历日期换算（Howard Hinnant 的算法），避免引入时间库。
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    Some(format!("{:04}-{m:02}-{d:02}", if m <= 2 { y + 1 } else { y }))
}

impl MetadataProvider for Igdb {
    fn id(&self) -> &'static str {
        "igdb"
    }

    fn online(&self) -> bool {
        true
    }

    fn supports(&self, q: &MetaQuery) -> bool {
        q.external_ids.contains_key("igdb")
    }

    fn fetch(&self, q: &MetaQuery) -> Result<MetaPatch, String> {
        let id = q.external_ids.get("igdb").ok_or("没有 IGDB ID")?;
        let id: u64 = id.parse().map_err(|_| "IGDB ID 无效")?;
        let list = self.query(&format!("{} where id = {id};", Self::FIELDS))?;
        list.first().map(Self::to_patch).ok_or_else(|| "IGDB 没有该游戏".into())
    }

    fn search(&self, query: &str) -> Result<Vec<Candidate>, String> {
        let q = query.replace('"', "");
        let list = self.query(&format!("search \"{q}\"; {} limit 8;", Self::FIELDS))?;
        Ok(list
            .iter()
            .filter_map(|g| {
                let p = Self::to_patch(g);
                Some(Candidate {
                    source: "igdb".into(),
                    id: p.external_ids.get("igdb")?.clone(),
                    name: p.name?,
                    year: p.release_date.and_then(|d| d.get(..4).and_then(|y| y.parse().ok())),
                    cover_url: match p.cover {
                        Some(ImageSource::Url(u)) => Some(u),
                        _ => None,
                    },
                })
            })
            .collect())
    }
}

// ---------- SteamGridDB ----------

pub struct SteamGridDb {
    pub http: Arc<dyn Http>,
    pub key: String,
    pub api: String,
}

impl SteamGridDb {
    pub fn new(http: Arc<dyn Http>, key: &str) -> Self {
        Self { http, key: key.into(), api: "https://www.steamgriddb.com/api/v2".into() }
    }

    fn get(&self, path: &str) -> Result<Value, String> {
        let auth = format!("Bearer {}", self.key);
        json(self.http.get(&format!("{}{path}", self.api), &[("Authorization", &auth)])?)
    }

    fn first_url(&self, path: &str) -> Option<String> {
        let v = self.get(path).ok()?;
        v["data"].as_array()?.first()?.get("url")?.as_str().map(String::from)
    }
}

impl MetadataProvider for SteamGridDb {
    fn id(&self) -> &'static str {
        "sgdb"
    }

    fn online(&self) -> bool {
        true
    }

    fn supports(&self, q: &MetaQuery) -> bool {
        q.steam_id().is_some() || q.external_ids.contains_key("sgdb")
    }

    fn fetch(&self, q: &MetaQuery) -> Result<MetaPatch, String> {
        let key = match (q.steam_id(), q.external_ids.get("sgdb")) {
            (Some(s), _) => format!("steam/{s}"),
            (None, Some(g)) => format!("game/{g}"),
            _ => return Err("没有可用的 ID".into()),
        };
        Ok(MetaPatch {
            cover: self.first_url(&format!("/grids/{key}?dimensions=600x900")).map(ImageSource::Url),
            hero: self.first_url(&format!("/heroes/{key}")).map(ImageSource::Url),
            icon: self.first_url(&format!("/icons/{key}")).map(ImageSource::Url),
            ..Default::default()
        })
    }

    fn search(&self, query: &str) -> Result<Vec<Candidate>, String> {
        let v = self.get(&format!("/search/autocomplete/{}", url_encode(query)))?;
        Ok(v["data"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|g| {
                        Some(Candidate {
                            source: "sgdb".into(),
                            id: g.get("id")?.as_i64()?.to_string(),
                            name: g.get("name")?.as_str()?.to_string(),
                            year: g.get("release_date").and_then(Value::as_i64).and_then(jiff_date).and_then(|d| d[..4].parse().ok()),
                            cover_url: None,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default())
    }
}

/// 名称查找候选：去掉版本号与符号后搜索（文档 4.3 匹配策略第 2 条）。
pub fn search_name(name: &str) -> String {
    // 去掉括号中的内容，如 (GOG)、[Repack]。
    let mut no_brackets = String::new();
    let mut depth = 0;
    for c in name.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => no_brackets.push(c),
            _ => {}
        }
    }
    // 版本号：v1、v1.4、1.2.3；标题中的普通数字（如 Witcher 3）保留。
    let is_version = |w: &str| {
        let body = w.strip_prefix(['v', 'V']).unwrap_or(w);
        !body.is_empty() && body.chars().all(|c| c.is_ascii_digit() || c == '.') && (w.len() > body.len() || body.contains('.'))
    };
    let words: Vec<String> = no_brackets
        .split_whitespace()
        .filter(|w| !is_version(w))
        .map(|w| w.chars().filter(|c| c.is_alphanumeric()).collect::<String>())
        .filter(|w| !w.is_empty())
        .collect();
    words.join(" ")
}

/// 候选唯一且名称一致（归一化后相等）时自动采用（文档 4.3 匹配策略第 3 条）。
pub fn auto_pick<'a>(name: &str, candidates: &'a [Candidate]) -> Option<&'a Candidate> {
    let n = normalize(name);
    let exact: Vec<&Candidate> = candidates.iter().filter(|c| normalize(&c.name) == n).collect();
    (exact.len() == 1).then(|| exact[0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::FakeHttp;
    use crate::platforms::tests::fake_steam;

    fn q(platform: &str, id: Option<&str>, name: &str) -> MetaQuery {
        MetaQuery { platform: platform.into(), app_id: id.map(String::from), name: name.into(), external_ids: BTreeMap::new() }
    }

    #[test]
    fn steam_store_parses_appdetails() {
        let body = r#"{"367520":{"success":true,"data":{"name":"空洞骑士","developers":["Team Cherry"],"publishers":["Team Cherry"],"release_date":{"date":"2017 年 2 月 24 日"},"genres":[{"description":"动作"},{"description":"独立"}]}}}"#;
        let http = Arc::new(FakeHttp::default().route("https://store.steampowered.com/api/appdetails?appids=367520", 200, body));
        let p = SteamStore::new(http.clone()).fetch(&q("steam", Some("367520"), "Hollow Knight")).unwrap();
        assert_eq!(p.name_zh.as_deref(), Some("空洞骑士"));
        assert_eq!(p.developer.as_deref(), Some("Team Cherry"));
        assert_eq!(p.genres.unwrap(), ["动作", "独立"]);
        assert!(matches!(p.cover, Some(ImageSource::Url(u)) if u.contains("367520/library_600x900")));
        assert!(http.requests.lock().unwrap()[0].0.contains("l=schinese"), "只发送游戏 ID");
        let fail = SteamStore::new(Arc::new(FakeHttp::default().route("https://store", 200, r#"{"1":{"success":false}}"#)));
        assert!(fail.fetch(&q("steam", Some("1"), "x")).is_err());
        assert!(!SteamStore::new(Arc::new(FakeHttp::default())).supports(&q("custom", None, "x")));
    }

    #[test]
    fn steam_local_reads_playtime_and_images() {
        let d = tempfile::tempdir().unwrap();
        let steam = fake_steam(d.path());
        // 写一张竖版封面与一张横幅。
        let cache = steam.root.join("appcache/librarycache/367520");
        image::RgbImage::new(60, 90).save(cache.join("abc/cover.png")).unwrap();
        image::RgbImage::new(300, 100).save(cache.join("hero.png")).unwrap();
        let p = SteamLocal { steam }.fetch(&q("steam", Some("367520"), "Hollow Knight")).unwrap();
        assert_eq!(p.playtime_min, Some(2520));
        assert_eq!(p.cloud_save, Some(true));
        assert!(matches!(p.cover, Some(ImageSource::File(f)) if f.ends_with("cover.png")));
        assert!(matches!(p.hero, Some(ImageSource::File(f)) if f.ends_with("hero.png")));
    }

    #[test]
    fn igdb_search_and_fetch() {
        let games = r#"[{"id":1942,"name":"The Witcher 3: Wild Hunt","first_release_date":1431993600,"cover":{"image_id":"co1wyy"},"genres":[{"name":"RPG"}],
            "involved_companies":[{"company":{"name":"CD PROJEKT RED"},"developer":true,"publisher":false},{"company":{"name":"CD PROJEKT"},"developer":false,"publisher":true}],
            "alternative_names":[{"name":"巫师3：狂猎"}]}]"#;
        let http = Arc::new(
            FakeHttp::default()
                .route("https://id.twitch.tv/oauth2/token", 200, r#"{"access_token":"tok"}"#)
                .route("https://api.igdb.com/v4/games", 200, games),
        );
        let igdb = Igdb::new(http.clone(), "cid", "secret");
        let c = igdb.search("Witcher 3").unwrap();
        assert_eq!((c[0].id.as_str(), c[0].year), ("1942", Some(2015)));
        let mut query = q("custom", None, "x");
        query.external_ids.insert("igdb".into(), "1942".into());
        let p = igdb.fetch(&query).unwrap();
        assert_eq!(p.developer.as_deref(), Some("CD PROJEKT RED"));
        assert_eq!(p.publisher.as_deref(), Some("CD PROJEKT"));
        assert_eq!(p.name_zh.as_deref(), Some("巫师3：狂猎"));
        assert_eq!(p.release_date.as_deref(), Some("2015-05-19"));
        let reqs = http.requests.lock().unwrap();
        assert_eq!(reqs.iter().filter(|r| r.0.contains("oauth2")).count(), 1, "令牌缓存");
        assert!(reqs.iter().any(|r| r.1.iter().any(|(k, v)| k == "Authorization" && v == "Bearer tok")));
    }

    #[test]
    fn sgdb_images_and_search() {
        let http = Arc::new(
            FakeHttp::default()
                .route("https://www.steamgriddb.com/api/v2/grids/steam/367520", 200, r#"{"data":[{"url":"https://cdn/c.png"}]}"#)
                .route("https://www.steamgriddb.com/api/v2/heroes/steam/367520", 200, r#"{"data":[]}"#)
                .route("https://www.steamgriddb.com/api/v2/search/autocomplete/", 200, r#"{"data":[{"id":5,"name":"Celeste","release_date":1516579200}]}"#),
        );
        let s = SteamGridDb::new(http, "k");
        let p = s.fetch(&q("steam", Some("367520"), "HK")).unwrap();
        assert_eq!(p.cover, Some(ImageSource::Url("https://cdn/c.png".into())));
        assert_eq!(p.hero, None);
        let c = s.search("Celeste v1.4").unwrap();
        assert_eq!((c[0].name.as_str(), c[0].year), ("Celeste", Some(2018)));
    }

    #[test]
    fn name_cleanup_and_auto_pick() {
        assert_eq!(search_name("Celeste v1.4.0.0 (GOG)!"), "Celeste");
        assert_eq!(search_name("The Witcher 3: Wild Hunt 1.32"), "The Witcher 3 Wild Hunt");
        assert_eq!(search_name("Hades™ v2"), "Hades");
        let c = |n: &str, id: &str| Candidate { source: "igdb".into(), id: id.into(), name: n.into(), year: None, cover_url: None };
        let list = vec![c("Celeste", "1"), c("Celeste Classic", "2")];
        assert_eq!(auto_pick("celeste", &list).unwrap().id, "1");
        assert!(auto_pick("Celest", &list).is_none());
        assert!(auto_pick("Celeste", &[c("Celeste", "1"), c("CELESTE", "3")]).is_none(), "多个同名候选时让用户选择");
    }

    #[test]
    fn date_conversion() {
        assert_eq!(jiff_date(0).as_deref(), Some("1970-01-01"));
        assert_eq!(jiff_date(1709251200).as_deref(), Some("2024-03-01"));
    }
}
