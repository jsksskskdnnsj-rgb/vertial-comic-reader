//! Shared HTTP client builder (rustls, optional proxy, per-host rate limit).

use super::config::MetadataNetworkConfig;
use super::{Provider, USER_AGENT};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::LazyLock;
use std::time::{Duration, Instant};

static LAST_HIT: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn redact_proxy(url: &str) -> String {
    let Ok(u) = url::Url::parse(url) else {
        return "(invalid proxy URL)".into();
    };
    let host = u.host_str().unwrap_or("?");
    format!("{}://{}", u.scheme(), host)
}

pub(crate) fn build_client(cfg: &MetadataNetworkConfig, provider: Provider) -> Result<reqwest::Client, String> {
    build_client_for(cfg, provider, USER_AGENT)
}

pub(crate) fn build_client_for(
    cfg: &MetadataNetworkConfig,
    provider: Provider,
    user_agent: &str,
) -> Result<reqwest::Client, String> {
    let mut b = reqwest::Client::builder()
        .user_agent(user_agent)
        .redirect(reqwest::redirect::Policy::limited(3))
        .timeout(Duration::from_secs(25))
        .use_rustls_tls();
    if cfg.use_proxy_for(provider) {
        let url = cfg.proxy_url.trim();
        let proxy = reqwest::Proxy::all(url)
            .map_err(|_| format!("代理地址无效：{}", redact_proxy(url)))?;
        b = b.proxy(proxy);
    }
    b.build()
        .map_err(|_| "无法创建网络客户端".into())
}

pub(crate) async fn throttle(cfg: &MetadataNetworkConfig, host: &str) {
    throttle_min(cfg, host, 100).await;
}

pub(crate) async fn throttle_min(cfg: &MetadataNetworkConfig, host: &str, min_ms: u64) {
    let gap = Duration::from_millis(cfg.rate_limit_ms.max(min_ms).max(100));
    let sleep = {
        let mut map = LAST_HIT.lock();
        let now = Instant::now();
        let wait = map.get(host).and_then(|last| {
            let elapsed = now.saturating_duration_since(*last);
            (elapsed < gap).then(|| gap - elapsed)
        });
        map.insert(host.to_string(), now);
        wait
    };
    if let Some(d) = sleep {
        tokio::time::sleep(d).await;
    }
}

/// Rebuilds the request on each attempt. 429 and 5xx back off; connect errors do too.
pub(crate) async fn send_retry<F>(
    cfg: &MetadataNetworkConfig,
    host: &str,
    min_gap_ms: u64,
    mut build: F,
) -> Result<reqwest::Response, String>
where
    F: FnMut() -> Result<reqwest::RequestBuilder, String>,
{
    let mut wait_ms = 350u64;
    let mut last = "网络请求失败".to_string();
    for attempt in 0..3u32 {
        throttle_min(cfg, host, min_gap_ms).await;
        let built = match build() {
            Ok(req) => req,
            Err(e) => return Err(e),
        };
        match built.send().await {
            Ok(resp) => {
                let code = resp.status().as_u16();
                if code == 429 || (500..600).contains(&code) {
                    last = format!("HTTP {code}");
                    if attempt == 2 {
                        return Ok(resp);
                    }
                    tokio::time::sleep(Duration::from_millis(wait_ms)).await;
                    wait_ms = (wait_ms * 2).min(4000);
                    continue;
                }
                return Ok(resp);
            }
            Err(e) => {
                last = offline_message(&e);
                if attempt == 2 {
                    return Err(last);
                }
                tokio::time::sleep(Duration::from_millis(wait_ms)).await;
                wait_ms = (wait_ms * 2).min(4000);
            }
        }
    }
    Err(last)
}

pub(crate) fn offline_message(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        "请求超时，请检查网络或代理设置".into()
    } else if err.is_connect() {
        "无法连接服务器（离线或需配置代理）".into()
    } else {
        "网络请求失败".into()
    }
}
