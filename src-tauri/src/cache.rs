//! On-disk page cache + LRU index.
//!
//! The index records `(book_id, idx, variant, ext, bytes)`, never a source path.
//! Deletes only remove a regular file whose name this process derived under the
//! cache directory. Symlinks, other directories, and files that are not in the
//! index are left in place.

use crate::error::ReaderError;
use crate::model::*;
use parking_lot::Mutex;
use rusqlite::{params, Connection};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering::Relaxed},
};

pub const DEFAULT_CACHE_LIMIT: u64 = 512 << 20;
pub const MIN_CACHE_LIMIT: u64 = 64 << 20;
pub const MAX_CACHE_LIMIT: u64 = 64 << 30;

pub fn clamp_limit(bytes: u64) -> u64 {
    bytes.clamp(MIN_CACHE_LIMIT, MAX_CACHE_LIMIT)
}

pub fn limit_path(data_dir: &Path) -> PathBuf {
    data_dir.join("cache-limit.json")
}

/// Read the saved page-cache budget. A missing or unreadable file is the default
/// 512 MiB, so an older data directory still opens.
pub fn load_limit(data_dir: &Path) -> u64 {
    let Ok(text) = std::fs::read_to_string(limit_path(data_dir)) else {
        return DEFAULT_CACHE_LIMIT;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return DEFAULT_CACHE_LIMIT;
    };
    match v.get("bytes").and_then(|n| n.as_u64()) {
        Some(n) => clamp_limit(n),
        None => DEFAULT_CACHE_LIMIT,
    }
}

pub fn store_limit(data_dir: &Path, bytes: u64) -> Result<(), String> {
    let bytes = clamp_limit(bytes);
    std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let path = limit_path(data_dir);
    write_regular_file(&path, format!("{{\"bytes\":{bytes}}}\n").as_bytes())
}

/// Copy the limit file when the data directory moves. Skip symlinks and never
/// overwrite a file already at the destination.
pub(crate) fn migrate_limit_file(old_dir: &Path, new_dir: &Path) -> Result<(), String> {
    let src = limit_path(old_dir);
    let dst = limit_path(new_dir);
    let Ok(meta) = std::fs::symlink_metadata(&src) else {
        return Ok(());
    };
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Ok(());
    }
    if std::fs::symlink_metadata(&dst).is_ok() {
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::copy(&src, &dst).map_err(|e| format!("迁移缓存上限失败：{e}"))?;
    Ok(())
}

fn write_regular_file(path: &Path, body: &[u8]) -> Result<(), String> {
    if std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err("refusing to write through a symlink".into());
    }
    let tmp = path.with_extension("json.part");
    if std::fs::symlink_metadata(&tmp)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err("refusing to write through a symlink".into());
    }
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn cache_ext(ext: &str) -> Option<&'static str> {
    match ext {
        "jpg" => Some("jpg"),
        "png" => Some("png"),
        "webp" => Some("webp"),
        "gif" => Some("gif"),
        "avif" => Some("avif"),
        "bmp" => Some("bmp"),
        "bin" => Some("bin"),
        _ => None,
    }
}

fn is_real_dir(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(m) => m.is_dir() && !m.file_type().is_symlink(),
        Err(_) => false,
    }
}

fn has_parent_dir(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
}

/// `cache_dir/<book>/<idx>.<variant>.<ext>` with no `..` and no symlink directory.
/// Missing book directory is allowed (the file is then absent). A symlinked book
/// directory is not owned.
pub(crate) fn owned_cache_path(
    cache_dir: &Path,
    book: i64,
    idx: u32,
    variant: u16,
    ext: &str,
) -> Option<PathBuf> {
    let ext = cache_ext(ext)?;
    if has_parent_dir(cache_dir) || !is_real_dir(cache_dir) {
        return None;
    }
    let book_name = book.to_string();
    if book_name.is_empty()
        || book_name.contains('/')
        || book_name.contains('\\')
        || book_name.contains("..")
    {
        return None;
    }
    let name = format!("{:05}.{variant}.{ext}", idx);
    let book_dir = cache_dir.join(&book_name);
    match std::fs::symlink_metadata(&book_dir) {
        Ok(m) if m.file_type().is_symlink() || !m.is_dir() => return None,
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return None,
    }
    let path = book_dir.join(&name);
    if has_parent_dir(&path) || !path.starts_with(cache_dir) {
        return None;
    }
    Some(path)
}

enum Delete {
    Gone,
    Failed,
}

fn delete_owned_file(path: &Path) -> Delete {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => Delete::Gone, // leave the link
        Ok(m) if m.is_file() => {
            if std::fs::remove_file(path).is_ok() {
                Delete::Gone
            } else {
                Delete::Failed
            }
        }
        Ok(_) => Delete::Gone, // not a regular file; do not remove it
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Delete::Gone,
        Err(_) => Delete::Failed,
    }
}

#[derive(Clone, Debug)]
pub struct CachedPage {
    pub path: PathBuf,
    pub mime: &'static str,
    pub bytes: u64,
}

pub fn ext_static(e: &str) -> &'static str {
    cache_ext(e).unwrap_or("bin")
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
            let mut s =
                db.prepare("SELECT book_id,idx,variant,ext,bytes,last_access FROM cache_entries")?;
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
        // Files that are not in this index are not ours to delete, including leftovers
        // from older builds. They stay until something can prove they are cache entries.
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
        let ext = cache_ext(ext).unwrap_or("bin");
        self.dir
            .join(k.0.to_string())
            .join(format!("{:05}.{}.{}", k.1, k.2, ext))
    }
    pub fn contains(&self, k: &Key) -> bool {
        self.inner.lock().map.contains_key(k)
    }
    pub fn has_room(&self) -> bool {
        let limit = self.limit.load(Relaxed);
        self.inner.lock().total < limit
    }
    pub fn byte_limit(&self) -> u64 {
        self.limit.load(Relaxed)
    }
    /// True when `path` is lexically inside this cache directory. Used so a
    /// failed read of an original is not treated as a cache entry to delete.
    pub fn hosts(&self, path: &Path) -> bool {
        !has_parent_dir(path) && path.starts_with(&self.dir)
    }
    pub(crate) fn stored_ext(&self, k: &Key) -> Option<&'static str> {
        self.inner.lock().map.get(k).map(|e| e.ext)
    }
    pub(crate) fn owned_path(&self, k: &Key, ext: &str) -> Option<PathBuf> {
        owned_cache_path(&self.dir, k.0, k.1, k.2, ext)
    }

    /// Where a new cache object may be written. Refuses symlink directories and
    /// symlink destinations so a cache entry cannot overwrite a file outside
    /// this directory.
    pub fn prepare_owned_write(&self, k: &Key, ext: &str) -> Result<PathBuf, ReaderError> {
        let path = owned_cache_path(&self.dir, k.0, k.1, k.2, ext).ok_or_else(|| {
            ReaderError::Io("refusing to write a cache file outside the cache directory".into())
        })?;
        if let Some(dir) = path.parent() {
            match std::fs::symlink_metadata(dir) {
                Ok(m) if m.is_dir() && !m.file_type().is_symlink() => {}
                Ok(_) => {
                    return Err(ReaderError::Io(
                        "cache book directory is not a real directory".into(),
                    ))
                }
                Err(_) => match std::fs::create_dir(dir) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && is_real_dir(dir) => {
                    }
                    Err(e) => return Err(e.into()),
                },
            }
        }
        for candidate in [&path, &path.with_extension("part")] {
            if std::fs::symlink_metadata(candidate)
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
            {
                return Err(ReaderError::Io(
                    "refusing to write through a cache symlink".into(),
                ));
            }
        }
        Ok(path)
    }

    pub fn get(&self, k: &Key) -> Option<CachedPage> {
        let ext = self.inner.lock().map.get(k).map(|e| e.ext)?;
        let path = self.path_for(k, ext);
        match std::fs::symlink_metadata(&path) {
            Ok(m) if m.is_file() && !m.file_type().is_symlink() => {}
            Ok(m) if m.file_type().is_symlink() => {
                self.forget(k);
                return None;
            }
            Ok(_) => return None,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.forget(k);
                return None;
            }
            Err(_) => return None,
        }
        let mut g = self.inner.lock();
        g.tick += 1;
        let t = g.tick;
        let e = g.map.get_mut(k)?;
        e.last = t;
        e.dirty = true;
        Some(CachedPage {
            path,
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

    /// Drop the index row. `delete` removes the owned regular file; a symlink or
    /// a path we cannot prove is left on disk.
    fn take_entry(&self, g: &mut Inner, k: &Key, delete: bool) -> bool {
        let Some(entry) = g.map.get(k) else {
            return true;
        };
        if delete {
            let Some(path) = owned_cache_path(&self.dir, k.0, k.1, k.2, entry.ext) else {
                // Not a path this cache is allowed to remove. Forget the row so a
                // later eviction does not keep aiming at it.
                self.drop_row(g, k);
                return true;
            };
            match delete_owned_file(&path) {
                Delete::Failed => return false,
                Delete::Gone => {}
            }
        }
        self.drop_row(g, k);
        true
    }
    fn drop_row(&self, g: &mut Inner, k: &Key) {
        if let Some(e) = g.map.remove(k) {
            g.total = g.total.saturating_sub(e.bytes);
            let _ = g.db.execute(
                "DELETE FROM cache_entries WHERE book_id=?1 AND idx=?2 AND variant=?3",
                params![k.0, k.1, k.2],
            );
        }
    }
    fn remove_locked(&self, g: &mut Inner, k: &Key) -> bool {
        self.take_entry(g, k, true)
    }
    pub fn remove(&self, k: &Key) {
        let mut g = self.inner.lock();
        let _ = self.remove_locked(&mut g, k);
    }
    /// Drop an index row without touching the file. Used when the indexed path
    /// is the reader's original (same file), which must not be deleted.
    pub fn forget(&self, k: &Key) {
        let mut g = self.inner.lock();
        self.drop_row(&mut g, k);
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
            if !self.remove_locked(&mut g, &k) {
                continue;
            }
        }
    }

    pub fn purge_book(&self, book: i64) -> u64 {
        let mut g = self.inner.lock();
        let keys: Vec<Key> = g.map.keys().filter(|k| k.0 == book).copied().collect();
        let before = g.total;
        for k in keys {
            let _ = self.remove_locked(&mut g, &k);
        }
        let freed = before - g.total;
        drop(g);
        // Only an empty real directory we created. Unknown files inside it stay,
        // and a symlinked directory is not removed.
        if let Some(dir) = owned_cache_path(&self.dir, book, 0, 0, "bin")
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        {
            if is_real_dir(&dir) {
                if let Ok(mut rd) = std::fs::read_dir(&dir) {
                    if rd.next().is_none() {
                        let _ = std::fs::remove_dir(&dir);
                    }
                }
            }
        }
        freed
    }
    pub fn clear(&self, pinned: &dyn Fn(&Key) -> bool) -> u64 {
        let mut g = self.inner.lock();
        let keys: Vec<Key> = g.map.keys().filter(|k| !pinned(k)).copied().collect();
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch(label: &str) -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("mh-cache-owned-{label}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn put(cache: &PageCache, key: Key, bytes: u64) -> PathBuf {
        let path = cache.path_for(&key, "jpg");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, vec![7u8; bytes as usize]).unwrap();
        cache.insert(key, "jpg", bytes);
        path
    }

    #[test]
    fn limit_round_trips_across_a_new_cache() {
        let dir = scratch("limit");
        assert_eq!(load_limit(&dir), DEFAULT_CACHE_LIMIT);
        store_limit(&dir, 80 << 20).unwrap();
        let saved = load_limit(&dir);
        assert_eq!(saved, 80 << 20);
        let first = PageCache::open(dir.join("cache"), saved).unwrap();
        assert_eq!(first.byte_limit(), 80 << 20);
        drop(first);
        let again = load_limit(&dir);
        assert_eq!(again, 80 << 20);
        let second = PageCache::open(dir.join("cache"), again).unwrap();
        assert_eq!(second.stats().limit_bytes, 80 << 20);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreadable_limit_file_stays_on_the_default() {
        let dir = scratch("bad-limit");
        std::fs::write(dir.join("cache-limit.json"), b"not json").unwrap();
        assert_eq!(load_limit(&dir), DEFAULT_CACHE_LIMIT);
        std::fs::write(dir.join("cache-limit.json"), b"{\"nope\":1}").unwrap();
        assert_eq!(load_limit(&dir), DEFAULT_CACHE_LIMIT);
        std::fs::write(dir.join("cache-limit.json"), b"{\"bytes\":10}").unwrap();
        assert_eq!(load_limit(&dir), MIN_CACHE_LIMIT);
        std::fs::write(dir.join("cache-limit.json"), b"{\"bytes\":999999999999999}").unwrap();
        assert_eq!(load_limit(&dir), MAX_CACHE_LIMIT);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn store_limit_refuses_a_symlink() {
        let dir = scratch("limit-link");
        let secret = dir.join("secret.json");
        std::fs::write(&secret, b"keep-secret").unwrap();
        std::os::unix::fs::symlink(&secret, dir.join("cache-limit.json")).unwrap();
        let err = store_limit(&dir, 80 << 20).unwrap_err();
        assert!(err.contains("symlink"), "{err}");
        assert_eq!(std::fs::read(&secret).unwrap(), b"keep-secret");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_limit_copies_only_a_regular_file() {
        let old = scratch("mig-old");
        let new = scratch("mig-new");
        store_limit(&old, 96 << 20).unwrap();
        migrate_limit_file(&old, &new).unwrap();
        assert_eq!(load_limit(&new), 96 << 20);
        assert_eq!(load_limit(&old), 96 << 20);
        std::fs::write(new.join("cache-limit.json"), b"{\"bytes\":134217728}").unwrap();
        migrate_limit_file(&old, &new).unwrap();
        assert_eq!(
            std::fs::read(new.join("cache-limit.json")).unwrap(),
            b"{\"bytes\":134217728}"
        );
        #[cfg(unix)]
        {
            let linked = scratch("mig-link");
            let target = linked.join("real.json");
            std::fs::write(&target, b"{\"bytes\":1}").unwrap();
            std::os::unix::fs::symlink(&target, linked.join("cache-limit.json")).unwrap();
            let dest = scratch("mig-dest");
            migrate_limit_file(&linked, &dest).unwrap();
            assert!(!dest.join("cache-limit.json").exists());
            assert_eq!(std::fs::read(&target).unwrap(), b"{\"bytes\":1}");
            let _ = std::fs::remove_dir_all(&linked);
            let _ = std::fs::remove_dir_all(&dest);
        }
        let _ = std::fs::remove_dir_all(&old);
        let _ = std::fs::remove_dir_all(&new);
    }

    #[test]
    fn eviction_keeps_the_pinned_file_and_startup_trims_only_indexed_bytes() {
        let dir = scratch("evict");
        let cache_dir = dir.join("cache");
        let cache = PageCache::open(cache_dir.clone(), 5_000).unwrap();
        let pinned = put(&cache, (7, 0, 0), 800);
        let old = put(&cache, (7, 1, 0), 800);
        let _newer = put(&cache, (7, 2, 0), 800);
        let orphan = cache_dir.join("7").join("notes.txt");
        std::fs::write(&orphan, b"do-not-touch").unwrap();
        let outside = dir.join("comics");
        std::fs::create_dir_all(&outside).unwrap();
        let original = outside.join("page.jpg");
        std::fs::write(&original, b"original-page").unwrap();
        let expanded = dir.join("expanded").join("book");
        std::fs::create_dir_all(&expanded).unwrap();
        let unpacked = expanded.join("p.jpg");
        std::fs::write(&unpacked, b"unpacked-archive").unwrap();
        cache.set_limit(1_000);
        cache.evict_if_needed(&|k| k.1 == 0);
        assert!(pinned.is_file());
        assert_eq!(std::fs::read(&pinned).unwrap(), vec![7u8; 800]);
        assert!(!old.is_file(), "unpinned cache file should be trimmed");
        assert!(cache.stats().bytes <= 900);
        assert_eq!(std::fs::read(&orphan).unwrap(), b"do-not-touch");
        assert_eq!(std::fs::read(&original).unwrap(), b"original-page");
        assert_eq!(std::fs::read(&unpacked).unwrap(), b"unpacked-archive");
        drop(cache);

        let restarted = PageCache::open(cache_dir.clone(), 500).unwrap();
        assert!(restarted.stats().bytes > restarted.byte_limit());
        restarted.evict_if_needed(&|_| false);
        assert!(restarted.stats().bytes <= restarted.byte_limit());
        assert_eq!(std::fs::read(&orphan).unwrap(), b"do-not-touch");
        assert_eq!(std::fs::read(&original).unwrap(), b"original-page");
        assert_eq!(std::fs::read(&unpacked).unwrap(), b"unpacked-archive");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn under_limit_restart_does_not_delete_indexed_files() {
        let dir = scratch("under");
        let cache_dir = dir.join("cache");
        let cache = PageCache::open(cache_dir.clone(), 50_000).unwrap();
        let file = put(&cache, (3, 0, 0), 100);
        drop(cache);
        let restarted = PageCache::open(cache_dir, 50_000).unwrap();
        restarted.evict_if_needed(&|_| false);
        assert!(file.is_file());
        assert_eq!(restarted.stats().entries, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_and_purge_do_not_touch_unowned_files() {
        let dir = scratch("clear");
        let cache = PageCache::open(dir.join("cache"), 50_000).unwrap();
        let owned = put(&cache, (4, 0, 0), 20);
        let pinned = put(&cache, (4, 1, 0), 20);
        // 00002 is not indexed. A name that looks like a cache file is not enough.
        let lookalike = dir.join("cache").join("4").join("00002.0.jpg");
        std::fs::write(&lookalike, b"looks-like-cache").unwrap();
        let note = dir.join("cache").join("4").join("notes.txt");
        std::fs::write(&note, b"note").unwrap();
        let stray = dir.join("cache").join("stray.jpg");
        std::fs::write(&stray, b"stray").unwrap();
        let original = dir.join("library").join("page.jpg");
        std::fs::create_dir_all(original.parent().unwrap()).unwrap();
        std::fs::write(&original, b"library-original").unwrap();
        let legacy = dir.join("page_cache").join("old.jpg");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, b"legacy").unwrap();
        cache.clear(&|k| k.1 == 1);
        assert!(!owned.is_file());
        assert_eq!(std::fs::read(&pinned).unwrap(), vec![7u8; 20]);
        assert_eq!(std::fs::read(&lookalike).unwrap(), b"looks-like-cache");
        assert_eq!(std::fs::read(&note).unwrap(), b"note");
        assert_eq!(std::fs::read(&stray).unwrap(), b"stray");
        assert_eq!(std::fs::read(&original).unwrap(), b"library-original");
        assert_eq!(std::fs::read(&legacy).unwrap(), b"legacy");
        cache.purge_book(4);
        assert_eq!(std::fs::read(&lookalike).unwrap(), b"looks-like-cache");
        assert_eq!(std::fs::read(&note).unwrap(), b"note");
        assert_eq!(std::fs::read(&original).unwrap(), b"library-original");
        assert_eq!(std::fs::read(&legacy).unwrap(), b"legacy");
        assert!(!cache.path_for(&(4, 1, 0), "jpg").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn symlink_cache_entries_are_not_followed() {
        let dir = scratch("symlink");
        let cache = PageCache::open(dir.join("cache"), 50_000).unwrap();
        let original = dir.join("comics").join("real.jpg");
        std::fs::create_dir_all(original.parent().unwrap()).unwrap();
        std::fs::write(&original, b"real-bytes").unwrap();
        let link = cache.path_for(&(9, 0, 0), "jpg");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&original, &link).unwrap();
        cache.insert((9, 0, 0), "jpg", 10);
        cache.remove(&(9, 0, 0));
        assert_eq!(std::fs::read(&original).unwrap(), b"real-bytes");
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert!(!cache.contains(&(9, 0, 0)));

        let comics = dir.join("elsewhere");
        std::fs::create_dir_all(&comics).unwrap();
        let planted = comics.join("00000.0.jpg");
        std::fs::write(&planted, b"planted").unwrap();
        std::fs::write(comics.join("page.jpg"), b"page").unwrap();
        std::os::unix::fs::symlink(&comics, dir.join("cache").join("11")).unwrap();
        cache.insert((11, 0, 0), "jpg", 7);
        cache.remove(&(11, 0, 0));
        assert_eq!(std::fs::read(&planted).unwrap(), b"planted");
        assert_eq!(std::fs::read(comics.join("page.jpg")).unwrap(), b"page");

        let secret = dir.join("secret.jpg");
        std::fs::write(&secret, b"secret").unwrap();
        let via = cache.path_for(&(12, 0, 0), "jpg");
        std::fs::create_dir_all(via.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&secret, &via).unwrap();
        assert!(cache.prepare_owned_write(&(12, 0, 0), "jpg").is_err());
        assert_eq!(std::fs::read(&secret).unwrap(), b"secret");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn index_has_no_source_path_and_rejects_escaped_names() {
        let dir = scratch("schema");
        let cache = PageCache::open(dir.join("cache"), 50_000).unwrap();
        let marker = dir.join("MH_ORIGINAL_PATH_MARKER.jpg");
        std::fs::write(&marker, b"original").unwrap();
        put(&cache, (1, 0, 0), 4);
        let conn = rusqlite::Connection::open(dir.join("cache").join("cache.db")).unwrap();
        let mut stmt = conn.prepare("PRAGMA table_info(cache_entries)").unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(
            cols,
            vec!["book_id", "idx", "variant", "ext", "bytes", "last_access"]
        );
        drop(stmt);
        drop(conn);
        drop(cache);
        let blob = std::fs::read(dir.join("cache").join("cache.db")).unwrap();
        let text = String::from_utf8_lossy(&blob);
        assert!(!text.contains("MH_ORIGINAL_PATH_MARKER"));
        assert!(owned_cache_path(&dir.join("cache"), 1, 0, 0, "../x").is_none());
        assert!(owned_cache_path(&dir.join("cache"), 1, 0, 0, "jpg/../../x").is_none());
        assert_eq!(std::fs::read(&marker).unwrap(), b"original");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
