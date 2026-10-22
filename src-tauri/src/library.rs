//! Library management + page-edit commands (v0.5).
//!
//! These are the commands the repaired frontend (legacy-api.js) had to give up
//! on because the design backend never implemented them:
//!   reorder_pages / delete_pages / append_pages / reset_page_edits / page_edit_info
//!   shelf_rename / book_reset_progress / check_books_missing / import_merged / library_stats
//!
//! Page edits NEVER touch the user's source files. They are stored as an
//! ordered overlay in `edited_pages` (see db.rs); open_book prefers the
//! overlay over a fresh scan, and a forced rescan drops it. Appended pages
//! from another source are read through RealPage::src_path (zip/folder
//! sources are extracted/copied ONCE into data_dir/appended/<book_id>/).

use crate::commands::ReaderState;
use crate::db;
use crate::model::RealPage;
use crate::scan;
use rusqlite::params;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tauri::State;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn now_secs() -> i64 {
    crate::now_secs()
}

/// The book's CURRENT real page list: edited overlay if present, else the
/// stored scan (pages_meta), else a fresh scan of the source.
fn current_pages(state: &ReaderState, book_id: i64) -> Result<Vec<RealPage>, String> {
    if let Some(e) = db::load_edited_pages(&state.db.lock(), book_id).map_err(|e| e.to_string())? {
        return Ok(e);
    }
    let stored = db::load_pages(&state.db.lock(), book_id).map_err(|e| e.to_string())?;
    if !stored.is_empty() {
        return Ok(stored);
    }
    let row = db::book_row(&state.db.lock(), book_id).map_err(|e| e.to_string())?;
    let kind = crate::model::SourceKind::from_db(&row.kind)
        .ok_or_else(|| format!("不支持的图书类型：{}", row.kind))?;
    let first = row.paths.first().cloned().unwrap_or_default();
    let pages = match kind {
        crate::model::SourceKind::Zip => {
            scan::scan_zip(Path::new(&first)).map_err(|e| e.to_string())?
        }
        crate::model::SourceKind::Folder => scan::scan_folder(Path::new(&first)),
        crate::model::SourceKind::Images => scan::scan_images(&row.paths),
    };
    if pages.is_empty() {
        return Err("这本书没有可用的页面（源文件可能已移动或删除）".to_string());
    }
    Ok(pages)
}

/// Persist an edited list and drop every cached derivative of the old layout.
fn apply_edit(state: &ReaderState, book_id: i64, pages: &[RealPage]) -> Result<(), String> {
    if pages.is_empty() {
        return Err("至少保留 1 页".to_string());
    }
    db::store_edited_pages(&mut state.db.lock(), book_id, pages).map_err(|e| e.to_string())?;
    state.cache.purge_book(book_id);
    Ok(())
}

fn dims_of(path: &Path) -> (u32, u32) {
    imagesize::size(path)
        .map(|s| (s.width as u32, s.height as u32))
        .unwrap_or((0, 0))
}

fn sanitize_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("page");
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.trim().is_empty() || cleaned == "." {
        "page".to_string()
    } else {
        cleaned
    }
}

/// Expand ONE append/merge source (zip file | folder | image file) into
/// RealPages whose src_path points at a real readable file. Zip entries are
/// extracted into `extract_dir` (numbered, so re-runs never overwrite a file
/// a page already points at); folder/image pages point at the originals.
fn expand_source(
    src: &str,
    extract_dir: &Path,
    seq_start: usize,
) -> Result<Vec<RealPage>, String> {
    let p = PathBuf::from(src);
    let mut out = Vec::new();
    if p.is_dir() {
        for rp in scan::scan_folder(&p) {
            out.push(RealPage {
                src_path: Some(p.join(&rp.name).to_string_lossy().to_string()),
                ..rp
            });
        }
        return Ok(out);
    }
    if crate::kind_of(&p) == Some("zip") {
        std::fs::create_dir_all(extract_dir).map_err(|e| e.to_string())?;
        let mut pages = scan::scan_zip(&p).map_err(|e| e.to_string())?;
        pages.sort_by(|a, b| natord::compare_ignore_case(&a.name, &b.name));
        let file = std::fs::File::open(&p).map_err(|e| e.to_string())?;
        let mut ar = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
        for (i, rp) in pages.into_iter().enumerate() {
            let dest = extract_dir.join(format!(
                "{:05}_{}",
                seq_start + i,
                sanitize_file_name(&rp.name)
            ));
            if !dest.exists() {
                let mut e = ar.by_index(rp.entry as usize).map_err(|e| e.to_string())?;
                let mut f = std::fs::File::create(&dest).map_err(|e| e.to_string())?;
                std::io::copy(&mut e, &mut f).map_err(|e| e.to_string())?;
            }
            let (w, h) = if rp.w > 0 && rp.h > 0 {
                (rp.w, rp.h)
            } else {
                dims_of(&dest)
            };
            out.push(RealPage {
                entry: 0,
                name: rp.name,
                w,
                h,
                bytes: std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(rp.bytes),
                src_path: Some(dest.to_string_lossy().to_string()),
            });
        }
        return Ok(out);
    }
    if scan::is_image(src) || crate::kind_of(&p) == Some("image") {
        let (w, h) = dims_of(&p);
        out.push(RealPage {
            entry: 0,
            name: p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "page".to_string()),
            w,
            h,
            bytes: std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0),
            src_path: Some(src.to_string()),
        });
        return Ok(out);
    }
    Err(format!("不支持的来源：{src}"))
}

// ---------------------------------------------------------------------------
// page edits
// ---------------------------------------------------------------------------

/// mode: "smart" (natural sort by page name, the naming.js order) | "reverse".
#[tauri::command]
pub async fn reorder_pages(
    state: State<'_, ReaderState>,
    book_id: i64,
    mode: String,
) -> Result<u32, String> {
    let s = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut pages = current_pages(&s, book_id)?;
        match mode.as_str() {
            "smart" => pages.sort_by(|a, b| natord::compare_ignore_case(&a.name, &b.name)),
            "reverse" => pages.reverse(),
            other => return Err(format!("未知排序方式：{other}")),
        }
        let n = pages.len() as u32;
        apply_edit(&s, book_id, &pages)?;
        Ok(n)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// `pages` are 0-based REAL page indices (what reader-tools.js selects).
#[tauri::command]
pub async fn delete_pages(
    state: State<'_, ReaderState>,
    book_id: i64,
    pages: Vec<u32>,
) -> Result<u32, String> {
    let s = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let cur = current_pages(&s, book_id)?;
        let drop: HashSet<u32> = pages.into_iter().collect();
        let kept: Vec<RealPage> = cur
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !drop.contains(&(*i as u32)))
            .map(|(_, p)| p)
            .collect();
        let n = kept.len() as u32;
        apply_edit(&s, book_id, &kept)?;
        Ok(n)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Append pages from other sources (folders / zips / images), already ordered
/// by the frontend (naming.js sortSources). Returns the new total page count.
#[tauri::command]
pub async fn append_pages(
    state: State<'_, ReaderState>,
    book_id: i64,
    sources: Vec<String>,
) -> Result<u32, String> {
    let s = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut pages = current_pages(&s, book_id)?;
        let dir = crate::data_dir()
            .map_err(|e| e.to_string())?
            .join("appended")
            .join(book_id.to_string());
        let mut seq = std::fs::read_dir(&dir).map(|d| d.count()).unwrap_or(0);
        let mut added = 0usize;
        for src in &sources {
            let more = expand_source(src, &dir, seq)?;
            seq += more.len();
            added += more.len();
            pages.extend(more);
        }
        if added == 0 {
            return Err("所选来源里没有图片页面".to_string());
        }
        let n = pages.len() as u32;
        apply_edit(&s, book_id, &pages)?;
        Ok(n)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Drop the overlay: the book goes back to exactly what its source contains.
#[tauri::command]
pub async fn reset_page_edits(
    state: State<'_, ReaderState>,
    book_id: i64,
) -> Result<(), String> {
    let s = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        db::clear_edited_pages(&s.db.lock(), book_id).map_err(|e| e.to_string())?;
        s.cache.purge_book(book_id);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageEditInfo {
    pub edited: bool,
    pub page_count: u32,
}

#[tauri::command]
pub fn page_edit_info(book_id: i64) -> Result<PageEditInfo, String> {
    let conn = crate::db_conn()?;
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM edited_pages WHERE book_id=?1",
            [book_id],
            |r| r.get(0),
        )
        .unwrap_or(0);
    Ok(PageEditInfo {
        edited: n > 0,
        page_count: n as u32,
    })
}

// ---------------------------------------------------------------------------
// library management
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn shelf_rename(book_id: i64, name: String) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("书名不能为空".to_string());
    }
    let conn = crate::db_conn()?;
    conn.execute(
        "UPDATE shelf SET name=?2 WHERE id=?1",
        params![book_id, name],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Start over: drop the saved position + finished flag (source untouched).
#[tauri::command]
pub fn book_reset_progress(book_id: i64) -> Result<(), String> {
    let conn = crate::db_conn()?;
    conn.execute("DELETE FROM progress WHERE book_id=?1", [book_id])
        .map_err(|e| e.to_string())?;
    conn.execute(
        "UPDATE shelf SET last_page=0, finished=0 WHERE id=?1",
        [book_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Ids of books whose source can no longer be found (zip/folder path gone,
/// or every loose image gone). Drives the library's "missing" badge + relink.
#[tauri::command]
pub fn check_books_missing() -> Result<Vec<i64>, String> {
    let conn = crate::db_conn()?;
    let mut stmt = conn
        .prepare("SELECT id, kind, paths FROM shelf")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut missing = Vec::new();
    for row in rows {
        let (id, kind, paths_json) = row.map_err(|e| e.to_string())?;
        let paths: Vec<String> = serde_json::from_str(&paths_json).unwrap_or_default();
        let gone = match kind.as_str() {
            "zip" | "folder" => paths
                .first()
                .map(|p| !Path::new(p).exists())
                .unwrap_or(true),
            _ => paths.iter().all(|p| !Path::new(p).exists()),
        };
        if gone {
            missing.push(id);
        }
    }
    Ok(missing)
}

/// Merge several sources into ONE new book (import page's merge mode).
/// Loose/folder pages keep pointing at the originals; zip pages are extracted
/// once into data_dir/merged/<book_id>/. Returns the new book id.
#[tauri::command]
pub async fn import_merged(
    paths: Vec<String>,
    name: Option<String>,
    folder_id: Option<i64>,
) -> Result<i64, String> {
    tauri::async_runtime::spawn_blocking(move || {
        if paths.is_empty() {
            return Err("没有选择任何来源".to_string());
        }
        let conn = crate::db_conn()?;
        let now = now_secs();
        let title = name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| {
                let first = Path::new(&paths[0]);
                let stem = first
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "合并图书".to_string());
                format!("{stem} 等 {} 个来源", paths.len())
            });
        // Insert first (kind 'images' with an empty list) to learn the id,
        // then fill paths + the edited overlay once extraction knows the id.
        conn.execute(
            "INSERT INTO shelf (name,kind,paths,cover,page_count,total_size,last_page,created_at,updated_at,folder_id)
             VALUES (?1,'images','[]','',0,0,0,?2,?2,?3)",
            params![title, now, folder_id],
        )
        .map_err(|e| e.to_string())?;
        let book_id = conn.last_insert_rowid();
        let dir = crate::data_dir()?
            .join("merged")
            .join(book_id.to_string());
        let mut pages: Vec<RealPage> = Vec::new();
        let mut seq = 0usize;
        for src in &paths {
            match expand_source(src, &dir, seq) {
                Ok(more) => {
                    seq += more.len();
                    pages.extend(more);
                }
                Err(e) => {
                    let _ = conn.execute("DELETE FROM shelf WHERE id=?1", [book_id]);
                    return Err(e);
                }
            }
        }
        if pages.is_empty() {
            let _ = conn.execute("DELETE FROM shelf WHERE id=?1", [book_id]);
            return Err("所选来源里没有图片页面".to_string());
        }
        // kind 'images' reads shelf.paths as the page list: use src_path when
        // present (extracted zip pages), else the page name (absolute path).
        let list: Vec<String> = pages
            .iter()
            .map(|p| p.src_path.clone().unwrap_or_else(|| p.name.clone()))
            .collect();
        let total: i64 = pages.iter().map(|p| p.bytes as i64).sum();
        conn.execute(
            "UPDATE shelf SET paths=?2, page_count=?3, total_size=?4 WHERE id=?1",
            params![
                book_id,
                serde_json::to_string(&list).unwrap_or_default(),
                pages.len() as i64,
                total
            ],
        )
        .map_err(|e| e.to_string())?;
        // scan_images would re-derive dims in shelf.paths order on first open;
        // the overlay pins the exact merged order + extracted-page mapping.
        let mut conn = crate::db_conn()?;
        db::store_edited_pages(&mut conn, book_id, &pages).map_err(|e| e.to_string())?;
        if let Ok(d) = crate::data_dir() {
            let _ = crate::cover_protocol::generate_cover(&d.join("covers"), book_id);
        }
        Ok(book_id)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStats {
    pub books: i64,
    pub pages: i64,
    pub folders: i64,
    pub finished: i64,
    pub in_progress: i64,
    pub bookmarked_books: i64,
}

#[tauri::command]
pub fn library_stats() -> Result<LibraryStats, String> {
    let conn = crate::db_conn()?;
    let one = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0);
    Ok(LibraryStats {
        books: one("SELECT COUNT(*) FROM shelf"),
        pages: one("SELECT COALESCE(SUM(page_count),0) FROM shelf"),
        folders: one("SELECT COUNT(*) FROM folders"),
        finished: one("SELECT COUNT(*) FROM shelf WHERE finished=1"),
        in_progress: one("SELECT COUNT(*) FROM shelf WHERE last_page>0 AND finished=0"),
        bookmarked_books: one("SELECT COUNT(DISTINCT book_id) FROM bookmarks"),
    })
}
