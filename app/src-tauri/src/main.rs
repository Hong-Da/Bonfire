//! 篝火 Bonfire 桌面应用：Tauri 命令层只调用 bonfire-app，不实现业务逻辑。

// 发布版不弹出控制台窗口。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod tray;

use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use bonfire_app::{AppPaths, Config, Locale};
use commands::Shared;
use tauri::Manager;

fn main() {
    let paths = AppPaths::resolve();
    // 配置文件不存在：首次运行，等向导完成后再打开仓库。配置有误时用默认值启动并在界面中提示。
    let first_run = !paths.config_file.is_file();
    let (config, config_error) = match Config::load(&paths.config_file) {
        Ok(c) => (c, None),
        Err(e) => (Config::default(), Some(e.to_string())),
    };
    let _log_guard =
        bonfire_app::logging::init(&paths.logs_dir, &config.log, cfg!(debug_assertions));
    if let Some(e) = &config_error {
        tracing::error!("配置加载失败，使用默认配置：{e}");
    }
    tracing::info!(
        version = bonfire_app::VERSION,
        portable = paths.portable,
        first_run,
        "桌面应用启动"
    );

    let locale = Locale::resolve(config.general.language.as_deref());
    let shared = Shared {
        app: Mutex::new(None),
        paths,
        cancel: AtomicBool::new(false),
        config_error: Mutex::new(config_error),
        tray_once: AtomicBool::new(false),
        close_asked_at: Mutex::new(None),
    };

    tauri::Builder::default()
        // 只运行一个实例：再次启动时显示已在运行（可能隐藏在托盘中）的窗口。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main(app)
        }))
        .plugin(tauri_plugin_dialog::init())
        .manage(shared)
        .on_window_event(tray::on_window_event)
        .setup(move |app| {
            tray::create_main_window(app.handle())?;
            tray::create(app.handle(), locale)?;
            commands::start_scheduler(app.handle().clone());
            if !first_run {
                let h = app.handle().clone();
                let shared = h.state::<Shared>();
                if let Err(e) = commands::open_app(&h, &shared, config) {
                    tracing::error!("打开仓库失败：{e}");
                    *shared.config_error.lock().unwrap() = Some(e.to_string());
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::restore_shots,
            commands::purge_shots,
            commands::set_shots_only,
            commands::shot_candidates,
            commands::add_shots_game,
            commands::auto_status,
            commands::set_manage_mode,
            commands::gallery,
            commands::shot_diff,
            commands::write_back_shots,
            commands::recollect_shots,
            commands::close_window,
            commands::running_games,
            commands::export_package,
            commands::export_package_zip,
            commands::sync_shots_to_disk,
            commands::open_location,
            commands::export_default_name,
            commands::export_files,
            commands::export_bundle,
            commands::inspect_bundle,
            commands::import_bundle,
            commands::inspect_files,
            commands::import_files,
            commands::export_collection,
            commands::inspect_archive,
            commands::import_archive,
            commands::inspect_files_sources,
            commands::import_files_auto,
            commands::delete_branch,
            commands::delete_tag,
            commands::delete_snapshot,
            commands::album,
            commands::sync_shots,
            commands::shot_thumb,
            commands::shot_image,
            commands::delete_shots,
            commands::export_shots,
            commands::set_shots_enabled,
            commands::add_shot_dir,
            commands::remove_shot_dir,
            commands::captures_dir,
            commands::shot_source_dirs,
            commands::assign_unsorted,
            commands::ui_state_get,
            commands::rename_branch,
            commands::open_repo,
            commands::set_primary_branch,
            commands::inspect_download,
            commands::import_download,
            commands::ui_state_set,
            commands::get_settings,
            commands::save_settings,
            commands::load_library,
            commands::discover_games,
            commands::include_games,
            commands::ignore_games,
            commands::game_row,
            commands::game_changes,
            commands::cancel_task,
            commands::run_backup,
            commands::history,
            commands::snapshot_files,
            commands::diff,
            commands::set_comment,
            commands::create_ref,
            commands::search_comments,
            commands::plan_restore,
            commands::restore,
            commands::undo_restore,
            commands::pending_undo,
            commands::add_game,
            commands::remove_game,
            commands::confirm_locations,
            commands::ignored_games,
            commands::enable_game,
            commands::game_info,
            commands::refresh_info,
            commands::set_info_field,
            commands::profiles,
            commands::save_profile,
            commands::delete_profile,
            commands::fsck,
            commands::clean_history,
            commands::reindex,
            commands::update_rules,
            commands::rules_info,
            commands::open_path,
        ])
        .build(tauri::generate_context!())
        .expect("Tauri 启动失败")
        .run(tray::on_run_event);
}
