//! Read page list + dimensions from HEADERS ONLY
//! (split from design/claude-v04/reader_backend.rs `pub mod scan`).

use crate::error::ReaderError;
use crate::model::*;
use std::{
    fs::File,
    io::{BufReader, Cursor, Read},
    path::Path,
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
    let mut buf = Vec::with_capacity(limit.min(size) as usize);
    e.by_ref().take(limit).read_to_end(&mut buf).ok()?;
    Some((buf, size))
}

pub fn open_zip(path: &Path) -> Result<zip::ZipArchive<BufReader<File>>, ReaderError> {
    Ok(zip::ZipArchive::new(BufReader::with_capacity(
        64 * 1024,
        File::open(path)?,
    ))?) // central directory only
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

pub fn scan_folder(root: &Path) -> Vec<RealPage> {
    let mut files: Vec<(String, u64)> = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| {
            let rel = e
                .path()
                .strip_prefix(root)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            if is_image(&rel) {
                Some((rel, e.metadata().ok()?.len()))
            } else {
                None
            }
        })
        .collect();
    crate::shelf_scan::sort_by_comic_name(&mut files, |f| f.0.as_str());
    probe_folder_pages(root, files)
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
    let (w, h) = imagesize::size(&full)
        .map(|s| (s.width as u32, s.height as u32))
        .unwrap_or((0, 0));
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
    let (w, h) = imagesize::size(path)
        .map(|s| (s.width as u32, s.height as u32))
        .unwrap_or((0, 0));
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
        SourceKind::Folder => {
            let (mut n, mut len, mut mx) = (0u64, 0u64, 0u128);
            for e in walkdir::WalkDir::new(path)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|e| e.file_type().is_file())
            {
                if let Ok(m) = e.metadata() {
                    n += 1;
                    len += m.len();
                    mx = mx.max(ms(&m));
                }
            }
            Ok(format!("f{n}:{len}:{mx}"))
        }
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
}
