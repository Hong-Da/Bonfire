//! 同步目标的设置（保存在 `config.toml` 的 `[[sync.targets]]`；令牌与密码另存，见 [`App::sync_secret`]）。

use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::error::{AppError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SyncKind {
    Github,
    Gitee,
    Webdav,
    /// 文件夹：本机目录、局域网共享（`\\NAS\…`）、网盘客户端的同步文件夹。
    #[default]
    Folder,
}

impl SyncKind {
    pub fn is_git(self) -> bool {
        matches!(self, SyncKind::Github | SyncKind::Gitee)
    }

    /// 默认的单文件上限与总大小上限（MB，0 为不限）。
    pub fn default_limits(self) -> (u64, u64) {
        match self {
            // GitHub：单文件 100 MiB 为硬限制，仓库建议 1 GB 以内。
            SyncKind::Github => (100, 1024),
            // Gitee：未找到官方文档，取社区记录的个人仓库 500 MB 与保守的单文件 50 MB。
            SyncKind::Gitee => (50, 500),
            _ => (0, 0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SyncContent {
    /// 完整仓库（含历史，可从远程恢复）。
    #[default]
    Repo,
    /// 最新存档：每个游戏当前快照的文件。
    Latest,
    /// 只同步截图。
    Shots,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SyncSchedule {
    #[default]
    Manual,
    /// 每次备份后（手动与自动备份）。
    AfterBackup,
    Daily,
    Hours,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SyncTarget {
    pub id: String,
    pub name: String,
    pub kind: SyncKind,
    /// 界面上选的类型：github / gitee / webdav / cloud（网盘同步文件夹）/ network（远程目录）/ local（指定目录）。
    pub place: String,
    /// Git 仓库地址、WebDAV 地址或文件夹路径。
    pub url: String,
    /// Git 分支。
    pub branch: String,
    /// Gitee 用户名、WebDAV 用户名（GitHub 不需要）。
    pub user: String,
    pub content: SyncContent,
    /// 同时同步截图（“只同步截图”时总是包含）。
    pub shots: bool,
    /// 同时同步篝火设置（不含密钥、令牌与本机记住的密码）。
    pub settings: bool,
    /// 只同步这些游戏（空为全部）。
    pub games: Vec<String>,
    /// 单文件上限、总大小上限（MB，0 为不限）。
    pub max_file_mb: u64,
    pub max_total_mb: u64,
    pub schedule: SyncSchedule,
    /// 每天同步的时刻 / 每隔几小时。
    pub at: String,
    pub every_hours: u32,
    pub enabled: bool,
}

impl Default for SyncTarget {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            kind: SyncKind::Folder,
            place: String::new(),
            url: String::new(),
            branch: "main".into(),
            user: String::new(),
            content: SyncContent::Repo,
            shots: true,
            settings: true,
            games: Vec::new(),
            max_file_mb: 0,
            max_total_mb: 0,
            schedule: SyncSchedule::Manual,
            at: "21:00".into(),
            every_hours: 24,
            enabled: true,
        }
    }
}

impl SyncTarget {
    /// 新目标：按类型填好默认的大小上限与是否带截图。
    pub fn new(kind: SyncKind, name: &str, url: &str) -> Self {
        let (f, t) = kind.default_limits();
        Self {
            name: name.into(),
            kind,
            url: url.into(),
            shots: !kind.is_git(),
            max_file_mb: f,
            max_total_mb: t,
            ..Default::default()
        }
    }

    fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(AppError::Invalid("请填写同步目标的名称".into()));
        }
        if self.url.trim().is_empty() {
            return Err(AppError::Invalid("请填写地址或文件夹".into()));
        }
        if self.kind.is_git() {
            let u = self.url.trim().to_lowercase();
            let host = if self.kind == SyncKind::Github {
                "github.com"
            } else {
                "gitee.com"
            };
            let local =
                u.starts_with("file://") || std::path::Path::new(self.url.trim()).is_absolute();
            if !(u.starts_with("https://") && u.contains(host)) && !local {
                return Err(AppError::Invalid(format!(
                    "请填写 {host} 上仓库的 HTTPS 地址，如 https://{host}/用户名/仓库.git"
                )));
            }
        }
        if self.kind == SyncKind::Webdav
            && !(self.url.starts_with("https://") || self.url.starts_with("http://"))
        {
            return Err(AppError::Invalid(
                "WebDAV 地址应以 https:// 或 http:// 开头".into(),
            ));
        }
        Ok(())
    }
}

impl App {
    /// 新增或修改同步目标（按 ID；新目标自动生成 ID）。`secret` 为令牌或密码，`None` 时不改动。
    pub fn save_sync_target(
        &mut self,
        mut t: SyncTarget,
        secret: Option<&str>,
    ) -> Result<SyncTarget> {
        t.validate()?;
        if t.id.is_empty() {
            let mut b = [0u8; 4];
            getrandom::fill(&mut b).expect("系统随机数可用");
            t.id = b.iter().map(|x| format!("{x:02x}")).collect();
        }
        if self
            .config
            .sync
            .targets
            .iter()
            .any(|o| o.id != t.id && o.name == t.name)
        {
            return Err(AppError::Invalid(format!(
                "已经有名为「{}」的同步目标",
                t.name
            )));
        }
        match self.config.sync.targets.iter_mut().find(|o| o.id == t.id) {
            Some(o) => *o = t.clone(),
            None => self.config.sync.targets.push(t.clone()),
        }
        if let Some(s) = secret {
            self.set_sync_secret(&t.id, s)?;
        }
        self.config.save(&self.paths.config_file)?;
        Ok(t)
    }

    /// 删除同步目标（远程上的文件不动）。
    pub fn remove_sync_target(&mut self, id: &str) -> Result<()> {
        let t = self.sync_target(id)?;
        self.config.sync.targets.retain(|o| o.id != t.id);
        self.config.save(&self.paths.config_file)?;
        let _ = std::fs::remove_file(self.secret_file(&t.id));
        let _ = std::fs::remove_file(
            self.paths
                .cache_dir
                .join("sync")
                .join(format!("{}.json", t.id)),
        );
        Ok(())
    }

    fn secret_file(&self, id: &str) -> std::path::PathBuf {
        let data = self
            .paths
            .config_file
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default();
        data.join("keys").join(format!("sync-{id}.key"))
    }

    /// 目标的令牌或密码（Windows 数据保护）。
    pub fn sync_secret(&self, id: &str) -> Option<String> {
        let sealed = std::fs::read(self.secret_file(id)).ok()?;
        let plain = crate::security::unprotect(&sealed)?;
        String::from_utf8(plain).ok()
    }

    pub fn set_sync_secret(&self, id: &str, secret: &str) -> Result<()> {
        let path = self.secret_file(id);
        if secret.is_empty() {
            let _ = std::fs::remove_file(&path);
            return Ok(());
        }
        let sealed = crate::security::protect(secret.as_bytes())
            .ok_or_else(|| AppError::Invalid("这台电脑不能保存令牌或密码".into()))?;
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| AppError::io(d, e))?;
        }
        std::fs::write(&path, sealed).map_err(|e| AppError::io(&path, e))
    }
}
