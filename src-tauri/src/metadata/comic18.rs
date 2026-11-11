//! 禁漫天堂 / 18comic 镜像站 — HTML 解析（无公开 API，仅作品资料）。不尝试绕过 CF 验证。

use regex::Regex;
use std::sync::LazyLock;

use super::client::{build_client_for, offline_message, throttle};
use super::config::MetadataNetworkConfig;
use super::{MetadataCandidate, MetadataDetail, Provider};

pub const BLOCKED_HINT: &str =
    "该站需要浏览器验证，已跳过自动抓取；可在书中使用「在浏览器打开资料页」手动查看";

const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36";
const ACCEPT_LANG: &str = "zh-CN,zh;q=0.9,en;q=0.8,ja;q=0.7,ko;q=0.6";

const DEFAULT_DOMAIN: &str = "18comic.vip";
const FALLBACK_DOMAINS: &[&str] = &["18comic-deadp.org", "comic18j-nini.com"];

static RE_ALBUM_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<a[^>]+href="/album/(\d+)"[^>]*>"#).unwrap());
static RE_ALBUM_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<a[^>]+href="/album/(\d+)"[^>]*title="([^"]*)""#).unwrap()
});
static RE_BOOK_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"id="book-name"[^>]*>([\s\S]*?)</"#).unwrap());
static RE_INTRO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:敘述|叙述|簡介|简介)[：:]\s*([^<]+)"#).unwrap()
});
static RE_COVER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"data-original="(https?://[^"]+/media/albums/[^"]+)""#).unwrap()
});
static RE_TAG_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"標籤[：:][\s\S]*?</div>"#).unwrap());
static RE_AUTHOR_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"作者[：:][\s\S]*?</div>"#).unwrap());
static RE_TAG_A: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<a[^>]*>\s*([^<]+?)\s*</a>"#).unwrap());

fn normalize_host(raw: &str) -> String {
    let d = raw.trim();
    let d = d.trim_start_matches("https://").trim_start_matches("http://");
    d.trim_end_matches('/').to_string()
}

pub fn domain_candidates(cfg: &MetadataNetworkConfig) -> Vec<String> {
    let primary = normalize_host(&cfg.comic18_domain);
    let primary = if primary.is_empty() {
        DEFAULT_DOMAIN.to_string()
    } else {
        primary
    };
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for h in [primary.as_str()]
        .into_iter()
        .chain(FALLBACK_DOMAINS.iter().copied())
    {
        if seen.insert(h.to_string()) {
            out.push(h.to_string());
        }
    }
    out
}

fn browser_headers<'a>(builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    builder
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("Accept-Language", ACCEPT_LANG)
}

pub fn is_challenge_page(status: u16, html: &str) -> bool {
    if status == 403 || status == 503 {
        return true;
    }
    let probe = html.to_ascii_lowercase();
    probe.contains("challenges.cloudflare.com")
        || probe.contains("cf-mitigated")
        || probe.contains("just a moment")
        || probe.contains("cloudflare")
            && probe.contains("challenge")
            && !probe.contains("/album/")
            && !probe.contains("book-name")
}

pub fn build_search_url(origin: &str, query: &str) -> String {
    let q = url::form_urlencoded::byte_serialize(query.trim().as_bytes()).collect::<String>();
    format!("{origin}/search/photos?main_tag=0&search_query={q}")
}

/// Best-effort search URL for opening in the system browser (no network).
pub fn search_url_for_title(cfg: &MetadataNetworkConfig, title: &str) -> String {
    let host = domain_candidates(cfg).into_iter().next().unwrap_or_else(|| DEFAULT_DOMAIN.into());
    build_search_url(&format!("https://{host}"), title)
}

fn album_url(origin: &str, id: &str) -> String {
    format!("{origin}/album/{id}")
}

async fn fetch_origin_for_host(
    cfg: &MetadataNetworkConfig,
    host: &str,
) -> Result<reqwest::Response, String> {
    let client = build_client_for(cfg, Provider::Comic18, BROWSER_UA)?;
    let entry = format!("https://{host}/");
    browser_headers(client.get(&entry))
        .send()
        .await
        .map_err(|e| offline_message(&e))
}

pub(crate) async fn resolve_origin(cfg: &MetadataNetworkConfig) -> Result<String, String> {
    throttle(cfg, "comic18").await;
    let mut last_block = None;
    for host in domain_candidates(cfg) {
        let resp = match fetch_origin_for_host(cfg, &host).await {
            Ok(r) => r,
            Err(msg) => {
                last_block = Some(msg);
                continue;
            }
        };
        let status = resp.status().as_u16();
        if status == 403 || status == 503 {
            last_block = Some(BLOCKED_HINT.into());
            continue;
        }
        if !resp.status().is_success() && status != 301 && status != 302 {
            last_block = Some(format!("镜像 {host} 不可达（HTTP {status}）"));
            continue;
        }
        let url = resp.url().clone();
        let h = url
            .host_str()
            .ok_or_else(|| "镜像站地址无效".to_string())?;
        return Ok(format!("{}://{}", url.scheme(), h));
    }
    Err(last_block.unwrap_or_else(|| BLOCKED_HINT.into()))
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tags_from_block(html: &str) -> Vec<String> {
    RE_TAG_A
        .captures_iter(html)
        .filter_map(|c| {
            let t = c.get(1)?.as_str().trim();
            (!t.is_empty() && t.len() <= 48).then(|| t.to_string())
        })
        .collect()
}

pub fn parse_search_html(html: &str, origin: &str, limit: usize) -> Vec<MetadataCandidate> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for cap in RE_ALBUM_TITLE.captures_iter(html) {
        let id = cap.get(1).map(|m| m.as_str()).unwrap_or("");
        let title = cap
            .get(2)
            .map(|m| strip_tags(m.as_str()))
            .unwrap_or_default();
        if id.is_empty() || !seen.insert(id.to_string()) {
            continue;
        }
        let page = album_url(origin, id);
        out.push(MetadataCandidate {
            provider: Provider::Comic18,
            external_id: id.to_string(),
            title: if title.is_empty() {
                format!("JM{id}")
            } else {
                title
            },
            original_title: None,
            aliases: Vec::new(),
            release_date: None,
            cover_url: None,
            rating: None,
            source_label: "18comic".into(),
            match_kind: None,
            page_url: Some(page),
            confidence: None,
        });
        if out.len() >= limit {
            break;
        }
    }
    if out.is_empty() {
        for cap in RE_ALBUM_LINK.captures_iter(html) {
            let id = cap.get(1).map(|m| m.as_str()).unwrap_or("");
            if id.is_empty() || !seen.insert(id.to_string()) {
                continue;
            }
            out.push(MetadataCandidate {
                provider: Provider::Comic18,
                external_id: id.to_string(),
                title: format!("JM{id}"),
                original_title: None,
                aliases: Vec::new(),
                release_date: None,
                cover_url: None,
                rating: None,
                source_label: "18comic".into(),
                match_kind: None,
                page_url: Some(album_url(origin, id)),
                confidence: None,
            });
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

pub fn parse_album_html(html: &str, origin: &str, id: &str) -> MetadataDetail {
    let title = RE_BOOK_NAME
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| strip_tags(m.as_str()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("JM{id}"));
    let summary = RE_INTRO
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| strip_tags(m.as_str()))
        .filter(|s| !s.is_empty());
    let cover_url = RE_COVER
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string());
    let authors = RE_AUTHOR_BLOCK
        .find(html)
        .map(|m| tags_from_block(m.as_str()))
        .unwrap_or_default();
    let tags = RE_TAG_BLOCK
        .find(html)
        .map(|m| tags_from_block(m.as_str()))
        .unwrap_or_default();
    let page_url = album_url(origin, id);
    MetadataDetail {
        provider: Provider::Comic18,
        external_id: id.to_string(),
        title,
        title_cn: None,
        title_original: None,
        summary,
        authors,
        tags,
        rating: None,
        rating_count: None,
        release_date: None,
        cover_url,
        aliases: Vec::new(),
        page_url: Some(page_url),
        facts: super::WorkFacts::default(),
    }
}

pub async fn search(
    cfg: &MetadataNetworkConfig,
    query: &str,
    limit: u32,
) -> Result<Vec<MetadataCandidate>, String> {
    if !cfg.sources.comic18 {
        return Ok(Vec::new());
    }
    let origin = resolve_origin(cfg).await?;
    throttle(cfg, "comic18").await;
    let client = build_client_for(cfg, Provider::Comic18, BROWSER_UA)?;
    let url = build_search_url(&origin, query);
    let resp = browser_headers(client.get(&url))
        .send()
        .await
        .map_err(|e| offline_message(&e))?;
    let status = resp.status().as_u16();
    let html = resp.text().await.map_err(|_| "18comic 页面读取失败".to_string())?;
    if is_challenge_page(status, &html) {
        return Err(BLOCKED_HINT.into());
    }
    if status < 200 || status >= 300 {
        return Err(format!("18comic 搜索失败（HTTP {status}）"));
    }
    Ok(parse_search_html(
        &html,
        &origin,
        limit.clamp(1, 25) as usize,
    ))
}

pub async fn detail(cfg: &MetadataNetworkConfig, id: &str) -> Result<MetadataDetail, String> {
    let origin = resolve_origin(cfg).await?;
    throttle(cfg, "comic18").await;
    let client = build_client_for(cfg, Provider::Comic18, BROWSER_UA)?;
    let url = album_url(&origin, id.trim());
    let resp = browser_headers(client.get(&url))
        .send()
        .await
        .map_err(|e| offline_message(&e))?;
    let status = resp.status().as_u16();
    let html = resp.text().await.map_err(|_| "18comic 页面读取失败".to_string())?;
    if is_challenge_page(status, &html) {
        return Err(BLOCKED_HINT.into());
    }
    if status < 200 || status >= 300 {
        return Err(format!("18comic 详情失败（HTTP {status}）"));
    }
    Ok(parse_album_html(&html, &origin, id.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search_and_album_fixture() {
        let origin = "https://comic18j-nini.com";
        let search = r#"
        <a href="/album/427413" title="[TG Washer] 作品名 (汉化)">
        <a href="/album/123" title="Other">
        "#;
        let hits = parse_search_html(search, origin, 5);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].external_id, "427413");
        let album = r#"
        <h1 id="book-name">我的漫画</h1>
        敘述：简介文字在这里
        data-original="https://cdn.example/media/albums/cover.jpg"
        作者：</span><div><a>作者甲</a></div>
        標籤：</span><div><a>韩漫</a></div>
        "#;
        let d = parse_album_html(album, origin, "427413");
        assert_eq!(d.title, "我的漫画");
        assert!(d.tags.contains(&"韩漫".to_string()));
    }

    #[test]
    fn detects_cf_challenge() {
        assert!(is_challenge_page(
            403,
            "<html>challenges.cloudflare.com</html>"
        ));
        assert!(!is_challenge_page(
            200,
            r#"<h1 id="book-name">ok</h1><a href="/album/1">"#,
        ));
    }

    #[test]
    fn domain_list_orders_primary_and_fallbacks() {
        let cfg = MetadataNetworkConfig {
            comic18_domain: "18comic.vip".into(),
            ..Default::default()
        };
        let hosts = domain_candidates(&cfg);
        assert_eq!(hosts[0], "18comic.vip");
        assert!(hosts.contains(&"18comic-deadp.org".to_string()));
    }
}
