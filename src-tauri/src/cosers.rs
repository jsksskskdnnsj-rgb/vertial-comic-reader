//! Embedded webview for the official coserslove site.
//!
//! The child webview loads https://coserslove.com/cosers as a top-level document.
//! Nothing in this module fetches, parses, or stores the page, and it does not
//! inject scripts that hide ads or rewrite the document.

use std::sync::Mutex;

use tauri::{LogicalPosition, LogicalSize, Manager, WebviewUrl};

const COSERS_LABEL: &str = "cosers-site";
/// Official page opened by the sidebar entry. Not a proxy and not a mirror.
pub const COSERS_URL: &str = "https://coserslove.com/cosers";

static COSERS_OP: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    COSERS_OP.lock().unwrap_or_else(|e| e.into_inner())
}

fn finite_bounds(x: f64, y: f64, width: f64, height: f64) -> Result<(f64, f64, f64, f64), String> {
    if ![x, y, width, height].iter().all(|n| n.is_finite()) {
        return Err("bad bounds".into());
    }
    Ok((x, y, width.max(1.0), height.max(1.0)))
}

fn place<R: tauri::Runtime>(wv: &tauri::Webview<R>, x: f64, y: f64, width: f64, height: f64) -> Result<(), String> {
    wv.set_position(LogicalPosition::new(x, y))
        .map_err(|e| e.to_string())?;
    wv.set_size(LogicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    wv.show().map_err(|e| e.to_string())
}

fn show_blocking(window: tauri::Window, x: f64, y: f64, width: f64, height: f64) -> Result<(), String> {
    let _guard = lock();
    let app = window.app_handle();
    if let Some(wv) = app.get_webview(COSERS_LABEL) {
        return place(&wv, x, y, width, height);
    }
    // Parse only. The webview itself requests the URL; we do not download the body.
    let url = url::Url::parse(COSERS_URL).map_err(|e| e.to_string())?;
    let builder = tauri::webview::WebviewBuilder::new(COSERS_LABEL, WebviewUrl::External(url));
    match window.add_child(builder, LogicalPosition::new(x, y), LogicalSize::new(width, height)) {
        Ok(wv) => {
            let _ = wv.set_focus();
            Ok(())
        }
        Err(e) => {
            if let Some(wv) = app.get_webview(COSERS_LABEL) {
                place(&wv, x, y, width, height)
            } else {
                Err(e.to_string())
            }
        }
    }
}

/// Show the official site in the content pane. Geometry is logical pixels.
/// `async` plus a blocking thread avoids the Windows WebView2 deadlock that
/// synchronous commands hit when creating a child webview.
#[tauri::command]
pub async fn cosers_show(window: tauri::Window, x: f64, y: f64, width: f64, height: f64) -> Result<(), String> {
    let (x, y, width, height) = finite_bounds(x, y, width, height)?;
    match tauri::async_runtime::spawn_blocking(move || show_blocking(window, x, y, width, height)).await {
        Ok(result) => result,
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub async fn cosers_hide(window: tauri::Window) -> Result<(), String> {
    match tauri::async_runtime::spawn_blocking(move || {
        let _guard = lock();
        if let Some(wv) = window.app_handle().get_webview(COSERS_LABEL) {
            wv.hide().map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
    {
        Ok(result) => result,
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub async fn cosers_close(window: tauri::Window) -> Result<(), String> {
    match tauri::async_runtime::spawn_blocking(move || {
        let _guard = lock();
        if let Some(wv) = window.app_handle().get_webview(COSERS_LABEL) {
            wv.close().map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
    {
        Ok(result) => result,
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn official_url_is_the_public_page() {
        assert_eq!(super::COSERS_URL, "https://coserslove.com/cosers");
        let url: url::Url = super::COSERS_URL.parse().unwrap();
        assert_eq!(url.scheme(), "https");
        assert_eq!(url.host_str(), Some("coserslove.com"));
        assert_eq!(url.path(), "/cosers");
        assert!(url.query().is_none());
    }
}
