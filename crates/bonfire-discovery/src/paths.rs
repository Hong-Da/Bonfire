//! 存档路径模板的变量展开与 glob 匹配（文档 4.2）。
//!
//! 变量：`<home>` `<winDocuments>` `<winAppData>` `<winLocalAppData>` `<winLocalAppDataLow>`
//! `<winSavedGames>` `<winPublic>` `<winProgramData>` `<winDir>` `<osUserName>`
//! `<base>`（游戏安装目录）`<root>`（库目录）`<game>`（安装文件夹名）
//! `<storeUserId>`（平台用户 ID，展开为 `*`，匹配本机所有账号）`<storeGameId>`。
//! 展开出的值会转义 glob 特殊字符；模板本身中的 `*` `?` `[...]` 保留为通配。

use std::path::{Path, PathBuf};

/// Windows 已知目录。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KnownDirs {
    pub home: Option<PathBuf>,
    pub app_data: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
    pub local_app_data_low: Option<PathBuf>,
    pub documents: Option<PathBuf>,
    pub saved_games: Option<PathBuf>,
    pub public: Option<PathBuf>,
    pub program_data: Option<PathBuf>,
    pub win_dir: Option<PathBuf>,
    pub os_user_name: Option<String>,
}

impl KnownDirs {
    /// 当前用户的系统目录。
    pub fn system() -> Self {
        let env = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let home = dirs::home_dir();
        Self {
            app_data: dirs::config_dir(),
            local_app_data: dirs::data_local_dir(),
            local_app_data_low: home.as_ref().map(|h| h.join("AppData").join("LocalLow")),
            documents: dirs::document_dir(),
            saved_games: home.as_ref().map(|h| h.join("Saved Games")),
            public: env("PUBLIC"),
            program_data: env("PROGRAMDATA"),
            win_dir: env("WINDIR"),
            os_user_name: std::env::var("USERNAME").ok(),
            home,
        }
    }

    /// 测试用：所有目录都放在一个根目录下的固定子目录中。
    pub fn under(root: &Path) -> Self {
        let home = root.join("Users").join("player");
        Self {
            app_data: Some(home.join("AppData/Roaming")),
            local_app_data: Some(home.join("AppData/Local")),
            local_app_data_low: Some(home.join("AppData/LocalLow")),
            documents: Some(home.join("Documents")),
            saved_games: Some(home.join("Saved Games")),
            public: Some(root.join("Users/Public")),
            program_data: Some(root.join("ProgramData")),
            win_dir: Some(root.join("Windows")),
            os_user_name: Some("player".into()),
            home: Some(home),
        }
    }

    /// 启发式探测时查找的常见存档目录（文档 4.2 第 3 条）。
    pub fn common_save_dirs(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = [
            &self.documents,
            &self.saved_games,
            &self.app_data,
            &self.local_app_data,
            &self.local_app_data_low,
        ]
        .into_iter()
        .flatten()
        .cloned()
        .collect();
        if let Some(d) = &self.documents {
            v.push(d.join("My Games"));
        }
        v
    }
}

/// 一个游戏的展开上下文。
#[derive(Debug, Clone, Default)]
pub struct PathContext {
    pub dirs: KnownDirs,
    pub base: Option<PathBuf>,
    pub root: Option<PathBuf>,
    pub game: Option<String>,
    pub store_game_id: Option<String>,
}

fn slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// 展开模板为 glob 模式；用到的变量没有值时返回 None（该条规则不适用）。
pub fn expand(template: &str, ctx: &PathContext) -> Option<String> {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let end = rest[start..].find('>')? + start;
        let var = &rest[start + 1..end];
        let d = &ctx.dirs;
        let path_val = |p: &Option<PathBuf>| p.as_ref().map(|p| glob::Pattern::escape(&slash(p)));
        let value = match var {
            "home" => path_val(&d.home),
            "winAppData" => path_val(&d.app_data),
            "winLocalAppData" => path_val(&d.local_app_data),
            "winLocalAppDataLow" => path_val(&d.local_app_data_low),
            "winDocuments" => path_val(&d.documents),
            "winSavedGames" => path_val(&d.saved_games),
            "winPublic" => path_val(&d.public),
            "winProgramData" => path_val(&d.program_data),
            "winDir" => path_val(&d.win_dir),
            "osUserName" => d.os_user_name.as_deref().map(glob::Pattern::escape),
            "base" => path_val(&ctx.base),
            "root" => path_val(&ctx.root),
            "game" => ctx.game.as_deref().map(glob::Pattern::escape),
            "storeGameId" => ctx.store_game_id.as_deref().map(glob::Pattern::escape),
            "storeUserId" => Some("*".to_string()),
            // Linux / macOS 专用变量（xdgData 等）在 Windows 上不适用。
            _ => None,
        }?;
        out.push_str(&value);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    // 模板必须展开为绝对路径，避免相对路径匹配到当前目录。
    let p = Path::new(&out);
    (p.is_absolute() || out.starts_with("//")).then_some(out)
}

/// 在磁盘上查找匹配的文件或目录（不区分大小写）。
pub fn find(pattern: &str) -> Vec<PathBuf> {
    let opts = glob::MatchOptions {
        case_sensitive: false,
        require_literal_separator: true,
        require_literal_leading_dot: false,
    };
    match glob::glob_with(pattern, opts) {
        Ok(paths) => paths.flatten().collect(),
        Err(e) => {
            tracing::debug!(pattern, "无效的路径模式：{e}");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(root: &Path) -> PathContext {
        PathContext {
            dirs: KnownDirs::under(root),
            base: Some(root.join("Steam/steamapps/common/Hollow Knight [GOTY]")),
            root: Some(root.join("Steam/steamapps/common")),
            game: Some("Hollow Knight [GOTY]".into()),
            store_game_id: Some("367520".into()),
        }
    }

    #[test]
    fn expands_variables_and_escapes_values() {
        let d = tempfile::tempdir().unwrap();
        let c = ctx(d.path());
        let p = expand("<base>/save/*.dat", &c).unwrap();
        assert!(p.ends_with("Hollow Knight [[]GOTY[]]/save/*.dat"), "{p}");
        let p = expand(
            "<winLocalAppDataLow>/Team Cherry/<storeUserId>/user*.dat",
            &c,
        )
        .unwrap();
        assert!(
            p.ends_with("AppData/LocalLow/Team Cherry/*/user*.dat"),
            "{p}"
        );
        assert!(expand("<xdgData>/foo", &c).is_none());
        assert!(expand("relative/path", &c).is_none());
        let empty = PathContext {
            dirs: KnownDirs::under(d.path()),
            ..Default::default()
        };
        assert!(
            expand("<base>/x", &empty).is_none(),
            "缺少安装目录时规则不适用"
        );
    }

    #[test]
    fn finds_files_case_insensitively_with_brackets_in_name() {
        let d = tempfile::tempdir().unwrap();
        let c = ctx(d.path());
        let base = c.base.clone().unwrap();
        std::fs::create_dir_all(base.join("Save")).unwrap();
        std::fs::write(base.join("Save/USER1.DAT"), b"1").unwrap();
        std::fs::write(base.join("Save/other.txt"), b"1").unwrap();
        let found = find(&expand("<base>/save/user*.dat", &c).unwrap());
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].ends_with("USER1.DAT"));
    }
}
