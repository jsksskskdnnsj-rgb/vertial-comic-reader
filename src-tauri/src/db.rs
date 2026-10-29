//! Tables used by the reader (split from design/claude-v04/reader_backend.rs `pub mod db`).
//!
//! ADAPTED per INTEGRATION_NOTES §3: there is no `books` table — the existing
//! `shelf` table IS the book table. Design field mapping:
//!   books.id          -> shelf.id
//!   books.title       -> shelf.name
//!   books.kind        -> shelf.kind
//!   books.path        -> shelf.paths JSON: [0] for zip/folder, full array for images
//!   books.page_count  -> shelf.page_count
//!   books.last_read_at-> shelf.updated_at (unix SECS in this app, not ms)
//!   books.scan_sig    -> shelf.scan_sig (new column, migrated by crate::db_conn)
//!   books.finished    -> shelf.finished (new column)
//!   books.cover_ver   -> shelf.cover_ver (new column)

use crate::error::ReaderError;
use crate::model::*;
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::Arc;

pub type Db = Arc<Mutex<Connection>>;

/// Design DDL run once from setup (AFTER crate::db_conn has created `shelf` and
/// added the scan_sig/finished/cover_ver columns — the design's
/// `ALTER TABLE books ADD COLUMN scan_sig` is handled there, guarded via
/// pragma_table_info like folder_id, so it is NOT repeated here).
/// NOTE: the progress column is `frac`, not `offset` - OFFSET is a reserved word in SQLite.
pub const DDL: &str = "
  CREATE TABLE IF NOT EXISTS pages_meta(
    book_id INTEGER NOT NULL, page INTEGER NOT NULL, entry INTEGER NOT NULL,
    name TEXT NOT NULL, w INTEGER NOT NULL, h INTEGER NOT NULL, bytes INTEGER NOT NULL,
    src_path TEXT,
    PRIMARY KEY(book_id, page)) WITHOUT ROWID;
  CREATE TABLE IF NOT EXISTS progress(
    book_id INTEGER PRIMARY KEY, page INTEGER NOT NULL, frac REAL NOT NULL, updated_at INTEGER NOT NULL);";

pub struct BookRow {
    pub kind: String,
    pub title: String,
    pub scan_sig: Option<String>,
    /// shelf.paths JSON, parsed: [0] for zip/folder, the full array for images.
    pub paths: Vec<String>,
    /// shelf.last_page — open_book resume fallback when no progress row exists.
    pub last_page: i64,
}

pub fn book_row(c: &Connection, id: i64) -> Result<BookRow, ReaderError> {
    let row = c
        .query_row(
            "SELECT kind, name, paths, scan_sig, last_page FROM shelf WHERE id=?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()?
        .ok_or(ReaderError::BookNotOpen)?;
    let paths: Vec<String> = serde_json::from_str(&row.2).unwrap_or_default();
    Ok(BookRow {
        kind: row.0,
        title: row.1,
        scan_sig: row.3,
        paths,
        last_page: row.4,
    })
}

fn ensure_page_src(c: &Connection) {
    let table: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='pages_meta'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if table == 0 {
        return;
    }
    let has: i64 = c
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('pages_meta') WHERE name='src_path'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has == 0 {
        let _ = c.execute("ALTER TABLE pages_meta ADD COLUMN src_path TEXT", []);
    }
}

pub fn load_pages(c: &Connection, id: i64) -> Result<Vec<RealPage>, ReaderError> {
    ensure_page_src(c);
    let mut s = c.prepare(
        "SELECT entry,name,w,h,bytes,src_path FROM pages_meta WHERE book_id=?1 ORDER BY page",
    )?;
    let rows = s.query_map([id], |r| {
        Ok(RealPage {
            entry: r.get(0)?,
            name: r.get(1)?,
            w: r.get(2)?,
            h: r.get(3)?,
            bytes: r.get(4)?,
            src_path: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub fn store_pages(
    c: &mut Connection,
    id: i64,
    pages: &[RealPage],
    sig: &str,
) -> Result<(), ReaderError> {
    ensure_page_src(c);
    let tx = c.transaction()?;
    tx.execute("DELETE FROM pages_meta WHERE book_id=?1", [id])?;
    {
        let mut ins = tx.prepare("INSERT INTO pages_meta(book_id,page,entry,name,w,h,bytes,src_path) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)")?;
        for (i, p) in pages.iter().enumerate() {
            ins.execute(params![
                id,
                i as u32,
                p.entry,
                p.name,
                p.w,
                p.h,
                p.bytes as i64,
                p.src_path
            ])?;
        }
    }
    tx.execute(
        "UPDATE shelf SET scan_sig=?2, page_count=?3 WHERE id=?1",
        params![id, sig, pages.len() as u32],
    )?;
    Ok(tx.commit()?)
}

// ---- page-edit overlay (library.rs commands write it; open_book honours it) ----
//
// `edited_pages` holds the book's CURRENT page list after the user deletes /
// reorders / appends pages in the page directory. It is a full ordered copy of
// RealPage rows (plus src_path for appended pages from other sources), so the
// original files and archives are never modified. A rescan (rescan_book with
// force) or reset_page_edits drops the overlay and the scan takes over again.

pub const EDITED_DDL: &str = "
  CREATE TABLE IF NOT EXISTS edited_pages(
    book_id INTEGER NOT NULL, page INTEGER NOT NULL, entry INTEGER NOT NULL,
    name TEXT NOT NULL, w INTEGER NOT NULL, h INTEGER NOT NULL, bytes INTEGER NOT NULL,
    src_path TEXT,
    PRIMARY KEY(book_id, page)) WITHOUT ROWID;";

/// The edited page list, if the user has edited this book's pages.
pub fn load_edited_pages(c: &Connection, id: i64) -> Result<Option<Vec<RealPage>>, ReaderError> {
    let mut s = c.prepare(
        "SELECT entry,name,w,h,bytes,src_path FROM edited_pages WHERE book_id=?1 ORDER BY page",
    )?;
    let rows = s.query_map([id], |r| {
        Ok(RealPage {
            entry: r.get(0)?,
            name: r.get(1)?,
            w: r.get(2)?,
            h: r.get(3)?,
            bytes: r.get(4)?,
            src_path: r.get(5)?,
        })
    })?;
    let pages: Vec<RealPage> = rows.collect::<Result<_, _>>()?;
    Ok(if pages.is_empty() { None } else { Some(pages) })
}

/// Replace pages_meta with `pages` WITHOUT touching shelf.scan_sig/page_count.
/// Bookmarks and episode_info read page names from pages_meta, so the effective
/// (possibly edited) list must be mirrored there.
pub fn sync_pages_meta(c: &Connection, id: i64, pages: &[RealPage]) -> Result<(), ReaderError> {
    ensure_page_src(c);
    c.execute("DELETE FROM pages_meta WHERE book_id=?1", [id])?;
    let mut ins = c.prepare("INSERT INTO pages_meta(book_id,page,entry,name,w,h,bytes,src_path) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)")?;
    for (i, p) in pages.iter().enumerate() {
        ins.execute(params![id, i as u32, p.entry, p.name, p.w, p.h, p.bytes as i64, p.src_path])?;
    }
    Ok(())
}

/// Persist an edited page list: overlay rows + pages_meta mirror + shelf count.
pub fn store_edited_pages(
    c: &mut Connection,
    id: i64,
    pages: &[RealPage],
) -> Result<(), ReaderError> {
    let tx = c.transaction()?;
    tx.execute("DELETE FROM edited_pages WHERE book_id=?1", [id])?;
    {
        let mut ins = tx.prepare("INSERT INTO edited_pages(book_id,page,entry,name,w,h,bytes,src_path) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)")?;
        for (i, p) in pages.iter().enumerate() {
            ins.execute(params![
                id,
                i as u32,
                p.entry,
                p.name,
                p.w,
                p.h,
                p.bytes as i64,
                p.src_path
            ])?;
        }
    }
    tx.execute("DELETE FROM pages_meta WHERE book_id=?1", [id])?;
    {
        let mut ins = tx.prepare("INSERT INTO pages_meta(book_id,page,entry,name,w,h,bytes) VALUES(?1,?2,?3,?4,?5,?6,?7)")?;
        for (i, p) in pages.iter().enumerate() {
            ins.execute(params![id, i as u32, p.entry, p.name, p.w, p.h, p.bytes as i64])?;
        }
    }
    tx.execute(
        "UPDATE shelf SET page_count=?2 WHERE id=?1",
        params![id, pages.len() as u32],
    )?;
    // Keep any saved position inside the new range.
    let last = pages.len().saturating_sub(1) as u32;
    tx.execute(
        "UPDATE progress SET page=MIN(page, ?2) WHERE book_id=?1",
        params![id, last],
    )?;
    tx.execute(
        "UPDATE shelf SET last_page=MIN(last_page, ?2) WHERE id=?1",
        params![id, last as i64],
    )?;
    Ok(tx.commit()?)
}

pub fn clear_edited_pages(c: &Connection, id: i64) -> Result<(), ReaderError> {
    c.execute("DELETE FROM edited_pages WHERE book_id=?1", [id])?;
    Ok(())
}

pub fn load_progress(c: &Connection, id: i64) -> Result<Option<SavedPosition>, ReaderError> {
    Ok(c.query_row(
        "SELECT page, frac FROM progress WHERE book_id=?1",
        [id],
        |r| {
            Ok(SavedPosition {
                page: r.get(0)?,
                frac: r.get(1)?,
            })
        },
    )
    .optional()?)
}

pub fn save_progress(
    c: &Connection,
    id: i64,
    p: SavedPosition,
    reached_end: bool,
) -> Result<(), ReaderError> {
    // progress.updated_at keeps the design's unix-ms convention (internal to this
    // table); shelf.updated_at uses the app's unix-secs convention (crate::now_secs)
    // so v0.3.x "recently read" ordering keeps working — INTEGRATION_NOTES §3.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    c.execute(
        "INSERT INTO progress(book_id,page,frac,updated_at) VALUES(?1,?2,?3,?4)
                   ON CONFLICT(book_id) DO UPDATE SET page=excluded.page, frac=excluded.frac, updated_at=excluded.updated_at",
        params![id, p.page, p.frac.clamp(0.0, 0.9999), now_ms],
    )?;
    // Mirror into shelf (INTEGRATION_NOTES §3): last_page + updated_at, finished sticky.
    c.execute(
        "UPDATE shelf SET last_page=?2, updated_at=?3, finished=CASE WHEN ?4 THEN 1 ELSE finished END WHERE id=?1",
        params![id, p.page, crate::now_secs(), reached_end],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(name: &str, src_path: Option<&str>) -> RealPage {
        RealPage {
            entry: 0,
            name: name.to_string(),
            w: 800,
            h: 1200,
            bytes: 10,
            src_path: src_path.map(|s| s.to_string()),
        }
    }

    #[test]
    fn edited_pages_round_trip_and_mirror() {
        let mut c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE shelf(id INTEGER PRIMARY KEY, page_count INTEGER, last_page INTEGER);
             CREATE TABLE progress(book_id INTEGER PRIMARY KEY, page INTEGER, frac REAL, updated_at INTEGER);
             CREATE TABLE pages_meta(book_id INTEGER, page INTEGER, entry INTEGER, name TEXT, w INTEGER, h INTEGER, bytes INTEGER);",
        )
        .unwrap();
        c.execute_batch(EDITED_DDL).unwrap();
        c.execute("INSERT INTO shelf VALUES(1, 3, 2)", []).unwrap();
        c.execute("INSERT INTO progress VALUES(1, 2, 0.5, 0)", []).unwrap();

        // delete page 3 + append one foreign page, as delete_pages/append_pages would
        let edited = vec![page("a/01.jpg", None), page("a/02.jpg", None), page("x/09.jpg", Some("/tmp/x.jpg"))];
        store_edited_pages(&mut c, 1, &edited).unwrap();

        let back = load_edited_pages(&c, 1).unwrap().expect("overlay exists");
        assert_eq!(back.len(), 3);
        assert_eq!(back[2].name, "x/09.jpg");
        assert_eq!(back[2].src_path.as_deref(), Some("/tmp/x.jpg"));
        // pages_meta mirrored (bookmarks/episode_info read names from there)
        let n: i64 = c.query_row("SELECT COUNT(*) FROM pages_meta WHERE book_id=1", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 3);
        // shelf count updated, progress still in range
        let pc: i64 = c.query_row("SELECT page_count FROM shelf WHERE id=1", [], |r| r.get(0)).unwrap();
        assert_eq!(pc, 3);

        clear_edited_pages(&c, 1).unwrap();
        assert!(load_edited_pages(&c, 1).unwrap().is_none());
    }
}

pub fn fix_dims(c: &mut Connection, id: i64, fixes: &[DimFix]) -> Result<(), ReaderError> {
    let tx = c.transaction()?;
    for f in fixes {
        tx.execute(
            "UPDATE pages_meta SET w=?3,h=?4 WHERE book_id=?1 AND page=?2",
            params![id, f.page, f.w, f.h],
        )?;
    }
    Ok(tx.commit()?)
}
