//! Covers for Home/Library lists (ported from design/claude-v04/cover_protocol.rs).
//!
//! Why a second protocol: `mhpage` only serves books that are OPEN in the reader (the extractor keeps
//! two BookMeta in RAM). Lists show dozens of covers, so they get persisted 240px JPEG files:
//!     {data}/covers/{book_id}.jpg      URL: http://mhcover.localhost/{book_id}?v={coverVer}
//!
//! `serve_cover` / `cover_handle` / `CoversDir` are ported as written in the design.
//! `ensure_cover` internals are REWRITTEN per INTEGRATION_NOTES §3.3: they do NOT use
//! the extractor. The first page's bytes come from the verified machinery instead:
//!   zip    -> open the archive directly with zip::ZipArchive, first image entry
//!   folder -> first file of crate::list_images (walk_dir)
//!   images -> paths[0] file bytes
//! then decode with the `image` crate, downscale to width 240, encode JPEG, write
//! atomically (.part + rename), and bump shelf.cover_ver.
//! The generation routine is factored as `generate_cover` (pub(crate)) so the import
//! pipeline (crate::adapters::import_paths) triggers the exact same code path.

use crate::commands::ReaderState;
use crate::error::ReaderError;
use crate::extractor::sniff;
use std::io::Read as _;
use std::{path::Path, path::PathBuf, sync::Arc};
use tauri::State;

pub struct CoversDir(pub Arc<PathBuf>);

/// Cover candidates must be formats the `image` crate can decode (`DECODE_EXTS`).
/// AVIF pages still import (the webview displays them as-is) but cannot become a JPEG cover.
const COVER_EXTS: &[&str] = crate::DECODE_EXTS;
const MAX_COVER_TRIES: usize = 4;

/// Cap on a single cover source read (a first page larger than this is not a cover candidate).
const MAX_COVER_SRC: u64 = 128 * 1024 * 1024; // 128 MiB
/// Covers are stored 240 px wide (matches model::VARIANTS' 240 entry).
const COVER_WIDTH: u32 = 240;

fn is_cover_name(name: &str) -> bool {
    let l = name.to_ascii_lowercase();
    let base = l.rsplit('/').next().unwrap_or(&l);
    if base.starts_with('.') || l.starts_with("__macosx/") {
        return false;
    }
    match base.rsplit('.').next() {
        Some(ext) => COVER_EXTS.contains(&ext),
        None => false,
    }
}

/// Register: .register_asynchronous_uri_scheme_protocol("mhcover", |ctx, req, responder| { ... cover_handle(dir, req, responder) })
pub fn serve_cover(dir: &PathBuf, req: &http::Request<Vec<u8>>) -> http::Response<Vec<u8>> {
    let id = req.uri().path().trim_matches('/').parse::<i64>().ok();
    let bytes = id.and_then(|id| std::fs::read(dir.join(format!("{id}.jpg"))).ok()); // id is parsed to i64: no path injection
    let base = http::Response::builder()
        .header("Access-Control-Allow-Origin", "*")
        .header("X-Content-Type-Options", "nosniff");
    match bytes {
        Some(b) => {
            let mime = sniff(&b).map(|m| m.0).unwrap_or("image/jpeg");
            base.status(200)
                .header("Content-Type", mime)
                .header("Cache-Control", "private, max-age=31536000, immutable")
                .body(b)
                .unwrap() // ?v= busts it
        }
        None => base.status(404).body(Vec::new()).unwrap(), // home.js reacts to 404 by calling ensure_cover once
    }
}

pub fn cover_handle(
    dir: Arc<PathBuf>,
    req: http::Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
) {
    tauri::async_runtime::spawn(async move {
        let resp = tauri::async_runtime::spawn_blocking(move || serve_cover(&dir, &req))
            .await
            .unwrap_or_else(|_| {
                http::Response::builder()
                    .status(500)
                    .body(Vec::new())
                    .unwrap()
            });
        responder.respond(resp);
    });
}

/// Up to `MAX_COVER_TRIES` early pages, from the verified machinery (NOT the extractor).
/// The first one `image` can decode becomes the cover; AVIF-only books yield an empty list.
fn cover_candidates(book_id: i64) -> Result<Vec<Vec<u8>>, ReaderError> {
    let conn = crate::db_conn().map_err(ReaderError::Io)?;
    let row = crate::db::book_row(&conn, book_id)?;
    match row.kind.as_str() {
        "zip" => {
            let src = row
                .paths
                .first()
                .cloned()
                .ok_or_else(|| ReaderError::Corrupt("zip book without path".into()))?;
            let file = std::fs::File::open(&src)?;
            let mut ar = zip::ZipArchive::new(file)?;
            // First image entry in natural (reading) order — same ordering scan_zip
            // uses. Keep the raw index: entry names are read via by_index because the
            // normalised name may differ from the stored one (backslashes).
            let mut cands: Vec<(usize, String)> = ar
                .file_names()
                .enumerate()
                .filter(|(_, n)| !n.ends_with('/') && is_cover_name(n))
                .map(|(i, n)| (i, n.replace('\\', "/")))
                .collect();
            cands.sort_by(|a, b| natord::compare_ignore_case(&a.1, &b.1));
            let mut out = Vec::new();
            for (index, _) in cands {
                if out.len() >= MAX_COVER_TRIES {
                    break;
                }
                let mut e = ar.by_index(index)?;
                if e.size() > MAX_COVER_SRC {
                    continue;
                }
                let mut buf = Vec::with_capacity(e.size() as usize);
                e.read_to_end(&mut buf)?;
                if !buf.is_empty() {
                    out.push(buf);
                }
            }
            Ok(out)
        }
        "folder" => {
            let src = row
                .paths
                .first()
                .cloned()
                .ok_or_else(|| ReaderError::Corrupt("folder book without path".into()))?;
            // crate::list_images returns walk_dir order (not naturally sorted), so
            // sort by display name the same way scan_folder does.
            let mut files = crate::list_images(src).map_err(ReaderError::Io)?;
            files.retain(|f| is_cover_name(&f.name));
            files.sort_by(|a, b| natord::compare_ignore_case(&a.name, &b.name));
            let mut out = Vec::new();
            for f in files {
                if out.len() >= MAX_COVER_TRIES {
                    break;
                }
                if std::fs::metadata(&f.path).map(|m| m.len()).unwrap_or(0) > MAX_COVER_SRC {
                    continue;
                }
                if let Ok(buf) = std::fs::read(&f.path) {
                    if !buf.is_empty() {
                        out.push(buf);
                    }
                }
            }
            Ok(out)
        }
        "images" => {
            let mut out = Vec::new();
            for path in &row.paths {
                if out.len() >= MAX_COVER_TRIES {
                    break;
                }
                if !is_cover_name(path) {
                    continue;
                }
                if std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) > MAX_COVER_SRC {
                    continue;
                }
                if let Ok(buf) = std::fs::read(path) {
                    if !buf.is_empty() {
                        out.push(buf);
                    }
                }
            }
            Ok(out)
        }
        other => Err(ReaderError::Unsupported(other.to_string())),
    }
}

/// Generate covers/{book_id}.jpg if missing: decode an early page, downscale to
/// COVER_WIDTH px wide, encode JPEG, write atomically (.part + rename), then bump
/// shelf.cover_ver. No-op Ok when the jpg already exists or no candidate decodes
/// (AVIF-only books: the pages still display, but this crate has no AVIF decoder).
/// Shared by the `ensure_cover` command and the import pipeline (adapters).
pub(crate) fn generate_cover(covers_dir: &Path, book_id: i64) -> Result<(), ReaderError> {
    let out = covers_dir.join(format!("{book_id}.jpg"));
    if out.exists() {
        return Ok(());
    }
    let img = cover_candidates(book_id)?.into_iter().find_map(|bytes| {
        image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .ok()
            .and_then(|r| r.decode().ok())
    });
    let Some(img) = img else {
        return Ok(());
    };
    let img = if img.width() > COVER_WIDTH {
        let nh = ((img.height() as u64 * COVER_WIDTH as u64) / img.width() as u64).max(1) as u32;
        img.resize_exact(
            COVER_WIDTH,
            nh,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        img
    };
    let mut jpg: Vec<u8> = Vec::new();
    {
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 85);
        image::DynamicImage::ImageRgb8(img.to_rgb8())
            .write_with_encoder(enc)
            .map_err(ReaderError::from)?;
    }
    std::fs::create_dir_all(covers_dir)?;
    let tmp = out.with_extension("part");
    std::fs::write(&tmp, &jpg)?;
    std::fs::rename(&tmp, &out)?; // atomic; readers never see half a file
    // Bump cover_ver so mhcover URLs (?v=coverVer) bust caches on regeneration.
    let conn = crate::db_conn().map_err(ReaderError::Io)?;
    conn.execute(
        "UPDATE shelf SET cover_ver = cover_ver + 1 WHERE id = ?1",
        rusqlite::params![book_id],
    )
    .map_err(ReaderError::from)?;
    Ok(())
}

/// Drop the oldest cover files until the directory is under `limit_bytes` (and then
/// under 90% of that, so one new cover does not immediately evict another).
/// Missing directories and unreadable entries are skipped.
pub(crate) fn trim_covers(dir: &std::path::Path, limit_bytes: u64) {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };
    let mut files = Vec::new();
    let mut total = 0u64;
    for ent in rd.flatten() {
        let Ok(meta) = ent.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let len = meta.len();
        total = total.saturating_add(len);
        let modified = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        files.push((ent.path(), len, modified));
    }
    if total <= limit_bytes {
        return;
    }
    files.sort_by_key(|f| f.2);
    let target = limit_bytes.saturating_mul(9) / 10;
    for (path, len, _) in files {
        if total <= target {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

/// Generate covers/{id}.jpg if missing. Called lazily from home.js on a cover 404,
/// and (via generate_cover) from the import pipeline right after inserting a book.
#[tauri::command]
pub async fn ensure_cover(
    _state: State<'_, ReaderState>,
    covers: State<'_, CoversDir>,
    book_id: i64,
) -> Result<(), ReaderError> {
    let dir = covers.0.clone();
    tauri::async_runtime::spawn_blocking(move || generate_cover(&dir, book_id))
        .await
        .map_err(|e| ReaderError::Io(e.to_string()))?
}
