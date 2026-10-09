//! Tauri 命令层：只做参数转换、线程调度与事件推送，业务逻辑全部调用 bonfire-app。
//!
//! 所有命令在后台线程执行（`spawn_blocking`），通过共享的 `Mutex<App>` 串行访问仓库；
//! 长任务逐个游戏推送事件，并在游戏之间检查取消标志。

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use bonfire_app::backup::{BackupOptions, BackupResult, DetachedChoice};
use bonfire_app::core::index::LocationKind;
use bonfire_app::core::index::{GameRecord, Profile};
use bonfire_app::core::refs::RefKind;
use bonfire_app::library::{GameRow, HistoryView, SearchHit};
use bonfire_app::prune::DeleteReport;
use bonfire_app::restore::{RestoreOptions, RestorePlan, RestoreReport};
use bonfire_app::transfer::{
    ArchiveImportItem, ArchivePick, ArchivePreview, BundleImportReport, BundlePreview,
    BundleReport, BundleScope, CollectionReport, ExportFormat, ExportReport, FilesAutoReport,
    FilesPreview, FilesSource, ImportFilesOptions, ImportFilesReport, ImportPlace,
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
}

#[tauri::command]
pub async fn app_info(h: AppHandle) -> CmdResult<AppInfo> {
    let shared = h.state::<Shared>();
    let guard = shared.app.lock().unwrap_or_else(|p| p.into_inner());
    let (locale, theme, repo_dir) = match guard.as_ref() {
        Some(a) => (
            a.locale,
            a.config.general.theme,
            Some(a.repo.root().to_path_buf()),
        ),
        None => (Locale::resolve(None), Default::default(), None),
    };
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
                force,
                on_detached: match &on_detached {
                    OnDetached::New => DetachedChoice::NewBranch,
                    OnDetached::Named(n) => DetachedChoice::NamedBranch(n.clone()),
                    OnDetached::Move => DetachedChoice::MoveCurrent,
                },
                ..Default::default()
            };
            let r = app
                .find_game(&item.game_id)
                .and_then(|g| app.backup(&lock, &g, &opts));
            let res = match r {
                Ok(r) => BackupItemResult {
                    game_id: item.game_id,
                    ok: true,
                    result: Some(r),
                    error: None,
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

#[tauri::command]
pub async fn remove_game(h: AppHandle, game_id: String) -> CmdResult<GameRecord> {
    with_app(h, move |app, _| {
        let lock = app.repo.lock()?;
        Ok(app.remove_game(&lock, &game_id)?)
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

/// 在资源管理器中打开文件夹（或选中文件）。
#[tauri::command]
pub async fn open_path(path: PathBuf) -> CmdResult<()> {
    // 资源管理器不认识正斜杠，带 `/` 的路径会被忽略并打开默认的用户目录。
    let path = PathBuf::from(path.to_string_lossy().replace('/', "\\"));
    let mut cmd = std::process::Command::new("explorer");
    if path.is_file() {
        cmd.arg(format!("/select,{}", path.display()));
    } else {
        cmd.arg(&path);
    }
    cmd.spawn().map_err(|e| CmdError {
        kind: "general",
        message: e.to_string(),
    })?;
    Ok(())
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
    // 文件夹不存在时打开最近的上级目录。
    let existing = path
        .ancestors()
        .find(|p| p.exists())
        .map(PathBuf::from)
        .unwrap_or(path);
    open_path(existing).await
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
) -> CmdResult<BundleReport> {
    with_app(h, move |app, _| {
        let g = find(app, &game_id)?;
        Ok(app.export_bundle(&g, &scope, &dest)?)
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
) -> CmdResult<CollectionReport> {
    with_app(h, move |app, _| {
        let games = game_ids
            .iter()
            .map(|id| find(app, id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(app.export_collection(&games, &dest)?)
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
pub async fn inspect_files_source(h: AppHandle, src: PathBuf) -> CmdResult<FilesSource> {
    with_app(h, move |app, _| Ok(app.inspect_files_source(&src)?)).await
}

#[tauri::command]
pub async fn import_files_auto(
    h: AppHandle,
    src: PathBuf,
    comment: Option<String>,
) -> CmdResult<FilesAutoReport> {
    with_app(h, move |app, _| {
        let lock = app.repo.lock()?;
        Ok(app.import_files_auto(&lock, &src, comment, false)?)
    })
    .await
}
