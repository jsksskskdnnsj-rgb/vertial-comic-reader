//! Read page list + dimensions from HEADERS ONLY
//! (split from design/claude-v04/reader_backend.rs `pub mod scan`).

use crate::error::ReaderError;
use crate::model::*;
use std::{
    fs::File,
    io::{BufReader, Cursor},
    path::{Path, PathBuf},
    sync::Mutex,
};

/// How many workers to use for header/stat probes. Small jobs stay on one thread.
pub(crate) fn io_workers(n: usize) -> usize {
    if n < 24 {
        return 1;
    }
    std::thread::available_parallelism()
        .map(|p| p.get())
        .unwrap_or(4)
        .clamp(2, 8)
        .min(n)
}

pub const HEAD_SMALL: usize = 64 * 1024; // tier 1
pub const HEAD_LARGE: usize = 1024 * 1024; // tier 2: JPEG with fat EXIF/ICC/thumbnail
pub const MAX_ENTRY: u64 = 256 * 1024 * 1024; // refuse zip bombs
pub const SLICE_TRIGGER: u32 = 16_000; // taller than this -> slice
pub const SLICE_MAX: u32 = 8_000; // each slice at most this tall
pub const MAX_PIXELS: u64 = 150_000_000; // refuse to decode beyond this

pub fn is_image(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    let base = l.rsplit('/').next().unwrap_or(&l);
    if base.starts_with('.') || l.starts_with("__macosx/") {
        return false;
    } // ._foo, .DS_Store
    matches!(
        base.rsplit('.').next(),
        Some("jpg" | "jpeg" | "jfif" | "jpe" | "png" | "webp" | "gif" | "avif" | "bmp")
    )
}

fn head_dims(buf: &[u8]) -> Option<(u32, u32)> {
    imagesize::blob_size(buf)
        .ok()
        .map(|s| (s.width as u32, s.height as u32))
}

fn read_up_to(
    ar: &mut zip::ZipArchive<BufReader<File>>,
    entry: u32,
    limit: u64,
) -> Option<(Vec<u8>, u64)> {
    let mut e = ar.by_index(entry as usize).ok()?; // seeks to local header, decompresses lazily
    let size = e.size();
    // A prefix only. The claimed uncompressed size is never used as a capacity.
    let buf = crate::pipeline::read_prefix(&mut e, limit).ok()?;
    Some((buf, size))
}

pub fn open_zip(path: &Path) -> Result<zip::ZipArchive<BufReader<File>>, ReaderError> {
    let opened = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        zip::ZipArchive::new(BufReader::with_capacity(64 * 1024, File::open(path)?))
            .map_err(ReaderError::from)
    }));
    match opened {
        Ok(r) => r, // central directory only
        Err(_) => Err(ReaderError::Corrupt("压缩包异常，已跳过".into())),
    }
}

fn zip_candidates(ar: &mut zip::ZipArchive<BufReader<File>>) -> Vec<(u32, String)> {
    // file_names() comes from the central directory: no per-entry I/O. enumerate() == entry index.
    let mut cands: Vec<(u32, String)> = ar
        .file_names()
        .enumerate()
        .filter(|(_, n)| !n.ends_with('/') && is_image(n))
        .map(|(i, n)| (i as u32, n.replace('\\', "/")))
        .collect();
    crate::shelf_scan::sort_by_comic_name(&mut cands, |c| c.1.as_str()); // 2.jpg before 10.jpg, 第一话 before 第2话
    cands
}

fn zip_header_key(ar: &mut zip::ZipArchive<BufReader<File>>, entry: u32, name: &str) -> Option<(String, u64)> {
    let e = ar.by_index(entry as usize).ok()?;
    let bytes = e.size();
    // crc32 + uncompressed size come from the central directory. A replaced
    // page misses; every other entry in the same archive still hits.
    Some((format!("z:{:08x}:{bytes}:{name}", e.crc32()), bytes))
}

fn probe_zip_entry(ar: &mut zip::ZipArchive<BufReader<File>>, entry: u32, name: String) -> RealPage {
    crate::pipeline::cooperate();
    let cached_key = zip_header_key(ar, entry, &name);
    if let Some((key, bytes)) = &cached_key {
        if let Some((w, h, stored)) = crate::header_cache::get(key) {
            return RealPage {
                entry,
                name,
                w,
                h,
                bytes: stored.max(*bytes),
                src_path: None,
            };
        }
    }
    let mut dims = None;
    let mut bytes = cached_key.as_ref().map(|(_, n)| *n).unwrap_or(0);
    for limit in [HEAD_SMALL as u64, HEAD_LARGE as u64] {
        let Some((buf, size)) = read_up_to(ar, entry, limit) else {
            break;
        };
        bytes = size;
        if let Some(d) = head_dims(&buf) {
            dims = Some(d);
            break;
        }
        if (buf.len() as u64) < limit {
            break;
        } // whole entry read, header still unparsable
    }
    if dims.is_none() {
        // tier 3: decoder reads dims (AVIF, odd files)
        if let Some((buf, _)) = read_up_to(ar, entry, MAX_ENTRY) {
            dims = image::ImageReader::new(Cursor::new(buf))
                .with_guessed_format()
                .ok()
                .and_then(|r| r.into_dimensions().ok());
        }
    }
    let (w, h) = dims.unwrap_or((0, 0)); // (0,0) = unknown; JS corrects on load
    if let Some((key, _)) = &cached_key {
        crate::header_cache::put(key, w, h, bytes);
    }
    RealPage {
        entry,
        name,
        w,
        h,
        bytes,
        src_path: None,
    }
}

/// One page list for every shelf kind.
///
/// Zip and cbz are read from the archive. A folder (including a book that was
/// a 7z/cb7/rar/cbr and unpacked at import) is a directory walk. Loose images
/// keep the saved path order. Callers do not grow a second scanner.
pub fn index_book(kind: SourceKind, paths: &[String]) -> Result<Vec<RealPage>, ReaderError> {
    match kind {
        SourceKind::Zip => {
            let path = paths.first().map(std::path::PathBuf::from).unwrap_or_default();
            scan_zip(&path)
        }
        SourceKind::Folder => {
            if paths.len() > 1 {
                Ok(scan_folder_parts(paths))
            } else {
                let path = paths.first().map(std::path::PathBuf::from).unwrap_or_default();
                Ok(scan_folder(&path))
            }
        }
        SourceKind::Images => Ok(scan_images(paths)),
    }
}

/// One archive, one thread. Used by the benchmark as the pre-parallel baseline.
pub(crate) fn scan_zip_linear(path: &Path) -> Result<Vec<RealPage>, ReaderError> {
    let mut ar = open_zip(path)?;
    let cands = zip_candidates(&mut ar);
    let mut out = Vec::with_capacity(cands.len());
    for (entry, name) in cands {
        out.push(probe_zip_entry(&mut ar, entry, name));
    }
    Ok(out)
}

pub fn scan_zip(path: &Path) -> Result<Vec<RealPage>, ReaderError> {
    let mut ar = open_zip(path)?;
    let cands = zip_candidates(&mut ar);
    let workers = io_workers(cands.len());
    if workers == 1 {
        let mut out = Vec::with_capacity(cands.len());
        for (entry, name) in cands {
            out.push(probe_zip_entry(&mut ar, entry, name));
        }
        return Ok(out);
    }
    drop(ar);
    let n = cands.len();
    let jobs: Vec<(usize, u32, String)> = cands
        .into_iter()
        .enumerate()
        .map(|(i, (entry, name))| (i, entry, name))
        .collect();
    let chunk = (n + workers - 1) / workers;
    let mut groups: Vec<Vec<(usize, u32, String)>> = Vec::new();
    let mut rest = jobs;
    while !rest.is_empty() {
        let take = chunk.min(rest.len());
        groups.push(rest.drain(..take).collect());
    }
    let slots: Mutex<Vec<Option<RealPage>>> = Mutex::new((0..n).map(|_| None).collect());
    let open_err: Mutex<Option<String>> = Mutex::new(None);
    std::thread::scope(|scope| {
        for group in groups {
            let path = path.to_path_buf();
            let slots = &slots;
            let open_err = &open_err;
            scope.spawn(move || {
                let mut ar = match open_zip(&path) {
                    Ok(a) => a,
                    Err(e) => {
                        *open_err.lock().unwrap_or_else(|err| err.into_inner()) = Some(e.to_string());
                        return;
                    }
                };
                let local: Vec<(usize, RealPage)> = group
                    .into_iter()
                    .map(|(i, entry, name)| (i, probe_zip_entry(&mut ar, entry, name)))
                    .collect();
                let mut guard = slots.lock().unwrap_or_else(|err| err.into_inner());
                for (i, page) in local {
                    guard[i] = Some(page);
                }
            });
        }
    });
    if let Some(msg) = open_err.into_inner().unwrap_or_else(|err| err.into_inner()) {
        return Err(ReaderError::Io(msg));
    }
    let mut out = Vec::with_capacity(n);
    for page in slots.into_inner().unwrap_or_else(|err| err.into_inner()) {
        out.push(page.ok_or_else(|| ReaderError::Io("zip scan incomplete".into()))?);
    }
    Ok(out)
}

pub fn count_images_recursive(root: &Path) -> usize {
    scan_folder(root).len()
}

/// One directory walk: image names in reading order, plus the folder signature
/// (`f{count}:{total_len}:{max_mtime}`) that `quick_sig` has always used.
/// Counting every file (not only images) keeps the signature comparable with
/// books already stored in the shelf.
fn walk_folder(root: &Path) -> (Vec<(String, u64)>, String) {
    let walked = crate::pipeline::walk_tree(root, &std::sync::atomic::AtomicBool::new(false), |_| {});
    let files = walked
        .images
        .into_iter()
        .map(|f| (f.rel, f.bytes))
        .collect();
    (
        files,
        format!("f{}:{}:{}", walked.files, walked.bytes, walked.max_mtime_ms),
    )
}

pub fn scan_folder(root: &Path) -> Vec<RealPage> {
    let (files, _) = walk_folder(root);
    probe_folder_pages(root, files)
}

/// How many real pages around the resume point to header-probe before the
/// reader is allowed to paint. The rest are filled in the background.
pub(crate) const OPEN_PROBE_BEHIND: usize = 4;
pub(crate) const OPEN_PROBE_AHEAD: usize = 24;

pub(crate) fn probe_range(n: usize, around: usize) -> std::ops::Range<usize> {
    if n == 0 {
        return 0..0;
    }
    let around = around.min(n - 1);
    let start = around.saturating_sub(OPEN_PROBE_BEHIND);
    let end = (around + 1 + OPEN_PROBE_AHEAD).min(n);
    start..end
}

/// Page list ready for `open_book`.
///
/// `full` reads every image header before returning (rescan, tests).
/// The interactive open probes only `probe_range` so the first screen has
/// real aspect ratios, and reports `pending` for the pages left at 0×0.
pub struct FreshIndex {
    pub pages: Vec<RealPage>,
    pub sig: String,
    pub pending: usize,
}

pub fn index_for_open(
    kind: SourceKind,
    paths: &[String],
    around: usize,
    full: bool,
) -> Result<FreshIndex, ReaderError> {
    match kind {
        SourceKind::Zip => index_zip_open(paths, around, full),
        SourceKind::Folder => index_folder_open(paths, around, full),
        SourceKind::Images => index_images_open(paths, around, full),
    }
}

fn blank_page(name: String, bytes: u64) -> RealPage {
    RealPage {
        entry: 0,
        name,
        w: 0,
        h: 0,
        bytes,
        src_path: None,
    }
}

fn zip_name_list(path: &Path) -> Result<Vec<RealPage>, ReaderError> {
    let mut ar = open_zip(path)?;
    let cands = zip_candidates(&mut ar);
    Ok(cands
        .into_iter()
        .map(|(entry, name)| RealPage {
            entry,
            name,
            w: 0,
            h: 0,
            bytes: 0,
            src_path: None,
        })
        .collect())
}

fn probe_zip_indices(
    path: &Path,
    pages: &mut [RealPage],
    range: std::ops::Range<usize>,
) -> Result<(), ReaderError> {
    if range.start >= range.end {
        return Ok(());
    }
    let mut ar = open_zip(path)?;
    for i in range {
        let entry = pages[i].entry;
        let name = pages[i].name.clone();
        pages[i] = probe_zip_entry(&mut ar, entry, name);
    }
    Ok(())
}

fn index_zip_open(paths: &[String], around: usize, full: bool) -> Result<FreshIndex, ReaderError> {
    let path = paths.first().map(PathBuf::from).unwrap_or_default();
    let sig = quick_sig(SourceKind::Zip, &path)?;
    if full {
        return Ok(FreshIndex {
            pages: scan_zip(&path)?,
            sig,
            pending: 0,
        });
    }
    let mut pages = zip_name_list(&path)?;
    let range = probe_range(pages.len(), around);
    let pending = pages.len().saturating_sub(range.len());
    probe_zip_indices(&path, &mut pages, range)?;
    Ok(FreshIndex { pages, sig, pending })
}

fn folder_part_prefix(root: &Path, index: usize, total: usize) -> String {
    let raw = root
        .file_name()
        .map(|s| s.to_string_lossy().replace(['/', '\\'], "／"))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("part{}", index + 1));
    if total > 1 {
        format!("{:02}-{}", index + 1, raw)
    } else {
        raw
    }
}

fn prefix_folder_pages(root: &Path, prefix: &str, pages: &mut [RealPage]) {
    for page in pages {
        let abs = root.join(&page.name);
        page.name = format!("{prefix}/{}", page.name);
        page.src_path = Some(abs.to_string_lossy().to_string());
    }
}

fn index_folder_open(paths: &[String], around: usize, full: bool) -> Result<FreshIndex, ReaderError> {
    // Signature covers every root, including ones that are not directories
    // (those contribute `f0:0:0`, matching `quick_sig` on an empty walk).
    // Pages are only taken from real directories, and a multi-root book
    // prefixes names the same way `scan_folder_parts` does.
    let mut sigs = Vec::with_capacity(paths.len());
    struct Part {
        root: PathBuf,
        files: Vec<(String, u64)>,
        prefix: Option<String>,
    }
    let mut parts = Vec::new();
    for (i, root_s) in paths.iter().enumerate() {
        let root = PathBuf::from(root_s);
        let (files, sig) = walk_folder(&root);
        sigs.push(sig);
        if root.is_dir() {
            let prefix = if paths.len() > 1 {
                Some(folder_part_prefix(&root, i, paths.len()))
            } else {
                None
            };
            parts.push(Part { root, files, prefix });
        }
    }
    let sig = if paths.len() > 1 {
        sigs.join("|")
    } else {
        sigs.pop().unwrap_or_else(|| "f0:0:0".into())
    };

    if full {
        let mut pages = Vec::new();
        for part in parts {
            let mut probed = probe_folder_pages(&part.root, part.files);
            if let Some(prefix) = &part.prefix {
                prefix_folder_pages(&part.root, prefix, &mut probed);
            }
            pages.extend(probed);
        }
        return Ok(FreshIndex { pages, sig, pending: 0 });
    }

    let mut listed: Vec<(PathBuf, String, u64, Option<String>)> = Vec::new();
    for part in parts {
        for (rel, bytes) in part.files {
            listed.push((part.root.clone(), rel, bytes, part.prefix.clone()));
        }
    }
    let n = listed.len();
    let range = probe_range(n, around);
    let pending = n.saturating_sub(range.len());
    let mut pages: Vec<RealPage> = listed
        .iter()
        .map(|(_, rel, bytes, _)| blank_page(rel.clone(), *bytes))
        .collect();
    // Probe each root's slice of the window with the original relative names,
    // then apply the multi-root prefix to every page.
    let mut cursor = 0;
    while cursor < n {
        let root = listed[cursor].0.clone();
        let mut end = cursor + 1;
        while end < n && listed[end].0 == root {
            end += 1;
        }
        let lo = cursor.max(range.start);
        let hi = end.min(range.end);
        if lo < hi {
            let files: Vec<(String, u64)> = (lo..hi)
                .map(|i| (listed[i].1.clone(), listed[i].2))
                .collect();
            let probed = probe_folder_pages(&root, files);
            for (page, slot) in probed.into_iter().zip(lo..hi) {
                pages[slot] = page;
            }
        }
        cursor = end;
    }
    for (i, item) in listed.iter().enumerate() {
        if let Some(prefix) = &item.3 {
            prefix_folder_pages(&item.0, prefix, &mut pages[i..i + 1]);
        }
    }
    Ok(FreshIndex { pages, sig, pending })
}

fn index_images_open(paths: &[String], around: usize, full: bool) -> Result<FreshIndex, ReaderError> {
    let sig = "i".into();
    if full {
        return Ok(FreshIndex {
            pages: scan_images(paths),
            sig,
            pending: 0,
        });
    }
    let mut pages: Vec<RealPage> = paths
        .iter()
        .map(|p| {
            let bytes = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            blank_page(p.clone(), bytes)
        })
        .collect();
    let range = probe_range(pages.len(), around);
    let pending = pages.len().saturating_sub(range.len());
    for i in range {
        let name = pages[i].name.clone();
        let bytes = pages[i].bytes;
        pages[i] = loose_page(name, bytes);
    }
    Ok(FreshIndex { pages, sig, pending })
}

fn folder_page(root: &Path, name: String, bytes: u64) -> RealPage {
    let full = root.join(&name);
    let key = crate::header_cache::file_key(&full);
    if let Some(key) = &key {
        if let Some((w, h, stored)) = crate::header_cache::get(key) {
            return RealPage {
                entry: 0,
                name,
                w,
                h,
                bytes: if bytes == 0 { stored } else { bytes },
                src_path: None,
            };
        }
    }
    crate::pipeline::cooperate();
    let (w, h) = crate::pipeline::image_dims(&full);
    if let Some(key) = &key {
        crate::header_cache::put(key, w, h, bytes);
    }
    RealPage {
        entry: 0,
        name,
        w,
        h,
        bytes,
        src_path: None,
    }
}

fn probe_folder_pages(root: &Path, files: Vec<(String, u64)>) -> Vec<RealPage> {
    let workers = io_workers(files.len());
    if workers == 1 {
        return files
            .into_iter()
            .map(|(name, bytes)| folder_page(root, name, bytes))
            .collect();
    }
    let n = files.len();
    let jobs: Vec<(usize, String, u64)> = files
        .into_iter()
        .enumerate()
        .map(|(i, (name, bytes))| (i, name, bytes))
        .collect();
    let chunk = (n + workers - 1) / workers;
    let mut groups: Vec<Vec<(usize, String, u64)>> = Vec::new();
    let mut rest = jobs;
    while !rest.is_empty() {
        let take = chunk.min(rest.len());
        groups.push(rest.drain(..take).collect());
    }
    let slots: Mutex<Vec<Option<RealPage>>> = Mutex::new((0..n).map(|_| None).collect());
    let root = root.to_path_buf();
    std::thread::scope(|scope| {
        for group in groups {
            let root = root.clone();
            let slots = &slots;
            scope.spawn(move || {
                let local: Vec<(usize, RealPage)> = group
                    .into_iter()
                    .map(|(i, name, bytes)| (i, folder_page(&root, name, bytes)))
                    .collect();
                let mut guard = slots.lock().unwrap_or_else(|err| err.into_inner());
                for (i, page) in local {
                    guard[i] = Some(page);
                }
            });
        }
    });
    slots
        .into_inner()
        .unwrap_or_else(|err| err.into_inner())
        .into_iter()
        .map(|page| page.expect("folder probe"))
        .collect()
}

/// Several folder roots, in reading order, as one book.
/// Each page name is prefixed with the folder name so folder-chapters stay distinct,
/// and `src_path` is the absolute file so the extractor does not look under paths[0] only.
pub fn scan_folder_parts(roots: &[String]) -> Vec<RealPage> {
    let mut out = Vec::new();
    for (i, root_s) in roots.iter().enumerate() {
        let root = Path::new(root_s);
        if !root.is_dir() {
            continue;
        }
        let raw = root
            .file_name()
            .map(|s| s.to_string_lossy().replace(['/', '\\'], "／"))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("part{}", i + 1));
        let part = if roots.len() > 1 {
            format!("{:02}-{}", i + 1, raw)
        } else {
            raw
        };
        for mut page in scan_folder(root) {
            let abs = root.join(&page.name);
            page.name = format!("{part}/{}", page.name);
            page.src_path = Some(abs.to_string_lossy().to_string());
            out.push(page);
        }
    }
    out
}

/// Loose images: `paths` come from the DB (user's drop order), name = absolute path.
pub fn scan_images(paths: &[String]) -> Vec<RealPage> {
    let files: Vec<(String, u64)> = paths
        .iter()
        .map(|p| {
            let bytes = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
            (p.clone(), bytes)
        })
        .collect();
    // Loose images keep the caller's order. Dimensions are read in parallel past the small-job cutoff.
    let workers = io_workers(files.len());
    if workers == 1 {
        return files
            .into_iter()
            .map(|(name, bytes)| loose_page(name, bytes))
            .collect();
    }
    let n = files.len();
    let jobs: Vec<(usize, String, u64)> = files
        .into_iter()
        .enumerate()
        .map(|(i, (name, bytes))| (i, name, bytes))
        .collect();
    let chunk = (n + workers - 1) / workers;
    let mut groups: Vec<Vec<(usize, String, u64)>> = Vec::new();
    let mut rest = jobs;
    while !rest.is_empty() {
        let take = chunk.min(rest.len());
        groups.push(rest.drain(..take).collect());
    }
    let slots: Mutex<Vec<Option<RealPage>>> = Mutex::new((0..n).map(|_| None).collect());
    std::thread::scope(|scope| {
        for group in groups {
            let slots = &slots;
            scope.spawn(move || {
                let local: Vec<(usize, RealPage)> = group
                    .into_iter()
                    .map(|(i, name, bytes)| (i, loose_page(name, bytes)))
                    .collect();
                let mut guard = slots.lock().unwrap_or_else(|err| err.into_inner());
                for (i, page) in local {
                    guard[i] = Some(page);
                }
            });
        }
    });
    slots
        .into_inner()
        .unwrap_or_else(|err| err.into_inner())
        .into_iter()
        .map(|page| page.expect("image probe"))
        .collect()
}

fn loose_page(name: String, bytes: u64) -> RealPage {
    let path = Path::new(&name);
    let key = crate::header_cache::file_key(path);
    if let Some(key) = &key {
        if let Some((w, h, stored)) = crate::header_cache::get(key) {
            return RealPage {
                entry: 0,
                name,
                w,
                h,
                bytes: if bytes == 0 { stored } else { bytes },
                src_path: None,
            };
        }
    }
    crate::pipeline::cooperate();
    let (w, h) = crate::pipeline::image_dims(path);
    if let Some(key) = &key {
        crate::header_cache::put(key, w, h, bytes);
    }
    RealPage {
        entry: 0,
        name,
        w,
        h,
        bytes,
        src_path: None,
    }
}

/// n equal slices, each <= SLICE_MAX, for images taller than SLICE_TRIGGER. Returns (y0, h).
pub fn plan_slices(h: u32) -> Vec<(u32, u32)> {
    if h <= SLICE_TRIGGER {
        return vec![(0, h)];
    }
    let n = h.div_ceil(SLICE_MAX);
    let sh = h.div_ceil(n);
    (0..n)
        .map(|k| {
            let y0 = k * sh;
            (y0, sh.min(h - y0))
        })
        .collect()
}

/// Real pages -> virtual pages. PURE: re-run on every open, nothing derived is stored,
/// so fixing a wrong dimension later automatically changes the slicing.
pub fn build_pages(real: &[RealPage]) -> Vec<PageMeta> {
    let mut out = Vec::new();
    for (g, r) in real.iter().enumerate() {
        let sliceable = r.h > SLICE_TRIGGER && (r.w as u64 * r.h as u64) <= MAX_PIXELS;
        let slices = if sliceable {
            plan_slices(r.h)
        } else {
            vec![(0, r.h)]
        };
        for (y0, h) in slices {
            out.push(PageMeta {
                src_page: g as u32,
                entry: r.entry,
                name: r.name.clone(),
                w: r.w,
                h,
                full_h: r.h,
                y0,
                src_path: r.src_path.clone(),
            });
        }
    }
    out
}

pub fn groups(pages: &[PageMeta]) -> Vec<(u32, u32)> {
    let mut g: Vec<(u32, u32)> = Vec::new();
    for (i, p) in pages.iter().enumerate() {
        if p.src_page as usize == g.len() {
            g.push((i as u32, i as u32));
        } else {
            g[p.src_page as usize].1 = i as u32;
        }
    }
    g
}

/// Chapters = change of parent directory between consecutive real pages.
pub fn chapters(real: &[RealPage]) -> Vec<Chapter> {
    let mut out: Vec<Chapter> = Vec::new();
    for (g, r) in real.iter().enumerate() {
        let dir = r
            .name
            .rsplit_once('/')
            .map(|x| x.0.to_string())
            .unwrap_or_default();
        if !dir.is_empty() && out.last().map(|c| c.title != dir).unwrap_or(true) {
            out.push(Chapter {
                page: g as u32,
                title: dir,
            });
        }
    }
    if out.len() < 2 {
        out.clear();
    } // a single folder is not a chapter list
    out
}

/// Cheap change detector. Zip: size:mtime. Folder: count:total_len:max_mtime (one metadata walk).
/// Images: constant (loose-image sets are not change-detected; INTEGRATION_NOTES §3).
pub fn quick_sig(kind: SourceKind, path: &Path) -> Result<String, ReaderError> {
    let ms = |m: &std::fs::Metadata| {
        m.modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis())
            .unwrap_or(0)
    };
    match kind {
        SourceKind::Zip => {
            let m = std::fs::metadata(path)?;
            Ok(format!("z{}:{}", m.len(), ms(&m)))
        }
        SourceKind::Folder => Ok(walk_folder(path).1),
        SourceKind::Images => Ok("i".into()), // loose-image books: caller hashes the path list if needed
    }
}

#[cfg(test)]
mod bench_tests {
    use super::*;
    use std::io::{Cursor, Write};
    use std::time::Instant;

    fn note(line: &str) {
        eprintln!("{line}");
        let path = std::env::temp_dir().join("mh-bench.txt");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{line}");
        }
    }

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 40]));
        let mut buf = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut buf), image::ImageFormat::Jpeg)
            .unwrap();
        buf
    }

    fn write_zip(path: &std::path::Path, prefix: &str, jpeg: &[u8], n: u32) {
        let f = std::fs::File::create(path).unwrap();
        let mut zw = zip::ZipWriter::new(f);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for i in (1..=n).rev() {
            zw.start_file(format!("{prefix}{i:03}.jpg"), opts).unwrap();
            zw.write_all(jpeg).unwrap();
        }
        zw.finish().unwrap();
    }

    fn names_of(pages: &[RealPage]) -> Vec<&str> {
        pages.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn zip_and_folder_share_one_index_and_header_cache() {
        let dir = std::env::temp_dir().join(format!("mh-index-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let jpeg = jpeg(24, 36);
        let zip_path = dir.join("book.cbz");
        write_zip(&zip_path, "", &jpeg, 40);
        let folder = dir.join("folder");
        std::fs::create_dir_all(&folder).unwrap();
        for i in (1..=40).rev() {
            std::fs::write(folder.join(format!("{i:03}.jpg")), &jpeg).unwrap();
        }
        let (hits_before, _) = crate::header_cache::stats();
        let t_cold = Instant::now();
        let linear = scan_zip_linear(&zip_path).unwrap();
        let cold_ms = t_cold.elapsed().as_millis();
        let t_warm = Instant::now();
        let warm = scan_zip_linear(&zip_path).unwrap();
        let warm_ms = t_warm.elapsed().as_millis();
        let via_index = index_book(SourceKind::Zip, &[zip_path.to_string_lossy().to_string()]).unwrap();
        assert_eq!(linear.len(), 40);
        assert_eq!(linear[0].name, "001.jpg");
        assert_eq!(linear.last().unwrap().name, "040.jpg");
        assert!(linear[0].w > 0 && linear[0].h > 0);
        for (a, b) in linear.iter().zip(warm.iter()) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.entry, b.entry);
            assert_eq!((a.w, a.h), (b.w, b.h));
        }
        assert_eq!(names_of(&linear), names_of(&via_index));
        let (hits_after, _) = crate::header_cache::stats();
        assert!(hits_after > hits_before, "second zip read should use the header cache");
        note(&format!("BENCH zip40 cold_linear_ms={cold_ms} warm_linear_ms={warm_ms} header_hits={}", hits_after - hits_before));

        let par_zip = dir.join("par.cbz");
        let lin_zip = dir.join("lin.cbz");
        write_zip(&par_zip, "p", &jpeg, 300);
        write_zip(&lin_zip, "q", &jpeg, 300);
        let t_par = Instant::now();
        let parallel = scan_zip(&par_zip).unwrap();
        let par_ms = t_par.elapsed().as_millis();
        let t_lin = Instant::now();
        let linear300 = scan_zip_linear(&lin_zip).unwrap();
        let lin_ms = t_lin.elapsed().as_millis();
        assert_eq!(parallel.len(), 300);
        assert_eq!(linear300.len(), 300);
        assert_eq!(parallel[0].name, "p001.jpg");
        assert_eq!(parallel.last().unwrap().name, "p300.jpg");
        assert_eq!(linear300[0].name, "q001.jpg");
        for (a, b) in parallel.iter().zip(linear300.iter()) {
            assert_eq!(a.w, b.w);
            assert_eq!(a.h, b.h);
            assert_eq!(a.entry, b.entry);
        }
        note(&format!("BENCH zip300 parallel_ms={par_ms} linear_ms={lin_ms}"));

        let t_folder = Instant::now();
        let folder_pages = index_book(SourceKind::Folder, &[folder.to_string_lossy().to_string()]).unwrap();
        let folder_cold = t_folder.elapsed().as_millis();
        let t_folder2 = Instant::now();
        let folder_warm = scan_folder(&folder);
        let folder_warm_ms = t_folder2.elapsed().as_millis();
        assert_eq!(folder_pages.len(), 40);
        assert_eq!(folder_pages[0].name, "001.jpg");
        assert_eq!(folder_pages.last().unwrap().name, "040.jpg");
        assert_eq!(names_of(&folder_pages), names_of(&folder_warm));
        note(&format!("BENCH folder40 cold_ms={folder_cold} warm_ms={folder_warm_ms}"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn interactive_index_returns_before_the_tail_is_probed() {
        let dir = std::env::temp_dir().join(format!("mh-fast-open-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = dir.join("a");
        let b = dir.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let jpeg = jpeg(20, 28);
        for i in 1..=40 {
            let name = format!("{i:03}.jpg");
            std::fs::write(a.join(&name), &jpeg).unwrap();
            std::fs::write(b.join(&name), &jpeg).unwrap();
        }
        std::fs::write(a.join("notes.txt"), b"not a page").unwrap();
        std::fs::write(b.join("notes.txt"), b"not a page").unwrap();

        let t_fast = Instant::now();
        let fast = index_for_open(SourceKind::Folder, &[a.to_string_lossy().to_string()], 0, false).unwrap();
        let fast_ms = t_fast.elapsed().as_millis();
        let t_full = Instant::now();
        let full = index_for_open(SourceKind::Folder, &[b.to_string_lossy().to_string()], 0, true).unwrap();
        let full_ms = t_full.elapsed().as_millis();

        assert_eq!(fast.pages.len(), 40);
        assert_eq!(full.pages.len(), 40);
        assert!(fast.pending > 0, "tail should still be pending");
        assert_eq!(full.pending, 0);
        assert!(fast.pages[0].w > 0 && fast.pages[0].h > 0, "first page is probed");
        assert_eq!(fast.pages[35].w, 0, "pages past the window stay unprobed");
        assert!(full.pages[35].w > 0 && full.pages[35].h > 0);
        assert_eq!(fast.sig, quick_sig(SourceKind::Folder, &a).unwrap());
        assert_eq!(full.sig, quick_sig(SourceKind::Folder, &b).unwrap());
        assert_ne!(fast.sig, "f40:0:0", "non-image files count toward the signature");
        assert_eq!(names_of(&full.pages), names_of(&scan_folder(&b)));
        if full_ms > 20 {
            assert!(
                fast_ms * 2 < full_ms,
                "interactive {fast_ms}ms should beat a full probe {full_ms}ms"
            );
        }
        note(&format!("BENCH open-index fast_ms={fast_ms} full_ms={full_ms} pending={}", fast.pending));

        let multi = index_for_open(
            SourceKind::Folder,
            &[a.to_string_lossy().to_string(), b.to_string_lossy().to_string()],
            0,
            true,
        )
        .unwrap();
        let expect_sig = format!(
            "{}|{}",
            quick_sig(SourceKind::Folder, &a).unwrap(),
            quick_sig(SourceKind::Folder, &b).unwrap()
        );
        assert_eq!(multi.sig, expect_sig);
        assert_eq!(multi.pages.len(), 80);
        assert!(multi.pages[0].name.contains('/'));
        assert!(multi.pages[0].src_path.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
