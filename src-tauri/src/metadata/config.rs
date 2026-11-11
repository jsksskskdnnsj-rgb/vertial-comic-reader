//! Network / provider settings persisted beside the database (`metadata-network.json`).

use crate::appdata::data_dir;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerSourceProxy {
    #[serde(default = "default_true")]
    pub bangumi: bool,
    #[serde(default = "default_true")]
    pub mangaupdates: bool,
    #[serde(default = "default_true")]
    pub anilist: bool,
    #[serde(default = "default_true")]
    pub comic18: bool,
    #[serde(default = "default_true")]
    pub mangadex: bool,
    #[serde(default = "default_true")]
    pub jikan: bool,
    #[serde(default = "default_true")]
    pub moegirl: bool,
}

impl Default for PerSourceProxy {
    fn default() -> Self {
        Self {
            bangumi: true,
            mangaupdates: true,
            anilist: true,
            comic18: true,
            mangadex: true,
            jikan: true,
            moegirl: true,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceToggles {
    #[serde(default = "default_true")]
    pub bangumi: bool,
    #[serde(default = "default_true")]
    pub mangaupdates: bool,
    #[serde(default = "default_true")]
    pub anilist: bool,
    #[serde(default = "default_false")]
    pub comic18: bool,
    #[serde(default = "default_true")]
    pub mangadex: bool,
    #[serde(default = "default_true")]
    pub jikan: bool,
    #[serde(default = "default_true")]
    pub moegirl: bool,
}

impl Default for SourceToggles {
    fn default() -> Self {
        Self {
            bangumi: true,
            mangaupdates: true,
            anilist: true,
            comic18: false,
            mangadex: true,
            jikan: true,
            moegirl: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataNetworkConfig {
    /// Optional Bangumi personal access token (adult / restricted subjects).
    #[serde(default)]
    pub bangumi_token: String,
    /// `http://host:port` or `socks5://host:port`
    #[serde(default)]
    pub proxy_url: String,
    #[serde(default)]
    pub proxy_per_source: PerSourceProxy,
    #[serde(default)]
    pub sources: SourceToggles,
    /// Minimum delay between outbound requests to the same host (milliseconds).
    #[serde(default = "default_rate_ms")]
    pub rate_limit_ms: u64,
    /// Batch auto-match: bind when match is exact/strong without prompting.
    #[serde(default = "default_true")]
    pub auto_bind_confident: bool,
    /// 入口域名（会跟随 301 跳转到当前镜像）。
    #[serde(default = "default_comic18_domain")]
    pub comic18_domain: String,
}

fn default_comic18_domain() -> String {
    "18comic.vip".into()
}

fn default_rate_ms() -> u64 {
    400
}

impl Default for MetadataNetworkConfig {
    fn default() -> Self {
        Self {
            bangumi_token: String::new(),
            proxy_url: String::new(),
            proxy_per_source: PerSourceProxy::default(),
            sources: SourceToggles::default(),
            comic18_domain: default_comic18_domain(),
            rate_limit_ms: default_rate_ms(),
            auto_bind_confident: true,
        }
    }
}

pub fn config_path() -> Result<std::path::PathBuf, String> {
    Ok(data_dir()?.join("metadata-network.json"))
}

pub fn load() -> MetadataNetworkConfig {
    let path = match config_path() {
        Ok(p) => p,
        Err(_) => return MetadataNetworkConfig::default(),
    };
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(cfg: &MetadataNetworkConfig) -> Result<(), String> {
    let path = config_path()?;
    let s = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(path, s).map_err(|e| e.to_string())
}

impl MetadataNetworkConfig {
    pub fn provider_order(&self) -> Vec<super::Provider> {
        let mut out = Vec::new();
        if self.sources.bangumi {
            out.push(super::Provider::Bangumi);
        }
        if self.sources.moegirl {
            out.push(super::Provider::Moegirl);
        }
        if self.sources.mangadex {
            out.push(super::Provider::Mangadex);
        }
        if self.sources.jikan {
            out.push(super::Provider::Jikan);
        }
        if self.sources.mangaupdates {
            out.push(super::Provider::Mangaupdates);
        }
        if self.sources.anilist {
            out.push(super::Provider::Anilist);
        }
        if self.sources.comic18 {
            out.push(super::Provider::Comic18);
        }
        out
    }

    pub fn enabled(&self, provider: super::Provider) -> bool {
        match provider {
            super::Provider::Bangumi => self.sources.bangumi,
            super::Provider::Mangaupdates => self.sources.mangaupdates,
            super::Provider::Anilist => self.sources.anilist,
            super::Provider::Comic18 => self.sources.comic18,
            super::Provider::Mangadex => self.sources.mangadex,
            super::Provider::Jikan => self.sources.jikan,
            super::Provider::Moegirl => self.sources.moegirl,
            super::Provider::Manual => false,
        }
    }

    pub fn use_proxy_for(&self, provider: super::Provider) -> bool {
        if self.proxy_url.trim().is_empty() {
            return false;
        }
        match provider {
            super::Provider::Bangumi => self.proxy_per_source.bangumi,
            super::Provider::Mangaupdates => self.proxy_per_source.mangaupdates,
            super::Provider::Anilist => self.proxy_per_source.anilist,
            super::Provider::Comic18 => self.proxy_per_source.comic18,
            super::Provider::Mangadex => self.proxy_per_source.mangadex,
            super::Provider::Jikan => self.proxy_per_source.jikan,
            super::Provider::Moegirl => self.proxy_per_source.moegirl,
            super::Provider::Manual => false,
        }
    }
}
