//! 篝火的标识（U-47）：导出文件统一以 `Bonfire-` 开头命名，附带 `readme.txt` 说明，
//! 包内说明文件写上 `generator`（如 `Bonfire 0.1.0`）；导入时据此识别是篝火导出的文件。

/// 应用名称。
pub const APP_NAME: &str = "篝火 Bonfire";
/// 导出文件名的前缀。
pub const FILE_PREFIX: &str = "Bonfire-";
/// 导出中附带的说明文件名。
pub const README_NAME: &str = "readme.txt";
/// readme.txt 中的识别标记（导入时据此判断是篝火导出的文件）。
pub const README_MARKER: &str = "[Bonfire Export]";
/// 代码仓库地址。
pub const REPOS: [(&str, &str); 2] = [
    ("Gitee", "https://gitee.com/Hong_Xiao_Feng/bonfire.git"),
    ("GitHub", "https://github.com/Hong-Da/Bonfire.git"),
];

/// 写入包内说明文件的生成者，如 `Bonfire 0.1.0`。
pub fn generator() -> String {
    format!("Bonfire {}", crate::VERSION)
}

/// 说明文件（`bonfire-export.json` 等）的 `generator` 是否表明由篝火生成。
pub fn is_bonfire_generator(g: Option<&str>) -> bool {
    g.is_some_and(|g| g.starts_with("Bonfire"))
}

/// readme.txt 的文字是否为篝火导出时写的。
pub fn is_bonfire_readme(text: &str) -> bool {
    text.contains(README_MARKER)
}

/// 导出的内容，用来写 readme.txt。
pub enum ReadmeKind<'a> {
    /// 存档文件（zip 或文件夹）：每个子文件夹对应一个存档位置（文件夹名, 原来的路径）。
    Files {
        game: &'a str,
        snapshot: Option<&'a str>,
        comment: Option<&'a str>,
        locations: Vec<(String, String)>,
    },
    /// 篝火包：(游戏名, 快照数, 截图数)。
    Package { games: Vec<(String, usize, usize)> },
    /// 截图。
    Shots { game: &'a str, count: usize },
    /// zip 格式的篝火导出（U-49）：(文件夹, 游戏名, 有没有存档, 截图数)；文件夹为空表示在根目录。
    /// `split` 时每个游戏的存档与截图分在“存档 / 截图”两个文件夹中。
    Zip {
        games: Vec<(String, String, bool, usize)>,
        split: bool,
    },
}

/// 生成 readme.txt：中英双语，UTF-8（带 BOM，记事本也能正确显示）、Windows 换行。
pub fn readme(kind: &ReadmeKind<'_>, exported_at: &str) -> Vec<u8> {
    let mut zh = Vec::new();
    let mut en = Vec::new();
    match kind {
        ReadmeKind::Files {
            game,
            snapshot,
            comment,
            locations,
        } => {
            zh.push(format!("这是用篝火导出的《{game}》存档文件。"));
            en.push(format!("Save files of \"{game}\" exported by Bonfire."));
            if let Some(s) = snapshot {
                zh.push(format!(
                    "快照：{s}{}",
                    comment.map(|c| format!("（{c}）")).unwrap_or_default()
                ));
                en.push(format!(
                    "Snapshot: {s}{}",
                    comment.map(|c| format!(" ({c})")).unwrap_or_default()
                ));
            } else {
                zh.push("内容：导出时磁盘上的当前存档。".into());
                en.push("Contents: the save on disk at export time.".into());
            }
            zh.push(String::new());
            zh.push("每个子文件夹对应一个存档位置：".into());
            en.push(String::new());
            en.push("Each subfolder is one save location:".into());
            for (folder, path) in locations {
                zh.push(format!("  {folder}/  →  {path}"));
                en.push(format!("  {folder}/  ->  {path}"));
            }
            zh.push(String::new());
            zh.push("怎么用：".into());
            zh.push("  有篝火：游戏详情 → 导入 → 导入存档文件 / 篝火包，选择这个 zip 或文件夹（不用解压）。".into());
            zh.push("  没有篝火：先备份你自己的存档，关闭游戏，再把每个子文件夹里的文件复制到上面对应的位置。".into());
            en.push(String::new());
            en.push("How to use:".into());
            en.push("  With Bonfire: game page -> Import -> Import exported files / Bonfire package, pick this zip or folder.".into());
            en.push("  Without Bonfire: back up your own save, close the game, then copy each subfolder's files to the location above.".into());
        }
        ReadmeKind::Package { games } => {
            zh.push("这是篝火的存档包（.bonfire），包含存档历史（各个快照与路线）和截图。".into());
            en.push("A Bonfire package (.bonfire) with save history (snapshots and branches) and screenshots.".into());
            zh.push(String::new());
            zh.push(format!("包含 {} 个游戏：", games.len()));
            en.push(String::new());
            en.push(format!("{} game(s):", games.len()));
            for (name, snaps, shots) in games {
                zh.push(format!("  {name}：{snaps} 个快照，{shots} 张截图"));
                en.push(format!("  {name}: {snaps} snapshots, {shots} screenshots"));
            }
            zh.push(String::new());
            zh.push("怎么用：在篝火的游戏库中选“添加 / 导入 → 导入存档文件 / 篝火包”，选择这个文件，不需要解压。".into());
            zh.push(
                "迁移到新电脑时，导入页可以选“这是我旧电脑的存档”，把旧电脑的路线作为本机主存档。"
                    .into(),
            );
            en.push(String::new());
            en.push("How to use: in Bonfire's library choose Add / Import -> Import exported files / Bonfire package. No need to unpack.".into());
            en.push("Moving to a new PC? Choose \"My old PC's saves\" on the import page to make the old line this PC's primary.".into());
        }
        ReadmeKind::Zip { games, split } => {
            zh.push(format!(
                "这是用篝火导出的 zip，包含 {} 个游戏：",
                games.len()
            ));
            en.push(format!(
                "A Bonfire zip export with {} game(s):",
                games.len()
            ));
            for (dir, name, saves, shots) in games {
                let head = if dir.is_empty() {
                    name.clone()
                } else {
                    format!("{dir}/ ({name})")
                };
                let mut zh_parts = Vec::new();
                let mut en_parts = Vec::new();
                if *saves {
                    zh_parts.push(if *split {
                        "存档/ 中是存档".to_string()
                    } else {
                        "存档".into()
                    });
                    en_parts.push(if *split {
                        "saves in 存档/".to_string()
                    } else {
                        "saves".into()
                    });
                }
                if *shots > 0 {
                    zh_parts.push(if *split {
                        format!("截图/ 中是 {shots} 张截图")
                    } else {
                        format!("{shots} 张截图")
                    });
                    en_parts.push(if *split {
                        format!("{shots} screenshots in 截图/")
                    } else {
                        format!("{shots} screenshots")
                    });
                }
                zh.push(format!("  {head}：{}", zh_parts.join("，")));
                en.push(format!("  {head}: {}", en_parts.join(", ")));
            }
            zh.push(String::new());
            zh.push("存档是导出时各游戏最新快照的存档文件，每个存档文件夹里另有 readme.txt 说明对应的存档位置；截图都是原图。".into());
            zh.push("怎么用：有篝火时在游戏库中选“添加 / 导入 → 导入存档文件 / 篝火包”，选择这个 zip（不用解压）；没有篝火时按存档文件夹里的 readme.txt 手动复制。".into());
            en.push(String::new());
            en.push("Saves are each game's latest snapshot; every save folder has its own readme.txt listing the save locations. Screenshots are originals.".into());
            en.push("How to use: in Bonfire's library choose Add / Import -> Import exported files / Bonfire package and pick this zip (no need to unpack). Without Bonfire, copy files by hand as each save folder's readme.txt describes.".into());
        }
        ReadmeKind::Shots { game, count } => {
            zh.push(format!(
                "这是用篝火导出的《{game}》截图，共 {count} 张，都是原图。"
            ));
            en.push(format!(
                "{count} original screenshot(s) of \"{game}\" exported by Bonfire."
            ));
        }
    }
    let mut lines = vec![
        format!("{APP_NAME} — 游戏存档备份 / Game save backup"),
        README_MARKER.to_string(),
        format!(
            "生成 / Generated by: {}    导出时间 / Exported at: {exported_at}",
            generator()
        ),
        String::new(),
    ];
    lines.extend(zh);
    lines.push(String::new());
    lines.push("-----".into());
    lines.push(String::new());
    lines.extend(en);
    lines.push(String::new());
    lines.push("篝火 Bonfire 代码仓库 / Source:".into());
    for (name, url) in REPOS {
        lines.push(format!("  {name}: {url}"));
    }
    let mut out = "\u{feff}".as_bytes().to_vec();
    out.extend(lines.join("\r\n").into_bytes());
    out.extend(b"\r\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readme_has_marker_repos_and_bom() {
        let r = readme(
            &ReadmeKind::Files {
                game: "空洞骑士",
                snapshot: Some("1a2b3c4d"),
                comment: Some("白宫之前"),
                locations: vec![(
                    "HollowKnight".into(),
                    "%USERPROFILE%/AppData/LocalLow/Team Cherry/Hollow Knight".into(),
                )],
            },
            "2026-10-11T12:00:00+08:00",
        );
        assert!(r.starts_with("\u{feff}".as_bytes()));
        let text = String::from_utf8(r).unwrap();
        assert!(is_bonfire_readme(&text));
        assert!(text.contains("https://gitee.com/Hong_Xiao_Feng/bonfire.git"));
        assert!(text.contains("https://github.com/Hong-Da/Bonfire.git"));
        assert!(text.contains("白宫之前") && text.contains("\r\n"));
        assert!(is_bonfire_generator(Some(&generator())) && !is_bonfire_generator(None));
    }
}
