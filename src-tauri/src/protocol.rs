//! mhpage:// protocol
//! (split from design/claude-v04/reader_backend.rs `pub mod protocol`, unchanged).
//!
//! Windows/WebView2 URL:  http://mhpage.localhost/{book_id}/{virtual_idx}?v={variant}&s={sig}[&r={retry}]

use crate::error::ReaderError;
use crate::extractor::Extractor;
use crate::model::*;
use std::sync::Arc;

fn parse(uri: &http::Uri) -> Result<Key, ReaderError> {
    let segs: Vec<&str> = uri.path().trim_matches('/').split('/').collect();
    if segs.len() != 2 {
        return Err(ReaderError::BadRequest("expected /book/idx".into()));
    }
    let book = segs[0]
        .parse::<i64>()
        .map_err(|_| ReaderError::BadRequest("book id".into()))?;
    let idx = segs[1]
        .parse::<u32>()
        .map_err(|_| ReaderError::BadRequest("page idx".into()))?;
    let variant = uri
        .query()
        .unwrap_or("")
        .split('&')
        .find_map(|kv| kv.strip_prefix("v="))
        .map(|v| v.parse::<u16>())
        .transpose()
        .map_err(|_| ReaderError::BadRequest("variant".into()))?
        .unwrap_or(0);
    if !VARIANTS.contains(&variant) {
        return Err(ReaderError::BadRequest("variant not allowed".into()));
    }
    Ok((book, idx, variant)) // NOTE: nothing from the URI ever becomes a filesystem path
}

fn base() -> http::response::Builder {
    http::Response::builder()
        .header("Access-Control-Allow-Origin", "*") // page origin (tauri.localhost) != mhpage.localhost
        .header("X-Content-Type-Options", "nosniff")
}
fn err(e: &ReaderError) -> http::Response<Vec<u8>> {
    base()
        .status(e.http_status())
        .header("Content-Type", "application/json")
        .header("x-mh-error", e.code())
        .body(serde_json::to_vec(e).unwrap_or_default())
        .unwrap()
}

/// BLOCKING (call inside spawn_blocking). Tauri v2 response bodies are whole buffers - there is no
/// chunked streaming - so a page is read fully; pages are <= a few MB, slices keep it bounded.
pub fn serve(ex: &Extractor, req: &http::Request<Vec<u8>>) -> http::Response<Vec<u8>> {
    if req.method() == http::Method::OPTIONS {
        return base()
            .status(204)
            .header("Access-Control-Allow-Methods", "GET")
            .body(vec![])
            .unwrap();
    }
    let key = match parse(req.uri()) {
        Ok(k) => k,
        Err(e) => return err(&e),
    };
    for attempt in 0..2 {
        let page = match ex.get_or_extract(key) {
            Ok(p) => p,
            Err(e) => return err(&e),
        };
        match std::fs::read(&page.path) {
            Ok(bytes) => {
                return base()
                    .status(200)
                    .header("Content-Type", page.mime)
                    .header("Content-Length", bytes.len())
                    .header("Cache-Control", "private, max-age=31536000, immutable") // URL carries sig; harmless if ignored
                    .body(bytes)
                    .unwrap()
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && attempt == 0 => {
                ex.cache.remove(&key)
            } // index lied: re-extract once
            Err(e) => return err(&e.into()),
        }
    }
    err(&ReaderError::Io("cache read failed".into()))
}

pub fn handle(
    ex: Arc<Extractor>,
    req: http::Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
) {
    tauri::async_runtime::spawn(async move {
        let resp = tauri::async_runtime::spawn_blocking(move || serve(&ex, &req))
            .await
            .unwrap_or_else(|_| err(&ReaderError::Io("join".into())));
        responder.respond(resp);
    });
}

const THEME_FILE_MAX: u64 = 32 * 1024 * 1024;

fn pct_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(
                std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""),
                16,
            ) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `convertFileSrc(path, "mhfile")` puts the absolute path in the URL.
/// Windows is `/C:/...` (backslashes may arrive percent-encoded). Unix keeps the leading slash.
fn local_path_from_uri(uri: &http::Uri) -> Option<std::path::PathBuf> {
    let decoded = pct_decode(uri.path()).replace('\\', "/");
    let b = decoded.as_bytes();
    if b.len() >= 4 && b[0] == b'/' && b[2] == b':' && b[1].is_ascii_alphabetic() {
        return Some(std::path::PathBuf::from(&decoded[1..]));
    }
    if decoded.is_empty() || decoded == "/" {
        return None;
    }
    Some(std::path::PathBuf::from(decoded))
}

#[cfg(test)]
mod theme_path_tests {
    use super::local_path_from_uri;

    #[test]
    fn decodes_windows_and_unix_paths() {
        let win = http::Uri::from_static(
            "http://mhfile.localhost/C%3A%5CData%5Chero-custom.jpg",
        );
        assert_eq!(
            local_path_from_uri(&win).unwrap(),
            std::path::PathBuf::from("C:/Data/hero-custom.jpg")
        );
        let unix = http::Uri::from_static("mhfile://localhost/home/me/banner-custom.png");
        assert_eq!(
            local_path_from_uri(&unix).unwrap(),
            std::path::PathBuf::from("/home/me/banner-custom.png")
        );
    }
}

fn theme_status(code: u16) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(code)
        .header("X-Content-Type-Options", "nosniff")
        .header("Cache-Control", "no-store")
        .body(Vec::new())
        .unwrap()
}

/// Serves only `hero-custom.*` / `banner-custom.*` from the data directory.
/// Replaces the asset protocol (which was scoped to `**`) for wallpapers and banners.
pub fn serve_theme_file(req: &http::Request<Vec<u8>>) -> http::Response<Vec<u8>> {
    let Ok(root) = crate::data_dir() else {
        return theme_status(500);
    };
    let Some(asked) = local_path_from_uri(req.uri()) else {
        return theme_status(400);
    };
    let (Ok(file), Ok(root)) = (asked.canonicalize(), root.canonicalize()) else {
        return theme_status(404);
    };
    if !file.starts_with(&root) || !file.is_file() {
        return theme_status(403);
    }
    let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if !(name.starts_with("hero-custom.") || name.starts_with("banner-custom.")) {
        return theme_status(403);
    }
    let Ok(meta) = std::fs::metadata(&file) else {
        return theme_status(404);
    };
    if meta.len() > THEME_FILE_MAX {
        return theme_status(413);
    }
    let Ok(bytes) = std::fs::read(&file) else {
        return theme_status(500);
    };
    let mime = crate::extractor::sniff(&bytes)
        .map(|m| m.0)
        .unwrap_or("application/octet-stream");
    http::Response::builder()
        .status(200)
        .header("Content-Type", mime)
        .header("X-Content-Type-Options", "nosniff")
        .header("Cache-Control", "private, max-age=86400")
        .body(bytes)
        .unwrap()
}
