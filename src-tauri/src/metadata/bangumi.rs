//! Bangumi api.bgm.tv — subject type 1 (book / 漫画).

use serde::{Deserialize, Serialize};

use super::client::{build_client, send_retry};
use super::config::MetadataNetworkConfig;
use super::{MetadataCandidate, MetadataDetail, Provider};

const ROOT: &str = "https://api.bgm.tv";
const TYPE_MANGA: i64 = 1;
const HOST: &str = "api.bgm.tv";

const ALIAS_KEYS: &[&str] = &["别名", "別名", "alias", "中文名", "原作"];
const AUTHOR_KEYS: &[&str] = &["作者", "作画", "原作者", "漫画", "出版社"];

#[derive(Debug, Deserialize)]
struct SearchResponse {
    #[serde(default)]
    data: Vec<Subject>,
}

#[derive(Debug, Deserialize)]
struct Subject {
    id: i64,
    name: Option<String>,
    name_cn: Option<String>,
    date: Option<String>,
    #[serde(default)]
    volumes: Option<i64>,
    #[serde(default)]
    eps: Option<i64>,
    images: Option<Images>,
    summary: Option<String>,
    rating: Option<Rating>,
    #[serde(default)]
    tags: Vec<Tag>,
    #[serde(default)]
    infobox: Vec<Infobox>,
}

#[derive(Debug, Deserialize)]
struct Images {
    large: Option<String>,
    common: Option<String>,
    medium: Option<String>,
}

impl Images {
    fn best(&self) -> Option<String> {
        self.large
            .clone()
            .or_else(|| self.common.clone())
            .or_else(|| self.medium.clone())
            .filter(|u| !u.trim().is_empty())
    }
}

#[derive(Debug, Deserialize)]
struct Rating {
    score: Option<f64>,
    total: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct Tag {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Infobox {
    key: Option<String>,
    value: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct SearchBody<'a> {
    keyword: &'a str,
    filter: SearchFilter,
}

#[derive(Debug, Serialize)]
struct SearchFilter {
    #[serde(rename = "type")]
    kind: Vec<i64>,
    nsfw: Option<bool>,
}

fn infobox_strings(value: &serde_json::Value) -> Vec<String> {
    match value {
        serde_json::Value::String(s) => vec![s.clone()],
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                item.get("v")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .or_else(|| item.as_str().map(str::to_string))
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn infobox_lookup<'a>(boxes: &'a [Infobox], keys: &[&str]) -> Option<&'a serde_json::Value> {
    keys.iter().find_map(|key| {
        boxes
            .iter()
            .find(|b| b.key.as_deref() == Some(*key))
            .and_then(|b| b.value.as_ref())
    })
}

fn alias_title(subject: &Subject) -> Option<String> {
    let raw = infobox_lookup(&subject.infobox, ALIAS_KEYS)?;
    infobox_strings(raw)
        .into_iter()
        .flat_map(|entry| {
            entry
                .split(['|', '／', '/'])
                .map(|part| part.trim().to_string())
                .collect::<Vec<_>>()
        })
        .find(|part| {
            !part.is_empty()
                && Some(part.as_str()) != subject.name.as_deref()
                && Some(part.as_str()) != subject.name_cn.as_deref()
        })
}

fn authors(subject: &Subject) -> Vec<String> {
    let raw = match infobox_lookup(&subject.infobox, AUTHOR_KEYS) {
        Some(v) => v,
        None => return Vec::new(),
    };
    infobox_strings(raw)
        .into_iter()
        .flat_map(|s| {
            let parts: Vec<String> = s
                .split(['、', ',', '，'])
                .map(|p| p.trim().to_string())
                .collect();
            parts.into_iter()
        })
        .filter(|s| !s.is_empty())
        .collect()
}

fn normalise_rating(score: Option<f64>) -> Option<f64> {
    let score = score?;
    if !score.is_finite() || score <= 0.0 {
        return None;
    }
    let scaled = if score > 10.0 { score / 10.0 } else { score };
    Some(scaled.clamp(0.0, 10.0))
}

fn best_title(subject: &Subject) -> String {
    subject
        .name_cn
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| subject.name.clone())
        .unwrap_or_else(|| subject.id.to_string())
}

fn collect_aliases(subject: &Subject, title: &str, original: &Option<String>) -> Vec<String> {
    let alias = alias_title(subject);
    let mut aliases = Vec::new();
    for value in [
        subject.name.as_deref(),
        subject.name_cn.as_deref(),
        alias.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        let value = value.trim();
        if value.is_empty() || value == title || original.as_deref() == Some(value) {
            continue;
        }
        if !aliases.iter().any(|a| a == value) {
            aliases.push(value.to_string());
        }
    }
    aliases
}

fn http_status_message(status: reqwest::StatusCode) -> String {
    match status.as_u16() {
        401 | 403 => "Bangumi 拒绝访问，请检查个人令牌".into(),
        404 => "Bangumi 未找到该条目".into(),
        429 => "Bangumi 请求过于频繁".into(),
        _ => format!("Bangumi 请求失败（HTTP {}）", status),
    }
}

fn request(
    cfg: &MetadataNetworkConfig,
    method: reqwest::Method,
    url: &str,
) -> Result<reqwest::RequestBuilder, String> {
    let client = build_client(cfg, Provider::Bangumi)?;
    let token = cfg.bangumi_token.trim();
    let mut req = client
        .request(method, url)
        .header("Accept", "application/json");
    if !token.is_empty() {
        req = req.header("Authorization", format!("Bearer {token}"));
    }
    Ok(req)
}

pub async fn search(
    cfg: &MetadataNetworkConfig,
    query: &str,
    limit: u32,
) -> Result<Vec<MetadataCandidate>, String> {
    let root = super::endpoints::api_root(&cfg.bases.bangumi_api, ROOT);
    let url = format!("{root}/v0/search/subjects?limit={}", limit.clamp(1, 25));
    let host = super::endpoints::host_of(&url, HOST);
    let response = send_retry(cfg, &host, 100, || {
        Ok(request(cfg, reqwest::Method::POST, &url)?.json(&SearchBody {
            keyword: query,
            filter: SearchFilter {
                kind: vec![TYPE_MANGA],
                nsfw: None,
            },
        }))
    })
    .await?;
    if !response.status().is_success() {
        return Err(http_status_message(response.status()));
    }
    let parsed: SearchResponse = response
        .json()
        .await
        .map_err(|_| "Bangumi 搜索结果无法解析".to_string())?;
    Ok(parsed
        .data
        .into_iter()
        .map(|subject| {
            let title = best_title(&subject);
            let original = subject
                .name
                .clone()
                .filter(|n| *n != title)
                .or_else(|| alias_title(&subject));
            let aliases = collect_aliases(&subject, &title, &original);
            MetadataCandidate {
                provider: Provider::Bangumi,
                external_id: subject.id.to_string(),
                title,
                original_title: original,
                aliases,
                release_date: subject.date.clone(),
                cover_url: subject.images.as_ref().and_then(Images::best),
                rating: normalise_rating(subject.rating.as_ref().and_then(|r| r.score)),
                source_label: "Bangumi".into(),
                match_kind: None,
                page_url: Some(format!("https://bgm.tv/subject/{}", subject.id)),
                confidence: None,
            }
        })
        .collect())
}

pub async fn detail(cfg: &MetadataNetworkConfig, id: &str) -> Result<MetadataDetail, String> {
    let root = super::endpoints::api_root(&cfg.bases.bangumi_api, ROOT);
    let url = format!("{root}/v0/subjects/{}", id.trim());
    let host = super::endpoints::host_of(&url, HOST);
    let response = send_retry(cfg, &host, 100, || request(cfg, reqwest::Method::GET, &url)).await?;
    if !response.status().is_success() {
        return Err(http_status_message(response.status()));
    }
    let subject: Subject = response.json().await.map_err(|_| "Bangumi 详情无法解析".to_string())?;
    let title = best_title(&subject);
    let title_cn = subject.name_cn.clone().filter(|s| !s.trim().is_empty());
    let title_original = subject
        .name
        .clone()
        .filter(|n| *n != title)
        .or_else(|| alias_title(&subject));
    let tags: Vec<String> = subject
        .tags
        .iter()
        .filter_map(|t| t.name.clone())
        .filter(|n| !n.trim().is_empty())
        .collect();
    let rating = normalise_rating(subject.rating.as_ref().and_then(|r| r.score));
    let aliases = collect_aliases(&subject, &title, &title_original);
    Ok(MetadataDetail {
        provider: Provider::Bangumi,
        external_id: subject.id.to_string(),
        title,
        title_cn,
        title_original,
        summary: subject.summary.clone().filter(|s| !s.trim().is_empty()),
        authors: authors(&subject),
        tags,
        rating,
        rating_count: subject.rating.as_ref().and_then(|r| r.total),
        release_date: subject.date.clone(),
        cover_url: subject.images.as_ref().and_then(Images::best),
        aliases,
        page_url: Some(format!("https://bgm.tv/subject/{}", subject.id)),
        facts: super::WorkFacts {
            status: None,
            chapter_count: subject.eps.filter(|n| *n > 0),
            volume_count: subject.volumes.filter(|n| *n > 0),
            year: subject.date.as_deref().and_then(super::year_prefix),
        },
    })
}
