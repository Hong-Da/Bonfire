//! 篝火 Bonfire 桌面应用：Tauri 命令层只调用 bonfire-app，不实现业务逻辑。

// 发布版不弹出控制台窗口。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use bonfire_app::{AppPaths, Config};
use serde::Serialize;

#[derive(Serialize)]
struct AppInfo {
    version: &'static str,
    portable: bool,
}

#[tauri::command]
fn app_info(paths: tauri::State<'_, AppPaths>) -> AppInfo {
    AppInfo { version: bonfire_app::VERSION, portable: paths.portable }
}

fn main() {
    let paths = AppPaths::resolve();
    // 配置有误时先用默认值启动，错误写入日志；M3 起在界面中提示。
    let (config, config_error) = match Config::load(&paths.config_file) {
        Ok(c) => (c, None),
        Err(e) => (Config::default(), Some(e)),
    };
    let _log_guard = bonfire_app::logging::init(&paths.logs_dir, &config.log, cfg!(debug_assertions));
    if let Some(e) = config_error {
        tracing::error!("配置加载失败，使用默认配置：{e}");
    }
    tracing::info!(version = bonfire_app::VERSION, portable = paths.portable, "桌面应用启动");

    tauri::Builder::default()
        .manage(paths)
        .invoke_handler(tauri::generate_handler![app_info])
        .run(tauri::generate_context!())
        .expect("Tauri 运行失败");
}
