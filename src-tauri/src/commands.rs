//! Reader core commands (split from design/claude-v04/reader_backend.rs `pub mod commands`).
//!
//! ADAPTED per INTEGRATION_NOTES §3:
//! - `open_blocking` reads the book row from `shelf` via crate::db (no `books` table).
//! - SourceKind::Images branch is IMPLEMENTED via `scan::scan_images` on the shelf
//!   paths array (the design returned Unsupported there).
//! - BookMeta.path for Images = paths[0] as an anchor only; page files are read via
//!   PageMeta.name (absolute path) in the extractor.
//! - Resume falls back to {page: shelf.last_page, frac: 0} when there is no
//!   progress row (design only had the progress table).

use crate::cache::PageCache;
use crate::chapters;
use crate::db::{self, Db};
use crate::error::ReaderError;
use crate::extractor::{Extractor, Window};
use crate::model::*;
use crate::scan;
use std::{path::PathBuf, sync::Arc};
use tauri::State;

#[derive(Clone)]
pub struct ReaderState {
    pub db: Db,
    pub cache: Arc<PageCache>,
    pub extractor: Arc<Extractor>,
}

fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, ReaderError> + Send + 'static,
) -> impl std::future::Future<Output = Result<T, ReaderError>> {
    async move {
        tauri::async_runtime::spawn_blocking(f)
            .await
            .map_err(|e| ReaderError::Io(e.to_string()))?
    }
}

pub(crate) fn open_blocking(
    s: &ReaderState,
    id: i64,
    force: bool,
) -> Result<OpenedBook, ReaderError> {
    let row = db::book_row(&s.db.lock(), id)?;
    let kind = SourceKind::from_db(&row.kind)
        .ok_or_else(|| ReaderError::Unsupported(row.kind.clone()))?;
    // zip/folder: the single source path is paths[0]. images: paths is the page list
    // itself; `path` below is only an anchor (BookMeta.path is unused for Images —
    // the extractor reads PageMeta.name, the absolute per-page path).
    let first = row.paths.first().cloned().unwrap_or_default();
    let anchor = row
        .paths
        .iter()
        .find(|p| PathBuf::from(p).exists())
        .cloned()
        .unwrap_or_else(|| first.clone());
    let path = PathBuf::from(&anchor);
    if kind != SourceKind::Images && !row.paths.iter().any(|p| PathBuf::from(p).exists()) {
        return Err(ReaderError::SourceMissing(first));
    } // -> "Locate file..." dialog
    // Page-edit overlay: a user-edited page list wins over the scan. A forced
    // rescan is the explicit "back to the source as it is" action, so it drops
    // the overlay first.
    if force {
        db::clear_edited_pages(&s.db.lock(), id)?;
    }
    let edited = if force {
        None
    } else {
        db::load_edited_pages(&s.db.lock(), id)?
    };
    let sig = if kind == SourceKind::Folder && row.paths.len() > 1 && edited.is_none() {
        let mut parts = Vec::new();
        for p in &row.paths {
            parts.push(
                scan::quick_sig(SourceKind::Folder, std::path::Path::new(p))
                    .unwrap_or_else(|_| "missing".into()),
            );
        }
        parts.join("|")
    } else if let Some(e) = &edited {
        // Content-derived: every page edit changes the sig, so JS image URLs
        // (which carry `s=<sig>`) never serve a stale page from the webview cache.
        let mut h: u64 = 0xcbf29ce484222325;
        for p in e {
            for b in format!("{}|{}|{:?};", p.entry, p.name, p.src_path).bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
        }
        format!("edited:{h:016x}")
    } else {
        scan::quick_sig(kind, &path)?
    };

    let cached = if !force && edited.is_none() && row.scan_sig.as_deref() == Some(sig.as_str()) {
        db::load_pages(&s.db.lock(), id)?
    } else {
        vec![]
    };
    let real = if let Some(e) = edited {
        // Mirror into pages_meta (bookmarks / episode_info read names from there).
        db::sync_pages_meta(&s.db.lock(), id, &e)?;
        e
    } else if !cached.is_empty() {
        cached
    } else {
        let fresh = match kind {
            // zip: central dir + header reads, folder: header reads, images: header reads per file
            SourceKind::Zip => scan::scan_zip(&path)?,
            SourceKind::Folder => {
                if row.paths.len() > 1 {
                    scan::scan_folder_parts(&row.paths)
                } else {
                    scan::scan_folder(&path)
                }
            }
            SourceKind::Images => scan::scan_images(&row.paths),
        };
        if fresh.is_empty() {
            return Err(ReaderError::Corrupt("no pages found".into()));
        }
        s.cache.purge_book(id); // source changed: derived files are stale
        db::store_pages(&mut s.db.lock(), id, &fresh, &sig)?;
        fresh
    };

    let pages = scan::build_pages(&real); // slicing decided here, every open
    let groups = scan::groups(&pages);
    let meta = Arc::new(BookMeta {
        id,
        kind,
        path,
        sig: sig.clone(),
        pages,
        groups,
    });
    let eff = chapters::effective(&s.db.lock(), id)?; // per-book override, else global default
    let resume = db::load_progress(&s.db.lock(), id)?
        .filter(|p| (p.page as usize) < real.len())
        .or_else(|| {
            // §3 fallback: no progress row -> mirror the legacy shelf.last_page.
            let lp = row.last_page.max(0) as u32;
            if (lp as usize) < real.len() {
                Some(SavedPosition {
                    page: lp,
                    frac: 0.0,
                })
            } else {
                None
            }
        });
    let out = OpenedBook {
        book_id: id,
        title: row.title,
        sig,
        real_pages: real.len() as u32,
        dims: meta.pages.iter().map(|p| [p.w, p.h]).collect(),
        src: meta.pages.iter().map(|p| p.src_page).collect(),
        chapters: chapters::resolve(eff.mode, eff.pages, &real), // Off => [], Folder => by dir, Pages => by N
        folder_chapters: chapters::folder(&real),
        chapter_mode: eff.mode,
        chapter_pages: eff.pages,
        chapter_overridden: eff.overridden,
        resume,
    };
    s.extractor.register_book(meta);
    Ok(out)
}

// Tauri v2: JS passes camelCase keys (bookId) -> snake_case args (book_id). async + borrowed State => must return Result.
#[tauri::command]
pub async fn open_book(
    state: State<'_, ReaderState>,
    book_id: i64,
) -> Result<OpenedBook, ReaderError> {
    let s = state.inner().clone();
    blocking(move || open_blocking(&s, book_id, false)).await
}
#[tauri::command]
pub async fn rescan_book(
    state: State<'_, ReaderState>,
    book_id: i64,
) -> Result<OpenedBook, ReaderError> {
    let s = state.inner().clone();
    blocking(move || open_blocking(&s, book_id, true)).await
}
#[tauri::command]
pub async fn close_book(state: State<'_, ReaderState>, book_id: i64) -> Result<(), ReaderError> {
    state.extractor.close_book(book_id);
    Ok(())
}
#[tauri::command]
pub async fn set_reading_window(
    state: State<'_, ReaderState>,
    book_id: i64,
    center: u32,
    direction: i8,
    ahead: u32,
    behind: u32,
    variant: u16,
) -> Result<(), ReaderError> {
    if !VARIANTS.contains(&variant) {
        return Err(ReaderError::BadRequest("variant".into()));
    }
    state.extractor.set_window(Window {
        book: book_id,
        center,
        dir: direction.signum().max(-1).min(1),
        ahead: ahead.min(12),
        behind: behind.min(4),
        variant,
    })
}
#[tauri::command]
pub async fn save_progress(
    state: State<'_, ReaderState>,
    book_id: i64,
    pos: SavedPosition,
    reached_end: bool,
) -> Result<(), ReaderError> {
    let s = state.inner().clone();
    blocking(move || db::save_progress(&s.db.lock(), book_id, pos, reached_end)).await
}
#[tauri::command]
pub async fn load_progress(
    state: State<'_, ReaderState>,
    book_id: i64,
) -> Result<Option<SavedPosition>, ReaderError> {
    let s = state.inner().clone();
    blocking(move || db::load_progress(&s.db.lock(), book_id)).await
}
#[tauri::command]
pub async fn fix_page_dims(
    state: State<'_, ReaderState>,
    book_id: i64,
    fixes: Vec<DimFix>,
) -> Result<(), ReaderError> {
    let s = state.inner().clone();
    blocking(move || {
        db::fix_dims(&mut s.db.lock(), book_id, &fixes)?;
        s.cache.purge_book(book_id); // slicing may differ next open (virtual indices shift) -> cached slices are stale
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn cache_stats(state: State<'_, ReaderState>) -> Result<CacheStats, ReaderError> {
    Ok(state.cache.stats())
}
#[tauri::command]
pub async fn cache_set_limit(state: State<'_, ReaderState>, bytes: u64) -> Result<(), ReaderError> {
    state.cache.set_limit(bytes.max(64 << 20));
    let win = state.extractor.window();
    state
        .cache
        .evict_if_needed(&|k| win.map_or(false, |w| w.pins(k)));
    Ok(())
}
#[tauri::command]
pub async fn cache_clear(
    state: State<'_, ReaderState>,
    book_id: Option<i64>,
) -> Result<u64, ReaderError> {
    let s = state.inner().clone();
    blocking(move || {
        Ok(match book_id {
            Some(id) => s.cache.purge_book(id),
            None => {
                let win = s.extractor.window();
                let n = s
                    .cache
                    .clear(&|k| win.map_or(false, |w| w.pins(k)));
                // v0.3 `zip_extract_page` wrote data/page_cache. The reader no longer
                // uses that folder; a full clear removes it along with the page cache.
                if let Ok(dir) = crate::data_dir() {
                    let legacy = dir.join("page_cache");
                    if legacy.is_dir() {
                        let _ = std::fs::remove_dir_all(legacy);
                    }
                }
                n
            }
        })
    })
    .await
}
