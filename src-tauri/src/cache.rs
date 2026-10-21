//! On-disk page cache + LRU index
//! (split from design/claude-v04/reader_backend.rs `pub mod cache`, unchanged).

use crate::error::ReaderError;
use crate::model::*;
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering::Relaxed},
};

#[derive(Clone, Debug)]
pub struct CachedPage {
    pub path: PathBuf,
    pub mime: &'static str,
    pub bytes: u64,
}

pub fn ext_static(e: &str) -> &'static str {
    match e {
        "jpg" => "jpg",
        "png" => "png",
        "webp" => "webp",
        "gif" => "gif",
        "avif" => "avif",
        _ => "bin",
    }
}
pub fn ext_mime(e: &str) -> &'static str {
    match e {
        "jpg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        _ => "application/octet-stream",
    }
}

struct Entry {
    ext: &'static str,
    bytes: u64,
    last: u64,
    dirty: bool,
}
struct Inner {
    map: HashMap<Key, Entry>,
    total: u64,
    tick: u64,
    db: Connection,
}

/// Index lives in cache.db (separate from the library DB: no lock contention with progress
/// saves, and "delete the cache" can never touch the library). It is loaded into RAM at
/// startup; inserts/deletes write through, LRU "last access" ticks are batched.
pub struct PageCache {
    dir: PathBuf,
    inner: Mutex<Inner>,
    limit: AtomicU64,
}

impl PageCache {
    pub fn open(dir: PathBuf, limit: u64) -> Result<Self, ReaderError> {
        std::fs::create_dir_all(&dir)?;
        let db = Connection::open(dir.join("cache.db"))?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
                CREATE TABLE IF NOT EXISTS cache_entries(book_id INTEGER, idx INTEGER, variant INTEGER,
                  ext TEXT NOT NULL, bytes INTEGER NOT NULL, last_access INTEGER NOT NULL,
                  PRIMARY KEY(book_id, idx, variant));",
        )?;
        let (mut map, mut total, mut tick) = (HashMap::new(), 0u64, 0u64);
        {
            let mut s = db.prepare(
                "SELECT book_id,idx,variant,ext,bytes,last_access FROM cache_entries",
            )?;
            let rows = s.query_map([], |r| {
                Ok((
                    (
                        r.get::<_, i64>(0)?,
                        r.get::<_, u32>(1)?,
                        r.get::<_, u16>(2)?,
                    ),
                    r.get::<_, String>(3)?,
                    r.get::<_, u64>(4)?,
                    r.get::<_, u64>(5)?,
                ))
            })?;
            for row in rows {
                let (k, ext, bytes, last) = row?;
                total += bytes;
                tick = tick.max(last + 1);
                map.insert(
                    k,
                    Entry {
                        ext: ext_static(&ext),
                        bytes,
                        last,
                        dirty: false,
                    },
                );
            }
        }
        // (do an orphan-file GC pass on a background thread here: walk dir, delete files with no row)
        Ok(Self {
            dir,
            inner: Mutex::new(Inner {
                map,
                total,
                tick,
                db,
            }),
            limit: AtomicU64::new(limit),
        })
    }

    pub fn path_for(&self, k: &Key, ext: &str) -> PathBuf {
        self.dir
            .join(k.0.to_string())
            .join(format!("{:05}.{}.{}", k.1, k.2, ext))
    }
    pub fn contains(&self, k: &Key) -> bool {
        self.inner.lock().map.contains_key(k)
    }

    pub fn get(&self, k: &Key) -> Option<CachedPage> {
        let mut g = self.inner.lock();
        g.tick += 1;
        let t = g.tick;
        let e = g.map.get_mut(k)?;
        e.last = t;
        e.dirty = true;
        Some(CachedPage {
            path: self.path_for(k, e.ext),
            mime: ext_mime(e.ext),
            bytes: e.bytes,
        })
    }

    pub fn insert(&self, k: Key, ext: &'static str, bytes: u64) -> CachedPage {
        let mut g = self.inner.lock();
        g.tick += 1;
        let t = g.tick;
        if let Some(old) = g.map.insert(
            k,
            Entry {
                ext,
                bytes,
                last: t,
                dirty: false,
            },
        ) {
            g.total -= old.bytes;
        }
        g.total += bytes;
        let _ = g.db.execute(
            "INSERT OR REPLACE INTO cache_entries VALUES(?1,?2,?3,?4,?5,?6)",
            params![k.0, k.1, k.2, ext, bytes as i64, t as i64],
        );
        CachedPage {
            path: self.path_for(&k, ext),
            mime: ext_mime(ext),
            bytes,
        }
    }

    fn remove_locked(&self, g: &mut Inner, k: &Key) {
        if let Some(e) = g.map.remove(k) {
            g.total = g.total.saturating_sub(e.bytes);
            let _ = std::fs::remove_file(self.path_for(k, e.ext));
            let _ = g.db.execute("DELETE FROM cache_entries WHERE book_id=?1 AND idx=?2 AND variant=?3", params![k.0, k.1, k.2]);
        }
    }
    pub fn remove(&self, k: &Key) {
        let mut g = self.inner.lock();
        self.remove_locked(&mut g, k);
    }

    /// Called by workers after each insert. Cheap when under the limit.
    /// Evicts least-recently-used entries NOT in the pinned window until total <= 90% of limit
    /// (hysteresis: avoids evicting on every single insert once full).
    pub fn evict_if_needed(&self, pinned: &dyn Fn(&Key) -> bool) {
        let limit = self.limit.load(Relaxed);
        let mut g = self.inner.lock();
        if g.total <= limit {
            return;
        }
        let target = limit / 10 * 9;
        let mut v: Vec<(u64, Key)> = g
            .map
            .iter()
            .filter(|(k, _)| !pinned(k))
            .map(|(k, e)| (e.last, *k))
            .collect();
        v.sort_unstable(); // oldest first
        for (_, k) in v {
            if g.total <= target {
                break;
            }
            self.remove_locked(&mut g, &k);
        }
    }

    pub fn purge_book(&self, book: i64) -> u64 {
        let mut g = self.inner.lock();
        let keys: Vec<Key> = g.map.keys().filter(|k| k.0 == book).copied().collect();
        let before = g.total;
        for k in keys {
            self.remove_locked(&mut g, &k);
        }
        let _ = std::fs::remove_dir_all(self.dir.join(book.to_string()));
        before - g.total
    }
    pub fn clear(&self, pinned: &dyn Fn(&Key) -> bool) -> u64 {
        let mut g = self.inner.lock();
        let keys: Vec<Key> = g
            .map
            .keys()
            .filter(|k| !pinned(k))
            .copied()
            .collect();
        let before = g.total;
        for k in keys {
            self.remove_locked(&mut g, &k);
        }
        before - g.total
    }

    /// Persist batched LRU ticks (call every ~30 s from a worker and on exit).
    pub fn flush_touches(&self) {
        let mut g = self.inner.lock();
        let dirty: Vec<(Key, u64)> = g
            .map
            .iter()
            .filter(|(_, e)| e.dirty)
            .map(|(k, e)| (*k, e.last))
            .collect();
        if dirty.is_empty() {
            return;
        }
        if let Ok(tx) = g.db.transaction() {
            for (k, t) in &dirty {
                let _ = tx.execute("UPDATE cache_entries SET last_access=?4 WHERE book_id=?1 AND idx=?2 AND variant=?3", params![k.0, k.1, k.2, *t as i64]);
            }
            let _ = tx.commit();
        }
        for (k, _) in dirty {
            if let Some(e) = g.map.get_mut(&k) {
                e.dirty = false;
            }
        }
    }

    pub fn set_limit(&self, bytes: u64) {
        self.limit.store(bytes, Relaxed);
    }
    pub fn stats(&self) -> CacheStats {
        let g = self.inner.lock();
        CacheStats {
            bytes: g.total,
            limit_bytes: self.limit.load(Relaxed),
            entries: g.map.len() as u64,
        }
    }
}
