//! 同步备份（REQ-015）的命令：目标设置、预览、同步、恢复。联网同步在独立实例上进行，不占用界面使用的实例。
//! 同步过程推送 `sync-progress`（目标、已上传字节、总字节），结束推送 `sync-done`（目标、结果或错误）。

use std::path::PathBuf;

use bonfire_app::App;
use bonfire_app::sync::{SyncPlan, SyncRun, SyncTarget, TargetStatus};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::commands::{CmdError, CmdResult, Shared, with_app};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    id: String,
    done: u64,
    total: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncDone {
    id: String,
    run: Option<SyncRun>,
    error: Option<String>,
}

/// 正在同步的目标（同一目标不同时同步两次）。
static RUNNING: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// 在界面实例上取出独立的同步实例。
fn worker(h: &AppHandle) -> CmdResult<App> {
    let shared = h.state::<Shared>();
    let guard = shared.app.lock().unwrap_or_else(|p| p.into_inner());
    let app = guard.as_ref().ok_or_else(CmdError::not_ready)?;
    Ok(app.sync_worker()?)
}

/// 同步一个目标（阻塞，在后台线程中调用）。
pub fn run_one(h: &AppHandle, id: &str) -> SyncDone {
    {
        let mut r = RUNNING.lock().unwrap_or_else(|p| p.into_inner());
        if r.iter().any(|x| x == id) {
            return SyncDone {
                id: id.into(),
                run: None,
                error: Some("这个目标正在同步".into()),
            };
        }
        r.push(id.to_string());
    }
    let result = worker(h).and_then(|mut w| {
        let hh = h.clone();
        let idc = id.to_string();
        Ok(w.sync_push(id, &mut |done, total| {
            let _ = hh.emit("sync-progress", Progress { id: idc.clone(), done, total });
        })?)
    });
    RUNNING.lock().unwrap_or_else(|p| p.into_inner()).retain(|x| x != id);
    let done = match result {
        Ok(run) => SyncDone { id: id.into(), run: Some(run), error: None },
        Err(e) => SyncDone { id: id.into(), run: None, error: Some(e.message) },
    };
    let _ = h.emit("sync-done", done.clone());
    done
}

/// 在后台依次同步这些目标（备份后、定时）。
pub fn spawn_sync(h: AppHandle, ids: Vec<String>) {
    if ids.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        for id in ids {
            let d = run_one(&h, &id);
            if let Some(e) = d.error {
                tracing::warn!(target = %id, "同步失败：{e}");
            }
        }
    });
}

/// 每次备份后同步（设成“每次备份后”的目标）。
pub fn after_backup(h: &AppHandle) {
    let ids = {
        let shared = h.state::<Shared>();
        let guard = shared.app.lock().unwrap_or_else(|p| p.into_inner());
        guard.as_ref().map(|a| a.sync_after_backup_targets()).unwrap_or_default()
    };
    spawn_sync(h.clone(), ids);
}

#[tauri::command]
pub async fn sync_status(h: AppHandle) -> CmdResult<Vec<TargetStatus>> {
    with_app(h, move |app, _| Ok(app.sync_status())).await
}

#[tauri::command]
pub async fn sync_save(h: AppHandle, target: SyncTarget, secret: Option<String>) -> CmdResult<SyncTarget> {
    with_app(h, move |app, _| Ok(app.save_sync_target(target, secret.as_deref())?)).await
}

#[tauri::command]
pub async fn sync_remove(h: AppHandle, id: String) -> CmdResult<()> {
    with_app(h, move |app, _| Ok(app.remove_sync_target(&id)?)).await
}

/// 测试连接（可以是还没保存的目标与填写中的令牌）。
#[tauri::command]
pub async fn sync_test(h: AppHandle, target: SyncTarget, secret: Option<String>) -> CmdResult<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let w = worker(&h)?;
        Ok(w.sync_test_with(&target, secret.as_deref())?)
    })
    .await
    .map_err(|e| CmdError { kind: "general", message: e.to_string() })?
}

#[tauri::command]
pub async fn sync_plan(h: AppHandle, id: String) -> CmdResult<SyncPlan> {
    with_app(h, move |app, _| {
        let t = app.sync_target(&id)?;
        Ok(app.sync_plan(&t)?.0)
    })
    .await
}

#[tauri::command]
pub async fn sync_push(h: AppHandle, id: String) -> CmdResult<SyncDone> {
    tauri::async_runtime::spawn_blocking(move || run_one(&h, &id))
        .await
        .map_err(|e| CmdError { kind: "general", message: e.to_string() })
}

#[tauri::command]
pub async fn sync_compact(h: AppHandle, id: String) -> CmdResult<()> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut w = worker(&h)?;
        Ok(w.sync_compact(&id)?)
    })
    .await
    .map_err(|e| CmdError { kind: "general", message: e.to_string() })?
}

/// 从远程恢复完整仓库到空文件夹，返回文件数；远程有篝火设置时一起放在文件夹中。
#[tauri::command]
pub async fn sync_restore(h: AppHandle, id: String, dest: PathBuf) -> CmdResult<usize> {
    tauri::async_runtime::spawn_blocking(move || {
        let w = worker(&h)?;
        Ok(w.sync_restore(&id, &dest)?)
    })
    .await
    .map_err(|e| CmdError { kind: "general", message: e.to_string() })?
}

/// 恢复出的文件夹中是否有篝火设置。
#[tauri::command]
pub async fn sync_has_settings(dest: PathBuf) -> CmdResult<bool> {
    Ok(dest.join(bonfire_app::sync::SETTINGS_FILE).is_file())
}

/// 套用恢复出的篝火设置（保留本机的仓库位置与密钥）。
#[tauri::command]
pub async fn sync_apply_settings(h: AppHandle, dest: PathBuf) -> CmdResult<()> {
    with_app(h, move |app, _| {
        let p = dest.join(bonfire_app::sync::SETTINGS_FILE);
        let text = std::fs::read_to_string(&p).map_err(|e| bonfire_app::AppError::io(&p, e))?;
        Ok(app.apply_portable_settings(&text)?)
    })
    .await
}
