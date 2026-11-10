//! SQLite persistence for book ↔ external metadata bindings.

use crate::appdata::{db_conn, now_secs};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{MetadataDetail, Provider};

pub const DDL: &str = "
CREATE TABLE IF NOT EXISTS book_metadata(
  book_id INTEGER PRIMARY KEY,
  provider TEXT NOT NULL,
  external_id TEXT NOT NULL,
  title_display TEXT,
  title_cn TEXT,
  title_original TEXT,
  summary TEXT,
  authors_json TEXT,
  tags_json TEXT,
  rating REAL,
  rating_count INTEGER,
  release_date TEXT,
  cover_url TEXT,
  cover_cache TEXT,
  match_kind TEXT NOT NULL DEFAULT 'manual',
  bound_at INTEGER NOT NULL,
  FOREIGN KEY(book_id) REFERENCES shelf(id) ON DELETE CASCADE
);
CREATE TABLE IF NOT EXISTS metadata_pending(
  book_id INTEGER PRIMARY KEY,
  provider TEXT NOT NULL,
  external_id TEXT NOT NULL,
  match_kind TEXT NOT NULL,
  query TEXT NOT NULL,
  candidates_json TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  FOREIGN KEY(book_id) REFERENCES shelf(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_book_metadata_provider ON book_metadata(provider);
";

pub fn migrate(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(DDL).map_err(|e| e.to_string())?;
    let has: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('book_metadata') WHERE name='source_page_url'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has == 0 {
        conn.execute(
            "ALTER TABLE book_metadata ADD COLUMN source_page_url TEXT",
            [],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundMetadata {
    pub book_id: i64,
    pub provider: Provider,
    pub external_id: String,
    pub title_display: Option<String>,
    pub title_cn: Option<String>,
    pub title_original: Option<String>,
    pub summary: Option<String>,
    pub authors: Vec<String>,
    pub tags: Vec<String>,
    pub rating: Option<f64>,
    pub rating_count: Option<i64>,
    pub release_date: Option<String>,
    pub cover_url: Option<String>,
    pub cover_cache: Option<String>,
    pub match_kind: String,
    pub bound_at: i64,
    pub source_page_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingMatch {
    pub book_id: i64,
    pub provider: Provider,
    pub external_id: String,
    pub match_kind: String,
    pub query: String,
    pub candidates_json: String,
    pub created_at: i64,
}

fn decode_json_list(raw: Option<String>) -> Vec<String> {
    raw.and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn get_bound(book_id: i64) -> Result<Option<BoundMetadata>, String> {
    let conn = db_conn()?;
    let row = conn
        .query_row(
            "SELECT book_id, provider, external_id, title_display, title_cn, title_original,
                    summary, authors_json, tags_json, rating, rating_count, release_date,
                    cover_url, cover_cache, match_kind, bound_at, source_page_url
             FROM book_metadata WHERE book_id=?1",
            [book_id],
            |r| {
                Ok(BoundMetadata {
                    book_id: r.get(0)?,
                    provider: Provider::parse(&r.get::<_, String>(1)?).unwrap_or(Provider::Bangumi),
                    external_id: r.get(2)?,
                    title_display: r.get(3)?,
                    title_cn: r.get(4)?,
                    title_original: r.get(5)?,
                    summary: r.get(6)?,
                    authors: decode_json_list(r.get(7)?),
                    tags: decode_json_list(r.get(8)?),
                    rating: r.get(9)?,
                    rating_count: r.get(10)?,
                    release_date: r.get(11)?,
                    cover_url: r.get(12)?,
                    cover_cache: r.get(13)?,
                    match_kind: r.get(14)?,
                    bound_at: r.get(15)?,
                    source_page_url: r.get(16)?,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(row)
}

pub fn write_bound(
    book_id: i64,
    detail: &MetadataDetail,
    match_kind: &str,
    cover_cache: Option<&str>,
    source_page_url: Option<&str>,
) -> Result<(), String> {
    let conn = db_conn()?;
    let authors = serde_json::to_string(&detail.authors).map_err(|e| e.to_string())?;
    let tags = serde_json::to_string(&detail.tags).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO book_metadata(
            book_id, provider, external_id, title_display, title_cn, title_original, summary,
            authors_json, tags_json, rating, rating_count, release_date, cover_url, cover_cache,
            match_kind, bound_at, source_page_url)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)
         ON CONFLICT(book_id) DO UPDATE SET
            provider=excluded.provider, external_id=excluded.external_id,
            title_display=excluded.title_display, title_cn=excluded.title_cn,
            title_original=excluded.title_original, summary=excluded.summary,
            authors_json=excluded.authors_json, tags_json=excluded.tags_json,
            rating=excluded.rating, rating_count=excluded.rating_count,
            release_date=excluded.release_date, cover_url=excluded.cover_url,
            cover_cache=excluded.cover_cache, match_kind=excluded.match_kind,
            bound_at=excluded.bound_at, source_page_url=excluded.source_page_url",
        params![
            book_id,
            detail.provider.as_str(),
            detail.external_id,
            detail.title,
            detail.title_cn,
            detail.title_original,
            detail.summary,
            authors,
            tags,
            detail.rating,
            detail.rating_count,
            detail.release_date,
            detail.cover_url,
            cover_cache,
            match_kind,
            now_secs(),
            source_page_url,
        ],
    )
    .map_err(|e| e.to_string())?;
    clear_pending(book_id)?;
    Ok(())
}

pub fn unbind(book_id: i64) -> Result<(), String> {
    let conn = db_conn()?;
    conn.execute("DELETE FROM book_metadata WHERE book_id=?1", [book_id])
        .map_err(|e| e.to_string())?;
    clear_pending(book_id)?;
    Ok(())
}

pub fn clear_pending(book_id: i64) -> Result<(), String> {
    let conn = db_conn()?;
    conn.execute("DELETE FROM metadata_pending WHERE book_id=?1", [book_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn write_pending(row: &PendingMatch) -> Result<(), String> {
    let conn = db_conn()?;
    conn.execute(
        "INSERT INTO metadata_pending(book_id, provider, external_id, match_kind, query, candidates_json, created_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7)
         ON CONFLICT(book_id) DO UPDATE SET
            provider=excluded.provider, external_id=excluded.external_id,
            match_kind=excluded.match_kind, query=excluded.query,
            candidates_json=excluded.candidates_json, created_at=excluded.created_at",
        params![
            row.book_id,
            row.provider.as_str(),
            row.external_id,
            row.match_kind,
            row.query,
            row.candidates_json,
            row.created_at,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn list_pending() -> Result<Vec<PendingMatch>, String> {
    let conn = db_conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT book_id, provider, external_id, match_kind, query, candidates_json, created_at
             FROM metadata_pending ORDER BY created_at DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(PendingMatch {
                book_id: r.get(0)?,
                provider: Provider::parse(&r.get::<_, String>(1)?).unwrap_or(Provider::Bangumi),
                external_id: r.get(2)?,
                match_kind: r.get(3)?,
                query: r.get(4)?,
                candidates_json: r.get(5)?,
                created_at: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn list_unbound_book_ids() -> Result<Vec<i64>, String> {
    let conn = db_conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT id FROM shelf s
             WHERE NOT EXISTS (SELECT 1 FROM book_metadata m WHERE m.book_id=s.id)",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

pub fn book_title(book_id: i64) -> Result<String, String> {
    let conn = db_conn()?;
    conn.query_row("SELECT name FROM shelf WHERE id=?1", [book_id], |r| r.get(0))
        .map_err(|_| "书本不存在".into())
}

pub fn set_cover_cache(book_id: i64, cache_name: &str) -> Result<(), String> {
    let conn = db_conn()?;
    conn.execute(
        "UPDATE book_metadata SET cover_cache=?2 WHERE book_id=?1",
        params![book_id, cache_name],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
