//! Chapter modes (navigation/display layer ONLY).
//! Split from design/claude-v04/reader_backend.rs `pub mod chapters`.
//! None of this touches virtual pages, slicing, cache keys or the extraction queue. Episodes are derived
//! on demand from (a) page NAMES already in pages_meta (folder mode) or (b) the page COUNT (pages mode),
//! so changing mode or N never rescans files and never changes what the strip renders.
//!
//! ADAPTED (§3): `episode_info` pages mode reads `shelf.page_count` (design read `books.page_count`).

use crate::commands::ReaderState;
use crate::db;
use crate::error::ReaderError;
use crate::model::*;
use crate::scan;
use rusqlite::{params, Connection, OptionalExtension};
use tauri::State;

pub const DEFAULT_PAGES: u32 = 20;
pub const MAX_PAGES: u32 = 9_999;

/// Run once from the migration step (setup). The design's `settings` / `book_settings`
/// tables are new in this app, so the DDL applies as-is. Global defaults are two rows
/// in the key/value `settings` table: 'chapter.mode', 'chapter.pages'.
pub const DDL: &str = "
  CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
  CREATE TABLE IF NOT EXISTS book_settings(book_id INTEGER PRIMARY KEY, chapter_mode TEXT, chapter_pages INTEGER);";

fn sane(n: i64) -> u32 {
    n.clamp(1, MAX_PAGES as i64) as u32
}

// ---- settings resolution -------------------------------------------------------------------
/// Global default: mode 'folder' (= the previous behaviour; flat books simply show no chapters), N = 20.
pub fn defaults(c: &Connection) -> Result<ChapterSettings, ReaderError> {
    let get = |k: &str| -> Result<Option<String>, ReaderError> {
        Ok(c.query_row("SELECT value FROM settings WHERE key=?1", [k], |r| r.get(0))
            .optional()?)
    };
    Ok(ChapterSettings {
        mode: get("chapter.mode")?
            .and_then(|s| ChapterMode::parse(&s))
            .unwrap_or(ChapterMode::Folder),
        pages: get("chapter.pages")?
            .and_then(|s| s.parse::<i64>().ok())
            .map(sane)
            .unwrap_or(DEFAULT_PAGES),
    })
}

pub struct Effective {
    pub mode: ChapterMode,
    pub pages: u32,
    pub overridden: bool,
}

/// Per-book value wins; NULL columns fall back to the global default, field by field.
pub fn effective(c: &Connection, book_id: i64) -> Result<Effective, ReaderError> {
    let d = defaults(c)?;
    let row: Option<(Option<String>, Option<i64>)> = c
        .query_row(
            "SELECT chapter_mode, chapter_pages FROM book_settings WHERE book_id=?1",
            [book_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (m, p) = row.unwrap_or((None, None));
    let mode = m.as_deref().and_then(ChapterMode::parse);
    let pages = p.map(sane);
    Ok(Effective {
        mode: mode.unwrap_or(d.mode),
        pages: pages.unwrap_or(d.pages),
        overridden: mode.is_some() || pages.is_some(),
    })
}

// ---- episode computation (pure) ------------------------------------------------------------
fn to_episodes(mut starts: Vec<(u32, String)>, total: u32) -> Vec<Episode> {
    if starts.first().map_or(false, |s| s.0 > 0) {
        starts.insert(0, (0, String::new()));
    } // loose pages before the first folder
    (0..starts.len())
        .map(|i| {
            let end = starts.get(i + 1).map_or(total, |n| n.0);
            Episode {
                index: i as u32 + 1,
                page: starts[i].0,
                len: end - starts[i].0,
                title: starts[i].1.clone(),
            }
        })
        .collect()
}
/// Folder mode: scan::chapters already returns [] unless >= 2 distinct folders exist.
pub fn folder(real: &[RealPage]) -> Vec<Episode> {
    to_episodes(
        scan::chapters(real)
            .into_iter()
            .map(|c| (c.page, c.title))
            .collect(),
        real.len() as u32,
    )
}
/// Pages mode: episode k = pages (k-1)*N+1 ..= k*N (1-based), last one may be shorter; < 2 episodes => [].
pub fn by_pages(total: u32, n: u32) -> Vec<Episode> {
    if total == 0 {
        return vec![];
    }
    let n = n.clamp(1, total);
    let count = total.div_ceil(n);
    if count < 2 {
        return vec![];
    }
    (0..count)
        .map(|k| {
            let p = k * n;
            Episode {
                index: k + 1,
                page: p,
                len: n.min(total - p),
                title: String::new(),
            }
        })
        .collect()
}
pub fn resolve(mode: ChapterMode, pages: u32, real: &[RealPage]) -> Vec<Episode> {
    match mode {
        ChapterMode::Off => vec![],
        ChapterMode::Folder => folder(real),
        ChapterMode::Pages => by_pages(real.len() as u32, pages),
    }
}
pub fn locate(eps: &[Episode], page: u32) -> Option<EpisodeInfo> {
    if eps.is_empty() {
        return None;
    }
    let i = eps.partition_point(|e| e.page <= page).saturating_sub(1);
    Some(EpisodeInfo {
        index: eps[i].index,
        total: eps.len() as u32,
    })
}

// ---- commands ------------------------------------------------------------------------------
#[tauri::command]
pub async fn get_chapter_defaults(
    state: State<'_, ReaderState>,
) -> Result<ChapterSettings, ReaderError> {
    defaults(&state.db.lock())
}

/// Settings page: the global default.
#[tauri::command]
pub async fn set_chapter_defaults(
    state: State<'_, ReaderState>,
    mode: ChapterMode,
    pages: u32,
) -> Result<(), ReaderError> {
    let c = state.db.lock();
    for (k, v) in [
        ("chapter.mode", mode.as_str().to_string()),
        ("chapter.pages", sane(pages as i64).to_string()),
    ] {
        c.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![k, v])?;
    }
    Ok(())
}

/// Reader quick settings with "Remember for this book" ON.
#[tauri::command]
pub async fn set_book_chapter_settings(
    state: State<'_, ReaderState>,
    book_id: i64,
    mode: ChapterMode,
    pages: u32,
) -> Result<(), ReaderError> {
    state.db.lock().execute(
        "INSERT INTO book_settings(book_id,chapter_mode,chapter_pages) VALUES(?1,?2,?3)
             ON CONFLICT(book_id) DO UPDATE SET chapter_mode=excluded.chapter_mode, chapter_pages=excluded.chapter_pages",
        params![book_id, mode.as_str(), sane(pages as i64)],
    )?;
    Ok(())
}

/// "Use default": drop this book's override (other per-book settings stay).
#[tauri::command]
pub async fn clear_book_chapter_settings(
    state: State<'_, ReaderState>,
    book_id: i64,
) -> Result<(), ReaderError> {
    state
        .db
        .lock()
        .execute("UPDATE book_settings SET chapter_mode=NULL, chapter_pages=NULL WHERE book_id=?1", [book_id])?;
    Ok(())
}

/// Batch lookup for lists ("Episode 12 / 128" next to progress). One call per list render.
/// Pages mode needs only shelf.page_count; folder mode reads that book's stored page names (no file access).
/// For very large libraries add a small in-memory cache keyed by (book_id, scan_sig, mode, N).
#[tauri::command]
pub async fn episode_info(
    state: State<'_, ReaderState>,
    items: Vec<EpisodeQuery>,
) -> Result<Vec<Option<EpisodeInfo>>, ReaderError> {
    let s = state.inner().clone();
    tauri::async_runtime::spawn_blocking(
        move || -> Result<Vec<Option<EpisodeInfo>>, ReaderError> {
            let c = s.db.lock();
            items
                .iter()
                .map(|q| -> Result<Option<EpisodeInfo>, ReaderError> {
                    let eff = effective(&c, q.book_id)?;
                    let eps = match eff.mode {
                        ChapterMode::Off => vec![],
                        ChapterMode::Pages => {
                            let total: u32 = c
                                .query_row(
                                    "SELECT page_count FROM shelf WHERE id=?1",
                                    [q.book_id],
                                    |r| r.get(0),
                                )
                                .unwrap_or(0);
                            by_pages(total, eff.pages)
                        }
                        ChapterMode::Folder => folder(&db::load_pages(&c, q.book_id)?),
                    };
                    Ok(locate(&eps, q.page))
                })
                .collect()
        },
    )
    .await
    .map_err(|e| ReaderError::Io(e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pages_mode_ranges() {
        let e = by_pages(100, 30);
        assert_eq!(
            e.iter()
                .map(|x| (x.index, x.page, x.len))
                .collect::<Vec<_>>(),
            vec![(1, 0, 30), (2, 30, 30), (3, 60, 30), (4, 90, 10)]
        );
        assert!(by_pages(100, 100).is_empty()
            && by_pages(100, 999).is_empty()
            && by_pages(0, 5).is_empty());
    }
    #[test]
    fn locate_finds_containing_episode() {
        let e = by_pages(100, 30);
        assert_eq!(locate(&e, 29).unwrap().index, 1);
        assert_eq!(locate(&e, 30).unwrap().index, 2);
        assert_eq!(locate(&e, 99).unwrap().index, 4);
        assert!(locate(&[], 3).is_none());
    }
    #[test]
    fn loose_pages_before_first_folder_form_episode_one() {
        let e = to_episodes(vec![(5, "A".into()), (20, "B".into())], 30);
        assert_eq!(
            e.iter().map(|x| (x.page, x.len)).collect::<Vec<_>>(),
            vec![(0, 5), (5, 15), (20, 10)]
        );
    }
}
