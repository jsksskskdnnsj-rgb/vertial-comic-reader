//! Built-in metadata origins. An empty override keeps the official host.
//! Only http and https are accepted, so a mirror cannot be a file path.

pub(crate) fn api_root(custom: &str, default_root: &str) -> String {
    let t = custom.trim().trim_end_matches('/');
    if t.is_empty() {
        return default_root.trim_end_matches('/').to_string();
    }
    match url::Url::parse(t) {
        Ok(u) if u.scheme() == "https" || u.scheme() == "http" => t.to_string(),
        _ => default_root.trim_end_matches('/').to_string(),
    }
}

pub(crate) fn host_of(url: &str, fallback: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()))
        .unwrap_or_else(|| fallback.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_http_and_keeps_the_official_origin() {
        assert_eq!(api_root("", "https://api.mangadex.org"), "https://api.mangadex.org");
        assert_eq!(api_root("file:///tmp", "https://api.jikan.moe"), "https://api.jikan.moe");
        assert_eq!(api_root("javascript:alert(1)", "https://zh.moegirl.org.cn"), "https://zh.moegirl.org.cn");
        assert_eq!(api_root("https://mirror.example/md/", "https://api.mangadex.org"), "https://mirror.example/md");
    }
}
