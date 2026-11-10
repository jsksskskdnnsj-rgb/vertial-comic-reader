//! Tauri commands for metadata lookup, binding, and batch auto-match.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

use super::anilist;
use super::bangumi;
use super::client::{build_client, offline_message};
use super::config::{self, MetadataNetworkConfig};
use super::comic18;
use super::mangaupdates;
use super::matching::{self, Match};
use super::store::{self, BoundMetadata, PendingMatch};
use super::{MetadataCandidate, MetadataDetail, Provider, USER_AGENT};
use crate::appdata::data_dir;
use crate::cover_protocol::CoversDir;

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub provider: Provider,
    pub error: Option<String>,
    pub candidates: Vec<MetadataCandidate>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoMatchItem {
    pub book_id: i64,
    pub status: String,
    pub provider: Option<String>,
    pub match_kind: Option<String>,
    pub message: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BindOptions {
    pub import_tags: bool,
    pub use_remote_cover: bool,
}

fn metadata_covers_dir() -> Result<PathBuf, String> {
    let dir = data_dir()?.join("metadata-covers");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

async fn search_provider(
    cfg: &MetadataNetworkConfig,
    provider: Provider,
    query: &str,
    limit: u32,
) -> SearchResult {
    let candidates = match provider {
        Provider::Bangumi => bangumi::search(cfg, query, limit).await,
        Provider::Mangaupdates => mangaupdates::search(cfg, query, limit).await,
        Provider::Anilist => anilist::search(cfg, query, limit).await,
        Provider::Comic18 => comic18::search(cfg, query, limit).await,
    };
    match candidates {
        Ok(mut list) => {
            for c in &mut list {
                let m = matching::score(query, c);
                if m != Match::None {
                    c.match_kind = Some(m.as_str().into());
                }
            }
            SearchResult {
                provider,
                error: None,
                candidates: list,
            }
        }
        Err(e) => SearchResult {
            provider,
            error: Some(e),
            candidates: Vec::new(),
        },
    }
}

async fn search_fallback_chain(
    cfg: &MetadataNetworkConfig,
    query: &str,
    limit: u32,
) -> Vec<MetadataCandidate> {
    for p in cfg.provider_order() {
        let hit = search_provider(cfg, p, query, limit).await;
        if !hit.candidates.is_empty() {
            return hit.candidates;
        }
    }
    Vec::new()
}

pub(crate) async fn fetch_detail(cfg: &MetadataNetworkConfig, provider: Provider, id: &str) -> Result<MetadataDetail, String> {
    match provider {
        Provider::Bangumi => bangumi::detail(cfg, id).await,
        Provider::Mangaupdates => mangaupdates::detail(cfg, id).await,
        Provider::Anilist => anilist::detail(cfg, id).await,
        Provider::Comic18 => comic18::detail(cfg, id).await,
    }
}

fn open_in_system_browser(url: &str) -> Result<(), String> {
    let url = url.trim();
    if !url.starts_with("https://") {
        return Err("仅支持以 https:// 开头的链接".into());
    }
    if url.contains('\n') || url.contains('\r') {
        return Err("链接无效".into());
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        return Err("当前系统不支持打开外部浏览器".into());
    }
    Ok(())
}

async fn cache_cover_file(book_id: i64, url: &str, cfg: &MetadataNetworkConfig, provider: Provider) -> Result<String, String> {
    let client = build_client(cfg, provider)?;
    let response = client
        .get(url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await
        .map_err(|e| offline_message(&e))?;
    if !response.status().is_success() {
        return Err(format!("封面下载失败（HTTP {}）", response.status()));
    }
    let bytes = response.bytes().await.map_err(|_| "封面数据读取失败".to_string())?;
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("封面文件过大".into());
    }
    let ext = if bytes.starts_with(b"\x89PNG") {
        "png"
    } else if bytes.starts_with(b"RIFF") && bytes.len() > 12 && &bytes[8..12] == b"WEBP" {
        "webp"
    } else {
        "jpg"
    };
    let name = format!("book-{book_id}.{ext}");
    let path = metadata_covers_dir()?.join(&name);
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    store::set_cover_cache(book_id, &name)?;
    Ok(name)
}

fn import_tags(book_id: i64, tags: &[String]) -> Result<(), String> {
    let conn = crate::appdata::db_conn()?;
    let now = crate::appdata::now_secs();
    for name in tags {
        let name = name.trim();
        if name.is_empty() || name.len() > 48 {
            continue;
        }
        let tag_id: i64 = match conn.query_row(
            "SELECT id FROM tags WHERE name=?1 COLLATE NOCASE",
            [name],
            |r| r.get(0),
        ) {
            Ok(id) => id,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                conn.execute(
                    "INSERT INTO tags(name,color,created_at) VALUES(?1,?2,?3)",
                    rusqlite::params![name, "#80e5b6", now],
                )
                .map_err(|e| e.to_string())?;
                conn.last_insert_rowid()
            }
            Err(e) => return Err(e.to_string()),
        };
        let _ = conn.execute(
            "INSERT OR IGNORE INTO book_tags(book_id, tag_id) VALUES (?1, ?2)",
            rusqlite::params![book_id, tag_id],
        );
    }
    Ok(())
}

fn apply_remote_cover_to_shelf(
    app: &AppHandle,
    book_id: i64,
    cache_name: &str,
) -> Result<(), String> {
    let src = metadata_covers_dir()?.join(cache_name);
    if !src.is_file() {
        return Err("缓存封面不存在".into());
    }
    let covers = app.state::<CoversDir>().0.clone();
    std::fs::create_dir_all(covers.as_ref()).map_err(|e| e.to_string())?;
    let dest = covers.join(format!("{book_id}.jpg"));
    let bytes = std::fs::read(&src).map_err(|e| e.to_string())?;
    let img = image::load_from_memory(&bytes).map_err(|_| "封面无法解码".to_string())?;
    let thumb = img.thumbnail(240, 320);
    thumb
        .save_with_format(&dest, image::ImageFormat::Jpeg)
        .map_err(|e| e.to_string())?;
    let conn = crate::appdata::db_conn()?;
    conn.execute(
        "UPDATE shelf SET cover_ver = cover_ver + 1 WHERE id=?1",
        [book_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn metadata_get_config() -> Result<MetadataNetworkConfig, String> {
    Ok(config::load())
}

#[tauri::command]
pub fn metadata_set_config(cfg: MetadataNetworkConfig) -> Result<(), String> {
    config::save(&cfg)
}

#[tauri::command]
pub async fn metadata_search(
    query: String,
    limit: Option<u32>,
    providers: Option<Vec<String>>,
    fallback_chain: Option<bool>,
) -> Result<Vec<SearchResult>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("搜索词不能为空".into());
    }
    let cfg = config::load();
    let limit = limit.unwrap_or(12).clamp(1, 25);
    if fallback_chain.unwrap_or(false) {
        let candidates = search_fallback_chain(&cfg, query, limit).await;
        return Ok(vec![SearchResult {
            provider: Provider::Bangumi,
            error: None,
            candidates,
        }]);
    }
    let list: Vec<Provider> = providers
        .unwrap_or_default()
        .iter()
        .filter_map(|s| Provider::parse(s))
        .collect();
    let order = if list.is_empty() {
        cfg.provider_order()
    } else {
        list
    };
    let mut out = Vec::new();
    for p in order {
        if !cfg.sources.bangumi && p == Provider::Bangumi {
            continue;
        }
        if !cfg.sources.mangaupdates && p == Provider::Mangaupdates {
            continue;
        }
        if !cfg.sources.anilist && p == Provider::Anilist {
            continue;
        }
        if !cfg.sources.comic18 && p == Provider::Comic18 {
            continue;
        }
        out.push(search_provider(&cfg, p, query, limit).await);
    }
    Ok(out)
}

#[tauri::command]
pub async fn metadata_detail(provider: String, external_id: String) -> Result<MetadataDetail, String> {
    let provider = Provider::parse(&provider).ok_or_else(|| "未知数据源".to_string())?;
    let cfg = config::load();
    fetch_detail(&cfg, provider, &external_id).await
}

#[tauri::command]
pub fn metadata_get(book_id: i64) -> Result<Option<BoundMetadata>, String> {
    store::get_bound(book_id)
}

#[tauri::command]
pub fn metadata_list_pending() -> Result<Vec<PendingMatch>, String> {
    store::list_pending()
}

#[tauri::command]
pub async fn metadata_bind(
    app: AppHandle,
    book_id: i64,
    provider: String,
    external_id: String,
    match_kind: Option<String>,
    options: Option<BindOptions>,
) -> Result<BoundMetadata, String> {
    let provider = Provider::parse(&provider).ok_or_else(|| "未知数据源".to_string())?;
    let cfg = config::load();
    let detail = fetch_detail(&cfg, provider, &external_id).await?;
    let match_kind = match_kind.unwrap_or_else(|| "manual".into());
    let mut cover_cache = None;
    if let Some(url) = detail.cover_url.as_deref() {
        if let Ok(name) = cache_cover_file(book_id, url, &cfg, provider).await {
            cover_cache = Some(name);
        }
    }
    let page = detail.infer_page_url();
    store::write_bound(
        book_id,
        &detail,
        &match_kind,
        cover_cache.as_deref(),
        page.as_deref(),
    )?;
    let opts = options.unwrap_or(BindOptions {
        import_tags: false,
        use_remote_cover: false,
    });
    if opts.import_tags {
        let _ = import_tags(book_id, &detail.tags);
    }
    if opts.use_remote_cover {
        if let Some(ref name) = cover_cache {
            let _ = apply_remote_cover_to_shelf(&app, book_id, name);
        }
    }
    store::get_bound(book_id)?.ok_or_else(|| "绑定失败".into())
}

#[tauri::command]
pub fn metadata_unbind(book_id: i64) -> Result<(), String> {
    store::unbind(book_id)
}

#[tauri::command]
pub async fn metadata_automatch(
    app: AppHandle,
    book_ids: Option<Vec<i64>>,
    dry_run: Option<bool>,
) -> Result<Vec<AutoMatchItem>, String> {
    let cfg = config::load();
    let ids = match book_ids {
        Some(v) if !v.is_empty() => v,
        _ => store::list_unbound_book_ids()?,
    };
    let dry = dry_run.unwrap_or(false);
    let mut out = Vec::new();
    for book_id in ids {
        let title = match store::book_title(book_id) {
            Ok(t) => t,
            Err(e) => {
                out.push(AutoMatchItem {
                    book_id,
                    status: "error".into(),
                    provider: None,
                    match_kind: None,
                    message: Some(e),
                });
                continue;
            }
        };
        let mut bound: Option<(Provider, String, Match)> = None;
        let mut pending_candidates: Vec<MetadataCandidate> = Vec::new();
        for provider in cfg.provider_order() {
            let search = search_provider(&cfg, provider, &title, 8).await;
            if search.candidates.is_empty() {
                continue;
            }
            pending_candidates = search.candidates.clone();
            if let Some((cand, m)) = matching::best_for_query(&title, &search.candidates) {
                if m.is_confident() {
                    bound = Some((cand.provider, cand.external_id.clone(), m));
                    break;
                }
                if bound.is_none() && m == Match::Weak {
                    bound = Some((cand.provider, cand.external_id.clone(), m));
                }
            }
            if bound.as_ref().map(|(_, _, m)| m.is_confident()).unwrap_or(false) {
                break;
            }
        }
        let Some((provider, external_id, kind)) = bound else {
            out.push(AutoMatchItem {
                book_id,
                status: "none".into(),
                provider: None,
                match_kind: None,
                message: Some("未找到可信匹配".into()),
            });
            continue;
        };
        if kind.is_confident()
            && cfg.auto_bind_confident
            && provider != Provider::Comic18
            && !dry
        {
            match metadata_bind(
                app.clone(),
                book_id,
                provider.as_str().into(),
                external_id.clone(),
                Some(kind.as_str().into()),
                Some(BindOptions {
                    import_tags: false,
                    use_remote_cover: false,
                }),
            )
            .await
            {
                Ok(_) => {
                    out.push(AutoMatchItem {
                        book_id,
                        status: "bound".into(),
                        provider: Some(provider.as_str().into()),
                        match_kind: Some(kind.as_str().into()),
                        message: None,
                    });
                }
                Err(e) => {
                    out.push(AutoMatchItem {
                        book_id,
                        status: "error".into(),
                        provider: Some(provider.as_str().into()),
                        match_kind: Some(kind.as_str().into()),
                        message: Some(e),
                    });
                }
            }
        } else {
            let candidates_json =
                serde_json::to_string(&pending_candidates).unwrap_or_else(|_| "[]".into());
            if !dry {
                store::write_pending(&PendingMatch {
                    book_id,
                    provider,
                    external_id: external_id.clone(),
                    match_kind: kind.as_str().into(),
                    query: title,
                    candidates_json,
                    created_at: crate::appdata::now_secs(),
                })?;
            }
            out.push(AutoMatchItem {
                book_id,
                status: "pending".into(),
                provider: Some(provider.as_str().into()),
                match_kind: Some(kind.as_str().into()),
                message: None,
            });
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn metadata_refresh_cover(book_id: i64) -> Result<Option<String>, String> {
    let bound = store::get_bound(book_id)?.ok_or_else(|| "尚未绑定在线资料".to_string())?;
    let url = bound
        .cover_url
        .as_deref()
        .ok_or_else(|| "该条目没有远程封面".to_string())?;
    let cfg = config::load();
    let name = cache_cover_file(book_id, url, &cfg, bound.provider).await?;
    Ok(Some(name))
}

#[tauri::command]
pub fn metadata_cover_local_path(book_id: i64) -> Result<Option<String>, String> {
    let bound = match store::get_bound(book_id)? {
        Some(b) => b,
        None => return Ok(None),
    };
    let Some(name) = bound.cover_cache else {
        return Ok(None);
    };
    let path = metadata_covers_dir()?.join(name);
    if path.is_file() {
        Ok(Some(path.to_string_lossy().to_string()))
    } else {
        Ok(None)
    }
}

fn fallback_page_url(
    bound: Option<&BoundMetadata>,
    cfg: &MetadataNetworkConfig,
    book_title: &str,
) -> Option<String> {
    if let Some(b) = bound {
        if let Some(u) = b.source_page_url.as_ref().filter(|u| !u.trim().is_empty()) {
            return Some(u.clone());
        }
        match b.provider {
            Provider::Bangumi => {
                return Some(format!("https://bgm.tv/subject/{}", b.external_id));
            }
            Provider::Anilist => {
                return Some(format!("https://anilist.co/manga/{}", b.external_id));
            }
            Provider::Mangaupdates => {
                let q =
                    url::form_urlencoded::byte_serialize(book_title.as_bytes()).collect::<String>();
                return Some(format!(
                    "https://www.mangaupdates.com/series.html?search={q}"
                ));
            }
            Provider::Comic18 => {
                return Some(comic18::search_url_for_title(cfg, book_title));
            }
        }
    }
    Some(comic18::search_url_for_title(cfg, book_title))
}

#[tauri::command]
pub fn metadata_open_source_page(book_id: i64) -> Result<(), String> {
    let cfg = config::load();
    let title = store::book_title(book_id)?;
    let bound = store::get_bound(book_id)?;
    let url = fallback_page_url(bound.as_ref(), &cfg, &title)
        .filter(|u| !u.trim().is_empty())
        .ok_or_else(|| "没有可打开的链接".to_string())?;
    open_in_system_browser(&url)
}

#[tauri::command]
pub fn metadata_comic18_search_url(book_id: i64) -> Result<String, String> {
    let cfg = config::load();
    let title = store::book_title(book_id)?;
    Ok(comic18::search_url_for_title(&cfg, &title))
}
