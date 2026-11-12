//! Frameless window controls, system tray, drop-queue, and app exit.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::AppHandle;

use crate::appdata::data_dir;
use crate::config::get_reader_config;

// Hiding to the tray is only allowed once the tray icon exists (otherwise a hidden window could never be brought back).
static TRAY_OK: AtomicBool = AtomicBool::new(false);

#[tauri::command]
pub fn win_minimize(window: tauri::Window) -> Result<(), String> {
    window.minimize().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn win_toggle_maximize(window: tauri::Window) -> Result<(), String> {
    if window.is_maximized().map_err(|e| e.to_string())? {
        window.unmaximize().map_err(|e| e.to_string())
    } else {
        window.maximize().map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub fn win_close(window: tauri::Window) -> Result<(), String> {
    window.close().map_err(|e| e.to_string())
}

/// 原生文件拖放：事件把路径写入队列文件，前端轮询 take_drop_queue 取走
/// （全程走 invoke 通道，不依赖前端 webview/event API）
fn drop_queue_path() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("drop-queue.json"))
}

pub(crate) fn push_drop_queue(paths: &[PathBuf]) -> Result<(), String> {
    let p = drop_queue_path()?;
    let mut q: Vec<String> = std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    q.extend(paths.iter().map(|x| x.to_string_lossy().to_string()));
    std::fs::write(&p, serde_json::to_string(&q).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn take_drop_queue() -> Result<Vec<String>, String> {
    let p = drop_queue_path()?;
    let q: Vec<String> = std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let _ = std::fs::remove_file(&p);
    Ok(q)
}

pub(crate) fn hide_to_tray_enabled() -> bool {
    TRAY_OK.load(Ordering::SeqCst) && get_reader_config().map(|c| c.close_to_tray).unwrap_or(false)
}

pub(crate) fn show_main_window(app: &tauri::AppHandle) {
    use tauri::Manager;
    let w = app
        .get_webview_window("main")
        .or_else(|| app.webview_windows().into_values().next());
    if let Some(w) = w {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

pub(crate) fn init_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    // no icon -> no tray, and hide-to-tray stays off (see TRAY_OK)
    let icon = match app.default_window_icon() {
        Some(i) => i.clone(),
        None => return Ok(()),
    };
    let show = MenuItem::with_id(app, "tray_show", "显示 / Show", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray_quit", "退出 / Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    #[allow(deprecated)] // renamed show_menu_on_left_click in tauri 2.2; the old name compiles on every 2.x
    let builder = TrayIconBuilder::with_id("mh-tray")
        .icon(icon)
        .tooltip("MH Reader")
        .menu(&menu)
        .menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray_show" => show_main_window(app),
            "tray_quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    builder.build(app)?;
    TRAY_OK.store(true, Ordering::SeqCst);
    Ok(())
}

/// 拖动窗口（自定义标题栏按下鼠标时调用）
#[tauri::command]
pub fn win_start_drag(window: tauri::Window) -> Result<(), String> {
    window.start_dragging().map_err(|e| e.to_string())
}

/// 关闭到托盘开启时隐藏窗口并返回 true；否则返回 false（调用方自行退出）。阅读页用它代替 destroy()
#[tauri::command]
pub fn win_hide_if_tray(window: tauri::Window) -> Result<bool, String> {
    if hide_to_tray_enabled() {
        window.hide().map_err(|e| e.to_string())?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// 真正退出程序（设置页"立即关闭程序"用；win_close 在关闭到托盘时只会隐藏窗口）
#[tauri::command]
pub fn app_exit(app: AppHandle) -> Result<(), String> {
    crate::metadata::proxy::shutdown_core();
    app.exit(0);
    Ok(())
}
