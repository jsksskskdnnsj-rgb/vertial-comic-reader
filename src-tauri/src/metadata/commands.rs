//! Tauri commands for metadata lookup, binding, and batch auto-match.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

use super::anilist;
use super::bangumi;
use super::client::{build_client, offline_message};
use super::config::{self, MetadataNetworkConfig};
use super::comic18;
use super::custom;
use super::proxy;
use super::jikan;
use super::mangadex;
use super::mangaupdates;
use super::matching::{self, Match};
use super::moegirl;
use super::store::{self, BoundMetadata, ManualPatch, PendingMatch};
use super::{MetadataCandidate, MetadataDetail, Provider, USER_AGENT};
use crate::appdata::data_dir;
use crate::cover_protocol::CoversDir;

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub provider: Provider,
    pub error: Option<String>,
    pub candidates: Vec<MetadataCandidate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
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

pub(crate) fn require_shelf_book(book_id: i64) -> Result<(), String> {
    store::book_title(book_id).map(|_| ()).map_err(|_| "只能为书架中已有的书查询在线资料".into())
}

fn search_cache_key(provider: Provider, query: &str, limit: u32) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    query.hash(&mut hasher);
    format!("s:{}:{limit}:{:x}", provider.as_str(), hasher.finish())
}

fn detail_cache_key(provider: Provider, id: &str) -> String {
    format!("d:{}:{id}", provider.as_str())
}

fn annotate(query: &str, list: &mut [MetadataCandidate]) {
    for c in list.iter_mut() {
        let (kind, conf) = matching::score_parts(query, c);
        c.confidence = Some(conf);
        if kind != Match::None {
            c.match_kind = Some(kind.as_str().into());
        }
    }
}

async fn search_provider_net(
    cfg: &MetadataNetworkConfig,
    provider: Provider,
    query: &str,
    limit: u32,
) -> SearchResult {
    if provider == Provider::Manual {
        return SearchResult {
            provider,
            error: Some("手改不是检索源".into()),
            candidates: Vec::new(),
            label: None,
        };
    }
    let candidates = match provider {
        Provider::Bangumi => bangumi::search(cfg, query, limit).await,
        Provider::Mangaupdates => mangaupdates::search(cfg, query, limit).await,
        Provider::Anilist => anilist::search(cfg, query, limit).await,
        Provider::Comic18 => comic18::search(cfg, query, limit).await,
        Provider::Mangadex => mangadex::search(cfg, query, limit).await,
        Provider::Jikan => jikan::search(cfg, query, limit).await,
        Provider::Moegirl => moegirl::search(cfg, query, limit).await,
        Provider::Manual | Provider::Custom => Ok(Vec::new()),
    };
    match candidates {
        Ok(mut list) => {
            annotate(query, &mut list);
            SearchResult {
                provider,
                error: None,
                candidates: list,
                label: None,
            }
        }
        Err(e) => SearchResult {
            provider,
            error: Some(e),
            candidates: Vec::new(),
            label: None,
        },
    }
}

async fn search_provider(
    cfg: &MetadataNetworkConfig,
    provider: Provider,
    query: &str,
    limit: u32,
) -> SearchResult {
    let key = search_cache_key(provider, query, limit);
    if let Some(body) = store::cache_fresh(&key) {
        if let Ok(hit) = serde_json::from_str::<SearchResult>(&body) {
            return hit;
        }
    }
    let hit = search_provider_net(cfg, provider, query, limit).await;
    if hit.error.is_none() {
        if let Ok(body) = serde_json::to_string(&hit) {
            store::cache_put(&key, &body, 6 * 3600);
        }
        return hit;
    }
    if let Some(body) = store::cache_stale(&key) {
        if let Ok(mut cached) = serde_json::from_str::<SearchResult>(&body) {
            let note = hit.error.clone().unwrap_or_default();
            cached.error = Some(format!("离线缓存：{note}"));
            return cached;
        }
    }
    hit
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

async fn fetch_detail_net(cfg: &MetadataNetworkConfig, provider: Provider, id: &str) -> Result<MetadataDetail, String> {
    match provider {
        Provider::Bangumi => bangumi::detail(cfg, id).await,
        Provider::Mangaupdates => mangaupdates::detail(cfg, id).await,
        Provider::Anilist => anilist::detail(cfg, id).await,
        Provider::Comic18 => comic18::detail(cfg, id).await,
        Provider::Mangadex => mangadex::detail(cfg, id).await,
        Provider::Jikan => jikan::detail(cfg, id).await,
        Provider::Moegirl => moegirl::detail(cfg, id).await,
        Provider::Manual => Err("手改条目没有远程详情".into()),
        Provider::Custom => custom::detail(cfg, id).await,
    }
}

pub(crate) async fn fetch_detail(cfg: &MetadataNetworkConfig, provider: Provider, id: &str) -> Result<MetadataDetail, String> {
    let key = detail_cache_key(provider, id);
    if let Some(body) = store::cache_fresh(&key) {
        if let Ok(detail) = serde_json::from_str::<MetadataDetail>(&body) {
            return Ok(detail);
        }
    }
    match fetch_detail_net(cfg, provider, id).await {
        Ok(detail) => {
            if let Ok(body) = serde_json::to_string(&detail) {
                store::cache_put(&key, &body, 7 * 24 * 3600);
            }
            Ok(detail)
        }
        Err(err) => {
            if let Some(body) = store::cache_stale(&key) {
                if let Ok(detail) = serde_json::from_str::<MetadataDetail>(&body) {
                    return Ok(detail);
                }
            }
            Err(err)
        }
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
    let img = image::load_from_memory(&bytes).map_err(|_| "封面无法解码".to_string())?;
    let thumb = img.thumbnail(480, 480);
    let name = format!("book-{book_id}.jpg");
    let path = metadata_covers_dir()?.join(&name);
    thumb
        .save_with_format(&path, image::ImageFormat::Jpeg)
        .map_err(|e| e.to_string())?;
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
    config::save(&cfg)?;
    proxy::sync_lifecycle(&cfg);
    Ok(())
}

#[tauri::command]
pub async fn metadata_search(
    book_id: i64,
    query: String,
    limit: Option<u32>,
    providers: Option<Vec<String>>,
    fallback_chain: Option<bool>,
) -> Result<Vec<SearchResult>, String> {
    require_shelf_book(book_id)?;
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
            label: None,
        }]);
    }
    let list: Vec<Provider> = providers
        .unwrap_or_default()
        .iter()
        .filter_map(|s| Provider::parse(s))
        .collect();
    let include_custom = list.is_empty() || list.iter().any(|p| *p == Provider::Custom);
    let order = if list.is_empty() {
        cfg.provider_order()
    } else {
        list
    };
    let mut out = Vec::new();
    for p in order {
        if !cfg.enabled(p) {
            continue;
        }
        out.push(search_provider(&cfg, p, query, limit).await);
    }
    if include_custom {
        for hit in custom::search_enabled(&cfg, query, "").await {
            let mut candidates = hit.candidates;
            if hit.error.is_none() {
                annotate(query, &mut candidates);
            }
            out.push(SearchResult {
                provider: Provider::Custom,
                error: hit.error,
                candidates,
                label: Some(hit.label),
            });
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn metadata_detail(book_id: i64, provider: String, external_id: String) -> Result<MetadataDetail, String> {
    require_shelf_book(book_id)?;
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
        if let Ok(Some(bound)) = store::get_bound(book_id) {
            if bound.locked || bound.provider == Provider::Manual {
                out.push(AutoMatchItem {
                    book_id,
                    status: "locked".into(),
                    provider: Some(bound.provider.as_str().into()),
                    match_kind: Some(bound.match_kind),
                    message: Some("已锁定，跳过自动匹配".into()),
                });
                continue;
            }
        }
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
            Provider::Mangadex => {
                return Some(format!("https://mangadex.org/title/{}", b.external_id));
            }
            Provider::Jikan => {
                return Some(format!("https://myanimelist.net/manga/{}", b.external_id));
            }
            Provider::Moegirl => {
                return Some(format!("https://zh.moegirl.org.cn/?curid={}", b.external_id));
            }
            Provider::Manual => {}
            Provider::Custom => return None,
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshOutcome {
    pub refreshed: bool,
    pub message: Option<String>,
}

#[tauri::command]
pub fn metadata_save_manual(book_id: i64, patch: ManualPatch) -> Result<BoundMetadata, String> {
    require_shelf_book(book_id)?;
    store::save_manual(book_id, &patch)?;
    store::get_bound(book_id)?.ok_or_else(|| "保存失败".into())
}

#[tauri::command]
pub async fn metadata_refresh(book_id: i64) -> Result<RefreshOutcome, String> {
    require_shelf_book(book_id)?;
    let bound = store::get_bound(book_id)?.ok_or_else(|| "尚未绑定在线资料".to_string())?;
    if bound.locked || bound.provider == Provider::Manual {
        return Ok(RefreshOutcome {
            refreshed: false,
            message: Some("资料已锁定，未访问网络".into()),
        });
    }
    let cfg = config::load();
    let detail = fetch_detail(&cfg, bound.provider, &bound.external_id).await?;
    let mut cover_cache = bound.cover_cache.clone();
    if let Some(url) = detail.cover_url.as_deref() {
        if let Ok(name) = cache_cover_file(book_id, url, &cfg, bound.provider).await {
            cover_cache = Some(name);
        }
    }
    let page = detail.infer_page_url();
    store::write_bound(
        book_id,
        &detail,
        bound.match_kind.as_str(),
        cover_cache.as_deref(),
        page.as_deref(),
    )?;
    Ok(RefreshOutcome {
        refreshed: true,
        message: None,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaCacheRow {
    pub provider: String,
    /// fresh | stale | miss | off
    pub search: String,
    /// fresh | stale | miss | off | na
    pub detail: String,
}

fn cache_label(stamp: Option<store::CacheStamp>, now: i64) -> String {
    match stamp {
        Some(s) if s.expires_at >= now => "fresh".into(),
        Some(s) if s.fetched_at >= now.saturating_sub(30 * 24 * 3600) => "stale".into(),
        _ => "miss".into(),
    }
}

#[tauri::command]
pub fn metadata_shelf_index() -> Result<Vec<store::MetaShelfRow>, String> {
    store::list_shelf_meta()
}

/// Cache rows for one shelf book. `query` defaults to the shelf title (limit 12, same as search).
#[tauri::command]
pub fn metadata_cache_status(book_id: i64, query: Option<String>) -> Result<Vec<MetaCacheRow>, String> {
    require_shelf_book(book_id)?;
    let title = store::book_title(book_id)?;
    let query = query.unwrap_or_default();
    let query = if query.trim().is_empty() { title } else { query.trim().to_string() };
    let bound = store::get_bound(book_id)?;
    let cfg = config::load();
    let now = crate::now_secs();
    let providers = [
        Provider::Bangumi,
        Provider::Moegirl,
        Provider::Mangadex,
        Provider::Jikan,
        Provider::Mangaupdates,
        Provider::Anilist,
        Provider::Comic18,
    ];
    let mut out = Vec::new();
    for provider in providers {
        if !cfg.enabled(provider) {
            out.push(MetaCacheRow {
                provider: provider.as_str().into(),
                search: "off".into(),
                detail: "off".into(),
            });
            continue;
        }
        let search = cache_label(store::cache_stamp(&search_cache_key(provider, &query, 12)), now);
        let detail = match bound.as_ref() {
            Some(b) if b.provider == provider => {
                cache_label(store::cache_stamp(&detail_cache_key(provider, &b.external_id)), now)
            }
            _ => "na".into(),
        };
        out.push(MetaCacheRow {
            provider: provider.as_str().into(),
            search,
            detail,
        });
    }
    Ok(out)
}
