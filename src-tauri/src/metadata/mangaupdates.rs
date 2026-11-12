//! MangaUpdates REST API — strong coverage for Korean webtoon / manhwa entries.

use serde::Deserialize;

use super::client::send_retry;
use super::config::MetadataNetworkConfig;
use super::{MetadataCandidate, MetadataDetail, Provider};

const ROOT: &str = "https://api.mangaupdates.com";
const HOST: &str = "api.mangaupdates.com";

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    results: Vec<SearchHit>,
}

#[derive(Debug, Deserialize)]
struct SearchHit {
    record: SeriesStub,
}

#[derive(Debug, Deserialize)]
struct SeriesStub {
    series_id: i64,
    title: String,
    #[serde(default)]
    description: Option<String>,
    image: Option<Images>,
    #[serde(default)]
    year: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Images {
    url: Option<MetadataImageUrl>,
}

#[derive(Debug, Deserialize)]
struct MetadataImageUrl {
    #[serde(default)]
    original: Option<String>,
    #[serde(default)]
    thumb: Option<String>,
}

impl Images {
    fn best(&self) -> Option<String> {
        self.url
            .as_ref()
            .and_then(|u| u.original.clone().or_else(|| u.thumb.clone()))
            .filter(|s| !s.trim().is_empty())
    }
}

#[derive(Debug, Deserialize)]
struct SeriesDetail {
    series_id: i64,
    title: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    associated: Vec<AssociatedTitle>,
    #[serde(default)]
    description: Option<String>,
    image: Option<Images>,
    #[serde(default)]
    year: Option<String>,
    bayesian_rating: Option<f64>,
    #[serde(default)]
    rating_votes: Option<i64>,
    #[serde(default)]
    genres: Vec<GenreRow>,
    #[serde(default)]
    authors: Vec<AuthorRow>,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AssociatedTitle {
    title: String,
}

#[derive(Debug, Deserialize)]
struct GenreRow {
    genre: String,
}

#[derive(Debug, Deserialize)]
struct AuthorRow {
    name: String,
    #[serde(default)]
    r#type: Option<String>,
}

fn normalize_mu_rating(score: Option<f64>) -> Option<f64> {
    let score = score?;
    if !score.is_finite() || score <= 0.0 {
        return None;
    }
    Some((score / 2.0).clamp(0.0, 10.0))
}

fn strip_html(s: &str) -> String {
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

fn http_status_message(status: reqwest::StatusCode) -> String {
    match status.as_u16() {
        429 => "MangaUpdates 请求过于频繁".into(),
        _ => format!("MangaUpdates 请求失败（HTTP {}）", status),
    }
}

pub async fn search(
    cfg: &MetadataNetworkConfig,
    query: &str,
    limit: u32,
) -> Result<Vec<MetadataCandidate>, String> {
    let body = serde_json::json!({
        "search": query,
        "page": 1,
        "perpage": limit.clamp(1, 25),
    });
    let root = super::endpoints::api_root(&cfg.bases.mangaupdates_api, ROOT);
    let search = format!("{root}/v1/series/search");
    let host = super::endpoints::host_of(&search, HOST);
    let response = send_retry(cfg, &host, 100, || {
        let client = super::client::build_client(cfg, Provider::Mangaupdates)?;
        Ok(client
            .post(&search)
            .header("Accept", "application/json")
            .json(&body))
    })
    .await?;
    if !response.status().is_success() {
        return Err(http_status_message(response.status()));
    }
    let parsed: SearchResponse = response
        .json()
        .await
        .map_err(|_| "MangaUpdates 搜索结果无法解析".to_string())?;
    Ok(parsed
        .results
        .into_iter()
        .map(|hit| {
            let s = hit.record;
            MetadataCandidate {
                provider: Provider::Mangaupdates,
                external_id: s.series_id.to_string(),
                title: s.title.clone(),
                original_title: None,
                aliases: Vec::new(),
                release_date: s.year.clone(),
                cover_url: s.image.as_ref().and_then(Images::best),
                rating: None,
                source_label: "MangaUpdates".into(),
                match_kind: None,
                page_url: None,
                confidence: None,
            }
        })
        .collect())
}

pub async fn detail(cfg: &MetadataNetworkConfig, id: &str) -> Result<MetadataDetail, String> {
    let root = super::endpoints::api_root(&cfg.bases.mangaupdates_api, ROOT);
    let url = format!("{root}/v1/series/{}", id.trim());
    let host = super::endpoints::host_of(&url, HOST);
    let response = send_retry(cfg, &host, 100, || {
        let client = super::client::build_client(cfg, Provider::Mangaupdates)?;
        Ok(client.get(&url).header("Accept", "application/json"))
    })
    .await?;
    if !response.status().is_success() {
        return Err(http_status_message(response.status()));
    }
    let s: SeriesDetail = response
        .json()
        .await
        .map_err(|_| "MangaUpdates 详情无法解析".to_string())?;
    let aliases: Vec<String> = s
        .associated
        .iter()
        .map(|a| a.title.trim().to_string())
        .filter(|t| !t.is_empty() && *t != s.title)
        .collect();
    let authors: Vec<String> = s
        .authors
        .iter()
        .map(|a| a.name.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
    let tags: Vec<String> = s
        .genres
        .iter()
        .map(|g| g.genre.trim().to_string())
        .filter(|n| !n.is_empty())
        .collect();
  let summary = s
        .description
        .as_deref()
        .map(strip_html)
        .filter(|t| !t.is_empty());
    Ok(MetadataDetail {
        provider: Provider::Mangaupdates,
        external_id: s.series_id.to_string(),
        title: s.title,
        title_cn: None,
        title_original: aliases.first().cloned(),
        summary,
        authors,
        tags,
        rating: normalize_mu_rating(s.bayesian_rating),
        rating_count: s.rating_votes,
        release_date: s.year.clone(),
        cover_url: s.image.as_ref().and_then(Images::best),
        aliases,
        page_url: s.url.clone(),
        facts: super::WorkFacts {
            status: s.status.clone().filter(|t| !t.trim().is_empty()),
            chapter_count: None,
            volume_count: None,
            year: s.year.as_deref().and_then(super::year_prefix),
        },
    })
}
