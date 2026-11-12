//! File-type helpers and folder/image picking used by import and the shelf.

use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, FilePath};

/// Extensions admitted at import and recognised by `extractor::sniff`.
/// `avif` is passthrough: the webview displays the original bytes. The `image`
/// crate is not built with an AVIF decoder, so covers, slices, and width
/// variants use `DECODE_EXTS` only. svg/tif/heic/jxl are rejected at import —
/// they used to succeed and then fail every page as "not a recognised image".
pub(crate) const IMAGE_EXTS: &[&str] = &[
    "jpg", "jpeg", "jfif", "jpe", "png", "webp", "gif", "bmp", "avif",
];
/// Formats `image` 0.25 is compiled to decode (see Cargo.toml features).
pub(crate) const DECODE_EXTS: &[&str] = &[
    "jpg", "jpeg", "jfif", "jpe", "png", "webp", "gif", "bmp",
];
pub(crate) const ZIP_EXTS: &[&str] = &["zip", "cbz"];
pub(crate) const ARCHIVE_IMPORT_EXTS: &[&str] = &["zip", "cbz", "7z", "cb7", "rar", "cbr"];

#[derive(Serialize, Clone)]
pub struct PickedFile {
    pub name: String, // 显示/排序用（文件夹场景为相对路径）
    pub path: String, // 本地绝对路径
    pub size: u64,
}

pub(crate) fn ext_of(p: &Path) -> String {
    p.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

pub(crate) fn kind_of(p: &Path) -> Option<&'static str> {
    let e = ext_of(p);
    if IMAGE_EXTS.contains(&e.as_str()) {
        Some("image")
    } else if ZIP_EXTS.contains(&e.as_str()) {
        Some("zip")
    } else if crate::archives::SEVENZ_EXTS.contains(&e.as_str())
        || crate::archives::RAR_EXTS.contains(&e.as_str())
    {
        Some("archive")
    } else {
        None
    }
}

pub(crate) fn fp_to_string(fp: FilePath) -> Option<String> {
    match fp {
        FilePath::Path(p) => Some(p.to_string_lossy().to_string()),
        FilePath::Url(u) => Some(u.to_string()),
    }
}

fn to_picked(path_str: String, rel: String) -> Option<PickedFile> {
    let p = PathBuf::from(&path_str);
    if kind_of(&p).is_none() {
        return None;
    }
    let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
    Some(PickedFile {
        name: rel,
        path: path_str,
        size,
    })
}

fn walk_dir(dir: &Path, base: &Path, out: &mut Vec<PickedFile>, parallel: bool) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut subs: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            subs.push(p);
            continue;
        }
        if kind_of(&p).is_none() {
            continue;
        }
        let rel = p
            .strip_prefix(base)
            .unwrap_or(&p)
            .to_string_lossy()
            .replace('\\', "/");
        let abs = p.to_string_lossy().to_string();
        if let Some(pf) = to_picked(abs, rel) {
            out.push(pf);
        }
    }
    subs.sort();
    // Sibling folders are independent. Only the top call fans out, so a deep tree
    // does not spawn a thread per directory. Order stays "files here, then sorted subs".
    if parallel && subs.len() >= 4 {
        let parts = std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(subs.len());
            for s in &subs {
                let s = s.clone();
                let base = base.to_path_buf();
                handles.push(scope.spawn(move || {
                    let mut local = Vec::new();
                    walk_dir(&s, &base, &mut local, false);
                    local
                }));
            }
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or_default())
                .collect::<Vec<_>>()
        });
        for part in parts {
            out.extend(part);
        }
        return;
    }
    for s in subs {
        walk_dir(&s, base, out, false);
    }
}

/// 选择文件夹（可多选），只返回文件夹路径
#[tauri::command]
pub fn pick_folder_paths(app: AppHandle) -> Vec<String> {
    app.dialog()
        .file()
        .blocking_pick_folders()
        .unwrap_or_default()
        .into_iter()
        .filter_map(fp_to_string)
        .collect()
}

/// 列出文件夹下的图片/压缩包（供书架打开时用）
#[tauri::command]
pub fn list_images(path: String) -> Result<Vec<PickedFile>, String> {
    let base = PathBuf::from(&path);
    if !base.is_dir() {
        return Err("文件夹不存在".to_string());
    }
    let mut out = Vec::new();
    walk_dir(&base, &base, &mut out, true);
    Ok(out)
}
