//! Local library backup (SQLite + reader config); idea from Houdoku backup util (MIT) — reimplemented.

use crate::appdata::data_dir;
use serde::Serialize;
use std::fs::File;
use std::io::{copy, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupResult {
    pub path: String,
    pub bytes: u64,
}

fn add_file<W: Write + std::io::Seek>(zw: &mut ZipWriter<W>, disk: &Path, name: &str) -> Result<(), String> {
    if !disk.is_file() {
        return Ok(());
    }
    zw.start_file(name, SimpleFileOptions::default())
        .map_err(|e| e.to_string())?;
    let mut f = File::open(disk).map_err(|e| e.to_string())?;
    copy(&mut f, zw).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn export_library_backup(dest_path: String) -> Result<BackupResult, String> {
    let dir = data_dir()?;
    let dest = PathBuf::from(dest_path.trim());
    if dest.as_os_str().is_empty() {
        return Err("未选择保存位置".into());
    }
    let file = File::create(&dest).map_err(|e| e.to_string())?;
    let mut zw = ZipWriter::new(file);
    add_file(&mut zw, &dir.join("mh-reader.db"), "mh-reader.db")?;
    add_file(&mut zw, &dir.join("reader-config.json"), "reader-config.json")?;
    let finished = zw.finish().map_err(|e| e.to_string())?;
    let bytes = finished.metadata().map(|m| m.len()).unwrap_or(0);
    Ok(BackupResult {
        path: dest.to_string_lossy().to_string(),
        bytes,
    })
}

#[tauri::command]
pub fn import_library_backup(src_path: String) -> Result<(), String> {
    let dir = data_dir()?;
    let src = PathBuf::from(src_path.trim());
    let file = File::open(&src).map_err(|e| format!("无法打开备份：{e}"))?;
    let mut ar = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    for i in 0..ar.len() {
        let mut entry = ar.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().to_string();
        if name != "mh-reader.db" && name != "reader-config.json" {
            continue;
        }
        let out = dir.join(name);
        let tmp = out.with_extension("bakpart");
        let mut f = File::create(&tmp).map_err(|e| e.to_string())?;
        copy(&mut entry, &mut f).map_err(|e| e.to_string())?;
        drop(f);
        std::fs::rename(&tmp, &out).map_err(|e| e.to_string())?;
    }
    Ok(())
}
