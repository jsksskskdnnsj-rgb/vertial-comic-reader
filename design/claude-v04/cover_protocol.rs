// cover_protocol.rs - covers for Home/Library lists.   *** NOT COMPILED ***  (same caveats as reader_backend.rs)
//
// Why a second protocol: `mhpage` only serves books that are OPEN in the reader (the extractor keeps
// two BookMeta in RAM). Lists show dozens of covers, so they get persisted 240px JPEG files:
//     {data}/covers/{book_id}.jpg      URL: http://mhcover.localhost/{book_id}?v={coverVer}
// Edits needed in reader_backend.rs:
//   1. model::VARIANTS: add 240  ->  [0, 160, 240, 800, 1200, 1600, 2400]
//   2. commands::open_blocking: make it `pub(crate)`
use super::{commands::{open_blocking, ReaderState}, error::ReaderError, extractor::sniff};
use std::{path::PathBuf, sync::Arc};
use tauri::State;

pub struct CoversDir(pub Arc<PathBuf>);

/// Register: .register_asynchronous_uri_scheme_protocol("mhcover", |ctx, req, responder| { ... cover_handle(dir, req, responder) })
pub fn serve_cover(dir: &PathBuf, req: &http::Request<Vec<u8>>) -> http::Response<Vec<u8>> {
    let id = req.uri().path().trim_matches('/').parse::<i64>().ok();
    let bytes = id.and_then(|id| std::fs::read(dir.join(format!("{id}.jpg"))).ok());   // id is parsed to i64: no path injection
    let base = http::Response::builder().header("Access-Control-Allow-Origin", "*").header("X-Content-Type-Options", "nosniff");
    match bytes {
        Some(b) => {
            let mime = sniff(&b).map(|m| m.0).unwrap_or("image/jpeg");
            base.status(200).header("Content-Type", mime).header("Cache-Control", "private, max-age=31536000, immutable").body(b).unwrap()  // ?v= busts it
        }
        None => base.status(404).body(Vec::new()).unwrap(),          // home.js reacts to 404 by calling ensure_cover once
    }
}

pub fn cover_handle(dir: Arc<PathBuf>, req: http::Request<Vec<u8>>, responder: tauri::UriSchemeResponder) {
    tauri::async_runtime::spawn(async move {
        let resp = tauri::async_runtime::spawn_blocking(move || serve_cover(&dir, &req)).await
            .unwrap_or_else(|_| http::Response::builder().status(500).body(Vec::new()).unwrap());
        responder.respond(resp);
    });
}

/// Generate covers/{id}.jpg if missing. Call it from your import pipeline right after inserting a book
/// (no reader is open then) and lazily from home.js on a cover 404.
/// Known wrinkle: open_blocking() registers the book in the extractor (max 2 books kept). It never evicts the
/// book the reader is currently using, but do NOT run this in a tight loop while a book is being opened.
#[tauri::command]
pub async fn ensure_cover(state: State<'_, ReaderState>, covers: State<'_, CoversDir>, book_id: i64) -> Result<(), ReaderError> {
    let (s, dir) = (state.inner().clone(), covers.0.clone());
    tauri::async_runtime::spawn_blocking(move || -> Result<(), ReaderError> {
        let out = dir.join(format!("{book_id}.jpg"));
        if out.exists() { return Ok(()); }
        std::fs::create_dir_all(&*dir)?;
        open_blocking(&s, book_id, false)?;                                  // scans headers if needed + registers BookMeta
        let meta = s.extractor.book(book_id).ok_or(ReaderError::BookNotOpen)?;
        let first_virtual = meta.groups.first().map(|g| g.0).unwrap_or(0);   // cover = real page 0 (add books.cover_page later)
        let page = s.extractor.get_or_extract((book_id, first_virtual, 240))?;   // decode + Lanczos to 240px wide, cached
        let tmp = out.with_extension("part");
        std::fs::copy(&page.path, &tmp)?;
        std::fs::rename(tmp, out)?;                                          // atomic; survives cache eviction
        Ok(())
    }).await.map_err(|e| ReaderError::Io(e.to_string()))?
}

// Wiring additions in lib.rs:
//   app.manage(CoversDir(Arc::new(data.join("covers"))));
//   .register_asynchronous_uri_scheme_protocol("mhcover", |ctx, req, responder| {
//        let dir = ctx.app_handle().state::<CoversDir>().0.clone(); cover_handle(dir, req, responder); })
//   generate_handler![..., cover_protocol::ensure_cover]
//   CSP img-src: add http://mhcover.localhost
// Cover busting: store books.cover_ver (int) and bump it when the user changes the cover; pass it as ?v=.
