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
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        sevenz_rust::decompress_file_with_extract_fn(
            archive.to_string_lossy().as_ref(),
            dest.to_string_lossy().as_ref(),
            |entry, reader, dest| write_sevenz_entry(entry, reader, dest),
        )
        .map_err(|e| format!("7z 解压失败：{e}"))
    }));
    match result {
        Ok(r) => r,
        Err(_) => Err("7z 条目异常，已跳过".into()),
    }
}

fn write_sevenz_entry(
    entry: &sevenz_rust::SevenZArchiveEntry,
    reader: &mut dyn std::io::Read,
    dest: &Path,
) -> Result<bool, sevenz_rust::Error> {
    crate::pipeline::wait_foreground_idle();
    if entry.is_directory() {
        std::fs::create_dir_all(dest).map_err(sevenz_rust::Error::io)?;
        return Ok(true);
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(sevenz_rust::Error::io)?;
    }
    if entry.size() > crate::pipeline::PAGE_BYTE_CAP {
        std::io::copy(reader, &mut std::io::sink()).map_err(sevenz_rust::Error::io)?;
        return Ok(true);
    }
    let file = std::fs::File::create(dest).map_err(sevenz_rust::Error::io)?;
    let mut writer = std::io::BufWriter::new(file);
    let copied = copy_at_most(reader, &mut writer, crate::pipeline::PAGE_BYTE_CAP.saturating_add(1))
        .map_err(sevenz_rust::Error::io)?;
    if copied > crate::pipeline::PAGE_BYTE_CAP {
        drop(writer);
        let _ = std::fs::remove_file(dest);
        std::io::copy(reader, &mut std::io::sink()).map_err(sevenz_rust::Error::io)?;
    }
    Ok(true)
}

fn copy_at_most(reader: &mut dyn std::io::Read, writer: &mut dyn std::io::Write, max: u64) -> std::io::Result<u64> {
    let mut buf = [0u8; 64 * 1024];
    let mut copied = 0u64;
    while copied < max {
        let want = ((max - copied) as usize).min(buf.len());
        let n = reader.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n])?;
        copied += n as u64;
    }
    Ok(copied)
}

pub fn extract_rar(archive: &Path, dest: &Path) -> Result<(), String> {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| extract_rar_inner(archive, dest)));
    match result {
        Ok(r) => r,
        Err(_) => Err("RAR 条目异常，已跳过".into()),
    }
}

fn extract_rar_inner(archive: &Path, dest: &Path) -> Result<(), String> {
    let mut archive = unrar::Archive::new(archive)
        .open_for_processing()
        .map_err(|e| format!("无法打开 RAR：{e}"))?;
    while let Some(entry) = archive
        .read_header()
        .map_err(|e| format!("RAR 读取失败：{e}"))?
    {
        crate::pipeline::wait_foreground_idle();
        let huge = entry.entry().is_file() && entry.entry().unpacked_size > crate::pipeline::PAGE_BYTE_CAP;
        archive = if entry.entry().is_file() && !huge {
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
