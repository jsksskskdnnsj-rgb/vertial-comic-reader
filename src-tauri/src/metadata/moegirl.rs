//! 萌娘百科 MediaWiki API. Intro extract + 240px thumbnail only.

use super::client::send_retry;
use super::config::MetadataNetworkConfig;
use super::{MetadataCandidate, MetadataDetail, Provider, WorkFacts};

const HOST: &str = "zh.moegirl.org.cn";
const API: &str = "https://zh.moegirl.org.cn/api.php";

pub(crate) fn search_url(query: &str, limit: u32) -> String {
    let q = url::form_urlencoded::byte_serialize(query.trim().as_bytes()).collect::<String>();
    format!(
        "{API}?action=query&list=search&srsearch={q}&srnamespace=0&srlimit={}&format=json&utf8=1",
        limit.clamp(1, 10)
    )
}

pub(crate) fn detail_url(page_id: &str) -> String {
    format!(
        "{API}?action=query&pageids={}&prop=extracts|pageimages|info&exintro=1&explaintext=1&pithumbsize=240&inprop=url&format=json&utf8=1",
        page_id.trim()
    )
}

fn clip(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() > 800 {
        t.chars().take(800).collect::<String>() + "…"
    } else {
        t.to_string()
    }
}

pub async fn search(
    cfg: &MetadataNetworkConfig,
    query: &str,
    limit: u32,
) -> Result<Vec<MetadataCandidate>, String> {
    let root = super::endpoints::api_root(&cfg.bases.moegirl_api, "https://zh.moegirl.org.cn");
    let url = search_url(query, limit).replacen(API, &format!("{root}/api.php"), 1);
    let host = super::endpoints::host_of(&url, HOST);
    let response = send_retry(cfg, &host, 800, || {
        let client = super::client::build_client(cfg, Provider::Moegirl)?;
        Ok(client.get(&url).header("Accept", "application/json"))
    })
    .await?;
    if !response.status().is_success() {
        return Err(format!("萌娘百科请求失败（HTTP {}）", response.status()));
    }
    let body: serde_json::Value = response.json().await.map_err(|_| "萌娘百科搜索结果无法解析".to_string())?;
    let hits = body
        .get("query")
        .and_then(|q| q.get("search"))
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for hit in hits {
        let id = hit.get("pageid").and_then(|v| v.as_i64()).unwrap_or(0);
        if id <= 0 {
            continue;
        }
        let title = hit.get("title").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        if title.is_empty() {
            continue;
        }
        out.push(MetadataCandidate {
            provider: Provider::Moegirl,
            external_id: id.to_string(),
            title,
            original_title: None,
            aliases: Vec::new(),
            release_date: None,
            cover_url: None,
            rating: None,
            source_label: "萌娘百科".into(),
            match_kind: None,
            page_url: Some(format!("https://zh.moegirl.org.cn/?curid={id}")),
            confidence: None,
        });
    }
    Ok(out)
}

pub async fn detail(cfg: &MetadataNetworkConfig, id: &str) -> Result<MetadataDetail, String> {
    let root = super::endpoints::api_root(&cfg.bases.moegirl_api, "https://zh.moegirl.org.cn");
    let url = detail_url(id).replacen(API, &format!("{root}/api.php"), 1);
    let host = super::endpoints::host_of(&url, HOST);
    let response = send_retry(cfg, &host, 800, || {
        let client = super::client::build_client(cfg, Provider::Moegirl)?;
        Ok(client.get(&url).header("Accept", "application/json"))
    })
    .await?;
    if !response.status().is_success() {
        return Err(format!("萌娘百科详情失败（HTTP {}）", response.status()));
    }
    let body: serde_json::Value = response.json().await.map_err(|_| "萌娘百科详情无法解析".to_string())?;
    let page = body
        .pointer(&format!("/query/pages/{}", id.trim()))
        .cloned()
        .or_else(|| {
            body.get("query")
                .and_then(|q| q.get("pages"))
                .and_then(|p| p.as_object())
                .and_then(|o| o.values().next().cloned())
        })
        .ok_or_else(|| "萌娘百科未找到该页".to_string())?;
    if page.get("missing").is_some() {
        return Err("萌娘百科未找到该页".into());
    }
    let title = page.get("title").and_then(|v| v.as_str()).unwrap_or(id).trim().to_string();
    let summary = page.get("extract").and_then(|v| v.as_str()).map(clip).filter(|s| !s.is_empty());
    let cover = page
        .pointer("/thumbnail/source")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());
    let page_url = page
        .get("fullurl")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| Some(format!("https://zh.moegirl.org.cn/?curid={}", id.trim())));
    Ok(MetadataDetail {
        provider: Provider::Moegirl,
        external_id: id.trim().to_string(),
        title,
        title_cn: None,
        title_original: None,
        summary,
        authors: Vec::new(),
        tags: Vec::new(),
        rating: None,
        rating_count: None,
        release_date: None,
        cover_url: cover,
        aliases: Vec::new(),
        page_url,
        facts: WorkFacts::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_ask_for_a_thumbnail_extract() {
        let s = search_url("进击的巨人", 4);
        assert!(s.contains("action=query"));
        assert!(s.contains("list=search"));
        assert!(!s.contains("images"));
        let d = detail_url("12");
        assert!(d.contains("pithumbsize=240"));
        assert!(d.contains("exintro=1"));
        assert!(!d.contains("imlimit"));
    }
}
