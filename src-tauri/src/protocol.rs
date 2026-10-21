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
