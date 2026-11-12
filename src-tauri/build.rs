fn main() {
    // Tauri's sidecar list is resolved for the crate target. Windows packages
    // ship the real mihomo exe fetched by scripts/fetch-mihomo.mjs. Other targets
    // only need a placeholder so `cargo test` can run without that Windows binary.
    ensure_sidecar_placeholder();
    tauri_build::build()
}

fn ensure_sidecar_placeholder() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("windows") {
        return;
    }
    let path = std::path::Path::new("binaries").join(format!("mihomo-{target}"));
    if path.exists() {
        return;
    }
    let _ = std::fs::create_dir_all("binaries");
    // Not a proxy core. The Windows installer uses mihomo-x86_64-pc-windows-msvc.exe.
    let _ = std::fs::write(&path, b"#!/bin/sh\nexit 1\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
    }
}
