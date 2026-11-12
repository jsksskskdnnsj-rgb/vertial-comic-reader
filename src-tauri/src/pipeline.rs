//! Reading pipeline. The screen still talks to the same commands; these layers
//! are what the commands call.
//!
//! 1. **Source.** A directory walk yields names and sizes in batches. It does not
//!    read pixels, and it stops when cancelled.
//! 2. **Archive.** A zip is indexed from the central directory only. One page is
//!    streamed later, and a claimed size above the cap is rejected before any
//!    large allocation. Non-UTF-8 names are decoded as GBK.
//! 3. **Decode.** Header dimensions are checked first. The decoder has a byte
//!    budget and a hard width/height. Images past the reading budget are
//!    downscaled; a decode that panics becomes an error for that page.
//! 4. **Schedule.** Visible page reads set a foreground flag. Thumbnail and scan
//!    work wait. The thumbnail pool is two threads. Queued scan jobs for a book
//!    the reader has left are dropped. The extractor keeps visible/prefetch
//!    ordering and cancels pages outside the window.
//! 5. **Transport.** Page bytes already travel on the `mhpage` stream. Import
//!    runs off the UI thread and emits `import-progress`. One bad file is a
//!    failed row, not a process crash.

use crate::error::ReaderError;
use encoding_rs::GBK;
use parking_lot::{Condvar, Mutex};
use std::collections::BinaryHeap;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Same ceiling the zip scanner has always used. A larger claim is an error.
pub const PAGE_BYTE_CAP: u64 = 256 * 1024 * 1024;
/// Longest edge we will keep after a decode. Taller webtoon pages are sliced
/// before this, so a normal page is not resized.
pub const DOWNSCALE_EDGE: u32 = 16_384;
/// Bitmaps above this are resized down before they are cached.
pub const DOWNSCALE_PIXELS: u64 = 40_000_000;
const READ_CHUNK: usize = 64 * 1024;

#[derive(Clone, Debug)]
pub struct TreeFile {
    pub rel: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default)]
pub struct TreeWalk {
    pub images: Vec<TreeFile>,
    pub files: u64,
    pub bytes: u64,
    pub max_mtime_ms: u128,
}

/// Names and sizes only. `on_batch` sees each group of image files and can drop
/// them; the returned walk still holds the full image list for the shelf index.
pub fn walk_tree(
    root: &Path,
    cancel: &AtomicBool,
    mut on_batch: impl FnMut(&[TreeFile]),
) -> TreeWalk {
    let walked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        walk_tree_inner(root, cancel, &mut on_batch)
    }));
    walked.unwrap_or_default()
}

fn walk_tree_inner(
    root: &Path,
    cancel: &AtomicBool,
    on_batch: &mut impl FnMut(&[TreeFile]),
) -> TreeWalk {
    let mut out = TreeWalk::default();
    let mut batch: Vec<TreeFile> = Vec::new();
    for ent in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if !ent.file_type().is_file() {
            continue;
        }
        let Ok(meta) = ent.metadata() else { continue };
        out.files += 1;
        out.bytes = out.bytes.saturating_add(meta.len());
        out.max_mtime_ms = out.max_mtime_ms.max(mtime_ms(&meta));
        let Ok(rel) = ent.path().strip_prefix(root) else { continue };
        let rel = rel.to_string_lossy().replace('\\', "/");
        if !crate::scan::is_image(&rel) {
            continue;
        }
        batch.push(TreeFile { rel, bytes: meta.len() });
        if batch.len() >= 64 {
            on_batch(&batch);
            out.images.append(&mut batch);
        }
    }
    if !batch.is_empty() {
        on_batch(&batch);
        out.images.append(&mut batch);
    }
    crate::shelf_scan::sort_by_comic_name(&mut out.images, |f| f.rel.as_str());
    out
}

fn mtime_ms(m: &std::fs::Metadata) -> u128 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Read at most `max` bytes. Never reserves from a claimed size.
pub fn read_prefix(reader: &mut dyn Read, max: u64) -> Result<Vec<u8>, ReaderError> {
    let max = max.min(PAGE_BYTE_CAP) as usize;
    let mut buf = Vec::new();
    let mut chunk = [0u8; READ_CHUNK];
    while buf.len() < max {
        let want = chunk.len().min(max - buf.len());
        let n = reader.read(&mut chunk[..want])?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(buf)
}

/// Read a whole payload that claims to be `claimed` bytes.
/// A claim above `max`, or a stream that runs past `max`, is an error and the
/// buffer is dropped. Nothing is reserved from `claimed` when it is over the cap.
pub fn read_capped(reader: &mut dyn Read, claimed: u64, max: u64) -> Result<Vec<u8>, ReaderError> {
    if claimed > max {
        return Err(ReaderError::Corrupt(format!("文件过大（{claimed} 字节），已跳过")));
    }
    let mut buf = Vec::new();
    let hint = claimed.min(max).min(8 * 1024 * 1024) as usize;
    if hint > 0 && buf.try_reserve(hint).is_err() {
        return Err(ReaderError::Corrupt("内存不足，已跳过该页".into()));
    }
    let mut chunk = [0u8; READ_CHUNK];
    let limit = max as usize;
    loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        if buf.len().saturating_add(n) > limit {
            return Err(ReaderError::Corrupt("文件过大，已跳过".into()));
        }
        if buf.try_reserve(n).is_err() {
            return Err(ReaderError::Corrupt("内存不足，已跳过该页".into()));
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(buf)
}

pub fn read_file_capped(path: &Path) -> Result<Vec<u8>, ReaderError> {
    let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let file = std::fs::File::open(path)?;
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        if len > PAGE_BYTE_CAP {
            return Err(ReaderError::Corrupt(format!("文件过大（{len} 字节），已跳过")));
        }
        let mut file = file;
        read_capped(&mut file, len, PAGE_BYTE_CAP)
    }));
    match read {
        Ok(r) => r,
        Err(_) => Err(ReaderError::Corrupt("该文件异常，已跳过".into())),
    }
}

#[derive(Clone, Debug)]
pub struct CdEntry {
    pub index: u32,
    pub name: String,
    pub bytes: u64,
    pub crc32: u32,
    pub dir: bool,
}

/// Central directory only. Local file payloads are not read.
pub fn zip_central_index(path: &Path) -> Result<Vec<CdEntry>, ReaderError> {
    let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut file = std::fs::File::open(path)?;
        parse_central_directory(&mut file)
    }));
    match parsed {
        Ok(r) => r,
        Err(_) => Err(ReaderError::Corrupt("压缩包目录异常，已跳过".into())),
    }
}

fn parse_central_directory(file: &mut std::fs::File) -> Result<Vec<CdEntry>, ReaderError> {
    use std::io::{Seek, SeekFrom};
    let len = file.metadata()?.len();
    if len < 22 {
        return Err(ReaderError::Corrupt("压缩包太小".into()));
    }
    let tail_len = len.min(65_557 + 22);
    file.seek(SeekFrom::Start(len - tail_len))?;
    let mut tail = vec![0u8; tail_len as usize];
    file.read_exact(&mut tail)?;
    let eocd = find_eocd(&tail).ok_or_else(|| ReaderError::Corrupt("找不到中央目录".into()))?;
    let mut total = u16_at(&tail, eocd + 10) as u64;
    let mut cd_size = u32_at(&tail, eocd + 12) as u64;
    let mut cd_off = u32_at(&tail, eocd + 16) as u64;
    if total == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_off == 0xFFFF_FFFF {
        let loc = eocd.checked_sub(20).ok_or_else(|| ReaderError::Corrupt("ZIP64 目录无效".into()))?;
        if u32_at(&tail, loc) != 0x0706_4b50 {
            return Err(ReaderError::Corrupt("ZIP64 定位无效".into()));
        }
        let zoff = u64_at(&tail, loc + 8);
        file.seek(SeekFrom::Start(zoff))?;
        let mut hdr = [0u8; 56];
        file.read_exact(&mut hdr)?;
        if u32_at(&hdr, 0) != 0x0606_4b50 {
            return Err(ReaderError::Corrupt("ZIP64 目录无效".into()));
        }
        total = u64_at(&hdr, 32);
        cd_size = u64_at(&hdr, 40);
        cd_off = u64_at(&hdr, 48);
    }
    if cd_size > 64 * 1024 * 1024 || total > 1_000_000 {
        return Err(ReaderError::Corrupt("压缩包目录过大，已跳过".into()));
    }
    file.seek(SeekFrom::Start(cd_off))?;
    let mut cd = vec![0u8; cd_size as usize];
    file.read_exact(&mut cd)?;
    let mut entries = Vec::with_capacity(total.min(4096) as usize);
    let mut i = 0usize;
    let mut index = 0u32;
    while i + 46 <= cd.len() && (total == 0 || entries.len() < total as usize) {
        if u32_at(&cd, i) != 0x0201_4b50 {
            break;
        }
        let flags = u16_at(&cd, i + 8);
        let crc = u32_at(&cd, i + 16);
        let comp = u32_at(&cd, i + 20);
        let uncomp = u32_at(&cd, i + 24);
        let name_len = u16_at(&cd, i + 28) as usize;
        let extra_len = u16_at(&cd, i + 30) as usize;
        let comment_len = u16_at(&cd, i + 32) as usize;
        let name_at = i + 46;
        let extra_at = name_at + name_len;
        let next = extra_at + extra_len + comment_len;
        if next > cd.len() {
            return Err(ReaderError::Corrupt("中央目录被截断".into()));
        }
        let raw_name = &cd[name_at..name_at + name_len];
        let extra = &cd[extra_at..extra_at + extra_len];
        let mut bytes = uncomp as u64;
        if uncomp == 0xFFFF_FFFF || comp == 0xFFFF_FFFF {
            if let Some(z) = zip64_sizes(extra, uncomp == 0xFFFF_FFFF, comp == 0xFFFF_FFFF) {
                bytes = z;
            }
        }
        let name = decode_zip_name(raw_name, flags & (1 << 11) != 0);
        let dir = name.ends_with('/') || name.ends_with('\\');
        entries.push(CdEntry { index, name, bytes, crc32: crc, dir });
        index = index.saturating_add(1);
        i = next;
    }
    if entries.is_empty() && total > 0 {
        return Err(ReaderError::Corrupt("中央目录是空的".into()));
    }
    Ok(entries)
}

fn zip64_sizes(extra: &[u8], want_uncomp: bool, want_comp: bool) -> Option<u64> {
    let mut p = 0;
    while p + 4 <= extra.len() {
        let id = u16_at(extra, p);
        let sz = u16_at(extra, p + 2) as usize;
        let data_at = p + 4;
        if data_at + sz > extra.len() {
            return None;
        }
        if id == 0x0001 {
            let data = &extra[data_at..data_at + sz];
            let q = 0;
            if want_uncomp {
                if q + 8 > data.len() {
                    return None;
                }
                return Some(u64_at(data, q));
            }
            if want_comp && q + 8 <= data.len() {
                return Some(u64_at(data, q));
            }
        }
        p = data_at + sz;
    }
    None
}

pub fn decode_zip_name(raw: &[u8], utf8_flag: bool) -> String {
    // Bit 11 means the central directory stored UTF-8. Without it, a Chinese
    // archive is GBK. ASCII stays ASCII. Bytes that are also valid UTF-8
    // (for example CD BC) are still GBK when the flag is off.
    let text = if utf8_flag || raw.is_ascii() {
        String::from_utf8_lossy(raw).into_owned()
    } else {
        let (decoded, _, failed) = GBK.decode(raw);
        if failed {
            String::from_utf8_lossy(raw).into_owned()
        } else {
            decoded.into_owned()
        }
    };
    text.replace('\\', "/")
}

fn find_eocd(buf: &[u8]) -> Option<usize> {
    if buf.len() < 22 {
        return None;
    }
    let mut i = buf.len() - 22;
    loop {
        if buf[i] == 0x50 && buf[i + 1] == 0x4b && buf[i + 2] == 0x05 && buf[i + 3] == 0x06 {
            return Some(i);
        }
        if i == 0 {
            return None;
        }
        i -= 1;
    }
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}
fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}
fn u64_at(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes([
        b[i],
        b[i + 1],
        b[i + 2],
        b[i + 3],
        b[i + 4],
        b[i + 5],
        b[i + 6],
        b[i + 7],
    ])
}

pub fn image_dims(path: &Path) -> (u32, u32) {
    let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        imagesize::size(path)
            .ok()
            .map(|s| (s.width as u32, s.height as u32))
    }));
    match got {
        Ok(Some(d)) => d,
        _ => (0, 0),
    }
}

pub fn should_downscale(w: u32, h: u32, bytes: usize) -> bool {
    if w > 0 && h > 0 {
        let px = w as u64 * h as u64;
        return px > DOWNSCALE_PIXELS || w.max(h) > DOWNSCALE_EDGE;
    }
    bytes > 48 * 1024 * 1024
}

pub fn downscale_huge(img: image::DynamicImage) -> image::DynamicImage {
    let w = img.width();
    let h = img.height();
    if w == 0 || h == 0 {
        return img;
    }
    let px = w as u64 * h as u64;
    if px <= DOWNSCALE_PIXELS && w.max(h) <= DOWNSCALE_EDGE {
        return img;
    }
    let mut scale = 1.0f64;
    let edge = w.max(h);
    if edge > DOWNSCALE_EDGE {
        scale = f64::from(DOWNSCALE_EDGE) / f64::from(edge);
    }
    let mut nw = (f64::from(w) * scale).max(1.0);
    let mut nh = (f64::from(h) * scale).max(1.0);
    if nw * nh > DOWNSCALE_PIXELS as f64 {
        let fit = (DOWNSCALE_PIXELS as f64 / (nw * nh)).sqrt();
        nw = (nw * fit).max(1.0);
        nh = (nh * fit).max(1.0);
    }
    img.resize(nw as u32, nh as u32, image::imageops::FilterType::Triangle)
}

pub fn decode_limited(raw: &[u8]) -> Result<image::DynamicImage, ReaderError> {
    let decoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode_limited_inner(raw)));
    match decoded {
        Ok(r) => r,
        Err(_) => Err(ReaderError::Corrupt("图片解码失败，已跳过".into())),
    }
}

fn decode_limited_inner(raw: &[u8]) -> Result<image::DynamicImage, ReaderError> {
    if let Some(size) = imagesize::blob_size(raw).ok() {
        let px = size.width as u64 * size.height as u64;
        if px > crate::scan::MAX_PIXELS {
            return Err(ReaderError::Unsupported("图片过大，已跳过".into()));
        }
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(raw)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(384 * 1024 * 1024);
    limits.max_image_width = Some(32_000);
    limits.max_image_height = Some(200_000);
    reader.limits(limits);
    reader.decode().map_err(|e| ReaderError::Corrupt(format!("无法解码，已跳过：{e}")))
}

static FOREGROUND: AtomicUsize = AtomicUsize::new(0);
static FOCUS_BOOK: AtomicI64 = AtomicI64::new(0);

fn fg_gate() -> &'static (Mutex<()>, Condvar) {
    static GATE: OnceLock<(Mutex<()>, Condvar)> = OnceLock::new();
    GATE.get_or_init(|| (Mutex::new(()), Condvar::new()))
}

pub struct ForegroundGuard;
impl Drop for ForegroundGuard {
    fn drop(&mut self) {
        if FOREGROUND.fetch_sub(1, Ordering::SeqCst) == 1 {
            fg_gate().1.notify_all();
        }
    }
}

/// A visible (or in-flight) page read. Scan and thumbnail work waits.
pub fn enter_foreground() -> ForegroundGuard {
    FOREGROUND.fetch_add(1, Ordering::SeqCst);
    ForegroundGuard
}

pub fn foreground_active() -> bool {
    FOREGROUND.load(Ordering::SeqCst) > 0
}

pub fn note_visible(book: i64) {
    FOCUS_BOOK.store(book, Ordering::Relaxed);
}

pub fn focused_book() -> i64 {
    FOCUS_BOOK.load(Ordering::Relaxed)
}

/// Block until page reads have released the foreground flag.
pub fn wait_foreground_idle() {
    let (lock, cv) = fg_gate();
    let mut guard = lock.lock();
    while foreground_active() {
        cv.wait_for(&mut guard, Duration::from_millis(30));
    }
}

/// Used between header probes so a background fill does not run through a page read.
pub fn cooperate() {
    if foreground_active() {
        wait_foreground_idle();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[repr(u8)]
pub enum Priority {
    Scan = 0,
    Thumb = 1,
    Prefetch = 2,
    Visible = 3,
}

struct Job {
    prio: Priority,
    seq: u64,
    book: Option<i64>,
    cancel: bool,
    run: Option<Box<dyn FnOnce() + Send>>,
}

impl PartialEq for Job {
    fn eq(&self, other: &Self) -> bool {
        self.prio == other.prio && self.seq == other.seq
    }
}
impl Eq for Job {}
impl PartialOrd for Job {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Job {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.prio.cmp(&other.prio).then(other.seq.cmp(&self.seq))
    }
}

struct SchedInner {
    jobs: BinaryHeap<Job>,
    seq: u64,
    shutdown: bool,
    cancelled: Vec<i64>,
}

pub struct Scheduler {
    inner: Mutex<SchedInner>,
    cv: Condvar,
}

impl Scheduler {
    pub fn start(workers: usize) -> Arc<Self> {
        let s = Arc::new(Self {
            inner: Mutex::new(SchedInner {
                jobs: BinaryHeap::new(),
                seq: 0,
                shutdown: false,
                cancelled: Vec::new(),
            }),
            cv: Condvar::new(),
        });
        for n in 0..workers.max(1) {
            let w = s.clone();
            let _ = std::thread::Builder::new()
                .name(format!("mh-sched-{n}"))
                .spawn(move || w.worker());
        }
        s
    }

    pub fn submit(
        &self,
        prio: Priority,
        book: Option<i64>,
        cancel: bool,
        run: impl FnOnce() + Send + 'static,
    ) {
        let mut g = self.inner.lock();
        if g.shutdown {
            return;
        }
        if cancel {
            if let Some(id) = book {
                if g.cancelled.contains(&id) {
                    return;
                }
            }
        }
        g.seq += 1;
        let seq = g.seq;
        g.jobs.push(Job {
            prio,
            seq,
            book,
            cancel,
            run: Some(Box::new(run)),
        });
        drop(g);
        self.cv.notify_one();
    }

    pub fn cancel_book(&self, id: i64) {
        let mut g = self.inner.lock();
        if !g.cancelled.contains(&id) {
            g.cancelled.push(id);
        }
        let kept: Vec<Job> = g.jobs.drain().filter(|j| !(j.cancel && j.book == Some(id))).collect();
        for job in kept {
            g.jobs.push(job);
        }
        self.cv.notify_all();
    }

    pub fn shutdown(&self) {
        self.inner.lock().shutdown = true;
        self.cv.notify_all();
    }

    fn worker(self: Arc<Self>) {
        loop {
            let job = {
                let mut g = self.inner.lock();
                loop {
                    if g.shutdown && g.jobs.is_empty() {
                        return;
                    }
                    if let Some(job) = g.jobs.pop() {
                        break job;
                    }
                    self.cv.wait(&mut g);
                }
            };
            if job.cancel {
                if let Some(id) = job.book {
                    if self.inner.lock().cancelled.contains(&id) {
                        continue;
                    }
                }
            }
            if job.prio <= Priority::Thumb {
                wait_foreground_idle();
            }
            if let Some(run) = job.run {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
            }
        }
    }
}

fn global_scheduler() -> &'static Arc<Scheduler> {
    static S: OnceLock<Arc<Scheduler>> = OnceLock::new();
    S.get_or_init(|| Scheduler::start(2))
}

pub fn scheduler() -> &'static Arc<Scheduler> {
    global_scheduler()
}

pub fn proc_kb(field: &str) -> u64 {
    let Ok(text) = std::fs::read_to_string("/proc/self/status") else {
        return 0;
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(field) {
            return rest.split_whitespace().next().unwrap_or("0").parse().unwrap_or(0);
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::mpsc;
    use std::time::Instant;

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &b in data {
            crc ^= u32::from(b);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }

    fn push_u16(out: &mut Vec<u8>, v: u16) {
        out.extend_from_slice(&v.to_le_bytes());
    }
    fn push_u32(out: &mut Vec<u8>, v: u32) {
        out.extend_from_slice(&v.to_le_bytes());
    }

    fn stored_zip(name: &[u8], data: &[u8], uncomp: u32) -> Vec<u8> {
        let crc = crc32(data);
        let comp = data.len() as u32;
        let mut out = Vec::new();
        push_u32(&mut out, 0x0403_4b50);
        push_u16(&mut out, 20);
        push_u16(&mut out, 0);
        push_u16(&mut out, 0);
        push_u16(&mut out, 0);
        push_u16(&mut out, 0);
        push_u32(&mut out, crc);
        push_u32(&mut out, comp);
        push_u32(&mut out, uncomp);
        push_u16(&mut out, name.len() as u16);
        push_u16(&mut out, 0);
        out.extend_from_slice(name);
        out.extend_from_slice(data);
        let cd_off = out.len() as u32;
        push_u32(&mut out, 0x0201_4b50);
        push_u16(&mut out, 20);
        push_u16(&mut out, 20);
        push_u16(&mut out, 0);
        push_u16(&mut out, 0);
        push_u16(&mut out, 0);
        push_u16(&mut out, 0);
        push_u32(&mut out, crc);
        push_u32(&mut out, comp);
        push_u32(&mut out, uncomp);
        push_u16(&mut out, name.len() as u16);
        push_u16(&mut out, 0);
        push_u16(&mut out, 0);
        push_u16(&mut out, 0);
        push_u16(&mut out, 0);
        push_u32(&mut out, 0);
        push_u32(&mut out, 0);
        out.extend_from_slice(name);
        let cd_size = out.len() as u32 - cd_off;
        push_u32(&mut out, 0x0605_4b50);
        push_u16(&mut out, 0);
        push_u16(&mut out, 0);
        push_u16(&mut out, 1);
        push_u16(&mut out, 1);
        push_u32(&mut out, cd_size);
        push_u32(&mut out, cd_off);
        push_u16(&mut out, 0);
        out
    }

    #[test]
    fn gbk_central_directory_name_and_large_claim_are_not_read() {
        let name = [0xCD, 0xBC, b'.', b'j', b'p', b'g'];
        let bytes = stored_zip(&name, b"hi", 0x8000_0000);
        let dir = std::env::temp_dir().join(format!("mh-pipe-gbk-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.zip");
        std::fs::write(&path, &bytes).unwrap();
        let t = Instant::now();
        let idx = zip_central_index(&path).unwrap();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(idx.len(), 1);
        assert_eq!(idx[0].index, 0);
        assert_eq!(idx[0].name, "图.jpg");
        assert_eq!(idx[0].bytes, 0x8000_0000);
        assert_eq!(idx[0].crc32, crc32(b"hi"));
        let before = proc_kb("VmSize:");
        let err = read_capped(&mut &bytes[..], idx[0].bytes, PAGE_BYTE_CAP).unwrap_err();
        let after = proc_kb("VmSize:");
        let delta = after.saturating_sub(before);
        eprintln!(
            "BENCH archive claimed=2GiB index_ms={ms:.2} refuse_vmsize_delta_kb={delta} err={err}"
        );
        assert!(delta < 8 * 1024, "refusing the claim reserved {delta} KiB");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn multi_gb_file_is_refused_without_reading_it() {
        let dir = std::env::temp_dir().join(format!("mh-pipe-sparse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("huge.bin");
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(2 * 1024 * 1024 * 1024).unwrap();
        drop(file);
        let rss_before = proc_kb("VmHWM:");
        let vm_before = proc_kb("VmSize:");
        let reserve_started = Instant::now();
        let mut probe = Vec::<u8>::new();
        let reserved = probe.try_reserve(2 * 1024 * 1024 * 1024);
        let reserve_ms = reserve_started.elapsed().as_secs_f64() * 1000.0;
        let vm_after_reserve = proc_kb("VmSize:");
        let reserve_ok = reserved.is_ok();
        drop(probe);
        let t = Instant::now();
        let err = read_file_capped(&path).unwrap_err();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let rss_after = proc_kb("VmHWM:");
        let vm_after = proc_kb("VmSize:");
        eprintln!(
            "BENCH file claimed=2GiB refuse_ms={ms:.2} rss_delta_kb={} vmsize_delta_kb={} reserve_ok={reserve_ok} reserve_ms={reserve_ms:.2} reserve_vmsize_delta_kb={} err={err}",
            rss_after.saturating_sub(rss_before),
            vm_after.saturating_sub(vm_before),
            vm_after_reserve.saturating_sub(vm_before)
        );
        assert!(ms < 500.0, "refusing a sparse 2GiB file took {ms:.1} ms");
        assert!(rss_after.saturating_sub(rss_before) < 64 * 1024);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn folder_of_1200_images_lists_without_reading_bytes() {
        let dir = std::env::temp_dir().join(format!("mh-pipe-1200-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let blob = [0xFFu8, 0xD8, 0xFF, 0xD9];
        for i in 0..1200 {
            let sub = dir.join(format!("ch{:02}", i / 100));
            std::fs::create_dir_all(&sub).unwrap();
            std::fs::write(sub.join(format!("{i}.jpg")), blob).unwrap();
        }
        let rss_before = proc_kb("VmHWM:");
        let mut batches = 0usize;
        let t = Instant::now();
        let walk = walk_tree(&dir, &AtomicBool::new(false), |_| batches += 1);
        let walk_ms = t.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(walk.images.len(), 1200);
        assert!(batches >= 2, "walk did not flush batches");
        assert_eq!(walk.images[0].rel.contains('/'), true);
        let read_started = Instant::now();
        let mut sink = 0usize;
        for img in &walk.images {
            let bytes = std::fs::read(dir.join(&img.rel)).unwrap();
            sink += bytes.len();
        }
        let read_ms = read_started.elapsed().as_secs_f64() * 1000.0;
        let rss_after = proc_kb("VmHWM:");
        eprintln!(
            "BENCH folder images=1200 walk_ms={walk_ms:.2} read_all_ms={read_ms:.2} batches={batches} rss_delta_kb={} sink={sink}",
            rss_after.saturating_sub(rss_before)
        );
        assert!(walk_ms < 5_000.0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn central_index_matches_zip_crate_for_stored_entries() {
        let dir = std::env::temp_dir().join(format!("mh-pipe-zip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pages.zip");
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut zw = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for i in 0..30u32 {
                zw.start_file(format!("{i}.jpg"), opts).unwrap();
                zw.write_all(&[0xFF, 0xD8, 0xFF, i as u8]).unwrap();
            }
            zw.finish().unwrap();
        }
        let idx = zip_central_index(&path).unwrap();
        let mut ar = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(idx.len(), ar.len());
        for (i, ent) in idx.iter().enumerate() {
            let f = ar.by_index(i).unwrap();
            assert_eq!(ent.name, f.name());
            assert_eq!(ent.bytes, f.size());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scheduler_runs_thumb_before_scan_and_drops_a_cancelled_book() {
        let sched = Scheduler::start(1);
        let (started_tx, started_rx) = mpsc::channel();
        let (go_tx, go_rx) = mpsc::channel::<()>();
        let order = Arc::new(Mutex::new(Vec::new()));
        sched.submit(Priority::Thumb, None, false, {
            let order = order.clone();
            move || {
                started_tx.send(()).ok();
                go_rx.recv().ok();
                order.lock().push(1);
            }
        });
        started_rx.recv().unwrap();
        sched.submit(Priority::Scan, Some(7), true, {
            let order = order.clone();
            move || order.lock().push(3)
        });
        sched.submit(Priority::Thumb, None, false, {
            let order = order.clone();
            move || order.lock().push(2)
        });
        let ran = Arc::new(AtomicBool::new(false));
        sched.submit(Priority::Scan, Some(9), true, {
            let ran = ran.clone();
            move || ran.store(true, Ordering::SeqCst)
        });
        sched.cancel_book(9);
        go_tx.send(()).ok();
        let deadline = Instant::now() + Duration::from_secs(2);
        while order.lock().len() < 3 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(order.lock().clone(), vec![1, 2, 3]);
        assert!(!ran.load(Ordering::SeqCst));
        sched.shutdown();
    }

    #[test]
    fn downscale_threshold_leaves_a_normal_page_alone() {
        assert!(!should_downscale(1600, 2400, 400_000));
        assert!(should_downscale(20_000, 20_000, 1000));
        assert!(should_downscale(0, 0, 50 * 1024 * 1024));
        assert!(Priority::Visible > Priority::Prefetch);
        assert!(Priority::Prefetch > Priority::Thumb);
        assert!(Priority::Thumb > Priority::Scan);
        assert_eq!(focused_book(), 0);
    }
}
