//! Adapter commands designed in INTEGRATION_NOTES §2.3: the design frontend
//! (home.js / library / reader-boot) calls these, but the design backend package
//! has no implementation for them. They bridge the new frontend to the existing,
//! verified `shelf` machinery in main.rs.
//!
//! JS argument keys (Tauri v2 converts camelCase -> snake_case for plain commands):
//!   list_recent_books { limit }        list_recent_added { limit }
//!   import_paths      { paths, folderId }
//!   reveal_book       { bookId }       relink_book       { bookId, path }

use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Image extensions for import filtering, mirroring the frontend MH.imgRe
/// (mh-common.js): \.(jpe?g|jfif|png|apng|webp|gif|bmp|dib|avif|svg|ico|tif|tiff|heic|heif|jxl)$/i
/// Broader than main.rs IMAGE_EXTS (adds jfif, apng, dib, heic, heif).
const IMPORT_IMG_EXTS: &[&str] = &[
    // Servable formats only (see main.rs IMAGE_EXTS note): admitting tif /
    // heic / jxl / svg / ico here produced books whose every page failed
    // with "not a recognised image" in the reader.
    "jpg", "jpeg", "jfif", "jpe", "png", "apng", "webp", "gif", "bmp", "avif",
];

fn is_import_image(p: &Path) -> bool {
    IMPORT_IMG_EXTS.contains(&crate::ext_of(p).as_str())
}

fn file_name_of(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string())
}

fn stem_of(p: &Path) -> String {
    p.file_stem()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| file_name_of(p))
}

// ---------------------------------------------------------------------------
// list_recent_books / list_recent_added
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentBook {
    pub id: i64,
    pub title: String,
    pub page_count: i64,
    pub page: i64,
    pub frac: f64,
    /// shelf.updated_at (unix secs) as milliseconds for JS Date.
    pub last_read_at: i64,
    pub cover_ver: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentAdded {
    pub id: i64,
    pub title: String,
    pub page_count: i64,
    /// shelf.created_at (unix secs) as milliseconds for JS Date.
    pub added_at: i64,
    pub cover_ver: i64,
}

/// Books with reading progress (a `progress` row or shelf.last_page > 0),
/// most recently read first. page/frac come from `progress`, falling back to
/// shelf.last_page / 0 (INTEGRATION_NOTES §2.3).
#[tauri::command]
pub fn list_recent_books(limit: i64) -> Result<Vec<RecentBook>, String> {
    let conn = crate::db_conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT s.id, s.name, s.page_count,
                    COALESCE(p.page, s.last_page), COALESCE(p.frac, 0.0),
                    s.updated_at, s.cover_ver
             FROM shelf s LEFT JOIN progress p ON p.book_id = s.id
             WHERE p.book_id IS NOT NULL OR s.last_page > 0
             ORDER BY s.updated_at DESC
             LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([limit.max(0)], |r| {
            Ok(RecentBook {
                id: r.get(0)?,
                title: r.get(1)?,
                page_count: r.get(2)?,
                page: r.get(3)?,
                frac: r.get(4)?,
                last_read_at: r.get::<_, i64>(5)? * 1000,
                cover_ver: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// All books, most recently added first.
#[tauri::command]
pub fn list_recent_added(limit: i64) -> Result<Vec<RecentAdded>, String> {
    let conn = crate::db_conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT id, name, page_count, created_at, cover_ver
             FROM shelf ORDER BY created_at DESC LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([limit.max(0)], |r| {
            Ok(RecentAdded {
                id: r.get(0)?,
                title: r.get(1)?,
                page_count: r.get(2)?,
                added_at: r.get::<_, i64>(3)? * 1000,
                cover_ver: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// import_paths
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct ImportFailure {
    pub path: String,
    pub reason: String,
}

#[derive(Serialize)]
pub struct ImportResult {
    pub added: i64,
    pub skipped: i64,
    pub failed: Vec<ImportFailure>,
}

/// Insert one shelf row with shelf_add's shape (plus folder_id). Returns the new id.
fn shelf_insert(
    conn: &Connection,
    name: &str,
    kind: &str,
    paths_json: &str,
    page_count: i64,
    total_size: i64,
    folder_id: Option<i64>,
) -> Result<i64, String> {
    let now = crate::now_secs();
    conn.execute(
        "INSERT INTO shelf (name,kind,paths,cover,page_count,total_size,last_page,created_at,updated_at,folder_id)
         VALUES (?,?,?,'',?,?,0,?,?,?)",
        params![name, kind, paths_json, page_count, total_size, now, now, folder_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

/// Exact (kind, paths) duplicate check — duplicates count as `skipped`.
fn shelf_exists(conn: &Connection, kind: &str, paths_json: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM shelf WHERE kind=?1 AND paths=?2",
        params![kind, paths_json],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .unwrap_or(false)
}

fn trigger_cover(book_id: i64) {
    // Same code path as ensure_cover; failures are ignored (cover can be
    // generated lazily later via the mhcover 404 -> ensure_cover flow).
    if let Ok(dir) = crate::data_dir() {
        let _ = crate::cover_protocol::generate_cover(&dir.join("covers"), book_id);
    }
}

fn import_zip(
    conn: &Connection,
    path: &str,
    folder_id: Option<i64>,
    res: &mut ImportResult,
) {
    let paths_json = serde_json::to_string(&[path]).unwrap_or_default();
    if shelf_exists(conn, "zip", &paths_json) {
        res.skipped += 1;
        return;
    }
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            res.failed.push(ImportFailure {
                path: path.to_string(),
                reason: format!("无法打开压缩包：{e}"),
            });
            return;
        }
    };
    let mut ar = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(e) => {
            res.failed.push(ImportFailure {
                path: path.to_string(),
                reason: format!("压缩包无法读取：{e}"),
            });
            return;
        }
    };
    let mut page_count: i64 = 0;
    let mut total_size: i64 = 0;
    for i in 0..ar.len() {
        let Ok(f) = ar.by_index(i) else { continue };
        if f.is_dir() {
            continue;
        }
        // v0.3.x semantics: total_size sums ALL entries; page_count counts images only.
        total_size += f.size() as i64;
        if is_import_image(Path::new(f.name())) {
            page_count += 1;
        }
    }
    if page_count == 0 {
        res.failed.push(ImportFailure {
            path: path.to_string(),
            reason: "包里没有图片".to_string(),
        });
        return;
    }
    let name = stem_of(Path::new(path));
    match shelf_insert(conn, &name, "zip", &paths_json, page_count, total_size, folder_id) {
        Ok(id) => {
            res.added += 1;
            trigger_cover(id);
        }
        Err(e) => res.failed.push(ImportFailure {
            path: path.to_string(),
            reason: e,
        }),
    }
}

fn import_folder(
    conn: &Connection,
    path: &str,
    folder_id: Option<i64>,
    res: &mut ImportResult,
) {
    let paths_json = serde_json::to_string(&[path]).unwrap_or_default();
    if shelf_exists(conn, "folder", &paths_json) {
        res.skipped += 1;
        return;
    }
    // Reuse the verified walk (crate::list_images = walk_dir over the folder).
    let files = match crate::list_images(path.to_string()) {
        Ok(f) => f,
        Err(e) => {
            res.failed.push(ImportFailure {
                path: path.to_string(),
                reason: e,
            });
            return;
        }
    };
    if files.is_empty() {
        res.failed.push(ImportFailure {
            path: path.to_string(),
            reason: "文件夹里没有图片".to_string(),
        });
        return;
    }
    let total_size: i64 = files.iter().map(|f| f.size as i64).sum();
    let name = file_name_of(Path::new(path));
    match shelf_insert(
        conn,
        &name,
        "folder",
        &paths_json,
        files.len() as i64,
        total_size,
        folder_id,
    ) {
        Ok(id) => {
            res.added += 1;
            trigger_cover(id);
        }
        Err(e) => res.failed.push(ImportFailure {
            path: path.to_string(),
            reason: e,
        }),
    }
}

/// Rust-side import mirroring the verified v0.3.x frontend pipeline
/// (INTEGRATION_NOTES §2.3): zip/cbz paths become zip books, folder paths
/// become folder books, loose image files accumulate into ONE `images` book
/// named `<first stem> 等<N>张` with total_size 0. Exact (kind, paths)
/// duplicates count as skipped. folder_id (JS `folderId`) sets shelf.folder_id.
#[tauri::command]
pub fn import_paths(
    paths: Vec<String>,
    folder_id: Option<i64>,
) -> Result<ImportResult, String> {
    let conn = crate::db_conn()?;
    let mut res = ImportResult {
        added: 0,
        skipped: 0,
        failed: Vec::new(),
    };
    let mut loose: Vec<String> = Vec::new();
    for raw in &paths {
        let p = PathBuf::from(raw);
        if p.is_dir() {
            import_folder(&conn, raw, folder_id, &mut res);
        } else if crate::kind_of(&p) == Some("zip") {
            import_zip(&conn, raw, folder_id, &mut res);
        } else if is_import_image(&p) {
            loose.push(raw.clone());
        } else {
            res.failed.push(ImportFailure {
                path: raw.clone(),
                reason: "不支持的格式（RAR / 7z 请先解压）".to_string(),
            });
        }
    }
    if !loose.is_empty() {
        // v0.3.x: loose images are naturally sorted by file name into one book.
        loose.sort_by(|a, b| {
            natord::compare_ignore_case(&file_name_of(Path::new(a)), &file_name_of(Path::new(b)))
        });
        let paths_json = serde_json::to_string(&loose).unwrap_or_default();
        if shelf_exists(&conn, "images", &paths_json) {
            res.skipped += 1;
        } else {
            let name = format!(
                "{} 等{}张",
                stem_of(Path::new(&loose[0])),
                loose.len()
            );
            match shelf_insert(
                &conn,
                &name,
                "images",
                &paths_json,
                loose.len() as i64,
                0,
                folder_id,
            ) {
                Ok(id) => {
                    res.added += 1;
                    trigger_cover(id);
                }
                Err(e) => res.failed.push(ImportFailure {
                    path: loose.join(", "),
                    reason: e,
                }),
            }
        }
    }
    Ok(res)
}

// ---------------------------------------------------------------------------
// reveal_book / relink_book
// ---------------------------------------------------------------------------

fn shelf_kind_paths(conn: &Connection, book_id: i64) -> Result<(String, Vec<String>), String> {
    let mut stmt = conn
        .prepare("SELECT kind, paths FROM shelf WHERE id=?1")
        .map_err(|e| e.to_string())?;
    let mut rows = stmt
        .query_map([book_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?;
    match rows.next() {
        Some(r) => {
            let (kind, paths_json) = r.map_err(|e| e.to_string())?;
            let paths: Vec<String> = serde_json::from_str(&paths_json).unwrap_or_default();
            Ok((kind, paths))
        }
        None => Err("图书不存在".to_string()),
    }
}

/// Open the book's source location in Explorer: the folder itself for folder
/// books, `explorer /select,` on paths[0] for zip / images books.
/// No-op Ok off Windows, mirroring open_data_dir.
#[tauri::command]
pub fn reveal_book(book_id: i64) -> Result<(), String> {
    let conn = crate::db_conn()?;
    let (kind, paths) = shelf_kind_paths(&conn, book_id)?;
    let first = paths.first().cloned().unwrap_or_default();
    #[cfg(target_os = "windows")]
    {
        if kind == "folder" {
            std::process::Command::new("explorer")
                .arg(&first)
                .spawn()
                .map_err(|e| e.to_string())?;
        } else {
            std::process::Command::new("explorer")
                .arg(format!("/select,{}", first))
                .spawn()
                .map_err(|e| e.to_string())?;
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (kind, first);
    }
    Ok(())
}

/// Re-point a zip/folder book at a new source path: replaces paths[0] and sets
/// scan_sig = NULL so the next open_book rescans (INTEGRATION_NOTES §2.3).
#[tauri::command]
pub fn relink_book(book_id: i64, path: String) -> Result<(), String> {
    let conn = crate::db_conn()?;
    let (kind, mut paths) = shelf_kind_paths(&conn, book_id)?;
    if kind != "zip" && kind != "folder" {
        return Err("只有压缩包 / 文件夹图书可以重新定位".to_string());
    }
    if paths.is_empty() {
        paths.push(path);
    } else {
        paths[0] = path;
    }
    let paths_json = serde_json::to_string(&paths).map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE shelf SET paths=?2, scan_sig=NULL WHERE id=?1",
        params![book_id, paths_json],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
