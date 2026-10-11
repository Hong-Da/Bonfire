//! 游戏截图来源（只读）：Steam 截图、Windows 游戏栏 / Xbox 应用的“捕获”文件夹、用户为游戏指定的文件夹。
//!
//! - Steam：`<Steam>\userdata\<账号>\760\remote\<AppID>\screenshots\*.jpg`，按 AppID 精确对应；
//!   文件名 `YYYYMMDDhhmmss_n.jpg` 即拍摄时间。`thumbnails\` 是 Steam 自己的缩略图，不收集。
//! - 捕获文件夹：文件名为“窗口标题 + 时间”，如 `Minishoot 2026_7_18 0_02_36.png`，按标题与游戏名对应；
//!   视频（mp4）不收集。位置可被用户在系统中修改，由调用方读取实际位置。
//! - 手动文件夹：其中（含子文件夹）的全部图片都属于该游戏。

use std::path::{Path, PathBuf};

use crate::manifest::normalize;

/// 本地时间：年、月、日、时、分、秒。
pub type LocalTime = (i16, i8, i8, i8, i8, i8);

/// 截图来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShotSource {
    Steam,
    Capture,
    Folder,
}

impl ShotSource {
    pub fn as_str(self) -> &'static str {
        match self {
            ShotSource::Steam => "steam",
            ShotSource::Capture => "capture",
            ShotSource::Folder => "folder",
        }
    }
}

/// 磁盘上的一张截图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShotFile {
    pub path: PathBuf,
    pub size: u64,
    /// 文件修改时间（纳秒）。
    pub mtime_ns: i64,
    /// 从文件名解析出的拍摄时间（本地时间：年、月、日、时、分、秒）；解析不出时为 None，由调用方用修改时间。
    pub taken_local: Option<LocalTime>,
    pub source: ShotSource,
}

const IMAGE_EXTS: [&str; 5] = ["jpg", "jpeg", "png", "webp", "bmp"];

pub fn is_image(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

fn shot_file(path: PathBuf, source: ShotSource) -> Option<ShotFile> {
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0);
    let name = path.file_stem()?.to_string_lossy().into_owned();
    let taken_local = match source {
        ShotSource::Steam => parse_steam_time(&name),
        ShotSource::Capture => parse_capture_name(&name).map(|(_, t)| t),
        ShotSource::Folder => {
            parse_steam_time(&name).or_else(|| parse_capture_name(&name).map(|(_, t)| t))
        }
    };
    Some(ShotFile {
        size: meta.len(),
        path,
        mtime_ns,
        taken_local,
        source,
    })
}

/// Steam 截图：所有账号下该 AppID 的截图。
pub fn steam_shots(steam_root: &Path, app_id: &str) -> Vec<ShotFile> {
    let mut out = Vec::new();
    let Ok(users) = std::fs::read_dir(steam_root.join("userdata")) else {
        return out;
    };
    for u in users.flatten() {
        let dir = u.path().join("760/remote").join(app_id).join("screenshots");
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if is_image(&p)
                && let Some(f) = shot_file(p, ShotSource::Steam)
            {
                out.push(f);
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// 捕获文件夹中标题与游戏名对应的截图（不进入子文件夹）。`names` 为游戏的各种名称（英文名、中文名等）。
pub fn capture_shots(dir: &Path, names: &[&str], others: &[&str]) -> Vec<ShotFile> {
    let wanted = wanted_names(names);
    let others = wanted_names(others);
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if !is_image(&p) {
            continue;
        }
        let stem = p
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Some((title, _)) = parse_capture_name(&stem) else {
            continue;
        };
        // 同一个标题可能对应多个游戏（“Hades” 与 “Hades II”）：只归到对应得最好的游戏。
        let own = best_score(&title, &wanted);
        if own > 0
            && own >= best_score(&title, &others)
            && let Some(f) = shot_file(p, ShotSource::Capture)
        {
            out.push(f);
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Steam 截图中出现的全部 AppID：(AppID, 截图数, 截图文件夹)，多个账号的合并。
pub fn steam_shot_apps(steam_root: &Path) -> Vec<(String, usize, PathBuf)> {
    let mut found: std::collections::BTreeMap<String, (usize, PathBuf)> = Default::default();
    let Ok(users) = std::fs::read_dir(steam_root.join("userdata")) else {
        return Vec::new();
    };
    for u in users.flatten() {
        let Ok(apps) = std::fs::read_dir(u.path().join("760/remote")) else {
            continue;
        };
        for a in apps.flatten() {
            let id = a.file_name().to_string_lossy().into_owned();
            if !id.chars().all(|c| c.is_ascii_digit()) {
                continue;
            }
            let dir = a.path().join("screenshots");
            let Ok(rd) = std::fs::read_dir(&dir) else {
                continue;
            };
            let n = rd.flatten().filter(|e| is_image(&e.path())).count();
            if n > 0 {
                let e = found.entry(id).or_insert((0, dir));
                e.0 += n;
            }
        }
    }
    found.into_iter().map(|(id, (n, d))| (id, n, d)).collect()
}

/// 捕获文件夹中与 `names`（游戏库中的游戏名）都不对应的截图，按窗口标题分组：(标题, 截图数)。
pub fn capture_titles(dir: &Path, names: &[&str]) -> Vec<(String, usize)> {
    let wanted = wanted_names(names);
    let mut groups: std::collections::BTreeMap<String, (String, usize)> = Default::default();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    for e in rd.flatten() {
        let p = e.path();
        if !is_image(&p) {
            continue;
        }
        let stem = p
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Some((title, _)) = parse_capture_name(&stem) else {
            continue;
        };
        let key = normalize(&title);
        if key.is_empty() || best_score(&title, &wanted) > 0 {
            continue;
        }
        groups.entry(key).or_insert((title, 0)).1 += 1;
    }
    groups.into_values().collect()
}

fn wanted_names(names: &[&str]) -> Vec<String> {
    names
        .iter()
        .map(|n| normalize(n))
        .filter(|n| n.chars().count() >= 2)
        .collect()
}

/// 窗口标题与游戏名的对应程度（越大越好，0 为不对应）：
/// 3 规整后相同；2 标题以游戏名开头（标题带副标题、版本号）；
/// 1 游戏名以标题开头（游戏名带版本后缀，如标题 “Grand Theft Auto V”、游戏名 “Grand Theft Auto V Enhanced”），
/// 标题至少 4 个字符，避免过短的标题误配。
fn best_score(title: &str, wanted: &[String]) -> u8 {
    let t = normalize(title);
    if t.is_empty() {
        return 0;
    }
    wanted
        .iter()
        .map(|w| {
            if t == *w {
                3
            } else if t.starts_with(w.as_str()) {
                2
            } else if t.chars().count() >= 4 && w.starts_with(t.as_str()) {
                1
            } else {
                0
            }
        })
        .max()
        .unwrap_or(0)
}

/// 手动文件夹（含子文件夹）中的全部图片。
pub fn folder_shots(dir: &Path) -> Vec<ShotFile> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if is_image(&p)
                && let Some(f) = shot_file(p, ShotSource::Folder)
            {
                out.push(f);
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Steam 文件名 `20261009123456_1` → 2026-10-09 12:34:56。
fn parse_steam_time(stem: &str) -> Option<LocalTime> {
    let digits: String = stem.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() != 14 {
        return None;
    }
    let n = |a: usize, b: usize| digits[a..b].parse::<i32>().ok();
    valid((
        n(0, 4)?,
        n(4, 6)?,
        n(6, 8)?,
        n(8, 10)?,
        n(10, 12)?,
        n(12, 14)?,
    ))
}

/// 游戏栏文件名 `Minishoot 2026_7_18 0_02_36` → (标题, 时间)。
fn parse_capture_name(stem: &str) -> Option<(String, LocalTime)> {
    let parts: Vec<&str> = stem.rsplitn(3, ' ').collect();
    if parts.len() != 3 {
        return None;
    }
    let (time, date, title) = (parts[0], parts[1], parts[2]);
    let d: Vec<i32> = date.split('_').filter_map(|x| x.parse().ok()).collect();
    let t: Vec<i32> = time.split('_').filter_map(|x| x.parse().ok()).collect();
    if d.len() != 3 || t.len() != 3 || title.trim().is_empty() {
        return None;
    }
    let when = valid((d[0], d[1], d[2], t[0], t[1], t[2]))?;
    Some((title.trim().to_string(), when))
}

fn valid(v: (i32, i32, i32, i32, i32, i32)) -> Option<LocalTime> {
    let (y, mo, d, h, mi, s) = v;
    ((1990..=2200).contains(&y)
        && (1..=12).contains(&mo)
        && (1..=31).contains(&d)
        && (0..24).contains(&h)
        && (0..60).contains(&mi)
        && (0..61).contains(&s))
    .then_some((y as i16, mo as i8, d as i8, h as i8, mi as i8, s as i8))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"img").unwrap();
    }

    #[test]
    fn parses_file_names() {
        assert_eq!(
            parse_steam_time("20261009123456_1"),
            Some((2026, 10, 9, 12, 34, 56))
        );
        assert_eq!(parse_steam_time("2026100912345_1"), None);
        assert_eq!(
            parse_capture_name("Minishoot 2026_7_18 0_02_36"),
            Some(("Minishoot".into(), (2026, 7, 18, 0, 2, 36)))
        );
        assert_eq!(
            parse_capture_name("Hollow Knight Silksong 2026_10_9 21_5_0").map(|x| x.0),
            Some("Hollow Knight Silksong".into())
        );
        assert_eq!(parse_capture_name("random picture"), None);
    }

    #[test]
    fn finds_steam_capture_and_folder_shots() {
        let d = tempfile::tempdir().unwrap();
        let steam = d.path().join("Steam");
        let shots = steam.join("userdata/111/760/remote/367520/screenshots");
        touch(&shots.join("20261009123456_1.jpg"));
        touch(&shots.join("thumbnails/20261009123456_1.jpg"));
        touch(&steam.join("userdata/222/760/remote/367520/screenshots/20261010080000_1.jpg"));
        touch(&steam.join("userdata/111/760/remote/999/screenshots/20261009123456_1.jpg"));
        let s = steam_shots(&steam, "367520");
        assert_eq!(s.len(), 2);
        assert!(
            s.iter()
                .all(|f| f.source == ShotSource::Steam && f.taken_local.is_some())
        );

        let cap = d.path().join("Captures");
        touch(&cap.join("Hollow Knight 2026_10_9 21_05_00.png"));
        touch(&cap.join("Hollow Knight 2026_10_9 21_06_00.mp4"));
        touch(&cap.join("Minishoot 2026_7_18 0_02_36.png"));
        touch(&cap.join("空洞骑士 2026_10_9 22_00_00.png"));
        let c = capture_shots(&cap, &["Hollow Knight", "空洞骑士"], &[]);
        let names: Vec<String> = c
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "Hollow Knight 2026_10_9 21_05_00.png",
                "空洞骑士 2026_10_9 22_00_00.png"
            ]
        );

        let own = d.path().join("MyShots");
        touch(&own.join("a.png"));
        touch(&own.join("sub/b.JPG"));
        touch(&own.join("notes.txt"));
        assert_eq!(folder_shots(&own).len(), 2);
        assert!(folder_shots(&d.path().join("missing")).is_empty());

        // 不在游戏库中的：Steam 按 AppID，游戏栏按标题（与游戏名对应的除外）。
        let apps: Vec<(String, usize)> = steam_shot_apps(&steam)
            .into_iter()
            .map(|(id, n, _)| (id, n))
            .collect();
        assert_eq!(apps, [("367520".into(), 2), ("999".into(), 1)]);
        assert_eq!(
            capture_titles(&cap, &["Hollow Knight", "空洞骑士"]),
            [("Minishoot".to_string(), 1)]
        );
    }

    #[test]
    fn capture_title_shorter_than_game_name() {
        let d = tempfile::tempdir().unwrap();
        let cap = d.path().join("Captures");
        touch(&cap.join("Grand Theft Auto V 2026_7_18 11_23_09.png"));
        touch(&cap.join("Hades 2026_7_18 11_23_09.png"));
        touch(&cap.join("Hades II 2026_7_19 11_23_09.png"));
        touch(&cap.join("Ha 2026_7_19 11_23_09.png"));
        let names = |v: Vec<ShotFile>| -> Vec<String> {
            v.iter()
                .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
                .collect()
        };
        // 游戏名带后缀：标题较短也对应。
        assert_eq!(
            names(capture_shots(&cap, &["Grand Theft Auto V Enhanced"], &["Hades"])),
            ["Grand Theft Auto V 2026_7_18 11_23_09.png"]
        );
        // 两个游戏都在库中：各自只拿到对应最好的截图；过短的标题不误配。
        assert_eq!(
            names(capture_shots(&cap, &["Hades"], &["Hades II"])),
            ["Hades 2026_7_18 11_23_09.png"]
        );
        assert_eq!(
            names(capture_shots(&cap, &["Hades II"], &["Hades"])),
            ["Hades II 2026_7_19 11_23_09.png"]
        );
        // 发现页：已对应游戏库中游戏的标题不再列出。
        assert_eq!(
            capture_titles(&cap, &["Grand Theft Auto V Enhanced", "Hades", "Hades II"]),
            [("Ha".to_string(), 1)]
        );
    }
}
