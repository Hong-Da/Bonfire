//! 需要处理的问题（决策 OV-4）：导航“总览”的红色数字、任务栏图标与托盘图标的红点、Windows 系统通知。
//!
//! 关闭到托盘时窗口与界面已销毁，所以由这里（而不是界面）计算：同步完成、自动备份、手动备份后，
//! 以及调度器每分钟检查一次。有变化时推送 `attention`（[`Attention`]）给界面。
//! 系统通知：设置 `general.notify_failures` 开启、篝火窗口不在前台时，每个新问题（条目键 + 失败时间）只通知一次。

use std::collections::HashSet;
use std::sync::Mutex;

use bonfire_app::overview::Attention;
use bonfire_app::{Locale, t, tf};
use tauri::image::Image;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::commands::Shared;

/// 已通知过的问题（键|时间）。
static NOTIFIED: Mutex<Option<HashSet<String>>> = Mutex::new(None);
/// 上次显示的条数（不变时不重画图标）。
static SHOWN: Mutex<Option<usize>> = Mutex::new(None);

/// 在后台线程中刷新（可以在持有应用锁时调用：等锁释放后进行）。
pub fn spawn_refresh(h: AppHandle) {
    std::thread::spawn(move || refresh(&h));
}

/// 重新计算并更新红点、托盘提示，必要时弹出通知。不能在持有应用锁时调用。
pub fn refresh(h: &AppHandle) {
    let shared = h.state::<Shared>();
    let (att, locale, notify) = {
        let guard = shared.app.lock().unwrap_or_else(|p| p.into_inner());
        let Some(app) = guard.as_ref() else { return };
        if !app.repo.is_unlocked() {
            return;
        }
        match app.attention() {
            Ok(a) => (a, app.locale, app.config.general.notify_failures),
            Err(e) => {
                tracing::warn!("检查需要处理的问题失败：{e}");
                return;
            }
        }
    };
    let n = att.items.len();
    let changed = {
        let mut s = SHOWN.lock().unwrap_or_else(|p| p.into_inner());
        let c = *s != Some(n);
        *s = Some(n);
        c
    };
    if changed {
        show_badge(h, n, locale);
    }
    if notify && !main_focused(h) {
        notify_new(h, &att, locale);
    } else {
        // 窗口在前台时看得到，记为已通知，之后不再弹出。
        remember(&att);
    }
    let _ = h.emit("attention", &att);
}

fn main_focused(h: &AppHandle) -> bool {
    h.get_webview_window("main")
        .is_some_and(|w| w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false))
}

fn key(i: &bonfire_app::overview::AttentionItem) -> String {
    format!("{}|{}", i.key, i.at)
}

fn remember(att: &Attention) {
    let mut g = NOTIFIED.lock().unwrap_or_else(|p| p.into_inner());
    let set = g.get_or_insert_with(HashSet::new);
    set.extend(att.items.iter().map(key));
}

fn notify_new(h: &AppHandle, att: &Attention, locale: Locale) {
    let fresh: Vec<_> = {
        let mut g = NOTIFIED.lock().unwrap_or_else(|p| p.into_inner());
        let set = g.get_or_insert_with(HashSet::new);
        att.items
            .iter()
            .filter(|i| set.insert(key(i)))
            .cloned()
            .collect()
    };
    for i in fresh {
        let issue = i
            .issue
            .as_deref()
            .map(|k| t(locale, &format!("notify.issue.{k}")).to_string())
            .unwrap_or_default();
        let title = match i.kind.as_str() {
            "sync" => tf(locale, "notify.sync_failed", &[("name", &i.name)]),
            "fsck" => t(locale, "notify.fsck_failed").to_string(),
            _ => tf(locale, "notify.backup_failed", &[("name", &i.name)]),
        };
        let body = if issue.is_empty() {
            t(locale, "notify.open").to_string()
        } else {
            format!("{issue}{}", t(locale, "notify.open_suffix"))
        };
        if let Err(e) = h.notification().builder().title(title).body(body).show() {
            tracing::warn!("弹出系统通知失败：{e}");
        }
    }
}

/// 任务栏图标右下角的红点、托盘图标右上角的红点与提示文字。
fn show_badge(h: &AppHandle, n: usize, locale: Locale) {
    let base = h.default_window_icon().cloned();
    if let Some(w) = h.get_webview_window("main") {
        let overlay = (n > 0).then(|| dot_image(32));
        let _ = w.set_overlay_icon(overlay);
    }
    if let Some(tray) = h.tray_by_id("main") {
        let icon = match (&base, n > 0) {
            (Some(b), true) => Some(with_dot(b)),
            (Some(b), false) => Some(b.clone().to_owned()),
            _ => None,
        };
        let _ = tray.set_icon(icon);
        let tip = if n > 0 {
            tf(locale, "tray.tooltip_attention", &[("n", &n)])
        } else {
            t(locale, "tray.tooltip").to_string()
        };
        let _ = tray.set_tooltip(Some(tip));
    }
}

const RED: [u8; 3] = [0xd9, 0x2d, 0x20];

/// 带白边的红色圆点。
fn dot_image(size: u32) -> Image<'static> {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    paint_dot(
        &mut rgba,
        size,
        size,
        size as f32 / 2.0,
        size as f32 / 2.0,
        size as f32 / 2.0,
    );
    Image::new_owned(rgba, size, size)
}

/// 图标右上角加红点。
fn with_dot(base: &Image<'_>) -> Image<'static> {
    let (w, h) = (base.width(), base.height());
    let mut rgba = base.rgba().to_vec();
    let r = w as f32 * 0.24;
    paint_dot(&mut rgba, w, h, w as f32 - r - 0.5, r + 0.5, r);
    Image::new_owned(rgba, w, h)
}

fn paint_dot(rgba: &mut [u8], w: u32, h: u32, cx: f32, cy: f32, r: f32) {
    let ring = (r * 0.18).max(1.0);
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            if d > r {
                continue;
            }
            // 边缘 1 像素抗锯齿。
            let alpha = (r - d).clamp(0.0, 1.0);
            let c: [u8; 3] = if d > r - ring { [255, 255, 255] } else { RED };
            let i = ((y * w + x) * 4) as usize;
            for k in 0..3 {
                rgba[i + k] = (c[k] as f32 * alpha + rgba[i + k] as f32 * (1.0 - alpha)) as u8;
            }
            rgba[i + 3] = (255.0 * alpha + rgba[i + 3] as f32 * (1.0 - alpha)) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dot_is_red_inside_and_transparent_outside() {
        let img = dot_image(32);
        let px = |x: u32, y: u32| {
            let i = ((y * 32 + x) * 4) as usize;
            img.rgba()[i..i + 4].to_vec()
        };
        assert_eq!(px(16, 16), vec![RED[0], RED[1], RED[2], 255]);
        assert_eq!(px(0, 0)[3], 0, "角上透明");
        let base = Image::new_owned(vec![10u8; 64 * 64 * 4], 64, 64);
        let marked = with_dot(&base);
        let i = ((12 * 64 + 51) * 4) as usize;
        assert_eq!(marked.rgba()[i..i + 3], RED, "右上角是红点");
        assert_eq!(marked.rgba()[(32 * 64 + 10) * 4], 10, "其余不变");
    }
}
