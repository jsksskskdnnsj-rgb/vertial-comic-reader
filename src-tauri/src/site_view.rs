//! One child webview for the official Cosers site.
//!
//! Tauri only exposes child webviews behind the `unstable` feature. The address
//! is fixed here. The page is the site itself: no injected script, no request
//! filter, and no path that saves images into the library. The child is not
//! given IPC access (capabilities do not list a remote URL).

use tauri::webview::{Color, WebviewBuilder};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, WebviewUrl, Window};
use url::Url;

const LABEL: &str = "mh-cosers";
pub const COSERS_URL: &str = "https://coserslove.com/cosers";

fn official_url() -> Url {
    Url::parse(COSERS_URL).expect("cosers url")
}

fn finite_box(x: f64, y: f64, width: f64, height: f64) -> Option<(f64, f64, f64, f64)> {
    if ![x, y, width, height].iter().all(|v| v.is_finite()) {
        return None;
    }
    Some((
        x.clamp(-2000.0, 16_000.0),
        y.clamp(-2000.0, 16_000.0),
        width.clamp(0.0, 16_000.0),
        height.clamp(0.0, 16_000.0),
    ))
}

fn host(app: &AppHandle) -> Result<Window, String> {
    app.get_window("main")
        .ok_or_else(|| "no main window".to_string())
}

fn find_child(window: &Window) -> Option<tauri::Webview> {
    window
        .webviews()
        .into_iter()
        .find(|view| view.label() == LABEL)
}

fn place_on(window: &Window, x: f64, y: f64, width: f64, height: f64) -> Result<(), String> {
    let (x, y, width, height) = finite_box(x, y, width, height).ok_or("bad bounds")?;
    if width < 2.0 || height < 2.0 {
        if let Some(view) = find_child(window) {
            view.hide().map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    let pos = LogicalPosition::new(x, y);
    let size = LogicalSize::new(width, height);
    if let Some(view) = find_child(window) {
        view.set_position(pos).map_err(|e| e.to_string())?;
        view.set_size(size).map_err(|e| e.to_string())?;
        view.show().map_err(|e| e.to_string())?;
        return Ok(());
    }
    let builder = WebviewBuilder::new(LABEL, WebviewUrl::External(official_url()))
        .background_color(Color(255, 255, 255, 255));
    match window.add_child(builder, pos, size) {
        Ok(_) => Ok(()),
        Err(err) => {
            if let Some(view) = find_child(window) {
                view.set_position(pos).map_err(|e| e.to_string())?;
                view.set_size(size).map_err(|e| e.to_string())?;
                view.show().map_err(|e| e.to_string())?;
                Ok(())
            } else {
                Err(err.to_string())
            }
        }
    }
}

/// Show or move the embedded site over the content host. The URL is not an argument.
#[tauri::command]
pub fn site_view_place(
    app: AppHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    place_on(&host(&app)?, x, y, width, height)
}

/// Remove the embedded site. Called when the shell leaves that page.
#[tauri::command]
pub fn site_view_close(app: AppHandle) -> Result<(), String> {
    let Ok(window) = host(&app) else {
        return Ok(());
    };
    if let Some(view) = find_child(&window) {
        view.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{finite_box, COSERS_URL};

    #[test]
    fn url_is_the_official_site() {
        assert_eq!(COSERS_URL, "https://coserslove.com/cosers");
        assert!(super::official_url().as_str().starts_with(COSERS_URL));
    }

    #[test]
    fn bounds_reject_non_finite_values() {
        assert!(finite_box(1.0, 2.0, 3.0, 4.0).is_some());
        assert!(finite_box(f64::NAN, 0.0, 10.0, 10.0).is_none());
        let (x, _, w, _) = finite_box(-9_000.0, 0.0, 99_000.0, 10.0).unwrap();
        assert_eq!(x, -2000.0);
        assert_eq!(w, 16_000.0);
    }
}
