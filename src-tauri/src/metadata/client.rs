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
    let gap = Duration::from_millis(cfg.rate_limit_ms.max(100));
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

pub(crate) fn offline_message(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        "请求超时，请检查网络或代理设置".into()
    } else if err.is_connect() {
        "无法连接服务器（离线或需配置代理）".into()
    } else {
        "网络请求失败".into()
    }
}
