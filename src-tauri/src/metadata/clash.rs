//! Clash / mihomo YAML import. The raw document stays in the data directory.
//! Previews expose the node name, type, and server host only.

use std::path::PathBuf;

use serde::Serialize;
use serde_yaml::Value;

use super::proxy::direct_get;

const MAX_BODY: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodePreview {
    pub name: String,
    pub kind: String,
    pub server: String,
    pub encrypted: bool,
}

pub(crate) fn import_path() -> Result<PathBuf, String> {
    let dir = crate::data_dir()?.join("cache");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("clash-import.yaml"))
}

pub(crate) fn read_import() -> Result<String, String> {
    let path = import_path()?;
    std::fs::read_to_string(&path).map_err(|_| "还没有导入订阅".to_string())
}

fn write_import(body: &str) -> Result<(), String> {
    if body.len() > MAX_BODY {
        return Err("订阅内容过大".into());
    }
    let path = import_path()?;
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| e.to_string())?;
        f.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(&path, body).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn host_only(server: &str) -> String {
    let s = server.trim();
    if s.is_empty() {
        return String::new();
    }
    if let Ok(u) = url::Url::parse(s) {
        return u.host_str().unwrap_or("").to_string();
    }
    let bare = s.split('/').next().unwrap_or(s);
    bare.split('@').next_back().unwrap_or(bare).split(':').next().unwrap_or(bare).to_string()
}

fn encrypted(kind: &str) -> bool {
    !matches!(kind, "http" | "https" | "socks" | "socks5" | "socks5h")
}

pub(crate) fn previews_from(text: &str) -> Result<Vec<NodePreview>, String> {
    let value: Value = serde_yaml::from_str(text).map_err(|_| "订阅格式无法解析".to_string())?;
    let list = value
        .get("proxies")
        .and_then(Value::as_sequence)
        .ok_or_else(|| "这份配置里没有内联的 proxies 节点".to_string())?;
    let mut out = Vec::new();
    for item in list {
        let Some(name) = item.get("name").and_then(Value::as_str) else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || name.contains('\n') || name.contains('\r') {
            continue;
        }
        let kind = item.get("type").and_then(Value::as_str).unwrap_or("").trim().to_string();
        let server = item
            .get("server")
            .and_then(Value::as_str)
            .map(host_only)
            .unwrap_or_default();
        out.push(NodePreview {
            name: name.to_string(),
            kind: kind.clone(),
            server,
            encrypted: encrypted(&kind),
        });
    }
    if out.is_empty() {
        return Err("没有解析到节点".into());
    }
    Ok(out)
}

pub(crate) fn node_by_name(text: &str, name: &str) -> Result<Value, String> {
    let value: Value = serde_yaml::from_str(text).map_err(|_| "订阅格式无法解析".to_string())?;
    let list = value
        .get("proxies")
        .and_then(Value::as_sequence)
        .ok_or_else(|| "这份配置里没有内联的 proxies 节点".to_string())?;
    list.iter()
        .find(|item| item.get("name").and_then(Value::as_str) == Some(name))
        .cloned()
        .ok_or_else(|| "没有这个节点".to_string())
}

pub(crate) fn minimal_config(node: &Value, mixed: u16, controller: u16, secret: &str) -> Result<String, String> {
    let name = node.get("name").and_then(Value::as_str).unwrap_or("").trim();
    if name.is_empty() {
        return Err("节点没有名称".into());
    }
    let group = if name == "mh-reader" { "mh-reader-out" } else { "mh-reader" };
    let seq = Value::Sequence(vec![node.clone()]);
    let dumped = serde_yaml::to_string(&seq).map_err(|_| "节点无法写入配置".to_string())?;
    let dumped = dumped.trim_start_matches("---").trim_start_matches('\n');
    if dumped.contains("tun:") {
        return Err("节点配置里带有 tun，已拒绝".into());
    }
    let mut body = format!(
        "mixed-port: {mixed}\nallow-lan: false\nbind-address: 127.0.0.1\nmode: rule\nlog-level: silent\nipv6: false\nfind-process-mode: off\ngeo-auto-update: false\nunified-delay: true\nexternal-controller: 127.0.0.1:{controller}\nsecret: \"{secret}\"\nproxies:\n{dumped}proxy-groups:\n  - name: {group}\n    type: select\n    proxies:\n      - \"{name}\"\nrules:\n  - MATCH,{group}\n"
    );
    let _ = &mut body;
    if body.contains("\ntun:") || body.contains("tun:\n") {
        return Err("生成的配置包含 tun".into());
    }
    Ok(body)
}

pub(crate) fn redact_text(text: &str, secret: &str) -> String {
    let mut out = text.to_string();
    if !secret.is_empty() {
        out = out.replace(secret, "***");
    }
    out
}

#[tauri::command]
pub async fn metadata_clash_import(text: Option<String>, path: Option<String>) -> Result<Vec<NodePreview>, String> {
    let raw = if let Some(path) = path.filter(|p| !p.trim().is_empty()) {
        std::fs::read_to_string(path.trim()).map_err(|_| "无法读取这个文件".to_string())?
    } else {
        text.unwrap_or_default()
    };
    let raw = raw.trim().to_string();
    if raw.is_empty() {
        return Err("请粘贴配置、选择文件，或填写订阅地址".into());
    }
    let body = if raw.starts_with("http://") || raw.starts_with("https://") {
        if raw.len() > 2000 || raw.contains('\n') {
            return Err("订阅地址无效".into());
        }
        let cfg = super::config::load();
        match direct_get(&cfg, &raw).await {
            Ok(text) => text,
            Err(_) => return Err("订阅下载失败".into()),
        }
    } else {
        raw
    };
    let previews = previews_from(&body)?;
    write_import(&body)?;
    Ok(previews)
}

#[tauri::command]
pub fn metadata_clash_nodes() -> Result<Vec<NodePreview>, String> {
    previews_from(&read_import()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_hides_secrets_and_minimal_config_is_loopback_only() {
        let yaml = r#"
proxies:
  - name: "hk 01"
    type: ss
    server: edge.example
    port: 443
    cipher: aes-128-gcm
    password: super-secret
  - name: local
    type: http
    server: 127.0.0.1
    port: 8080
"#;
        let nodes = previews_from(yaml).unwrap();
        assert_eq!(nodes.len(), 2);
        assert!(nodes[0].encrypted);
        assert!(!nodes[1].encrypted);
        assert_eq!(nodes[0].server, "edge.example");
        let node = node_by_name(yaml, "hk 01").unwrap();
        let cfg = minimal_config(&node, 40123, 40124, "sekret-value").unwrap();
        assert!(cfg.contains("allow-lan: false"));
        assert!(cfg.contains("bind-address: 127.0.0.1"));
        assert!(cfg.contains("external-controller: 127.0.0.1:40124"));
        assert!(!cfg.contains("tun:"));
        assert!(cfg.contains("super-secret"));
        let shown = redact_text(&cfg, "sekret-value");
        assert!(!shown.contains("sekret-value"));
        assert_eq!(cfg.matches("- name:").count(), 2);
    }
}
