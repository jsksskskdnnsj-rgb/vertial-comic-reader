//! Read page list + dimensions from HEADERS ONLY
//! (split from design/claude-v04/reader_backend.rs `pub mod scan`).

use crate::error::ReaderError;
use crate::model::*;
use std::{
    fs::File,
    io::{BufReader, Cursor, Read},
    path::Path,
};

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
        Some("jpg" | "jpeg" | "png" | "webp" | "gif" | "avif" | "bmp")
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

pub fn scan_zip(path: &Path) -> Result<Vec<RealPage>, ReaderError> {
    let mut ar = open_zip(path)?;
    // file_names() comes from the central directory: no per-entry I/O. enumerate() == entry index.
    let mut cands: Vec<(u32, String)> = ar
        .file_names()
        .enumerate()
        .filter(|(_, n)| !n.ends_with('/') && is_image(n))
        .map(|(i, n)| (i as u32, n.replace('\\', "/")))
        .collect();
    cands.sort_by(|a, b| natord::compare_ignore_case(&a.1, &b.1)); // 2.jpg before 10.jpg

    let mut out = Vec::with_capacity(cands.len());
    for (entry, name) in cands {
        let mut dims = None;
        let mut bytes = 0;
        for limit in [HEAD_SMALL as u64, HEAD_LARGE as u64] {
            let Some((buf, size)) = read_up_to(&mut ar, entry, limit) else {
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
            if let Some((buf, _)) = read_up_to(&mut ar, entry, MAX_ENTRY) {
                dims = image::ImageReader::new(Cursor::new(buf))
                    .with_guessed_format()
                    .ok()
                    .and_then(|r| r.into_dimensions().ok());
            }
        }
        let (w, h) = dims.unwrap_or((0, 0)); // (0,0) = unknown; JS corrects on load
        out.push(RealPage {
            entry,
            name,
            w,
            h,
            bytes,
            src_path: None,
        });
    }
    Ok(out)
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
    files.sort_by(|a, b| natord::compare_ignore_case(&a.0, &b.0));
    files
        .into_iter()
        .map(|(name, bytes)| {
            let (w, h) = imagesize::size(root.join(&name))
                .map(|s| (s.width as u32, s.height as u32))
                .unwrap_or((0, 0));
            RealPage {
                entry: 0,
                name,
                w,
                h,
                bytes,
                src_path: None,
            }
        })
        .collect()
}

/// Loose images: `paths` come from the DB (user's drop order), name = absolute path.
pub fn scan_images(paths: &[String]) -> Vec<RealPage> {
    paths
        .iter()
        .map(|p| {
            let (w, h) = imagesize::size(p)
                .map(|s| (s.width as u32, s.height as u32))
                .unwrap_or((0, 0));
            RealPage {
                entry: 0,
                name: p.clone(),
                w,
                h,
                bytes: std::fs::metadata(p).map(|m| m.len()).unwrap_or(0),
                src_path: None,
            }
        })
        .collect()
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
