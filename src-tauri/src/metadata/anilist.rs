//! AniList GraphQL (manga media type).

use serde::Deserialize;

use super::client::{build_client, send_retry};
use super::config::MetadataNetworkConfig;
use super::{MetadataCandidate, MetadataDetail, Provider};

const ENDPOINT: &str = "https://graphql.anilist.co";
const HOST: &str = "graphql.anilist.co";

const SEARCH_QUERY: &str = r#"
query ($search: String, $perPage: Int) {
  Page(page: 1, perPage: $perPage) {
    media(search: $search, type: MANGA, sort: SEARCH_MATCH) {
      id
      title { romaji english native userPreferred }
      synonyms
      status
      startDate { year month day }
      coverImage { large medium }
      averageScore
      tags { name }
    }
  }
}"#;

const DETAIL_QUERY: &str = r#"
query ($id: Int) {
  Media(id: $id, type: MANGA) {
    id
    title { romaji english native userPreferred }
    synonyms
    description(asHtml: false)
    status
    chapters
    volumes
    startDate { year month day }
    coverImage { large extraLarge }
    averageScore
    favourites
    tags { name }
    staff(perPage: 12, sort: RELEVANCE) {
      edges { role node { name { full userPreferred native } } }
    }
  }
}"#;

#[derive(Debug, Deserialize)]
struct GqlResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GqlError>>,
}

#[derive(Debug, Deserialize)]
struct GqlError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct SearchData {
    #[serde(rename = "Page")]
    page: SearchPage,
}

#[derive(Debug, Deserialize)]
struct SearchPage {
    media: Vec<MediaStub>,
}

#[derive(Debug, Deserialize)]
struct MediaStub {
    id: i64,
    title: TitleBlock,
    #[serde(default)]
    synonyms: Vec<String>,
    startDate: Option<DateBlock>,
    coverImage: Option<CoverBlock>,
    averageScore: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct DetailData {
    #[serde(rename = "Media")]
    media: Option<MediaDetail>,
}

#[derive(Debug, Deserialize)]
struct MediaDetail {
    id: i64,
    title: TitleBlock,
    #[serde(default)]
    synonyms: Vec<String>,
    description: Option<String>,
    status: Option<String>,
    chapters: Option<i64>,
    volumes: Option<i64>,
    startDate: Option<DateBlock>,
    coverImage: Option<CoverBlock>,
    averageScore: Option<i64>,
    favourites: Option<i64>,
    tags: Option<Vec<TagRow>>,
    staff: Option<StaffBlock>,
}

#[derive(Debug, Deserialize)]
struct TitleBlock {
    romaji: Option<String>,
    english: Option<String>,
    native: Option<String>,
    userPreferred: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DateBlock {
    year: Option<i64>,
    month: Option<i64>,
    day: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct CoverBlock {
  #[serde(default)]
    large: Option<String>,
    #[serde(default)]
    medium: Option<String>,
    #[serde(default)]
    extraLarge: Option<String>,
}

impl CoverBlock {
    fn best(&self) -> Option<String> {
        self.extraLarge
            .clone()
            .or_else(|| self.large.clone())
            .or_else(|| self.medium.clone())
            .filter(|u| !u.trim().is_empty())
    }
}

#[derive(Debug, Deserialize)]
struct TagRow {
    name: String,
}

#[derive(Debug, Deserialize)]
struct StaffBlock {
    edges: Vec<StaffEdge>,
}

#[derive(Debug, Deserialize)]
struct StaffEdge {
    role: Option<String>,
    node: StaffNode,
}

#[derive(Debug, Deserialize)]
struct StaffNode {
    name: NameBlock,
}

#[derive(Debug, Deserialize)]
struct NameBlock {
    full: Option<String>,
    userPreferred: Option<String>,
    native: Option<String>,
}

fn pick_title(t: &TitleBlock) -> String {
    t.userPreferred
        .clone()
        .or_else(|| t.english.clone())
        .or_else(|| t.romaji.clone())
        .or_else(|| t.native.clone())
        .unwrap_or_else(|| "—".into())
}

fn title_aliases(t: &TitleBlock, main: &str) -> (Option<String>, Vec<String>) {
    let mut aliases = Vec::new();
    let original = t.native.clone().filter(|n| *n != main);
    for v in [t.romaji.as_deref(), t.english.as_deref(), t.native.as_deref()] {
        let Some(v) = v else { continue };
        let v = v.trim();
        if v.is_empty() || v == main {
            continue;
        }
        if !aliases.iter().any(|a| a == v) {
            aliases.push(v.to_string());
        }
    }
    (original, aliases)
}

fn format_date(d: &Option<DateBlock>) -> Option<String> {
    let d = d.as_ref()?;
    let y = d.year?;
    let m = d.month.unwrap_or(0);
    let day = d.day.unwrap_or(0);
    if m > 0 && day > 0 {
        Some(format!("{y:04}-{m:02}-{day:02}"))
    } else {
        Some(y.to_string())
    }
}

fn anilist_score(score: Option<i64>) -> Option<f64> {
    let s = score?;
    if s <= 0 {
        return None;
    }
    Some((s as f64 / 10.0).clamp(0.0, 10.0))
}

async fn gql<T: for<'de> Deserialize<'de>>(
    cfg: &MetadataNetworkConfig,
    query: &str,
    variables: serde_json::Value,
) -> Result<T, String> {
    let endpoint = super::endpoints::api_root(&cfg.bases.anilist_api, ENDPOINT);
    let host = super::endpoints::host_of(&endpoint, HOST);
    let response = send_retry(cfg, &host, 100, || {
        let client = build_client(cfg, Provider::Anilist)?;
        Ok(client
            .post(&endpoint)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&serde_json::json!({ "query": query, "variables": variables })))
    })
    .await?;
    if !response.status().is_success() {
        return Err(format!("AniList 请求失败（HTTP {}）", response.status()));
    }
    let envelope: GqlResponse<T> = response
        .json()
        .await
        .map_err(|_| "AniList 响应无法解析".to_string())?;
    if let Some(errs) = envelope.errors {
        if let Some(first) = errs.first() {
            return Err(format!("AniList：{}", first.message));
        }
    }
    envelope.data.ok_or_else(|| "AniList 未返回数据".into())
}

pub async fn search(
    cfg: &MetadataNetworkConfig,
    query: &str,
    limit: u32,
) -> Result<Vec<MetadataCandidate>, String> {
    let data: SearchData = gql(
        cfg,
        SEARCH_QUERY,
        serde_json::json!({ "search": query, "perPage": limit.clamp(1, 25) }),
    )
    .await?;
    Ok(data
        .page
        .media
        .into_iter()
        .map(|m| {
            let title = pick_title(&m.title);
            let (original, mut aliases) = title_aliases(&m.title, &title);
            for s in &m.synonyms {
                let s = s.trim();
                if !s.is_empty() && s != title && !aliases.iter().any(|a| a == s) {
                    aliases.push(s.to_string());
                }
            }
            MetadataCandidate {
                provider: Provider::Anilist,
                external_id: m.id.to_string(),
                title,
                original_title: original,
                aliases,
                release_date: format_date(&m.startDate),
                cover_url: m.coverImage.as_ref().and_then(CoverBlock::best),
                rating: anilist_score(m.averageScore),
                source_label: "AniList".into(),
                match_kind: None,
                page_url: Some(format!("https://anilist.co/manga/{}", m.id)),
                confidence: None,
            }
        })
        .collect())
}

pub async fn detail(cfg: &MetadataNetworkConfig, id: &str) -> Result<MetadataDetail, String> {
    let id: i64 = id.parse().map_err(|_| "AniList ID 无效".to_string())?;
    let data: DetailData = gql(cfg, DETAIL_QUERY, serde_json::json!({ "id": id })).await?;
    let m = data.media.ok_or_else(|| "AniList 条目不存在".to_string())?;
    let title = pick_title(&m.title);
    let (title_original, mut aliases) = title_aliases(&m.title, &title);
    for s in &m.synonyms {
        let s = s.trim();
        if !s.is_empty() && s != title && !aliases.iter().any(|a| a == s) {
            aliases.push(s.to_string());
        }
    }
    let title_cn = m.title.english.clone().filter(|s| *s != title);
    let tags: Vec<String> = m
        .tags
        .unwrap_or_default()
        .into_iter()
        .map(|t| t.name)
        .filter(|n| !n.trim().is_empty())
        .collect();
    let authors: Vec<String> = m
        .staff
        .map(|s| {
            s.edges
                .into_iter()
                .filter(|e| {
                    e.role
                        .as_deref()
                        .map(|r| r.contains("Story") || r.contains("Art") || r.contains("作者"))
                        .unwrap_or(true)
                })
                .filter_map(|e| {
                    e.node
                        .name
                        .userPreferred
                        .clone()
                        .or_else(|| e.node.name.full.clone())
                })
                .collect()
        })
        .unwrap_or_default();
    aliases.retain(|a| *a != title);
    let summary = m
        .description
        .as_deref()
        .map(|s| s.replace("<br>", "\n").replace("<i>", "").replace("</i>", ""))
        .filter(|s| !s.trim().is_empty());
    Ok(MetadataDetail {
        provider: Provider::Anilist,
        external_id: m.id.to_string(),
        title,
        title_cn,
        title_original,
        summary,
        authors,
        tags,
        rating: anilist_score(m.averageScore),
        rating_count: m.favourites,
        release_date: format_date(&m.startDate),
        cover_url: m.coverImage.as_ref().and_then(CoverBlock::best),
        aliases,
        page_url: Some(format!("https://anilist.co/manga/{}", m.id)),
        facts: super::WorkFacts {
            status: m.status.clone().filter(|s| !s.trim().is_empty()),
            chapter_count: m.chapters.filter(|n| *n > 0),
            volume_count: m.volumes.filter(|n| *n > 0),
            year: m.startDate.as_ref().and_then(|d| d.year).and_then(|y| i32::try_from(y).ok()),
        },
    })
}
