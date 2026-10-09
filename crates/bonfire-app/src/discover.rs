//! 扫描服务（文档 §3）：调用平台适配器与规则库识别本机游戏及其存档位置；
//! 由用户选择哪些游戏加入游戏库（加入、忽略），已在库中的游戏随扫描更新存档位置。

use std::path::PathBuf;

use bonfire_core::index::{GameRecord, LocationKind, LocationRecord};
use bonfire_core::scan::{RootKind, ScanRoot, collect_files};
use bonfire_core::{FileTag, RepoLock};
use bonfire_discovery::manifest::{self, LoadedFrom, ManifestPaths, RuleIndex, UpdateOutcome};
use bonfire_discovery::platforms::{self, PlatformAdapter};
use bonfire_discovery::resolve::{self, Discovered, FoundKind, LocationSource};
use serde::Serialize;

use crate::app::App;
use crate::error::{AppError, Result};

/// 内置规则库快照的文件名（随安装包放在程序目录或其 resources 子目录中）。
pub const BUNDLED_MANIFEST: &str = manifest::COMPACT_FILE;

/// 测试用：`BONFIRE_FAKE_ROOT` 指向伪造的“本机”，平台目录与系统已知目录都取自其中。
pub const ENV_FAKE_ROOT: &str = "BONFIRE_FAKE_ROOT";

/// 两个目录是否相同（不区分大小写与分隔符，忽略末尾分隔符）。
pub fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
    let abs = |p: &std::path::Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    resolve::path_key(&abs(a).to_string_lossy()) == resolve::path_key(&abs(b).to_string_lossy())
}

pub fn fake_root() -> Option<PathBuf> {
    std::env::var_os(ENV_FAKE_ROOT).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// 伪造根目录下的平台：`<root>/Steam`、`<root>/ProgramData/Epic/EpicGamesLauncher/Data/Manifests`。
pub(crate) fn fake_adapters(root: &std::path::Path, disabled: &[String]) -> Vec<Box<dyn PlatformAdapter>> {
    let on = |id: &str| !disabled.iter().any(|d| d.eq_ignore_ascii_case(id));
    let mut v: Vec<Box<dyn PlatformAdapter>> = Vec::new();
    let steam = root.join("Steam");
    if on("steam") && steam.join("steamapps").is_dir() {
        v.push(Box::new(platforms::SteamAdapter { root: steam }));
    }
    let epic = root.join("ProgramData/Epic/EpicGamesLauncher/Data/Manifests");
    if on("epic") && epic.is_dir() {
        v.push(Box::new(platforms::EpicAdapter { manifests_dir: epic }));
    }
    v
}

/// 扫描结果中游戏的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FoundState {
    /// 新发现，尚未加入游戏库。
    New,
    /// 已在游戏库中（扫描会更新其存档位置）。
    Managed,
    /// 用户选择忽略，之后扫描不再作为新发现提示。
    Ignored,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredGame {
    pub id: String,
    pub name: String,
    pub name_zh: Option<String>,
    pub platform: String,
    /// 命中的规则名称；为空表示启发式推测。
    pub rule: Option<String>,
    pub state: FoundState,
    /// 存档位置（实际路径或通配模式）。
    pub locations: Vec<String>,
    /// 需要确认的启发式位置数。
    pub unconfirmed: usize,
    /// 当前存档的文件数与总大小（帮助判断是否值得管理）。
    pub files: u64,
    pub bytes: u64,
    pub cover_thumb: Option<String>,
    pub cloud_save: bool,
    pub playtime_min: Option<u64>,
    pub last_played: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub platforms: Vec<String>,
    pub rules: Option<LoadedFrom>,
    /// 找到存档的游戏（新发现、已在库中、已忽略）。
    pub games: Vec<DiscoveredGame>,
    /// 已安装但没有找到存档的游戏。
    pub without_saves: Vec<NoSaveGame>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoSaveGame {
    pub id: String,
    pub name: String,
    pub platform: String,
    /// 规则库中有该游戏（只是本机还没有存档，例如没玩过）。
    pub rule: Option<String>,
}

impl App {
    pub fn manifest_paths(&self) -> ManifestPaths {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(PathBuf::from));
        let bundled = exe_dir.map(|d| vec![d.join(BUNDLED_MANIFEST), d.join("resources").join(BUNDLED_MANIFEST)]).unwrap_or_default();
        let config_dir = self.paths.config_file.parent().map(PathBuf::from).unwrap_or_default();
        ManifestPaths { cache_dir: self.paths.cache_dir.join("manifest"), bundled, custom_rules: config_dir.join(manifest::CUSTOM_RULES_FILE) }
    }

    pub fn load_rules(&self) -> Result<(RuleIndex, LoadedFrom)> {
        manifest::load(&self.manifest_paths()).map_err(|e| AppError::Invalid(e.to_string()))
    }

    /// 下载最新规则库到缓存目录。`bundle_to` 指定时同时写出一份内置快照（打包用）。
    pub fn update_rules(&self, url: &str, bundle_to: Option<&std::path::Path>) -> Result<UpdateOutcome> {
        let paths = self.manifest_paths();
        let out = manifest::update(self.http.as_ref(), url, &paths.cache_dir).map_err(|e| match e {
            manifest::ManifestError::Network(m) => AppError::Network(m),
            other => AppError::Invalid(other.to_string()),
        })?;
        if let Some(dest) = bundle_to {
            let src = paths.cache_dir.join(manifest::COMPACT_FILE);
            if let Some(dir) = dest.parent() {
                std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, e))?;
            }
            std::fs::copy(&src, dest).map_err(|e| AppError::io(dest, e))?;
        }
        Ok(out)
    }

    /// 规则库缺失或超过 7 天未更新时联网更新（文档 4.2：启动时增量更新）。
    /// 关闭联网或更新失败时继续使用已有规则，返回是否更新了。
    pub fn ensure_rules(&self) -> bool {
        if !self.config.metadata.online {
            return false;
        }
        let paths = self.manifest_paths();
        let cached = paths.cache_dir.join(manifest::COMPACT_FILE);
        let age_days = std::fs::metadata(&cached)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|d| d.as_secs() / 86_400);
        if age_days.is_some_and(|d| d < 7) {
            return false;
        }
        match self.update_rules(manifest::MANIFEST_URL, None) {
            Ok(r) => {
                tracing::info!(?r, "已更新规则库");
                true
            }
            Err(e) => {
                tracing::info!("规则库更新失败，继续使用已有规则：{e}");
                // 避免离线时每次启动都等待超时：记下这次尝试的时间。
                if cached.is_file() {
                    let _ = std::fs::File::options().write(true).open(&cached).and_then(|f| f.set_modified(std::time::SystemTime::now()));
                }
                false
            }
        }
    }

    /// 本机已安装的平台（`only` 指定时只取该平台；测试时取伪造目录）。
    pub fn platform_adapters(&self, only: Option<&str>) -> Vec<Box<dyn PlatformAdapter>> {
        let mut disabled = self.config.backup.disabled_platforms.clone();
        if let Some(p) = only {
            disabled.extend(["steam", "epic", "gog"].iter().filter(|x| !x.eq_ignore_ascii_case(p)).map(|s| s.to_string()));
        }
        match &self.fake_root {
            Some(root) => fake_adapters(root, &disabled),
            None => platforms::detect_all(&disabled),
        }
    }

    /// 扫描本机已安装的平台（`only` 指定时只扫描该平台）。
    pub fn scan_platforms(&mut self, lock: &RepoLock, only: Option<&str>) -> Result<ScanReport> {
        let adapters = self.platform_adapters(only);
        self.scan_with(lock, &adapters)
    }

    /// 识别已安装的游戏及其存档位置，不改动游戏库。
    fn discover_all(&self, adapters: &[Box<dyn PlatformAdapter>]) -> Result<(Vec<Discovered>, Vec<NoSaveGame>, LoadedFrom)> {
        let (rules, loaded) = self.load_rules()?;
        let mut found = Vec::new();
        let mut none = Vec::new();
        for a in adapters {
            for installed in a.list_games() {
                let d = resolve::discover(&rules, &self.known_dirs, a.store(), &installed);
                if d.locations.is_empty() {
                    none.push(NoSaveGame { id: installed.game_id(), name: installed.name.clone(), platform: installed.platform.clone(), rule: d.rule.clone() });
                } else {
                    found.push(d);
                }
            }
        }
        Ok((found, none, loaded))
    }

    /// 扫描（发现游戏）：不会把游戏加入游戏库，由用户选择加入或忽略。
    /// 已在游戏库中的游戏会更新其存档位置（新出现的位置追加）。
    pub fn scan_with(&mut self, _lock: &RepoLock, adapters: &[Box<dyn PlatformAdapter>]) -> Result<ScanReport> {
        let (found, without_saves, loaded) = self.discover_all(adapters)?;
        let mut report = ScanReport { platforms: adapters.iter().map(|a| a.id().to_string()).collect(), rules: Some(loaded), games: Vec::new(), without_saves };
        let mut managed_changed = false;
        for d in &found {
            let id = d.game.game_id();
            let state = match self.index.game(&id)? {
                Some(g) if g.enabled => FoundState::Managed,
                Some(_) => FoundState::Ignored,
                None => FoundState::New,
            };
            let record = if state == FoundState::Managed {
                managed_changed = true;
                self.upsert_discovered(d, true)?
            } else {
                // 未加入的游戏只用本地数据补全封面与游玩时长（不联网），便于用户挑选。
                let temp = discovered_record(d, false);
                if let Err(e) = self.refresh_metadata_offline(&temp) {
                    tracing::debug!(game = %id, "读取本地游戏信息失败：{e}");
                }
                temp
            };
            report.games.push(self.found_view(d, &record, state)?);
        }
        if self.fetch_metadata_on_scan {
            for g in report.games.iter().filter(|g| g.state == FoundState::Managed) {
                if let Ok(Some(rec)) = self.index.game(&g.id)
                    && let Err(e) = self.refresh_metadata(&rec, false)
                {
                    tracing::info!(game = %g.id, "刷新游戏信息失败：{e}");
                }
            }
        }
        if managed_changed {
            self.save_games_file()?;
        }
        report.games.sort_by_key(|g| g.name.to_lowercase());
        Ok(report)
    }

    fn found_view(&self, d: &Discovered, record: &GameRecord, state: FoundState) -> Result<DiscoveredGame> {
        let roots: Vec<ScanRoot> = d
            .locations
            .iter()
            .enumerate()
            .filter(|(_, l)| l.kind != FoundKind::Registry)
            .map(|(ord, l)| ScanRoot {
                ord,
                path: PathBuf::from(&l.path),
                kind: match l.kind {
                    FoundKind::File => RootKind::File,
                    FoundKind::Dir => RootKind::Dir,
                    _ => RootKind::Glob,
                },
                tag: if l.config { FileTag::Config } else { FileTag::Save },
            })
            .collect();
        let files = collect_files(&roots).unwrap_or_default();
        let meta = self.metadata(&record.id)?;
        Ok(DiscoveredGame {
            id: record.id.clone(),
            name: record.name.clone(),
            name_zh: meta.as_ref().and_then(|m| m.data.name_zh.clone()),
            platform: record.platform.clone(),
            rule: d.rule.clone(),
            state,
            locations: d.locations.iter().map(|l| l.path.clone()).collect(),
            unconfirmed: d.locations.iter().filter(|l| !l.confirmed).count(),
            files: files.len() as u64,
            bytes: files.iter().map(|f| f.size).sum(),
            cover_thumb: meta.as_ref().and_then(|m| m.cover_thumb_path.clone().or_else(|| m.data.cover_path.clone())),
            cloud_save: d.cloud_save,
            playtime_min: d.game.playtime_min,
            last_played: d.game.last_played,
        })
    }

    /// 把扫描发现的游戏加入游戏库（按 ID 或名称；`all_new` 时加入全部新发现的游戏）。返回加入的游戏。
    pub fn include_games(&mut self, lock: &RepoLock, keys: &[String], all_new: bool) -> Result<Vec<GameRecord>> {
        let adapters = self.platform_adapters(None);
        self.include_with(lock, &adapters, keys, all_new)
    }

    pub fn include_with(&mut self, _lock: &RepoLock, adapters: &[Box<dyn PlatformAdapter>], keys: &[String], all_new: bool) -> Result<Vec<GameRecord>> {
        let (found, _, _) = self.discover_all(adapters)?;
        let mut picked: Vec<&Discovered> = Vec::new();
        for k in keys {
            let lower = k.to_lowercase();
            let d = found
                .iter()
                .find(|d| d.game.game_id() == *k)
                .or_else(|| found.iter().find(|d| d.game.name.to_lowercase() == lower))
                .ok_or_else(|| AppError::GameNotFound(k.clone()))?;
            picked.push(d);
        }
        if all_new {
            for d in &found {
                if self.index.game(&d.game.game_id())?.is_none() && !picked.iter().any(|p| p.game.game_id() == d.game.game_id()) {
                    picked.push(d);
                }
            }
        }
        let mut out = Vec::new();
        for d in picked {
            out.push(self.upsert_discovered(d, true)?);
        }
        if !out.is_empty() {
            self.save_games_file()?;
        }
        Ok(out)
    }

    /// 忽略扫描发现的游戏：不加入游戏库，之后扫描不再作为新发现提示；已在库中的游戏移出游戏库（历史快照保留）。
    pub fn ignore_games(&mut self, lock: &RepoLock, keys: &[String]) -> Result<Vec<GameRecord>> {
        let adapters = self.platform_adapters(None);
        self.ignore_with(lock, &adapters, keys)
    }

    pub fn ignore_with(&mut self, _lock: &RepoLock, adapters: &[Box<dyn PlatformAdapter>], keys: &[String]) -> Result<Vec<GameRecord>> {
        let (found, _, _) = self.discover_all(adapters)?;
        let mut out = Vec::new();
        for k in keys {
            let lower = k.to_lowercase();
            let record = match found.iter().find(|d| d.game.game_id() == *k || d.game.name.to_lowercase() == lower) {
                Some(d) => self.upsert_discovered(d, false)?,
                None => {
                    let mut g = self.find_game(k)?;
                    g.enabled = false;
                    self.index.upsert_game(&g)?;
                    g
                }
            };
            out.push(record);
        }
        if !out.is_empty() {
            self.save_games_file()?;
        }
        Ok(out)
    }

    /// 写入或更新识别到的游戏。保留用户设置（是否备份配置文件）以及已确认的启发式位置。
    fn upsert_discovered(&mut self, d: &Discovered, enabled: bool) -> Result<GameRecord> {
        let id = d.game.game_id();
        let existing = self.index.game(&id)?;
        let mut record = discovered_record(d, enabled);
        if let Some(e) = &existing {
            record.name = e.name.clone();
            record.include_config = e.include_config;
            record.custom_rule_json = e.custom_rule_json.clone();
        }
        let previous = self.index.locations(&id)?;
        let locations: Vec<LocationRecord> = d
            .locations
            .iter()
            .map(|l| {
                let was_confirmed = previous.iter().any(|p| resolve::path_key(&p.resolved_path) == resolve::path_key(&l.path) && p.confirmed);
                LocationRecord {
                    path_template: l.template.clone(),
                    resolved_path: l.path.clone(),
                    kind: match l.kind {
                        FoundKind::File => LocationKind::File,
                        FoundKind::Dir => LocationKind::Dir,
                        FoundKind::Glob => LocationKind::Glob,
                        FoundKind::Registry => LocationKind::Registry,
                    },
                    tag: if l.config { FileTag::Config } else { FileTag::Save },
                    source: l.source.as_str().to_string(),
                    confirmed: l.confirmed || was_confirmed,
                }
            })
            .collect();
        // 存档位置的顺序决定清单路径前缀：保持已有位置的序号不变，新位置追加在后面。
        let merged = merge_locations(previous, locations);
        self.index.upsert_game(&record)?;
        self.index.set_locations(&id, &merged)?;
        Ok(record)
    }

    /// 确认启发式找到的存档位置，并写入用户自定义规则（文档 4.2：确认后转为自定义规则）。
    pub fn confirm_locations(&mut self, _lock: &RepoLock, key: &str) -> Result<(GameRecord, usize)> {
        let g = self.find_game(key)?;
        let mut locs = self.index.locations(&g.id)?;
        let mut n = 0;
        for l in locs.iter_mut().filter(|l| !l.confirmed) {
            l.confirmed = true;
            l.source = LocationSource::User.as_str().to_string();
            n += 1;
        }
        if n > 0 {
            self.index.set_locations(&g.id, &locs)?;
            let files: Vec<(String, bool)> = locs
                .iter()
                .filter(|l| l.kind != LocationKind::Registry && l.source == "user")
                .map(|l| (l.resolved_path.replace('\\', "/"), l.tag == FileTag::Config))
                .collect();
            let path = self.manifest_paths().custom_rules;
            manifest::save_custom_rule(&path, &g.name, &files).map_err(|e| AppError::Invalid(e.to_string()))?;
            self.save_games_file()?;
        }
        Ok((g, n))
    }
}

/// 由扫描结果构造游戏记录。
fn discovered_record(d: &Discovered, enabled: bool) -> GameRecord {
    let g = &d.game;
    GameRecord {
        id: g.game_id(),
        name: g.name.clone(),
        platform: g.platform.clone(),
        app_id: Some(g.app_id.clone()),
        install_dir: g.install_dir.as_ref().map(|p| p.to_string_lossy().into_owned()),
        enabled,
        include_config: None,
        custom_rule_json: None,
    }
}

/// 保持已有位置的顺序（序号即清单路径前缀，改变会导致历史对比错位），新位置追加；
/// 已有但本次未找到的位置保留（存档可能暂时被删除）。
fn merge_locations(previous: Vec<LocationRecord>, found: Vec<LocationRecord>) -> Vec<LocationRecord> {
    let mut out = previous;
    for f in found {
        match out.iter_mut().find(|p| resolve::path_key(&p.resolved_path) == resolve::path_key(&f.resolved_path)) {
            Some(p) => {
                let confirmed = p.confirmed || f.confirmed;
                *p = LocationRecord { confirmed, ..f };
            }
            None => out.push(f),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Env;
    use crate::{BackupOptions, BackupResult, GameState};
    use bonfire_discovery::KnownDirs;
    use bonfire_discovery::platforms::SteamAdapter;
    use std::path::Path;

    const RULES: &str = r#"
"Hollow Knight":
  files:
    "<winLocalAppDataLow>/Team Cherry/Hollow Knight/*.dat":
      tags: [save]
  steam:
    id: 367520
"#;

    fn write(p: &Path, s: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    }

    /// 伪造：Steam 安装了 Hollow Knight（有存档）与 Indie Quest（规则库没有，存档在 LocalLow），以及没有存档的 Empty Game。
    fn setup(env: &mut Env) -> Vec<Box<dyn PlatformAdapter>> {
        let root = env.dir.path().join("pc");
        env.app.known_dirs = KnownDirs::under(&root);
        let steam = root.join("Steam");
        write(&steam.join("steamapps/appmanifest_367520.acf"), "\"AppState\" { \"appid\" \"367520\" \"name\" \"Hollow Knight\" \"installdir\" \"Hollow Knight\" }");
        write(&steam.join("steamapps/appmanifest_9.acf"), "\"AppState\" { \"appid\" \"9\" \"name\" \"Indie Quest\" \"installdir\" \"Indie Quest\" }");
        write(&steam.join("steamapps/appmanifest_10.acf"), "\"AppState\" { \"appid\" \"10\" \"name\" \"Empty Game\" \"installdir\" \"Empty\" }");
        let low = env.app.known_dirs.local_app_data_low.clone().unwrap();
        write(&low.join("Team Cherry/Hollow Knight/user1.dat"), "save 1");
        write(&low.join("Studio/Indie Quest/save.json"), "{}");
        let paths = env.app.manifest_paths();
        std::fs::create_dir_all(&paths.cache_dir).unwrap();
        std::fs::write(paths.cache_dir.join(manifest::COMPACT_FILE), manifest::Manifest::from_yaml(RULES).unwrap().to_compact()).unwrap();
        vec![Box::new(SteamAdapter { root: steam })]
    }

    fn scan(env: &mut Env, adapters: &[Box<dyn PlatformAdapter>]) -> ScanReport {
        let lock = env.app.repo.lock().unwrap();
        env.app.scan_with(&lock, adapters).unwrap()
    }

    fn state_of(r: &ScanReport, id: &str) -> FoundState {
        r.games.iter().find(|g| g.id == id).unwrap().state
    }

    #[test]
    fn scan_only_discovers_and_user_picks_games() {
        let mut env = Env::new();
        let adapters = setup(&mut env);
        let r = scan(&mut env, &adapters);
        assert_eq!(r.games.len(), 2);
        assert_eq!(r.without_saves.len(), 1);
        assert!(env.app.games().unwrap().is_empty(), "扫描不会自动加入游戏库");
        let hk = r.games.iter().find(|g| g.id == "steam-367520").unwrap();
        assert_eq!((hk.state, hk.files, hk.bytes, hk.unconfirmed), (FoundState::New, 1, 6, 0));
        assert_eq!(hk.rule.as_deref(), Some("Hollow Knight"));
        let iq = r.games.iter().find(|g| g.id == "steam-9").unwrap();
        assert_eq!((iq.unconfirmed, iq.rule.as_deref()), (1, None), "启发式结果待确认");

        let lock = env.app.repo.lock().unwrap();
        let added = env.app.include_with(&lock, &adapters, &["Hollow Knight".into()], false).unwrap();
        assert_eq!(added.len(), 1);
        env.app.ignore_with(&lock, &adapters, &["steam-9".into()]).unwrap();
        assert!(matches!(env.app.include_with(&lock, &adapters, &["nope".into()], false), Err(AppError::GameNotFound(_))));
        drop(lock);
        assert_eq!(env.app.games().unwrap().iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), ["steam-367520"]);

        let r = scan(&mut env, &adapters);
        assert_eq!(state_of(&r, "steam-367520"), FoundState::Managed);
        assert_eq!(state_of(&r, "steam-9"), FoundState::Ignored);

        let lock = env.app.repo.lock().unwrap();
        let res = env.app.backup(&lock, &env.app.find_game("steam-367520").unwrap(), &BackupOptions::default()).unwrap();
        assert!(matches!(res, BackupResult::Created { files: 1, .. }));
        // 忽略后仍可加入；加入后启发式位置需确认才备份。
        env.app.include_with(&lock, &adapters, &["steam-9".into()], false).unwrap();
        drop(lock);
        let iq_game = env.app.find_game("steam-9").unwrap();
        assert!(iq_game.enabled);
        assert_eq!(env.app.status(&iq_game).unwrap().files, 0, "未确认的位置不参与备份");
        let lock = env.app.repo.lock().unwrap();
        let (_, n) = env.app.confirm_locations(&lock, "Indie Quest").unwrap();
        drop(lock);
        assert_eq!(n, 1);
        assert_eq!(env.app.status(&iq_game).unwrap().files, 1);
        let custom = std::fs::read_to_string(env.app.manifest_paths().custom_rules).unwrap();
        assert!(custom.contains("Indie Quest"), "{custom}");
        let r = scan(&mut env, &adapters);
        assert_eq!(r.games.iter().find(|g| g.id == "steam-9").unwrap().unconfirmed, 0, "确认后由用户规则命中");
        assert_eq!(env.app.locations("steam-9").unwrap().len(), 1);
    }

    #[test]
    fn include_all_new_and_remove_returns_to_ignored() {
        let mut env = Env::new();
        let adapters = setup(&mut env);
        let lock = env.app.repo.lock().unwrap();
        let added = env.app.include_with(&lock, &adapters, &[], true).unwrap();
        assert_eq!(added.len(), 2);
        assert!(env.app.include_with(&lock, &adapters, &[], true).unwrap().is_empty(), "没有新发现时不再加入");
        env.app.remove_game(&lock, "Hollow Knight").unwrap();
        drop(lock);
        let r = scan(&mut env, &adapters);
        assert_eq!(state_of(&r, "steam-367520"), FoundState::Ignored);
        assert_eq!(env.app.games().unwrap().len(), 1);
        let lock = env.app.repo.lock().unwrap();
        env.app.include_with(&lock, &adapters, &["steam-367520".into()], false).unwrap();
        drop(lock);
        assert_eq!(env.app.status(&env.app.find_game("Hollow Knight").unwrap()).unwrap().state, GameState::NeverBackedUp);
    }

    #[test]
    fn location_order_is_stable_across_scans() {
        let a = |p: &str| LocationRecord { path_template: p.into(), resolved_path: p.into(), kind: LocationKind::Dir, tag: FileTag::Save, source: "manifest".into(), confirmed: true };
        let merged = merge_locations(vec![a("C:/x"), a("C:/y")], vec![a("C:/z"), a("c:/X")]);
        let paths: Vec<_> = merged.iter().map(|l| l.resolved_path.as_str()).collect();
        assert_eq!(paths, ["c:/X", "C:/y", "C:/z"]);
    }
}
