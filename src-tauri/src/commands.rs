//! Reader core commands (split from design/claude-v04/reader_backend.rs `pub mod commands`).
//!
//! ADAPTED per INTEGRATION_NOTES §3:
//! - `open_blocking` reads the book row from `shelf` via crate::db (no `books` table).
//! - SourceKind::Images branch is IMPLEMENTED via `scan::scan_images` on the shelf
//!   paths array (the design returned Unsupported there).
//! - BookMeta.path for Images = paths[0] as an anchor only; page files are read via
//!   PageMeta.name (absolute path) in the extractor.
//! - Resume falls back to {page: shelf.last_page, frac: 0} when there is no
//!   progress row (design only had the progress table).

use crate::cache::PageCache;
use crate::chapters;
use crate::db::{self, Db};
use crate::error::ReaderError;
use crate::extractor::{Extractor, Window};
use crate::model::*;
use crate::scan;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Emitter, State};

#[derive(Clone)]
pub struct ReaderState {
    pub db: Db,
    pub cache: Arc<PageCache>,
    pub extractor: Arc<Extractor>,
    /// In-memory page lists so opening a book again does not walk the disk.
    pub lists: Arc<ListCache>,
}

impl ReaderState {
    pub fn new(db: Db, cache: Arc<PageCache>, extractor: Arc<Extractor>) -> Self {
        Self {
            db,
            cache,
            extractor,
            lists: ListCache::new(),
        }
    }
}

struct ListEntry {
    sig: String,
    pages: Arc<Vec<RealPage>>,
    /// Signature has been checked against the files on disk.
    verified: bool,
    /// Every page header has been read (zeros that remain are unreadable files).
    dims_ready: bool,
    busy: bool,
}

struct ListInner {
    gen: HashMap<i64, u64>,
    books: HashMap<i64, ListEntry>,
}

/// Page lists kept for the life of the process, keyed by book id.
/// A generation counter cancels a follow-up when the reader opens the book again.
pub struct ListCache {
    inner: parking_lot::Mutex<ListInner>,
}

impl ListCache {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: parking_lot::Mutex::new(ListInner {
                gen: HashMap::new(),
                books: HashMap::new(),
            }),
        })
    }

    pub fn bump(&self, id: i64) -> u64 {
        let mut g = self.inner.lock();
        let n = g.gen.get(&id).copied().unwrap_or(0) + 1;
        g.gen.insert(id, n);
        // The previous follow-up is cancelled by `n`. Clear busy so this open
        // can schedule its own fill/verify instead of waiting on a dead job.
        if let Some(e) = g.books.get_mut(&id) {
            e.busy = false;
        }
        n
    }

    pub fn current(&self, id: i64) -> u64 {
        self.inner.lock().gen.get(&id).copied().unwrap_or(0)
    }

    /// Drop the cached list and cancel any follow-up still running for `id`.
    pub fn invalidate(&self, id: i64) {
        let mut g = self.inner.lock();
        g.books.remove(&id);
        let n = g.gen.get(&id).copied().unwrap_or(0) + 1;
        g.gen.insert(id, n);
    }

    fn forget(&self, id: i64) {
        self.inner.lock().books.remove(&id);
    }

    fn get(&self, id: i64) -> Option<ListEntry> {
        let g = self.inner.lock();
        let e = g.books.get(&id)?;
        Some(ListEntry {
            sig: e.sig.clone(),
            pages: Arc::clone(&e.pages),
            verified: e.verified,
            dims_ready: e.dims_ready,
            busy: e.busy,
        })
    }

    fn put(
        &self,
        id: i64,
        sig: String,
        pages: Vec<RealPage>,
        verified: bool,
        dims_ready: bool,
        busy: bool,
    ) {
        self.put_arc(id, sig, Arc::new(pages), verified, dims_ready, busy);
    }

    fn put_arc(
        &self,
        id: i64,
        sig: String,
        pages: Arc<Vec<RealPage>>,
        verified: bool,
        dims_ready: bool,
        busy: bool,
    ) {
        self.inner.lock().books.insert(
            id,
            ListEntry {
                sig,
                pages,
                verified,
                dims_ready,
                busy,
            },
        );
    }

    /// Store a finished list only if `gen` is still the open the follow-up belongs to.
    fn finish(
        &self,
        id: i64,
        gen: u64,
        sig: String,
        pages: Vec<RealPage>,
        verified: bool,
        dims_ready: bool,
    ) -> bool {
        let mut g = self.inner.lock();
        if g.gen.get(&id).copied().unwrap_or(0) != gen {
            return false;
        }
        g.books.insert(
            id,
            ListEntry {
                sig,
                pages: Arc::new(pages),
                verified,
                dims_ready,
                busy: false,
            },
        );
        true
    }

    fn mark_verified(&self, id: i64, gen: u64) -> bool {
        let mut g = self.inner.lock();
        if g.gen.get(&id).copied().unwrap_or(0) != gen {
            return false;
        }
        if let Some(e) = g.books.get_mut(&id) {
            e.verified = true;
            e.busy = false;
            return true;
        }
        false
    }
}

fn perf_ms(label: &str, t0: Instant) {
    if std::env::var_os("MH_PERF").is_some() {
        eprintln!("[mh-perf] {label} {}ms", t0.elapsed().as_millis());
    }
}

fn names_fingerprint(pages: &[RealPage]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for p in pages {
        for b in p.name.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h ^= p.entry as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h ^= pages.len() as u64;
    h
}

enum Followup {
    /// Cached folder list was returned before the directory walk.
    Verify {
        id: i64,
        gen: u64,
        kind: SourceKind,
        paths: Vec<String>,
        sig: String,
    },
    /// Names are known; image headers past the first screen are still missing.
    Fill {
        id: i64,
        gen: u64,
        kind: SourceKind,
        paths: Vec<String>,
        sig: String,
        names: u64,
    },
}

struct OpenOutcome {
    book: OpenedBook,
    followup: Option<Followup>,
}

fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, ReaderError> + Send + 'static,
) -> impl std::future::Future<Output = Result<T, ReaderError>> {
    async move {
        tauri::async_runtime::spawn_blocking(f)
            .await
            .map_err(|e| ReaderError::Io(e.to_string()))?
    }
}

fn source_sig(kind: SourceKind, paths: &[String]) -> Result<String, ReaderError> {
    if kind == SourceKind::Images {
        return Ok("i".into());
    }
    if kind == SourceKind::Folder && paths.len() > 1 {
        let mut parts = Vec::with_capacity(paths.len());
        for p in paths {
            parts.push(
                scan::quick_sig(SourceKind::Folder, Path::new(p)).unwrap_or_else(|_| "missing".into()),
            );
        }
        return Ok(parts.join("|"));
    }
    let path = paths.first().map(PathBuf::from).unwrap_or_default();
    scan::quick_sig(kind, &path)
}

fn edited_sig(pages: &[RealPage]) -> String {
    // Content-derived: every page edit changes the sig, so JS image URLs
    // (which carry `s=<sig>`) never serve a stale page from the webview cache.
    let mut h: u64 = 0xcbf29ce484222325;
    for p in pages {
        for b in format!("{}|{}|{:?};", p.entry, p.name, p.src_path).bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    format!("edited:{h:016x}")
}

fn resume_around(s: &ReaderState, id: i64, last_page: i64) -> usize {
    if let Ok(Some(p)) = db::load_progress(&s.db.lock(), id) {
        return p.page as usize;
    }
    last_page.max(0) as usize
}

fn assemble(
    s: &ReaderState,
    id: i64,
    title: &str,
    kind: SourceKind,
    path: PathBuf,
    sig: &str,
    real: &[RealPage],
    last_page: i64,
    interactive: bool,
    gen: u64,
) -> Result<OpenedBook, ReaderError> {
    if interactive && s.lists.current(id) != gen {
        return Err(ReaderError::BadRequest("superseded".into()));
    }
    let pages = scan::build_pages(real);
    let groups = scan::groups(&pages);
    let meta = Arc::new(BookMeta {
        id,
        kind,
        path,
        sig: sig.to_string(),
        pages,
        groups,
    });
    let eff = chapters::effective(&s.db.lock(), id)?;
    let resume = db::load_progress(&s.db.lock(), id)?
        .filter(|p| (p.page as usize) < real.len())
        .or_else(|| {
            let lp = last_page.max(0) as u32;
            if (lp as usize) < real.len() {
                Some(SavedPosition { page: lp, frac: 0.0 })
            } else {
                None
            }
        });
    let out = OpenedBook {
        book_id: id,
        title: title.to_string(),
        sig: sig.to_string(),
        real_pages: real.len() as u32,
        dims: meta.pages.iter().map(|p| [p.w, p.h]).collect(),
        src: meta.pages.iter().map(|p| p.src_page).collect(),
        chapters: chapters::resolve(eff.mode, eff.pages, real),
        folder_chapters: chapters::folder(real),
        chapter_mode: eff.mode,
        chapter_pages: eff.pages,
        chapter_overridden: eff.overridden,
        resume,
    };
    s.extractor.register_book(meta);
    Ok(out)
}

fn serve_cached(
    s: &ReaderState,
    id: i64,
    title: &str,
    kind: SourceKind,
    path: PathBuf,
    sig: &str,
    pages: Vec<RealPage>,
    last_page: i64,
    interactive: bool,
    gen: u64,
    verified: bool,
    dims_ready: bool,
    followup: Option<Followup>,
) -> Result<OpenOutcome, ReaderError> {
    let book = assemble(
        s, id, title, kind, path, sig, &pages, last_page, interactive, gen,
    )?;
    s.lists.put(
        id,
        sig.to_string(),
        pages,
        verified,
        dims_ready,
        followup.is_some(),
    );
    Ok(OpenOutcome { book, followup })
}

/// Interactive open: memory or the stored page list, without walking the folder.
fn try_fast(
    s: &ReaderState,
    id: i64,
    kind: SourceKind,
    row: &db::BookRow,
    path: &Path,
    gen: u64,
) -> Result<Option<OpenOutcome>, ReaderError> {
    let title = row.title.as_str();
    let last_page = row.last_page;
    let anchor = path.to_path_buf();

    if kind == SourceKind::Zip {
        let now = source_sig(kind, &row.paths)?;
        if let Some(hit) = s.lists.get(id) {
            if hit.sig == now && (hit.dims_ready || hit.busy) {
                let book = assemble(
                    s, id, title, kind, anchor, &hit.sig, hit.pages.as_slice(), last_page, true, gen,
                )?;
                return Ok(Some(OpenOutcome { book, followup: None }));
            }
            if hit.sig == now && !hit.dims_ready {
                let names = names_fingerprint(hit.pages.as_slice());
                let followup = Followup::Fill {
                    id,
                    gen,
                    kind,
                    paths: row.paths.clone(),
                    sig: hit.sig.clone(),
                    names,
                };
                let book = assemble(
                    s, id, title, kind, anchor, &hit.sig, hit.pages.as_slice(), last_page, true, gen,
                )?;
                s.lists.put_arc(id, hit.sig, hit.pages, true, false, true);
                return Ok(Some(OpenOutcome {
                    book,
                    followup: Some(followup),
                }));
            }
            s.lists.forget(id);
        }
        if row.scan_sig.as_deref() == Some(now.as_str()) {
            let pages = db::load_pages(&s.db.lock(), id)?;
            if !pages.is_empty() {
                return Ok(Some(serve_cached(
                    s, id, title, kind, anchor, &now, pages, last_page, true, gen, true, true, None,
                )?));
            }
        }
        return Ok(None);
    }

    if kind == SourceKind::Images {
        if let Some(hit) = s.lists.get(id) {
            if hit.dims_ready || hit.busy {
                let book = assemble(
                    s, id, title, kind, anchor, &hit.sig, hit.pages.as_slice(), last_page, true, gen,
                )?;
                return Ok(Some(OpenOutcome { book, followup: None }));
            }
            let names = names_fingerprint(hit.pages.as_slice());
            let followup = Followup::Fill {
                id,
                gen,
                kind,
                paths: row.paths.clone(),
                sig: hit.sig.clone(),
                names,
            };
            let book = assemble(
                s, id, title, kind, anchor, &hit.sig, hit.pages.as_slice(), last_page, true, gen,
            )?;
            s.lists.put_arc(id, hit.sig, hit.pages, true, false, true);
            return Ok(Some(OpenOutcome {
                book,
                followup: Some(followup),
            }));
        }
        if row.scan_sig.as_deref() == Some("i") {
            let pages = db::load_pages(&s.db.lock(), id)?;
            if !pages.is_empty() {
                return Ok(Some(serve_cached(
                    s, id, title, kind, anchor, "i", pages, last_page, true, gen, true, true, None,
                )?));
            }
        }
        return Ok(None);
    }

    // Folder: the signature walk is the slow part, so a stored list is returned
    // first and checked on a background thread.
    if let Some(hit) = s.lists.get(id) {
        let followup = if hit.busy {
            None
        } else if !hit.dims_ready {
            Some(Followup::Fill {
                id,
                gen,
                kind,
                paths: row.paths.clone(),
                sig: hit.sig.clone(),
                names: names_fingerprint(hit.pages.as_slice()),
            })
        } else if !hit.verified {
            Some(Followup::Verify {
                id,
                gen,
                kind,
                paths: row.paths.clone(),
                sig: hit.sig.clone(),
            })
        } else {
            None
        };
        let book = assemble(
            s, id, title, kind, anchor, &hit.sig, hit.pages.as_slice(), last_page, true, gen,
        )?;
        if followup.is_some() {
            s.lists.put_arc(id, hit.sig, hit.pages, hit.verified, hit.dims_ready, true);
        }
        return Ok(Some(OpenOutcome { book, followup }));
    }
    if let Some(stored) = row.scan_sig.clone() {
        let pages = db::load_pages(&s.db.lock(), id)?;
        if !pages.is_empty() {
            let followup = Followup::Verify {
                id,
                gen,
                kind,
                paths: row.paths.clone(),
                sig: stored.clone(),
            };
            return Ok(Some(serve_cached(
                s, id, title, kind, anchor, &stored, pages, last_page, true, gen, false, true,
                Some(followup),
            )?));
        }
    }
    Ok(None)
}

fn open_inner(
    s: &ReaderState,
    id: i64,
    force: bool,
    interactive: bool,
    gen: u64,
) -> Result<OpenOutcome, ReaderError> {
    let t0 = Instant::now();
    let row = db::book_row(&s.db.lock(), id)?;
    let kind = SourceKind::from_db(&row.kind)
        .ok_or_else(|| ReaderError::Unsupported(row.kind.clone()))?;
    // zip/folder: the single source path is paths[0]. images: paths is the page list
    // itself; `path` below is only an anchor (BookMeta.path is unused for Images —
    // the extractor reads PageMeta.name, the absolute per-page path).
    let first = row.paths.first().cloned().unwrap_or_default();
    let anchor = row
        .paths
        .iter()
        .find(|p| PathBuf::from(p).exists())
        .cloned()
        .unwrap_or_else(|| first.clone());
    let path = PathBuf::from(&anchor);
    if kind != SourceKind::Images && !row.paths.iter().any(|p| PathBuf::from(p).exists()) {
        return Err(ReaderError::SourceMissing(first));
    }
    if force {
        db::clear_edited_pages(&s.db.lock(), id)?;
    }
    let edited = if force {
        None
    } else {
        db::load_edited_pages(&s.db.lock(), id)?
    };
    if let Some(pages) = edited {
        s.lists.forget(id);
        db::sync_pages_meta(&s.db.lock(), id, &pages)?;
        let sig = edited_sig(&pages);
        let book = assemble(
            s, id, &row.title, kind, path, &sig, &pages, row.last_page, interactive, gen,
        )?;
        perf_ms(&format!("open book {id} edited"), t0);
        return Ok(OpenOutcome { book, followup: None });
    }

    if interactive {
        if let Some(out) = try_fast(s, id, kind, &row, &path, gen)? {
            perf_ms(&format!("open book {id} cached pages={}", out.book.real_pages), t0);
            return Ok(out);
        }
    } else if !force {
        if let Some(stored) = row.scan_sig.clone() {
            let now = source_sig(kind, &row.paths)?;
            if now == stored {
                let pages = db::load_pages(&s.db.lock(), id)?;
                if !pages.is_empty() {
                    let out = serve_cached(
                        s, id, &row.title, kind, path, &now, pages, row.last_page, false, gen,
                        true, true, None,
                    )?;
                    perf_ms(&format!("open book {id} sync-cache"), t0);
                    return Ok(out);
                }
            }
        }
    }

    let around = resume_around(s, id, row.last_page);
    let indexed = scan::index_for_open(kind, &row.paths, around, !interactive)?;
    if indexed.pages.is_empty() {
        return Err(ReaderError::Corrupt("no pages found".into()));
    }
    let pending = if interactive { indexed.pending } else { 0 };
    // A newer open_book owns this id. Do not purge its cache or publish this list.
    if interactive && s.lists.current(id) != gen {
        return Err(ReaderError::BadRequest("superseded".into()));
    }
    s.cache.purge_book(id);
    if pending == 0 {
        db::store_pages(&mut s.db.lock(), id, &indexed.pages, &indexed.sig)?;
    }
    let followup = if pending > 0 {
        Some(Followup::Fill {
            id,
            gen,
            kind,
            paths: row.paths.clone(),
            sig: indexed.sig.clone(),
            names: names_fingerprint(&indexed.pages),
        })
    } else {
        None
    };
    let book = assemble(
        s, id, &row.title, kind, path, &indexed.sig, &indexed.pages, row.last_page, interactive, gen,
    )?;
    s.lists.put(
        id,
        indexed.sig,
        indexed.pages,
        true,
        pending == 0,
        followup.is_some(),
    );
    perf_ms(
        &format!("open book {id} index pending={pending} pages={}", book.real_pages),
        t0,
    );
    Ok(OpenOutcome { book, followup })
}

pub(crate) fn open_blocking(
    s: &ReaderState,
    id: i64,
    force: bool,
) -> Result<OpenedBook, ReaderError> {
    if force {
        s.lists.invalidate(id);
    }
    Ok(open_inner(s, id, force, false, s.lists.current(id))?.book)
}

fn emit_refresh(app: Option<&AppHandle>, id: i64, sig: &str, pages: u32) {
    let Some(app) = app else { return };
    let _ = app.emit(
        "book-list-refreshed",
        serde_json::json!({ "bookId": id, "sig": sig, "realPages": pages }),
    );
}

fn run_followup(s: &ReaderState, app: Option<&AppHandle>, job: Followup) -> Result<(), ReaderError> {
    let t0 = Instant::now();
    match job {
        Followup::Verify { id, gen, kind, paths, sig } => {
            let now = source_sig(kind, &paths)?;
            if s.lists.current(id) != gen {
                return Ok(());
            }
            if now == sig {
                s.lists.mark_verified(id, gen);
                perf_ms(&format!("verify book {id} unchanged"), t0);
                return Ok(());
            }
            if s.lists.current(id) != gen {
                return Ok(());
            }
            let indexed = scan::index_for_open(kind, &paths, 0, true)?;
            if indexed.pages.is_empty() || s.lists.current(id) != gen {
                return Ok(());
            }
            s.cache.purge_book(id);
            db::store_pages(&mut s.db.lock(), id, &indexed.pages, &indexed.sig)?;
            let n = indexed.pages.len() as u32;
            let sig = indexed.sig.clone();
            if s.lists.finish(id, gen, indexed.sig, indexed.pages, true, true) {
                emit_refresh(app, id, &sig, n);
            }
            perf_ms(&format!("verify book {id} refreshed"), t0);
            Ok(())
        }
        Followup::Fill { id, gen, kind, paths, sig, names } => {
            if s.lists.current(id) != gen {
                return Ok(());
            }
            let indexed = scan::index_for_open(kind, &paths, 0, true)?;
            if indexed.pages.is_empty() || s.lists.current(id) != gen {
                return Ok(());
            }
            s.cache.purge_book(id);
            db::store_pages(&mut s.db.lock(), id, &indexed.pages, &indexed.sig)?;
            let changed = names_fingerprint(&indexed.pages) != names || indexed.sig != sig;
            let n = indexed.pages.len() as u32;
            let new_sig = indexed.sig.clone();
            if s.lists.finish(id, gen, indexed.sig, indexed.pages, true, true) && changed {
                emit_refresh(app, id, &new_sig, n);
            }
            perf_ms(&format!("fill book {id} dims"), t0);
            Ok(())
        }
    }
}

// Tauri v2: JS passes camelCase keys (bookId) -> snake_case args (book_id). async + borrowed State => must return Result.
#[tauri::command]
pub async fn open_book(
    app: AppHandle,
    state: State<'_, ReaderState>,
    book_id: i64,
) -> Result<OpenedBook, ReaderError> {
    let gen = state.lists.bump(book_id);
    let s = state.inner().clone();
    let mut out = blocking(move || open_inner(&s, book_id, false, true, gen)).await?;
    if let Some(job) = out.followup.take() {
        let s = state.inner().clone();
        let app = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(err) = run_followup(&s, Some(&app), job) {
                if std::env::var_os("MH_PERF").is_some() {
                    eprintln!("[mh-perf] followup: {err}");
                }
            }
        });
    }
    Ok(out.book)
}
#[tauri::command]
pub async fn rescan_book(
    state: State<'_, ReaderState>,
    book_id: i64,
) -> Result<OpenedBook, ReaderError> {
    state.lists.invalidate(book_id);
    let s = state.inner().clone();
    blocking(move || open_blocking(&s, book_id, true)).await
}
#[tauri::command]
pub async fn close_book(state: State<'_, ReaderState>, book_id: i64) -> Result<(), ReaderError> {
    state.extractor.close_book(book_id);
    Ok(())
}
#[tauri::command]
pub async fn set_reading_window(
    state: State<'_, ReaderState>,
    book_id: i64,
    center: u32,
    direction: i8,
    ahead: u32,
    behind: u32,
    variant: u16,
) -> Result<(), ReaderError> {
    if !VARIANTS.contains(&variant) {
        return Err(ReaderError::BadRequest("variant".into()));
    }
    state.extractor.set_window(Window {
        book: book_id,
        center,
        dir: direction.signum().max(-1).min(1),
        ahead: ahead.min(12),
        behind: behind.min(4),
        variant,
    })
}
#[tauri::command]
pub async fn save_progress(
    state: State<'_, ReaderState>,
    book_id: i64,
    pos: SavedPosition,
    reached_end: bool,
) -> Result<(), ReaderError> {
    let s = state.inner().clone();
    blocking(move || db::save_progress(&s.db.lock(), book_id, pos, reached_end)).await
}
#[tauri::command]
pub async fn load_progress(
    state: State<'_, ReaderState>,
    book_id: i64,
) -> Result<Option<SavedPosition>, ReaderError> {
    let s = state.inner().clone();
    blocking(move || db::load_progress(&s.db.lock(), book_id)).await
}
#[tauri::command]
pub async fn fix_page_dims(
    state: State<'_, ReaderState>,
    book_id: i64,
    fixes: Vec<DimFix>,
) -> Result<(), ReaderError> {
    let s = state.inner().clone();
    blocking(move || {
        db::fix_dims(&mut s.db.lock(), book_id, &fixes)?;
        s.cache.purge_book(book_id); // slicing may differ next open (virtual indices shift) -> cached slices are stale
        // The in-memory page list still has the old header sizes.
        s.lists.invalidate(book_id);
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn cache_stats(state: State<'_, ReaderState>) -> Result<CacheStats, ReaderError> {
    Ok(state.cache.stats())
}
#[tauri::command]
pub async fn cache_set_limit(state: State<'_, ReaderState>, bytes: u64) -> Result<(), ReaderError> {
    state.cache.set_limit(bytes.max(64 << 20));
    let win = state.extractor.window();
    state
        .cache
        .evict_if_needed(&|k| win.map_or(false, |w| w.pins(k)));
    Ok(())
}
#[tauri::command]
pub async fn cache_clear(
    state: State<'_, ReaderState>,
    book_id: Option<i64>,
) -> Result<u64, ReaderError> {
    let s = state.inner().clone();
    blocking(move || {
        Ok(match book_id {
            Some(id) => s.cache.purge_book(id),
            None => {
                let win = s.extractor.window();
                let n = s
                    .cache
                    .clear(&|k| win.map_or(false, |w| w.pins(k)));
                // v0.3 `zip_extract_page` wrote data/page_cache. The reader no longer
                // uses that folder; a full clear removes it along with the page cache.
                if let Ok(dir) = crate::data_dir() {
                    let legacy = dir.join("page_cache");
                    if legacy.is_dir() {
                        let _ = std::fs::remove_dir_all(legacy);
                    }
                }
                n
            }
        })
    })
    .await
}

#[cfg(test)]
mod open_speed {
    use super::{open_blocking, open_inner, run_followup, Db, Followup, ReaderState};
    use crate::extractor::Extractor;
    use std::io::Cursor;
    use std::sync::Arc;
    use std::time::Instant;

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 90])
        });
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Jpeg)
            .unwrap();
        buf
    }

    fn fresh_state(db: Db, dir: &std::path::Path) -> ReaderState {
        let cache = Arc::new(
            crate::cache::PageCache::open(dir.to_path_buf(), 32 << 20).unwrap(),
        );
        let ex = Extractor::start(cache.clone(), 1);
        ReaderState::new(db, cache, ex)
    }

    #[test]
    fn interactive_open_paints_before_the_tail_is_probed() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("mh-open-speed-{stamp}"));
        let folder = dir.join("book");
        std::fs::create_dir_all(&folder).unwrap();
        let jpeg = jpeg(16, 24);
        for i in 1..=40 {
            std::fs::write(folder.join(format!("{i:03}.jpg")), &jpeg).unwrap();
        }
        std::fs::write(folder.join("notes.txt"), b"not a page").unwrap();

        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE shelf(
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                paths TEXT NOT NULL,
                page_count INTEGER NOT NULL DEFAULT 0,
                last_page INTEGER NOT NULL DEFAULT 0,
                scan_sig TEXT
            );",
        )
        .unwrap();
        conn.execute_batch(crate::db::DDL).unwrap();
        conn.execute_batch(crate::db::EDITED_DDL).unwrap();
        conn.execute_batch(crate::chapters::DDL).unwrap();
        let paths = serde_json::to_string(&vec![folder.to_string_lossy().to_string()]).unwrap();
        conn.execute(
            "INSERT INTO shelf(id, name, kind, paths) VALUES(1, 'speed', 'folder', ?1)",
            [paths],
        )
        .unwrap();
        let db: Db = Arc::new(parking_lot::Mutex::new(conn));
        let state = fresh_state(db.clone(), &dir.join("cache-a"));

        let gen = state.lists.bump(1);
        let t_cold = Instant::now();
        let mut cold = open_inner(&state, 1, false, true, gen).unwrap();
        let cold_us = t_cold.elapsed().as_micros();
        assert_eq!(cold.book.real_pages, 40);
        assert!(cold.book.dims[0][0] > 0 && cold.book.dims[0][1] > 0);
        assert_eq!(cold.book.dims[35], [0, 0], "tail stays unprobed on the first return");
        assert!(matches!(cold.followup, Some(Followup::Fill { .. })));
        let job = cold.followup.take().unwrap();

        let t_fill = Instant::now();
        run_followup(&state, None, job).unwrap();
        let fill_us = t_fill.elapsed().as_micros();

        let gen2 = state.lists.bump(1);
        let t_mem = Instant::now();
        let again = open_inner(&state, 1, false, true, gen2).unwrap();
        let mem_us = t_mem.elapsed().as_micros();
        assert!(again.followup.is_none(), "a finished list does not schedule more work");
        assert!(
            again.book.dims.iter().all(|d| d[0] > 0 && d[1] > 0),
            "fill wrote every header: {:?}",
            again.book.dims[35]
        );

        let state_db = fresh_state(db.clone(), &dir.join("cache-b"));
        let gen3 = state_db.lists.bump(1);
        let t_db = Instant::now();
        let from_db = open_inner(&state_db, 1, false, true, gen3).unwrap();
        let db_us = t_db.elapsed().as_micros();
        assert!(matches!(from_db.followup, Some(Followup::Verify { .. })));
        assert!(from_db.book.dims[35][0] > 0, "stored pages return with real sizes");

        let state_sync = fresh_state(db, &dir.join("cache-c"));
        let t_sync = Instant::now();
        let blocked = open_blocking(&state_sync, 1, false).unwrap();
        let sync_us = t_sync.elapsed().as_micros();
        assert_eq!(blocked.real_pages, 40);
        assert!(blocked.dims[35][0] > 0);

        eprintln!(
            "BENCH open cold_us={cold_us} fill_us={fill_us} memory_us={mem_us} db_us={db_us} sync_walk_us={sync_us}"
        );
        if sync_us > db_us.saturating_add(2_000) {
            assert!(
                db_us * 2 < sync_us,
                "cached open {db_us}us should beat the signature walk {sync_us}us"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
