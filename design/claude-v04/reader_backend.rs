// ============================================================================
// reader_backend.rs - MH Reader reader backend (Tauri v2).   *** NOT COMPILED ***
// Written against: tauri 2, zip 2, image 0.25, imagesize 0.13, rusqlite 0.31, parking_lot 0.12.
// Crate APIs were written from memory: expect a few signature fixes on first `cargo check`.
// Split each `pub mod` into its own file when you integrate.
//
// Cargo.toml deps:
//   tauri = "2"   serde = { version = "1", features = ["derive"] }   serde_json = "1"
//   thiserror = "1"   parking_lot = "0.12"   rusqlite = { version = "0.31", features = ["bundled"] }
//   zip = "2"   imagesize = "0.13"   natord = "1"   walkdir = "2"   http = "1"
//   image = { version = "0.25", default-features = false, features = ["jpeg","png","webp","gif","bmp"] }
// ============================================================================

// ==== mod error =============================================================
pub mod error {
    use serde::Serialize;

    /// Serialises as {"code":"corrupt","detail":"..."} - JS gets a real object from invoke() rejections.
    #[derive(Debug, Clone, thiserror::Error, Serialize)]
    #[serde(tag = "code", content = "detail", rename_all = "snake_case")]
    pub enum ReaderError {
        #[error("book is not open")]           BookNotOpen,
        #[error("source file missing: {0}")]   SourceMissing(String),
        #[error("page out of range")]          PageOutOfRange,
        #[error("corrupt or unreadable: {0}")] Corrupt(String),
        #[error("unsupported: {0}")]           Unsupported(String),
        #[error("bad request: {0}")]           BadRequest(String),
        #[error("cancelled")]                  Cancelled,
        #[error("io: {0}")]                    Io(String),
        #[error("db: {0}")]                    Db(String),
    }
    impl ReaderError {
        pub fn http_status(&self) -> u16 {
            use ReaderError::*;
            match self {
                BookNotOpen | PageOutOfRange => 404,
                SourceMissing(_) => 410,
                BadRequest(_) => 400,
                Corrupt(_) | Unsupported(_) => 422,
                Cancelled => 503,           // JS retries 503 (see ImageWindow.onError)
                Io(_) | Db(_) => 500,
            }
        }
        pub fn code(&self) -> String {
            serde_json::to_value(self).ok().and_then(|v| v["code"].as_str().map(String::from)).unwrap_or_default()
        }
    }
    impl From<std::io::Error> for ReaderError {
        fn from(e: std::io::Error) -> Self {
            if e.kind() == std::io::ErrorKind::NotFound { Self::SourceMissing(e.to_string()) } else { Self::Io(e.to_string()) }
        }
    }
    impl From<rusqlite::Error> for ReaderError { fn from(e: rusqlite::Error) -> Self { Self::Db(e.to_string()) } }
    impl From<zip::result::ZipError> for ReaderError {
        fn from(e: zip::result::ZipError) -> Self {
            use zip::result::ZipError as Z;
            match e { Z::Io(io) => io.into(), Z::UnsupportedArchive(m) => Self::Unsupported(m.to_string()), o => Self::Corrupt(o.to_string()) }
        }
    }
    impl From<image::ImageError> for ReaderError { fn from(e: image::ImageError) -> Self { Self::Corrupt(e.to_string()) } }
}

// ==== mod model =============================================================
pub mod model {
    use serde::{Deserialize, Serialize};
    use std::path::PathBuf;

    /// (book_id, virtual_page_idx, variant). variant 0 = full size, N = max width N px (160 = thumbnail).
    pub type Key = (i64, u32, u16);
    pub const VARIANTS: [u16; 6] = [0, 160, 800, 1200, 1600, 2400]; // whitelist: bounds cache variants

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum SourceKind { Zip, Folder, Images }
    impl SourceKind {
        pub fn from_db(s: &str) -> Option<Self> {
            match s { "zip" => Some(Self::Zip), "folder" => Some(Self::Folder), "images" => Some(Self::Images), _ => None }
        }
    }

    /// One REAL page as stored in SQLite (dims straight from the file header).
    #[derive(Clone, Debug)]
    pub struct RealPage { pub entry: u32, pub name: String, pub w: u32, pub h: u32, pub bytes: u64 }

    /// One VIRTUAL page = what JS gets a box for. Tall images are cut into several of these.
    #[derive(Clone, Debug)]
    pub struct PageMeta {
        pub src_page: u32,   // real page number
        pub entry: u32,      // zip entry index (Zip); unused otherwise
        pub name: String,    // zip entry name | path relative to folder | absolute path (Images)
        pub w: u32, pub h: u32,   // dims of THIS box (h = slice height); 0,0 = unknown
        pub full_h: u32,     // height of the whole underlying image
        pub y0: u32,         // slice offset inside the image
    }
    impl PageMeta { pub fn is_slice(&self) -> bool { self.h != 0 && self.h < self.full_h } }

    pub struct BookMeta {
        pub id: i64, pub kind: SourceKind, pub path: PathBuf, pub sig: String,
        pub pages: Vec<PageMeta>,
        pub groups: Vec<(u32, u32)>,   // real page -> (first, last) virtual idx
    }

    #[derive(Serialize, Deserialize, Clone, Copy, Debug)]
    pub struct SavedPosition { pub page: u32, pub frac: f64 }

    /// Raw folder-derived chapter start (internal; produced by scan::chapters). Clients get `Episode`.
    #[derive(Serialize)] pub struct Chapter { pub page: u32, pub title: String }

    /// How pages are grouped into episodes. A NAVIGATION/DISPLAY layer only: it never touches virtual
    /// pages, slicing, cache keys or the extraction queue, so it cannot break the seamless strip.
    #[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
    pub enum ChapterMode {
        #[serde(rename = "none")]   Off,       // continuous: no chapter UI at all
        #[serde(rename = "folder")] Folder,    // episodes = parent-folder changes (computed from names)
        #[serde(rename = "pages")]  Pages,     // N pages = 1 episode (computed from page count)
    }
    impl ChapterMode {
        pub fn as_str(&self) -> &'static str { match self { Self::Off => "none", Self::Folder => "folder", Self::Pages => "pages" } }
        pub fn parse(s: &str) -> Option<Self> { match s { "none" => Some(Self::Off), "folder" => Some(Self::Folder), "pages" => Some(Self::Pages), _ => None } }
    }
    /// index is 1-based; page is the 0-based first REAL page; title is a folder path (folder mode) or "" (pages mode).
    #[derive(Serialize, Clone, Debug, PartialEq)]
    pub struct Episode { pub index: u32, pub page: u32, pub len: u32, pub title: String }
    #[derive(Serialize, Deserialize, Clone, Copy, Debug)]
    pub struct ChapterSettings { pub mode: ChapterMode, pub pages: u32 }
    #[derive(Deserialize)] #[serde(rename_all = "camelCase")]
    pub struct EpisodeQuery { pub book_id: i64, pub page: u32 }
    #[derive(Serialize)] pub struct EpisodeInfo { pub index: u32, pub total: u32 }

    #[derive(Serialize)] #[serde(rename_all = "camelCase")]
    pub struct OpenedBook {
        pub book_id: i64, pub title: String, pub sig: String,
        pub real_pages: u32,
        pub dims: Vec<[u32; 2]>,   // per VIRTUAL page [w, h]   (0,0 = unknown)
        pub src: Vec<u32>,         // per VIRTUAL page: real page index
        pub chapters: Vec<Episode>,          // EFFECTIVE list for this book's mode (empty = show no chapter UI)
        pub folder_chapters: Vec<Episode>,   // folder-derived list whatever the mode, so JS can switch modes with no round trip
        pub chapter_mode: ChapterMode,       // effective mode for this book (per-book override, else global default)
        pub chapter_pages: u32,              // effective N for "pages" mode
        pub chapter_overridden: bool,        // true = this book has its own row in book_settings ("remember for this book" is on)
        pub resume: Option<SavedPosition>,
    }
    #[derive(Deserialize)] pub struct DimFix { pub page: u32, pub w: u32, pub h: u32 }
    #[derive(Serialize)] #[serde(rename_all = "camelCase")]
    pub struct CacheStats { pub bytes: u64, pub limit_bytes: u64, pub entries: u64 }
}

// ==== mod scan: read page list + dimensions from HEADERS ONLY ================
pub mod scan {
    use super::{error::ReaderError, model::*};
    use std::{fs::File, io::{BufReader, Cursor, Read}, path::Path};

    pub const HEAD_SMALL: usize = 64 * 1024;          // tier 1
    pub const HEAD_LARGE: usize = 1024 * 1024;        // tier 2: JPEG with fat EXIF/ICC/thumbnail
    pub const MAX_ENTRY: u64 = 256 * 1024 * 1024;     // refuse zip bombs
    pub const SLICE_TRIGGER: u32 = 16_000;            // taller than this -> slice
    pub const SLICE_MAX: u32 = 8_000;                 // each slice at most this tall
    pub const MAX_PIXELS: u64 = 150_000_000;          // refuse to decode beyond this

    pub fn is_image(name: &str) -> bool {
        let l = name.to_ascii_lowercase();
        let base = l.rsplit('/').next().unwrap_or(&l);
        if base.starts_with('.') || l.starts_with("__macosx/") { return false; }   // ._foo, .DS_Store
        matches!(base.rsplit('.').next(), Some("jpg" | "jpeg" | "png" | "webp" | "gif" | "avif" | "bmp"))
    }

    fn head_dims(buf: &[u8]) -> Option<(u32, u32)> {
        imagesize::blob_size(buf).ok().map(|s| (s.width as u32, s.height as u32))
    }

    fn read_up_to(ar: &mut zip::ZipArchive<BufReader<File>>, entry: u32, limit: u64) -> Option<(Vec<u8>, u64)> {
        let mut e = ar.by_index(entry as usize).ok()?;      // seeks to local header, decompresses lazily
        let size = e.size();
        let mut buf = Vec::with_capacity((limit.min(size)) as usize);
        e.by_ref().take(limit).read_to_end(&mut buf).ok()?;
        Some((buf, size))
    }

    pub fn open_zip(path: &Path) -> Result<zip::ZipArchive<BufReader<File>>, ReaderError> {
        Ok(zip::ZipArchive::new(BufReader::with_capacity(64 * 1024, File::open(path)?))?) // central directory only
    }

    pub fn scan_zip(path: &Path) -> Result<Vec<RealPage>, ReaderError> {
        let mut ar = open_zip(path)?;
        // file_names() comes from the central directory: no per-entry I/O. enumerate() == entry index.
        let mut cands: Vec<(u32, String)> = ar.file_names().enumerate()
            .filter(|(_, n)| !n.ends_with('/') && is_image(n))
            .map(|(i, n)| (i as u32, n.replace('\\', "/"))).collect();
        cands.sort_by(|a, b| natord::compare_ignore_case(&a.1, &b.1));   // 2.jpg before 10.jpg

        let mut out = Vec::with_capacity(cands.len());
        for (entry, name) in cands {
            let mut dims = None; let mut bytes = 0;
            for limit in [HEAD_SMALL as u64, HEAD_LARGE as u64] {
                let Some((buf, size)) = read_up_to(&mut ar, entry, limit) else { break };
                bytes = size;
                if let Some(d) = head_dims(&buf) { dims = Some(d); break; }
                if (buf.len() as u64) < limit { break; }       // whole entry read, header still unparsable
            }
            if dims.is_none() {                                  // tier 3: decoder reads dims (AVIF, odd files)
                if let Some((buf, _)) = read_up_to(&mut ar, entry, MAX_ENTRY) {
                    dims = image::ImageReader::new(Cursor::new(buf)).with_guessed_format().ok()
                        .and_then(|r| r.into_dimensions().ok());
                }
            }
            let (w, h) = dims.unwrap_or((0, 0));                 // (0,0) = unknown; JS corrects on load
            out.push(RealPage { entry, name, w, h, bytes });
        }
        Ok(out)
    }

    pub fn scan_folder(root: &Path) -> Vec<RealPage> {
        let mut files: Vec<(String, u64)> = walkdir::WalkDir::new(root).follow_links(false).into_iter()
            .filter_map(Result::ok).filter(|e| e.file_type().is_file())
            .filter_map(|e| {
                let rel = e.path().strip_prefix(root).ok()?.to_string_lossy().replace('\\', "/");
                if is_image(&rel) { Some((rel, e.metadata().ok()?.len())) } else { None }
            }).collect();
        files.sort_by(|a, b| natord::compare_ignore_case(&a.0, &b.0));
        files.into_iter().map(|(name, bytes)| {
            let (w, h) = imagesize::size(root.join(&name)).map(|s| (s.width as u32, s.height as u32)).unwrap_or((0, 0));
            RealPage { entry: 0, name, w, h, bytes }
        }).collect()
    }

    /// Loose images: `paths` come from the DB (user's drop order), name = absolute path.
    pub fn scan_images(paths: &[String]) -> Vec<RealPage> {
        paths.iter().map(|p| {
            let (w, h) = imagesize::size(p).map(|s| (s.width as u32, s.height as u32)).unwrap_or((0, 0));
            RealPage { entry: 0, name: p.clone(), w, h, bytes: std::fs::metadata(p).map(|m| m.len()).unwrap_or(0) }
        }).collect()
    }

    /// n equal slices, each <= SLICE_MAX, for images taller than SLICE_TRIGGER. Returns (y0, h).
    pub fn plan_slices(h: u32) -> Vec<(u32, u32)> {
        if h <= SLICE_TRIGGER { return vec![(0, h)]; }
        let n = h.div_ceil(SLICE_MAX);
        let sh = h.div_ceil(n);
        (0..n).map(|k| { let y0 = k * sh; (y0, sh.min(h - y0)) }).collect()
    }

    /// Real pages -> virtual pages. PURE: re-run on every open, nothing derived is stored,
    /// so fixing a wrong dimension later automatically changes the slicing.
    pub fn build_pages(real: &[RealPage]) -> Vec<PageMeta> {
        let mut out = Vec::new();
        for (g, r) in real.iter().enumerate() {
            let sliceable = r.h > SLICE_TRIGGER && (r.w as u64 * r.h as u64) <= MAX_PIXELS;
            let slices = if sliceable { plan_slices(r.h) } else { vec![(0, r.h)] };
            for (y0, h) in slices {
                out.push(PageMeta { src_page: g as u32, entry: r.entry, name: r.name.clone(), w: r.w, h, full_h: r.h, y0 });
            }
        }
        out
    }

    pub fn groups(pages: &[PageMeta]) -> Vec<(u32, u32)> {
        let mut g: Vec<(u32, u32)> = Vec::new();
        for (i, p) in pages.iter().enumerate() {
            if p.src_page as usize == g.len() { g.push((i as u32, i as u32)); } else { g[p.src_page as usize].1 = i as u32; }
        }
        g
    }

    /// Chapters = change of parent directory between consecutive real pages.
    pub fn chapters(real: &[RealPage]) -> Vec<Chapter> {
        let mut out: Vec<Chapter> = Vec::new();
        for (g, r) in real.iter().enumerate() {
            let dir = r.name.rsplit_once('/').map(|x| x.0.to_string()).unwrap_or_default();
            if !dir.is_empty() && out.last().map(|c| c.title != dir).unwrap_or(true) {
                out.push(Chapter { page: g as u32, title: dir });
            }
        }
        if out.len() < 2 { out.clear(); }       // a single folder is not a chapter list
        out
    }

    /// Cheap change detector. Zip: size:mtime. Folder: count:total_len:max_mtime (one metadata walk).
    pub fn quick_sig(kind: SourceKind, path: &Path) -> Result<String, ReaderError> {
        let ms = |m: &std::fs::Metadata| m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis()).unwrap_or(0);
        match kind {
            SourceKind::Zip => { let m = std::fs::metadata(path)?; Ok(format!("z{}:{}", m.len(), ms(&m))) }
            SourceKind::Folder => {
                let (mut n, mut len, mut mx) = (0u64, 0u64, 0u128);
                for e in walkdir::WalkDir::new(path).into_iter().filter_map(Result::ok).filter(|e| e.file_type().is_file()) {
                    if let Ok(m) = e.metadata() { n += 1; len += m.len(); mx = mx.max(ms(&m)); }
                }
                Ok(format!("f{n}:{len}:{mx}"))
            }
            SourceKind::Images => Ok("i".into()),   // loose-image books: caller hashes the path list if needed
        }
    }
}

// ==== mod db: tables used by the reader ======================================
pub mod db {
    use super::{error::ReaderError, model::*};
    use parking_lot::Mutex;
    use rusqlite::{params, Connection, OptionalExtension};
    use std::sync::Arc;

    pub type Db = Arc<Mutex<Connection>>;

    /// Run once from your migration step (bump meta.schema_version).
    /// NOTE: the progress column is `frac`, not `offset` - OFFSET is a reserved word in SQLite.
    pub const DDL: &str = "
      ALTER TABLE books ADD COLUMN scan_sig TEXT;
      CREATE TABLE IF NOT EXISTS pages_meta(
        book_id INTEGER NOT NULL, page INTEGER NOT NULL, entry INTEGER NOT NULL,
        name TEXT NOT NULL, w INTEGER NOT NULL, h INTEGER NOT NULL, bytes INTEGER NOT NULL,
        PRIMARY KEY(book_id, page)) WITHOUT ROWID;
      CREATE TABLE IF NOT EXISTS progress(
        book_id INTEGER PRIMARY KEY, page INTEGER NOT NULL, frac REAL NOT NULL, updated_at INTEGER NOT NULL);";

    pub struct BookRow { pub path: String, pub kind: String, pub title: String, pub scan_sig: Option<String> }

    pub fn book_row(c: &Connection, id: i64) -> Result<BookRow, ReaderError> {
        c.query_row("SELECT path, kind, title, scan_sig FROM books WHERE id=?1", [id],
            |r| Ok(BookRow { path: r.get(0)?, kind: r.get(1)?, title: r.get(2)?, scan_sig: r.get(3)? }))
            .optional()?.ok_or(ReaderError::BookNotOpen)
    }
    pub fn load_pages(c: &Connection, id: i64) -> Result<Vec<RealPage>, ReaderError> {
        let mut s = c.prepare("SELECT entry,name,w,h,bytes FROM pages_meta WHERE book_id=?1 ORDER BY page")?;
        let rows = s.query_map([id], |r| Ok(RealPage { entry: r.get(0)?, name: r.get(1)?, w: r.get(2)?, h: r.get(3)?, bytes: r.get(4)? }))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
    pub fn store_pages(c: &mut Connection, id: i64, pages: &[RealPage], sig: &str) -> Result<(), ReaderError> {
        let tx = c.transaction()?;
        tx.execute("DELETE FROM pages_meta WHERE book_id=?1", [id])?;
        {
            let mut ins = tx.prepare("INSERT INTO pages_meta(book_id,page,entry,name,w,h,bytes) VALUES(?1,?2,?3,?4,?5,?6,?7)")?;
            for (i, p) in pages.iter().enumerate() { ins.execute(params![id, i as u32, p.entry, p.name, p.w, p.h, p.bytes as i64])?; }
        }
        tx.execute("UPDATE books SET scan_sig=?2, page_count=?3 WHERE id=?1", params![id, sig, pages.len() as u32])?;
        Ok(tx.commit()?)
    }
    pub fn load_progress(c: &Connection, id: i64) -> Result<Option<SavedPosition>, ReaderError> {
        Ok(c.query_row("SELECT page, frac FROM progress WHERE book_id=?1", [id], |r| Ok(SavedPosition { page: r.get(0)?, frac: r.get(1)? })).optional()?)
    }
    pub fn save_progress(c: &Connection, id: i64, p: SavedPosition, reached_end: bool) -> Result<(), ReaderError> {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
        c.execute("INSERT INTO progress(book_id,page,frac,updated_at) VALUES(?1,?2,?3,?4)
                   ON CONFLICT(book_id) DO UPDATE SET page=excluded.page, frac=excluded.frac, updated_at=excluded.updated_at",
                  params![id, p.page, p.frac.clamp(0.0, 0.9999), now])?;
        c.execute("UPDATE books SET last_read_at=?2, finished=CASE WHEN ?3 THEN 1 ELSE finished END WHERE id=?1",
                  params![id, now, reached_end])?;
        Ok(())
    }
    pub fn fix_dims(c: &mut Connection, id: i64, fixes: &[DimFix]) -> Result<(), ReaderError> {
        let tx = c.transaction()?;
        for f in fixes { tx.execute("UPDATE pages_meta SET w=?3,h=?4 WHERE book_id=?1 AND page=?2", params![id, f.page, f.w, f.h])?; }
        Ok(tx.commit()?)
    }
}

// ==== mod cache: on-disk page cache + LRU index ==============================
pub mod cache {
    use super::{error::ReaderError, model::*};
    use parking_lot::Mutex;
    use rusqlite::{params, Connection};
    use std::{collections::HashMap, path::PathBuf, sync::atomic::{AtomicU64, Ordering::Relaxed}};

    #[derive(Clone, Debug)]
    pub struct CachedPage { pub path: PathBuf, pub mime: &'static str, pub bytes: u64 }

    pub fn ext_static(e: &str) -> &'static str {
        match e { "jpg" => "jpg", "png" => "png", "webp" => "webp", "gif" => "gif", "avif" => "avif", _ => "bin" }
    }
    pub fn ext_mime(e: &str) -> &'static str {
        match e { "jpg" => "image/jpeg", "png" => "image/png", "webp" => "image/webp", "gif" => "image/gif", "avif" => "image/avif", "bmp" => "image/bmp", _ => "application/octet-stream" }
    }

    struct Entry { ext: &'static str, bytes: u64, last: u64, dirty: bool }
    struct Inner { map: HashMap<Key, Entry>, total: u64, tick: u64, db: Connection }

    /// Index lives in cache.db (separate from the library DB: no lock contention with progress
    /// saves, and "delete the cache" can never touch the library). It is loaded into RAM at
    /// startup; inserts/deletes write through, LRU "last access" ticks are batched.
    pub struct PageCache { dir: PathBuf, inner: Mutex<Inner>, limit: AtomicU64 }

    impl PageCache {
        pub fn open(dir: PathBuf, limit: u64) -> Result<Self, ReaderError> {
            std::fs::create_dir_all(&dir)?;
            let db = Connection::open(dir.join("cache.db"))?;
            db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
                CREATE TABLE IF NOT EXISTS cache_entries(book_id INTEGER, idx INTEGER, variant INTEGER,
                  ext TEXT NOT NULL, bytes INTEGER NOT NULL, last_access INTEGER NOT NULL,
                  PRIMARY KEY(book_id, idx, variant));")?;
            let (mut map, mut total, mut tick) = (HashMap::new(), 0u64, 0u64);
            {
                let mut s = db.prepare("SELECT book_id,idx,variant,ext,bytes,last_access FROM cache_entries")?;
                let rows = s.query_map([], |r| Ok(((r.get::<_, i64>(0)?, r.get::<_, u32>(1)?, r.get::<_, u16>(2)?),
                    r.get::<_, String>(3)?, r.get::<_, u64>(4)?, r.get::<_, u64>(5)?)))?;
                for row in rows { let (k, ext, bytes, last) = row?; total += bytes; tick = tick.max(last + 1);
                    map.insert(k, Entry { ext: ext_static(&ext), bytes, last, dirty: false }); }
            }
            // (do an orphan-file GC pass on a background thread here: walk dir, delete files with no row)
            Ok(Self { dir, inner: Mutex::new(Inner { map, total, tick, db }), limit: AtomicU64::new(limit) })
        }

        pub fn path_for(&self, k: &Key, ext: &str) -> PathBuf {
            self.dir.join(k.0.to_string()).join(format!("{:05}.{}.{}", k.1, k.2, ext))
        }
        pub fn contains(&self, k: &Key) -> bool { self.inner.lock().map.contains_key(k) }

        pub fn get(&self, k: &Key) -> Option<CachedPage> {
            let mut g = self.inner.lock();
            g.tick += 1; let t = g.tick;
            let e = g.map.get_mut(k)?;
            e.last = t; e.dirty = true;
            Some(CachedPage { path: self.path_for(k, e.ext), mime: ext_mime(e.ext), bytes: e.bytes })
        }

        pub fn insert(&self, k: Key, ext: &'static str, bytes: u64) -> CachedPage {
            let mut g = self.inner.lock();
            g.tick += 1; let t = g.tick;
            if let Some(old) = g.map.insert(k, Entry { ext, bytes, last: t, dirty: false }) { g.total -= old.bytes; }
            g.total += bytes;
            let _ = g.db.execute("INSERT OR REPLACE INTO cache_entries VALUES(?1,?2,?3,?4,?5,?6)", params![k.0, k.1, k.2, ext, bytes as i64, t as i64]);
            CachedPage { path: self.path_for(&k, ext), mime: ext_mime(ext), bytes }
        }

        fn remove_locked(&self, g: &mut Inner, k: &Key) {
            if let Some(e) = g.map.remove(k) {
                g.total = g.total.saturating_sub(e.bytes);
                let _ = std::fs::remove_file(self.path_for(k, e.ext));
                let _ = g.db.execute("DELETE FROM cache_entries WHERE book_id=?1 AND idx=?2 AND variant=?3", params![k.0, k.1, k.2]);
            }
        }
        pub fn remove(&self, k: &Key) { let mut g = self.inner.lock(); self.remove_locked(&mut g, k); }

        /// Called by workers after each insert. Cheap when under the limit.
        /// Evicts least-recently-used entries NOT in the pinned window until total <= 90% of limit
        /// (hysteresis: avoids evicting on every single insert once full).
        pub fn evict_if_needed(&self, pinned: &dyn Fn(&Key) -> bool) {
            let limit = self.limit.load(Relaxed);
            let mut g = self.inner.lock();
            if g.total <= limit { return; }
            let target = limit / 10 * 9;
            let mut v: Vec<(u64, Key)> = g.map.iter().filter(|(k, _)| !pinned(k)).map(|(k, e)| (e.last, *k)).collect();
            v.sort_unstable();                                   // oldest first
            for (_, k) in v { if g.total <= target { break; } self.remove_locked(&mut g, &k); }
        }

        pub fn purge_book(&self, book: i64) -> u64 {
            let mut g = self.inner.lock();
            let keys: Vec<Key> = g.map.keys().filter(|k| k.0 == book).copied().collect();
            let before = g.total;
            for k in keys { self.remove_locked(&mut g, &k); }
            let _ = std::fs::remove_dir_all(self.dir.join(book.to_string()));
            before - g.total
        }
        pub fn clear(&self, pinned: &dyn Fn(&Key) -> bool) -> u64 {
            let mut g = self.inner.lock();
            let keys: Vec<Key> = g.map.keys().filter(|k| !pinned(k)).copied().collect();
            let before = g.total;
            for k in keys { self.remove_locked(&mut g, &k); }
            before - g.total
        }

        /// Persist batched LRU ticks (call every ~30 s from a worker and on exit).
        pub fn flush_touches(&self) {
            let mut g = self.inner.lock();
            let dirty: Vec<(Key, u64)> = g.map.iter().filter(|(_, e)| e.dirty).map(|(k, e)| (*k, e.last)).collect();
            if dirty.is_empty() { return; }
            if let Ok(tx) = g.db.transaction() {
                for (k, t) in &dirty { let _ = tx.execute("UPDATE cache_entries SET last_access=?4 WHERE book_id=?1 AND idx=?2 AND variant=?3", params![k.0, k.1, k.2, *t as i64]); }
                let _ = tx.commit();
            }
            for (k, _) in dirty { if let Some(e) = g.map.get_mut(&k) { e.dirty = false; } }
        }

        pub fn set_limit(&self, bytes: u64) { self.limit.store(bytes, Relaxed); }
        pub fn stats(&self) -> CacheStats {
            let g = self.inner.lock();
            CacheStats { bytes: g.total, limit_bytes: self.limit.load(Relaxed), entries: g.map.len() as u64 }
        }
    }
}

// ==== mod extractor: prioritised, cancellable extraction queue ================
pub mod extractor {
    use super::{cache::*, error::ReaderError, model::*, scan};
    use parking_lot::{Condvar, Mutex};
    use std::{cmp::Ordering, collections::{BinaryHeap, HashMap}, fs::File, io::{BufReader, Cursor, Read},
              sync::{atomic::{AtomicBool, Ordering::SeqCst}, Arc}};

    type Res = Result<CachedPage, ReaderError>;
    type ZipHandle = Option<(i64, zip::ZipArchive<BufReader<File>>)>;   // per-worker open archive

    /// What the reader currently needs. Set by `set_reading_window`.
    #[derive(Clone, Copy, Debug)]
    pub struct Window { pub book: i64, pub center: u32, pub dir: i8, pub ahead: u32, pub behind: u32, pub variant: u16 }
    impl Window {
        fn range(&self, n: u32) -> (u32, u32) { (self.center.saturating_sub(self.behind), (self.center + self.ahead).min(n.saturating_sub(1))) }
        /// Lower = sooner. Pages ahead in the reading direction are cheap, pages behind cost extra,
        /// so the 1st page behind (7) still beats the 4th page ahead (8).
        fn cost(&self, idx: u32) -> u32 {
            let d = (idx as i64 - self.center as i64) * if self.dir < 0 { -1 } else { 1 };
            if d >= 0 { (d * 2) as u32 } else { ((-d) * 2 + 5) as u32 }
        }
        /// Never evict these (window +-2 extra pages of slack).
        pub fn pins(&self, k: &Key) -> bool { k.0 == self.book && k.1 + self.behind + 2 >= self.center && k.1 <= self.center + self.ahead + 2 }
    }

    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)] enum Class { Prefetch = 0, Demand = 1 }
    #[derive(PartialEq, Eq)] struct Job { key: Key, class: Class, cost: u32, seq: u64 }
    impl Ord for Job { // BinaryHeap pops the GREATEST: Demand first, then lowest cost, then oldest
        fn cmp(&self, o: &Self) -> Ordering { self.class.cmp(&o.class).then(o.cost.cmp(&self.cost)).then(o.seq.cmp(&self.seq)) }
    }
    impl PartialOrd for Job { fn partial_cmp(&self, o: &Self) -> Option<Ordering> { Some(self.cmp(o)) } }

    /// One per queued/in-flight key: lets any number of waiters (protocol handlers) join the same job.
    struct Slot { done: Mutex<Option<Res>>, cv: Condvar, started: AtomicBool, demanded: AtomicBool }
    impl Slot { fn new() -> Arc<Self> { Arc::new(Self { done: Mutex::new(None), cv: Condvar::new(), started: AtomicBool::new(false), demanded: AtomicBool::new(false) }) } }

    struct State {
        win: Option<Window>, heap: BinaryHeap<Job>, slots: HashMap<Key, Arc<Slot>>,
        books: HashMap<i64, Arc<BookMeta>>, seq: u64, shutdown: bool,
    }
    pub struct Extractor { st: Mutex<State>, cv: Condvar, pub cache: Arc<PageCache> }

    impl Extractor {
        pub fn start(cache: Arc<PageCache>, workers: usize) -> Arc<Self> {
            let ex = Arc::new(Self { cache, cv: Condvar::new(),
                st: Mutex::new(State { win: None, heap: BinaryHeap::new(), slots: HashMap::new(), books: HashMap::new(), seq: 0, shutdown: false }) });
            for n in 0..workers {
                let w = ex.clone();
                std::thread::Builder::new().name(format!("mh-extract-{n}")).spawn(move || w.worker()).expect("spawn");
            }
            ex
        }
        pub fn shutdown(&self) { self.st.lock().shutdown = true; self.cv.notify_all(); self.cache.flush_touches(); }

        pub fn register_book(&self, meta: Arc<BookMeta>) {
            let mut st = self.st.lock();
            st.books.insert(meta.id, meta.clone());
            while st.books.len() > 2 {                              // keep current + previous only
                let victim = st.books.keys().copied().find(|k| *k != meta.id && Some(*k) != st.win.map(|w| w.book));
                match victim { Some(v) => { st.books.remove(&v); } None => break }
            }
        }
        pub fn book(&self, id: i64) -> Option<Arc<BookMeta>> { self.st.lock().books.get(&id).cloned() }
        pub fn window(&self) -> Option<Window> { self.st.lock().win }

        pub fn close_book(&self, id: i64) {
            let mut st = self.st.lock();
            st.books.remove(&id);
            if st.win.map(|w| w.book) == Some(id) { st.win = None; }
            let jobs: Vec<Job> = st.heap.drain().collect();
            for j in jobs { if j.key.0 == id { Self::cancel(&mut st, j.key); } else { st.heap.push(j); } }
        }

        fn cancel(st: &mut State, key: Key) {
            if let Some(slot) = st.slots.remove(&key) {
                if !slot.started.load(SeqCst) {                     // in-flight work is allowed to finish (it's cached for later)
                    *slot.done.lock() = Some(Err(ReaderError::Cancelled)); slot.cv.notify_all();
                }
            }
        }

        /// Re-aim the queue. Called on scroll (throttled) and immediately on jumps.
        pub fn set_window(&self, w: Window) -> Result<(), ReaderError> {
            let mut st = self.st.lock();
            let n = st.books.get(&w.book).ok_or(ReaderError::BookNotOpen)?.pages.len() as u32;
            if n == 0 { return Ok(()); }
            st.win = Some(w);
            let (lo, hi) = w.range(n);

            // 1) re-prioritise and cancel: drain the (small) heap, keep what is still wanted
            let old: Vec<Job> = st.heap.drain().collect();
            for mut j in old {
                let other_variant = j.key.2 != w.variant;          // e.g. scrubber thumbnails: never auto-cancelled
                let inside = j.key.0 == w.book && (lo..=hi).contains(&j.key.1);
                if inside || other_variant { j.cost = w.cost(j.key.1); st.heap.push(j); }
                else { Self::cancel(&mut st, j.key); }              // outside window: waiters get Cancelled (-> HTTP 503)
            }
            // 2) enqueue what is missing
            for idx in lo..=hi {
                let key = (w.book, idx, w.variant);
                if st.slots.contains_key(&key) || self.cache.contains(&key) { continue; }
                st.slots.insert(key, Slot::new());
                st.seq += 1; let seq = st.seq;
                st.heap.push(Job { key, class: Class::Prefetch, cost: w.cost(idx), seq });
            }
            drop(st);
            self.cv.notify_all();
            Ok(())
        }

        /// BLOCKING. Used by the protocol handler: cache hit, or join/promote the job and wait for it.
        pub fn get_or_extract(&self, key: Key) -> Res {
            if let Some(p) = self.cache.get(&key) { return Ok(p); }
            let slot = {
                let mut st = self.st.lock();
                let n = st.books.get(&key.0).ok_or(ReaderError::BookNotOpen)?.pages.len() as u32;
                if key.1 >= n { return Err(ReaderError::PageOutOfRange); }
                let slot = st.slots.entry(key).or_insert_with(Slot::new).clone();
                slot.demanded.store(true, SeqCst);
                if !slot.started.load(SeqCst) {                     // (re)queue as Demand; a stale Prefetch entry is skipped by `started`
                    let cost = st.win.map(|w| w.cost(key.1)).unwrap_or(0);
                    st.seq += 1; let seq = st.seq;
                    st.heap.push(Job { key, class: Class::Demand, cost, seq });
                    self.cv.notify_one();
                }
                slot
            };
            let mut g = slot.done.lock();
            while g.is_none() { slot.cv.wait(&mut g); }
            g.clone().unwrap()
        }

        fn worker(self: Arc<Self>) {
            let mut zip: ZipHandle = None;
            let mut last_flush = std::time::Instant::now();
            loop {
                let (job, slot, book) = {
                    let mut st = self.st.lock();
                    loop {
                        if st.shutdown { return; }
                        if let Some(job) = st.heap.pop() {
                            let Some(slot) = st.slots.get(&job.key).cloned() else { continue };   // cancelled
                            if slot.started.swap(true, SeqCst) { continue; }                      // duplicate heap entry
                            let Some(book) = st.books.get(&job.key.0).cloned() else { st.slots.remove(&job.key); continue };
                            break (job, slot, book);
                        }
                        self.cv.wait(&mut st);
                    }
                };
                let res = self.process(job.key, &slot, &book, &mut zip);
                self.st.lock().slots.remove(&job.key);
                *slot.done.lock() = Some(res); slot.cv.notify_all();

                let win = self.window();
                self.cache.evict_if_needed(&|k| win.map_or(false, |w| w.pins(k)));   // never holds st while in cache lock
                if last_flush.elapsed().as_secs() >= 30 { self.cache.flush_touches(); last_flush = std::time::Instant::now(); }
            }
        }

        fn still_wanted(&self, key: Key) -> bool {
            match self.window() { Some(w) => key.2 != w.variant || w.pins(&key), None => false }
        }

        fn process(&self, key: Key, slot: &Slot, book: &BookMeta, zip: &mut ZipHandle) -> Res {
            if let Some(p) = self.cache.get(&key) { return Ok(p); }            // sibling slice already produced it
            let (_, idx, variant) = key;
            let page = book.pages.get(idx as usize).ok_or(ReaderError::PageOutOfRange)?;
            let raw = read_entry(book, page, zip)?;                            // phase 1: bytes out of the source
            if !slot.demanded.load(SeqCst) && !self.still_wanted(key) { return Err(ReaderError::Cancelled); } // phase check: jumped away

            if variant == 0 && !page.is_slice() {                              // fast path: store as-is
                let (_, ext) = sniff(&raw).ok_or_else(|| ReaderError::Corrupt("not a recognised image".into()))?;
                return self.store(key, ext, &raw);
            }
            // decode path: slices and/or width variants. Decode ONCE, emit every slice of the image.
            if page.w as u64 * page.full_h as u64 > scan::MAX_PIXELS { return Err(ReaderError::Unsupported("image too large".into())); }
            let img = decode(&raw)?;
            let (first, last) = if page.is_slice() { book.groups[page.src_page as usize] } else { (idx, idx) };
            let mut wanted = None;
            for vi in first..=last {
                let k = (book.id, vi, variant);
                let p = &book.pages[vi as usize];
                if vi != idx && self.cache.contains(&k) { continue; }
                let part = if p.is_slice() { img.crop_imm(0, p.y0, img.width(), p.h) } else { img.clone() };
                let part = shrink_to(part, variant);
                let (bytes, ext) = encode(&part)?;
                let cp = self.store(k, ext, &bytes)?;
                if vi == idx { wanted = Some(cp); }
            }
            wanted.ok_or(ReaderError::PageOutOfRange)
        }

        fn store(&self, key: Key, ext: &'static str, bytes: &[u8]) -> Res {
            let path = self.cache.path_for(&key, ext);
            if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
            let tmp = path.with_extension("part");
            std::fs::write(&tmp, bytes)?;
            std::fs::rename(&tmp, &path)?;                                     // atomic: readers never see half a file
            Ok(self.cache.insert(key, ext, bytes.len() as u64))
        }
    }

    fn read_entry(book: &BookMeta, page: &PageMeta, zip: &mut ZipHandle) -> Result<Vec<u8>, ReaderError> {
        match book.kind {
            SourceKind::Zip => {
                if zip.as_ref().map(|(id, _)| *id) != Some(book.id) { *zip = Some((book.id, scan::open_zip(&book.path)?)); }
                let ar = &mut zip.as_mut().unwrap().1;
                let mut e = ar.by_index(page.entry as usize)?;
                if e.size() > scan::MAX_ENTRY { return Err(ReaderError::Corrupt("entry too large".into())); }
                let mut buf = Vec::with_capacity(e.size() as usize);
                e.read_to_end(&mut buf)?;                                      // CRC checked by the zip crate at EOF
                Ok(buf)
            }
            SourceKind::Folder => Ok(std::fs::read(book.path.join(&page.name))?),
            SourceKind::Images => Ok(std::fs::read(&page.name)?),
        }
    }

    /// Trust the BYTES, not the file extension.
    pub fn sniff(b: &[u8]) -> Option<(&'static str, &'static str)> {
        if b.starts_with(&[0xFF, 0xD8, 0xFF]) { Some(("image/jpeg", "jpg")) }
        else if b.starts_with(&[0x89, b'P', b'N', b'G']) { Some(("image/png", "png")) }
        else if b.starts_with(b"GIF8") { Some(("image/gif", "gif")) }
        else if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" { Some(("image/webp", "webp")) }
        else if b.len() > 12 && &b[4..8] == b"ftyp" && (&b[8..12] == b"avif" || &b[8..12] == b"avis") { Some(("image/avif", "avif")) }
        else if b.starts_with(b"BM") { Some(("image/bmp", "bmp")) }
        else { None }
    }

    fn decode(raw: &[u8]) -> Result<image::DynamicImage, ReaderError> {
        let mut r = image::ImageReader::new(Cursor::new(raw)).with_guessed_format()?;
        let mut lim = image::Limits::default(); lim.max_alloc = Some(1 << 30);    // 1 GiB hard ceiling
        r.limits(lim);
        Ok(r.decode()?)
    }
    fn shrink_to(img: image::DynamicImage, variant: u16) -> image::DynamicImage {
        if variant == 0 || img.width() <= variant as u32 { return img; }
        let nh = ((img.height() as u64 * variant as u64) / img.width() as u64).max(1) as u32;
        img.resize_exact(variant as u32, nh, image::imageops::FilterType::Lanczos3)
    }
    /// JPEG q92 normally; PNG if the source has alpha (keeps transparency, lossless).
    fn encode(img: &image::DynamicImage) -> Result<(Vec<u8>, &'static str), ReaderError> {
        let mut out = Vec::new();
        if img.color().has_alpha() {
            img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)?; Ok((out, "png"))
        } else {
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92);
            image::DynamicImage::ImageRgb8(img.to_rgb8()).write_with_encoder(enc)?; Ok((out, "jpg"))
        }
    }
}

// ==== mod protocol: mhpage://  ================================================
// Windows/WebView2 URL:  http://mhpage.localhost/{book_id}/{virtual_idx}?v={variant}&s={sig}[&r={retry}]
pub mod protocol {
    use super::{error::ReaderError, extractor::Extractor, model::*};
    use std::sync::Arc;

    fn parse(uri: &http::Uri) -> Result<Key, ReaderError> {
        let segs: Vec<&str> = uri.path().trim_matches('/').split('/').collect();
        if segs.len() != 2 { return Err(ReaderError::BadRequest("expected /book/idx".into())); }
        let book = segs[0].parse::<i64>().map_err(|_| ReaderError::BadRequest("book id".into()))?;
        let idx = segs[1].parse::<u32>().map_err(|_| ReaderError::BadRequest("page idx".into()))?;
        let variant = uri.query().unwrap_or("").split('&').find_map(|kv| kv.strip_prefix("v=")).map(|v| v.parse::<u16>()).transpose()
            .map_err(|_| ReaderError::BadRequest("variant".into()))?.unwrap_or(0);
        if !VARIANTS.contains(&variant) { return Err(ReaderError::BadRequest("variant not allowed".into())); }
        Ok((book, idx, variant))               // NOTE: nothing from the URI ever becomes a filesystem path
    }

    fn base() -> http::response::Builder {
        http::Response::builder()
            .header("Access-Control-Allow-Origin", "*")          // page origin (tauri.localhost) != mhpage.localhost
            .header("X-Content-Type-Options", "nosniff")
    }
    fn err(e: &ReaderError) -> http::Response<Vec<u8>> {
        base().status(e.http_status()).header("Content-Type", "application/json").header("x-mh-error", e.code())
            .body(serde_json::to_vec(e).unwrap_or_default()).unwrap()
    }

    /// BLOCKING (call inside spawn_blocking). Tauri v2 response bodies are whole buffers - there is no
    /// chunked streaming - so a page is read fully; pages are <= a few MB, slices keep it bounded.
    pub fn serve(ex: &Extractor, req: &http::Request<Vec<u8>>) -> http::Response<Vec<u8>> {
        if req.method() == http::Method::OPTIONS {
            return base().status(204).header("Access-Control-Allow-Methods", "GET").body(vec![]).unwrap();
        }
        let key = match parse(req.uri()) { Ok(k) => k, Err(e) => return err(&e) };
        for attempt in 0..2 {
            let page = match ex.get_or_extract(key) { Ok(p) => p, Err(e) => return err(&e) };
            match std::fs::read(&page.path) {
                Ok(bytes) => return base().status(200)
                    .header("Content-Type", page.mime)
                    .header("Content-Length", bytes.len())
                    .header("Cache-Control", "private, max-age=31536000, immutable") // URL carries sig; harmless if ignored
                    .body(bytes).unwrap(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && attempt == 0 => ex.cache.remove(&key), // index lied: re-extract once
                Err(e) => return err(&e.into()),
            }
        }
        err(&ReaderError::Io("cache read failed".into()))
    }

    pub fn handle(ex: Arc<Extractor>, req: http::Request<Vec<u8>>, responder: tauri::UriSchemeResponder) {
        tauri::async_runtime::spawn(async move {
            let resp = tauri::async_runtime::spawn_blocking(move || serve(&ex, &req)).await
                .unwrap_or_else(|_| err(&ReaderError::Io("join".into())));
            responder.respond(resp);
        });
    }
}

// ==== mod commands: everything callable via invoke() ==========================
pub mod commands {
    use super::{cache::PageCache, chapters, db::{self, Db}, error::ReaderError, extractor::{Extractor, Window}, model::*, scan};
    use std::{path::PathBuf, sync::Arc};
    use tauri::State;

    #[derive(Clone)]
    pub struct ReaderState { pub db: Db, pub cache: Arc<PageCache>, pub extractor: Arc<Extractor> }

    fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ReaderError> + Send + 'static) -> impl std::future::Future<Output = Result<T, ReaderError>> {
        async move { tauri::async_runtime::spawn_blocking(f).await.map_err(|e| ReaderError::Io(e.to_string()))? }
    }

    fn open_blocking(s: &ReaderState, id: i64, force: bool) -> Result<OpenedBook, ReaderError> {
        let row = db::book_row(&s.db.lock(), id)?;
        let kind = SourceKind::from_db(&row.kind).ok_or_else(|| ReaderError::Unsupported(row.kind.clone()))?;
        let path = PathBuf::from(&row.path);
        if kind != SourceKind::Images && !path.exists() { return Err(ReaderError::SourceMissing(row.path)); }   // -> "Locate file..." dialog
        let sig = scan::quick_sig(kind, &path)?;

        let cached = if !force && row.scan_sig.as_deref() == Some(sig.as_str()) { db::load_pages(&s.db.lock(), id)? } else { vec![] };
        let real = if !cached.is_empty() { cached } else {
            let fresh = match kind {                           // zip: central dir + header reads, folder: header reads
                SourceKind::Zip => scan::scan_zip(&path)?,
                SourceKind::Folder => scan::scan_folder(&path),
                SourceKind::Images => return Err(ReaderError::Unsupported("loose-image books: load path list from your table".into())),
            };
            if fresh.is_empty() { return Err(ReaderError::Corrupt("no pages found".into())); }
            s.cache.purge_book(id);                            // source changed: derived files are stale
            db::store_pages(&mut s.db.lock(), id, &fresh, &sig)?;
            fresh
        };

        let pages = scan::build_pages(&real);                  // slicing decided here, every open
        let groups = scan::groups(&pages);
        let meta = Arc::new(BookMeta { id, kind, path, sig: sig.clone(), pages, groups });
        let eff = chapters::effective(&s.db.lock(), id)?;       // per-book override, else global default
        let out = OpenedBook {
            book_id: id, title: row.title, sig, real_pages: real.len() as u32,
            dims: meta.pages.iter().map(|p| [p.w, p.h]).collect(),
            src: meta.pages.iter().map(|p| p.src_page).collect(),
            chapters: chapters::resolve(eff.mode, eff.pages, &real),   // Off => [], Folder => by dir, Pages => by N
            folder_chapters: chapters::folder(&real),
            chapter_mode: eff.mode, chapter_pages: eff.pages, chapter_overridden: eff.overridden,
            resume: db::load_progress(&s.db.lock(), id)?.filter(|p| (p.page as usize) < real.len()),
        };
        s.extractor.register_book(meta);
        Ok(out)
    }

    // Tauri v2: JS passes camelCase keys (bookId) -> snake_case args (book_id). async + borrowed State => must return Result.
    #[tauri::command] pub async fn open_book(state: State<'_, ReaderState>, book_id: i64) -> Result<OpenedBook, ReaderError> {
        let s = state.inner().clone(); blocking(move || open_blocking(&s, book_id, false)).await
    }
    #[tauri::command] pub async fn rescan_book(state: State<'_, ReaderState>, book_id: i64) -> Result<OpenedBook, ReaderError> {
        let s = state.inner().clone(); blocking(move || open_blocking(&s, book_id, true)).await
    }
    #[tauri::command] pub async fn close_book(state: State<'_, ReaderState>, book_id: i64) -> Result<(), ReaderError> {
        state.extractor.close_book(book_id); Ok(())
    }
    #[tauri::command] pub async fn set_reading_window(state: State<'_, ReaderState>, book_id: i64, center: u32, direction: i8,
        ahead: u32, behind: u32, variant: u16) -> Result<(), ReaderError> {
        if !VARIANTS.contains(&variant) { return Err(ReaderError::BadRequest("variant".into())); }
        state.extractor.set_window(Window { book: book_id, center, dir: direction.signum().max(-1).min(1), ahead: ahead.min(16), behind: behind.min(8), variant })
    }
    #[tauri::command] pub async fn save_progress(state: State<'_, ReaderState>, book_id: i64, pos: SavedPosition, reached_end: bool) -> Result<(), ReaderError> {
        let s = state.inner().clone(); blocking(move || db::save_progress(&s.db.lock(), book_id, pos, reached_end)).await
    }
    #[tauri::command] pub async fn load_progress(state: State<'_, ReaderState>, book_id: i64) -> Result<Option<SavedPosition>, ReaderError> {
        let s = state.inner().clone(); blocking(move || db::load_progress(&s.db.lock(), book_id)).await
    }
    #[tauri::command] pub async fn fix_page_dims(state: State<'_, ReaderState>, book_id: i64, fixes: Vec<DimFix>) -> Result<(), ReaderError> {
        let s = state.inner().clone();
        blocking(move || { db::fix_dims(&mut s.db.lock(), book_id, &fixes)?;
            s.cache.purge_book(book_id);   // slicing may differ next open (virtual indices shift) -> cached slices are stale
            Ok(()) }).await
    }
    #[tauri::command] pub async fn cache_stats(state: State<'_, ReaderState>) -> Result<CacheStats, ReaderError> { Ok(state.cache.stats()) }
    #[tauri::command] pub async fn cache_set_limit(state: State<'_, ReaderState>, bytes: u64) -> Result<(), ReaderError> {
        state.cache.set_limit(bytes.max(64 << 20));
        let win = state.extractor.window();
        state.cache.evict_if_needed(&|k| win.map_or(false, |w| w.pins(k))); Ok(())
    }
    #[tauri::command] pub async fn cache_clear(state: State<'_, ReaderState>, book_id: Option<i64>) -> Result<u64, ReaderError> {
        let s = state.inner().clone();
        blocking(move || Ok(match book_id {
            Some(id) => s.cache.purge_book(id),
            None => { let win = s.extractor.window(); s.cache.clear(&|k| win.map_or(false, |w| w.pins(k))) }
        })).await
    }
}

// ==== wiring (lib.rs) ========================================================
//
// pub fn run() {
//     tauri::Builder::default()
//         .setup(|app| {
//             let data = resolve_data_dir(app)?;                    // your existing data-dir logic (+ writable fallback)
//             let cache = Arc::new(cache::PageCache::open(data.join("cache"), 512 << 20)?);
//             let extractor = extractor::Extractor::start(cache.clone(), 2);   // 1 in memory-saver mode
//             app.manage(commands::ReaderState { db: your_db_handle, cache, extractor });
//             Ok(())
//         })
//         .register_asynchronous_uri_scheme_protocol("mhpage", |ctx, req, responder| {
//             let ex = ctx.app_handle().state::<commands::ReaderState>().extractor.clone();
//             protocol::handle(ex, req, responder);
//         })
//         .invoke_handler(tauri::generate_handler![
//             commands::open_book, commands::rescan_book, commands::close_book, commands::set_reading_window,
//             commands::save_progress, commands::load_progress, commands::fix_page_dims,
//             commands::cache_stats, commands::cache_set_limit, commands::cache_clear,
//             chapters::get_chapter_defaults, chapters::set_chapter_defaults,
//             chapters::set_book_chapter_settings, chapters::clear_book_chapter_settings, chapters::episode_info,
//             bookmarks::add_bookmark, bookmarks::list_bookmarks, bookmarks::delete_bookmark, bookmarks::update_bookmark_note,
//             bookmarks::reanchor_bookmark, bookmarks::bookmark_counts])
// Migrations at startup, in order: db::DDL, chapters::DDL, then bookmarks::migrate(&conn) (it also runs bookmarks::DDL).
//         .run(tauri::generate_context!()).expect("run");
// }
//
// tauri.conf.json:
//   "app": { "withGlobalTauri": true,
//            "security": { "csp": "default-src 'self'; img-src 'self' http://mhpage.localhost data:; connect-src 'self' ipc: http://ipc.localhost http://mhpage.localhost" } }
// (if you set windows[].useHttpsScheme = true, use https://mhpage.localhost instead)
// Portable: also point WebView2's user-data folder into your data dir (WebviewWindowBuilder::data_directory),
// otherwise its cache lands in %LOCALAPPDATA% and "portable" isn't.

// ==== mod chapters: chapter modes (navigation/display layer ONLY) =============
// None of this touches virtual pages, slicing, cache keys or the extraction queue. Episodes are derived
// on demand from (a) page NAMES already in pages_meta (folder mode) or (b) the page COUNT (pages mode),
// so changing mode or N never rescans files and never changes what the strip renders.
pub mod chapters {
    use super::{commands::ReaderState, db, error::ReaderError, model::*, scan};
    use rusqlite::{params, Connection, OptionalExtension};
    use tauri::State;

    pub const DEFAULT_PAGES: u32 = 20;
    pub const MAX_PAGES: u32 = 9_999;

    /// Run once from your migration step. If `settings` / `book_settings` already exist in your app, adapt:
    ///   ALTER TABLE book_settings ADD COLUMN chapter_mode TEXT;      -- NULL = use global default
    ///   ALTER TABLE book_settings ADD COLUMN chapter_pages INTEGER;  -- NULL = use global default
    /// Global defaults are two rows in the key/value `settings` table: 'chapter.mode', 'chapter.pages'.
    pub const DDL: &str = "
      CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS book_settings(book_id INTEGER PRIMARY KEY, chapter_mode TEXT, chapter_pages INTEGER);";

    fn sane(n: i64) -> u32 { n.clamp(1, MAX_PAGES as i64) as u32 }

    // ---- settings resolution -------------------------------------------------------------------
    /// Global default: mode 'folder' (= the previous behaviour; flat books simply show no chapters), N = 20.
    pub fn defaults(c: &Connection) -> Result<ChapterSettings, ReaderError> {
        let get = |k: &str| -> Result<Option<String>, ReaderError> {
            Ok(c.query_row("SELECT value FROM settings WHERE key=?1", [k], |r| r.get(0)).optional()?)
        };
        Ok(ChapterSettings {
            mode: get("chapter.mode")?.and_then(|s| ChapterMode::parse(&s)).unwrap_or(ChapterMode::Folder),
            pages: get("chapter.pages")?.and_then(|s| s.parse::<i64>().ok()).map(sane).unwrap_or(DEFAULT_PAGES),
        })
    }

    pub struct Effective { pub mode: ChapterMode, pub pages: u32, pub overridden: bool }

    /// Per-book value wins; NULL columns fall back to the global default, field by field.
    pub fn effective(c: &Connection, book_id: i64) -> Result<Effective, ReaderError> {
        let d = defaults(c)?;
        let row: Option<(Option<String>, Option<i64>)> = c
            .query_row("SELECT chapter_mode, chapter_pages FROM book_settings WHERE book_id=?1", [book_id],
                       |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let (m, p) = row.unwrap_or((None, None));
        let mode = m.as_deref().and_then(ChapterMode::parse);
        let pages = p.map(sane);
        Ok(Effective { mode: mode.unwrap_or(d.mode), pages: pages.unwrap_or(d.pages), overridden: mode.is_some() || pages.is_some() })
    }

    // ---- episode computation (pure) ------------------------------------------------------------
    fn to_episodes(mut starts: Vec<(u32, String)>, total: u32) -> Vec<Episode> {
        if starts.first().map_or(false, |s| s.0 > 0) { starts.insert(0, (0, String::new())); }   // loose pages before the first folder
        (0..starts.len()).map(|i| {
            let end = starts.get(i + 1).map_or(total, |n| n.0);
            Episode { index: i as u32 + 1, page: starts[i].0, len: end - starts[i].0, title: starts[i].1.clone() }
        }).collect()
    }
    /// Folder mode: scan::chapters already returns [] unless >= 2 distinct folders exist.
    pub fn folder(real: &[RealPage]) -> Vec<Episode> {
        to_episodes(scan::chapters(real).into_iter().map(|c| (c.page, c.title)).collect(), real.len() as u32)
    }
    /// Pages mode: episode k = pages (k-1)*N+1 ..= k*N (1-based), last one may be shorter; < 2 episodes => [].
    pub fn by_pages(total: u32, n: u32) -> Vec<Episode> {
        if total == 0 { return vec![]; }
        let n = n.clamp(1, total);
        let count = total.div_ceil(n);
        if count < 2 { return vec![]; }
        (0..count).map(|k| { let p = k * n; Episode { index: k + 1, page: p, len: n.min(total - p), title: String::new() } }).collect()
    }
    pub fn resolve(mode: ChapterMode, pages: u32, real: &[RealPage]) -> Vec<Episode> {
        match mode { ChapterMode::Off => vec![], ChapterMode::Folder => folder(real), ChapterMode::Pages => by_pages(real.len() as u32, pages) }
    }
    pub fn locate(eps: &[Episode], page: u32) -> Option<EpisodeInfo> {
        if eps.is_empty() { return None; }
        let i = eps.partition_point(|e| e.page <= page).saturating_sub(1);
        Some(EpisodeInfo { index: eps[i].index, total: eps.len() as u32 })
    }

    // ---- commands ------------------------------------------------------------------------------
    #[tauri::command]
    pub async fn get_chapter_defaults(state: State<'_, ReaderState>) -> Result<ChapterSettings, ReaderError> { defaults(&state.db.lock()) }

    /// Settings page: the global default.
    #[tauri::command]
    pub async fn set_chapter_defaults(state: State<'_, ReaderState>, mode: ChapterMode, pages: u32) -> Result<(), ReaderError> {
        let c = state.db.lock();
        for (k, v) in [("chapter.mode", mode.as_str().to_string()), ("chapter.pages", sane(pages as i64).to_string())] {
            c.execute("INSERT INTO settings(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![k, v])?;
        }
        Ok(())
    }

    /// Reader quick settings with "Remember for this book" ON.
    #[tauri::command]
    pub async fn set_book_chapter_settings(state: State<'_, ReaderState>, book_id: i64, mode: ChapterMode, pages: u32) -> Result<(), ReaderError> {
        state.db.lock().execute(
            "INSERT INTO book_settings(book_id,chapter_mode,chapter_pages) VALUES(?1,?2,?3)
             ON CONFLICT(book_id) DO UPDATE SET chapter_mode=excluded.chapter_mode, chapter_pages=excluded.chapter_pages",
            params![book_id, mode.as_str(), sane(pages as i64)])?;
        Ok(())
    }

    /// "Use default": drop this book's override (other per-book settings stay).
    #[tauri::command]
    pub async fn clear_book_chapter_settings(state: State<'_, ReaderState>, book_id: i64) -> Result<(), ReaderError> {
        state.db.lock().execute("UPDATE book_settings SET chapter_mode=NULL, chapter_pages=NULL WHERE book_id=?1", [book_id])?;
        Ok(())
    }

    /// Batch lookup for lists ("Episode 12 / 128" next to progress). One call per list render.
    /// Pages mode needs only books.page_count; folder mode reads that book's stored page names (no file access).
    /// For very large libraries add a small in-memory cache keyed by (book_id, scan_sig, mode, N).
    #[tauri::command]
    pub async fn episode_info(state: State<'_, ReaderState>, items: Vec<EpisodeQuery>) -> Result<Vec<Option<EpisodeInfo>>, ReaderError> {
        let s = state.inner().clone();
        tauri::async_runtime::spawn_blocking(move || -> Result<Vec<Option<EpisodeInfo>>, ReaderError> {
            let c = s.db.lock();
            items.iter().map(|q| -> Result<Option<EpisodeInfo>, ReaderError> {
                let eff = effective(&c, q.book_id)?;
                let eps = match eff.mode {
                    ChapterMode::Off => vec![],
                    ChapterMode::Pages => {
                        let total: u32 = c.query_row("SELECT page_count FROM books WHERE id=?1", [q.book_id], |r| r.get(0)).unwrap_or(0);
                        by_pages(total, eff.pages)
                    }
                    ChapterMode::Folder => folder(&db::load_pages(&c, q.book_id)?),
                };
                Ok(locate(&eps, q.page))
            }).collect()
        }).await.map_err(|e| ReaderError::Io(e.to_string()))?
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test] fn pages_mode_ranges() {
            let e = by_pages(100, 30);
            assert_eq!(e.iter().map(|x| (x.index, x.page, x.len)).collect::<Vec<_>>(), vec![(1, 0, 30), (2, 30, 30), (3, 60, 30), (4, 90, 10)]);
            assert!(by_pages(100, 100).is_empty() && by_pages(100, 999).is_empty() && by_pages(0, 5).is_empty());
        }
        #[test] fn locate_finds_containing_episode() {
            let e = by_pages(100, 30);
            assert_eq!(locate(&e, 29).unwrap().index, 1); assert_eq!(locate(&e, 30).unwrap().index, 2);
            assert_eq!(locate(&e, 99).unwrap().index, 4); assert!(locate(&[], 3).is_none());
        }
        #[test] fn loose_pages_before_first_folder_form_episode_one() {
            let e = to_episodes(vec![(5, "A".into()), (20, "B".into())], 30);
            assert_eq!(e.iter().map(|x| (x.page, x.len)).collect::<Vec<_>>(), vec![(0, 5), (5, 15), (20, 10)]);
        }
    }
}

// ==== mod bookmarks: bookmarks anchored to the IMAGE, not the page number =====
// Identity = the page's name inside the source, exactly as stored in pages_meta.name:
//   ZIP: the entry's full path (normalised to '/'), folder book: path relative to the book folder, loose images: absolute path.
// page_hint is only a cache of "where it was last seen". Resolution NEVER uses the hint to pick an image:
//   name found  -> status "ok"     (page = the CURRENT index of that image; hint is refreshed)
//   name absent -> status "broken" (page = None). The UI may offer the hint as an explicit user choice
//                  ("Go to old position" / "Anchor to page N") but nothing ever jumps there silently.
// So adding, removing or re-sorting pages inside the archive cannot move a bookmark onto a different image.
pub mod bookmarks {
    use super::{commands::ReaderState, error::ReaderError};
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
            if cols.iter().any(|n| n == "offset") { c.execute_batch("ALTER TABLE bookmarks RENAME COLUMN \"offset\" TO frac;")?; }
            if cols.iter().any(|n| n == "page") && !cols.iter().any(|n| n == "page_hint") { c.execute_batch("ALTER TABLE bookmarks RENAME COLUMN page TO page_hint;")?; }
            if !cols.iter().any(|n| n == "entry_name") { c.execute_batch("ALTER TABLE bookmarks ADD COLUMN entry_name TEXT NOT NULL DEFAULT '';")?; }
            if !cols.iter().any(|n| n == "created_at") { c.execute_batch("ALTER TABLE bookmarks ADD COLUMN created_at INTEGER NOT NULL DEFAULT 0;")?; }
        }
        c.execute_batch(DDL)?;
        Ok(())
    }

    #[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)] #[serde(rename_all = "snake_case")]
    pub enum BookmarkStatus { Ok, Broken }

    #[derive(Serialize, Clone, Debug)] #[serde(rename_all = "camelCase")]
    pub struct Bookmark {
        pub id: i64, pub book_id: i64,
        pub entry_name: String,        // identity (full path inside the source)
        pub file_name: String,         // last path segment, for display
        pub page_hint: u32,            // cached index; refreshed whenever the name resolves
        pub page: Option<u32>,         // CURRENT real page of that image, found BY NAME; None = broken
        pub frac: f64, pub note: Option<String>, pub created_at: i64,
        pub status: BookmarkStatus,
    }
    #[derive(Serialize)] #[serde(rename_all = "camelCase")]
    pub struct BookmarkCount { pub book_id: i64, pub count: u32 }

    struct Row { id: i64, book_id: i64, entry: String, hint: u32, frac: f64, note: Option<String>, created: i64 }

    fn now_ms() -> i64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0) }
    fn clean_note(n: Option<String>) -> Option<String> {
        n.map(|s| s.trim().chars().take(500).collect::<String>()).filter(|s| !s.is_empty())
    }
    fn file_name(entry: &str) -> String { entry.rsplit(|c| c == '/' || c == '\\').next().unwrap_or(entry).to_string() }

    /// name -> current real page. The ONLY way a bookmark finds its image.
    fn names(c: &Connection, book_id: i64) -> Result<HashMap<String, u32>, ReaderError> {
        let mut s = c.prepare("SELECT name, page FROM pages_meta WHERE book_id=?1")?;
        let rows = s.query_map([book_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
    pub fn resolve_entry(names: &HashMap<String, u32>, entry: &str) -> Option<u32> { names.get(entry).copied() }   // never falls back to the hint

    fn row_of(r: &rusqlite::Row) -> rusqlite::Result<Row> {
        Ok(Row { id: r.get(0)?, book_id: r.get(1)?, entry: r.get(2)?, hint: r.get(3)?, frac: r.get(4)?, note: r.get(5)?, created: r.get(6)? })
    }
    const COLS: &str = "id, book_id, entry_name, page_hint, frac, note, created_at";
    fn get_row(c: &Connection, id: i64) -> Result<Row, ReaderError> {
        c.query_row(&format!("SELECT {COLS} FROM bookmarks WHERE id=?1"), [id], row_of).optional()?
            .ok_or_else(|| ReaderError::BadRequest("bookmark not found".into()))
    }
    fn to_bookmark(r: Row, names: &HashMap<String, u32>) -> Bookmark {
        let page = resolve_entry(names, &r.entry);
        Bookmark { id: r.id, book_id: r.book_id, file_name: file_name(&r.entry), page_hint: page.unwrap_or(r.hint), page, frac: r.frac, note: r.note,
                   created_at: r.created, status: if page.is_some() { BookmarkStatus::Ok } else { BookmarkStatus::Broken }, entry_name: r.entry }
    }

    // ---- commands -------------------------------------------------------------------------------------
    /// Bookmark the image at real page `page`, `frac` of the way down the whole image (the reader sends the
    /// position at the VIEWPORT CENTER; for a sliced tall image that is the real page plus its slice offset).
    /// Idempotent: an existing bookmark on the same image within 1% of the same offset is returned unchanged.
    #[tauri::command]
    pub async fn add_bookmark(state: State<'_, ReaderState>, book_id: i64, page: u32, frac: f64, note: Option<String>) -> Result<Bookmark, ReaderError> {
        let c = state.db.lock();
        let nm = names(&c, book_id)?;
        if nm.is_empty() { return Err(ReaderError::BookNotOpen); }                     // open the book (open_book) first
        let entry: String = c.query_row("SELECT name FROM pages_meta WHERE book_id=?1 AND page=?2", params![book_id, page], |r| r.get(0))
            .optional()?.ok_or(ReaderError::PageOutOfRange)?;
        let frac = if frac.is_finite() { frac.clamp(0.0, 0.9999) } else { 0.0 };
        if let Some(id) = c.query_row("SELECT id FROM bookmarks WHERE book_id=?1 AND entry_name=?2 AND ABS(frac-?3)<0.01 LIMIT 1",
                                      params![book_id, entry, frac], |r| r.get::<_, i64>(0)).optional()? {
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
    pub async fn list_bookmarks(state: State<'_, ReaderState>, book_id: i64) -> Result<Vec<Bookmark>, ReaderError> {
        let c = state.db.lock();
        let nm = names(&c, book_id)?;
        if nm.is_empty() { return Err(ReaderError::BookNotOpen); }
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
            if let Some(p) = stale { c.execute("UPDATE bookmarks SET page_hint=?2 WHERE id=?1", params![id, p])?; }
            out.push(to_bookmark(r, &nm));
        }
        out.sort_by(|a, b| match (a.page, b.page) {
            (Some(x), Some(y)) => x.cmp(&y).then(a.frac.partial_cmp(&b.frac).unwrap_or(std::cmp::Ordering::Equal)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.created_at.cmp(&b.created_at),
        });
        Ok(out)
    }

    #[tauri::command]
    pub async fn delete_bookmark(state: State<'_, ReaderState>, bookmark_id: i64) -> Result<(), ReaderError> {
        state.db.lock().execute("DELETE FROM bookmarks WHERE id=?1", [bookmark_id])?;
        Ok(())
    }

    /// `note` = None or blank clears it; trimmed; max 500 chars. Returns the bookmark as now stored.
    #[tauri::command]
    pub async fn update_bookmark_note(state: State<'_, ReaderState>, bookmark_id: i64, note: Option<String>) -> Result<Bookmark, ReaderError> {
        let c = state.db.lock();
        let row = get_row(&c, bookmark_id)?;
        c.execute("UPDATE bookmarks SET note=?2 WHERE id=?1", params![bookmark_id, clean_note(note)])?;
        Ok(to_bookmark(get_row(&c, bookmark_id)?, &names(&c, row.book_id)?))
    }

    /// Explicit repair for a BROKEN bookmark: the user picks "Anchor to page N" (N = old hint) and the bookmark now
    /// points at whatever image is at real page `page` today. Never called automatically.
    #[tauri::command]
    pub async fn reanchor_bookmark(state: State<'_, ReaderState>, bookmark_id: i64, page: u32) -> Result<Bookmark, ReaderError> {
        let c = state.db.lock();
        let row = get_row(&c, bookmark_id)?;
        let entry: String = c.query_row("SELECT name FROM pages_meta WHERE book_id=?1 AND page=?2", params![row.book_id, page], |r| r.get(0))
            .optional()?.ok_or(ReaderError::PageOutOfRange)?;
        c.execute("UPDATE bookmarks SET entry_name=?2, page_hint=?3 WHERE id=?1", params![bookmark_id, entry, page])?;
        Ok(to_bookmark(get_row(&c, bookmark_id)?, &names(&c, row.book_id)?))
    }

    /// For the library-card ribbon: bookmark count per book (books without bookmarks are omitted).
    #[tauri::command]
    pub async fn bookmark_counts(state: State<'_, ReaderState>, book_ids: Vec<i64>) -> Result<Vec<BookmarkCount>, ReaderError> {
        let c = state.db.lock();
        let mut s = c.prepare("SELECT COUNT(*) FROM bookmarks WHERE book_id=?1")?;
        let mut out = Vec::new();
        for id in book_ids { let n: u32 = s.query_row([id], |r| r.get(0))?; if n > 0 { out.push(BookmarkCount { book_id: id, count: n }); } }
        Ok(out)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn names_of(v: &[&str]) -> HashMap<String, u32> { v.iter().enumerate().map(|(i, n)| (n.to_string(), i as u32)).collect() }
        #[test] fn found_by_name_even_after_pages_are_inserted_or_reordered() {
            let before = names_of(&["a/01.jpg", "a/02.jpg", "a/03.jpg"]);
            assert_eq!(resolve_entry(&before, "a/02.jpg"), Some(1));
            let after = names_of(&["a/00_credits.jpg", "a/01.jpg", "a/03.jpg", "a/02.jpg"]);   // page added + re-sorted
            assert_eq!(resolve_entry(&after, "a/02.jpg"), Some(3));                              // still the SAME image
        }
        #[test] fn missing_image_is_broken_not_a_neighbour() {
            let after = names_of(&["a/01.jpg", "a/03.jpg"]);
            assert_eq!(resolve_entry(&after, "a/02.jpg"), None);                                 // index 1 now holds 03.jpg: must NOT be used
        }
        #[test] fn display_name_is_last_segment() { assert_eq!(file_name("Vol 1/ch02/005.jpg"), "005.jpg"); assert_eq!(file_name("x.jpg"), "x.jpg"); }
        #[test] fn notes_are_trimmed_and_blank_clears() {
            assert_eq!(clean_note(Some("  hi  ".into())).as_deref(), Some("hi")); assert_eq!(clean_note(Some("   ".into())), None); assert_eq!(clean_note(None), None);
        }
    }
}
