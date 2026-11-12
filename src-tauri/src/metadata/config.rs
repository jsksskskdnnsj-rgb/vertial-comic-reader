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
    /// Master switch. When true, every in-app HTTP request uses the selected mode.
    #[serde(default)]
    pub app_proxy: bool,
    /// off | system | manual | local | core
    #[serde(default)]
    pub proxy_mode: String,
    #[serde(default)]
    pub proxy_username: String,
    #[serde(default)]
    pub proxy_password: String,
    #[serde(default = "default_local_proxy")]
    pub local_proxy: String,
    #[serde(default)]
    pub local_controller: String,
    #[serde(default)]
    pub local_secret: String,
    #[serde(default)]
    pub local_node: String,
    #[serde(default)]
    pub local_group: String,
    #[serde(default)]
    pub core_node: String,
    #[serde(default)]
    pub bases: SourceBases,
    #[serde(default)]
    pub custom_sources: Vec<CustomSource>,
}

fn default_comic18_domain() -> String {
    "18comic.vip".into()
}

fn default_rate_ms() -> u64 {
    400
}

fn default_local_proxy() -> String {
    "http://127.0.0.1:7897".into()
}

fn default_json_format() -> String {
    "json".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeaderPair {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomSource {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Kept so older configs round-trip. The in-app proxy switch covers every request.
    #[serde(default = "default_true")]
    pub use_proxy: bool,
    #[serde(default)]
    pub search_url: String,
    #[serde(default)]
    pub headers: Vec<HeaderPair>,
    #[serde(default = "default_json_format")]
    pub format: String,
    #[serde(default)]
    pub list: String,
    #[serde(default)]
    pub id_path: String,
    #[serde(default)]
    pub title_path: String,
    #[serde(default)]
    pub author_path: String,
    #[serde(default)]
    pub summary_path: String,
    #[serde(default)]
    pub tags_path: String,
    #[serde(default)]
    pub cover_path: String,
    #[serde(default)]
    pub page_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SourceBases {
    #[serde(default)]
    pub bangumi_api: String,
    #[serde(default)]
    pub mangadex_api: String,
    #[serde(default)]
    pub jikan_api: String,
    #[serde(default)]
    pub moegirl_api: String,
    #[serde(default)]
    pub mangaupdates_api: String,
    #[serde(default)]
    pub anilist_api: String,
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
            app_proxy: false,
            proxy_mode: String::new(),
            proxy_username: String::new(),
            proxy_password: String::new(),
            local_proxy: default_local_proxy(),
            local_controller: String::new(),
            local_secret: String::new(),
            local_node: String::new(),
            local_group: String::new(),
            core_node: String::new(),
            bases: SourceBases::default(),
            custom_sources: Vec::new(),
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
    let Ok(text) = std::fs::read_to_string(&path) else {
        return MetadataNetworkConfig::default();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return MetadataNetworkConfig::default();
    };
    let mut cfg: MetadataNetworkConfig = serde_json::from_value(value.clone()).unwrap_or_default();
    if value.get("appProxy").is_none() && !cfg.proxy_url.trim().is_empty() {
        cfg.app_proxy = true;
        if cfg.proxy_mode.trim().is_empty() {
            cfg.proxy_mode = "manual".into();
        }
    }
    cfg
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
            super::Provider::Manual | super::Provider::Custom => false,
        }
    }
}
