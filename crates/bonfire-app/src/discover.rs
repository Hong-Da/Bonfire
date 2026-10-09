//! 扫描服务（文档 §3）：调用平台适配器与规则库，把识别到的游戏及其存档位置写入索引。

use std::path::PathBuf;

use bonfire_core::index::{GameRecord, LocationKind, LocationRecord};
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredGame {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub rule: Option<String>,
    pub locations: usize,
    /// 需要确认的启发式位置数。
    pub unconfirmed: usize,
    /// 本次新加入游戏库。
    pub added: bool,
    pub ignored: bool,
    pub cloud_save: bool,
    pub playtime_min: Option<u64>,
    pub last_played: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub platforms: Vec<String>,
    pub rules: Option<LoadedFrom>,
    /// 找到存档的游戏。
    pub games: Vec<DiscoveredGame>,
    /// 已安装但没有找到存档的游戏（不入库）。
    pub without_saves: Vec<NoSaveGame>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoSaveGame {
    pub id: String,
    pub name: String,
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

    /// 扫描本机已安装的平台（`only` 指定时只扫描该平台）。
    pub fn scan_platforms(&mut self, lock: &RepoLock, only: Option<&str>) -> Result<ScanReport> {
        let mut disabled = self.config.backup.disabled_platforms.clone();
        if let Some(p) = only {
            disabled.extend(["steam", "epic", "gog"].iter().filter(|x| !x.eq_ignore_ascii_case(p)).map(|s| s.to_string()));
        }
        let adapters = match &self.fake_root {
            Some(root) => fake_adapters(root, &disabled),
            None => platforms::detect_all(&disabled),
        };
        self.scan_with(lock, &adapters)
    }

    /// 用给定的适配器扫描（测试中传入伪造的平台目录）。
    pub fn scan_with(&mut self, _lock: &RepoLock, adapters: &[Box<dyn PlatformAdapter>]) -> Result<ScanReport> {
        let (rules, loaded) = self.load_rules()?;
        let mut report = ScanReport { platforms: adapters.iter().map(|a| a.id().to_string()).collect(), rules: Some(loaded), ..Default::default() };
        for a in adapters {
            for installed in a.list_games() {
                let d = resolve::discover(&rules, &self.known_dirs, a.store(), &installed);
                if d.locations.is_empty() {
                    report.without_saves.push(NoSaveGame { id: installed.game_id(), name: installed.name.clone(), rule: d.rule.clone() });
                    continue;
                }
                report.games.push(self.upsert_discovered(&d)?);
            }
        }
        // 游戏信息：新游戏与过期的联网获取，本地字段（游玩时长等）每次重读；失败不影响扫描。
        if self.fetch_metadata_on_scan {
            for dg in &report.games {
                if let Ok(Some(g)) = self.index.game(&dg.id)
                    && let Err(e) = self.refresh_metadata(&g, false)
                {
                    tracing::info!(game = %g.id, "刷新游戏信息失败：{e}");
                }
            }
        }
        if !report.games.is_empty() {
            self.save_games_file()?;
        }
        Ok(report)
    }

    /// 写入或更新识别到的游戏。已有游戏保留用户设置（启用状态、是否备份配置文件）以及已确认的启发式位置。
    fn upsert_discovered(&mut self, d: &Discovered) -> Result<DiscoveredGame> {
        let g = &d.game;
        let id = g.game_id();
        let existing = self.index.game(&id)?;
        let record = GameRecord {
            id: id.clone(),
            name: existing.as_ref().map(|e| e.name.clone()).unwrap_or_else(|| g.name.clone()),
            platform: g.platform.clone(),
            app_id: Some(g.app_id.clone()),
            install_dir: g.install_dir.as_ref().map(|p| p.to_string_lossy().into_owned()),
            enabled: existing.as_ref().is_none_or(|e| e.enabled),
            include_config: existing.as_ref().and_then(|e| e.include_config),
            custom_rule_json: existing.as_ref().and_then(|e| e.custom_rule_json.clone()),
        };
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
        Ok(DiscoveredGame {
            id,
            name: record.name,
            platform: g.platform.clone(),
            rule: d.rule.clone(),
            locations: merged.iter().filter(|l| l.confirmed).count(),
            unconfirmed: merged.iter().filter(|l| !l.confirmed).count(),
            added: existing.is_none(),
            ignored: !record.enabled,
            cloud_save: d.cloud_save,
            playtime_min: g.playtime_min,
            last_played: g.last_played,
        })
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

    #[test]
    fn scan_adds_games_and_backs_up_confirmed_locations_only() {
        let mut env = Env::new();
        let adapters = setup(&mut env);
        let r = scan(&mut env, &adapters);
        assert_eq!(r.games.len(), 2);
        assert_eq!(r.without_saves.len(), 1);
        let hk = r.games.iter().find(|g| g.id == "steam-367520").unwrap();
        assert!(hk.added && hk.locations == 1 && hk.unconfirmed == 0);
        let iq = r.games.iter().find(|g| g.id == "steam-9").unwrap();
        assert_eq!((iq.locations, iq.unconfirmed), (0, 1), "启发式结果待确认");

        let iq_game = env.app.find_game("steam-9").unwrap();
        assert_eq!(env.app.status(&iq_game).unwrap().files, 0, "未确认的位置不参与备份");
        let lock = env.app.repo.lock().unwrap();
        let (_, n) = env.app.confirm_locations(&lock, "Indie Quest").unwrap();
        drop(lock);
        assert_eq!(n, 1);
        assert_eq!(env.app.status(&iq_game).unwrap().files, 1);
        let custom = std::fs::read_to_string(env.app.manifest_paths().custom_rules).unwrap();
        assert!(custom.contains("Indie Quest"), "{custom}");

        // 再次扫描：不重复添加，确认状态保留（现在由用户规则命中）。
        let r = scan(&mut env, &adapters);
        assert!(r.games.iter().all(|g| !g.added));
        assert_eq!(r.games.iter().find(|g| g.id == "steam-9").unwrap().unconfirmed, 0);
        assert_eq!(env.app.locations("steam-9").unwrap().len(), 1);

        let lock = env.app.repo.lock().unwrap();
        let res = env.app.backup(&lock, &env.app.find_game("steam-367520").unwrap(), &BackupOptions::default()).unwrap();
        assert!(matches!(res, BackupResult::Created { files: 1, .. }));
    }

    #[test]
    fn removed_discovered_game_stays_ignored() {
        let mut env = Env::new();
        let adapters = setup(&mut env);
        scan(&mut env, &adapters);
        let lock = env.app.repo.lock().unwrap();
        env.app.remove_game(&lock, "Hollow Knight").unwrap();
        drop(lock);
        let r = scan(&mut env, &adapters);
        assert!(r.games.iter().find(|g| g.id == "steam-367520").unwrap().ignored);
        assert!(env.app.games().unwrap().iter().all(|g| g.id != "steam-367520"));
        assert_eq!(env.app.all_games().unwrap().len(), 2);
        let lock = env.app.repo.lock().unwrap();
        env.app.enable_game(&lock, "steam-367520").unwrap();
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
