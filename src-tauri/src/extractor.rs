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
    sync::{
        atomic::{AtomicBool, Ordering::SeqCst},
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
    pub fn book(&self, id: i64) -> Option<Arc<BookMeta>> {
        self.st.lock().books.get(&id).cloned()
    }
    pub fn window(&self) -> Option<Window> {
        self.st.lock().win
    }

    pub fn close_book(&self, id: i64) {
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
        let (lo, hi) = w.range(n);

        // 1) re-prioritise and cancel: drain the (small) heap, keep what is still wanted
        let old: Vec<Job> = st.heap.drain().collect();
        for mut j in old {
            let other_variant = j.key.2 != w.variant; // e.g. scrubber thumbnails: never auto-cancelled
            let inside = j.key.0 == w.book && (lo..=hi).contains(&j.key.1);
            if inside || other_variant {
                j.cost = w.cost(j.key.1);
                st.heap.push(j);
            } else {
                Self::cancel(&mut st, j.key);
            } // outside window: waiters get Cancelled (-> HTTP 503)
        }
        // 2) enqueue what is missing
        for idx in lo..=hi {
            let key = (w.book, idx, w.variant);
            if st.slots.contains_key(&key) || self.cache.contains(&key) {
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
            let res = self.process(job.key, &slot, &book, &mut zip);
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

    fn process(&self, key: Key, slot: &Slot, book: &BookMeta, zip: &mut ZipHandle) -> Res {
        if let Some(p) = self.cache.get(&key) {
            return Ok(p);
        } // sibling slice already produced it
        let (_, idx, variant) = key;
        let page = book
            .pages
            .get(idx as usize)
            .ok_or(ReaderError::PageOutOfRange)?;
        let raw = read_entry(book, page, zip)?; // phase 1: bytes out of the source
        if !slot.demanded.load(SeqCst) && !self.still_wanted(key) {
            return Err(ReaderError::Cancelled);
        } // phase check: jumped away

        if variant == 0 && !page.is_slice() {
            // fast path: store as-is
            let (_, ext) = sniff(&raw)
                .ok_or_else(|| ReaderError::Corrupt("not a recognised image".into()))?;
            return self.store(key, ext, &raw);
        }
        // decode path: slices and/or width variants. Decode ONCE, emit every slice of the image.
        if page.w as u64 * page.full_h as u64 > scan::MAX_PIXELS {
            return Err(ReaderError::Unsupported("image too large".into()));
        }
        let img = decode(&raw)?;
        let (first, last) = if page.is_slice() {
            book.groups[page.src_page as usize]
        } else {
            (idx, idx)
        };
        let mut wanted = None;
        for vi in first..=last {
            let k = (book.id, vi, variant);
            let p = &book.pages[vi as usize];
            if vi != idx && self.cache.contains(&k) {
                continue;
            }
            let part = if p.is_slice() {
                img.crop_imm(0, p.y0, img.width(), p.h)
            } else {
                img.clone()
            };
            let part = shrink_to(part, variant);
            let (bytes, ext) = encode(&part)?;
            let cp = self.store(k, ext, &bytes)?;
            if vi == idx {
                wanted = Some(cp);
            }
        }
        wanted.ok_or(ReaderError::PageOutOfRange)
    }

    fn store(&self, key: Key, ext: &'static str, bytes: &[u8]) -> Res {
        let path = self.cache.path_for(&key, ext);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("part");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?; // atomic: readers never see half a file
        Ok(self.cache.insert(key, ext, bytes.len() as u64))
    }
}

fn read_entry(
    book: &BookMeta,
    page: &PageMeta,
    zip: &mut ZipHandle,
) -> Result<Vec<u8>, ReaderError> {
    // Appended/merged pages carry their own source file (see model::PageMeta).
    if let Some(p) = &page.src_path {
        return Ok(std::fs::read(p)?);
    }
    match book.kind {
        SourceKind::Zip => {
            if zip.as_ref().map(|(id, _)| *id) != Some(book.id) {
                *zip = Some((book.id, scan::open_zip(&book.path)?));
            }
            let ar = &mut zip.as_mut().unwrap().1;
            let mut e = ar.by_index(page.entry as usize)?;
            if e.size() > scan::MAX_ENTRY {
                return Err(ReaderError::Corrupt("entry too large".into()));
            }
            let mut buf = Vec::with_capacity(e.size() as usize);
            e.read_to_end(&mut buf)?; // CRC checked by the zip crate at EOF
            Ok(buf)
        }
        SourceKind::Folder => Ok(std::fs::read(book.path.join(&page.name))?),
        SourceKind::Images => Ok(std::fs::read(&page.name)?),
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
    } else if b.len() > 12 && &b[4..8] == b"ftyp" && (&b[8..12] == b"avif" || &b[8..12] == b"avis") {
        Some(("image/avif", "avif"))
    } else if b.starts_with(b"BM") {
        Some(("image/bmp", "bmp"))
    } else {
        None
    }
}

fn decode(raw: &[u8]) -> Result<image::DynamicImage, ReaderError> {
    let mut r = image::ImageReader::new(Cursor::new(raw)).with_guessed_format()?;
    let mut lim = image::Limits::default();
    lim.max_alloc = Some(1 << 30); // 1 GiB hard ceiling
    r.limits(lim);
    Ok(r.decode()?)
}
fn shrink_to(img: image::DynamicImage, variant: u16) -> image::DynamicImage {
    if variant == 0 || img.width() <= variant as u32 {
        return img;
    }
    let nh = ((img.height() as u64 * variant as u64) / img.width() as u64).max(1) as u32;
    img.resize_exact(
        variant as u32,
        nh,
        image::imageops::FilterType::Lanczos3,
    )
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
