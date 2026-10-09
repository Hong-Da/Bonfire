//! 把已安装的游戏匹配到规则，解析出磁盘上实际存在的存档位置；规则库没有的游戏用启发式探测。

use std::path::PathBuf;

use serde::Serialize;

use crate::manifest::{GameRule, RuleIndex, RuleSource, normalize};
use crate::paths::{KnownDirs, PathContext, expand, find};
use crate::platforms::InstalledGame;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FoundKind {
    File,
    Dir,
    /// 规则中的路径模板（已展开变量的通配模式），每次扫描重新匹配。
    Glob,
    Registry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LocationSource {
    Manifest,
    User,
    Heuristic,
}

impl LocationSource {
    pub fn as_str(self) -> &'static str {
        match self {
            LocationSource::Manifest => "manifest",
            LocationSource::User => "user",
            LocationSource::Heuristic => "heuristic",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FoundLocation {
    /// 规则中的路径模板（启发式结果为实际路径）。
    pub template: String,
    /// 实际路径；注册表为 `HKCU\...` 形式。
    pub path: String,
    pub kind: FoundKind,
    /// 只标了 config 的规则。
    pub config: bool,
    pub source: LocationSource,
    /// 启发式结果需要用户确认后才备份。
    pub confirmed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovered {
    pub game: InstalledGame,
    /// 匹配到的规则名称。
    pub rule: Option<String>,
    pub locations: Vec<FoundLocation>,
    /// 规则库标明该商店支持云存档，或平台本地数据显示已启用。
    pub cloud_save: bool,
}

/// 匹配顺序：平台 ID → 安装文件夹名 → 名称（归一化）。
pub fn match_rule<'a>(idx: &'a RuleIndex, g: &InstalledGame) -> Option<(&'a GameRule, RuleSource)> {
    let id = g.app_id.parse::<u64>().ok();
    let by_id = match (g.platform.as_str(), id) {
        ("steam", Some(id)) => idx.by_steam_id(id),
        ("gog", Some(id)) => idx.by_gog_id(id),
        _ => None,
    };
    by_id
        .or_else(|| {
            g.install_folder
                .as_deref()
                .and_then(|f| idx.by_install_dir(f))
        })
        .or_else(|| idx.by_name(&g.name))
        .or_else(|| idx.by_fuzzy_name(&g.name))
}

pub fn context(dirs: &KnownDirs, g: &InstalledGame) -> PathContext {
    PathContext {
        dirs: dirs.clone(),
        base: g.install_dir.clone(),
        root: g.library_root.clone(),
        game: g.install_folder.clone(),
        store_game_id: Some(g.app_id.clone()),
    }
}

/// 按规则解析存档位置（只返回磁盘上存在的）。
pub fn resolve_rule(
    rule: &GameRule,
    source: RuleSource,
    ctx: &PathContext,
    store: &str,
) -> Vec<FoundLocation> {
    let src = match source {
        RuleSource::Manifest => LocationSource::Manifest,
        RuleSource::User => LocationSource::User,
    };
    // 每条规则一个位置（通配模式），磁盘上至少有一个匹配时才登记；之后每次扫描重新匹配，
    // 游戏新建的存档栏位无需重新扫描平台即被备份。
    let mut out: Vec<FoundLocation> = Vec::new();
    for r in rule.files.iter().filter(|r| r.applies_to(store)) {
        let Some(pattern) = expand(&r.path, ctx) else {
            continue;
        };
        if find(&pattern).is_empty() || out.iter().any(|l| path_key(&l.path) == path_key(&pattern))
        {
            continue;
        }
        out.push(FoundLocation {
            template: r.path.clone(),
            path: pattern,
            kind: FoundKind::Glob,
            config: r.is_config_only(),
            source: src,
            confirmed: true,
        });
    }
    for r in rule.registry.iter().filter(|r| r.applies_to(store)) {
        if let Some(reg) = crate::registry::to_reg_path(&r.path)
            && crate::registry::key_exists(&reg)
        {
            out.push(FoundLocation {
                template: r.path.clone(),
                path: reg,
                kind: FoundKind::Registry,
                config: r.is_config_only(),
                source: src,
                confirmed: true,
            });
        }
    }
    out
}

/// 统一为系统原生的路径分隔符（glob 结果在 Windows 上混用 `/` 与 `\`）。
pub fn native(p: &std::path::Path) -> String {
    let s = p.to_string_lossy();
    if cfg!(windows) {
        s.replace('/', "\\")
    } else {
        s.into_owned()
    }
}

/// 路径比较用的键：不区分大小写与分隔符。
pub fn path_key(s: &str) -> String {
    s.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

/// 去重，并去掉已被上级目录包含的项（同一目录下的文件规则与目录规则同时命中时只保留目录）。
fn dedupe_nested(v: &mut Vec<FoundLocation>) {
    let key = path_key;
    v.sort_by_key(|l| key(&l.path));
    v.dedup_by(|a, b| key(&a.path) == key(&b.path));
    let dirs: Vec<String> = v
        .iter()
        .filter(|l| l.kind == FoundKind::Dir)
        .map(|l| key(&l.path) + "/")
        .collect();
    v.retain(|l| {
        let k = key(&l.path);
        !dirs.iter().any(|d| k.starts_with(d.as_str()))
    });
}

/// 启发式探测：在常见存档目录（及其下一级，如发行商目录）中找名称与游戏名或安装文件夹名一致的目录。
pub fn heuristic(dirs: &KnownDirs, g: &InstalledGame) -> Vec<FoundLocation> {
    let mut names: Vec<String> = vec![normalize(&g.name)];
    if let Some(f) = &g.install_folder {
        names.push(normalize(f));
    }
    names.retain(|n| n.chars().count() >= 4);
    names.dedup();
    if names.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for base in dirs.common_save_dirs() {
        let Ok(rd) = std::fs::read_dir(&base) else {
            continue;
        };
        for e in rd.flatten().filter(|e| e.path().is_dir()) {
            let mut candidates = vec![e.path()];
            // 发行商目录下的一级子目录（如 LocalLow/Team Cherry/Hollow Knight）。
            if let Ok(sub) = std::fs::read_dir(e.path()) {
                candidates.extend(sub.flatten().map(|s| s.path()).filter(|p| p.is_dir()));
            }
            for c in candidates {
                let n = c
                    .file_name()
                    .map(|n| normalize(&n.to_string_lossy()))
                    .unwrap_or_default();
                let inside_install = g.install_dir.as_ref().is_some_and(|i| c.starts_with(i));
                if names.contains(&n) && !inside_install {
                    let path = native(&c);
                    out.push(FoundLocation {
                        template: path.clone(),
                        path,
                        kind: FoundKind::Dir,
                        config: false,
                        source: LocationSource::Heuristic,
                        confirmed: false,
                    });
                }
            }
        }
    }
    dedupe_nested(&mut out);
    out
}

/// 解析一个已安装游戏：有规则按规则；没有规则时启发式探测。
pub fn discover(idx: &RuleIndex, dirs: &KnownDirs, store: &str, g: &InstalledGame) -> Discovered {
    let ctx = context(dirs, g);
    let (rule, locations, rule_cloud) = match match_rule(idx, g) {
        Some((rule, source)) => (
            Some(rule.name.clone()),
            resolve_rule(rule, source, &ctx, store),
            rule.cloud.iter().any(|c| c == store),
        ),
        None => (None, heuristic(dirs, g), false),
    };
    Discovered {
        game: g.clone(),
        rule,
        locations,
        cloud_save: g.cloud_save || rule_cloud,
    }
}

/// 手动添加的游戏没有平台信息时，也可以按名称套用规则库（例如独立游戏、模拟器）。
pub fn discover_by_name(
    idx: &RuleIndex,
    dirs: &KnownDirs,
    name: &str,
) -> Option<(String, Vec<FoundLocation>)> {
    let (rule, source) = idx.by_name(name).or_else(|| idx.by_fuzzy_name(name))?;
    let ctx = PathContext {
        dirs: dirs.clone(),
        ..Default::default()
    };
    Some((rule.name.clone(), resolve_rule(rule, source, &ctx, "other")))
}

/// 已知存档目录的路径（用于测试与诊断）。
pub fn as_paths(v: &[FoundLocation]) -> Vec<PathBuf> {
    v.iter().map(|l| PathBuf::from(&l.path)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Manifest, tests::SAMPLE};
    use crate::platforms::PlatformAdapter;
    use crate::platforms::tests::{fake_steam, write};

    fn index() -> RuleIndex {
        RuleIndex::new(Manifest::from_yaml(SAMPLE).unwrap(), Manifest::default())
    }

    #[test]
    fn steam_games_resolve_by_app_id() {
        let d = tempfile::tempdir().unwrap();
        let steam = fake_steam(d.path());
        let dirs = KnownDirs::under(d.path());
        let low = dirs.local_app_data_low.clone().unwrap();
        write(&low.join("Team Cherry/Hollow Knight/user1.dat"), "save");
        write(&low.join("Team Cherry/Hollow Knight/user2.dat"), "save");
        write(&low.join("Team Cherry/Hollow Knight/shader.cache"), "x");
        let roaming = dirs.app_data.clone().unwrap();
        write(
            &roaming.join("EldenRing/76561198000000000/ER0000.sl2"),
            "er",
        );
        write(&roaming.join("EldenRing/GraphicsConfig.xml"), "<x/>");

        let games = steam.list_games();
        let found: Vec<Discovered> = games
            .iter()
            .map(|g| discover(&index(), &dirs, "steam", g))
            .collect();
        let er = &found[0];
        assert_eq!(er.rule.as_deref(), Some("Elden Ring"));
        assert_eq!(er.locations.len(), 2, "{:#?}", er.locations);
        assert!(
            er.locations
                .iter()
                .any(|l| l.config && l.path.ends_with("GraphicsConfig.xml"))
        );
        assert!(
            er.locations
                .iter()
                .any(|l| !l.config && l.path.ends_with("EldenRing/*/*.sl2")),
            "<storeUserId> 匹配任意账号"
        );
        let hk = &found[1];
        assert_eq!(hk.locations.len(), 1, "每条规则一个位置");
        assert!(
            hk.locations
                .iter()
                .all(|l| l.source == LocationSource::Manifest
                    && l.confirmed
                    && l.kind == FoundKind::Glob)
        );
        assert!(hk.cloud_save);
    }

    #[test]
    fn store_specific_rules_are_skipped_for_other_stores() {
        let d = tempfile::tempdir().unwrap();
        let dirs = KnownDirs::under(d.path());
        write(
            &dirs
                .app_data
                .clone()
                .unwrap()
                .join("EldenRing/GraphicsConfig.xml"),
            "x",
        );
        let g = InstalledGame {
            platform: "epic".into(),
            app_id: "x".into(),
            name: "Elden Ring".into(),
            install_dir: None,
            library_root: None,
            install_folder: None,
            last_played: None,
            playtime_min: None,
            cloud_save: false,
        };
        let found = discover(&index(), &dirs, "epic", &g);
        assert_eq!(found.rule.as_deref(), Some("Elden Ring"), "按名称匹配");
        assert!(found.locations.is_empty(), "该配置文件规则只适用于 Steam");
    }

    #[test]
    fn heuristic_finds_publisher_subfolder_unconfirmed() {
        let d = tempfile::tempdir().unwrap();
        let dirs = KnownDirs::under(d.path());
        write(
            &dirs
                .local_app_data_low
                .clone()
                .unwrap()
                .join("Some Studio/Indie Quest/save.json"),
            "{}",
        );
        write(
            &dirs
                .documents
                .clone()
                .unwrap()
                .join("My Games/Indie-Quest/x.sav"),
            "x",
        );
        let g = InstalledGame {
            platform: "gog".into(),
            app_id: "99".into(),
            name: "Indie Quest™".into(),
            install_dir: Some(d.path().join("GOG/Indie Quest")),
            library_root: None,
            install_folder: Some("Indie Quest".into()),
            last_played: None,
            playtime_min: None,
            cloud_save: false,
        };
        let found = discover(&index(), &dirs, "gog", &g);
        assert!(found.rule.is_none());
        assert_eq!(found.locations.len(), 2, "{:#?}", found.locations);
        assert!(
            found
                .locations
                .iter()
                .all(|l| l.source == LocationSource::Heuristic && !l.confirmed)
        );
    }

    #[test]
    fn nested_matches_keep_outer_dir() {
        let mut v = vec![
            FoundLocation {
                template: "a".into(),
                path: "C:/x/save".into(),
                kind: FoundKind::Dir,
                config: false,
                source: LocationSource::Manifest,
                confirmed: true,
            },
            FoundLocation {
                template: "b".into(),
                path: "C:/x/save/1.dat".into(),
                kind: FoundKind::File,
                config: false,
                source: LocationSource::Manifest,
                confirmed: true,
            },
            FoundLocation {
                template: "c".into(),
                path: "c:\\x\\SAVE".into(),
                kind: FoundKind::Dir,
                config: false,
                source: LocationSource::Manifest,
                confirmed: true,
            },
        ];
        dedupe_nested(&mut v);
        assert_eq!(v.len(), 1);
    }

    #[test]
    #[cfg(windows)]
    fn registry_rule_found_when_key_exists() {
        let key = r"HKCU\Software\BonfireTest\RegistryGame";
        let ok = |args: &[&str]| {
            assert!(
                std::process::Command::new("reg")
                    .args(args)
                    .output()
                    .unwrap()
                    .status
                    .success()
            )
        };
        ok(&["add", key, "/v", "Slot", "/t", "REG_SZ", "/d", "1", "/f"]);
        let d = tempfile::tempdir().unwrap();
        let g = InstalledGame {
            platform: "gog".into(),
            app_id: "5".into(),
            name: "Whatever".into(),
            install_dir: None,
            library_root: None,
            install_folder: Some("RegGame".into()),
            last_played: None,
            playtime_min: None,
            cloud_save: false,
        };
        let found = discover(&index(), &KnownDirs::under(d.path()), "gog", &g);
        ok(&["delete", key, "/f"]);
        assert_eq!(found.locations.len(), 1);
        assert_eq!(found.locations[0].kind, FoundKind::Registry);
        assert_eq!(found.locations[0].path, key);
    }

    #[test]
    fn steam_adapter_trait_object() {
        let d = tempfile::tempdir().unwrap();
        let a: Box<dyn PlatformAdapter> = Box::new(fake_steam(d.path()));
        assert_eq!((a.id(), a.store()), ("steam", "steam"));
    }
}
