//! 应用配置：全部配置项的唯一定义。
//!
//! 优先级：环境变量 > 配置文件 > 代码默认值。配置文件缺失时使用默认值，程序仍能启动。
//! `config/config.example.toml` 由 [`Config::example_toml`] 生成，文档测试会检查两者一致。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::i18n::Locale;

pub const ENV_LANG: &str = "BONFIRE_LANG";
pub const ENV_REPO: &str = "BONFIRE_REPO";
pub const ENV_LOG_LEVEL: &str = "BONFIRE_LOG_LEVEL";

pub const LOG_LEVELS: [&str; 5] = ["error", "warn", "info", "debug", "trace"];

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("无法读取配置文件 {path}: {source}")]
    Read { path: PathBuf, source: std::io::Error },
    #[error("配置文件格式错误 {path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("无法写入配置文件 {path}: {source}")]
    Write { path: PathBuf, source: std::io::Error },
    #[error("配置项 {key} 的值无效：{message}")]
    Invalid { key: &'static str, message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Auto,
    Light,
    Dark,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub general: General,
    pub backup: Backup,
    pub metadata: Metadata,
    pub log: Log,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    /// 界面语言 `zh-CN` / `en`；为空时跟随系统语言。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub theme: Theme,
    /// 存档仓库位置；为空时使用默认位置。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Backup {
    /// 是否同时备份标记为 config 的游戏配置文件。
    pub include_config_files: bool,
    /// 单个游戏存档超过该大小（GB）时提示，防止误匹配整个安装目录。
    pub size_warning_gb: f64,
    /// 关闭扫描的平台 id，如 `["epic", "gog"]`。
    pub disabled_platforms: Vec<String>,
}

impl Default for Backup {
    fn default() -> Self {
        Self { include_config_files: true, size_warning_gb: 2.0, disabled_platforms: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Metadata {
    /// 是否联网获取游戏信息（名称、封面等）；关闭后只用本地数据。
    pub online: bool,
    pub igdb_client_id: String,
    pub igdb_client_secret: String,
    pub steamgriddb_api_key: String,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            online: true,
            igdb_client_id: String::new(),
            igdb_client_secret: String::new(),
            steamgriddb_api_key: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Log {
    pub level: String,
    pub keep_days: u32,
}

impl Default for Log {
    fn default() -> Self {
        Self { level: "info".into(), keep_days: 14 }
    }
}

impl Config {
    /// 读取配置文件（不存在时使用默认值），再应用环境变量并校验。
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let mut cfg = Self::load_file(path)?;
        cfg.apply_env(|k| std::env::var(k).ok().filter(|v| !v.is_empty()));
        cfg.validate()?;
        Ok(cfg)
    }

    fn load_file(path: &Path) -> Result<Self, ConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => return Err(ConfigError::Read { path: path.into(), source }),
        };
        toml::from_str(&text).map_err(|e| ConfigError::Parse { path: path.into(), message: e.to_string() })
    }

    fn apply_env(&mut self, get: impl Fn(&str) -> Option<String>) {
        if let Some(v) = get(ENV_LANG) {
            self.general.language = Some(v);
        }
        if let Some(v) = get(ENV_REPO) {
            self.general.repo = Some(PathBuf::from(v));
        }
        if let Some(v) = get(ENV_LOG_LEVEL) {
            self.log.level = v;
        }
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if let Some(lang) = &self.general.language
            && Locale::parse(lang).is_none()
        {
            return Err(ConfigError::Invalid { key: "general.language", message: format!("{lang}（可选 zh-CN、en）") });
        }
        let level = self.log.level.to_ascii_lowercase();
        if !LOG_LEVELS.contains(&level.as_str()) {
            return Err(ConfigError::Invalid { key: "log.level", message: format!("{}（可选 {}）", self.log.level, LOG_LEVELS.join("、")) });
        }
        if !(1..=365).contains(&self.log.keep_days) {
            return Err(ConfigError::Invalid { key: "log.keep_days", message: format!("{}（范围 1–365）", self.log.keep_days) });
        }
        let gb = self.backup.size_warning_gb;
        if gb.is_nan() || gb <= 0.0 {
            return Err(ConfigError::Invalid { key: "backup.size_warning_gb", message: format!("{}（必须大于 0）", self.backup.size_warning_gb) });
        }
        Ok(())
    }

    /// 写入配置文件（先写临时文件再替换）。
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let werr = |source| ConfigError::Write { path: path.into(), source };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(werr)?;
        }
        let text = toml::to_string_pretty(self).expect("配置序列化不会失败");
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).map_err(werr)?;
        std::fs::rename(&tmp, path).map_err(werr)
    }

    /// 实际使用的仓库目录。
    pub fn repo_dir(&self, default: &Path) -> PathBuf {
        self.general.repo.clone().unwrap_or_else(|| default.to_path_buf())
    }

    /// 带注释的配置示例，内容即默认值。
    pub fn example_toml() -> String {
        let d = Self::default();
        format!(
            r#"# 篝火 Bonfire 应用配置示例
# 实际配置文件位置：程序目录下的 data\config.toml（开发时为项目根目录下的 data\config.toml）
# 所有配置项都有默认值，可以只写需要修改的项。环境变量优先于本文件。

[general]
# 界面语言：zh-CN / en；不写时跟随系统语言。环境变量 BONFIRE_LANG
# language = "zh-CN"
# 界面主题：auto（跟随系统）/ light / dark
theme = "{theme}"
# 存档仓库位置；不写时为 data\repo。可放在网盘同步目录中，但不能是 data 本身。环境变量 BONFIRE_REPO
# repo = 'D:\Backups\Bonfire'

[backup]
# 是否同时备份游戏的配置文件（画质、按键等设置）
include_config_files = {include}
# 单个游戏存档超过该大小（GB）时提示，防止误匹配到整个安装目录
size_warning_gb = {size:.1}
# 不扫描的平台：steam / epic / gog / ubisoft / ea / xbox / battlenet
disabled_platforms = []

[metadata]
# 是否联网获取游戏名称、封面等信息；只发送游戏 ID 或名称，不上传存档
online = {online}
# IGDB（Twitch 开发者凭证）与 SteamGridDB 的 API Key；不填则跳过这两个来源
igdb_client_id = ""
igdb_client_secret = ""
steamgriddb_api_key = ""

[log]
# 日志级别：error / warn / info / debug / trace。环境变量 BONFIRE_LOG_LEVEL
level = "{level}"
# 日志保留天数（1–365），按日期轮转
keep_days = {keep}
"#,
            theme = "auto",
            include = d.backup.include_config_files,
            size = d.backup.size_warning_gb,
            online = d.metadata.online,
            level = d.log.level,
            keep = d.log.keep_days,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::load_file(&dir.path().join("none.toml")).unwrap();
        assert_eq!(cfg, Config::default());
        assert_eq!(cfg.log.keep_days, 14);
        assert!(cfg.backup.include_config_files);
    }

    #[test]
    fn partial_file_keeps_other_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.toml");
        std::fs::write(&p, "[log]\nlevel = \"debug\"\n").unwrap();
        let cfg = Config::load_file(&p).unwrap();
        assert_eq!(cfg.log.level, "debug");
        assert_eq!(cfg.log.keep_days, 14);
        assert_eq!(cfg.general.theme, Theme::Auto);
    }

    #[test]
    fn unknown_keys_are_tolerated() {
        let cfg: Config = toml::from_str("[general]\nfuture_option = 1\n").unwrap();
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn bad_syntax_is_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.toml");
        std::fs::write(&p, "[log\nlevel=").unwrap();
        assert!(matches!(Config::load_file(&p), Err(ConfigError::Parse { .. })));
    }

    #[test]
    fn env_overrides_file() {
        let mut cfg = Config::default();
        cfg.general.language = Some("zh-CN".into());
        cfg.apply_env(|k| match k {
            ENV_LANG => Some("en".into()),
            ENV_REPO => Some("E:/repo".into()),
            _ => None,
        });
        assert_eq!(cfg.general.language.as_deref(), Some("en"));
        assert_eq!(cfg.general.repo, Some(PathBuf::from("E:/repo")));
        assert_eq!(cfg.log.level, "info");
    }

    #[test]
    fn validate_rejects_bad_values() {
        let mut cfg = Config::default();
        cfg.general.language = Some("pt-BR".into());
        assert!(matches!(cfg.validate(), Err(ConfigError::Invalid { key: "general.language", .. })));

        let mut cfg = Config::default();
        cfg.log.level = "verbose".into();
        assert!(matches!(cfg.validate(), Err(ConfigError::Invalid { key: "log.level", .. })));

        let mut cfg = Config::default();
        cfg.log.keep_days = 0;
        assert!(matches!(cfg.validate(), Err(ConfigError::Invalid { key: "log.keep_days", .. })));
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub/config.toml");
        let mut cfg = Config::default();
        cfg.general.repo = Some(PathBuf::from("D:/备份/存档"));
        cfg.general.theme = Theme::Dark;
        cfg.save(&p).unwrap();
        assert_eq!(Config::load_file(&p).unwrap(), cfg);
    }

    #[test]
    fn example_parses_to_defaults() {
        let cfg: Config = toml::from_str(&Config::example_toml()).unwrap();
        assert_eq!(cfg, Config::default());
    }
}
