//! MangaDex metadata only. Never calls `/at-home` or chapter image endpoints.

use std::collections::HashMap;

use serde::Deserialize;

use super::client::{offline_message, send_retry};
use super::config::MetadataNetworkConfig;
use super::{MetadataCandidate, MetadataDetail, Provider, WorkFacts};

const HOST: &str = "api.mangadex.org";
const API: &str = "https://api.mangadex.org";

#[derive(Debug, Deserialize)]
struct ListBody {
    #[serde(default)]
    data: Vec<Manga>,
}

#[derive(Debug, Deserialize)]
struct OneBody {
    data: Manga,
}

#[derive(Debug, Deserialize)]
struct Manga {
    id: String,
    attributes: Attr,
    #[serde(default)]
    relationships: Vec<Rel>,
}

#[derive(Debug, Deserialize)]
struct Attr {
    #[serde(default)]
    title: HashMap<String, String>,
    #[serde(default)]
    #[serde(rename = "altTitles")]
    alt_titles: Vec<HashMap<String, String>>,
    #[serde(default)]
    description: HashMap<String, String>,
    status: Option<String>,
    year: Option<i32>,
    #[serde(default, rename = "lastChapter")]
    last_chapter: Option<String>,
    #[serde(default, rename = "lastVolume")]
    last_volume: Option<String>,
    #[serde(default)]
    tags: Vec<Tag>,
}

#[derive(Debug, Deserialize)]
struct Tag {
    attributes: TagAttr,
}

#[derive(Debug, Deserialize)]
struct TagAttr {
    #[serde(default)]
    name: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct Rel {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    attributes: Option<serde_json::Value>,
}

pub(crate) fn search_url(query: &str, limit: u32) -> String {
    let q = url::form_urlencoded::byte_serialize(query.trim().as_bytes()).collect::<String>();
    let n = limit.clamp(1, 25);
    format!(
        "{API}/manga?title={q}&limit={n}&includes[]=cover_art&includes[]=author&includes[]=artist&order[relevance]=desc&contentRating[]=safe&contentRating[]=suggestive&contentRating[]=erotica&contentRating[]=pornographic"
    )
}

pub(crate) fn detail_url(id: &str) -> String {
    format!("{API}/manga/{}?includes[]=cover_art&includes[]=author&includes[]=artist", id.trim())
}

/// Thumbnail only (`file.jpg` → `file.jpg.256.jpg`). Not the original upload.
pub(crate) fn cover_thumb_url(manga_id: &str, file_name: &str) -> String {
    format!(
        "https://uploads.mangadex.org/covers/{}/{}.256.jpg",
        manga_id.trim(),
        file_name.trim()
    )
}

fn prefer_text(map: &HashMap<String, String>) -> Option<String> {
    for key in ["zh", "zh-hk", "zh-ro", "ja", "ja-ro", "en", "ko"] {
        if let Some(v) = map.get(key).map(|s| s.trim()).filter(|s| !s.is_empty()) {
            return Some(v.to_string());
        }
    }
    map.values().map(|s| s.trim()).find(|s| !s.is_empty()).map(|s| s.to_string())
}

fn parse_count(raw: &Option<String>) -> Option<i64> {
    let s = raw.as_deref()?.trim();
    if s.is_empty() {
        return None;
    }
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let n: f64 = digits.parse().ok()?;
    if n <= 0.0 || !n.is_finite() {
        return None;
    }
    Some(n.floor() as i64)
}

fn people(rels: &[Rel]) -> Vec<String> {
    let mut out = Vec::new();
    for rel in rels {
        if rel.kind != "author" && rel.kind != "artist" {
            continue;
        }
        let Some(name) = rel
            .attributes
            .as_ref()
            .and_then(|v| v.get("name"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if !out.iter().any(|s| s == name) {
            out.push(name.to_string());
        }
    }
    out
}

fn cover_file(rels: &[Rel]) -> Option<String> {
    rels.iter().find(|r| r.kind == "cover_art").and_then(|r| {
        r.attributes
            .as_ref()
            .and_then(|v| v.get("fileName"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    })
}

fn aliases_of(attr: &Attr, title: &str) -> (Option<String>, Vec<String>) {
    let mut aliases = Vec::new();
    for (lang, value) in &attr.title {
        let value = value.trim();
        if value.is_empty() || value == title {
            continue;
        }
        if !aliases.iter().any(|a| a == value) {
            aliases.push(value.to_string());
        }
        let _ = lang;
    }
    for map in &attr.alt_titles {
        for value in map.values() {
            let value = value.trim();
            if value.is_empty() || value == title || aliases.iter().any(|a| a == value) {
                continue;
            }
            aliases.push(value.to_string());
        }
    }
    let original = attr
        .title
        .get("ja")
        .or_else(|| attr.title.get("ja-ro"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != title);
    (original, aliases)
}

fn to_candidate(m: Manga) -> MetadataCandidate {
    let title = prefer_text(&m.attributes.title).unwrap_or_else(|| m.id.clone());
    let (original, aliases) = aliases_of(&m.attributes, &title);
    let cover = cover_file(&m.relationships).map(|file| cover_thumb_url(&m.id, &file));
    MetadataCandidate {
        provider: Provider::Mangadex,
        external_id: m.id.clone(),
        title,
        original_title: original,
        aliases,
        release_date: m.attributes.year.map(|y| y.to_string()),
        cover_url: cover,
        rating: None,
        source_label: "MangaDex".into(),
        match_kind: None,
        page_url: Some(format!("https://mangadex.org/title/{}", m.id)),
        confidence: None,
    }
}

fn to_detail(m: Manga) -> MetadataDetail {
    let id = m.id.clone();
    let title = prefer_text(&m.attributes.title).unwrap_or_else(|| id.clone());
    let (original, aliases) = aliases_of(&m.attributes, &title);
    let summary = prefer_text(&m.attributes.description).map(|s| {
        let t = s.replace('\r', "");
        if t.chars().count() > 900 {
            t.chars().take(900).collect::<String>() + "…"
        } else {
            t
        }
    });
    let tags = m
        .attributes
        .tags
        .iter()
        .filter_map(|t| prefer_text(&t.attributes.name))
        .take(24)
        .collect();
    let cover = cover_file(&m.relationships).map(|file| cover_thumb_url(&id, &file));
    MetadataDetail {
        provider: Provider::Mangadex,
        external_id: id.clone(),
        title,
        title_cn: m.attributes.title.get("zh").cloned().filter(|s| !s.trim().is_empty()),
        title_original: original,
        summary,
        authors: people(&m.relationships),
        tags,
        rating: None,
        rating_count: None,
        release_date: m.attributes.year.map(|y| y.to_string()),
        cover_url: cover,
        aliases,
        page_url: Some(format!("https://mangadex.org/title/{id}")),
        facts: WorkFacts {
            status: m.attributes.status.filter(|s| !s.trim().is_empty()),
            chapter_count: parse_count(&m.attributes.last_chapter),
            volume_count: parse_count(&m.attributes.last_volume),
            year: m.attributes.year,
        },
    }
}

pub async fn search(
    cfg: &MetadataNetworkConfig,
    query: &str,
    limit: u32,
) -> Result<Vec<MetadataCandidate>, String> {
    let url = search_url(query, limit);
    let response = send_retry(cfg, HOST, 250, || {
        let client = super::client::build_client(cfg, Provider::Mangadex)?;
        Ok(client.get(&url).header("Accept", "application/json"))
    })
    .await?;
    if !response.status().is_success() {
        return Err(format!("MangaDex 请求失败（HTTP {}）", response.status()));
    }
    let body: ListBody = response
        .json()
        .await
        .map_err(|e| offline_message(&e).to_string())?;
    Ok(body.data.into_iter().map(to_candidate).collect())
}

pub async fn detail(cfg: &MetadataNetworkConfig, id: &str) -> Result<MetadataDetail, String> {
    let url = detail_url(id);
    let response = send_retry(cfg, HOST, 250, || {
        let client = super::client::build_client(cfg, Provider::Mangadex)?;
        Ok(client.get(&url).header("Accept", "application/json"))
    })
    .await?;
    if !response.status().is_success() {
        return Err(format!("MangaDex 详情失败（HTTP {}）", response.status()));
    }
    let body: OneBody = response.json().await.map_err(|_| "MangaDex 详情无法解析".to_string())?;
    Ok(to_detail(body.data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_metadata_only() {
        let s = search_url("进击的巨人", 5);
        assert!(s.starts_with("https://api.mangadex.org/manga?"));
        assert!(!s.contains("/chapter"));
        assert!(!s.contains("at-home"));
        let d = detail_url("abc");
        assert_eq!(
            d,
            "https://api.mangadex.org/manga/abc?includes[]=cover_art&includes[]=author&includes[]=artist"
        );
        let c = cover_thumb_url("mid", "file.jpg");
        assert_eq!(c, "https://uploads.mangadex.org/covers/mid/file.jpg.256.jpg");
        assert!(c.contains(".256.jpg"));
        assert!(!c.contains("/chapter"));
    }
}
