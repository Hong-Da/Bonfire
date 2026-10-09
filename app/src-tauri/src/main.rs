//! 篝火 Bonfire 桌面应用：Tauri 命令层只调用 bonfire-app，不实现业务逻辑。

// 发布版不弹出控制台窗口。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use bonfire_app::{AppPaths, Config};
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
    let _log_guard = bonfire_app::logging::init(&paths.logs_dir, &config.log, cfg!(debug_assertions));
    if let Some(e) = &config_error {
        tracing::error!("配置加载失败，使用默认配置：{e}");
    }
    tracing::info!(version = bonfire_app::VERSION, portable = paths.portable, first_run, "桌面应用启动");

    let webview_dir = paths.webview_dir();
    let shared = Shared { app: Mutex::new(None), paths, cancel: AtomicBool::new(false), config_error: Mutex::new(config_error) };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(shared)
        .setup(move |app| {
            // 窗口数据（WebView 本地存储等）也放在 data 中，而不是用户目录。
            let window = app.config().app.windows.first().cloned().expect("tauri.conf.json 中定义了主窗口");
            tauri::WebviewWindowBuilder::from_config(app.handle(), &window)?.data_directory(webview_dir).build()?;
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
            commands::reindex,
            commands::update_rules,
            commands::rules_info,
            commands::open_path,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri 运行失败");
}
