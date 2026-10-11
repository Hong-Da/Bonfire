//! Tauri 命令层：只做参数转换、线程调度与事件推送，业务逻辑全部调用 bonfire-app。
//!
//! 所有命令在后台线程执行（`spawn_blocking`），通过共享的 `Mutex<App>` 串行访问仓库；
//! 长任务逐个游戏推送事件，并在游戏之间检查取消标志。

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use bonfire_app::app::{RemoveOptions, RemoveReport};
use bonfire_app::auto::{AutoKind, AutoReport};
use bonfire_app::backup::{BackupOptions, BackupResult, DetachedChoice};
use bonfire_app::core::index::LocationKind;
use bonfire_app::core::index::{GameRecord, Profile};
use bonfire_app::core::refs::RefKind;
use bonfire_app::gallery::{GalleryView, ShotDiff, WriteBackReport};
use bonfire_app::library::{GameRow, HistoryView, SearchHit};
use bonfire_app::prune::DeleteReport;
use bonfire_app::restore::{RestoreOptions, RestorePlan, RestoreReport};
use bonfire_app::shots::{
    AlbumView, ManageMode, PurgeReport, ShotCandidate, ShotExportReport, ShotSyncReport,
};
use bonfire_app::transfer::{
    ArchiveImportItem, ArchivePick, ArchivePreview, BundleImportReport, BundlePreview,
    BundleReport, BundleScope, CollectionReport, ExportFormat, ExportReport, FilesAutoReport,
    FilesPreview, FilesSource, ImportFilesOptions, ImportFilesReport, ImportPlace, PackageOptions,
};
use bonfire_app::{App, AppError, AppPaths, Config, ErrorKind, Locale, NewGame};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

/// 应用共享状态。首次运行完成向导之前 `app` 为空（避免在用户选择仓库位置前就创建默认仓库）。
pub struct Shared {
    pub app: Mutex<Option<App>>,
    pub paths: AppPaths,
    pub cancel: AtomicBool,
    pub config_error: Mutex<Option<String>>,
    /// 这次关闭窗口留在托盘（询问时选了“最小化到托盘”但没有记住选择）。
    pub tray_once: AtomicBool,
    /// 上次询问关闭方式的时间：询问后很快再次关闭时按默认处理，避免界面无响应时窗口关不掉。
    pub close_asked_at: Mutex<Option<std::time::Instant>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CmdError {
    pub kind: &'static str,
    pub message: String,
}

impl From<AppError> for CmdError {
    fn from(e: AppError) -> Self {
        let kind = match e.kind() {
            ErrorKind::General => "general",
            ErrorKind::Usage => "usage",
            ErrorKind::Config => "config",
            ErrorKind::NotFound => "not-found",
            ErrorKind::External => "external",
            ErrorKind::Locked => "locked",
            ErrorKind::GameRunning => "game-running",
        };
        Self {
            kind,
            message: e.to_string(),
        }
    }
}

impl From<bonfire_app::core::CoreError> for CmdError {
    fn from(e: bonfire_app::core::CoreError) -> Self {
        AppError::from(e).into()
    }
}

impl CmdError {
    fn not_ready() -> Self {
        Self {
            kind: "not-ready",
            message: "尚未完成首次配置".into(),
        }
    }

    fn cancelled() -> Self {
        Self {
            kind: "cancelled",
            message: "已取消".into(),
        }
    }
}

type CmdResult<T> = Result<T, CmdError>;

/// 打开（或重新打开）应用上下文，并允许界面读取图片缓存目录。
pub fn open_app(h: &AppHandle, shared: &Shared, config: Config) -> Result<(), AppError> {
    let locale = Locale::resolve(config.general.language.as_deref());
    let repo_dir = config.repo_dir(&shared.paths.default_repo_dir);
    let app = App::open(shared.paths.clone(), config, locale, &repo_dir)?;
    std::fs::create_dir_all(&shared.paths.cache_dir)
        .map_err(|e| AppError::io(&shared.paths.cache_dir, e))?;
    if let Err(e) = h
        .asset_protocol_scope()
        .allow_directory(&shared.paths.cache_dir, true)
    {
        tracing::warn!("无法开放图片缓存目录：{e}");
    }
    // 查看大图时直接读取备份中的截图原图。
    let shots_dir = app.repo.root().join(bonfire_app::shots::SHOTS_DIR);
    if let Err(e) = h.asset_protocol_scope().allow_directory(&shots_dir, true) {
        tracing::warn!("无法开放截图目录：{e}");
    }
    *shared.app.lock().unwrap() = Some(app);
    Ok(())
}

/// 在后台线程中使用应用上下文。
async fn with_app<T, F>(h: AppHandle, f: F) -> CmdResult<T>
where
    T: Send + 'static,
    F: FnOnce(&mut App, &AppHandle) -> Result<T, CmdError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let shared = h.state::<Shared>();
        let mut guard = shared.app.lock().unwrap_or_else(|p| p.into_inner());
        let app = guard.as_mut().ok_or_else(CmdError::not_ready)?;
        f(app, &h)
    })
    .await
    .map_err(|e| CmdError {
        kind: "general",
        message: e.to_string(),
    })?
}

fn find(app: &App, id: &str) -> CmdResult<GameRecord> {
    Ok(app.find_game(id)?)
}

// ---------- 应用与设置 ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: &'static str,
    portable: bool,
    first_run: bool,
    locale: &'static str,
    theme: bonfire_app::config::Theme,
    config_path: PathBuf,
    default_repo_dir: PathBuf,
    repo_dir: Option<PathBuf>,
    config_error: Option<String>,
    /// 本机电脑名（快照记录的电脑名与它相同即为本机备份）。
    host: String,
    /// 存档安全保护是否开启。
    protect_local: bool,
    /// 记住游戏库的筛选 / 勾选的游戏。
    remember_filters: bool,
    remember_selection: bool,
}

#[tauri::command]
pub async fn app_info(h: AppHandle) -> CmdResult<AppInfo> {
    let shared = h.state::<Shared>();
    let guard = shared.app.lock().unwrap_or_else(|p| p.into_inner());
    let (locale, theme, repo_dir, protect_local) = match guard.as_ref() {
        Some(a) => (
            a.locale,
            a.config.general.theme,
            Some(a.repo.root().to_path_buf()),
            a.config.backup.protect_local,
        ),
        None => (Locale::resolve(None), Default::default(), None, true),
    };
    let general = guard
        .as_ref()
        .map(|a| a.config.general.clone())
        .unwrap_or_default();
    Ok(AppInfo {
        version: bonfire_app::VERSION,
        portable: shared.paths.portable,
        first_run: guard.is_none(),
        locale: locale.code(),
        theme,
        config_path: shared.paths.config_file.clone(),
        default_repo_dir: shared.paths.default_repo_dir.clone(),
        repo_dir,
        config_error: shared.config_error.lock().unwrap().clone(),
        host: bonfire_app::util::host_name(),
        protect_local,
        remember_filters: general.remember_filters,
        remember_selection: general.remember_selection,
    })
}

#[tauri::command]
pub async fn get_settings(h: AppHandle) -> CmdResult<Config> {
    let shared = h.state::<Shared>();
    let guard = shared.app.lock().unwrap_or_else(|p| p.into_inner());
    Ok(guard.as_ref().map(|a| a.config.clone()).unwrap_or_default())
}

/// 保存设置（首次运行向导完成时也调用）。仓库位置改变或尚未打开时重新打开仓库。
#[tauri::command]
pub async fn save_settings(h: AppHandle, config: Config) -> CmdResult<AppInfo> {
    let h2 = h.clone();
    tauri::async_runtime::spawn_blocking(move || -> CmdResult<()> {
        config.validate().map_err(AppError::from)?;
        let shared = h2.state::<Shared>();
        config
            .save(&shared.paths.config_file)
            .map_err(AppError::from)?;
        *shared.config_error.lock().unwrap() = None;
        let reopen = {
            let guard = shared.app.lock().unwrap_or_else(|p| p.into_inner());
            match guard.as_ref() {
                None => true,
                Some(a) => a.repo.root() != config.repo_dir(&shared.paths.default_repo_dir),
            }
        };
        if reopen {
            open_app(&h2, &shared, config)?;
        } else if let Some(a) = shared
            .app
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_mut()
        {
            a.locale = Locale::resolve(config.general.language.as_deref());
            a.config = config;
        }
        Ok(())
    })
    .await
    .map_err(|e| CmdError {
        kind: "general",
        message: e.to_string(),
    })??;
    app_info(h).await
}

// ---------- 游戏库 ----------

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Phase {
    phase: &'static str,
    done: usize,
    total: usize,
}

/// 加载游戏库（只含已加入管理的游戏）：逐个检测状态并推送 `library-row` 事件；
/// `fetch_info` 时再逐个补全游戏信息（新游戏与过期的联网获取），获取一个就更新一行。
#[tauri::command]
pub async fn load_library(
    h: AppHandle,
    fetch_info: bool,
    force_verify: bool,
) -> CmdResult<Vec<GameRow>> {
    with_app(h, move |app, h| {
        let shared = h.state::<Shared>();
        shared.cancel.store(false, Ordering::SeqCst);
        let games = app.games()?;
        let total = games.len();
        let mut rows = Vec::new();
        for (i, g) in games.iter().enumerate() {
            if shared.cancel.load(Ordering::SeqCst) {
                return Err(CmdError::cancelled());
            }
            let _ = h.emit(
                "library-phase",
                Phase {
                    phase: "checking",
                    done: i,
                    total,
                },
            );
            match app.game_row(g, force_verify) {
                Ok(row) => {
                    let _ = h.emit("library-row", &row);
                    rows.push(row);
                }
                Err(e) => tracing::warn!(game = %g.id, "检测失败：{e}"),
            }
        }
        if fetch_info {
            for (i, row) in rows.iter_mut().enumerate() {
                if shared.cancel.load(Ordering::SeqCst) {
                    break;
                }
                let _ = h.emit(
                    "library-phase",
                    Phase {
                        phase: "info",
                        done: i,
                        total,
                    },
                );
                let Ok(g) = app.find_game(&row.id) else {
                    continue;
                };
                if let Err(e) = app.refresh_metadata(&g, false) {
                    tracing::info!(game = %g.id, "获取游戏信息失败：{e}");
                    continue;
                }
                if let Ok(updated) = app.game_row(&g, false) {
                    let _ = h.emit("library-row", &updated);
                    *row = updated;
                }
            }
        }
        let _ = h.emit(
            "library-phase",
            Phase {
                phase: "idle",
                done: total,
                total,
            },
        );
        Ok(rows)
    })
    .await
}

/// 发现游戏：识别本机平台上的游戏与存档，不加入游戏库，由用户选择。
#[tauri::command]
pub async fn discover_games(h: AppHandle) -> CmdResult<bonfire_app::discover::ScanReport> {
    with_app(h, move |app, h| {
        let _ = h.emit("scan-phase", "rules");
        // 规则库缺失或过期时先更新（离线或失败时继续用已有规则）。
        app.ensure_rules();
        let _ = h.emit("scan-phase", "discovering");
        app.fetch_metadata_on_scan = false;
        let lock = app.repo.lock()?;
        let r = app.scan_platforms(&lock, None);
        app.fetch_metadata_on_scan = true;
        let _ = h.emit("scan-phase", "idle");
        Ok(r?)
    })
    .await
}

/// 把发现的游戏加入游戏库。
#[tauri::command]
pub async fn include_games(h: AppHandle, ids: Vec<String>) -> CmdResult<Vec<GameRecord>> {
    with_app(h, move |app, _| {
        let lock = app.repo.lock()?;
        Ok(app.include_games(&lock, &ids, false)?)
    })
    .await
}

/// 忽略发现的游戏（已在库中的会移出游戏库，历史快照保留）。
#[tauri::command]
pub async fn ignore_games(h: AppHandle, ids: Vec<String>) -> CmdResult<Vec<GameRecord>> {
    with_app(h, move |app, _| {
        let lock = app.repo.lock()?;
        Ok(app.ignore_games(&lock, &ids)?)
    })
    .await
}

#[tauri::command]
pub async fn game_row(h: AppHandle, game_id: String) -> CmdResult<GameRow> {
    with_app(h, move |app, _| {
        Ok(app.game_row(&find(app, &game_id)?, false)?)
    })
    .await
}

#[tauri::command]
pub async fn game_changes(
    h: AppHandle,
    game_id: String,
) -> CmdResult<Vec<bonfire_app::core::scan::FileChange>> {
    with_app(h, move |app, _| {
        Ok(app.changed_files(&find(app, &game_id)?)?)
    })
    .await
}

#[tauri::command]
pub async fn cancel_task(h: AppHandle) -> CmdResult<()> {
    h.state::<Shared>().cancel.store(true, Ordering::SeqCst);
    Ok(())
}

// ---------- 备份 ----------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupItem {
    game_id: String,
    comment: Option<String>,
    tag: Option<String>,
    /// 另开新路线（手动替换成下载的存档后）。
    #[serde(default)]
    new_branch: Option<String>,
    /// 备份存档（默认是；只管理截图的游戏始终不备份存档）。
    #[serde(default = "yes")]
    saves: bool,
    /// 收集新截图（默认是）。
    #[serde(default = "yes")]
    shots: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", tag = "mode", content = "name")]
pub enum OnDetached {
    New,
    Named(String),
    Move,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupItemResult {
    game_id: String,
    ok: bool,
    result: Option<BackupResult>,
    error: Option<CmdError2>,
    /// 顺带收集的新截图数。
    shots: usize,
    /// 只管理截图的游戏：没有备份存档，只收集了截图。
    shots_only: bool,
}

/// CmdError 的可克隆版本（事件负载需要 Clone）。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CmdError2 {
    kind: &'static str,
    message: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BackupProgress {
    index: usize,
    total: usize,
    item: BackupItemResult,
}

/// 备份所选游戏：逐个执行，单个失败不影响其他游戏（文档 8.1），每完成一个推送 `backup-progress`。
#[tauri::command]
pub async fn run_backup(
    h: AppHandle,
    items: Vec<BackupItem>,
    force: bool,
    on_detached: OnDetached,
) -> CmdResult<Vec<BackupItemResult>> {
    with_app(h, move |app, h| {
        let shared = h.state::<Shared>();
        shared.cancel.store(false, Ordering::SeqCst);
        let lock = app.repo.lock()?;
        let total = items.len();
        let mut out = Vec::new();
        for (index, item) in items.into_iter().enumerate() {
            if shared.cancel.load(Ordering::SeqCst) {
                break;
            }
            let opts = BackupOptions {
                comment: item.comment.filter(|c| !c.trim().is_empty()),
                tag: item.tag.filter(|t| !t.trim().is_empty()),
                new_branch: item.new_branch.filter(|b| !b.trim().is_empty()),
                force,
                on_detached: match &on_detached {
                    OnDetached::New => DetachedChoice::NewBranch,
                    OnDetached::Named(n) => DetachedChoice::NamedBranch(n.clone()),
                    OnDetached::Move => DetachedChoice::MoveCurrent,
                },
                ..Default::default()
            };
            // 按选择备份存档、收集新截图（截图失败只记日志，不影响备份结果）。
            // 只管理截图的游戏不备份存档，只收集截图。
            let shots_only = !item.saves || app.is_shots_only(&item.game_id).unwrap_or(false);
            let (r, shots) = match app.find_game(&item.game_id) {
                Ok(g) => {
                    let r = if shots_only {
                        Ok(BackupResult::NoChanges)
                    } else {
                        app.backup(&lock, &g, &opts)
                    };
                    let shots = if !item.shots {
                        0
                    } else {
                        match app.sync_shots(&lock, &g) {
                            Ok(s) => s.added,
                            Err(e) => {
                                tracing::warn!(game = %g.id, "收集截图失败：{e}");
                                0
                            }
                        }
                    };
                    (r, shots)
                }
                Err(e) => (Err(e), 0),
            };
            let res = match r {
                Ok(r) => BackupItemResult {
                    game_id: item.game_id,
                    ok: true,
                    result: (!shots_only).then_some(r),
                    error: None,
                    shots,
                    shots_only,
                },
                Err(e) => {
                    let e = CmdError::from(e);
                    BackupItemResult {
                        game_id: item.game_id,
                        ok: false,
                        result: None,
                        error: Some(CmdError2 {
                            kind: e.kind,
                            message: e.message,
                        }),
                        shots,
                        shots_only,
                    }
                }
            };
            let _ = h.emit(
                "backup-progress",
                BackupProgress {
                    index,
                    total,
                    item: res.clone(),
                },
            );
            out.push(res);
        }
        Ok(out)
    })
    .await
}

// ---------- 历史 ----------

#[tauri::command]
pub async fn history(h: AppHandle, game_id: String) -> CmdResult<HistoryView> {
    with_app(
        h,
        move |app, _| Ok(app.history_view(&find(app, &game_id)?)?),
    )
    .await
}

#[tauri::command]
pub async fn snapshot_files(
    h: AppHandle,
    game_id: String,
    snapshot: String,
) -> CmdResult<Vec<bonfire_app::core::TreeEntry>> {
    with_app(h, move |app, _| {
        Ok(app.snapshot_files(&find(app, &game_id)?, &snapshot)?)
    })
    .await
}

#[tauri::command]
pub async fn diff(
    h: AppHandle,
    game_id: String,
    a: String,
    b: Option<String>,
) -> CmdResult<Vec<bonfire_app::DiffEntry>> {
    with_app(h, move |app, _| {
        Ok(app.diff(&find(app, &game_id)?, &a, b.as_deref())?)
    })
    .await
}

#[tauri::command]
pub async fn set_comment(
    h: AppHandle,
    game_id: String,
    snapshot: String,
    comment: String,
) -> CmdResult<()> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        let lock = app.repo.lock()?;
        app.set_comment(&lock, &g, &snapshot, &comment)?;
        Ok(())
    })
    .await
}

/// 删除分支（`dry_run` 时只返回会删除的快照）。
/// 预览导入下载的存档（U-42）。密码只用于这次读取。
#[tauri::command]
pub async fn inspect_download(
    h: AppHandle,
    game_id: String,
    path: PathBuf,
    password: Option<String>,
) -> CmdResult<bonfire_app::downloads::DownloadPreview> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        Ok(app.inspect_download(&g, &path, password.as_deref())?)
    })
    .await
}

/// 导入下载的存档为新路线（不写入存档目录）。
#[tauri::command]
pub async fn import_download(
    h: AppHandle,
    game_id: String,
    path: PathBuf,
    password: Option<String>,
    request: bonfire_app::downloads::DownloadImport,
) -> CmdResult<bonfire_app::downloads::DownloadReport> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        let lock = app.repo.lock()?;
        Ok(app.import_download(&lock, &g, &path, password.as_deref(), &request)?)
    })
    .await
}

/// 设为本机主存档（`tidy` 时整理名称）。
#[tauri::command]
pub async fn set_primary_branch(
    h: AppHandle,
    game_id: String,
    branch: String,
    tidy: bool,
) -> CmdResult<bonfire_app::primary::SetPrimaryReport> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        let lock = app.repo.lock()?;
        Ok(app.set_primary(&lock, &g, &branch, tidy)?)
    })
    .await
}

/// 用系统浏览器打开篝火的代码仓库（只能打开 `branding::REPOS` 中的地址）。
#[tauri::command]
pub async fn open_repo(index: usize) -> CmdResult<()> {
    let (_, url) = bonfire_app::branding::REPOS
        .get(index)
        .copied()
        .ok_or_else(|| CmdError {
            kind: "invalid",
            message: "没有这个仓库地址".into(),
        })?;
    std::process::Command::new("explorer")
        .arg(url)
        .spawn()
        .map_err(|e| CmdError {
            kind: "general",
            message: e.to_string(),
        })?;
    Ok(())
}

/// 分支改名。
#[tauri::command]
pub async fn rename_branch(
    h: AppHandle,
    game_id: String,
    from: String,
    to: String,
) -> CmdResult<()> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        let lock = app.repo.lock()?;
        Ok(app.rename_branch(&lock, &g, &from, &to)?)
    })
    .await
}

#[tauri::command]
pub async fn delete_branch(
    h: AppHandle,
    game_id: String,
    name: String,
    dry_run: bool,
) -> CmdResult<DeleteReport> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        let lock = app.repo.lock()?;
        Ok(app.delete_branch(&lock, &g, &name, dry_run)?)
    })
    .await
}

#[tauri::command]
pub async fn delete_tag(h: AppHandle, game_id: String, name: String) -> CmdResult<()> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        let lock = app.repo.lock()?;
        Ok(app.delete_tag(&lock, &g, &name)?)
    })
    .await
}

/// 删除快照（`dry_run` 时只返回影响范围）。
#[tauri::command]
pub async fn delete_snapshot(
    h: AppHandle,
    game_id: String,
    snapshot: String,
    dry_run: bool,
) -> CmdResult<DeleteReport> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        let lock = app.repo.lock()?;
        Ok(app.delete_snapshot(&lock, &g, &snapshot, dry_run)?)
    })
    .await
}

#[tauri::command]
pub async fn create_ref(
    h: AppHandle,
    game_id: String,
    kind: RefKind,
    name: String,
    snapshot: Option<String>,
) -> CmdResult<()> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        let lock = app.repo.lock()?;
        app.create_ref(&lock, &g, kind, &name, snapshot.as_deref())?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn search_comments(h: AppHandle, query: String) -> CmdResult<Vec<SearchHit>> {
    with_app(h, move |app, _| Ok(app.search_all_comments(&query)?)).await
}

// ---------- 恢复 ----------

#[tauri::command]
pub async fn plan_restore(
    h: AppHandle,
    game_id: String,
    target: String,
    paths: Vec<String>,
    full: bool,
) -> CmdResult<RestorePlan> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        Ok(app.plan_restore(
            &g,
            &target,
            &RestoreOptions {
                paths,
                full,
                dry_run: true,
                force: false,
            },
        )?)
    })
    .await
}

#[tauri::command]
pub async fn restore(
    h: AppHandle,
    game_id: String,
    target: String,
    paths: Vec<String>,
    full: bool,
    force: bool,
) -> CmdResult<RestoreReport> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        let lock = app.repo.lock()?;
        Ok(app.restore(
            &lock,
            &g,
            &target,
            &RestoreOptions {
                paths,
                full,
                dry_run: false,
                force,
            },
        )?)
    })
    .await
}

#[tauri::command]
pub async fn undo_restore(h: AppHandle, game_id: String) -> CmdResult<RestoreReport> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        let lock = app.repo.lock()?;
        Ok(app.undo_restore(&lock, &g)?)
    })
    .await
}

#[tauri::command]
pub async fn pending_undo(h: AppHandle, game_id: String) -> CmdResult<Option<serde_json::Value>> {
    with_app(h, move |app, _| Ok(app.pending_undo(&game_id)?)).await
}

// ---------- 游戏管理与信息 ----------

#[tauri::command]
pub async fn add_game(
    h: AppHandle,
    name: String,
    save_paths: Vec<PathBuf>,
    config_paths: Vec<PathBuf>,
    install_dir: Option<PathBuf>,
) -> CmdResult<GameRecord> {
    with_app(h, move |app, _| {
        let lock = app.repo.lock()?;
        let g = app.add_game(
            &lock,
            &NewGame {
                name,
                save_paths,
                config_paths,
                install_dir,
            },
        )?;
        let _ = app.refresh_metadata(&g, false);
        Ok(g)
    })
    .await
}

/// 移出游戏库；`options` 选择是否同时删除备份的存档（快照）与截图（默认都不删）。
#[tauri::command]
pub async fn remove_game(
    h: AppHandle,
    game_id: String,
    options: Option<RemoveOptions>,
) -> CmdResult<RemoveReport> {
    with_app(h, move |app, _| {
        let lock = app.repo.lock()?;
        Ok(app.remove_game_with(&lock, &game_id, &options.unwrap_or_default())?)
    })
    .await
}

#[tauri::command]
pub async fn confirm_locations(h: AppHandle, game_id: String) -> CmdResult<usize> {
    with_app(h, move |app, _| {
        let lock = app.repo.lock()?;
        Ok(app.confirm_locations(&lock, &game_id)?.1)
    })
    .await
}

#[tauri::command]
pub async fn ignored_games(h: AppHandle) -> CmdResult<Vec<GameRecord>> {
    with_app(h, move |app, _| {
        Ok(app
            .all_games()?
            .into_iter()
            .filter(|g| !g.enabled)
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn enable_game(h: AppHandle, game_id: String) -> CmdResult<GameRecord> {
    with_app(h, move |app, _| {
        let lock = app.repo.lock()?;
        Ok(app.enable_game(&lock, &game_id)?)
    })
    .await
}

#[tauri::command]
pub async fn game_info(
    h: AppHandle,
    game_id: String,
) -> CmdResult<Option<bonfire_app::metadata::MetadataView>> {
    with_app(h, move |app, _| Ok(app.metadata(&game_id)?)).await
}

#[tauri::command]
pub async fn refresh_info(h: AppHandle, game_id: String) -> CmdResult<()> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        app.refresh_metadata(&g, true)?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn set_info_field(
    h: AppHandle,
    game_id: String,
    field: String,
    value: String,
) -> CmdResult<()> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        app.set_metadata_field(&g, &field, &value)?;
        Ok(())
    })
    .await
}

// ---------- 备份方案 ----------

#[tauri::command]
pub async fn profiles(h: AppHandle) -> CmdResult<Vec<Profile>> {
    with_app(h, move |app, _| {
        Ok(app.index.profiles().map_err(AppError::from)?)
    })
    .await
}

#[tauri::command]
pub async fn save_profile(h: AppHandle, profile: Profile) -> CmdResult<()> {
    with_app(h, move |app, _| {
        if profile.name.trim().is_empty() {
            return Err(AppError::Invalid("方案名称不能为空".into()).into());
        }
        app.index.save_profile(&profile).map_err(AppError::from)?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn delete_profile(h: AppHandle, name: String) -> CmdResult<()> {
    with_app(h, move |app, _| {
        app.index.delete_profile(&name).map_err(AppError::from)?;
        Ok(())
    })
    .await
}

// ---------- 维护 ----------

#[tauri::command]
pub async fn fsck(h: AppHandle, deep: bool) -> CmdResult<bonfire_app::core::fsck::FsckReport> {
    with_app(h, move |app, _| Ok(app.fsck(deep)?)).await
}

/// 清理历史（M4-4）：按保留策略压掉多余的快照，再回收不再使用的空间。`dry_run` 时只预览。
#[tauri::command]
pub async fn clean_history(
    h: AppHandle,
    dry_run: bool,
) -> CmdResult<bonfire_app::retention::CleanupReport> {
    with_app(h, move |app, _| {
        let lock = app.repo.lock()?;
        let opts = bonfire_app::retention::CleanupOptions {
            dry_run,
            ..Default::default()
        };
        Ok(app.clean_history(&lock, &opts)?)
    })
    .await
}

#[tauri::command]
pub async fn reindex(h: AppHandle) -> CmdResult<bonfire_app::core::index::ReindexStats> {
    with_app(h, move |app, _| {
        let lock = app.repo.lock()?;
        Ok(app.reindex(&lock)?)
    })
    .await
}

#[tauri::command]
pub async fn update_rules(
    h: AppHandle,
) -> CmdResult<bonfire_app::discovery::manifest::UpdateOutcome> {
    with_app(h, move |app, _| {
        Ok(app.update_rules(bonfire_app::discovery::manifest::MANIFEST_URL, None)?)
    })
    .await
}

#[tauri::command]
pub async fn rules_info(h: AppHandle) -> CmdResult<bonfire_app::discovery::manifest::LoadedFrom> {
    with_app(h, move |app, _| Ok(app.load_rules()?.1)).await
}

/// 在资源管理器中打开文件夹（或选中文件）。路径不存在时打开最近的上级目录。
#[tauri::command]
pub async fn open_path(path: PathBuf) -> CmdResult<()> {
    let path = explorer_target(&path);
    let mut cmd = std::process::Command::new("explorer");
    let arg = explorer_arg(&path, path.is_file());
    #[cfg(windows)]
    {
        // 资源管理器不按常规规则解析参数：标准库给含空格的参数整体加引号（`"/select,D:\x y.zip"`），
        // 它不认识这种写法，会打开默认的“文档”目录。只能原样传入 `/select,"D:\x y.zip"`。
        use std::os::windows::process::CommandExt;
        cmd.raw_arg(arg);
    }
    #[cfg(not(windows))]
    cmd.arg(arg);
    cmd.spawn().map_err(|e| CmdError {
        kind: "general",
        message: e.to_string(),
    })?;
    Ok(())
}

/// 规范化要打开的路径：正斜杠转为反斜杠（资源管理器不认识 `/`，会打开默认目录），
/// 去掉 `\\?\` 前缀，不存在时取最近的存在的上级目录。
fn explorer_target(path: &std::path::Path) -> PathBuf {
    let s = path.to_string_lossy().replace('/', "\\");
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s).to_string();
    let p = PathBuf::from(s);
    p.ancestors()
        .find(|a| !a.as_os_str().is_empty() && a.exists())
        .map(PathBuf::from)
        .unwrap_or(p)
}

/// 资源管理器的命令行：文件为 `/select,"<路径>"`，目录为 `"<路径>"`。
fn explorer_arg(path: &std::path::Path, is_file: bool) -> String {
    let quoted = format!("\"{}\"", path.display());
    if is_file {
        format!("/select,{quoted}")
    } else {
        quoted
    }
}

// ---------- 导出与导入 ----------

/// 打开一个存档位置：目录直接打开，通配位置打开其固定前缀目录，单个文件在资源管理器中选中。
#[tauri::command]
pub async fn open_location(h: AppHandle, game_id: String, ord: usize) -> CmdResult<()> {
    let path = with_app(h, move |app, _| {
        let loc = app
            .locations(&game_id)?
            .into_iter()
            .nth(ord)
            .ok_or_else(|| AppError::Invalid(format!("存档位置 {ord} 不存在")))?;
        Ok(match loc.kind {
            // 不含通配符的“通配位置”本身就是目录（如 `.../SaveGames`），直接打开它。
            LocationKind::Glob if !loc.resolved_path.contains(['*', '?', '[']) => {
                PathBuf::from(&loc.resolved_path)
            }
            LocationKind::Glob => bonfire_app::core::scan::glob_base(&loc.resolved_path),
            LocationKind::Registry => {
                return Err(AppError::Invalid("注册表存档没有文件夹".into()).into());
            }
            _ => PathBuf::from(&loc.resolved_path),
        })
    })
    .await?;
    open_path(path).await
}

#[tauri::command]
pub async fn export_default_name(
    h: AppHandle,
    game_id: String,
    spec: Option<String>,
) -> CmdResult<String> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        Ok(app.export_default_name(&g, spec.as_deref())?)
    })
    .await
}

#[tauri::command]
pub async fn export_files(
    h: AppHandle,
    game_id: String,
    spec: Option<String>,
    dest: PathBuf,
    format: ExportFormat,
) -> CmdResult<ExportReport> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        Ok(app.export_files(&g, spec.as_deref(), &dest, format)?)
    })
    .await
}

#[tauri::command]
pub async fn export_bundle(
    h: AppHandle,
    game_id: String,
    scope: BundleScope,
    dest: PathBuf,
    include_shots: bool,
) -> CmdResult<BundleReport> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        Ok(app.export_bundle(&g, &scope, &dest, include_shots)?)
    })
    .await
}

#[tauri::command]
pub async fn inspect_bundle(
    h: AppHandle,
    path: PathBuf,
    game_id: Option<String>,
) -> CmdResult<BundlePreview> {
    with_app(h, move |app, _| {
        Ok(app.inspect_bundle(&path, game_id.as_deref())?)
    })
    .await
}

#[tauri::command]
pub async fn import_bundle(
    h: AppHandle,
    path: PathBuf,
    game_id: Option<String>,
) -> CmdResult<BundleImportReport> {
    with_app(h, move |app, _| {
        let lock = app.repo.lock()?;
        Ok(app.import_bundle(&lock, &path, game_id.as_deref(), false)?)
    })
    .await
}

#[tauri::command]
pub async fn inspect_files(h: AppHandle, game_id: String, src: PathBuf) -> CmdResult<FilesPreview> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        Ok(app.inspect_files(&g, &src)?)
    })
    .await
}

#[tauri::command]
pub async fn import_files(
    h: AppHandle,
    game_id: String,
    src: PathBuf,
    place: ImportPlace,
    comment: Option<String>,
) -> CmdResult<ImportFilesReport> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        let lock = app.repo.lock()?;
        Ok(app.import_files(
            &lock,
            &g,
            &src,
            &ImportFilesOptions {
                place,
                comment,
                dry_run: false,
            },
        )?)
    })
    .await
}

#[tauri::command]
pub async fn export_collection(
    h: AppHandle,
    game_ids: Vec<String>,
    dest: PathBuf,
    include_shots: bool,
) -> CmdResult<CollectionReport> {
    with_app(h, move |app, _| {
        let games = game_ids
            .iter()
            .map(|id| find(app, id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(app.export_collection(&games, &dest, include_shots)?)
    })
    .await
}

/// 导出篝火包：一个或多个游戏的存档历史和 / 或截图（U-16）。
#[tauri::command]
pub async fn export_package(
    h: AppHandle,
    game_ids: Vec<String>,
    dest: PathBuf,
    options: PackageOptions,
) -> CmdResult<CollectionReport> {
    with_app(h, move |app, _| {
        let games = game_ids
            .iter()
            .map(|id| find(app, id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(app.export_package(&games, &dest, &options)?)
    })
    .await
}

/// 导出 zip 格式的篝火导出（U-49）：存档文件与截图原图。
#[tauri::command]
pub async fn export_package_zip(
    h: AppHandle,
    game_ids: Vec<String>,
    dest: PathBuf,
    options: PackageOptions,
) -> CmdResult<CollectionReport> {
    with_app(h, move |app, _| {
        let games = game_ids
            .iter()
            .map(|id| find(app, id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(app.export_package_zip(&games, &dest, &options)?)
    })
    .await
}

/// 把相册中的截图（如刚导入的）写到本机这个游戏的截图文件夹（U-16）。
#[tauri::command]
pub async fn sync_shots_to_disk(
    h: AppHandle,
    game_id: String,
    blobs: Vec<String>,
) -> CmdResult<WriteBackReport> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        let ids = blob_ids(&blobs)?;
        Ok(app.sync_shots_to_disk(&g, &ids)?)
    })
    .await
}

#[tauri::command]
pub async fn inspect_archive(h: AppHandle, path: PathBuf) -> CmdResult<ArchivePreview> {
    with_app(h, move |app, _| Ok(app.inspect_archive(&path)?)).await
}

#[tauri::command]
pub async fn import_archive(
    h: AppHandle,
    path: PathBuf,
    picks: Vec<ArchivePick>,
) -> CmdResult<Vec<ArchiveImportItem>> {
    with_app(h, move |app, _| {
        let lock = app.repo.lock()?;
        Ok(app.import_archive(&lock, &path, &picks, false)?)
    })
    .await
}

#[tauri::command]
pub async fn inspect_files_sources(h: AppHandle, src: PathBuf) -> CmdResult<Vec<FilesSource>> {
    with_app(h, move |app, _| Ok(app.inspect_files_sources(&src)?)).await
}

#[tauri::command]
pub async fn import_files_auto(
    h: AppHandle,
    src: PathBuf,
    dir: Option<String>,
    comment: Option<String>,
) -> CmdResult<FilesAutoReport> {
    with_app(h, move |app, _| {
        let lock = app.repo.lock()?;
        Ok(app.import_files_auto(&lock, &src, dir.as_deref(), comment, false)?)
    })
    .await
}

// ---------- 截图相册 ----------

fn blob_ids(blobs: &[String]) -> CmdResult<Vec<bonfire_app::core::ObjectId>> {
    blobs
        .iter()
        .map(|b| {
            b.parse()
                .map_err(|_| CmdError::from(AppError::Invalid(format!("无效的截图 ID：{b}"))))
        })
        .collect()
}

#[tauri::command]
pub async fn album(h: AppHandle, game_id: String) -> CmdResult<AlbumView> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        Ok(app.album(&g)?)
    })
    .await
}

/// 收集新截图；不指定游戏时收集全部游戏。
#[tauri::command]
pub async fn sync_shots(h: AppHandle, game_id: Option<String>) -> CmdResult<Vec<ShotSyncReport>> {
    with_app(h, move |app, _| {
        let lock = app.repo.lock()?;
        Ok(match game_id {
            Some(id) => {
                let g = find(app, &id)?;
                vec![app.sync_shots(&lock, &g)?]
            }
            None => app.sync_all_shots(&lock)?,
        })
    })
    .await
}

#[tauri::command]
pub async fn shot_thumb(h: AppHandle, game_id: String, blob: String) -> CmdResult<PathBuf> {
    with_app(h, move |app, _| {
        let id = blob_ids(&[blob])?[0];
        Ok(app.shot_thumb(&game_id, &id)?)
    })
    .await
}

#[tauri::command]
pub async fn shot_image(h: AppHandle, game_id: String, blob: String) -> CmdResult<PathBuf> {
    with_app(h, move |app, _| {
        let id = blob_ids(&[blob])?[0];
        Ok(app.shot_image(&game_id, &id)?)
    })
    .await
}

#[tauri::command]
pub async fn delete_shots(h: AppHandle, game_id: String, blobs: Vec<String>) -> CmdResult<usize> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        let ids = blob_ids(&blobs)?;
        let lock = app.repo.lock()?;
        Ok(app.delete_shots(&lock, &g, &ids)?)
    })
    .await
}

/// 从回收站恢复截图。
#[tauri::command]
pub async fn restore_shots(h: AppHandle, game_id: String, blobs: Vec<String>) -> CmdResult<usize> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        let ids = blob_ids(&blobs)?;
        let lock = app.repo.lock()?;
        Ok(app.restore_shots(&lock, &g, &ids)?)
    })
    .await
}

/// 彻底删除截图；`blobs` 为空时清空回收站；`originals` 时同时删除磁盘上的原图（移到系统回收站）。
#[tauri::command]
pub async fn purge_shots(
    h: AppHandle,
    game_id: String,
    blobs: Vec<String>,
    originals: bool,
) -> CmdResult<PurgeReport> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        let ids = blob_ids(&blobs)?;
        let lock = app.repo.lock()?;
        Ok(app.purge_shots_with(&lock, &g, &ids, originals)?)
    })
    .await
}

/// 设为只管理截图，或恢复管理存档。
#[tauri::command]
pub async fn set_shots_only(h: AppHandle, game_id: String, on: bool) -> CmdResult<()> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        Ok(app.set_shots_only(&g, on)?)
    })
    .await
}

/// 有截图、但还不在游戏库中的游戏（发现页）。
#[tauri::command]
pub async fn shot_candidates(h: AppHandle) -> CmdResult<Vec<ShotCandidate>> {
    with_app(h, move |app, _| Ok(app.shot_candidates()?)).await
}

/// 加入只管理截图的游戏：`key` 为候选的键（`steam:<AppID>` / `capture:<标题>`），手动添加时为空。
#[tauri::command]
pub async fn add_shots_game(
    h: AppHandle,
    key: Option<String>,
    name: String,
    dirs: Vec<PathBuf>,
) -> CmdResult<GameRecord> {
    with_app(h, move |app, _| {
        let lock = app.repo.lock()?;
        Ok(app.add_shots_game(&lock, key.as_deref(), &name, &dirs)?)
    })
    .await
}

/// 切换游戏的管理方式：存档和截图 / 只管理存档 / 只管理截图。
#[tauri::command]
pub async fn set_manage_mode(
    h: AppHandle,
    game_ids: Vec<String>,
    mode: ManageMode,
) -> CmdResult<()> {
    with_app(h, move |app, _| {
        for id in game_ids {
            let g = find(app, &id)?;
            app.set_manage_mode(&g, mode)?;
        }
        Ok(())
    })
    .await
}

/// 图库：所有游戏的截图。
#[tauri::command]
pub async fn gallery(h: AppHandle) -> CmdResult<GalleryView> {
    with_app(h, move |app, _| Ok(app.gallery()?)).await
}

/// 对比截图备份与磁盘上的截图文件夹。
#[tauri::command]
pub async fn shot_diff(h: AppHandle, game_id: String) -> CmdResult<ShotDiff> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        Ok(app.shot_diff(&g)?)
    })
    .await
}

/// 把备份中有、磁盘上没有的截图写回原位置（`dest` 为空）或指定文件夹。
#[tauri::command]
pub async fn write_back_shots(
    h: AppHandle,
    game_id: String,
    blobs: Vec<String>,
    dest: Option<PathBuf>,
) -> CmdResult<WriteBackReport> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        let ids = blob_ids(&blobs)?;
        Ok(app.write_back_shots(&g, &ids, dest.as_deref())?)
    })
    .await
}

/// 重新收集磁盘上被忽略（在回收站中或已彻底删除）的截图。
#[tauri::command]
pub async fn recollect_shots(
    h: AppHandle,
    game_id: String,
    paths: Vec<PathBuf>,
) -> CmdResult<usize> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        let lock = app.repo.lock()?;
        Ok(app.recollect_shots(&lock, &g, &paths)?)
    })
    .await
}

/// 回答“关闭窗口时最小化到托盘还是退出”：`remember` 时写入设置，之后不再询问。
#[tauri::command]
pub async fn close_window(h: AppHandle, tray: bool, remember: bool) -> CmdResult<()> {
    let shared = h.state::<Shared>();
    if remember
        && let Some(a) = shared
            .app
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_mut()
    {
        a.config.general.close_to_tray = tray;
        a.config.general.close_asked = true;
        a.config
            .save(&shared.paths.config_file)
            .map_err(AppError::from)?;
    }
    if tray {
        shared.tray_once.store(true, Ordering::SeqCst);
        if let Some(w) = h.get_webview_window("main") {
            w.destroy().map_err(|e| CmdError {
                kind: "general",
                message: e.to_string(),
            })?;
        }
    } else {
        h.exit(0);
    }
    Ok(())
}

/// 正在运行的游戏（按安装目录检测），备份前提醒用，不阻止备份。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunningGame {
    game_id: String,
    processes: Vec<String>,
}

#[tauri::command]
pub async fn running_games(h: AppHandle, game_ids: Vec<String>) -> CmdResult<Vec<RunningGame>> {
    with_app(h, move |app, _| {
        Ok(app
            .running_games(&game_ids)?
            .into_iter()
            .map(|(game_id, processes)| RunningGame { game_id, processes })
            .collect())
    })
    .await
}

/// 自动备份的上次执行时间（设置页显示）。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoStatus {
    last_scheduled: Option<String>,
    last_check: Option<String>,
}

#[tauri::command]
pub async fn auto_status(h: AppHandle) -> CmdResult<AutoStatus> {
    with_app(h, move |app, _| {
        Ok(AutoStatus {
            last_scheduled: app.last_auto(AutoKind::Scheduled).map(|t| t.to_string()),
            last_check: app.last_auto(AutoKind::OnChange).map(|t| t.to_string()),
        })
    })
    .await
}

#[tauri::command]
pub async fn export_shots(
    h: AppHandle,
    game_id: String,
    blobs: Vec<String>,
    dest: PathBuf,
    format: ExportFormat,
) -> CmdResult<ShotExportReport> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        let ids = blob_ids(&blobs)?;
        Ok(app.export_shots(&g, &ids, &dest, format)?)
    })
    .await
}

/// 游戏的截图开关：`enabled` 为空时跟随全局设置。
#[tauri::command]
pub async fn set_shots_enabled(
    h: AppHandle,
    game_id: String,
    enabled: Option<bool>,
) -> CmdResult<()> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        Ok(app.set_shots_enabled(&g, enabled)?)
    })
    .await
}

#[tauri::command]
pub async fn add_shot_dir(h: AppHandle, game_id: String, dir: PathBuf) -> CmdResult<String> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        Ok(app.add_shot_dir(&g, &dir)?)
    })
    .await
}

#[tauri::command]
pub async fn remove_shot_dir(h: AppHandle, game_id: String, dir: String) -> CmdResult<bool> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        Ok(app.remove_shot_dir(&g, &dir)?)
    })
    .await
}

/// 界面状态（游戏库筛选、图库查看方式等），见 `bonfire_app::ui_state`。
#[tauri::command]
pub fn ui_state_get(h: AppHandle) -> std::collections::BTreeMap<String, String> {
    let shared = h.state::<Shared>();
    bonfire_app::ui_state::load_ui_state(&bonfire_app::ui_state::ui_state_path(&shared.paths))
}

#[tauri::command]
pub fn ui_state_set(h: AppHandle, key: String, value: Option<String>) -> CmdResult<()> {
    let shared = h.state::<Shared>();
    Ok(bonfire_app::ui_state::set_ui_state(
        &bonfire_app::ui_state::ui_state_path(&shared.paths),
        &key,
        value,
    )?)
}

/// 把“未识别的截图”中的截图归到游戏库中的某个游戏。
#[tauri::command]
pub async fn assign_unsorted(
    h: AppHandle,
    blobs: Vec<String>,
    game_id: String,
) -> CmdResult<usize> {
    with_app(h, move |app, _| {
        let game = find(app, &game_id)?;
        let ids = blob_ids(&blobs)?;
        let lock = app.repo.lock()?;
        Ok(app.assign_unsorted(&lock, &ids, &game)?)
    })
    .await
}

/// 截图来源的位置（设置页说明用）：Steam 截图所在的 `userdata` 文件夹与“捕获”文件夹。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShotSourceDirs {
    steam_userdata: Option<PathBuf>,
    captures: Option<PathBuf>,
}

#[tauri::command]
pub async fn shot_source_dirs(h: AppHandle) -> CmdResult<ShotSourceDirs> {
    with_app(h, move |app, _| {
        Ok(ShotSourceDirs {
            steam_userdata: app.steam_root().map(|r| r.join("userdata")),
            captures: app.captures_dir(),
        })
    })
    .await
}

/// “捕获”文件夹的实际位置（设置页显示）。
#[tauri::command]
pub async fn captures_dir(h: AppHandle) -> CmdResult<Option<PathBuf>> {
    with_app(h, move |app, _| Ok(app.captures_dir())).await
}

/// 自动备份与定时收集截图：每分钟检查一次是否到期；仓库正被其他操作（如命令行）使用时推迟到下一次。
/// 自动备份完成后推送 `auto-backup`（有备份、出错或收集到截图时），定时收集截图后推送 `shots-synced`。
pub fn start_scheduler(h: AppHandle) {
    start_exit_watcher(h.clone());
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(60));
            let shared = h.state::<Shared>();
            let report: Option<AutoReport> = {
                let mut guard = shared.app.lock().unwrap_or_else(|p| p.into_inner());
                let Some(app) = guard.as_mut() else { continue };
                match app.auto_backup_due(&jiff::Zoned::now()) {
                    None => None,
                    Some(kind) => match app.repo.lock() {
                        Ok(lock) => match app.run_auto_backup(&lock, kind) {
                            Ok(r) => Some(r),
                            Err(e) => {
                                tracing::warn!("自动备份失败：{e}");
                                None
                            }
                        },
                        Err(_) => None,
                    },
                }
            };
            if let Some(r) = report.filter(|r| !r.items.is_empty()) {
                let _ = h.emit("auto-backup", r);
            }
            let added = {
                let mut guard = shared.app.lock().unwrap_or_else(|p| p.into_inner());
                let Some(app) = guard.as_mut() else { continue };
                if !app.shots_sync_due() {
                    continue;
                }
                let Ok(lock) = app.repo.lock() else { continue };
                match app.sync_all_shots(&lock) {
                    Ok(r) => r.iter().map(|x| x.added).sum::<usize>(),
                    Err(e) => {
                        tracing::warn!("定时收集截图失败：{e}");
                        continue;
                    }
                }
            };
            tracing::info!(added, "定时收集截图完成");
            let _ = h.emit("shots-synced", added);
        }
    });
}

/// 退出游戏后自动备份（M4-3）：每隔几秒检测一次游戏进程，游戏退出后备份；
/// 只在开启了“退出游戏后自动备份”或有运行中被推迟的自动备份时检测。检测进程时不占用应用。
fn start_exit_watcher(h: AppHandle) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let shared = h.state::<Shared>();
            let watches = {
                let guard = shared.app.lock().unwrap_or_else(|p| p.into_inner());
                let Some(app) = guard.as_ref() else { continue };
                if !app.exit_watch_needed() {
                    continue;
                }
                app.all_watches().unwrap_or_default()
            };
            let running: std::collections::HashSet<String> =
                bonfire_app::process::running(&watches)
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect();
            let report: Option<AutoReport> = {
                let mut guard = shared.app.lock().unwrap_or_else(|p| p.into_inner());
                let Some(app) = guard.as_mut() else { continue };
                let ready = app.exit_watch_tick(running, std::time::Instant::now());
                if ready.is_empty() {
                    continue;
                }
                match app.repo.lock() {
                    Ok(lock) => match app.run_exit_backup(&lock, &ready) {
                        Ok(r) => Some(r),
                        Err(e) => {
                            tracing::warn!("退出游戏后自动备份失败：{e}");
                            None
                        }
                    },
                    // 仓库正被其他操作使用：放回等待，下次再试。
                    Err(_) => {
                        app.exit_watch.defer(ready);
                        None
                    }
                }
            };
            if let Some(r) = report.filter(|r| !r.items.is_empty()) {
                let _ = h.emit("auto-backup", r);
            }
        }
    });
}

/// 游戏程序：运行检测用的进程名。
#[tauri::command]
pub async fn game_processes(h: AppHandle, game_id: String) -> CmdResult<Vec<String>> {
    with_app(h, move |app, _| Ok(app.index.processes(&game_id)?)).await
}

#[tauri::command]
pub async fn set_game_processes(
    h: AppHandle,
    game_id: String,
    names: Vec<String>,
) -> CmdResult<Vec<String>> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        let lock = app.repo.lock()?;
        Ok(app.set_game_processes(&lock, &g, &names)?)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explorer_args_keep_spaces_inside_quotes() {
        let p = std::path::Path::new(r"D:\Exports\Hollow Knight-20261009.zip");
        assert_eq!(
            explorer_arg(p, true),
            r#"/select,"D:\Exports\Hollow Knight-20261009.zip""#
        );
        assert_eq!(
            explorer_arg(std::path::Path::new(r"D:\My Saves"), false),
            r#""D:\My Saves""#
        );
    }

    #[test]
    fn explorer_target_normalizes_and_falls_back_to_existing_parent() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a b.zip");
        std::fs::write(&file, b"x").unwrap();
        let slashed = file.to_string_lossy().replace('\\', "/");
        assert_eq!(explorer_target(std::path::Path::new(&slashed)), file);
        let missing = dir.path().join("gone").join("x.zip");
        assert_eq!(explorer_target(&missing), dir.path());
    }
}
