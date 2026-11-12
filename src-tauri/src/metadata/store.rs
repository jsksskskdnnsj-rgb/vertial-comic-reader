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
CREATE TABLE IF NOT EXISTS metadata_http_cache(
  cache_key TEXT PRIMARY KEY,
  body TEXT NOT NULL,
  fetched_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);
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
        crate::appdata::add_column(
            conn,
            "ALTER TABLE book_metadata ADD COLUMN source_page_url TEXT",
        )?;
    }
    for (name, decl) in [
        ("status", "TEXT"),
        ("chapter_count", "INTEGER"),
        ("volume_count", "INTEGER"),
        ("year", "INTEGER"),
        ("aliases_json", "TEXT"),
        ("locked", "INTEGER NOT NULL DEFAULT 0"),
        ("locks_json", "TEXT"),
    ] {
        let present: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('book_metadata') WHERE name=?1",
                [name],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if present == 0 {
            crate::appdata::add_column(
                conn,
                &format!("ALTER TABLE book_metadata ADD COLUMN {name} {decl}"),
            )?;
        }
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
    pub status: Option<String>,
    pub chapter_count: Option<i64>,
    pub volume_count: Option<i64>,
    pub year: Option<i32>,
    pub aliases: Vec<String>,
    pub locked: bool,
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
                    cover_url, cover_cache, match_kind, bound_at, source_page_url,
                    status, chapter_count, volume_count, year, aliases_json, locked
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
                    status: r.get(17)?,
                    chapter_count: r.get(18)?,
                    volume_count: r.get(19)?,
                    year: r.get(20)?,
                    aliases: decode_json_list(r.get(21)?),
                    locked: r.get::<_, i64>(22).unwrap_or(0) != 0,
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
    let aliases = serde_json::to_string(&detail.aliases).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO book_metadata(
            book_id, provider, external_id, title_display, title_cn, title_original, summary,
            authors_json, tags_json, rating, rating_count, release_date, cover_url, cover_cache,
            match_kind, bound_at, source_page_url, status, chapter_count, volume_count, year,
            aliases_json, locked, locks_json)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,0,NULL)
         ON CONFLICT(book_id) DO UPDATE SET
            provider=excluded.provider, external_id=excluded.external_id,
            title_display=excluded.title_display, title_cn=excluded.title_cn,
            title_original=excluded.title_original, summary=excluded.summary,
            authors_json=excluded.authors_json, tags_json=excluded.tags_json,
            rating=excluded.rating, rating_count=excluded.rating_count,
            release_date=excluded.release_date, cover_url=excluded.cover_url,
            cover_cache=excluded.cover_cache, match_kind=excluded.match_kind,
            bound_at=excluded.bound_at, source_page_url=excluded.source_page_url,
            status=excluded.status, chapter_count=excluded.chapter_count,
            volume_count=excluded.volume_count, year=excluded.year,
            aliases_json=excluded.aliases_json, locked=0, locks_json=NULL",
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
            detail.facts.status,
            detail.facts.chapter_count,
            detail.facts.volume_count,
            detail.facts.year,
            aliases,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualPatch {
    pub title_display: Option<String>,
    pub title_cn: Option<String>,
    pub title_original: Option<String>,
    pub summary: Option<String>,
    pub authors: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    pub status: Option<String>,
    pub chapter_count: Option<i64>,
    pub volume_count: Option<i64>,
    pub year: Option<i32>,
    pub release_date: Option<String>,
    #[serde(default)]
    pub lock: bool,
}

pub fn save_manual(book_id: i64, patch: &ManualPatch) -> Result<(), String> {
    let _ = book_title(book_id)?;
    let authors = serde_json::to_string(patch.authors.as_deref().unwrap_or(&[])).map_err(|e| e.to_string())?;
    let tags = serde_json::to_string(patch.tags.as_deref().unwrap_or(&[])).map_err(|e| e.to_string())?;
    let locks = if patch.lock {
        serde_json::to_string(&[
            "titleDisplay",
            "titleCn",
            "titleOriginal",
            "summary",
            "authors",
            "tags",
            "status",
            "chapterCount",
            "volumeCount",
            "year",
            "releaseDate",
        ])
        .ok()
    } else {
        None
    };
    let conn = db_conn()?;
    let existing = get_bound(book_id)?;
    let provider = existing
        .as_ref()
        .map(|b| b.provider.as_str().to_string())
        .unwrap_or_else(|| "manual".into());
    let external = existing
        .as_ref()
        .map(|b| b.external_id.clone())
        .unwrap_or_else(|| "local".into());
    let match_kind = if existing.is_some() { "manual" } else { "manual" };
    conn.execute(
        "INSERT INTO book_metadata(
            book_id, provider, external_id, title_display, title_cn, title_original, summary,
            authors_json, tags_json, rating, rating_count, release_date, cover_url, cover_cache,
            match_kind, bound_at, source_page_url, status, chapter_count, volume_count, year,
            aliases_json, locked, locks_json)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,NULL,NULL,?10,NULL,NULL,?11,?12,NULL,?13,?14,?15,?16,'[]',?17,?18)
         ON CONFLICT(book_id) DO UPDATE SET
            title_display=excluded.title_display, title_cn=excluded.title_cn,
            title_original=excluded.title_original, summary=excluded.summary,
            authors_json=excluded.authors_json, tags_json=excluded.tags_json,
            release_date=excluded.release_date, status=excluded.status,
            chapter_count=excluded.chapter_count, volume_count=excluded.volume_count,
            year=excluded.year, match_kind='manual', locked=excluded.locked, locks_json=excluded.locks_json",
        params![
            book_id,
            provider,
            external,
            patch.title_display,
            patch.title_cn,
            patch.title_original,
            patch.summary,
            authors,
            tags,
            patch.release_date,
            match_kind,
            now_secs(),
            patch.status,
            patch.chapter_count,
            patch.volume_count,
            patch.year,
            if patch.lock { 1 } else { 0 },
            locks,
        ],
    )
    .map_err(|e| e.to_string())?;
    clear_pending(book_id)?;
    Ok(())
}

pub fn cache_fresh(key: &str) -> Option<String> {
    let conn = db_conn().ok()?;
    let now = now_secs();
    conn.query_row(
        "SELECT body FROM metadata_http_cache WHERE cache_key=?1 AND expires_at>=?2",
        params![key, now],
        |r| r.get(0),
    )
    .ok()
}

pub fn cache_stale(key: &str) -> Option<String> {
    let conn = db_conn().ok()?;
    let oldest = now_secs() - 30 * 24 * 3600;
    conn.query_row(
        "SELECT body FROM metadata_http_cache WHERE cache_key=?1 AND fetched_at>=?2",
        params![key, oldest],
        |r| r.get(0),
    )
    .ok()
}

pub fn cache_put(key: &str, body: &str, ttl_secs: i64) {
    if body.len() > 400_000 {
        return;
    }
    let Ok(conn) = db_conn() else { return };
    let now = now_secs();
    let _ = conn.execute(
        "INSERT INTO metadata_http_cache(cache_key, body, fetched_at, expires_at)
         VALUES (?1,?2,?3,?4)
         ON CONFLICT(cache_key) DO UPDATE SET body=excluded.body, fetched_at=excluded.fetched_at, expires_at=excluded.expires_at",
        params![key, body, now, now + ttl_secs],
    );
    let _ = conn.execute(
        "DELETE FROM metadata_http_cache WHERE expires_at < ?1 AND cache_key NOT IN (
            SELECT cache_key FROM metadata_http_cache ORDER BY fetched_at DESC LIMIT 400
        )",
        params![now - 30 * 24 * 3600],
    );
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaShelfRow {
    pub book_id: i64,
    pub name: String,
    pub kind: String,
    pub page_count: i64,
    pub favorite: bool,
    pub updated_at: i64,
    pub provider: Option<String>,
    pub external_id: Option<String>,
    pub match_kind: Option<String>,
    pub locked: bool,
    pub title_display: Option<String>,
    pub bound_at: Option<i64>,
}

pub fn list_shelf_meta() -> Result<Vec<MetaShelfRow>, String> {
    let conn = db_conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT s.id, s.name, s.kind, s.page_count, s.favorite, s.updated_at,
                    m.provider, m.external_id, m.match_kind, m.locked, m.title_display, m.bound_at
             FROM shelf s
             LEFT JOIN book_metadata m ON m.book_id = s.id
             ORDER BY s.updated_at DESC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            let locked: Option<i64> = r.get(9)?;
            Ok(MetaShelfRow {
                book_id: r.get(0)?,
                name: r.get(1)?,
                kind: r.get(2)?,
                page_count: r.get(3)?,
                favorite: r.get::<_, i64>(4)? != 0,
                updated_at: r.get(5)?,
                provider: r.get(6)?,
                external_id: r.get(7)?,
                match_kind: r.get(8)?,
                locked: locked.unwrap_or(0) != 0,
                title_display: r.get(10)?,
                bound_at: r.get(11)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

#[derive(Debug, Clone)]
pub struct CacheStamp {
    pub fetched_at: i64,
    pub expires_at: i64,
}

pub fn cache_stamp(key: &str) -> Option<CacheStamp> {
    let conn = db_conn().ok()?;
    conn.query_row(
        "SELECT fetched_at, expires_at FROM metadata_http_cache WHERE cache_key=?1",
        [key],
        |r| {
            Ok(CacheStamp {
                fetched_at: r.get(0)?,
                expires_at: r.get(1)?,
            })
        },
    )
    .ok()
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
