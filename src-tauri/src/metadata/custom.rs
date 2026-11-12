//! User-defined metadata lookups for books that are already on the shelf.
//! A source is a search URL plus a JSON path or a small CSS-like selector.
//! It never downloads chapters and it never adds a comic to the shelf.

use serde::Serialize;
use serde_json::Value;

use super::client::send_retry;
use super::config::{CustomSource, MetadataNetworkConfig};
use super::endpoints::host_of;
use super::{MetadataCandidate, MetadataDetail, Provider, WorkFacts};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedItem {
    pub id: String,
    pub title: String,
    pub author: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub cover: String,
    pub page: String,
}

pub(crate) fn fill_template(tmpl: &str, title: &str, author: &str) -> Result<String, String> {
    let title = encode(title);
    let author = encode(author);
    let url = tmpl.trim().replace("{title}", &title).replace("{author}", &author);
    if url.contains('{') || url.contains('}') {
        return Err("网址里还有未替换的占位符".into());
    }
    let parsed = url::Url::parse(&url).map_err(|_| "网址无效".to_string())?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("自定义来源只允许 http 或 https".into());
    }
    Ok(url)
}

fn encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.trim().as_bytes()).collect()
}

pub(crate) fn parse_document(sample: &str, source: &CustomSource) -> Result<Vec<ParsedItem>, String> {
    let format = source.format.trim().to_ascii_lowercase();
    if format == "html" {
        let root = parse_html(sample);
        let nodes = if source.list.trim().is_empty() {
            vec![&root]
        } else {
            select(&[&root], source.list.trim())
        };
        let mut out = Vec::new();
        for node in nodes {
            let item = item_from_html(node, source);
            if !item.title.is_empty() {
                out.push(item);
            }
        }
        Ok(out)
    } else {
        let value: Value = serde_json::from_str(sample).map_err(|_| "JSON 无法解析".to_string())?;
        let found = if source.list.trim().is_empty() {
            vec![&value]
        } else {
            json_values(&value, source.list.trim())
        };
        let mut items = Vec::new();
        for value in found {
            if let Some(arr) = value.as_array() {
                items.extend(arr.iter());
            } else {
                items.push(value);
            }
        }
        let mut out = Vec::new();
        for item in items {
            let parsed = item_from_json(item, source);
            if !parsed.title.is_empty() {
                out.push(parsed);
            }
        }
        Ok(out)
    }
}

fn item_from_json(item: &Value, source: &CustomSource) -> ParsedItem {
    let title = json_text(item, &source.title_path);
    let id = {
        let id = json_text(item, &source.id_path);
        if id.is_empty() { title.clone() } else { id }
    };
    ParsedItem {
        id,
        title,
        author: json_text(item, &source.author_path),
        summary: json_text(item, &source.summary_path),
        tags: json_texts(item, &source.tags_path),
        cover: json_text(item, &source.cover_path),
        page: json_text(item, &source.page_path),
    }
}

fn item_from_html(node: &El, source: &CustomSource) -> ParsedItem {
    let title = html_text(node, &source.title_path);
    let id = {
        let id = html_text(node, &source.id_path);
        if id.is_empty() { title.clone() } else { id }
    };
    ParsedItem {
        id,
        title,
        author: html_text(node, &source.author_path),
        summary: html_text(node, &source.summary_path),
        tags: html_texts(node, &source.tags_path),
        cover: html_text(node, &source.cover_path),
        page: html_text(node, &source.page_path),
    }
}

fn json_text(value: &Value, path: &str) -> String {
    json_texts(value, path).into_iter().next().unwrap_or_default()
}

fn json_texts(value: &Value, path: &str) -> Vec<String> {
    if path.trim().is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for v in json_values(value, path.trim()) {
        push_json_strings(v, &mut out);
    }
    out
}

fn push_json_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => {
            let s = clean(s);
            if !s.is_empty() {
                out.push(s);
            }
        }
        Value::Number(n) => out.push(n.to_string()),
        Value::Array(items) => {
            for item in items {
                push_json_strings(item, out);
            }
        }
        Value::Object(map) => {
            if let Some(name) = map.get("name").and_then(Value::as_str) {
                let name = clean(name);
                if !name.is_empty() {
                    out.push(name);
                }
            }
        }
        _ => {}
    }
}

fn json_values<'a>(value: &'a Value, path: &str) -> Vec<&'a Value> {
    let mut cur = vec![value];
    for token in path.split('.') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let (name, indexes) = split_token(token);
        let mut next = Vec::new();
        for value in cur {
            let base = if name.is_empty() { Some(value) } else { value.get(name) };
            let Some(base) = base else { continue };
            if indexes.is_empty() {
                next.push(base);
                continue;
            }
            for index in &indexes {
                match index {
                    Index::All => {
                        if let Some(arr) = base.as_array() {
                            next.extend(arr.iter());
                        }
                    }
                    Index::One(i) => {
                        if let Some(item) = base.as_array().and_then(|a| a.get(*i)) {
                            next.push(item);
                        }
                    }
                }
            }
        }
        cur = next;
    }
    cur
}

enum Index {
    All,
    One(usize),
}

fn split_token(token: &str) -> (&str, Vec<Index>) {
    let Some(start) = token.find('[') else {
        return (token, Vec::new());
    };
    let name = &token[..start];
    let mut indexes = Vec::new();
    let mut rest = &token[start..];
    while rest.starts_with('[') {
        let Some(end) = rest.find(']') else { break };
        let inner = &rest[1..end];
        if inner.is_empty() || inner == "*" {
            indexes.push(Index::All);
        } else if let Ok(i) = inner.parse::<usize>() {
            indexes.push(Index::One(i));
        }
        rest = &rest[end + 1..];
    }
    (name, indexes)
}

fn clean(s: &str) -> String {
    s.replace(['\n', '\r'], " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

struct El {
    tag: String,
    id: String,
    class: Vec<String>,
    attrs: Vec<(String, String)>,
    text: String,
    children: Vec<El>,
}

fn parse_html(input: &str) -> El {
    let mut root = El::new("root");
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    parse_nodes(&chars, &mut i, &mut root);
    root
}

impl El {
    fn new(tag: &str) -> Self {
        Self {
            tag: tag.to_string(),
            id: String::new(),
            class: Vec::new(),
            attrs: Vec::new(),
            text: String::new(),
            children: Vec::new(),
        }
    }
}

fn parse_nodes(chars: &[char], i: &mut usize, parent: &mut El) {
    while *i < chars.len() {
        if chars[*i] == '<' {
            if starts_with(chars, *i, "</") {
                return;
            }
            if starts_with(chars, *i, "<!--") {
                *i += 4;
                while *i + 2 < chars.len() && !starts_with(chars, *i, "-->") {
                    *i += 1;
                }
                *i = (*i + 3).min(chars.len());
                continue;
            }
            *i += 1;
            if *i >= chars.len() {
                return;
            }
            let tag = read_name(chars, i);
            if tag.is_empty() {
                continue;
            }
            let mut el = El::new(&tag.to_ascii_lowercase());
            while *i < chars.len() && chars[*i] != '>' && !starts_with(chars, *i, "/>") {
                skip_ws(chars, i);
                if *i >= chars.len() || chars[*i] == '>' || starts_with(chars, *i, "/>") {
                    break;
                }
                let name = read_name(chars, i).to_ascii_lowercase();
                skip_ws(chars, i);
                let value = if *i < chars.len() && chars[*i] == '=' {
                    *i += 1;
                    skip_ws(chars, i);
                    read_attr(chars, i)
                } else {
                    String::new()
                };
                if name == "id" {
                    el.id = value.clone();
                } else if name == "class" {
                    el.class = value.split_whitespace().map(|s| s.to_string()).collect();
                }
                if !name.is_empty() {
                    el.attrs.push((name, value));
                }
            }
            let close = starts_with(chars, *i, "/>");
            if close {
                *i += 2;
            } else if *i < chars.len() && chars[*i] == '>' {
                *i += 1;
            }
            let void = close || matches!(el.tag.as_str(), "img" | "br" | "meta" | "link" | "hr" | "input" | "source");
            if !void {
                if el.tag == "script" || el.tag == "style" {
                    while *i < chars.len() && !starts_with(chars, *i, &format!("</{}", el.tag)) {
                        *i += 1;
                    }
                    skip_end(chars, i);
                } else {
                    parse_nodes(chars, i, &mut el);
                    skip_end(chars, i);
                }
            }
            parent.children.push(el);
        } else {
            let start = *i;
            while *i < chars.len() && chars[*i] != '<' {
                *i += 1;
            }
            let text: String = chars[start..*i].iter().collect();
            let text = clean(&text);
            if !text.is_empty() {
                if parent.text.is_empty() {
                    parent.text = text;
                } else {
                    parent.text.push(' ');
                    parent.text.push_str(&text);
                }
            }
        }
    }
}

fn skip_end(chars: &[char], i: &mut usize) {
    if starts_with(chars, *i, "</") {
        while *i < chars.len() && chars[*i] != '>' {
            *i += 1;
        }
        if *i < chars.len() {
            *i += 1;
        }
    }
}

fn starts_with(chars: &[char], i: usize, s: &str) -> bool {
    let p: Vec<char> = s.chars().collect();
    if i + p.len() > chars.len() {
        return false;
    }
    chars[i..i + p.len()].iter().zip(p.iter()).all(|(a, b)| a.eq_ignore_ascii_case(b))
}

fn skip_ws(chars: &[char], i: &mut usize) {
    while *i < chars.len() && chars[*i].is_whitespace() {
        *i += 1;
    }
}

fn read_name(chars: &[char], i: &mut usize) -> String {
    let start = *i;
    while *i < chars.len() && (chars[*i].is_ascii_alphanumeric() || chars[*i] == '-' || chars[*i] == ':') {
        *i += 1;
    }
    chars[start..*i].iter().collect()
}

fn read_attr(chars: &[char], i: &mut usize) -> String {
    if *i >= chars.len() {
        return String::new();
    }
    if chars[*i] == '"' || chars[*i] == '\'' {
        let q = chars[*i];
        *i += 1;
        let start = *i;
        while *i < chars.len() && chars[*i] != q {
            *i += 1;
        }
        let value: String = chars[start..*i].iter().collect();
        if *i < chars.len() {
            *i += 1;
        }
        value
    } else {
        read_name(chars, i)
    }
}

fn select<'a>(roots: &[&'a El], selector: &str) -> Vec<&'a El> {
    let mut cur: Vec<&El> = roots.to_vec();
    for part in selector.split_whitespace() {
        let mut next = Vec::new();
        for node in cur {
            walk_desc(node, part, &mut next);
        }
        cur = next;
    }
    cur
}

fn walk_desc<'a>(node: &'a El, simple: &str, out: &mut Vec<&'a El>) {
    for child in &node.children {
        if matches_simple(child, simple) {
            out.push(child);
        }
        walk_desc(child, simple, out);
    }
}

fn matches_simple(node: &El, simple: &str) -> bool {
    let (head, attr) = simple.split_once('[').map(|(h, rest)| (h, Some(rest.trim_end_matches(']')))).unwrap_or((simple, None));
    let mut tag = "";
    let mut id = "";
    let mut class = "";
    if let Some(rest) = head.strip_prefix('#') {
        id = rest;
    } else if let Some(rest) = head.strip_prefix('.') {
        class = rest;
    } else if let Some((t, rest)) = head.split_once('#') {
        tag = t;
        id = rest;
    } else if let Some((t, rest)) = head.split_once('.') {
        tag = t;
        class = rest;
    } else {
        tag = head;
    }
    if !tag.is_empty() && node.tag != tag {
        return false;
    }
    if !id.is_empty() && node.id != id {
        return false;
    }
    if !class.is_empty() && !node.class.iter().any(|c| c == class) {
        return false;
    }
    if let Some(spec) = attr {
        let (name, value) = spec.split_once('=').unwrap_or((spec, ""));
        let value = value.trim_matches('"').trim_matches('\'');
        let found = node.attrs.iter().any(|(n, v)| n == name && (value.is_empty() || v == value));
        if !found {
            return false;
        }
    }
    !head.is_empty() || attr.is_some()
}

fn html_text(node: &El, path: &str) -> String {
    html_texts(node, path).into_iter().next().unwrap_or_default()
}

fn html_texts(node: &El, path: &str) -> Vec<String> {
    let path = path.trim();
    if path.is_empty() {
        return vec![node_text(node)].into_iter().filter(|s| !s.is_empty()).collect();
    }
    let (selector, attr) = if let Some(rest) = path.strip_suffix("::text") {
        (rest.trim().trim_end_matches(|c: char| c == '@' || c.is_whitespace()), None)
    } else if let Some((sel, attr)) = path.rsplit_once('@') {
        (sel.trim(), Some(attr.trim()))
    } else {
        (path, None)
    };
    let nodes = if selector.is_empty() { vec![node] } else { select(&[node], selector) };
    let mut out = Vec::new();
    for node in nodes {
        let text = if let Some(attr) = attr {
            node.attrs.iter().find(|(n, _)| n == attr).map(|(_, v)| clean(v)).unwrap_or_default()
        } else {
            node_text(node)
        };
        if !text.is_empty() {
            out.push(text);
        }
    }
    out
}

fn node_text(node: &El) -> String {
    let mut parts = Vec::new();
    if !node.text.is_empty() {
        parts.push(node.text.clone());
    }
    for child in &node.children {
        let text = node_text(child);
        if !text.is_empty() {
            parts.push(text);
        }
    }
    clean(&parts.join(" "))
}

fn external_id(source: &CustomSource, item: &ParsedItem) -> String {
    format!("{}\n{}\n{}", source.id.trim(), clean(&item.id), clean(&item.title))
}

fn split_id(external_id: &str) -> (String, String, String) {
    let mut bits = external_id.splitn(3, '\n');
    (
        bits.next().unwrap_or("").to_string(),
        bits.next().unwrap_or("").to_string(),
        bits.next().unwrap_or("").to_string(),
    )
}

fn to_candidate(source: &CustomSource, item: &ParsedItem) -> MetadataCandidate {
    MetadataCandidate {
        provider: Provider::Custom,
        external_id: external_id(source, item),
        title: item.title.clone(),
        original_title: None,
        aliases: Vec::new(),
        release_date: None,
        cover_url: http_only(&item.cover),
        rating: None,
        source_label: source.name.clone(),
        match_kind: None,
        page_url: http_only(&item.page),
        confidence: None,
    }
}

fn http_only(url: &str) -> Option<String> {
    let url = url.trim();
    if url.starts_with("http://") || url.starts_with("https://") {
        Some(url.to_string())
    } else {
        None
    }
}

fn to_detail(source: &CustomSource, item: &ParsedItem) -> MetadataDetail {
    let authors = if item.author.trim().is_empty() {
        Vec::new()
    } else {
        item.author
            .split([',', '、', ';'])
            .map(clean)
            .filter(|s| !s.is_empty())
            .collect()
    };
    MetadataDetail {
        provider: Provider::Custom,
        external_id: external_id(source, item),
        title: item.title.clone(),
        title_cn: None,
        title_original: None,
        summary: if item.summary.trim().is_empty() { None } else { Some(item.summary.clone()) },
        authors,
        tags: item.tags.clone(),
        rating: None,
        rating_count: None,
        release_date: None,
        cover_url: http_only(&item.cover),
        aliases: Vec::new(),
        page_url: http_only(&item.page),
        facts: WorkFacts::default(),
    }
}

fn check_source(source: &CustomSource) -> Result<(), String> {
    if source.name.trim().is_empty() {
        return Err("请填写来源名称".into());
    }
    if source.id.contains('\n') || source.id.len() > 40 {
        return Err("来源编号无效".into());
    }
    for header in &source.headers {
        if header.name.contains(['\n', '\r', ':']) || header.value.contains(['\n', '\r']) {
            return Err("请求头无效".into());
        }
    }
    if source.headers.len() > 16 {
        return Err("请求头太多".into());
    }
    Ok(())
}

pub async fn search_source(
    cfg: &MetadataNetworkConfig,
    source: &CustomSource,
    title: &str,
    author: &str,
) -> Result<Vec<MetadataCandidate>, String> {
    check_source(source)?;
    let url = fill_template(&source.search_url, title, author)?;
    let host = host_of(&url, "custom");
    let sample = fetch_text(cfg, source, &url, &host).await?;
    let items = parse_document(&sample, source)?;
    Ok(items.iter().take(25).map(|item| to_candidate(source, item)).collect())
}

pub struct CustomHit {
    pub label: String,
    pub error: Option<String>,
    pub candidates: Vec<MetadataCandidate>,
}

pub async fn search_enabled(cfg: &MetadataNetworkConfig, title: &str, author: &str) -> Vec<CustomHit> {
    let mut out = Vec::new();
    for source in cfg.custom_sources.iter().filter(|s| s.enabled) {
        match search_source(cfg, source, title, author).await {
            Ok(candidates) => out.push(CustomHit {
                label: source.name.clone(),
                error: None,
                candidates,
            }),
            Err(error) => out.push(CustomHit {
                label: source.name.clone(),
                error: Some(error),
                candidates: Vec::new(),
            }),
        }
    }
    out
}

pub async fn detail(cfg: &MetadataNetworkConfig, external_id: &str) -> Result<MetadataDetail, String> {
    let (sid, remote, title) = split_id(external_id);
    let source = cfg
        .custom_sources
        .iter()
        .find(|s| s.id == sid)
        .ok_or_else(|| "自定义来源已删除".to_string())?;
    let items = {
        let url = fill_template(&source.search_url, &title, "")?;
        let host = host_of(&url, "custom");
        let sample = fetch_text(cfg, source, &url, &host).await?;
        parse_document(&sample, source)?
    };
    let item = items
        .into_iter()
        .find(|item| clean(&item.id) == remote || clean(&item.title) == title)
        .ok_or_else(|| "自定义来源里没有这条记录".to_string())?;
    Ok(to_detail(source, &item))
}

async fn fetch_text(cfg: &MetadataNetworkConfig, source: &CustomSource, url: &str, host: &str) -> Result<String, String> {
    let headers = source.headers.clone();
    let response = send_retry(cfg, host, 100, || {
        let client = super::client::build_client(cfg, Provider::Custom)?;
        let mut req = client.get(url).header("Accept", "application/json, text/html;q=0.9, */*;q=0.8");
        for header in &headers {
            let name = header.name.trim();
            if !name.is_empty() {
                req = req.header(name, header.value.trim());
            }
        }
        Ok(req)
    })
    .await?;
    if !response.status().is_success() {
        return Err(format!("自定义来源请求失败（HTTP {}）", response.status()));
    }
    let bytes = {
        if response.content_length().unwrap_or(0) > 2 * 1024 * 1024 {
            return Err("自定义来源的响应过大".into());
        }
        let bytes = response.bytes().await.map_err(|_| "自定义来源没有返回内容".to_string())?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err("自定义来源的响应过大".into());
        }
        bytes
    };
    String::from_utf8(bytes.to_vec()).map_err(|_| "自定义来源的响应不是文本".to_string())
}

#[tauri::command]
pub fn metadata_custom_parse(sample: String, source: CustomSource) -> Result<Vec<ParsedItem>, String> {
    if sample.len() > 2 * 1024 * 1024 {
        return Err("样例过大".into());
    }
    check_source(&source)?;
    let mut items = parse_document(&sample, &source)?;
    for item in &mut items {
        item.cover = http_only(&item.cover).unwrap_or_default();
        item.page = http_only(&item.page).unwrap_or_default();
    }
    Ok(items)
}

#[tauri::command]
pub async fn metadata_custom_test(book_id: i64, source: CustomSource) -> Result<Vec<MetadataCandidate>, String> {
    super::commands::require_shelf_book(book_id)?;
    let title = super::store::book_title(book_id)?;
    let cfg = super::config::load();
    search_source(&cfg, &source, &title, "").await
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::config::HeaderPair;

    fn source(format: &str) -> CustomSource {
        CustomSource {
            id: "demo".into(),
            name: "演示".into(),
            enabled: true,
            use_proxy: true,
            search_url: "https://example.test/search?q={title}&a={author}".into(),
            headers: vec![HeaderPair { name: "X-Test".into(), value: "1".into() }],
            format: format.into(),
            list: if format == "html" { ".item".into() } else { "data".into() },
            id_path: "id".into(),
            title_path: if format == "html" { "a.t".into() } else { "title".into() },
            author_path: if format == "html" { ".a".into() } else { "authors".into() },
            summary_path: "summary".into(),
            tags_path: if format == "html" { ".tag".into() } else { "tags".into() },
            cover_path: if format == "html" { "img@src".into() } else { "cover".into() },
            page_path: if format == "html" { "a.t@href".into() } else { "page".into() },
        }
    }

    #[test]
    fn template_encodes_and_blocks_file_urls() {
        let url = fill_template("https://ex/s?q={title}&a={author}", "one piece", "oda").unwrap();
        assert!(url.contains("one+piece") || url.contains("one%20piece"));
        assert!(fill_template("file:///tmp/{title}", "x", "").is_err());
    }

    #[test]
    fn json_and_html_rules_read_title_author_and_cover() {
        let json = r#"{"data":[{"id":"7","title":"One","authors":["Ann"],"tags":["a","b"],"cover":"https://cdn.example/c.jpg","page":"https://ex/p"}]}"#;
        let items = parse_document(json, &source("json")).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].title, "One");
        assert_eq!(items[0].author, "Ann");
        assert_eq!(items[0].tags, vec!["a", "b"]);
        assert!(items[0].cover.starts_with("https://"));
        let html = r#"<div class="item"><a class="t" href="https://ex/a">One</a><span class="a">Ann</span><i class="tag">a</i><img src="https://cdn.example/c.jpg"></div>"#;
        let html_source = source("html");
        let items = parse_document(html, &html_source).unwrap();
        assert_eq!(items[0].title, "One");
        assert_eq!(items[0].author, "Ann");
        assert_eq!(items[0].cover, "https://cdn.example/c.jpg");
        assert_eq!(items[0].page, "https://ex/a");
    }
}
