//! In-app proxy. Nothing here changes the system proxy, installs a service, or enables TUN.
//! The bundled mihomo process, when used, listens on 127.0.0.1 only and is stopped when the
//! switch is off or the process exits.

use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;

use super::config::MetadataNetworkConfig;

static CORE: LazyLock<Mutex<Option<Running>>> = LazyLock::new(|| Mutex::new(None));

struct Running {
    child: Child,
    mixed: u16,
    controller: u16,
    secret: String,
    node: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Off,
    System,
    Manual,
    Local,
    Core,
}

pub(crate) struct Outbound {
    pub url: String,
    pub user: Option<String>,
    pub pass: Option<String>,
}

pub(crate) fn mode_of(cfg: &MetadataNetworkConfig) -> Mode {
    if !cfg.app_proxy {
        return Mode::Off;
    }
    match cfg.proxy_mode.trim() {
        "system" => Mode::System,
        "manual" => Mode::Manual,
        "local" => Mode::Local,
        "core" => Mode::Core,
        _ => {
            if cfg.proxy_url.trim().is_empty() {
                Mode::Off
            } else {
                Mode::Manual
            }
        }
    }
}

pub(crate) fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Off => "off",
        Mode::System => "system",
        Mode::Manual => "manual",
        Mode::Local => "local",
        Mode::Core => "core",
    }
}

pub(crate) fn outbound(cfg: &MetadataNetworkConfig) -> Result<Outbound, String> {
    if !cfg.app_proxy {
        return Err("应用内代理没有开启".into());
    }
    match mode_of(cfg) {
        Mode::Off => Err("请选择代理方式".into()),
        Mode::System => system_outbound(),
        Mode::Manual => manual_outbound(cfg),
        Mode::Local => manual_parts(
            if cfg.local_proxy.trim().is_empty() {
                "http://127.0.0.1:7897"
            } else {
                cfg.local_proxy.trim()
            },
            "",
            "",
        ),
        Mode::Core => core_outbound(cfg),
    }
}

pub(crate) fn sync_lifecycle(cfg: &MetadataNetworkConfig) {
    if !cfg.app_proxy || mode_of(cfg) != Mode::Core {
        shutdown_core();
    }
}

pub fn shutdown_core() {
    if let Some(run) = CORE.lock().take() {
        drop_run(run);
    }
}

fn drop_run(mut run: Running) {
    let _ = run.child.kill();
    let _ = run.child.wait();
}

pub(crate) fn core_bundled() -> bool {
    core_exe().is_some()
}

fn core_exe() -> Option<PathBuf> {
    let names = ["mihomo.exe", "mihomo-x86_64-pc-windows-msvc.exe"];
    let mut dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.to_path_buf());
            dirs.push(parent.join("resources"));
        }
    }
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries"));
    for dir in dirs {
        for name in names {
            let path = dir.join(name);
            if let Ok(meta) = std::fs::metadata(&path) {
                if meta.is_file() && meta.len() > 1_000_000 {
                    return Some(path);
                }
            }
        }
    }
    None
}

fn core_outbound(cfg: &MetadataNetworkConfig) -> Result<Outbound, String> {
    let name = cfg.core_node.trim();
    if name.is_empty() {
        return Err("请先导入配置并选择节点".into());
    }
    let text = super::clash::read_import()?;
    let node = super::clash::node_by_name(&text, name)?;
    let mut guard = CORE.lock();
    if let Some(run) = guard.as_ref() {
        if run.node == name && tcp_open(run.mixed) {
            return Ok(Outbound {
                url: format!("http://127.0.0.1:{}", run.mixed),
                user: None,
                pass: None,
            });
        }
    }
    if let Some(run) = guard.take() {
        drop_run(run);
    }
    let run = spawn_core(&node, name)?;
    let url = format!("http://127.0.0.1:{}", run.mixed);
    *guard = Some(run);
    Ok(Outbound { url, user: None, pass: None })
}

fn spawn_core(node: &serde_yaml::Value, node_name: &str) -> Result<Running, String> {
    let exe = core_exe().ok_or_else(|| {
        "没有找到随包的 mihomo.exe。Windows 安装包会按 scripts/mihomo-pin.json 的版本和校验值把核心打进去。".to_string()
    })?;
    let mixed = free_port()?;
    let controller = free_port()?;
    let secret = random_hex(32);
    let yaml = super::clash::minimal_config(node, mixed, controller, &secret)?;
    let dir = crate::data_dir()?.join("cache").join("mihomo");
    std::fs::create_dir_all(&dir).map_err(|_| "无法创建核心工作目录".to_string())?;
    let cfg_path = dir.join("runtime.yaml");
    write_private(&cfg_path, yaml.as_bytes())?;
    let mut cmd = Command::new(&exe);
    cmd.arg("-d").arg(&dir).arg("-f").arg(&cfg_path);
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let child = cmd.spawn().map_err(|_| "无法启动代理核心".to_string())?;
    if !wait_port(controller, 4000) {
        let mut child = child;
        let _ = child.kill();
        let _ = child.wait();
        return Err("代理核心没有在 127.0.0.1 上就绪".into());
    }
    Ok(Running {
        child,
        mixed,
        controller,
        secret,
        node: node_name.to_string(),
    })
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(|_| "无法写入核心配置".to_string())?;
        file.write_all(bytes).map_err(|_| "无法写入核心配置".to_string())?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, bytes).map_err(|_| "无法写入核心配置".to_string())?;
    }
    Ok(())
}

fn free_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|_| "没有可用的本地端口".to_string())?;
    let port = listener.local_addr().map_err(|_| "没有可用的本地端口".to_string())?.port();
    drop(listener);
    Ok(port)
}

fn wait_port(port: u16, ms: u64) -> bool {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(ms) {
        if tcp_open(port) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    false
}

fn tcp_open(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(150)).is_ok()
}

fn random_hex(n: usize) -> String {
    let mut state = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0x9E37);
    state ^= std::process::id() as u128;
    let mut out = String::new();
    while out.len() < n {
        state = state.wrapping_mul(0x6C62_272E_07BB_0142).wrapping_add(0x9E37_79B9_7F4A_7C15);
        out.push_str(&format!("{state:032x}"));
    }
    out.truncate(n);
    out
}

fn system_outbound() -> Result<Outbound, String> {
    if let Some(url) = env_proxy() {
        return manual_parts(&url, "", "");
    }
    #[cfg(windows)]
    {
        return windows_proxy();
    }
    #[cfg(not(windows))]
    {
        Err("没有读到系统代理。本程序不会修改系统代理。".into())
    }
}

fn env_proxy() -> Option<String> {
    for key in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] {
        if let Ok(value) = std::env::var(key) {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

#[cfg(windows)]
fn windows_proxy() -> Result<Outbound, String> {
    let enabled = reg_value("ProxyEnable").unwrap_or_default();
    let on = enabled.contains("0x1") || enabled.trim().ends_with("1");
    if !on {
        return Err("系统代理没有开启。本程序不会替你打开它。".into());
    }
    let server = reg_value("ProxyServer").unwrap_or_default();
    let server = server.trim();
    if server.is_empty() {
        return Err("没有读到系统代理地址。".into());
    }
    let picked = if let Some(rest) = server.split(';').find_map(|part| part.trim().strip_prefix("https=")) {
        rest
    } else if let Some(rest) = server.split(';').find_map(|part| part.trim().strip_prefix("http=")) {
        rest
    } else {
        server.split(';').next().unwrap_or(server)
    };
    manual_parts(picked, "", "")
}

#[cfg(windows)]
fn reg_value(name: &str) -> Option<String> {
    let out = Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
            "/v",
            name,
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().rev().find_map(|line| {
        let mut bits = line.split_whitespace();
        let key = bits.next()?;
        if key != name {
            return None;
        }
        bits.next()?;
        Some(bits.collect::<Vec<_>>().join(" "))
    })
}

fn manual_outbound(cfg: &MetadataNetworkConfig) -> Result<Outbound, String> {
    let url = cfg.proxy_url.trim();
    if url.is_empty() {
        return Err("请填写 HTTP 或 SOCKS5 地址".into());
    }
    manual_parts(url, cfg.proxy_username.trim(), cfg.proxy_password.trim())
}

fn manual_parts(raw: &str, user: &str, pass: &str) -> Result<Outbound, String> {
    let mut text = raw.trim().to_string();
    if !text.contains("://") {
        text = format!("http://{text}");
    }
    let mut parsed = url::Url::parse(&text).map_err(|_| "代理地址无效".to_string())?;
    match parsed.scheme() {
        "http" | "https" | "socks5" | "socks5h" => {}
        _ => return Err("只支持 http 或 socks5 代理".into()),
    }
    let url_user = percent_decode(parsed.username());
    let url_pass = parsed.password().map(percent_decode);
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    let user = if !user.is_empty() {
        Some(user.to_string())
    } else if !url_user.is_empty() {
        Some(url_user)
    } else {
        None
    };
    let pass = if !pass.is_empty() {
        Some(pass.to_string())
    } else {
        url_pass
    };
    Ok(Outbound {
        url: parsed.to_string().trim_end_matches('/').to_string(),
        user,
        pass,
    })
}

fn percent_decode(s: &str) -> String {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub async fn direct_get(cfg: &MetadataNetworkConfig, url: &str) -> Result<String, String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("只允许 http 或 https".into());
    }
    let client = app_client(cfg)?;
    let response = client
        .get(url)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| "订阅下载失败".to_string())?;
    if !response.status().is_success() {
        return Err("订阅下载失败".into());
    }
    let bytes = limited_bytes(response, 2 * 1024 * 1024).await?;
    String::from_utf8(bytes).map_err(|_| "订阅不是文本".to_string())
}

pub(crate) fn app_client(cfg: &MetadataNetworkConfig) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .user_agent(super::USER_AGENT)
        .redirect(reqwest::redirect::Policy::limited(3))
        .timeout(Duration::from_secs(25))
        .use_rustls_tls();
    if cfg.app_proxy {
        let out = outbound(cfg)?;
        let mut proxy = reqwest::Proxy::all(&out.url).map_err(|_| "代理地址无效".to_string())?;
        if let Some(user) = out.user.as_deref() {
            proxy = proxy.basic_auth(user, out.pass.as_deref().unwrap_or(""));
        }
        builder = builder.proxy(proxy);
    }
    builder.build().map_err(|_| "无法创建网络客户端".to_string())
}

async fn limited_bytes(mut response: reqwest::Response, max: usize) -> Result<Vec<u8>, String> {
    if response.content_length().unwrap_or(0) > max as u64 {
        return Err("响应过大".into());
    }
    let mut buf = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "读取失败".to_string())? {
        if buf.len() + chunk.len() > max {
            return Err("响应过大".into());
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyStatus {
    pub enabled: bool,
    pub mode: String,
    pub running: bool,
    pub node: String,
    pub listen: String,
    pub core_bundled: bool,
    pub message: String,
}

#[tauri::command]
pub fn metadata_proxy_status() -> Result<ProxyStatus, String> {
    let cfg = super::config::load();
    let mode = mode_of(&cfg);
    let guard = CORE.lock();
    let running = guard.as_ref().map(|r| tcp_open(r.mixed)).unwrap_or(false);
    let node = if mode == Mode::Core {
        cfg.core_node.clone()
    } else if mode == Mode::Local {
        cfg.local_node.clone()
    } else {
        String::new()
    };
    let listen = guard
        .as_ref()
        .filter(|_| running)
        .map(|r| format!("127.0.0.1:{}", r.mixed))
        .unwrap_or_default();
    let message = if !cfg.app_proxy {
        "应用内代理关闭，本程序的联网请求直连。系统和其它程序不受影响。".into()
    } else if mode == Mode::Core && !core_bundled() {
        "已开启，但没有找到随包的 mihomo.exe。".into()
    } else if running {
        format!("核心在 {listen} 监听，只给本程序用。")
    } else if cfg.app_proxy {
        "应用内代理已开启。下一次联网请求会走当前方式。".into()
    } else {
        String::new()
    };
    Ok(ProxyStatus {
        enabled: cfg.app_proxy,
        mode: mode_name(mode).into(),
        running,
        node,
        listen,
        core_bundled: core_bundled(),
        message,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DelayResult {
    pub name: String,
    pub delay_ms: Option<u64>,
    pub message: String,
}

#[tauri::command]
pub async fn metadata_proxy_delay(name: String) -> Result<DelayResult, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("请选择节点".into());
    }
    let text = super::clash::read_import()?;
    let node = super::clash::node_by_name(&text, &name)?;
    let reuse = {
        let guard = CORE.lock();
        guard.as_ref().and_then(|run| {
            if run.node == name && tcp_open(run.controller) {
                Some((run.controller, run.secret.clone()))
            } else {
                None
            }
        })
    };
    let (port, secret, owned) = if let Some(pair) = reuse {
        (pair.0, pair.1, None)
    } else {
        let run = spawn_core(&node, &name)?;
        let port = run.controller;
        let secret = run.secret.clone();
        (port, secret, Some(run))
    };
    let result = query_delay(port, &secret, &name).await;
    if let Some(run) = owned {
        drop_run(run);
    }
    let (delay_ms, message) = result?;
    Ok(DelayResult { name, delay_ms, message })
}

async fn query_delay(port: u16, secret: &str, name: &str) -> Result<(Option<u64>, String), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .no_proxy()
        .build()
        .map_err(|_| "无法连接核心".to_string())?;
    let url = format!(
        "http://127.0.0.1:{port}/proxies/{}/delay?timeout=5000&url=https://www.gstatic.com/generate_204",
        path_encode(name)
    );
    let response = client
        .get(url)
        .header("Authorization", format!("Bearer {secret}"))
        .send()
        .await
        .map_err(|_| "延迟测试没有连上核心".to_string())?;
    if !response.status().is_success() {
        return Ok((None, format!("延迟测试失败（HTTP {}）", response.status())));
    }
    let body: serde_json::Value = response.json().await.map_err(|_| "核心没有返回延迟".to_string())?;
    let delay = body.get("delay").and_then(|v| v.as_u64());
    let message = match delay {
        Some(ms) => format!("{ms} ms"),
        None => "没有测到延迟".into(),
    };
    Ok((delay, message))
}

fn path_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyTest {
    pub ok: bool,
    pub status: u16,
    pub ms: u64,
    pub message: String,
}

#[tauri::command]
pub async fn metadata_proxy_test() -> Result<ProxyTest, String> {
    let cfg = super::config::load();
    if !cfg.app_proxy {
        return Ok(ProxyTest {
            ok: false,
            status: 0,
            ms: 0,
            message: "应用内代理是关闭的，请求会直连。".into(),
        });
    }
    let started = Instant::now();
    let client = app_client(&cfg)?;
    let response = client
        .get("https://www.gstatic.com/generate_204")
        .send()
        .await
        .map_err(|_| "连接测试失败".to_string())?;
    let ms = started.elapsed().as_millis() as u64;
    let status = response.status().as_u16();
    let ok = response.status().is_success() || status == 204;
    Ok(ProxyTest {
        ok,
        status,
        ms,
        message: if ok {
            format!("已连通，HTTP {status}，{ms} ms")
        } else {
            format!("代理有响应，但 HTTP {status}")
        },
    })
}

#[tauri::command]
pub fn metadata_proxy_stop() -> Result<(), String> {
    shutdown_core();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_proxy_strips_userinfo_from_the_url() {
        let out = manual_parts("http://user:s3cret@127.0.0.1:7890", "", "").unwrap();
        assert!(!out.url.contains("s3cret"));
        assert!(!out.url.contains("user"));
        assert_eq!(out.user.as_deref(), Some("user"));
        assert_eq!(out.pass.as_deref(), Some("s3cret"));
        assert!(out.url.starts_with("http://127.0.0.1:7890"));
    }

    #[test]
    fn bare_host_becomes_http_and_file_urls_are_rejected() {
        let out = manual_parts("127.0.0.1:7897", "a", "b").unwrap();
        assert_eq!(out.url, "http://127.0.0.1:7897");
        assert!(manual_parts("file:///tmp/x", "", "").is_err());
    }
}
