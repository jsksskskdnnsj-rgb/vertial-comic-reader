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

pub fn load_pages(c: &Connection, id: i64) -> Result<Vec<RealPage>, ReaderError> {
    let mut s = c.prepare(
        "SELECT entry,name,w,h,bytes FROM pages_meta WHERE book_id=?1 ORDER BY page",
    )?;
    let rows = s.query_map([id], |r| {
        Ok(RealPage {
            entry: r.get(0)?,
            name: r.get(1)?,
            w: r.get(2)?,
            h: r.get(3)?,
            bytes: r.get(4)?,
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
    let tx = c.transaction()?;
    tx.execute("DELETE FROM pages_meta WHERE book_id=?1", [id])?;
    {
        let mut ins = tx.prepare("INSERT INTO pages_meta(book_id,page,entry,name,w,h,bytes) VALUES(?1,?2,?3,?4,?5,?6,?7)")?;
        for (i, p) in pages.iter().enumerate() {
            ins.execute(params![
                id,
                i as u32,
                p.entry,
                p.name,
                p.w,
                p.h,
                p.bytes as i64
            ])?;
        }
    }
    tx.execute(
        "UPDATE shelf SET scan_sig=?2, page_count=?3 WHERE id=?1",
        params![id, sig, pages.len() as u32],
    )?;
    Ok(tx.commit()?)
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
