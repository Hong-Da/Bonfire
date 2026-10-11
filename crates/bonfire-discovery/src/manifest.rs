//! 存档规则库（文档 4.2）：复用 Ludusavi Manifest（YAML），转换为精简格式内置与缓存。
//!
//! 优先级：用户自定义规则 > 社区规则库 > 启发式探测。
//! 精简格式只保留 Windows 适用的规则：`zstd(JSON)`，文件名 `manifest.json.zst`。

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::http::Http;

pub const MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/mtkennerly/ludusavi-manifest/master/data/manifest.yaml";
pub const COMPACT_FILE: &str = "manifest.json.zst";
const ETAG_FILE: &str = "manifest.etag";
pub const CUSTOM_RULES_FILE: &str = "custom-rules.yaml";

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("规则库格式错误：{0}")]
    Parse(String),
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("下载规则库失败：{0}")]
    Network(String),
}

// ---------- Ludusavi 原始格式（只取需要的字段，其余忽略） ----------

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawGame {
    files: BTreeMap<String, RawRule>,
    registry: BTreeMap<String, RawRule>,
    #[serde(rename = "installDir")]
    install_dir: BTreeMap<String, serde::de::IgnoredAny>,
    steam: Option<RawId>,
    gog: Option<RawId>,
    cloud: Option<BTreeMap<String, bool>>,
    alias: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawRule {
    tags: Vec<String>,
    when: Vec<RawWhen>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawWhen {
    os: Option<String>,
    store: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct RawId {
    id: Option<u64>,
}

// ---------- 精简格式 ----------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathRule {
    pub path: String,
    /// `save` / `config`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// 适用的商店（steam / epic / gog / uplay / origin / microsoft …）；空表示全部。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stores: Vec<String>,
}

impl PathRule {
    pub fn applies_to(&self, store: &str) -> bool {
        self.stores.is_empty() || self.stores.iter().any(|s| s == store)
    }

    /// 只标了 config 的为配置文件，其余按存档处理。
    pub fn is_config_only(&self) -> bool {
        !self.tags.is_empty() && self.tags.iter().all(|t| t == "config")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GameRule {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<PathRule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub registry: Vec<PathRule>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub install_dirs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steam_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gog_id: Option<u64>,
    /// 支持平台云存档的商店。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cloud: Vec<String>,
    /// 本条是另一游戏的别名时，指向正式名称。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub games: Vec<GameRule>,
}

/// Windows 适用：when 为空，或任一条件的 os 为空/windows。返回适用的商店列表（空 = 全部）。
fn windows_stores(when: &[RawWhen]) -> Option<Vec<String>> {
    if when.is_empty() {
        return Some(Vec::new());
    }
    let applicable: Vec<&RawWhen> = when
        .iter()
        .filter(|w| w.os.as_deref().is_none_or(|o| o == "windows"))
        .collect();
    if applicable.is_empty() {
        return None;
    }
    if applicable.iter().any(|w| w.store.is_none()) {
        return Some(Vec::new());
    }
    let mut stores: Vec<String> = applicable.iter().filter_map(|w| w.store.clone()).collect();
    stores.sort();
    stores.dedup();
    Some(stores)
}

fn convert_rules(raw: BTreeMap<String, RawRule>) -> Vec<PathRule> {
    raw.into_iter()
        .filter_map(|(path, r)| {
            windows_stores(&r.when).map(|stores| PathRule {
                path,
                tags: r.tags,
                stores,
            })
        })
        .collect()
}

impl Manifest {
    /// 解析 Ludusavi YAML，只保留 Windows 适用、有存档规则或是别名的条目。
    pub fn from_yaml(text: &str) -> Result<Self, ManifestError> {
        // 完整规则库有数万款游戏、数百万个节点，放宽默认的解析预算（输入来自固定的可信地址）。
        let mut budget = serde_saphyr::Budget::default();
        budget.max_nodes = 50_000_000;
        budget.max_events = 100_000_000;
        let mut opts = serde_saphyr::Options::default();
        opts.budget = Some(budget);
        opts.emit_comments = false;
        opts.with_snippet = false;
        let raw: BTreeMap<String, RawGame> = serde_saphyr::from_str_with_options(text, opts)
            .map_err(|e| ManifestError::Parse(e.to_string()))?;
        let mut games = Vec::new();
        for (name, g) in raw {
            let files = convert_rules(g.files);
            let registry = convert_rules(g.registry);
            if files.is_empty() && registry.is_empty() && g.alias.is_none() {
                continue;
            }
            games.push(GameRule {
                name,
                files,
                registry,
                install_dirs: g.install_dir.into_keys().collect(),
                steam_id: g.steam.and_then(|s| s.id),
                gog_id: g.gog.and_then(|s| s.id),
                cloud: g
                    .cloud
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|(_, v)| *v)
                    .map(|(k, _)| k)
                    .collect(),
                alias: g.alias,
            });
        }
        Ok(Self { games })
    }

    pub fn to_compact(&self) -> Vec<u8> {
        let json = serde_json::to_vec(self).expect("规则库序列化不会失败");
        let mut enc = zstd::stream::Encoder::new(Vec::new(), 19).expect("zstd 初始化");
        enc.write_all(&json).expect("写入内存");
        enc.finish().expect("压缩完成")
    }

    pub fn from_compact(bytes: &[u8]) -> Result<Self, ManifestError> {
        let json =
            zstd::stream::decode_all(bytes).map_err(|e| ManifestError::Parse(e.to_string()))?;
        serde_json::from_slice(&json).map_err(|e| ManifestError::Parse(e.to_string()))
    }

    pub fn load_compact(path: &Path) -> Result<Self, ManifestError> {
        let bytes = std::fs::read(path).map_err(|e| ManifestError::Io {
            path: path.into(),
            source: e,
        })?;
        Self::from_compact(&bytes)
    }
}

/// 规则来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleSource {
    Manifest,
    User,
}

/// 合并用户规则后的索引。
#[derive(Debug, Default)]
pub struct RuleIndex {
    games: Vec<(GameRule, RuleSource)>,
    by_name: HashMap<String, usize>,
    by_steam: HashMap<u64, usize>,
    by_gog: HashMap<u64, usize>,
    by_install_dir: HashMap<String, usize>,
    by_normalized: HashMap<String, usize>,
    /// Microsoft Store 应用包名（规则路径中 `Packages/<包名>/` 的部分，小写）。
    by_package: HashMap<String, usize>,
}

/// 规则路径中 `Packages/<包名>/` 的包名（小写）；包名含 `_`（发布者 ID）。
fn package_of(path: &str) -> Option<String> {
    let lower = path.replace('\\', "/").to_lowercase();
    let rest = lower.split("/packages/").nth(1)?;
    let pkg = rest.split('/').next()?;
    (pkg.contains('_') && !pkg.contains('<') && !pkg.contains('*')).then(|| pkg.to_string())
}

/// 名称归一化：小写，只保留字母与数字（用于模糊匹配）。
pub fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

impl RuleIndex {
    /// 用户规则覆盖同名（不区分大小写）的社区规则。
    pub fn new(manifest: Manifest, user: Manifest) -> Self {
        let mut idx = Self::default();
        for g in manifest.games {
            idx.insert(g, RuleSource::Manifest);
        }
        for g in user.games {
            idx.insert(g, RuleSource::User);
        }
        idx
    }

    fn insert(&mut self, g: GameRule, source: RuleSource) {
        let key = g.name.to_lowercase();
        let i = match self.by_name.get(&key) {
            Some(&i) => {
                self.games[i] = (g, source);
                i
            }
            None => {
                self.games.push((g, source));
                self.games.len() - 1
            }
        };
        self.by_name.insert(key, i);
        let g = &self.games[i].0;
        if let Some(id) = g.steam_id {
            self.by_steam.insert(id, i);
        }
        if let Some(id) = g.gog_id {
            self.by_gog.insert(id, i);
        }
        for d in &g.install_dirs {
            self.by_install_dir.entry(d.to_lowercase()).or_insert(i);
        }
        for r in &g.files {
            if let Some(pkg) = package_of(&r.path) {
                self.by_package.entry(pkg).or_insert(i);
            }
        }
        self.by_normalized.entry(normalize(&g.name)).or_insert(i);
    }

    pub fn len(&self) -> usize {
        self.games.len()
    }

    pub fn is_empty(&self) -> bool {
        self.games.is_empty()
    }

    fn resolve_alias(&self, i: usize) -> Option<(&GameRule, RuleSource)> {
        let (g, s) = &self.games[i];
        match &g.alias {
            Some(target) => self
                .by_name
                .get(&target.to_lowercase())
                .map(|&j| (&self.games[j].0, self.games[j].1)),
            None => Some((g, *s)),
        }
    }

    pub fn by_name(&self, name: &str) -> Option<(&GameRule, RuleSource)> {
        self.by_name
            .get(&name.to_lowercase())
            .and_then(|&i| self.resolve_alias(i))
    }

    pub fn by_steam_id(&self, id: u64) -> Option<(&GameRule, RuleSource)> {
        self.by_steam.get(&id).and_then(|&i| self.resolve_alias(i))
    }

    pub fn by_gog_id(&self, id: u64) -> Option<(&GameRule, RuleSource)> {
        self.by_gog.get(&id).and_then(|&i| self.resolve_alias(i))
    }

    /// 按 Microsoft Store 应用包名（如 `Microsoft.SunriseBaseGame_8wekyb3d8bbwe`）匹配。
    pub fn by_package(&self, pfn: &str) -> Option<(&GameRule, RuleSource)> {
        self.by_package
            .get(&pfn.to_lowercase())
            .and_then(|&i| self.resolve_alias(i))
    }

    pub fn by_install_dir(&self, dir: &str) -> Option<(&GameRule, RuleSource)> {
        self.by_install_dir
            .get(&dir.to_lowercase())
            .and_then(|&i| self.resolve_alias(i))
    }

    /// 按归一化名称匹配（忽略大小写、空格与符号）。
    pub fn by_fuzzy_name(&self, name: &str) -> Option<(&GameRule, RuleSource)> {
        self.by_normalized
            .get(&normalize(name))
            .and_then(|&i| self.resolve_alias(i))
    }
}

/// 规则库文件的位置。
#[derive(Debug, Clone)]
pub struct ManifestPaths {
    /// 下载更新后存放的目录（缓存目录）。
    pub cache_dir: PathBuf,
    /// 安装包内置的快照（可多个候选位置，取第一个存在的）。
    pub bundled: Vec<PathBuf>,
    /// 用户自定义规则文件。
    pub custom_rules: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedFrom {
    pub path: Option<PathBuf>,
    pub games: usize,
    pub user_games: usize,
}

/// 加载规则：缓存中的更新版本优先，其次内置快照；都没有时只有用户规则。
pub fn load(paths: &ManifestPaths) -> Result<(RuleIndex, LoadedFrom), ManifestError> {
    let candidates =
        std::iter::once(paths.cache_dir.join(COMPACT_FILE)).chain(paths.bundled.iter().cloned());
    let mut manifest = Manifest::default();
    let mut from = None;
    for p in candidates {
        if p.is_file() {
            match Manifest::load_compact(&p) {
                Ok(m) => {
                    manifest = m;
                    from = Some(p);
                    break;
                }
                Err(e) => {
                    tracing::warn!(path = %p.display(), "规则库文件无法读取，尝试下一个：{e}")
                }
            }
        }
    }
    let user = load_custom_rules(&paths.custom_rules)?;
    let loaded = LoadedFrom {
        path: from,
        games: manifest.games.len(),
        user_games: user.games.len(),
    };
    Ok((RuleIndex::new(manifest, user), loaded))
}

pub fn load_custom_rules(path: &Path) -> Result<Manifest, ManifestError> {
    match std::fs::read_to_string(path) {
        Ok(t) if t.trim().is_empty() => Ok(Manifest::default()),
        Ok(t) => Manifest::from_yaml(&t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Manifest::default()),
        Err(e) => Err(ManifestError::Io {
            path: path.into(),
            source: e,
        }),
    }
}

fn yaml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// 以 Ludusavi 格式写出用户规则（只含本程序用到的字段）。
pub fn to_yaml(m: &Manifest) -> String {
    let mut out = String::from(
        "# 篝火 Bonfire 用户自定义存档规则（格式同 Ludusavi Manifest），优先于社区规则库\n",
    );
    for g in &m.games {
        out.push_str(&format!("{}:\n", yaml_str(&g.name)));
        for (key, rules) in [("files", &g.files), ("registry", &g.registry)] {
            if rules.is_empty() {
                continue;
            }
            out.push_str(&format!("  {key}:\n"));
            for r in rules {
                out.push_str(&format!("    {}:\n", yaml_str(&r.path)));
                let tags = if r.tags.is_empty() {
                    "save".to_string()
                } else {
                    r.tags.join(", ")
                };
                out.push_str(&format!("      tags: [{tags}]\n"));
                if !r.stores.is_empty() {
                    out.push_str("      when:\n");
                    for s in &r.stores {
                        out.push_str(&format!("        - os: windows\n          store: {s}\n"));
                    }
                }
            }
        }
        if !g.install_dirs.is_empty() {
            out.push_str("  installDir:\n");
            for d in &g.install_dirs {
                out.push_str(&format!("    {}: {{}}\n", yaml_str(d)));
            }
        }
        if let Some(id) = g.steam_id {
            out.push_str(&format!("  steam:\n    id: {id}\n"));
        }
        if let Some(id) = g.gog_id {
            out.push_str(&format!("  gog:\n    id: {id}\n"));
        }
        if let Some(a) = &g.alias {
            out.push_str(&format!("  alias: {}\n", yaml_str(a)));
        }
    }
    out
}

/// 新增或替换用户规则文件中的一个游戏（按名称，不区分大小写）。`files` 为 (路径, 是否配置文件)。
pub fn save_custom_rule(
    path: &Path,
    name: &str,
    files: &[(String, bool)],
) -> Result<(), ManifestError> {
    let mut m = load_custom_rules(path)?;
    let rule = GameRule {
        name: name.to_string(),
        files: files
            .iter()
            .map(|(p, cfg)| PathRule {
                path: p.clone(),
                tags: vec![if *cfg { "config" } else { "save" }.into()],
                stores: Vec::new(),
            })
            .collect(),
        ..Default::default()
    };
    match m
        .games
        .iter_mut()
        .find(|g| g.name.eq_ignore_ascii_case(name))
    {
        Some(g) => *g = rule,
        None => m.games.push(rule),
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| ManifestError::Io {
            path: dir.into(),
            source: e,
        })?;
    }
    std::fs::write(path, to_yaml(&m)).map_err(|e| ManifestError::Io {
        path: path.into(),
        source: e,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "result")]
pub enum UpdateOutcome {
    Updated { games: usize, bytes: u64 },
    NotModified,
}

/// 下载最新规则库并转换到缓存目录。服务器返回 304 时不改动（按 ETag 增量更新）。
pub fn update(
    http: &dyn Http,
    url: &str,
    cache_dir: &Path,
) -> Result<UpdateOutcome, ManifestError> {
    let io = |path: &Path, e| ManifestError::Io {
        path: path.into(),
        source: e,
    };
    let etag_path = cache_dir.join(ETAG_FILE);
    let compact = cache_dir.join(COMPACT_FILE);
    let etag = if compact.is_file() {
        std::fs::read_to_string(&etag_path).ok()
    } else {
        None
    };
    let headers: Vec<(&str, &str)> = etag
        .as_deref()
        .map(|e| vec![("If-None-Match", e.trim())])
        .unwrap_or_default();
    let resp = http.get(url, &headers).map_err(ManifestError::Network)?;
    match resp.status {
        304 => return Ok(UpdateOutcome::NotModified),
        200 => {}
        s => return Err(ManifestError::Network(format!("HTTP {s}"))),
    }
    let text = String::from_utf8(resp.body).map_err(|e| ManifestError::Parse(e.to_string()))?;
    let manifest = Manifest::from_yaml(&text)?;
    let bytes = manifest.to_compact();
    std::fs::create_dir_all(cache_dir).map_err(|e| io(cache_dir, e))?;
    let tmp = compact.with_extension("zst.tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| io(&tmp, e))?;
    std::fs::rename(&tmp, &compact).map_err(|e| io(&compact, e))?;
    match resp
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("etag"))
    {
        Some((_, e)) => std::fs::write(&etag_path, e).map_err(|e| io(&etag_path, e))?,
        None => {
            let _ = std::fs::remove_file(&etag_path);
        }
    }
    Ok(UpdateOutcome::Updated {
        games: manifest.games.len(),
        bytes: bytes.len() as u64,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::http::FakeHttp;

    pub const SAMPLE: &str = r#"
"Hollow Knight":
  files:
    "<winLocalAppDataLow>/Team Cherry/Hollow Knight/*.dat":
      tags: [save]
    "<winLocalAppDataLow>/Team Cherry/Hollow Knight/*.dat.bak":
      tags: [save]
      when:
        - os: linux
  installDir:
    Hollow Knight: {}
  steam:
    id: 367520
  gog:
    id: 1308320804
  cloud:
    steam: true
"Elden Ring":
  files:
    "<winAppData>/EldenRing/<storeUserId>/*.sl2":
      tags: [save]
    "<winAppData>/EldenRing/GraphicsConfig.xml":
      tags: [config]
      when:
        - os: windows
          store: steam
  steam:
    id: 1245620
"Registry Game":
  registry:
    HKEY_CURRENT_USER/Software/BonfireTest/RegistryGame:
      tags: [save]
  installDir:
    RegGame: {}
"Launch Only":
  launch:
    "<base>/game.exe": [{}]
  steam:
    id: 1
"HK Alias":
  alias: Hollow Knight
"#;

    #[test]
    fn converts_yaml_keeping_windows_rules() {
        let m = Manifest::from_yaml(SAMPLE).unwrap();
        let names: Vec<_> = m.games.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(
            names,
            ["Elden Ring", "HK Alias", "Hollow Knight", "Registry Game"],
            "没有存档规则的条目被丢弃"
        );
        let hk = m.games.iter().find(|g| g.name == "Hollow Knight").unwrap();
        assert_eq!(hk.files.len(), 1, "Linux 专用规则被丢弃");
        assert_eq!((hk.steam_id, hk.gog_id), (Some(367520), Some(1308320804)));
        assert_eq!(hk.cloud, ["steam"]);
        let er = m.games.iter().find(|g| g.name == "Elden Ring").unwrap();
        let cfg = er.files.iter().find(|r| r.path.ends_with(".xml")).unwrap();
        assert_eq!(cfg.stores, ["steam"]);
        assert!(cfg.is_config_only());
        assert!(cfg.applies_to("steam") && !cfg.applies_to("epic"));
    }

    #[test]
    fn compact_roundtrip_is_smaller() {
        let m = Manifest::from_yaml(SAMPLE).unwrap();
        let c = m.to_compact();
        assert!(c.len() < SAMPLE.len());
        assert_eq!(Manifest::from_compact(&c).unwrap(), m);
    }

    #[test]
    fn index_lookups_and_user_override() {
        let user =
            Manifest::from_yaml("\"hollow knight\":\n  files:\n    \"D:/MySaves/*.dat\": {}\n")
                .unwrap();
        let idx = RuleIndex::new(Manifest::from_yaml(SAMPLE).unwrap(), user);
        let (g, src) = idx.by_steam_id(1245620).unwrap();
        assert_eq!((g.name.as_str(), src), ("Elden Ring", RuleSource::Manifest));
        let (g, src) = idx.by_name("Hollow Knight").unwrap();
        assert_eq!(src, RuleSource::User, "用户规则覆盖社区规则");
        assert_eq!(g.files[0].path, "D:/MySaves/*.dat");
        assert_eq!(
            idx.by_name("HK Alias").unwrap().0.name,
            "hollow knight",
            "别名指向正式条目"
        );
        assert_eq!(
            idx.by_install_dir("regGAME").unwrap().0.name,
            "Registry Game"
        );
        assert_eq!(
            idx.by_fuzzy_name("ELDEN-RING™").unwrap().0.name,
            "Elden Ring"
        );
        assert!(idx.by_steam_id(1).is_none());
    }

    #[test]
    fn update_downloads_converts_and_uses_etag() {
        let dir = tempfile::tempdir().unwrap();
        let http = FakeHttp::default().route_with_headers(
            "https://example.test/m.yaml",
            200,
            &[("ETag", "\"v1\"")],
            SAMPLE,
        );
        let r = update(&http, "https://example.test/m.yaml", dir.path()).unwrap();
        assert!(
            matches!(r, UpdateOutcome::Updated { games: 4, .. }),
            "{r:?}"
        );
        let paths = ManifestPaths {
            cache_dir: dir.path().into(),
            bundled: vec![],
            custom_rules: dir.path().join("none.yaml"),
        };
        let (idx, from) = load(&paths).unwrap();
        assert_eq!((idx.len(), from.games), (4, 4));

        // 第二次带上 If-None-Match，服务器返回 304。
        let http = FakeHttp::default().route("https://example.test/m.yaml", 304, "");
        assert_eq!(
            update(&http, "https://example.test/m.yaml", dir.path()).unwrap(),
            UpdateOutcome::NotModified
        );
        let reqs = http.requests.lock().unwrap();
        assert_eq!(
            reqs[0].1,
            vec![("If-None-Match".to_string(), "\"v1\"".to_string())]
        );
    }

    #[test]
    fn custom_rules_yaml_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("custom-rules.yaml");
        save_custom_rule(
            &p,
            "独立 \"游戏\"",
            &[("D:/存档/a b".into(), false), ("C:\\cfg.ini".into(), true)],
        )
        .unwrap();
        save_custom_rule(&p, "Other", &[("E:/x".into(), false)]).unwrap();
        save_custom_rule(&p, "other", &[("E:/y".into(), false)]).unwrap();
        let m = load_custom_rules(&p).unwrap();
        assert_eq!(m.games.len(), 2, "同名替换");
        let g = m.games.iter().find(|g| g.name == "独立 \"游戏\"").unwrap();
        assert_eq!(
            g.files.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            ["C:\\cfg.ini", "D:/存档/a b"]
        );
        assert!(g.files[0].is_config_only());
        assert_eq!(
            m.games.iter().find(|g| g.name == "other").unwrap().files[0].path,
            "E:/y"
        );
        // 完整格式也能往返。
        let full = Manifest::from_yaml(SAMPLE).unwrap();
        let mut again = Manifest::from_yaml(&to_yaml(&full)).unwrap();
        for g in &mut again.games {
            g.cloud.clear();
        }
        let mut expect = full.clone();
        for g in &mut expect.games {
            g.cloud.clear();
        }
        assert_eq!(again, expect);
    }

    #[test]
    fn offline_update_fails_but_bundled_snapshot_loads() {
        let dir = tempfile::tempdir().unwrap();
        let bundled = dir.path().join("bundled.json.zst");
        std::fs::write(&bundled, Manifest::from_yaml(SAMPLE).unwrap().to_compact()).unwrap();
        assert!(matches!(
            update(
                &FakeHttp::default(),
                MANIFEST_URL,
                &dir.path().join("cache")
            ),
            Err(ManifestError::Network(_))
        ));
        let paths = ManifestPaths {
            cache_dir: dir.path().join("cache"),
            bundled: vec![dir.path().join("missing"), bundled.clone()],
            custom_rules: dir.path().join("x"),
        };
        let (idx, from) = load(&paths).unwrap();
        assert_eq!(from.path, Some(bundled));
        assert!(idx.by_steam_id(367520).is_some());
    }
}
