//! Remote images for the webview. The page never opens the remote host itself:
//! it asks this protocol, and Rust uses the same in-app proxy as every other request.
//! Switching the proxy does not require recreating the WebView.

use super::config;
use super::proxy;

fn base() -> http::response::Builder {
    http::Response::builder()
        .header("Access-Control-Allow-Origin", "*")
        .header("X-Content-Type-Options", "nosniff")
        .header("Cache-Control", "private, max-age=300")
}

fn fail(status: u16) -> http::Response<Vec<u8>> {
    base().status(status).body(Vec::new()).unwrap_or_else(|_| {
        http::Response::builder().status(500).body(Vec::new()).unwrap()
    })
}

fn query_url(uri: &http::Uri) -> Option<String> {
    let query = uri.query()?;
    for part in query.split('&') {
        let mut bits = part.splitn(2, '=');
        let key = bits.next().unwrap_or("");
        let value = bits.next().unwrap_or("");
        if key == "u" {
            return Some(percent_decode(value));
        }
    }
    None
}

fn percent_decode(s: &str) -> String {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn image_type(bytes: &[u8], header: &str) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Some("image/png");
    }
    if bytes.starts_with(b"GIF8") {
        return Some("image/gif");
    }
    if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes.starts_with(b"BM") {
        return Some("image/bmp");
    }
    let header = header.to_ascii_lowercase();
    if header.starts_with("image/jpeg") {
        return Some("image/jpeg");
    }
    if header.starts_with("image/png") {
        return Some("image/png");
    }
    if header.starts_with("image/gif") {
        return Some("image/gif");
    }
    if header.starts_with("image/webp") {
        return Some("image/webp");
    }
    None
}

pub async fn serve(req: http::Request<Vec<u8>>) -> http::Response<Vec<u8>> {
    if req.method() == http::Method::OPTIONS {
        return base()
            .status(204)
            .header("Access-Control-Allow-Methods", "GET")
            .body(Vec::new())
            .unwrap_or_else(|_| fail(500));
    }
    let Some(url) = query_url(req.uri()) else {
        return fail(400);
    };
    if !(url.starts_with("https://") || url.starts_with("http://")) || url.contains('\n') || url.contains('\r') {
        return fail(400);
    }
    let cfg = config::load();
    let client = match proxy::app_client(&cfg) {
        Ok(c) => c,
        Err(_) => return fail(502),
    };
    let response = match client.get(&url).send().await {
        Ok(r) => r,
        Err(_) => return fail(502),
    };
    if !response.status().is_success() {
        return fail(response.status().as_u16());
    }
    let final_url = response.url().to_string();
    if !(final_url.starts_with("https://") || final_url.starts_with("http://")) {
        return fail(400);
    }
    let header = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if response.content_length().unwrap_or(0) > 8 * 1024 * 1024 {
        return fail(413);
    }
    let bytes = match response.bytes().await {
        Ok(b) => b,
        Err(_) => return fail(502),
    };
    if bytes.len() > 8 * 1024 * 1024 {
        return fail(413);
    }
    let Some(mime) = image_type(&bytes, &header) else {
        return fail(415);
    };
    base()
        .status(200)
        .header("Content-Type", mime)
        .body(bytes.to_vec())
        .unwrap_or_else(|_| fail(500))
}
