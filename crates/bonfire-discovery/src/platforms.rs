//! 平台适配器（文档 4.1）：回答“装了哪些游戏、装在哪、用户 ID 是什么”。
//!
//! 每个适配器都可以用指定的根目录构造，便于用伪造的平台目录测试；
//! `detect()` 从注册表或默认位置查找真实安装。所有操作只读。

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::vdf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledGame {
    /// `steam` / `epic` / `gog` …
    pub platform: String,
    pub app_id: String,
    pub name: String,
    pub install_dir: Option<PathBuf>,
    /// 库目录（`<root>`），如 `steamapps/common`。
    pub library_root: Option<PathBuf>,
    /// 安装文件夹名（`<game>`）。
    pub install_folder: Option<String>,
    /// 最后游玩时间（Unix 秒）。
    pub last_played: Option<i64>,
    /// 游玩时长（分钟）。
    pub playtime_min: Option<u64>,
    /// 是否启用了平台云存档。
    pub cloud_save: bool,
}

impl InstalledGame {
    /// 游戏 ID：`<平台>-<appId>`（文档 7.3）。
    pub fn game_id(&self) -> String {
        format!("{}-{}", self.platform, self.app_id)
    }
}

pub trait PlatformAdapter {
    fn id(&self) -> &'static str;
    /// 规则库 `when.store` 中对应的商店名。
    fn store(&self) -> &'static str;
    fn list_games(&self) -> Vec<InstalledGame>;
    /// 本机登录过的账号 ID。
    fn list_users(&self) -> Vec<String> {
        Vec::new()
    }
}

fn read_text(p: &Path) -> Option<String> {
    let bytes = std::fs::read(p).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

// ---------- Steam ----------

pub struct SteamAdapter {
    pub root: PathBuf,
}

impl SteamAdapter {
    /// 注册表 `HKCU\Software\Valve\Steam\SteamPath`。
    pub fn detect() -> Option<Self> {
        let p = crate::registry::read_string(r"HKCU\Software\Valve\Steam", "SteamPath")?;
        let root = PathBuf::from(p);
        root.join("steamapps").is_dir().then_some(Self { root })
    }

    /// 所有库目录（含 Steam 安装目录本身）。
    pub fn libraries(&self) -> Vec<PathBuf> {
        let mut libs = vec![self.root.clone()];
        if let Some(text) = read_text(&self.root.join("steamapps/libraryfolders.vdf"))
            && let Ok(v) = vdf::parse(&text)
            && let Some(lf) = v.obj("libraryfolders")
        {
            for (_, entry) in lf.iter() {
                let path = match entry {
                    vdf::Vdf::Obj(o) => o.str("path").map(PathBuf::from),
                    vdf::Vdf::Str(s) => Some(PathBuf::from(s)),
                };
                if let Some(p) = path
                    && !libs.iter().any(|l| same_path(l, &p))
                {
                    libs.push(p);
                }
            }
        }
        libs
    }

    fn user_dirs(&self) -> Vec<PathBuf> {
        std::fs::read_dir(self.root.join("userdata"))
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.is_dir()
                            && p.file_name().is_some_and(|n| {
                                n.to_string_lossy().chars().all(|c| c.is_ascii_digit())
                            })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 各账号 localconfig.vdf 中的游玩时长与最后游玩时间，取最大值。
    fn play_stats(&self, app_id: &str) -> (Option<u64>, Option<i64>) {
        let mut playtime = None;
        let mut last = None;
        for u in self.user_dirs() {
            let Some(text) = read_text(&u.join("config/localconfig.vdf")) else {
                continue;
            };
            let Ok(v) = vdf::parse(&text) else { continue };
            let apps = v
                .path(&["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"])
                .or_else(|| {
                    v.path(&["UserLocalConfigStore", "Software", "Valve", "Steam", "Apps"])
                });
            if let Some(app) = apps.and_then(|a| a.obj(app_id)) {
                if let Some(p) = app.str("Playtime").and_then(|s| s.parse::<u64>().ok()) {
                    playtime = Some(playtime.map_or(p, |x: u64| x.max(p)));
                }
                if let Some(l) = app.str("LastPlayed").and_then(|s| s.parse::<i64>().ok()) {
                    last = Some(last.map_or(l, |x: i64| x.max(l)));
                }
            }
        }
        (playtime, last)
    }

    fn cloud_enabled(&self, app_id: &str) -> bool {
        self.user_dirs()
            .iter()
            .any(|u| u.join(app_id).join("remotecache.vdf").is_file())
    }

    /// 本地 librarycache 中的图片：旧版为 `<appid>_library_600x900.jpg` 等，新版为 `<appid>/<哈希>.jpg`。
    pub fn cached_images(&self, app_id: &str) -> Vec<PathBuf> {
        let cache = self.root.join("appcache/librarycache");
        let mut out = Vec::new();
        for suffix in [
            "_library_600x900.jpg",
            "_header.jpg",
            "_library_hero.jpg",
            "_icon.jpg",
            "_logo.png",
        ] {
            let p = cache.join(format!("{app_id}{suffix}"));
            if p.is_file() {
                out.push(p);
            }
        }
        let mut stack = vec![cache.join(app_id)];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("jpg") || x.eq_ignore_ascii_case("png"))
                {
                    out.push(p);
                }
            }
        }
        out
    }
}

impl PlatformAdapter for SteamAdapter {
    fn id(&self) -> &'static str {
        "steam"
    }

    fn store(&self) -> &'static str {
        "steam"
    }

    fn list_games(&self) -> Vec<InstalledGame> {
        let mut games = Vec::new();
        for lib in self.libraries() {
            let apps = lib.join("steamapps");
            let Ok(rd) = std::fs::read_dir(&apps) else {
                continue;
            };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if !(name.starts_with("appmanifest_") && name.ends_with(".acf")) {
                    continue;
                }
                let Some(text) = read_text(&e.path()) else {
                    continue;
                };
                let Ok(v) = vdf::parse(&text) else {
                    tracing::warn!(file = %e.path().display(), "无法解析 appmanifest");
                    continue;
                };
                let Some(state) = v.obj("AppState") else {
                    continue;
                };
                let (Some(app_id), Some(title)) = (state.str("appid"), state.str("name")) else {
                    continue;
                };
                // Steamworks 公共组件等非游戏条目。
                if app_id == "228980" {
                    continue;
                }
                let folder = state.str("installdir").map(String::from);
                let common = apps.join("common");
                let (playtime, last_local) = self.play_stats(app_id);
                let last_played = last_local
                    .or_else(|| state.str("LastPlayed").and_then(|s| s.parse().ok()))
                    .filter(|&t: &i64| t > 0);
                games.push(InstalledGame {
                    platform: "steam".into(),
                    app_id: app_id.to_string(),
                    name: title.to_string(),
                    install_dir: folder.as_ref().map(|f| common.join(f)),
                    library_root: Some(common),
                    install_folder: folder,
                    last_played,
                    playtime_min: playtime,
                    cloud_save: self.cloud_enabled(app_id),
                });
            }
        }
        games.sort_by_key(|a| a.name.to_lowercase());
        games.dedup_by(|a, b| a.app_id == b.app_id);
        games
    }

    fn list_users(&self) -> Vec<String> {
        self.user_dirs()
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect()
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .eq_ignore_ascii_case(b.to_string_lossy().replace('\\', "/").trim_end_matches('/'))
}

// ---------- Epic ----------

pub struct EpicAdapter {
    pub manifests_dir: PathBuf,
}

impl EpicAdapter {
    /// `%ProgramData%\Epic\EpicGamesLauncher\Data\Manifests`
    pub fn detect() -> Option<Self> {
        let pd = std::env::var_os("PROGRAMDATA")?;
        let dir = PathBuf::from(pd).join(r"Epic\EpicGamesLauncher\Data\Manifests");
        dir.is_dir().then_some(Self { manifests_dir: dir })
    }
}

impl PlatformAdapter for EpicAdapter {
    fn id(&self) -> &'static str {
        "epic"
    }

    fn store(&self) -> &'static str {
        "epic"
    }

    fn list_games(&self) -> Vec<InstalledGame> {
        let mut games = Vec::new();
        for e in std::fs::read_dir(&self.manifests_dir)
            .into_iter()
            .flatten()
            .flatten()
        {
            if !e
                .path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("item"))
            {
                continue;
            }
            let Some(text) = read_text(&e.path()) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            let s = |k: &str| {
                v.get(k)
                    .and_then(|x| x.as_str())
                    .filter(|x| !x.is_empty())
                    .map(String::from)
            };
            if v.get("bIsIncompleteInstall").and_then(|x| x.as_bool()) == Some(true) {
                continue;
            }
            // 只保留主游戏，跳过 DLC。
            if let (Some(app), Some(main)) = (s("AppName"), s("MainGameAppName"))
                && app != main
            {
                continue;
            }
            let (Some(id), Some(name)) = (
                s("CatalogItemId").or_else(|| s("AppName")),
                s("DisplayName"),
            ) else {
                continue;
            };
            let install = s("InstallLocation").map(PathBuf::from);
            games.push(InstalledGame {
                platform: "epic".into(),
                app_id: id,
                name,
                library_root: install
                    .as_ref()
                    .and_then(|p| p.parent().map(Path::to_path_buf)),
                install_folder: install
                    .as_ref()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())),
                install_dir: install,
                last_played: None,
                playtime_min: None,
                cloud_save: false,
            });
        }
        games.sort_by_key(|a| a.name.to_lowercase());
        games
    }
}

// ---------- GOG ----------

/// GOG 游戏登记：(gameID, 名称, 安装目录)。
pub struct GogAdapter {
    pub entries: Vec<(String, String, PathBuf)>,
}

impl GogAdapter {
    /// 注册表 `HKLM\SOFTWARE\WOW6432Node\GOG.com\Games\<id>`（gameName、path）。
    pub fn detect() -> Option<Self> {
        let base = r"HKLM\SOFTWARE\WOW6432Node\GOG.com\Games";
        let mut entries = Vec::new();
        for id in crate::registry::subkeys(base) {
            let key = format!("{base}\\{id}");
            if let (Some(name), Some(path)) = (
                crate::registry::read_string(&key, "gameName"),
                crate::registry::read_string(&key, "path"),
            ) {
                let gid = crate::registry::read_string(&key, "gameID").unwrap_or(id);
                entries.push((gid, name, PathBuf::from(path)));
            }
        }
        (!entries.is_empty()).then_some(Self { entries })
    }
}

impl PlatformAdapter for GogAdapter {
    fn id(&self) -> &'static str {
        "gog"
    }

    fn store(&self) -> &'static str {
        "gog"
    }

    fn list_games(&self) -> Vec<InstalledGame> {
        let mut games: Vec<InstalledGame> = self
            .entries
            .iter()
            .filter(|(_, _, path)| path.is_dir())
            .map(|(id, name, path)| InstalledGame {
                platform: "gog".into(),
                app_id: id.clone(),
                name: name.clone(),
                install_dir: Some(path.clone()),
                library_root: path.parent().map(Path::to_path_buf),
                install_folder: path.file_name().map(|n| n.to_string_lossy().into_owned()),
                last_played: None,
                playtime_min: None,
                cloud_save: false,
            })
            .collect();
        games.sort_by_key(|a| a.name.to_lowercase());
        games
    }
}

/// 检测本机已安装的平台（跳过被关闭的平台）。
pub fn detect_all(disabled: &[String]) -> Vec<Box<dyn PlatformAdapter>> {
    let on = |id: &str| !disabled.iter().any(|d| d.eq_ignore_ascii_case(id));
    let mut v: Vec<Box<dyn PlatformAdapter>> = Vec::new();
    if on("steam")
        && let Some(a) = SteamAdapter::detect()
    {
        v.push(Box::new(a));
    }
    if on("epic")
        && let Some(a) = EpicAdapter::detect()
    {
        v.push(Box::new(a));
    }
    if on("gog")
        && let Some(a) = GogAdapter::detect()
    {
        v.push(Box::new(a));
    }
    v
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn write(p: &Path, s: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    }

    /// 伪造的 Steam：两个库，一个账号；Hollow Knight 在主库，Elden Ring 在第二个库。
    pub fn fake_steam(root: &Path) -> SteamAdapter {
        let steam = root.join("Steam");
        let lib2 = root.join("SteamLibrary");
        write(
            &steam.join("steamapps/libraryfolders.vdf"),
            &format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
                steam.display().to_string().replace('\\', "\\\\"),
                lib2.display().to_string().replace('\\', "\\\\")
            ),
        );
        write(
            &steam.join("steamapps/appmanifest_367520.acf"),
            "\"AppState\" { \"appid\" \"367520\" \"name\" \"Hollow Knight\" \"installdir\" \"Hollow Knight\" \"LastPlayed\" \"1700000000\" }",
        );
        write(
            &steam.join("steamapps/appmanifest_228980.acf"),
            "\"AppState\" { \"appid\" \"228980\" \"name\" \"Steamworks Common Redistributables\" \"installdir\" \"Steamworks Shared\" }",
        );
        std::fs::create_dir_all(steam.join("steamapps/common/Hollow Knight")).unwrap();
        write(
            &lib2.join("steamapps/appmanifest_1245620.acf"),
            "\"AppState\" { \"appid\" \"1245620\" \"name\" \"ELDEN RING\" \"installdir\" \"ELDEN RING\" }",
        );
        std::fs::create_dir_all(lib2.join("steamapps/common/ELDEN RING")).unwrap();
        write(
            &steam.join("steamapps/appmanifest_broken.acf"),
            "\"AppState\" { ",
        );
        write(
            &steam.join("userdata/12345678/config/localconfig.vdf"),
            "\"UserLocalConfigStore\" { \"Software\" { \"Valve\" { \"Steam\" { \"apps\" { \"367520\" { \"LastPlayed\" \"1710000000\" \"Playtime\" \"2520\" } } } } } }",
        );
        write(
            &steam.join("userdata/12345678/367520/remotecache.vdf"),
            "\"367520\" { }",
        );
        write(
            &steam.join("appcache/librarycache/367520/abc/cover.jpg"),
            "jpg",
        );
        SteamAdapter { root: steam }
    }

    #[test]
    fn steam_lists_games_across_libraries() {
        let d = tempfile::tempdir().unwrap();
        let s = fake_steam(d.path());
        assert_eq!(s.libraries().len(), 2);
        let games = s.list_games();
        let names: Vec<_> = games.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(
            names,
            ["ELDEN RING", "Hollow Knight"],
            "跳过公共组件与损坏的清单"
        );
        let hk = &games[1];
        assert_eq!(hk.game_id(), "steam-367520");
        assert_eq!(hk.playtime_min, Some(2520));
        assert_eq!(
            hk.last_played,
            Some(1710000000),
            "localconfig 优先于 appmanifest"
        );
        assert!(hk.cloud_save);
        assert!(
            hk.install_dir
                .as_ref()
                .unwrap()
                .ends_with("steamapps/common/Hollow Knight")
        );
        assert!(
            games[0]
                .library_root
                .as_ref()
                .unwrap()
                .starts_with(d.path().join("SteamLibrary"))
        );
        assert!(!games[0].cloud_save);
        assert_eq!(s.list_users(), ["12345678"]);
        assert_eq!(s.cached_images("367520").len(), 1);
    }

    #[test]
    fn epic_reads_item_manifests() {
        let d = tempfile::tempdir().unwrap();
        let install = d.path().join("Epic Games/Celeste");
        std::fs::create_dir_all(&install).unwrap();
        let item = |name: &str, body: serde_json::Value| {
            write(
                &d.path().join(format!("Manifests/{name}.item")),
                &body.to_string(),
            )
        };
        item(
            "a",
            serde_json::json!({"DisplayName": "Celeste", "InstallLocation": install, "AppName": "Salt", "MainGameAppName": "Salt", "CatalogItemId": "cat123"}),
        );
        item(
            "b",
            serde_json::json!({"DisplayName": "Celeste DLC", "AppName": "SaltDLC", "MainGameAppName": "Salt", "CatalogItemId": "dlc"}),
        );
        item(
            "c",
            serde_json::json!({"DisplayName": "Half", "AppName": "H", "MainGameAppName": "H", "CatalogItemId": "h", "bIsIncompleteInstall": true}),
        );
        write(&d.path().join("Manifests/bad.item"), "{");
        let games = EpicAdapter {
            manifests_dir: d.path().join("Manifests"),
        }
        .list_games();
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].game_id(), "epic-cat123");
        assert_eq!(games[0].install_folder.as_deref(), Some("Celeste"));
    }

    #[test]
    fn gog_entries_need_existing_dir() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("GOG Games/Witcher 3")).unwrap();
        let a = GogAdapter {
            entries: vec![
                (
                    "1207664663".into(),
                    "The Witcher 3".into(),
                    d.path().join("GOG Games/Witcher 3"),
                ),
                (
                    "1".into(),
                    "Uninstalled".into(),
                    d.path().join("GOG Games/gone"),
                ),
            ],
        };
        let games = a.list_games();
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].game_id(), "gog-1207664663");
    }
}
