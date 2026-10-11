//! 应用上下文：打开仓库与索引，管理游戏及其存档位置。
//!
//! 游戏配置同时备份到仓库根目录的 `games.toml`（文档 §7），索引数据库丢失时据此恢复。

use std::path::{Path, PathBuf};

use bonfire_core::index::{GameRecord, LocationKind, LocationRecord, ReindexStats, ShotSettings};
use bonfire_core::scan::{RootKind, ScanRoot, glob_base};
use bonfire_core::{FileTag, Index, ObjectId, Repo, RepoLock};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};
use crate::{AppPaths, Config, Locale};

pub const GAMES_FILE: &str = "games.toml";

pub struct App {
    pub paths: AppPaths,
    pub config: Config,
    pub locale: Locale,
    pub repo: Repo,
    pub index: Index,
    /// 联网请求（规则库更新、游戏信息）；测试中替换为模拟实现。
    pub http: std::sync::Arc<dyn bonfire_discovery::http::Http>,
    /// Windows 已知目录（存档路径变量）；测试中替换为临时目录。
    pub known_dirs: bonfire_discovery::KnownDirs,
    /// 扫描平台后是否刷新游戏信息（CLI `scan --no-metadata` 关闭）。
    pub fetch_metadata_on_scan: bool,
    /// 伪造的“本机”根目录（测试用，见 `BONFIRE_FAKE_ROOT`）；设置后不读取真实的平台安装。
    pub fake_root: Option<PathBuf>,
    /// 退出游戏后自动备份的检测状态（M4-3）。
    pub exit_watch: crate::auto::ExitWatcher,
    /// 读取被占用文件的卷影复制（M5-3）；测试中替换为模拟实现。
    pub shadow: std::sync::Arc<dyn crate::vss::ShadowReader>,
    /// 打开时还没执行的、需要读取仓库的步骤（加密仓库未解锁时）；值为索引是否新建。
    pending_open: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GamesFile {
    #[serde(default)]
    game: Vec<GameEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GameEntry {
    #[serde(flatten)]
    record: GameRecord,
    #[serde(default)]
    location: Vec<LocationRecord>,
    /// 截图设置（是否收集、手动添加的截图文件夹）。
    #[serde(default, skip_serializing_if = "is_default_shots")]
    shots: ShotSettings,
    /// 游戏程序（运行检测用的进程名）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    processes: Vec<String>,
}

fn is_default_shots(s: &ShotSettings) -> bool {
    *s == ShotSettings::default()
}

/// 添加游戏的参数。
#[derive(Debug, Clone, Default)]
pub struct NewGame {
    pub name: String,
    pub save_paths: Vec<PathBuf>,
    pub config_paths: Vec<PathBuf>,
    pub install_dir: Option<PathBuf>,
}

impl App {
    /// 打开（必要时初始化）仓库与索引。索引是新建的时，从 `games.toml` 恢复游戏并从对象库重建索引。
    pub fn open(paths: AppPaths, config: Config, locale: Locale, repo_dir: &Path) -> Result<Self> {
        // 仓库自带的参数文件也叫 config.toml：仓库不能与应用配置放在同一目录。
        if paths
            .config_file
            .parent()
            .is_some_and(|d| crate::discover::same_dir(d, repo_dir))
        {
            return Err(AppError::Invalid(format!(
                "仓库位置不能与数据目录相同：{}，请改用其下的子目录（如 repo）",
                repo_dir.display()
            )));
        }
        let repo = Repo::init(repo_dir)?;
        // 加密仓库的索引在本机缓存目录（M5-2）。
        let index_file = crate::security::index_path(&paths, &repo);
        let fresh_db = !index_file.exists();
        if let Some(dir) = index_file.parent() {
            std::fs::create_dir_all(dir).map_err(|e| AppError::io(dir, e))?;
        }
        let index = Index::open(&index_file)?;
        let mut app = Self {
            paths,
            config,
            locale,
            repo,
            index,
            http: if std::env::var("BONFIRE_OFFLINE").is_ok_and(|v| v == "1") {
                std::sync::Arc::new(bonfire_discovery::http::Offline)
            } else {
                std::sync::Arc::new(bonfire_discovery::http::Throttled::new(
                    bonfire_discovery::http::UreqHttp::default(),
                ))
            },
            known_dirs: match crate::discover::fake_root() {
                Some(root) => bonfire_discovery::KnownDirs::under(&root),
                None => bonfire_discovery::KnownDirs::system(),
            },
            fetch_metadata_on_scan: true,
            fake_root: crate::discover::fake_root(),
            exit_watch: Default::default(),
            shadow: std::sync::Arc::new(crate::vss::ElevatedVss),
            pending_open: Some(fresh_db),
        };
        // 加密仓库：先尝试自动解锁；仍锁定时，下面需要读取仓库的步骤留到解锁后（`App::unlock`）。
        app.auto_unlock();
        if app.repo.is_unlocked() {
            app.after_unlock()?;
        }
        Ok(app)
    }

    /// 打开仓库后需要读取仓库内容的步骤（加密仓库在解锁后执行）。
    pub(crate) fn after_unlock(&mut self) -> Result<()> {
        let Some(fresh_db) = self.pending_open.take() else {
            return Ok(());
        };
        let app = self;
        if fresh_db {
            let restored = app.restore_games_file()?;
            let has_history = !app
                .repo
                .list_objects(bonfire_core::Kind::Snapshot)?
                .is_empty();
            if has_history {
                let stats = bonfire_core::index::reindex(&app.repo, &mut app.index)?;
                tracing::info!(restored, ?stats, "索引数据库不存在，已从仓库重建");
            }
            // 只管理截图的游戏没有快照，相册也要恢复。
            app.reindex_albums()?;
        }
        // 早期版本存放在对象库中的截图迁移为普通文件（失败不影响打开）。
        if let Err(e) = app.migrate_shot_files() {
            tracing::warn!("迁移截图失败：{e}");
        }
        Ok(())
    }

    /// 按 ID 或名称（不区分大小写）查找游戏。
    pub fn find_game(&self, key: &str) -> Result<GameRecord> {
        if let Some(g) = self.index.game(key)? {
            return Ok(g);
        }
        // “未识别的截图”相册（不是游戏库中的游戏，截图相关操作按游戏处理）。
        if key == crate::unsorted::UNSORTED {
            return Ok(crate::unsorted::unsorted_game());
        }
        let lower = key.to_lowercase();
        let matches: Vec<GameRecord> = self
            .index
            .games()?
            .into_iter()
            .filter(|g| g.name.to_lowercase() == lower)
            .collect();
        match matches.len() {
            0 => Err(AppError::GameNotFound(key.to_string())),
            1 => Ok(matches.into_iter().next().unwrap()),
            _ => Err(AppError::AmbiguousGame {
                name: key.to_string(),
                candidates: matches
                    .iter()
                    .map(|g| g.id.as_str())
                    .collect::<Vec<_>>()
                    .join("、"),
            }),
        }
    }

    /// 未被忽略的游戏。
    pub fn games(&self) -> Result<Vec<GameRecord>> {
        Ok(self
            .index
            .games()?
            .into_iter()
            .filter(|g| g.enabled)
            .collect())
    }

    /// 全部游戏（含已忽略的）。
    pub fn all_games(&self) -> Result<Vec<GameRecord>> {
        Ok(self.index.games()?)
    }

    /// 手动添加游戏：路径必须存在，文件或目录均可。
    pub fn add_game(&mut self, _lock: &RepoLock, new: &NewGame) -> Result<GameRecord> {
        let name = new.name.trim();
        if name.is_empty() {
            return Err(AppError::Invalid("游戏名称不能为空".into()));
        }
        if new.save_paths.is_empty() && new.config_paths.is_empty() {
            return Err(AppError::NoLocations(name.to_string()));
        }
        let mut locations = Vec::new();
        for (paths, tag) in [
            (&new.save_paths, FileTag::Save),
            (&new.config_paths, FileTag::Config),
        ] {
            for p in paths {
                let abs = absolute(p)?;
                let kind = if abs.is_file() {
                    LocationKind::File
                } else {
                    LocationKind::Dir
                };
                let s = abs.to_string_lossy().into_owned();
                locations.push(LocationRecord {
                    path_template: s.clone(),
                    resolved_path: s,
                    kind,
                    tag,
                    source: "user".into(),
                    confirmed: true,
                });
            }
        }
        let seed = format!(
            "{name}\0{}\0{}",
            bonfire_core::scan::now_ns(),
            std::process::id()
        );
        let id = format!("custom-{}", ObjectId::of(seed.as_bytes()).short(12));
        let record = GameRecord {
            id,
            name: name.to_string(),
            platform: "custom".into(),
            app_id: None,
            install_dir: new
                .install_dir
                .as_deref()
                .map(absolute)
                .transpose()?
                .map(|p| p.to_string_lossy().into_owned()),
            enabled: true,
            include_config: None,
            custom_rule_json: None,
        };
        self.index.upsert_game(&record)?;
        self.index.set_locations(&record.id, &locations)?;
        self.save_games_file()?;
        Ok(record)
    }

    /// 移除游戏。手动添加的游戏直接删除配置；自动识别的游戏标记为已忽略，重新扫描不会再加回。
    /// 历史快照都保留在仓库中。
    pub fn remove_game(&mut self, _lock: &RepoLock, key: &str) -> Result<GameRecord> {
        let mut g = self.find_game(key)?;
        if g.platform == "custom" {
            self.index.delete_game(&g.id)?;
        } else {
            g.enabled = false;
            self.index.upsert_game(&g)?;
        }
        self.save_games_file()?;
        Ok(g)
    }

    /// 移出游戏库，并按选择删除备份的存档（全部快照）与截图（U-15）。先删快照：存档安全保护拒绝时什么都不改。
    /// 磁盘上的游戏存档与截图原图都不受影响。
    pub fn remove_game_with(
        &mut self,
        lock: &RepoLock,
        key: &str,
        opts: &RemoveOptions,
    ) -> Result<RemoveReport> {
        let g = self.find_game(key)?;
        let snapshots = if opts.snapshots {
            self.delete_game_history(lock, &g)?
        } else {
            0
        };
        let shots = if opts.shots {
            self.delete_all_shots(lock, &g)?
        } else {
            0
        };
        let game = self.remove_game(lock, &g.id)?;
        if snapshots + shots > 0 {
            tracing::info!(game = %game.id, snapshots, shots, "移出游戏库并删除了备份");
        }
        Ok(RemoveReport {
            game,
            snapshots,
            shots,
        })
    }

    /// 恢复被忽略的游戏。
    pub fn enable_game(&mut self, _lock: &RepoLock, key: &str) -> Result<GameRecord> {
        let mut g = self.find_game(key)?;
        g.enabled = true;
        self.index.upsert_game(&g)?;
        self.save_games_file()?;
        Ok(g)
    }

    pub fn locations(&self, game_id: &str) -> Result<Vec<LocationRecord>> {
        Ok(self.index.locations(game_id)?)
    }

    /// 扫描用的存档位置。按游戏或全局设置决定是否包含配置文件；未确认的（启发式）位置不参与。
    /// 注册表位置先导出为缓存目录中的 .reg 文件，再当作普通文件处理（文档 4.2）。
    pub fn scan_roots(&self, game: &GameRecord) -> Result<Vec<ScanRoot>> {
        let include_config = game
            .include_config
            .unwrap_or(self.config.backup.include_config_files);
        let mut roots = Vec::new();
        for (ord, l) in self.locations(&game.id)?.into_iter().enumerate() {
            if !l.confirmed || (l.tag == FileTag::Config && !include_config) {
                continue;
            }
            let (path, kind) = match l.kind {
                LocationKind::Registry => {
                    let file = self.registry_file(&game.id, ord);
                    if let Err(e) = bonfire_discovery::registry::export(&l.resolved_path, &file) {
                        tracing::warn!(key = %l.resolved_path, "导出注册表失败，跳过：{e}");
                        let _ = std::fs::remove_file(&file);
                    }
                    (file, RootKind::File)
                }
                LocationKind::File => (PathBuf::from(&l.resolved_path), RootKind::File),
                LocationKind::Dir => (PathBuf::from(&l.resolved_path), RootKind::Dir),
                LocationKind::Glob => (PathBuf::from(&l.resolved_path), RootKind::Glob),
            };
            roots.push(ScanRoot {
                ord,
                path,
                kind,
                tag: l.tag,
            });
        }
        Ok(roots)
    }

    /// 注册表存档导出的缓存文件。
    pub fn registry_file(&self, game_id: &str, ord: usize) -> PathBuf {
        self.paths
            .cache_dir
            .join("registry")
            .join(game_id)
            .join(format!("{ord}.reg"))
    }

    /// 清单路径（`<序号>/<相对路径>`）对应的磁盘路径。
    pub fn disk_path(&self, game_id: &str, tree_path: &str) -> Result<PathBuf> {
        let locations = self.locations(game_id)?;
        let (ord, rel) = tree_path
            .split_once('/')
            .ok_or_else(|| AppError::Invalid(format!("无效的清单路径 {tree_path}")))?;
        let loc = ord
            .parse::<usize>()
            .ok()
            .and_then(|i| locations.get(i))
            .ok_or_else(|| {
                AppError::Invalid(format!("清单路径 {tree_path} 对应的存档位置不存在"))
            })?;
        if loc.kind == LocationKind::Registry {
            return Ok(self.registry_file(game_id, ord.parse().unwrap_or(0)));
        }
        // 导入的未安装游戏：需要安装目录的位置还没有确定。
        if loc.resolved_path.is_empty() {
            return Err(AppError::Invalid(format!(
                "存档位置 {} 在这台电脑上还没有确定（游戏未安装）；装好游戏并扫描后再恢复",
                loc.path_template
            )));
        }
        let base = match loc.kind {
            LocationKind::File => return Ok(PathBuf::from(&loc.resolved_path)),
            LocationKind::Glob => glob_base(&loc.resolved_path),
            _ => PathBuf::from(&loc.resolved_path),
        };
        let mut p = base;
        for part in rel.split('/') {
            if part.is_empty() || part == "." || part == ".." {
                return Err(AppError::Invalid(format!(
                    "清单路径 {tree_path} 含非法片段"
                )));
            }
            p.push(part);
        }
        Ok(p)
    }

    /// 从对象库重建索引，并在游戏表为空时从 `games.toml` 恢复游戏。
    pub(crate) fn reindex_raw(&mut self, _lock: &RepoLock) -> Result<ReindexStats> {
        if self.index.games()?.is_empty() {
            self.restore_games_file()?;
        }
        let stats = bonfire_core::index::reindex(&self.repo, &mut self.index)?;
        self.reindex_albums()?;
        Ok(stats)
    }

    fn games_file(&self) -> PathBuf {
        self.repo.root().join(GAMES_FILE)
    }

    pub(crate) fn save_games_file(&self) -> Result<()> {
        let mut file = GamesFile { game: Vec::new() };
        for g in self.index.games()? {
            let location = self.index.locations(&g.id)?;
            let shots = self.shot_settings_of(&g.id)?;
            let processes = self.index.processes(&g.id)?;
            file.game.push(GameEntry {
                record: g,
                location,
                shots,
                processes,
            });
        }
        let text = format!(
            "# 篝火 Bonfire 游戏配置备份（自动生成，索引数据库丢失时用于恢复）\n\n{}",
            toml::to_string_pretty(&file).expect("游戏配置序列化不会失败")
        );
        let path = self.games_file();
        let tmp = path.with_extension("toml.tmp");
        // 加密仓库中加密保存（M5-2）。
        let data = self.repo.seal(text.into_bytes())?;
        std::fs::write(&tmp, data).map_err(|e| AppError::io(&tmp, e))?;
        std::fs::rename(&tmp, &path).map_err(|e| AppError::io(&path, e))
    }

    /// 从 `games.toml` 恢复游戏，返回恢复的数量。
    fn restore_games_file(&mut self) -> Result<usize> {
        let path = self.games_file();
        let bytes = match std::fs::read(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(AppError::io(path, e)),
        };
        let text = String::from_utf8_lossy(&self.repo.unseal(bytes, "游戏列表")?).into_owned();
        let file: GamesFile = toml::from_str(&text)
            .map_err(|e| AppError::Invalid(format!("{} 格式错误：{e}", path.display())))?;
        for entry in &file.game {
            self.index.upsert_game(&entry.record)?;
            self.index
                .set_locations(&entry.record.id, &entry.location)?;
            if entry.shots != ShotSettings::default() {
                self.index
                    .save_shot_settings(&entry.record.id, &entry.shots)?;
            }
            self.index
                .set_processes(&entry.record.id, &entry.processes)?;
        }
        Ok(file.game.len())
    }
}

fn absolute(p: &Path) -> Result<PathBuf> {
    if !p.exists() {
        return Err(AppError::PathNotFound(p.to_path_buf()));
    }
    let abs = std::path::absolute(p).map_err(|e| AppError::io(p, e))?;
    Ok(abs)
}

/// 移出游戏库时一并删除的备份（默认都不删）。
#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveOptions {
    /// 删除备份的存档：全部快照、分支与标签。
    pub snapshots: bool,
    /// 删除备份的截图：相册与回收站。
    pub shots: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveReport {
    pub game: GameRecord,
    /// 删除的快照数。
    pub snapshots: usize,
    /// 删除的截图数。
    pub shots: usize,
}
