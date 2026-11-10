//! Online work metadata (Bangumi → MangaUpdates → AniList). HTTP stays in Rust; DB
//! writes only after the user confirms a match (or high-confidence auto-match).

pub mod anilist;
pub mod bangumi;
pub mod client;
pub mod commands;
pub mod comic18;
pub mod config;
pub mod mangaupdates;
pub mod matching;
pub mod store;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Bangumi,
    Mangaupdates,
    Anilist,
    #[serde(rename = "comic18")]
    Comic18,
}

impl Provider {
    pub fn as_str(self) -> &'static str {
        match self {
            Provider::Bangumi => "bangumi",
            Provider::Mangaupdates => "mangaupdates",
            Provider::Anilist => "anilist",
            Provider::Comic18 => "comic18",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "bangumi" => Some(Self::Bangumi),
            "mangaupdates" | "mu" => Some(Self::Mangaupdates),
            "anilist" => Some(Self::Anilist),
            "comic18" | "18comic" | "jmcomic" => Some(Self::Comic18),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Provider::Bangumi => "Bangumi",
            Provider::Mangaupdates => "MangaUpdates",
            Provider::Anilist => "AniList",
            Provider::Comic18 => "18comic",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataCandidate {
    pub provider: Provider,
    pub external_id: String,
    pub title: String,
    pub original_title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    pub release_date: Option<String>,
    pub cover_url: Option<String>,
    /// 0–10 when known
    pub rating: Option<f64>,
    pub source_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataDetail {
    pub provider: Provider,
    pub external_id: String,
    pub title: String,
    pub title_cn: Option<String>,
    pub title_original: Option<String>,
    pub summary: Option<String>,
    pub authors: Vec<String>,
    pub tags: Vec<String>,
    pub rating: Option<f64>,
    pub rating_count: Option<i64>,
    pub release_date: Option<String>,
    pub cover_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_url: Option<String>,
}

impl MetadataDetail {
    pub fn infer_page_url(&self) -> Option<String> {
        if let Some(u) = &self.page_url {
            return Some(u.clone());
        }
        match self.provider {
            Provider::Bangumi => Some(format!("https://bgm.tv/subject/{}", self.external_id)),
            Provider::Mangaupdates => None,
            Provider::Anilist => Some(format!("https://anilist.co/manga/{}", self.external_id)),
            Provider::Comic18 => None,
        }
    }
}

pub(crate) const USER_AGENT: &str =
    "MH-Reader/0.8.2 (https://github.com/jsksskskdnnsj-rgb/vertial-comic-reader; metadata)";
