//! Bookmarks anchored to the IMAGE, not the page number.
//! Split from design/claude-v04/reader_backend.rs `pub mod bookmarks`, unchanged.
//! Identity = the page's name inside the source, exactly as stored in pages_meta.name:
//!   ZIP: the entry's full path (normalised to '/'), folder book: path relative to the book folder, loose images: absolute path.
//! page_hint is only a cache of "where it was last seen". Resolution NEVER uses the hint to pick an image:
//!   name found  -> status "ok"     (page = the CURRENT index of that image; hint is refreshed)
//!   name absent -> status "broken" (page = None). The UI may offer the hint as an explicit user choice
//!                  ("Go to old position" / "Anchor to page N") but nothing ever jumps there silently.
//! So adding, removing or re-sorting pages inside the archive cannot move a bookmark onto a different image.

use crate::commands::ReaderState;
use crate::error::ReaderError;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::HashMap;
use tauri::State;

/// Run AFTER db::DDL (needs pages_meta). Idempotent. New installs get the final shape; v1 tables are migrated by `migrate`.
pub const DDL: &str = "
  CREATE TABLE IF NOT EXISTS bookmarks(
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    book_id    INTEGER NOT NULL,
    entry_name TEXT    NOT NULL,                 -- identity of the image inside the source
    page_hint  INTEGER NOT NULL DEFAULT 0,       -- cached index only
    frac       REAL    NOT NULL DEFAULT 0,       -- 0..1 down the WHOLE image (all slices of a tall image)
    note       TEXT,
    created_at INTEGER NOT NULL);                -- unix ms
  CREATE INDEX IF NOT EXISTS idx_bookmarks_book_entry ON bookmarks(book_id, entry_name);
  CREATE INDEX IF NOT EXISTS idx_pages_meta_name      ON pages_meta(book_id, name);   -- name -> page lookups";

/// v1 was (book_id, page, offset, note). Renames the columns, adds entry_name, backfills it from pages_meta where possible.
pub fn migrate(c: &Connection) -> Result<(), ReaderError> {
    let cols: Vec<String> = {
        let mut s = c.prepare("PRAGMA table_info(bookmarks)")?;
        let rows = s.query_map([], |r| r.get::<_, String>(1))?;
        rows.collect::<Result<_, _>>()?
    };
    if !cols.is_empty() {
        if cols.iter().any(|n| n == "offset") {
            c.execute_batch("ALTER TABLE bookmarks RENAME COLUMN \"offset\" TO frac;")?;
        }
        if cols.iter().any(|n| n == "page") && !cols.iter().any(|n| n == "page_hint") {
            c.execute_batch("ALTER TABLE bookmarks RENAME COLUMN page TO page_hint;")?;
        }
        if !cols.iter().any(|n| n == "entry_name") {
            c.execute_batch("ALTER TABLE bookmarks ADD COLUMN entry_name TEXT NOT NULL DEFAULT '';")?;
        }
        if !cols.iter().any(|n| n == "created_at") {
            c.execute_batch("ALTER TABLE bookmarks ADD COLUMN created_at INTEGER NOT NULL DEFAULT 0;")?;
        }
    }
    c.execute_batch(DDL)?;
    Ok(())
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum BookmarkStatus {
    Ok,
    Broken,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub id: i64,
    pub book_id: i64,
    pub entry_name: String, // identity (full path inside the source)
    pub file_name: String,  // last path segment, for display
    pub page_hint: u32,     // cached index; refreshed whenever the name resolves
    pub page: Option<u32>,  // CURRENT real page of that image, found BY NAME; None = broken
    pub frac: f64,
    pub note: Option<String>,
    pub created_at: i64,
    pub status: BookmarkStatus,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkCount {
    pub book_id: i64,
    pub count: u32,
}

struct Row {
    id: i64,
    book_id: i64,
    entry: String,
    hint: u32,
    frac: f64,
    note: Option<String>,
    created: i64,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
fn clean_note(n: Option<String>) -> Option<String> {
    n.map(|s| s.trim().chars().take(500).collect::<String>())
        .filter(|s| !s.is_empty())
}
fn file_name(entry: &str) -> String {
    entry
        .rsplit(|c| c == '/' || c == '\\')
        .next()
        .unwrap_or(entry)
        .to_string()
}

/// name -> current real page. The ONLY way a bookmark finds its image.
fn names(c: &Connection, book_id: i64) -> Result<HashMap<String, u32>, ReaderError> {
    let mut s = c.prepare("SELECT name, page FROM pages_meta WHERE book_id=?1")?;
    let rows = s.query_map([book_id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}
pub fn resolve_entry(names: &HashMap<String, u32>, entry: &str) -> Option<u32> {
    names.get(entry).copied()
} // never falls back to the hint

fn row_of(r: &rusqlite::Row) -> rusqlite::Result<Row> {
    Ok(Row {
        id: r.get(0)?,
        book_id: r.get(1)?,
        entry: r.get(2)?,
        hint: r.get(3)?,
        frac: r.get(4)?,
        note: r.get(5)?,
        created: r.get(6)?,
    })
}
const COLS: &str = "id, book_id, entry_name, page_hint, frac, note, created_at";
fn get_row(c: &Connection, id: i64) -> Result<Row, ReaderError> {
    c.query_row(&format!("SELECT {COLS} FROM bookmarks WHERE id=?1"), [id], row_of)
        .optional()?
        .ok_or_else(|| ReaderError::BadRequest("bookmark not found".into()))
}
fn to_bookmark(r: Row, names: &HashMap<String, u32>) -> Bookmark {
    let page = resolve_entry(names, &r.entry);
    Bookmark {
        id: r.id,
        book_id: r.book_id,
        file_name: file_name(&r.entry),
        page_hint: page.unwrap_or(r.hint),
        page,
        frac: r.frac,
        note: r.note,
        created_at: r.created,
        status: if page.is_some() {
            BookmarkStatus::Ok
        } else {
            BookmarkStatus::Broken
        },
        entry_name: r.entry,
    }
}

// ---- commands -------------------------------------------------------------------------------------
/// Bookmark the image at real page `page`, `frac` of the way down the whole image (the reader sends the
/// position at the VIEWPORT CENTER; for a sliced tall image that is the real page plus its slice offset).
/// Idempotent: an existing bookmark on the same image within 1% of the same offset is returned unchanged.
#[tauri::command]
pub async fn add_bookmark(
    state: State<'_, ReaderState>,
    book_id: i64,
    page: u32,
    frac: f64,
    note: Option<String>,
) -> Result<Bookmark, ReaderError> {
    let c = state.db.lock();
    let nm = names(&c, book_id)?;
    if nm.is_empty() {
        return Err(ReaderError::BookNotOpen);
    } // open the book (open_book) first
    let entry: String = c
        .query_row(
            "SELECT name FROM pages_meta WHERE book_id=?1 AND page=?2",
            params![book_id, page],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(ReaderError::PageOutOfRange)?;
    let frac = if frac.is_finite() {
        frac.clamp(0.0, 0.9999)
    } else {
        0.0
    };
    if let Some(id) = c
        .query_row(
            "SELECT id FROM bookmarks WHERE book_id=?1 AND entry_name=?2 AND ABS(frac-?3)<0.01 LIMIT 1",
            params![book_id, entry, frac],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    {
        return Ok(to_bookmark(get_row(&c, id)?, &nm));
    }
    c.execute("INSERT INTO bookmarks(book_id, entry_name, page_hint, frac, note, created_at) VALUES(?1,?2,?3,?4,?5,?6)",
                  params![book_id, entry, page, frac, clean_note(note), now_ms()])?;
    let id = c.last_insert_rowid();
    Ok(to_bookmark(get_row(&c, id)?, &nm))
}

/// All bookmarks of a book, resolved BY NAME against the current page list, in reading order
/// (broken ones last, oldest first). Refreshes page_hint for bookmarks whose image moved.
/// Call after open_book (which keeps pages_meta current).
#[tauri::command]
pub async fn list_bookmarks(
    state: State<'_, ReaderState>,
    book_id: i64,
) -> Result<Vec<Bookmark>, ReaderError> {
    let c = state.db.lock();
    let nm = names(&c, book_id)?;
    if nm.is_empty() {
        return Err(ReaderError::BookNotOpen);
    }
    // v1 leftovers (entry_name = ''): bind them to the image at their old index ONCE, then they are name-anchored like the rest.
    c.execute("UPDATE bookmarks SET entry_name = COALESCE((SELECT name FROM pages_meta p WHERE p.book_id=bookmarks.book_id AND p.page=bookmarks.page_hint), '')
                   WHERE book_id=?1 AND entry_name=''", [book_id])?;
    let rows: Vec<Row> = {
        let mut s = c.prepare(&format!("SELECT {COLS} FROM bookmarks WHERE book_id=?1"))?;
        let it = s.query_map([book_id], row_of)?;
        it.collect::<Result<_, _>>()?
    };
    let mut out: Vec<Bookmark> = Vec::with_capacity(rows.len());
    for r in rows {
        let stale = resolve_entry(&nm, &r.entry).filter(|p| *p != r.hint);
        let id = r.id;
        if let Some(p) = stale {
            c.execute(
                "UPDATE bookmarks SET page_hint=?2 WHERE id=?1",
                params![id, p],
            )?;
        }
        out.push(to_bookmark(r, &nm));
    }
    out.sort_by(|a, b| match (a.page, b.page) {
        (Some(x), Some(y)) => x
            .cmp(&y)
            .then(a.frac.partial_cmp(&b.frac).unwrap_or(std::cmp::Ordering::Equal)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.created_at.cmp(&b.created_at),
    });
    Ok(out)
}

#[tauri::command]
pub async fn delete_bookmark(
    state: State<'_, ReaderState>,
    bookmark_id: i64,
) -> Result<(), ReaderError> {
    state
        .db
        .lock()
        .execute("DELETE FROM bookmarks WHERE id=?1", [bookmark_id])?;
    Ok(())
}

/// `note` = None or blank clears it; trimmed; max 500 chars. Returns the bookmark as now stored.
#[tauri::command]
pub async fn update_bookmark_note(
    state: State<'_, ReaderState>,
    bookmark_id: i64,
    note: Option<String>,
) -> Result<Bookmark, ReaderError> {
    let c = state.db.lock();
    let row = get_row(&c, bookmark_id)?;
    c.execute(
        "UPDATE bookmarks SET note=?2 WHERE id=?1",
        params![bookmark_id, clean_note(note)],
    )?;
    Ok(to_bookmark(
        get_row(&c, bookmark_id)?,
        &names(&c, row.book_id)?,
    ))
}

/// Explicit repair for a BROKEN bookmark: the user picks "Anchor to page N" (N = old hint) and the bookmark now
/// points at whatever image is at real page `page` today. Never called automatically.
#[tauri::command]
pub async fn reanchor_bookmark(
    state: State<'_, ReaderState>,
    bookmark_id: i64,
    page: u32,
) -> Result<Bookmark, ReaderError> {
    let c = state.db.lock();
    let row = get_row(&c, bookmark_id)?;
    let entry: String = c
        .query_row(
            "SELECT name FROM pages_meta WHERE book_id=?1 AND page=?2",
            params![row.book_id, page],
            |r| r.get(0),
        )
        .optional()?
        .ok_or(ReaderError::PageOutOfRange)?;
    c.execute(
        "UPDATE bookmarks SET entry_name=?2, page_hint=?3 WHERE id=?1",
        params![bookmark_id, entry, page],
    )?;
    Ok(to_bookmark(
        get_row(&c, bookmark_id)?,
        &names(&c, row.book_id)?,
    ))
}

/// For the library-card ribbon: bookmark count per book (books without bookmarks are omitted).
#[tauri::command]
pub async fn bookmark_counts(
    state: State<'_, ReaderState>,
    book_ids: Vec<i64>,
) -> Result<Vec<BookmarkCount>, ReaderError> {
    let c = state.db.lock();
    let mut s = c.prepare("SELECT COUNT(*) FROM bookmarks WHERE book_id=?1")?;
    let mut out = Vec::new();
    for id in book_ids {
        let n: u32 = s.query_row([id], |r| r.get(0))?;
        if n > 0 {
            out.push(BookmarkCount {
                book_id: id,
                count: n,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn names_of(v: &[&str]) -> HashMap<String, u32> {
        v.iter()
            .enumerate()
            .map(|(i, n)| (n.to_string(), i as u32))
            .collect()
    }
    #[test]
    fn found_by_name_even_after_pages_are_inserted_or_reordered() {
        let before = names_of(&["a/01.jpg", "a/02.jpg", "a/03.jpg"]);
        assert_eq!(resolve_entry(&before, "a/02.jpg"), Some(1));
        let after = names_of(&["a/00_credits.jpg", "a/01.jpg", "a/03.jpg", "a/02.jpg"]); // page added + re-sorted
        assert_eq!(resolve_entry(&after, "a/02.jpg"), Some(3)); // still the SAME image
    }
    #[test]
    fn missing_image_is_broken_not_a_neighbour() {
        let after = names_of(&["a/01.jpg", "a/03.jpg"]);
        assert_eq!(resolve_entry(&after, "a/02.jpg"), None); // index 1 now holds 03.jpg: must NOT be used
    }
    #[test]
    fn display_name_is_last_segment() {
        assert_eq!(file_name("Vol 1/ch02/005.jpg"), "005.jpg");
        assert_eq!(file_name("x.jpg"), "x.jpg");
    }
    #[test]
    fn notes_are_trimmed_and_blank_clears() {
        assert_eq!(clean_note(Some("  hi  ".into())).as_deref(), Some("hi"));
        assert_eq!(clean_note(Some("   ".into())), None);
        assert_eq!(clean_note(None), None);
    }
}
