//! MyAnimeList metadata via the public Jikan API. No chapter images.

use serde::Deserialize;

use super::client::send_retry;
use super::config::MetadataNetworkConfig;
use super::{year_prefix, MetadataCandidate, MetadataDetail, Provider, WorkFacts};

const HOST: &str = "api.jikan.moe";
const API: &str = "https://api.jikan.moe/v4/manga";
/// Jikan asks clients to stay near one request a second.
const GAP_MS: u64 = 1100;

#[derive(Debug, Deserialize)]
struct SearchBody {
    #[serde(default)]
    data: Vec<Manga>,
}

#[derive(Debug, Deserialize)]
struct OneBody {
    data: Manga,
}

#[derive(Debug, Deserialize)]
struct Manga {
    mal_id: i64,
    url: Option<String>,
    title: Option<String>,
    title_english: Option<String>,
    title_japanese: Option<String>,
    #[serde(default)]
    titles: Vec<TitleRow>,
    synopsis: Option<String>,
    status: Option<String>,
    chapters: Option<i64>,
    volumes: Option<i64>,
    score: Option<f64>,
    scored_by: Option<i64>,
    published: Option<Published>,
    images: Option<Images>,
    #[serde(default)]
    authors: Vec<Named>,
    #[serde(default)]
    genres: Vec<Named>,
    #[serde(default)]
    themes: Vec<Named>,
}

#[derive(Debug, Deserialize)]
struct TitleRow {
    #[serde(default)]
    title: String,
}

#[derive(Debug, Deserialize)]
struct Published {
    from: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Images {
    jpg: Option<Jpg>,
}

#[derive(Debug, Deserialize)]
struct Jpg {
    image_url: Option<String>,
    large_image_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Named {
    name: Option<String>,
}

pub(crate) fn search_url(query: &str, limit: u32) -> String {
    let q = url::form_urlencoded::byte_serialize(query.trim().as_bytes()).collect::<String>();
    format!("{API}?q={q}&limit={}&sfw=false", limit.clamp(1, 25))
}

fn cover_of(images: &Option<Images>) -> Option<String> {
    let jpg = images.as_ref()?.jpg.as_ref()?;
    jpg.large_image_url
        .clone()
        .or_else(|| jpg.image_url.clone())
        .filter(|s| !s.trim().is_empty())
}

fn names(rows: &[Named]) -> Vec<String> {
    let mut out = Vec::new();
    for row in rows {
        let Some(name) = row.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        if !out.iter().any(|s| s == name) {
            out.push(name.to_string());
        }
    }
    out
}

fn aliases_of(m: &Manga, title: &str) -> (Option<String>, Vec<String>) {
    let mut aliases = Vec::new();
    for value in [m.title.as_deref(), m.title_english.as_deref(), m.title_japanese.as_deref()]
        .into_iter()
        .flatten()
        .chain(m.titles.iter().map(|t| t.title.as_str()))
    {
        let value = value.trim();
        if value.is_empty() || value == title || aliases.iter().any(|a| a == value) {
            continue;
        }
        aliases.push(value.to_string());
    }
    let original = m
        .title_japanese
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != title)
        .map(|s| s.to_string());
    (original, aliases)
}

fn to_candidate(m: Manga) -> MetadataCandidate {
    let title = m
        .title_english
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| m.title.clone())
        .unwrap_or_else(|| m.mal_id.to_string());
    let (original, aliases) = aliases_of(&m, &title);
    let year = m.published.as_ref().and_then(|p| p.from.as_deref()).and_then(year_prefix);
    MetadataCandidate {
        provider: Provider::Jikan,
        external_id: m.mal_id.to_string(),
        title,
        original_title: original,
        aliases,
        release_date: year.map(|y| y.to_string()).or_else(|| m.published.as_ref().and_then(|p| p.from.clone())),
        cover_url: cover_of(&m.images),
        rating: m.score.filter(|s| s.is_finite() && *s > 0.0),
        source_label: "MyAnimeList".into(),
        match_kind: None,
        page_url: m.url.clone().or_else(|| Some(format!("https://myanimelist.net/manga/{}", m.mal_id))),
        confidence: None,
    }
}

fn to_detail(m: Manga) -> MetadataDetail {
    let id = m.mal_id;
    let cand = to_candidate_parts(&m);
    let year = m.published.as_ref().and_then(|p| p.from.as_deref()).and_then(year_prefix);
    MetadataDetail {
        provider: Provider::Jikan,
        external_id: id.to_string(),
        title: cand.0,
        title_cn: None,
        title_original: cand.1,
        summary: m.synopsis.clone().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        authors: names(&m.authors),
        tags: names(&m.genres).into_iter().chain(names(&m.themes)).take(24).collect(),
        rating: m.score.filter(|s| s.is_finite() && *s > 0.0),
        rating_count: m.scored_by,
        release_date: year.map(|y| y.to_string()),
        cover_url: cover_of(&m.images),
        aliases: cand.2,
        page_url: m.url.or_else(|| Some(format!("https://myanimelist.net/manga/{id}"))),
        facts: WorkFacts {
            status: m.status.filter(|s| !s.trim().is_empty()),
            chapter_count: m.chapters.filter(|n| *n > 0),
            volume_count: m.volumes.filter(|n| *n > 0),
            year,
        },
    }
}

fn to_candidate_parts(m: &Manga) -> (String, Option<String>, Vec<String>) {
    let title = m
        .title_english
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| m.title.clone())
        .unwrap_or_else(|| m.mal_id.to_string());
    let (original, aliases) = aliases_of(m, &title);
    (title, original, aliases)
}

pub async fn search(
    cfg: &MetadataNetworkConfig,
    query: &str,
    limit: u32,
) -> Result<Vec<MetadataCandidate>, String> {
    let url = search_url(query, limit);
    let response = send_retry(cfg, HOST, GAP_MS, || {
        let client = super::client::build_client(cfg, Provider::Jikan)?;
        Ok(client.get(&url).header("Accept", "application/json"))
    })
    .await?;
    if !response.status().is_success() {
        return Err(format!("Jikan 请求失败（HTTP {}）", response.status()));
    }
    let body: SearchBody = response.json().await.map_err(|_| "Jikan 搜索结果无法解析".to_string())?;
    Ok(body.data.into_iter().map(to_candidate).collect())
}

pub async fn detail(cfg: &MetadataNetworkConfig, id: &str) -> Result<MetadataDetail, String> {
    let url = format!("{API}/{}", id.trim());
    let response = send_retry(cfg, HOST, GAP_MS, || {
        let client = super::client::build_client(cfg, Provider::Jikan)?;
        Ok(client.get(&url).header("Accept", "application/json"))
    })
    .await?;
    if !response.status().is_success() {
        return Err(format!("Jikan 详情失败（HTTP {}）", response.status()));
    }
    let body: OneBody = response.json().await.map_err(|_| "Jikan 详情无法解析".to_string())?;
    Ok(to_detail(body.data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_url_is_manga_metadata() {
        let u = search_url("one piece", 3);
        assert!(u.starts_with("https://api.jikan.moe/v4/manga?"));
        assert!(!u.contains("/chapters"));
        assert!(u.contains("q=one+piece") || u.contains("q=one%20piece"));
    }
}
