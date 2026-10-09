//! 系统托盘：常驻图标，左键单击显示主窗口；菜单“打开篝火 / 检查更新 / 退出”。
//! 关闭窗口时按设置 `general.close_to_tray`（默认开启）留在托盘：窗口与 WebView 销毁以节省内存，
//! 程序继续运行；从托盘或再次启动程序时重新创建窗口。从托盘菜单“退出”才结束程序。

use bonfire_app::{Config, Locale, t};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent};

use crate::commands::Shared;

/// 托盘菜单“检查更新”发给界面的事件。
pub const CHECK_EVENT: &str = "tray-check";

pub fn create(app: &AppHandle, locale: Locale) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", t(locale, "tray.open"), true, None::<&str>)?;
    let check = MenuItem::with_id(app, "check", t(locale, "tray.check"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", t(locale, "tray.quit"), true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&open, &check, &PredefinedMenuItem::separator(app)?, &quit],
    )?;
    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip(t(locale, "tray.tooltip"))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| match e.id.as_ref() {
            "open" => show_main(app),
            "check" => {
                show_main(app);
                let _ = app.emit(CHECK_EVENT, ());
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// 按 tauri.conf.json 创建主窗口。窗口数据（WebView 本地存储等）放在 data 中，而不是用户目录。
pub fn create_main_window(app: &AppHandle) -> tauri::Result<()> {
    let config = app
        .config()
        .app
        .windows
        .first()
        .cloned()
        .expect("tauri.conf.json 中定义了主窗口");
    let dir = app.state::<Shared>().paths.webview_dir();
    tauri::WebviewWindowBuilder::from_config(app, &config)?
        .data_directory(dir)
        .build()?;
    Ok(())
}

/// 显示并聚焦主窗口（从托盘恢复，或再次启动程序时）；窗口已关闭时重新创建。
pub fn show_main(app: &AppHandle) {
    match app.get_webview_window("main") {
        Some(w) => {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
        None => {
            if let Err(e) = create_main_window(app) {
                tracing::error!("重新创建主窗口失败：{e}");
            }
        }
    }
}

/// 最后一个窗口关闭时：设置为留在托盘则不退出（托盘菜单“退出”调用 `exit`，带退出码，照常退出）。
pub fn on_run_event(app: &AppHandle, event: RunEvent) {
    if let RunEvent::ExitRequested { api, code, .. } = event
        && code.is_none()
        && close_to_tray(app)
    {
        api.prevent_exit();
    }
}

/// 当前设置：仓库已打开时取应用中的配置（设置保存后立即生效），否则读配置文件。
fn close_to_tray(app: &AppHandle) -> bool {
    let shared = app.state::<Shared>();
    if let Some(a) = shared
        .app
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
    {
        return a.config.general.close_to_tray;
    }
    Config::load(&shared.paths.config_file)
        .map(|c| c.general.close_to_tray)
        .unwrap_or(true)
}
