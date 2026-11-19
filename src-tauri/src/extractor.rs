//! Prioritised, cancellable extraction queue
//! (split from design/claude-v04/reader_backend.rs `pub mod extractor`, unchanged).

use crate::cache::*;
use crate::error::ReaderError;
use crate::model::*;
use crate::scan;
use parking_lot::{Condvar, Mutex};
use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap},
    fs::File,
    io::{BufReader, Cursor, Read},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst},
        Arc,
    },
};

type Res = Result<CachedPage, ReaderError>;
type ZipHandle = Option<(i64, zip::ZipArchive<BufReader<File>>)>; // per-worker open archive

/// What the reader currently needs. Set by `set_reading_window`.
#[derive(Clone, Copy, Debug)]
pub struct Window {
    pub book: i64,
    pub center: u32,
    pub dir: i8,
    pub ahead: u32,
    pub behind: u32,
    pub variant: u16,
}
impl Window {
    fn range(&self, n: u32) -> (u32, u32) {
        (
            self.center.saturating_sub(self.behind),
            (self.center + self.ahead).min(n.saturating_sub(1)),
        )
    }
    /// Lower = sooner. Pages ahead in the reading direction are cheap, pages behind cost extra,
    /// so the 1st page behind (7) still beats the 4th page ahead (8).
    fn cost(&self, idx: u32) -> u32 {
        let d = (idx as i64 - self.center as i64) * if self.dir < 0 { -1 } else { 1 };
        if d >= 0 {
            (d * 2) as u32
        } else {
            ((-d) * 2 + 5) as u32
        }
    }
    /// Never evict these (window +-2 extra pages of slack).
    pub fn pins(&self, k: &Key) -> bool {
        k.0 == self.book
            && k.1 + self.behind + 2 >= self.center
            && k.1 <= self.center + self.ahead + 2
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    Prefetch = 0,
    Demand = 1,
}
#[derive(PartialEq, Eq)]
struct Job {
    key: Key,
    class: Class,
    cost: u32,
    seq: u64,
}
impl Ord for Job {
    // BinaryHeap pops the GREATEST: Demand first, then lowest cost, then oldest
    fn cmp(&self, o: &Self) -> Ordering {
        self.class
            .cmp(&o.class)
            .then(o.cost.cmp(&self.cost))
            .then(o.seq.cmp(&self.seq))
    }
}
impl PartialOrd for Job {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// One per queued/in-flight key: lets any number of waiters (protocol handlers) join the same job.
struct Slot {
    done: Mutex<Option<Res>>,
    cv: Condvar,
    started: AtomicBool,
    demanded: AtomicBool,
}
impl Slot {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            done: Mutex::new(None),
            cv: Condvar::new(),
            started: AtomicBool::new(false),
            demanded: AtomicBool::new(false),
        })
    }
}

struct State {
    win: Option<Window>,
    heap: BinaryHeap<Job>,
    slots: HashMap<Key, Arc<Slot>>,
    books: HashMap<i64, Arc<BookMeta>>,
    seq: u64,
    shutdown: bool,
}
pub struct Extractor {
    st: Mutex<State>,
    cv: Condvar,
    pub cache: Arc<PageCache>,
}

/// Parallel extract/decode workers. v0.7.6 always started 2, so ZIP inflate and the
/// variant/slice decode path queued behind one pair of threads. Follow the machine,
/// but stay inside 2..=4: several full-size pages decoding at once is enough RAM.
pub fn worker_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(2, 4)
}

impl Extractor {
    pub fn start(cache: Arc<PageCache>, workers: usize) -> Arc<Self> {
        let ex = Arc::new(Self {
            cache,
            cv: Condvar::new(),
            st: Mutex::new(State {
                win: None,
                heap: BinaryHeap::new(),
                slots: HashMap::new(),
                books: HashMap::new(),
                seq: 0,
                shutdown: false,
            }),
        });
        for n in 0..workers {
            let w = ex.clone();
            std::thread::Builder::new()
                .name(format!("mh-extract-{n}"))
                .spawn(move || w.worker())
                .expect("spawn");
        }
        ex
    }
    pub fn shutdown(&self) {
        self.st.lock().shutdown = true;
        self.cv.notify_all();
        self.cache.flush_touches();
    }

    pub fn register_book(&self, meta: Arc<BookMeta>) {
        {
            let mut st = self.st.lock();
            st.books.insert(meta.id, meta.clone());
            while st.books.len() > 2 {
                // keep current + previous only
                let victim = st
                    .books
                    .keys()
                    .copied()
                    .find(|k| *k != meta.id && Some(*k) != st.win.map(|w| w.book));
                match victim {
                    Some(v) => {
                        st.books.remove(&v);
                    }
                    None => break,
                }
            }
        }
        // Old full-size copies of folder/loose originals are indexed cache files.
        // Drop them once the original file is still there. The open call returns
        // before the view requests pages, so nothing is reading those copies yet.
        self.retire_direct_duplicates(&meta);
    }
    pub fn book(&self, id: i64) -> Option<Arc<BookMeta>> {
        self.st.lock().books.get(&id).cloned()
    }
    pub fn window(&self) -> Option<Window> {
        self.st.lock().win
    }

    pub fn close_book(&self, id: i64) {
        crate::pipeline::scheduler().cancel_book(id);
        let mut st = self.st.lock();
        st.books.remove(&id);
        if st.win.map(|w| w.book) == Some(id) {
            st.win = None;
        }
        let jobs: Vec<Job> = st.heap.drain().collect();
        for j in jobs {
            if j.key.0 == id {
                Self::cancel(&mut st, j.key);
            } else {
                st.heap.push(j);
            }
        }
    }

    fn cancel(st: &mut State, key: Key) {
        if let Some(slot) = st.slots.remove(&key) {
            if !slot.started.load(SeqCst) {
                // in-flight work is allowed to finish (it's cached for later)
                *slot.done.lock() = Some(Err(ReaderError::Cancelled));
                slot.cv.notify_all();
            }
        }
    }

    /// Re-aim the queue. Called on scroll (throttled) and immediately on jumps.
    pub fn set_window(&self, w: Window) -> Result<(), ReaderError> {
        let mut st = self.st.lock();
        let n = st
            .books
            .get(&w.book)
            .ok_or(ReaderError::BookNotOpen)?
            .pages
            .len() as u32;
        if n == 0 {
            return Ok(());
        }
        st.win = Some(w);
        crate::pipeline::note_visible(w.book);
        let (lo, hi) = w.range(n);
        let book = st.books.get(&w.book).cloned();
        // Snapshot once. Prefetch must not keep reading after the disk budget is full.
        // A page the view actually requests still goes through `get_or_extract`.
        let room = self.cache.has_room();

        // 1) re-prioritise and cancel: drain the (small) heap, keep what is still wanted
        let old: Vec<Job> = st.heap.drain().collect();
        for mut j in old {
            let other_variant = j.key.2 != w.variant; // e.g. scrubber thumbnails: never auto-cancelled
            let inside = j.key.0 == w.book && (lo..=hi).contains(&j.key.1);
            // An <img> already waiting on this slot must not be cancelled into a 503.
            let demanded = st
                .slots
                .get(&j.key)
                .map(|slot| slot.demanded.load(SeqCst))
                .unwrap_or(false);
            if inside || other_variant || demanded {
                j.cost = w.cost(j.key.1);
                st.heap.push(j);
            } else {
                Self::cancel(&mut st, j.key);
            } // outside window and not yet displayed: waiters get Cancelled (-> HTTP 503)
        }
        // 2) enqueue what is missing. Folder and loose originals at variant 0 are
        // read from their own files, so prefetch must not pull those bytes in.
        for idx in lo..=hi {
            let key = (w.book, idx, w.variant);
            if st.slots.contains_key(&key) || self.cache.contains(&key) {
                continue;
            }
            if let Some(meta) = book.as_ref() {
                let page = meta.pages.get(idx as usize).cloned();
                if page
                    .as_ref()
                    .is_some_and(|page| direct_source(meta, page, w.variant).is_some())
                {
                    continue;
                }
            }
            if !room {
                continue;
            }
            st.slots.insert(key, Slot::new());
            st.seq += 1;
            let seq = st.seq;
            st.heap.push(Job {
                key,
                class: Class::Prefetch,
                cost: w.cost(idx),
                seq,
            });
        }
        drop(st);
        self.cv.notify_all();
        Ok(())
    }

    /// BLOCKING. Used by the protocol handler: cache hit, or join/promote the job and wait for it.
    pub fn get_or_extract(&self, key: Key) -> Res {
        if let Some(hit) = self.direct_hit(key) {
            return Ok(hit);
        }
        if let Some(p) = self.cache.get(&key) {
            return Ok(p);
        }
        let slot = {
            let mut st = self.st.lock();
            let n = st
                .books
                .get(&key.0)
                .ok_or(ReaderError::BookNotOpen)?
                .pages
                .len() as u32;
            if key.1 >= n {
                return Err(ReaderError::PageOutOfRange);
            }
            let slot = st.slots.entry(key).or_insert_with(Slot::new).clone();
            slot.demanded.store(true, SeqCst);
            if !slot.started.load(SeqCst) {
                // (re)queue as Demand; a stale Prefetch entry is skipped by `started`
                let cost = st.win.map(|w| w.cost(key.1)).unwrap_or(0);
                st.seq += 1;
                let seq = st.seq;
                st.heap.push(Job {
                    key,
                    class: Class::Demand,
                    cost,
                    seq,
                });
                self.cv.notify_one();
            }
            slot
        };
        let mut g = slot.done.lock();
        while g.is_none() {
            slot.cv.wait(&mut g);
        }
        g.clone().unwrap()
    }

    fn worker(self: Arc<Self>) {
        let mut zip: ZipHandle = None;
        let mut last_flush = std::time::Instant::now();
        loop {
            let (job, slot, book) = {
                let mut st = self.st.lock();
                loop {
                    if st.shutdown {
                        return;
                    }
                    if let Some(job) = st.heap.pop() {
                        let Some(slot) = st.slots.get(&job.key).cloned() else {
                            continue;
                        }; // cancelled
                        if slot.started.swap(true, SeqCst) {
                            continue;
                        } // duplicate heap entry
                        let Some(book) = st.books.get(&job.key.0).cloned() else {
                            st.slots.remove(&job.key);
                            continue;
                        };
                        break (job, slot, book);
                    }
                    self.cv.wait(&mut st);
                }
            };
            let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.process(job.key, &slot, &book, &mut zip)
            }));
            let res = match panicked {
                Ok(r) => r,
                Err(_) => {
                    zip = None;
                    Err(ReaderError::Corrupt("该页异常，已跳过".into()))
                }
            };
            if let Err(err) = &res {
                if matches!(
                    err,
                    ReaderError::Corrupt(_)
                        | ReaderError::Unsupported(_)
                        | ReaderError::Io(_)
                        | ReaderError::SourceMissing(_)
                ) {
                    let page = book
                        .pages
                        .get(job.key.1 as usize)
                        .map(|p| p.name.as_str())
                        .unwrap_or("");
                    let path = if page.is_empty() {
                        book.path.display().to_string()
                    } else {
                        format!("{} / {page}", book.path.display())
                    };
                    crate::shelf_scan::note_reader_trouble(&path, &err.to_string());
                }
            }
            self.st.lock().slots.remove(&job.key);
            *slot.done.lock() = Some(res);
            slot.cv.notify_all();

            let win = self.window();
            self.cache
                .evict_if_needed(&|k| win.map_or(false, |w| w.pins(k))); // never holds st while in cache lock
            if last_flush.elapsed().as_secs() >= 30 {
                self.cache.flush_touches();
                last_flush = std::time::Instant::now();
            }
        }
    }

    fn still_wanted(&self, key: Key) -> bool {
        match self.window() {
            Some(w) => key.2 != w.variant || w.pins(&key),
            None => false,
        }
    }

    fn direct_hit(&self, key: Key) -> Option<CachedPage> {
        let book = self.book(key.0)?;
        let page = book.pages.get(key.1 as usize)?;
        direct_page(&book, page, key.2)
    }

    fn retire_direct_duplicates(&self, book: &BookMeta) {
        for (i, page) in book.pages.iter().enumerate() {
            let key = (book.id, i as u32, 0);
            let Some(ext) = self.cache.stored_ext(&key) else {
                continue;
            };
            let Some((src, _)) = direct_source(book, page, 0) else {
                continue;
            };
            let Some(dest) = self.cache.owned_path(&key, ext) else {
                // The indexed path is not a file this cache may delete.
                self.cache.forget(&key);
                continue;
            };
            if same_file(&dest, &src) {
                // The index points at the original itself. Drop the row only.
                self.cache.forget(&key);
                continue;
            }
            self.cache.remove(&key);
        }
    }

    fn process(&self, key: Key, slot: &Slot, book: &BookMeta, zip: &mut ZipHandle) -> Res {
        if let Some(page) = book.pages.get(key.1 as usize) {
            if let Some(hit) = direct_page(book, page, key.2) {
                return Ok(hit);
            }
        }
        if let Some(p) = self.cache.get(&key) {
            return Ok(p);
        } // sibling slice already produced it
        let demanded = slot.demanded.load(SeqCst);
        // Prefetch that the reader already left, or that would grow past the
        // disk budget, must not read the source into memory.
        if !demanded && (!self.still_wanted(key) || !self.cache.has_room()) {
            return Err(ReaderError::Cancelled);
        }
        // Page reads outrank thumbnail generation and folder/archive scans.
        let _fg = crate::pipeline::enter_foreground();
        let (_, idx, variant) = key;
        let page = book
            .pages
            .get(idx as usize)
            .ok_or(ReaderError::PageOutOfRange)?;
        let raw = read_entry(book, page, zip)?; // phase 1: bytes out of the source
        if !demanded && !self.still_wanted(key) {
            return Err(ReaderError::Cancelled);
        } // phase check: jumped away

        if variant == 0 && !page.is_slice() {
            if crate::pipeline::should_downscale(page.w, page.h, raw.len()) {
                let _decode_slot = acquire_decode_slot();
                let img = crate::pipeline::downscale_huge(decode(&raw)?);
                let (bytes, ext) = encode(&img)?;
                return self.store(key, ext, &bytes);
            }
            // fast path: store as-is. The protocol serves these bytes; nothing is base64.
            let (_, ext) =
                sniff(&raw).ok_or_else(|| ReaderError::Corrupt("not a recognised image".into()))?;
            return self.store(key, ext, &raw);
        }
        // decode path: slices and/or width variants. Decode ONCE, emit every slice of the image.
        if page.w as u64 * page.full_h as u64 > scan::MAX_PIXELS {
            return Err(ReaderError::Unsupported("image too large".into()));
        }
        // At most two full decodes at once, so four workers cannot each hold a page bitmap.
        let _decode_slot = acquire_decode_slot();
        let img = decode(&raw)?;
        // A normal page has nothing to crop. Skip the full-buffer clone the old
        // loop did before shrink_to.
        if !page.is_slice() {
            let part = shrink_to(img, variant);
            let (bytes, ext) = encode(&part)?;
            return self.store(key, ext, &bytes);
        }
        let (first, last) = book.groups[page.src_page as usize];
        let mut wanted = None;
        for vi in first..=last {
            let k = (book.id, vi, variant);
            let p = &book.pages[vi as usize];
            if vi != idx && self.cache.contains(&k) {
                continue;
            }
            let part = shrink_to(img.crop_imm(0, p.y0, img.width(), p.h), variant);
            let (bytes, ext) = encode(&part)?;
            let cp = self.store(k, ext, &bytes)?;
            if vi == idx {
                wanted = Some(cp);
            }
        }
        wanted.ok_or(ReaderError::PageOutOfRange)
    }

    fn store(&self, key: Key, ext: &'static str, bytes: &[u8]) -> Res {
        let path = self.cache.prepare_owned_write(&key, ext)?;
        let tmp = path.with_extension("part");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?; // atomic: readers never see half a file
        Ok(self.cache.insert(key, ext, bytes.len() as u64))
    }

    #[cfg(test)]
    fn idle(&self) -> bool {
        let st = self.st.lock();
        st.heap.is_empty() && st.slots.is_empty()
    }
}

/// Variant 0 of a folder page, a loose image, or an appended file can be served
/// from the original when it is not a slice and not a downscale. The returned
/// path is never written into the cache index.
fn direct_source(
    book: &BookMeta,
    page: &PageMeta,
    variant: u16,
) -> Option<(PathBuf, &'static str)> {
    if variant != 0 || page.is_slice() {
        return None;
    }
    let path = if let Some(p) = &page.src_path {
        PathBuf::from(p)
    } else {
        match book.kind {
            SourceKind::Folder => book.path.join(&page.name),
            SourceKind::Images => PathBuf::from(&page.name),
            SourceKind::Zip => return None,
        }
    };
    let meta = std::fs::symlink_metadata(&path).ok()?;
    let len = if meta.file_type().is_symlink() {
        let followed = std::fs::metadata(&path).ok()?;
        if !followed.is_file() {
            return None;
        }
        followed.len()
    } else if meta.is_file() {
        meta.len()
    } else {
        return None;
    };
    if crate::pipeline::should_downscale(page.w, page.h, len as usize) {
        return None;
    }
    let mime = sniff_file(&path)?;
    Some((path, mime))
}

fn direct_page(book: &BookMeta, page: &PageMeta, variant: u16) -> Option<CachedPage> {
    let (path, mime) = direct_source(book, page, variant)?;
    let bytes = std::fs::metadata(&path).ok()?.len();
    Some(CachedPage { path, mime, bytes })
}

fn sniff_file(path: &Path) -> Option<&'static str> {
    let mut f = File::open(path).ok()?;
    let mut buf = [0u8; 16];
    let n = f.read(&mut buf).ok()?;
    sniff(&buf[..n]).map(|(mime, _)| mime)
}

fn same_file(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    let Ok(ma) = std::fs::metadata(a) else {
        return false;
    };
    let Ok(mb) = std::fs::metadata(b) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        return ma.dev() == mb.dev() && ma.ino() == mb.ino();
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        return ma.volume_serial_number() == mb.volume_serial_number()
            && ma.file_index() == mb.file_index();
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (ma, mb);
        false
    }
}

fn read_entry(
    book: &BookMeta,
    page: &PageMeta,
    zip: &mut ZipHandle,
) -> Result<Vec<u8>, ReaderError> {
    // Appended/merged pages carry their own source file (see model::PageMeta).
    if let Some(p) = &page.src_path {
        return crate::pipeline::read_file_capped(std::path::Path::new(p));
    }
    match book.kind {
        SourceKind::Zip => {
            if zip.as_ref().map(|(id, _)| *id) != Some(book.id) {
                *zip = Some((book.id, scan::open_zip(&book.path)?));
            }
            let read = {
                let ar = &mut zip.as_mut().unwrap().1;
                match ar.by_index(page.entry as usize) {
                    Ok(mut e) => {
                        let claimed = e.size();
                        crate::pipeline::read_capped(&mut e, claimed, scan::MAX_ENTRY)
                    }
                    Err(err) => Err(err.into()),
                }
            };
            if read.is_err() {
                *zip = None;
            }
            read
        }
        SourceKind::Folder => crate::pipeline::read_file_capped(&book.path.join(&page.name)),
        SourceKind::Images => crate::pipeline::read_file_capped(std::path::Path::new(&page.name)),
    }
}

/// Trust the BYTES, not the file extension.
pub fn sniff(b: &[u8]) -> Option<(&'static str, &'static str)> {
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(("image/jpeg", "jpg"))
    } else if b.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some(("image/png", "png"))
    } else if b.starts_with(b"GIF8") {
        Some(("image/gif", "gif"))
    } else if b.len() > 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some(("image/webp", "webp"))
    // AVIF is served as-is (variant 0). Slices and width variants call `decode`,
    // which uses the `image` crate — built without the avif feature — so those paths fail.
    } else if b.len() > 12 && &b[4..8] == b"ftyp" && (&b[8..12] == b"avif" || &b[8..12] == b"avis")
    {
        Some(("image/avif", "avif"))
    } else if b.starts_with(b"BM") {
        Some(("image/bmp", "bmp"))
    } else {
        None
    }
}

const DECODE_SLOTS: usize = 2;
static DECODE_LIVE: AtomicUsize = AtomicUsize::new(0);

fn decode_gate() -> &'static (Mutex<()>, Condvar) {
    static GATE: std::sync::OnceLock<(Mutex<()>, Condvar)> = std::sync::OnceLock::new();
    GATE.get_or_init(|| (Mutex::new(()), Condvar::new()))
}

struct DecodeSlot;
impl Drop for DecodeSlot {
    fn drop(&mut self) {
        DECODE_LIVE.fetch_sub(1, SeqCst);
        decode_gate().1.notify_one();
    }
}

fn acquire_decode_slot() -> DecodeSlot {
    let (lock, cv) = decode_gate();
    let mut guard = lock.lock();
    while DECODE_LIVE.load(SeqCst) >= DECODE_SLOTS {
        cv.wait(&mut guard);
    }
    DECODE_LIVE.fetch_add(1, SeqCst);
    DecodeSlot
}

fn decode(raw: &[u8]) -> Result<image::DynamicImage, ReaderError> {
    crate::pipeline::decode_limited(raw)
}
fn shrink_to(img: image::DynamicImage, variant: u16) -> image::DynamicImage {
    if variant == 0 || img.width() <= variant as u32 {
        return img;
    }
    let nh = ((img.height() as u64 * variant as u64) / img.width() as u64).max(1) as u32;
    // Lanczos3 dominated every saver page and thumbnail. CatmullRom is the
    // reading-size resample; tiny thumbs (v=160) use the faster triangle filter.
    let filter = if variant <= 320 {
        image::imageops::FilterType::Triangle
    } else {
        image::imageops::FilterType::CatmullRom
    };
    img.resize_exact(variant as u32, nh, filter)
}
/// JPEG q92 normally; PNG if the source has alpha (keeps transparency, lossless).
fn encode(img: &image::DynamicImage) -> Result<(Vec<u8>, &'static str), ReaderError> {
    let mut out = Vec::new();
    if img.color().has_alpha() {
        img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)?;
        Ok((out, "png"))
    } else {
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92);
        image::DynamicImage::ImageRgb8(img.to_rgb8()).write_with_encoder(enc)?;
        Ok((out, "jpg"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{PageCache, DEFAULT_CACHE_LIMIT};
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    fn scratch(label: &str) -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("mh-read-cache-{label}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let mut img = image::RgbImage::new(w, h);
        for (x, y, px) in img.enumerate_pixels_mut() {
            *px = image::Rgb([((x * 13 + y) % 251) as u8, (y % 251) as u8, 40]);
        }
        let mut out = Vec::new();
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90);
        image::DynamicImage::ImageRgb8(img)
            .write_with_encoder(enc)
            .unwrap();
        out
    }

    fn page(name: &str, w: u32, h: u32) -> PageMeta {
        PageMeta {
            src_page: 0,
            entry: 0,
            name: name.to_string(),
            w,
            h,
            full_h: h,
            y0: 0,
            src_path: None,
        }
    }

    fn book(id: i64, kind: SourceKind, path: PathBuf, pages: Vec<PageMeta>) -> Arc<BookMeta> {
        let groups = if pages.iter().any(|p| p.is_slice()) {
            vec![(0, pages.len().saturating_sub(1) as u32)]
        } else {
            (0..pages.len() as u32).map(|i| (i, i)).collect()
        };
        Arc::new(BookMeta {
            id,
            kind,
            path,
            sig: "t".into(),
            pages,
            groups,
        })
    }

    struct Stop(Arc<Extractor>);
    impl Drop for Stop {
        fn drop(&mut self) {
            self.0.shutdown();
            crate::pipeline::note_visible(0);
            std::thread::sleep(Duration::from_millis(40));
        }
    }
    impl std::ops::Deref for Stop {
        type Target = Extractor;
        fn deref(&self) -> &Extractor {
            &self.0
        }
    }

    fn boot(dir: &Path, limit: u64) -> (Arc<PageCache>, Stop) {
        let cache = Arc::new(PageCache::open(dir.join("cache"), limit).unwrap());
        let ex = Extractor::start(cache.clone(), 2);
        (cache, Stop(ex))
    }

    fn settle(ex: &Extractor) {
        let start = Instant::now();
        while !ex.idle() {
            if start.elapsed() > Duration::from_secs(8) {
                panic!("extractor did not go idle");
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    fn names_containing(dir: &Path, needle: &str) -> Vec<String> {
        let mut out = Vec::new();
        let Ok(rd) = std::fs::read_dir(dir) else {
            return out;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(names_containing(&path, needle));
            } else if entry.file_name().to_string_lossy().contains(needle) {
                out.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        out
    }

    #[test]
    fn direct_source_is_only_a_real_full_size_original() {
        let dir = scratch("decide");
        let folder = dir.join("book");
        std::fs::create_dir_all(&folder).unwrap();
        let bytes = jpeg(12, 12);
        std::fs::write(folder.join("a.jpg"), &bytes).unwrap();
        let meta = book(
            1,
            SourceKind::Folder,
            folder.clone(),
            vec![page("a.jpg", 12, 12)],
        );
        assert!(direct_source(&meta, &meta.pages[0], 0).is_some());
        assert!(direct_source(&meta, &meta.pages[0], 160).is_none());

        let mut sliced = page("a.jpg", 12, 6);
        sliced.full_h = 12;
        assert!(sliced.is_slice());
        assert!(direct_source(&meta, &sliced, 0).is_none());

        let mut huge = page("a.jpg", 20_000, 20_000);
        huge.full_h = 20_000;
        assert!(direct_source(&meta, &huge, 0).is_none());

        let zip = book(
            1,
            SourceKind::Zip,
            dir.join("a.zip"),
            vec![page("a.jpg", 12, 12)],
        );
        assert!(direct_source(&zip, &zip.pages[0], 0).is_none());

        let loose = dir.join("loose.jpg");
        std::fs::write(&loose, &bytes).unwrap();
        let mut img_page = page(&loose.display().to_string(), 12, 12);
        let images = book(1, SourceKind::Images, dir.clone(), vec![img_page.clone()]);
        assert_eq!(
            direct_source(&images, &images.pages[0], 0).unwrap().0,
            loose
        );
        img_page.src_path = Some(loose.display().to_string());
        let with_src = book(1, SourceKind::Zip, dir.join("missing.zip"), vec![img_page]);
        assert_eq!(
            direct_source(&with_src, &with_src.pages[0], 0).unwrap().0,
            loose
        );
        std::fs::remove_file(&loose).unwrap();
        assert!(direct_source(&images, &images.pages[0], 0).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folder_read_does_not_write_a_full_size_copy() {
        let dir = scratch("folder");
        let folder = dir.join("book");
        std::fs::create_dir_all(&folder).unwrap();
        let marker = "MHORIG_9f3a.jpg";
        let bytes = jpeg(20, 16);
        let original = folder.join(marker);
        std::fs::write(&original, &bytes).unwrap();
        std::fs::write(folder.join("notes.txt"), b"keep-note").unwrap();
        let expanded = dir.join("expanded").join("unpacked");
        std::fs::create_dir_all(&expanded).unwrap();
        std::fs::write(expanded.join("p.jpg"), b"expanded-copy").unwrap();
        let (cache, ex) = boot(&dir, DEFAULT_CACHE_LIMIT);
        ex.register_book(book(
            0,
            SourceKind::Folder,
            folder.clone(),
            vec![page(marker, 20, 16)],
        ));
        ex.set_window(Window {
            book: 0,
            center: 0,
            dir: 1,
            ahead: 8,
            behind: 4,
            variant: 0,
        })
        .unwrap();
        settle(&ex);
        std::thread::sleep(Duration::from_millis(80));
        let hit = ex.get_or_extract((0, 0, 0)).unwrap();
        assert_eq!(hit.path, original);
        assert_eq!(std::fs::read(&hit.path).unwrap(), bytes);
        assert!(names_containing(&dir.join("cache"), ".0.").is_empty());
        assert_eq!(cache.stats().entries, 0);
        assert!(ex.idle());
        cache.clear(&|_| false);
        ex.close_book(0);
        assert_eq!(std::fs::read(&original).unwrap(), bytes);
        assert_eq!(
            std::fs::read(folder.join("notes.txt")).unwrap(),
            b"keep-note"
        );
        assert_eq!(
            std::fs::read(expanded.join("p.jpg")).unwrap(),
            b"expanded-copy"
        );
        let blob = std::fs::read(dir.join("cache").join("cache.db")).unwrap_or_default();
        assert!(!String::from_utf8_lossy(&blob).contains("MHORIG_9f3a"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folder_prefetch_of_many_pages_writes_nothing() {
        let dir = scratch("prefetch");
        let folder = dir.join("book");
        std::fs::create_dir_all(&folder).unwrap();
        let mut pages = Vec::new();
        let mut originals = Vec::new();
        for i in 0..5 {
            let name = format!("p{i}.jpg");
            let bytes = jpeg(18, 14);
            let path = folder.join(&name);
            std::fs::write(&path, &bytes).unwrap();
            originals.push((path, bytes));
            let mut p = page(&name, 18, 14);
            p.src_page = i;
            pages.push(p);
        }
        let (cache, ex) = boot(&dir, DEFAULT_CACHE_LIMIT);
        ex.register_book(book(0, SourceKind::Folder, folder, pages));
        ex.set_window(Window {
            book: 0,
            center: 0,
            dir: 1,
            ahead: 4,
            behind: 0,
            variant: 0,
        })
        .unwrap();
        settle(&ex);
        std::thread::sleep(Duration::from_millis(100));
        assert!(ex.idle());
        assert_eq!(cache.stats().entries, 0);
        assert!(names_containing(&dir.join("cache"), ".0.").is_empty());
        for (path, bytes) in originals {
            assert_eq!(std::fs::read(path).unwrap(), bytes);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn thumbs_slices_and_huge_pages_still_cache_a_derived_file() {
        let dir = scratch("derived");
        let folder = dir.join("book");
        std::fs::create_dir_all(&folder).unwrap();
        let full = jpeg(32, 32);
        let original = folder.join("tall.jpg");
        std::fs::write(&original, &full).unwrap();
        let (cache, ex) = boot(&dir, DEFAULT_CACHE_LIMIT);
        ex.register_book(book(
            3,
            SourceKind::Folder,
            folder.clone(),
            vec![page("tall.jpg", 32, 32)],
        ));
        let thumb = ex.get_or_extract((3, 0, 160)).unwrap();
        assert!(thumb.path.starts_with(&dir.join("cache")));
        assert!(thumb.path.is_file());
        assert_ne!(std::fs::read(&thumb.path).unwrap(), full);
        assert_eq!(std::fs::read(&original).unwrap(), full);
        assert!(names_containing(&dir.join("cache"), ".0.").is_empty());

        let mut top = page("tall.jpg", 32, 16);
        top.full_h = 32;
        top.h = 16;
        let mut bot = top.clone();
        bot.y0 = 16;
        ex.register_book(book(3, SourceKind::Folder, folder.clone(), vec![top, bot]));
        let slice = ex.get_or_extract((3, 0, 0)).unwrap();
        assert!(slice.path.starts_with(&dir.join("cache")));
        assert_ne!(slice.path, original);
        assert_eq!(std::fs::read(&original).unwrap(), full);
        let slice_bytes = std::fs::read(&slice.path).unwrap();
        assert_ne!(slice_bytes, full);

        let mut huge = page("tall.jpg", 20_000, 20_000);
        huge.full_h = 20_000;
        ex.register_book(book(31, SourceKind::Folder, folder, vec![huge]));
        let scaled = ex.get_or_extract((31, 0, 0)).unwrap();
        assert!(scaled.path.starts_with(&dir.join("cache")));
        assert_ne!(scaled.path, original);
        assert_eq!(std::fs::read(&original).unwrap(), full);
        assert!(cache.stats().entries >= 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn opening_a_folder_drops_only_the_old_full_size_duplicate() {
        let dir = scratch("upgrade");
        let folder = dir.join("book");
        std::fs::create_dir_all(&folder).unwrap();
        let bytes = jpeg(16, 16);
        let original = folder.join("a.jpg");
        std::fs::write(&original, &bytes).unwrap();
        let (cache, ex) = boot(&dir, DEFAULT_CACHE_LIMIT);
        let dup = cache.path_for(&(6, 0, 0), "jpg");
        std::fs::create_dir_all(dup.parent().unwrap()).unwrap();
        std::fs::write(&dup, b"old-full-copy").unwrap();
        cache.insert((6, 0, 0), "jpg", 13);
        let thumb = cache.path_for(&(6, 0, 160), "jpg");
        std::fs::write(&thumb, b"keep-thumb").unwrap();
        cache.insert((6, 0, 160), "jpg", 10);
        ex.register_book(book(
            6,
            SourceKind::Folder,
            folder,
            vec![page("a.jpg", 16, 16)],
        ));
        assert!(!dup.exists());
        assert_eq!(std::fs::read(&thumb).unwrap(), b"keep-thumb");
        assert_eq!(std::fs::read(&original).unwrap(), bytes);
        let hit = ex.get_or_extract((6, 0, 0)).unwrap();
        assert_eq!(hit.path, original);
        assert_eq!(std::fs::read(&hit.path).unwrap(), bytes);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_original_keeps_the_old_cache_copy() {
        let dir = scratch("missing");
        let folder = dir.join("book");
        std::fs::create_dir_all(&folder).unwrap();
        let original = folder.join("gone.jpg");
        std::fs::write(&original, jpeg(8, 8)).unwrap();
        let (cache, ex) = boot(&dir, DEFAULT_CACHE_LIMIT);
        let dup = cache.path_for(&(6, 0, 0), "jpg");
        std::fs::create_dir_all(dup.parent().unwrap()).unwrap();
        let kept = jpeg(8, 8);
        std::fs::write(&dup, &kept).unwrap();
        cache.insert((6, 0, 0), "jpg", kept.len() as u64);
        std::fs::remove_file(&original).unwrap();
        ex.register_book(book(
            6,
            SourceKind::Folder,
            folder,
            vec![page("gone.jpg", 8, 8)],
        ));
        assert_eq!(std::fs::read(&dup).unwrap(), kept);
        let hit = ex.get_or_extract((6, 0, 0)).unwrap();
        assert_eq!(hit.path, dup);
        assert_eq!(std::fs::read(&hit.path).unwrap(), kept);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_path_as_the_original_is_not_deleted() {
        let dir = scratch("same");
        let (cache, ex) = boot(&dir, 64);
        let file = cache.path_for(&(5, 0, 0), "jpg");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let bytes = jpeg(10, 10);
        std::fs::write(&file, &bytes).unwrap();
        cache.insert((5, 0, 0), "jpg", bytes.len() as u64);
        let book_dir = file.parent().unwrap().to_path_buf();
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        ex.register_book(book(
            5,
            SourceKind::Folder,
            book_dir,
            vec![page(&name, 10, 10)],
        ));
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
        cache.set_limit(1);
        cache.evict_if_needed(&|_| false);
        assert_eq!(std::fs::read(&file).unwrap(), bytes);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    #[cfg(unix)]
    fn symlink_original_is_served_and_not_removed() {
        let dir = scratch("link-src");
        let real = dir.join("real.jpg");
        let bytes = jpeg(12, 10);
        std::fs::write(&real, &bytes).unwrap();
        let folder = dir.join("book");
        std::fs::create_dir_all(&folder).unwrap();
        std::os::unix::fs::symlink(&real, folder.join("page.jpg")).unwrap();
        let (cache, ex) = boot(&dir, DEFAULT_CACHE_LIMIT);
        let dup = cache.path_for(&(2, 0, 0), "jpg");
        std::fs::create_dir_all(dup.parent().unwrap()).unwrap();
        std::fs::write(&dup, b"not-the-original").unwrap();
        cache.insert((2, 0, 0), "jpg", 16);
        ex.register_book(book(
            2,
            SourceKind::Folder,
            folder.clone(),
            vec![page("page.jpg", 12, 10)],
        ));
        assert!(!dup.exists());
        assert_eq!(std::fs::read(&real).unwrap(), bytes);
        assert!(folder
            .join("page.jpg")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
        let hit = ex.get_or_extract((2, 0, 0)).unwrap();
        assert_eq!(hit.path, folder.join("page.jpg"));
        assert_eq!(std::fs::read(&hit.path).unwrap(), bytes);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn zip_pages_stay_cached_and_prefetch_stops_when_the_budget_is_full() {
        let dir = scratch("zip");
        let mut pages_bytes = Vec::new();
        let mut pages = Vec::new();
        for i in 0..6u32 {
            let bytes = jpeg(16, 12);
            pages_bytes.push(bytes);
            let mut p = page(&format!("p{i}.jpg"), 16, 12);
            p.entry = i;
            p.src_page = i;
            pages.push(p);
        }
        let zip_path = dir.join("book.zip");
        {
            let mut zw = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
            for (i, bytes) in pages_bytes.iter().enumerate() {
                zw.start_file(
                    format!("p{i}.jpg"),
                    zip::write::SimpleFileOptions::default(),
                )
                .unwrap();
                zw.write_all(bytes).unwrap();
            }
            zw.finish().unwrap();
        }
        let zip_bytes = std::fs::read(&zip_path).unwrap();
        {
            let (cache, ex) = boot(&dir, 50 << 20);
            ex.register_book(book(0, SourceKind::Zip, zip_path.clone(), pages.clone()));
            ex.set_window(Window {
                book: 0,
                center: 0,
                dir: 1,
                ahead: 5,
                behind: 0,
                variant: 0,
            })
            .unwrap();
            settle(&ex);
            assert_eq!(cache.stats().entries, 6);
            assert_eq!(std::fs::read(&zip_path).unwrap(), zip_bytes);
            let cached = ex.get_or_extract((0, 2, 0)).unwrap();
            assert!(cached.path.starts_with(&dir.join("cache")));
            assert_eq!(std::fs::read(&cached.path).unwrap(), pages_bytes[2]);
            ex.close_book(0);
            assert_eq!(std::fs::read(&cached.path).unwrap(), pages_bytes[2]);
        }

        let tight = dir.join("tight");
        std::fs::create_dir_all(&tight).unwrap();
        let (cache, ex) = boot(&tight, 100);
        let filler = cache.path_for(&(0, 0, 0), "jpg");
        std::fs::create_dir_all(filler.parent().unwrap()).unwrap();
        std::fs::write(&filler, b"filler").unwrap();
        cache.insert((0, 0, 0), "jpg", 1_000);
        ex.register_book(book(0, SourceKind::Zip, zip_path.clone(), pages.clone()));
        // The filler is not a folder original, so register must not delete it.
        assert!(filler.is_file());
        ex.set_window(Window {
            book: 0,
            center: 0,
            dir: 1,
            ahead: 5,
            behind: 0,
            variant: 0,
        })
        .unwrap();
        settle(&ex);
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(
            cache.stats().entries,
            1,
            "prefetch must not grow a full cache"
        );
        let hit = ex.get_or_extract((0, 5, 0)).unwrap();
        assert_eq!(std::fs::read(&hit.path).unwrap(), pages_bytes[5]);
        ex.close_book(0);
        assert_eq!(std::fs::read(&hit.path).unwrap(), pages_bytes[5]);
        assert_eq!(std::fs::read(&zip_path).unwrap(), zip_bytes);

        // Empty cache, tiny limit: the window is queued, but workers stop reading
        // once the budget is exceeded. Two workers can pass the check together.
        let burst = dir.join("burst");
        std::fs::create_dir_all(&burst).unwrap();
        let (cache, ex) = boot(&burst, 1);
        ex.register_book(book(0, SourceKind::Zip, zip_path.clone(), pages));
        ex.set_window(Window {
            book: 0,
            center: 0,
            dir: 1,
            ahead: 5,
            behind: 0,
            variant: 0,
        })
        .unwrap();
        settle(&ex);
        let n = cache.stats().entries;
        assert!(
            n >= 1 && n <= 2,
            "prefetch stored {n} pages after the budget was already exceeded"
        );
        assert_eq!(std::fs::read(&zip_path).unwrap(), zip_bytes);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loose_images_and_appended_paths_are_not_copied() {
        let dir = scratch("loose");
        let bytes = jpeg(14, 14);
        let loose = dir.join("loose.jpg");
        std::fs::write(&loose, &bytes).unwrap();
        let (cache, ex) = boot(&dir, DEFAULT_CACHE_LIMIT);
        ex.register_book(book(
            1,
            SourceKind::Images,
            dir.clone(),
            vec![page(&loose.display().to_string(), 14, 14)],
        ));
        let hit = ex.get_or_extract((1, 0, 0)).unwrap();
        assert_eq!(hit.path, loose);
        assert_eq!(cache.stats().entries, 0);
        let appended = dir.join("appended.jpg");
        std::fs::write(&appended, &bytes).unwrap();
        let mut p = page("from-zip.jpg", 14, 14);
        p.src_path = Some(appended.display().to_string());
        ex.register_book(book(1, SourceKind::Zip, dir.join("nope.zip"), vec![p]));
        let hit = ex.get_or_extract((1, 0, 0)).unwrap();
        assert_eq!(hit.path, appended);
        assert_eq!(std::fs::read(&appended).unwrap(), bytes);
        assert!(names_containing(&dir.join("cache"), ".0.").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clear_keeps_the_page_being_read_and_switching_books_keeps_originals() {
        let dir = scratch("switch");
        let a = dir.join("a");
        let b = dir.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let a_bytes = jpeg(12, 12);
        let b_bytes = jpeg(13, 11);
        std::fs::write(a.join("a.jpg"), &a_bytes).unwrap();
        std::fs::write(b.join("b.jpg"), &b_bytes).unwrap();
        let unknown = dir.join("cache-looking");
        std::fs::create_dir_all(unknown.join("pages")).unwrap();
        std::fs::write(unknown.join("pages").join("1.jpg"), b"not-ours").unwrap();
        let (cache, ex) = boot(&dir, DEFAULT_CACHE_LIMIT);
        ex.register_book(book(
            0,
            SourceKind::Folder,
            a.clone(),
            vec![page("a.jpg", 12, 12)],
        ));
        ex.register_book(book(
            2,
            SourceKind::Folder,
            b.clone(),
            vec![page("b.jpg", 13, 11)],
        ));
        ex.set_window(Window {
            book: 0,
            center: 0,
            dir: 1,
            ahead: 0,
            behind: 0,
            variant: 160,
        })
        .unwrap();
        let reading = ex.get_or_extract((0, 0, 160)).unwrap();
        let reading_bytes = std::fs::read(&reading.path).unwrap();
        let other = cache.path_for(&(2, 3, 160), "jpg");
        std::fs::create_dir_all(other.parent().unwrap()).unwrap();
        std::fs::write(&other, vec![1u8; 30]).unwrap();
        cache.insert((2, 3, 160), "jpg", 30);
        let win = ex.window().unwrap();
        cache.clear(&|k| win.pins(k));
        assert_eq!(std::fs::read(&reading.path).unwrap(), reading_bytes);
        assert!(!other.exists());
        assert_eq!(std::fs::read(a.join("a.jpg")).unwrap(), a_bytes);
        assert_eq!(std::fs::read(b.join("b.jpg")).unwrap(), b_bytes);
        ex.close_book(0);
        let still = ex.get_or_extract((2, 0, 0)).unwrap();
        assert_eq!(still.path, b.join("b.jpg"));
        assert_eq!(std::fs::read(&still.path).unwrap(), b_bytes);
        assert_eq!(std::fs::read(a.join("a.jpg")).unwrap(), a_bytes);
        assert_eq!(
            std::fs::read(unknown.join("pages").join("1.jpg")).unwrap(),
            b"not-ours"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
