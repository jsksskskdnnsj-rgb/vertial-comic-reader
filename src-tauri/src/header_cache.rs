//! Persistent page-header cache.
//!
//! A header is the width and height read from the image (or zip entry) without
//! decoding the whole page. The key includes the bytes that change when the
//! file changes, so a stale row cannot be reused:
//!   folder / loose file: length + mtime + path
//!   zip entry: crc32 + uncompressed size + entry name
//!
//! The library database still stores the page list per book (`pages_meta`,
//! invalidated by `scan_sig`). This cache sits under that, so a signature
//! change does not re-read headers of pages whose bytes did not change.
//! Delete the cache directory and the next open rebuilds it.

use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

struct Cache {
    conn: Connection,
    hits: u64,
    misses: u64,
    writes: u64,
}

fn db_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("MH_HEADER_CACHE") {
        let p = PathBuf::from(p);
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).ok()?;
        }
        return Some(p);
    }
    let dir = crate::data_dir().ok()?.join("cache");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("page-headers.db"))
}

fn open_cache() -> Option<Cache> {
    let path = db_path()?;
    let conn = Connection::open(path).ok()?;
    conn.busy_timeout(std::time::Duration::from_millis(3000)).ok()?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
         CREATE TABLE IF NOT EXISTS headers(
            key TEXT PRIMARY KEY,
            w INTEGER NOT NULL,
            h INTEGER NOT NULL,
            bytes INTEGER NOT NULL
         );",
    )
    .ok()?;
    Some(Cache { conn, hits: 0, misses: 0, writes: 0 })
}

fn with_cache<T>(f: impl FnOnce(&mut Cache) -> T) -> Option<T> {
    static SLOT: OnceLock<Mutex<Option<Cache>>> = OnceLock::new();
    let slot = SLOT.get_or_init(|| Mutex::new(open_cache()));
    let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = open_cache();
    }
    let cache = guard.as_mut()?;
    Some(f(cache))
}

/// Identity of a loose file. None when the path cannot be stat'ed.
pub(crate) fn file_key(path: &Path) -> Option<String> {
    let meta = path.metadata().ok()?;
    let modified = meta.modified().ok()?;
    let ns = modified.duration_since(UNIX_EPOCH).ok()?.as_nanos();
    Some(format!("f:{}:{}:{}", meta.len(), ns, path.to_string_lossy()))
}

pub(crate) fn get(key: &str) -> Option<(u32, u32, u64)> {
    with_cache(|cache| {
        let row = cache.conn.query_row(
            "SELECT w, h, bytes FROM headers WHERE key=?1",
            [key],
            |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?, r.get::<_, i64>(2)? as u64)),
        );
        match row {
            Ok(v) => {
                cache.hits += 1;
                Some(v)
            }
            Err(_) => {
                cache.misses += 1;
                None
            }
        }
    })
    .flatten()
}

pub(crate) fn put(key: &str, w: u32, h: u32, bytes: u64) {
    let _ = with_cache(|cache| {
        let result = cache.conn.execute(
            "INSERT INTO headers(key, w, h, bytes) VALUES(?1,?2,?3,?4)
             ON CONFLICT(key) DO UPDATE SET w=excluded.w, h=excluded.h, bytes=excluded.bytes",
            params![key, w, h, bytes as i64],
        );
        if result.is_ok() {
            cache.writes += 1;
            if cache.writes % 256 == 0 {
                let n: i64 = cache
                    .conn
                    .query_row("SELECT COUNT(*) FROM headers", [], |r| r.get(0))
                    .unwrap_or(0);
                if n > 100_000 {
                    let _ = cache.conn.execute(
                        "DELETE FROM headers WHERE rowid IN (SELECT rowid FROM headers ORDER BY rowid LIMIT 2000)",
                        [],
                    );
                }
            }
        }
    });
}

#[cfg(test)]
pub(crate) fn stats() -> (u64, u64) {
    with_cache(|cache| (cache.hits, cache.misses)).unwrap_or((0, 0))
}
