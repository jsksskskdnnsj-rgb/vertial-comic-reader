//! Extra archive formats for import (ComiGo / exhentai fileLoader ideas, MIT — reimplemented).
//! zip/cbz stay on the existing zip path (`scan::index_book`). 7z/cb7/rar/cbr unpack
//! into a managed folder under the data dir and register as `folder` books, so the
//! same page index reads them. Nothing here searches for or downloads comics.

use crate::scan;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const SEVENZ_EXTS: &[&str] = &["7z", "cb7"];
pub(crate) const RAR_EXTS: &[&str] = &["rar", "cbr"];

pub(crate) fn archive_kind_ext(ext: &str) -> Option<&'static str> {
    let e = ext.to_ascii_lowercase();
    if crate::paths::ZIP_EXTS.iter().any(|x| *x == e) {
        return Some("zip");
    }
    if SEVENZ_EXTS.iter().any(|x| *x == e) {
        return Some("sevenz");
    }
    if RAR_EXTS.iter().any(|x| *x == e) {
        return Some("rar");
    }
    None
}

fn expanded_root() -> Result<PathBuf, String> {
    let dir = crate::data_dir()?.join("expanded");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn unique_dest(stem: &str) -> Result<PathBuf, String> {
    let root = expanded_root()?;
    let safe: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect();
    let mut dest = root.join(&safe);
    let mut n = 0;
    while dest.exists() {
        n += 1;
        dest = root.join(format!("{safe}-{n}"));
    }
    fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
    Ok(dest)
}

pub fn extract_sevenz(archive: &Path, dest: &Path) -> Result<(), String> {
    sevenz_rust::decompress_file(
        archive.to_string_lossy().as_ref(),
        dest.to_string_lossy().as_ref(),
    )
    .map_err(|e| format!("7z 解压失败：{e}"))
}

pub fn extract_rar(archive: &Path, dest: &Path) -> Result<(), String> {
    let mut archive = unrar::Archive::new(archive)
        .open_for_processing()
        .map_err(|e| format!("无法打开 RAR：{e}"))?;
    while let Some(entry) = archive
        .read_header()
        .map_err(|e| format!("RAR 读取失败：{e}"))?
    {
        archive = if entry.entry().is_file() {
            entry
                .extract_with_base(dest)
                .map_err(|e| format!("RAR 解压失败：{e}"))?
        } else {
            entry.skip().map_err(|e| format!("RAR 解压失败：{e}"))?
        };
    }
    Ok(())
}

/// Unpack archive → folder path. Returns None if the extension is not an archive we handle here.
pub fn import_unpack_to_folder(path: &str) -> Result<Option<PathBuf>, String> {
    let p = Path::new(path);
    let ext = crate::ext_of(p);
    let stem = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("book")
        .to_string();
    let dest = unique_dest(&stem)?;
    if SEVENZ_EXTS.iter().any(|e| *e == ext) {
        extract_sevenz(p, &dest)?;
    } else if RAR_EXTS.iter().any(|e| *e == ext) {
        extract_rar(p, &dest)?;
    } else {
        return Ok(None);
    }
    if scan::count_images_recursive(&dest) == 0 {
        let _ = fs::remove_dir_all(&dest);
        return Err("压缩包内没有可识别的漫画图片".into());
    }
    Ok(Some(dest))
}
