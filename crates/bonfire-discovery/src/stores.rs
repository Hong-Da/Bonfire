//! 其余平台的适配器（文档 4.1，M4-1）：Ubisoft Connect、EA app、Battle.net、Xbox / Microsoft Store。
//!
//! - Ubisoft、EA、Battle.net 的游戏都在 Windows 的“卸载程序”登记中（`...\CurrentVersion\Uninstall`），
//!   按登记的特征区分：Ubisoft 的键名为 `Uplay Install <游戏 ID>`；EA app 的卸载程序在 `EAInstaller` 下；
//!   Battle.net 的卸载程序是 Battle.net 启动器或 Blizzard 卸载程序。Ubisoft 另从 `Ubisoft\Launcher` 读取启动器目录
//!   （规则库中 `<root>` 指它，存档在 `<root>/savegames/<账号>/<ID>`）。
//! - Xbox / Microsoft Store：`%LOCALAPPDATA%\Packages\<包名>\SystemAppData\wgs` 是容器格式的存档，只能整个目录备份；
//!   规则库按路径中的包名对应游戏，规则库没有的游戏把整个 `wgs` 目录作为待确认的存档位置。
//!
//! 每个适配器都可以用登记数据或目录构造，便于测试；`detect()` 读取本机。所有操作只读。

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::platforms::{InstalledGame, PlatformAdapter};

/// 一条“卸载程序”登记。
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UninstallEntry {
    /// 注册表键名。
    pub key: String,
    pub name: String,
    pub install_dir: Option<PathBuf>,
    pub uninstall: String,
    pub publisher: String,
}

const UNINSTALL_KEYS: [&str; 3] = [
    r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
    r"HKLM\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall",
];

/// 本机所有“卸载程序”登记（读不到的跳过）。
pub fn uninstall_entries() -> Vec<UninstallEntry> {
    use crate::registry::{read_string, subkeys};
    let mut out = Vec::new();
    for base in UNINSTALL_KEYS {
        for key in subkeys(base) {
            let full = format!("{base}\\{key}");
            let Some(name) = read_string(&full, "DisplayName").filter(|n| !n.trim().is_empty())
            else {
                continue;
            };
            out.push(UninstallEntry {
                name,
                install_dir: read_string(&full, "InstallLocation")
                    .map(|p| p.trim().trim_matches('"').to_string())
                    .filter(|p| !p.is_empty())
                    .map(PathBuf::from),
                uninstall: read_string(&full, "UninstallString").unwrap_or_default(),
                publisher: read_string(&full, "Publisher").unwrap_or_default(),
                key,
            });
        }
    }
    out
}

/// 测试用：伪造根目录下的 `Registry/uninstall.json`（`UninstallEntry` 数组）。
pub fn fake_uninstall_entries(root: &Path) -> Vec<UninstallEntry> {
    std::fs::read(root.join("Registry").join("uninstall.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn installed(
    platform: &str,
    id: String,
    name: String,
    dir: PathBuf,
    root: Option<PathBuf>,
) -> InstalledGame {
    InstalledGame {
        platform: platform.into(),
        app_id: id,
        name,
        library_root: root.or_else(|| dir.parent().map(Path::to_path_buf)),
        install_folder: dir.file_name().map(|n| n.to_string_lossy().into_owned()),
        install_dir: Some(dir),
        last_played: None,
        playtime_min: None,
        cloud_save: false,
    }
}

fn sorted(mut games: Vec<InstalledGame>) -> Vec<InstalledGame> {
    games.sort_by_key(|a| a.name.to_lowercase());
    games.dedup_by(|a, b| a.app_id == b.app_id);
    games
}

/// 游戏 ID 只保留字母、数字与 `-_.`（用于游戏 ID 与文件名）。
fn id_of(s: &str) -> String {
    let id: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    id.trim_matches('-').to_lowercase()
}

// ---------- Ubisoft Connect ----------

pub struct UbisoftAdapter {
    /// Ubisoft Connect 启动器目录（规则库中的 `<root>`）。
    pub launcher_dir: Option<PathBuf>,
    pub entries: Vec<UninstallEntry>,
}

impl UbisoftAdapter {
    /// 启动器目录：`HKLM\SOFTWARE\WOW6432Node\Ubisoft\Launcher\InstallDir`。
    pub fn detect(entries: &[UninstallEntry]) -> Option<Self> {
        let launcher_dir = crate::registry::read_string(
            r"HKLM\SOFTWARE\WOW6432Node\Ubisoft\Launcher",
            "InstallDir",
        )
        .map(PathBuf::from);
        let a = Self {
            launcher_dir,
            entries: entries.to_vec(),
        };
        (a.launcher_dir.is_some() || !a.list_games().is_empty()).then_some(a)
    }
}

impl PlatformAdapter for UbisoftAdapter {
    fn id(&self) -> &'static str {
        "ubisoft"
    }

    fn store(&self) -> &'static str {
        "uplay"
    }

    fn list_games(&self) -> Vec<InstalledGame> {
        sorted(
            self.entries
                .iter()
                .filter_map(|e| {
                    let id = e.key.strip_prefix("Uplay Install ")?.trim();
                    let dir = e.install_dir.clone().filter(|d| d.is_dir())?;
                    Some(installed(
                        "ubisoft",
                        id.to_string(),
                        e.name.clone(),
                        dir,
                        self.launcher_dir.clone(),
                    ))
                })
                .collect(),
        )
    }
}

// ---------- EA app ----------

pub struct EaAdapter {
    pub entries: Vec<UninstallEntry>,
}

impl EaAdapter {
    fn is_ea(e: &UninstallEntry) -> bool {
        let u = e.uninstall.to_lowercase();
        u.contains("eainstaller") || u.contains("ea desktop") || u.contains("origin\\")
    }
}

impl PlatformAdapter for EaAdapter {
    fn id(&self) -> &'static str {
        "ea"
    }

    fn store(&self) -> &'static str {
        "ea"
    }

    fn list_games(&self) -> Vec<InstalledGame> {
        sorted(
            self.entries
                .iter()
                .filter(|e| Self::is_ea(e))
                .filter_map(|e| {
                    let dir = e.install_dir.clone().filter(|d| d.is_dir())?;
                    Some(installed("ea", id_of(&e.key), e.name.clone(), dir, None))
                })
                .collect(),
        )
    }
}

// ---------- Battle.net ----------

pub struct BattleNetAdapter {
    pub entries: Vec<UninstallEntry>,
}

impl BattleNetAdapter {
    fn is_battlenet(e: &UninstallEntry) -> bool {
        let u = e.uninstall.to_lowercase();
        // 启动器自己的登记（键名 Battle.net，显示名会本地化，如“暴雪战网”）不是游戏。
        let launcher = e.key.eq_ignore_ascii_case("Battle.net") || Self::product(e) == "battle.net";
        !launcher && (u.contains("battle.net") || u.contains("blizzard uninstaller"))
    }

    /// 卸载命令中的 `--uid=<产品>`；没有时用键名。
    fn product(e: &UninstallEntry) -> String {
        e.uninstall
            .split_whitespace()
            .find_map(|a| a.trim_matches('"').strip_prefix("--uid="))
            .map(id_of)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| id_of(&e.key))
    }
}

impl PlatformAdapter for BattleNetAdapter {
    fn id(&self) -> &'static str {
        "battlenet"
    }

    fn store(&self) -> &'static str {
        "other"
    }

    fn list_games(&self) -> Vec<InstalledGame> {
        sorted(
            self.entries
                .iter()
                .filter(|e| Self::is_battlenet(e))
                .filter_map(|e| {
                    let dir = e.install_dir.clone().filter(|d| d.is_dir())?;
                    Some(installed(
                        "battlenet",
                        Self::product(e),
                        e.name.clone(),
                        dir,
                        None,
                    ))
                })
                .collect(),
        )
    }
}

// ---------- Xbox / Microsoft Store ----------

pub struct XboxAdapter {
    /// `%LOCALAPPDATA%\Packages`
    pub packages_dir: PathBuf,
}

impl XboxAdapter {
    pub fn detect(local_app_data: Option<&Path>) -> Option<Self> {
        let dir = local_app_data?.join("Packages");
        let a = Self { packages_dir: dir };
        (!a.list_games().is_empty()).then_some(a)
    }

    /// 包名（如 `Microsoft.SunriseBaseGame_8wekyb3d8bbwe`）中的应用名，作为规则库没有时的显示名称。
    pub fn display_name(pfn: &str) -> String {
        let app = pfn.split('_').next().unwrap_or(pfn);
        let app = app.rsplit('.').next().unwrap_or(app);
        // 驼峰拆成单词：SunriseBaseGame → Sunrise Base Game。
        let mut out = String::new();
        let mut prev_lower = false;
        for c in app.chars() {
            if c.is_uppercase() && prev_lower {
                out.push(' ');
            }
            prev_lower = c.is_lowercase() || c.is_ascii_digit();
            out.push(c);
        }
        out
    }
}

impl PlatformAdapter for XboxAdapter {
    fn id(&self) -> &'static str {
        "xbox"
    }

    fn store(&self) -> &'static str {
        "microsoft"
    }

    /// 有 `SystemAppData\wgs` 容器存档的应用包（游戏）。
    fn list_games(&self) -> Vec<InstalledGame> {
        let mut games = Vec::new();
        for e in std::fs::read_dir(&self.packages_dir)
            .into_iter()
            .flatten()
            .flatten()
        {
            let wgs = e.path().join("SystemAppData").join("wgs");
            if !wgs.is_dir() {
                continue;
            }
            let pfn = e.file_name().to_string_lossy().into_owned();
            games.push(InstalledGame {
                platform: "xbox".into(),
                name: Self::display_name(&pfn),
                app_id: pfn,
                install_dir: None,
                library_root: None,
                install_folder: None,
                last_played: None,
                playtime_min: None,
                cloud_save: true,
            });
        }
        sorted(games)
    }
}

/// 本机的这几个平台（跳过被关闭的）。
pub fn detect(disabled: &[String]) -> Vec<Box<dyn PlatformAdapter>> {
    let on = |id: &str| !disabled.iter().any(|d| d.eq_ignore_ascii_case(id));
    let mut v: Vec<Box<dyn PlatformAdapter>> = Vec::new();
    let entries = if on("ubisoft") || on("ea") || on("battlenet") {
        uninstall_entries()
    } else {
        Vec::new()
    };
    v.extend(with_entries(
        &entries,
        &on,
        UbisoftAdapter::detect(&entries),
    ));
    if on("xbox")
        && let Some(a) = XboxAdapter::detect(crate::KnownDirs::system().local_app_data.as_deref())
    {
        v.push(Box::new(a));
    }
    v
}

/// 伪造根目录下的这几个平台（测试用）：登记取自 `Registry/uninstall.json`，
/// Ubisoft 启动器目录为 `<root>/Ubisoft Game Launcher`，应用包在伪造的 `%LOCALAPPDATA%\Packages`。
pub fn fake(root: &Path, disabled: &[String]) -> Vec<Box<dyn PlatformAdapter>> {
    let on = |id: &str| !disabled.iter().any(|d| d.eq_ignore_ascii_case(id));
    let entries = fake_uninstall_entries(root);
    let launcher = root.join("Ubisoft Game Launcher");
    let ubi = UbisoftAdapter {
        launcher_dir: launcher.is_dir().then_some(launcher),
        entries: entries.clone(),
    };
    let ubi = (ubi.launcher_dir.is_some() || !ubi.list_games().is_empty()).then_some(ubi);
    let mut v = with_entries(&entries, &on, ubi);
    let local = crate::KnownDirs::under(root).local_app_data;
    if on("xbox")
        && let Some(a) = XboxAdapter::detect(local.as_deref())
    {
        v.push(Box::new(a));
    }
    v
}

fn with_entries(
    entries: &[UninstallEntry],
    on: &dyn Fn(&str) -> bool,
    ubisoft: Option<UbisoftAdapter>,
) -> Vec<Box<dyn PlatformAdapter>> {
    let mut v: Vec<Box<dyn PlatformAdapter>> = Vec::new();
    if on("ubisoft")
        && let Some(a) = ubisoft
    {
        v.push(Box::new(a));
    }
    let ea = EaAdapter {
        entries: entries.to_vec(),
    };
    if on("ea") && !ea.list_games().is_empty() {
        v.push(Box::new(ea));
    }
    let bn = BattleNetAdapter {
        entries: entries.to_vec(),
    };
    if on("battlenet") && !bn.list_games().is_empty() {
        v.push(Box::new(bn));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, name: &str, dir: Option<&Path>, uninstall: &str) -> UninstallEntry {
        UninstallEntry {
            key: key.into(),
            name: name.into(),
            install_dir: dir.map(Path::to_path_buf),
            uninstall: uninstall.into(),
            publisher: String::new(),
        }
    }

    #[test]
    fn uninstall_entries_split_by_store() {
        let d = tempfile::tempdir().unwrap();
        let mk = |rel: &str| {
            let p = d.path().join(rel);
            std::fs::create_dir_all(&p).unwrap();
            p
        };
        let ac = mk("Ubisoft/Assassin's Creed Odyssey");
        let fifa = mk("EA Games/Mass Effect Legendary Edition");
        let d4 = mk("Battle.net Games/Diablo IV");
        let entries = vec![
            entry(
                "Uplay Install 5059",
                "Assassin's Creed Odyssey",
                Some(&ac),
                r#""C:\Program Files (x86)\Ubisoft\Ubisoft Game Launcher\Uplay.exe" uplay://uninstall/5059"#,
            ),
            entry(
                "Uplay Install 1",
                "Gone",
                Some(&d.path().join("missing")),
                "",
            ),
            entry(
                "{GUID-ME}",
                "Mass Effect™ Legendary Edition",
                Some(&fifa),
                r#""C:\Program Files\Common Files\EAInstaller\Mass Effect Legendary Edition\Cleanup.exe" uninstall_game -autologging"#,
            ),
            entry(
                "Diablo IV",
                "Diablo IV",
                Some(&d4),
                r#""C:\Program Files (x86)\Battle.net\Battle.net.exe" --uid=fenris_enus --productcode=Fen"#,
            ),
            entry(
                "Battle.net",
                "暴雪战网",
                Some(&d4),
                r#""C:\Program Files (x86)\Battle.net\Battle.net Uninstaller.exe""#,
            ),
            entry(
                "Battle.net Beta",
                "暴雪战网",
                Some(&d4),
                r#""C:\Program Files (x86)\Battle.net\Battle.net.exe" --uid=battle.net"#,
            ),
            entry(
                "7-Zip",
                "7-Zip",
                Some(&ac),
                r"C:\Program Files\7-Zip\Uninstall.exe",
            ),
        ];
        let ubi = UbisoftAdapter {
            launcher_dir: Some(d.path().join("Ubisoft Game Launcher")),
            entries: entries.clone(),
        }
        .list_games();
        assert_eq!(ubi.len(), 1, "安装目录不存在的跳过");
        assert_eq!(ubi[0].game_id(), "ubisoft-5059");
        assert_eq!(
            ubi[0].library_root.as_deref(),
            Some(d.path().join("Ubisoft Game Launcher").as_path()),
            "<root> 是启动器目录"
        );
        let ea = EaAdapter {
            entries: entries.clone(),
        }
        .list_games();
        assert_eq!(ea.len(), 1);
        assert_eq!(ea[0].game_id(), "ea-guid-me");
        assert_eq!(
            ea[0].install_folder.as_deref(),
            Some("Mass Effect Legendary Edition")
        );
        let bn = BattleNetAdapter { entries }.list_games();
        assert_eq!(bn.len(), 1, "启动器本身不是游戏");
        assert_eq!(bn[0].game_id(), "battlenet-fenris_enus");
        assert_eq!(bn[0].name, "Diablo IV");
    }

    #[test]
    fn xbox_lists_packages_with_wgs() {
        let d = tempfile::tempdir().unwrap();
        let pk = d.path().join("Packages");
        std::fs::create_dir_all(
            pk.join("Microsoft.SunriseBaseGame_8wekyb3d8bbwe/SystemAppData/wgs/000901F"),
        )
        .unwrap();
        std::fs::create_dir_all(pk.join("Microsoft.WindowsCalculator_8wekyb3d8bbwe/LocalState"))
            .unwrap();
        let games = XboxAdapter { packages_dir: pk }.list_games();
        assert_eq!(games.len(), 1);
        assert_eq!(
            games[0].game_id(),
            "xbox-Microsoft.SunriseBaseGame_8wekyb3d8bbwe"
        );
        assert_eq!(games[0].name, "Sunrise Base Game");
        assert!(XboxAdapter::detect(Some(&d.path().join("nothing"))).is_none());
    }

    #[test]
    fn fake_root_reads_uninstall_json() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("Games/Diablo IV");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(d.path().join("Registry")).unwrap();
        std::fs::write(
            d.path().join("Registry/uninstall.json"),
            serde_json::json!([{ "key": "Diablo IV", "name": "Diablo IV", "installDir": dir, "uninstall": "Battle.net.exe --uid=fenris" }]).to_string(),
        )
        .unwrap();
        let v = fake(d.path(), &[]);
        let ids: Vec<&str> = v.iter().map(|a| a.id()).collect();
        assert_eq!(ids, ["battlenet"]);
        assert!(fake(d.path(), &["battlenet".into()]).is_empty());
    }
}
