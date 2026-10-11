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
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("配置文件格式错误 {path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("无法写入配置文件 {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
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
    pub shots: Shots,
    pub auto_backup: AutoBackup,
    pub metadata: Metadata,
    pub log: Log,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    /// 界面语言 `zh-CN` / `en`；为空时跟随系统语言。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub theme: Theme,
    /// 存档仓库位置；为空时使用默认位置。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<PathBuf>,
    /// 关闭窗口时留在系统托盘（不退出）；从托盘菜单“退出”才结束程序。
    pub close_to_tray: bool,
    /// 已经问过（或在设置中选过）关闭窗口时的做法；为 false 时第一次关闭窗口会询问。
    pub close_asked: bool,
    /// 记住游戏库的筛选（状态、管理方式、平台、筛选方案），重新打开时恢复。
    pub remember_filters: bool,
    /// 记住游戏库中勾选的游戏，重新打开时恢复；关闭时每次按默认勾选有更新与未备份的游戏。
    pub remember_selection: bool,
}

impl Default for General {
    fn default() -> Self {
        Self {
            language: None,
            theme: Theme::Auto,
            repo: None,
            close_to_tray: true,
            close_asked: false,
            remember_filters: true,
            remember_selection: true,
        }
    }
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
    /// 存档安全保护：本机备份的快照（快照记录的电脑名与本机相同）不能删除，
    /// 包含这些快照的分支也不能删除。
    pub protect_local: bool,
}

impl Default for Backup {
    fn default() -> Self {
        Self {
            include_config_files: true,
            size_warning_gb: 2.0,
            disabled_platforms: Vec::new(),
            protect_local: true,
        }
    }
}

/// 定时任务（定时备份、定时收集截图）的频率。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShotSchedule {
    /// 只随备份收集。
    #[default]
    Off,
    Daily,
    Weekly,
    /// 每隔若干小时。
    Hours,
}

/// 截图备份（截图备份方案 S1–S3）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shots {
    /// 是否收集截图；每个游戏可单独开关。
    pub enabled: bool,
    /// Windows 游戏栏 / Xbox 应用的“捕获”文件夹；为空时读取系统设置中的实际位置。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub captures_dir: Option<PathBuf>,
    /// 定时收集（程序在托盘中运行时执行）；无论是否开启，每次备份都会顺带收集。
    pub schedule: ShotSchedule,
    /// 每天 / 每周收集的时刻 `HH:MM`。
    pub at: String,
    /// 每周收集的星期：1 = 周一 … 7 = 周日。
    pub weekday: u8,
    /// 每隔几小时收集（1–168）。
    pub every_hours: u32,
    /// 公用截图文件夹：多个游戏的截图放在一起（NVIDIA、ShareX 等），按子文件夹名或文件名对应游戏。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub shared_dirs: Vec<PathBuf>,
    /// 备份未识别游戏的截图：“捕获”文件夹与公用截图文件夹中对应不上游戏库中任何游戏的截图，
    /// 放在“未识别的截图”相册中，之后加入对应的游戏时自动归位。
    pub keep_unsorted: bool,
    /// 截图变化后自动收集：每隔 `check_minutes` 分钟检查截图文件夹，有新截图就收集。
    pub on_change: bool,
    /// 检查新截图的间隔（分钟，1–120）。
    pub check_minutes: u32,
}

impl Default for Shots {
    fn default() -> Self {
        Self {
            enabled: true,
            captures_dir: None,
            schedule: ShotSchedule::Off,
            at: "21:00".into(),
            weekday: 7,
            every_hours: 6,
            shared_dirs: Vec::new(),
            keep_unsorted: true,
            on_change: false,
            check_minutes: 5,
        }
    }
}

impl Shots {
    /// `at` 解析为（时, 分）。
    pub fn at_hm(&self) -> Option<(i8, i8)> {
        parse_hm(&self.at)
    }
}

/// `HH:MM` 解析为（时, 分）。
pub fn parse_hm(s: &str) -> Option<(i8, i8)> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (i8, i8) = (h.parse().ok()?, m.parse().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m)).then_some((h, m))
}

/// 自动备份（U-9c）：程序运行（含托盘中）时执行，只备份有变化、已经备份过的游戏。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoBackup {
    /// 定时备份：off / daily / weekly / hours。
    pub schedule: ShotSchedule,
    /// 每天 / 每周备份的时刻 `HH:MM`。
    pub at: String,
    /// 每周备份的星期：1 = 周一 … 7 = 周日。
    pub weekday: u8,
    /// 每隔几小时备份（1–168）。
    pub every_hours: u32,
    /// 存档变化后自动备份。
    pub on_change: bool,
    /// 检查存档变化的间隔（分钟，1–120）。
    pub check_minutes: u32,
    /// 存档停止写入多少分钟后才备份（0–60），避免备份到写了一半的存档。
    pub quiet_minutes: u32,
}

impl Default for AutoBackup {
    fn default() -> Self {
        Self {
            schedule: ShotSchedule::Off,
            at: "21:00".into(),
            weekday: 7,
            every_hours: 6,
            on_change: false,
            check_minutes: 5,
            quiet_minutes: 2,
        }
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
        Self {
            level: "info".into(),
            keep_days: 14,
        }
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
            Err(source) => {
                return Err(ConfigError::Read {
                    path: path.into(),
                    source,
                });
            }
        };
        toml::from_str(&text).map_err(|e| ConfigError::Parse {
            path: path.into(),
            message: e.to_string(),
        })
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
            return Err(ConfigError::Invalid {
                key: "general.language",
                message: format!("{lang}（可选 zh-CN、en）"),
            });
        }
        let level = self.log.level.to_ascii_lowercase();
        if !LOG_LEVELS.contains(&level.as_str()) {
            return Err(ConfigError::Invalid {
                key: "log.level",
                message: format!("{}（可选 {}）", self.log.level, LOG_LEVELS.join("、")),
            });
        }
        if !(1..=365).contains(&self.log.keep_days) {
            return Err(ConfigError::Invalid {
                key: "log.keep_days",
                message: format!("{}（范围 1–365）", self.log.keep_days),
            });
        }
        if self.shots.at_hm().is_none() {
            return Err(ConfigError::Invalid {
                key: "shots.at",
                message: format!("{}（格式为 HH:MM，如 21:00）", self.shots.at),
            });
        }
        if !(1..=7).contains(&self.shots.weekday) {
            return Err(ConfigError::Invalid {
                key: "shots.weekday",
                message: format!("{}（1 = 周一 … 7 = 周日）", self.shots.weekday),
            });
        }
        if !(1..=120).contains(&self.shots.check_minutes) {
            return Err(ConfigError::Invalid {
                key: "shots.check_minutes",
                message: format!("{}（范围 1–120）", self.shots.check_minutes),
            });
        }
        if !(1..=168).contains(&self.shots.every_hours) {
            return Err(ConfigError::Invalid {
                key: "shots.every_hours",
                message: format!("{}（范围 1–168）", self.shots.every_hours),
            });
        }
        let a = &self.auto_backup;
        if parse_hm(&a.at).is_none() {
            return Err(ConfigError::Invalid {
                key: "auto_backup.at",
                message: format!("{}（格式为 HH:MM，如 21:00）", a.at),
            });
        }
        if !(1..=7).contains(&a.weekday) {
            return Err(ConfigError::Invalid {
                key: "auto_backup.weekday",
                message: format!("{}（1 = 周一 … 7 = 周日）", a.weekday),
            });
        }
        if !(1..=168).contains(&a.every_hours) {
            return Err(ConfigError::Invalid {
                key: "auto_backup.every_hours",
                message: format!("{}（范围 1–168）", a.every_hours),
            });
        }
        if !(1..=120).contains(&a.check_minutes) {
            return Err(ConfigError::Invalid {
                key: "auto_backup.check_minutes",
                message: format!("{}（范围 1–120）", a.check_minutes),
            });
        }
        if a.quiet_minutes > 60 {
            return Err(ConfigError::Invalid {
                key: "auto_backup.quiet_minutes",
                message: format!("{}（范围 0–60）", a.quiet_minutes),
            });
        }
        let gb = self.backup.size_warning_gb;
        if gb.is_nan() || gb <= 0.0 {
            return Err(ConfigError::Invalid {
                key: "backup.size_warning_gb",
                message: format!("{}（必须大于 0）", self.backup.size_warning_gb),
            });
        }
        Ok(())
    }

    /// 写入配置文件（先写临时文件再替换）。
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let werr = |source| ConfigError::Write {
            path: path.into(),
            source,
        };
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
        self.general
            .repo
            .clone()
            .unwrap_or_else(|| default.to_path_buf())
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
# 关闭窗口时留在系统托盘（不退出）；从托盘菜单“退出”才结束程序
close_to_tray = {tray}
# 是否已经选过关闭窗口时的做法；为 false 时第一次关闭窗口会询问“最小化到托盘还是退出”
close_asked = {asked}
# 记住游戏库的筛选（状态、管理方式、平台、筛选方案），重新打开时恢复
remember_filters = {rem_filters}
# 记住游戏库中勾选的游戏；关闭时每次按默认勾选有更新与未备份的游戏
remember_selection = {rem_sel}

[backup]
# 是否同时备份游戏的配置文件（画质、按键等设置）
include_config_files = {include}
# 单个游戏存档超过该大小（GB）时提示，防止误匹配到整个安装目录
size_warning_gb = {size:.1}
# 不扫描的平台：steam / epic / gog / ubisoft / ea / xbox / battlenet
disabled_platforms = []
# 存档安全保护：本机备份的快照（以电脑名区分）及包含它们的分支不能删除
protect_local = {protect}

[shots]
# 是否备份游戏截图（Steam 截图、Windows 游戏栏截图、为游戏指定的截图文件夹）；每个游戏可单独关闭
enabled = {shots}
# Windows 游戏栏 / Xbox 应用的“捕获”文件夹；不写时读取系统设置中的实际位置
# captures_dir = 'D:\Videos\Captures'
# 公用截图文件夹：多个游戏的截图放在一起（NVIDIA、ShareX 等的截图目录），按子文件夹名或文件名中的游戏名对应游戏
# shared_dirs = ['D:\Screenshots']
# 定时收集截图：off（只随备份收集）/ daily / weekly / hours；在托盘中运行时执行
schedule = "{schedule}"
# 每天 / 每周收集的时刻
at = "{at}"
# 每周收集的星期：1 = 周一 … 7 = 周日
weekday = {weekday}
# schedule = "hours" 时每隔几小时收集（1–168）
every_hours = {hours}
# 备份未识别游戏的截图：对应不上游戏库中任何游戏的截图放在“未识别的截图”相册中，之后加入对应的游戏时自动归位
keep_unsorted = {s_unsorted}
# 截图变化后自动收集：每隔 check_minutes 分钟检查截图文件夹，有新截图就收集
on_change = {s_change}
# 检查新截图的间隔（分钟，1–120）
check_minutes = {s_check}

[auto_backup]
# 自动备份在篝火运行（含托盘中）时执行，只备份有变化、已经手动备份过的游戏，同时收集截图。
# 定时备份：off / daily / weekly / hours
schedule = "off"
# 每天 / 每周备份的时刻
at = "{a_at}"
# 每周备份的星期：1 = 周一 … 7 = 周日
weekday = {a_weekday}
# schedule = "hours" 时每隔几小时备份（1–168）
every_hours = {a_hours}
# 存档变化后自动备份（游戏存档后自动备份）
on_change = {a_change}
# 检查存档变化的间隔（分钟，1–120）
check_minutes = {a_check}
# 存档停止写入多少分钟后才备份（0–60），避免备份到写了一半的存档
quiet_minutes = {a_quiet}

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
            tray = d.general.close_to_tray,
            asked = d.general.close_asked,
            rem_filters = d.general.remember_filters,
            rem_sel = d.general.remember_selection,
            protect = d.backup.protect_local,
            shots = d.shots.enabled,
            schedule = "off",
            at = d.shots.at,
            weekday = d.shots.weekday,
            hours = d.shots.every_hours,
            s_change = d.shots.on_change,
            s_unsorted = d.shots.keep_unsorted,
            s_check = d.shots.check_minutes,
            a_at = d.auto_backup.at,
            a_weekday = d.auto_backup.weekday,
            a_hours = d.auto_backup.every_hours,
            a_change = d.auto_backup.on_change,
            a_check = d.auto_backup.check_minutes,
            a_quiet = d.auto_backup.quiet_minutes,
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
        assert!(matches!(
            Config::load_file(&p),
            Err(ConfigError::Parse { .. })
        ));
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
        assert!(matches!(
            cfg.validate(),
            Err(ConfigError::Invalid {
                key: "general.language",
                ..
            })
        ));

        let mut cfg = Config::default();
        cfg.log.level = "verbose".into();
        assert!(matches!(
            cfg.validate(),
            Err(ConfigError::Invalid {
                key: "log.level",
                ..
            })
        ));

        let mut cfg = Config::default();
        cfg.log.keep_days = 0;
        assert!(matches!(
            cfg.validate(),
            Err(ConfigError::Invalid {
                key: "log.keep_days",
                ..
            })
        ));
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
