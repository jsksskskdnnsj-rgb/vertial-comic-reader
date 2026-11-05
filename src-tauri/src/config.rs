//! Reader preferences and theme/banner/hero config stored under the data dir.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::appdata::{data_dir, now_secs};

/* ================= 阅读配置（省内存模式 / 默认阅读模式） ================= */

#[derive(Serialize, Deserialize, Clone)]
pub struct ReaderConfig {
    pub memory_saver: bool,
    pub mode: String, // scroll | page
    #[serde(default = "default_close_to_tray")]
    pub close_to_tray: bool,
}

fn default_close_to_tray() -> bool {
    true
}
impl Default for ReaderConfig {
    fn default() -> Self {
        ReaderConfig {
            memory_saver: false,
            mode: "scroll".to_string(),
            close_to_tray: default_close_to_tray(),
        }
    }
}

#[tauri::command]
pub fn get_reader_config() -> Result<ReaderConfig, String> {
    let p = data_dir()?.join("reader-config.json");
    if let Ok(s) = std::fs::read_to_string(&p) {
        if let Ok(c) = serde_json::from_str::<ReaderConfig>(&s) {
            return Ok(c);
        }
    }
    Ok(ReaderConfig::default())
}

#[tauri::command]
pub fn set_reader_config(cfg: ReaderConfig) -> Result<(), String> {
    let p = data_dir()?.join("reader-config.json");
    let s = serde_json::to_string(&cfg).map_err(|e| e.to_string())?;
    std::fs::write(&p, s).map_err(|e| e.to_string())
}

/* ================= 主题配置（整体配色 + 首页横幅） ================= */

#[derive(Serialize, Deserialize, Clone)]
pub struct ThemeConfig {
    pub theme: String,      // lavender | pink | blue | mint | dark | custom
    pub custom_pri: String, // 自定义主色 hex
    pub hero: String,       // "default" 或自定义横幅绝对路径
    /// "default" 或自定义首页横幅绝对路径（数据目录下的 banner-custom.<ext>），与整体背景 hero 相互独立
    #[serde(default = "default_banner")]
    pub banner: String,
}

fn default_banner() -> String {
    "default".to_string()
}
impl Default for ThemeConfig {
    fn default() -> Self {
        ThemeConfig {
            theme: "lavender".to_string(),
            custom_pri: "#8b5cf6".to_string(),
            hero: "default".to_string(),
            banner: default_banner(),
        }
    }
}

#[tauri::command]
pub fn get_theme_config() -> Result<ThemeConfig, String> {
    let p = data_dir()?.join("theme-config.json");
    if let Ok(s) = std::fs::read_to_string(&p) {
        if let Ok(c) = serde_json::from_str::<ThemeConfig>(&s) {
            return Ok(c);
        }
    }
    Ok(ThemeConfig::default())
}

#[tauri::command]
pub fn set_theme_config(cfg: ThemeConfig) -> Result<(), String> {
    let p = data_dir()?.join("theme-config.json");
    let s = serde_json::to_string(&cfg).map_err(|e| e.to_string())?;
    std::fs::write(&p, s).map_err(|e| e.to_string())
}

/// 把用户选的图复制为数据目录下的横幅（hero-custom.<ext>），返回目标路径
#[tauri::command]
pub fn set_hero_image(src: String) -> Result<String, String> {
    let dir = data_dir()?;
    let ext = PathBuf::from(&src)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .filter(|e| !e.is_empty() && e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or_else(|| "jpg".to_string());
    // 先清掉旧的自定义横幅
    clear_hero_files(&dir);
    let dest = dir.join(format!("hero-custom.{}", ext));
    std::fs::copy(&src, &dest).map_err(|e| e.to_string())?;
    Ok(dest.to_string_lossy().to_string())
}

fn clear_hero_files(dir: &PathBuf) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("hero-custom.") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

#[tauri::command]
pub fn reset_hero_image() -> Result<(), String> {
    clear_hero_files(&data_dir()?);
    Ok(())
}

/// 把用户选的图复制为数据目录下的首页横幅（banner-custom.<ext>），返回目标路径
#[tauri::command]
pub fn set_banner_image(src: String) -> Result<String, String> {
    let dir = data_dir()?;
    let ext = PathBuf::from(&src)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .filter(|e| !e.is_empty() && e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or_else(|| "jpg".to_string());
    let dest = dir.join(format!("banner-custom.{}", ext));
    // 先复制到临时文件再替换旧横幅：选的图本身就是当前横幅时不会被先删掉
    let tmp = dir.join(format!("banner-custom.tmp-{}", now_secs()));
    std::fs::copy(&src, &tmp).map_err(|e| e.to_string())?;
    clear_banner_files(&dir);
    std::fs::rename(&tmp, &dest).map_err(|e| e.to_string())?;
    Ok(dest.to_string_lossy().to_string())
}

fn clear_banner_files(dir: &PathBuf) {
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with("banner-custom.") && !n.starts_with("banner-custom.tmp-") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

#[tauri::command]
pub fn reset_banner_image() -> Result<(), String> {
    clear_banner_files(&data_dir()?);
    Ok(())
}

/// 迁移一个主题资源（hero/banner 自定义图）：把文件复制到新目录（已存在则不覆盖），返回新路径
fn migrate_theme_asset(old_dir: &Path, new_dir: &Path, p: &mut String) -> Result<(), String> {
    if p.as_str() == "default" {
        return Ok(());
    }
    if let Some(name) = Path::new(p.as_str()).file_name() {
        let old_f = old_dir.join(name);
        let new_f = new_dir.join(name);
        if old_f.exists() && !new_f.exists() {
            std::fs::copy(&old_f, &new_f).map_err(|e| format!("迁移图片失败：{}", e))?;
        }
        *p = new_f.to_string_lossy().to_string();
    }
    Ok(())
}

/// Copy theme-config.json + custom hero/banner assets when the data dir changes.
pub(crate) fn migrate_theme_config(old_dir: &Path, new_dir: &Path) -> Result<(), String> {
    let old_theme = old_dir.join("theme-config.json");
    let new_theme = new_dir.join("theme-config.json");
    if old_theme.exists() && !new_theme.exists() {
        let mut cfg: ThemeConfig = std::fs::read_to_string(&old_theme)
            .ok()
            .and_then(|s| serde_json::from_str::<ThemeConfig>(&s).ok())
            .unwrap_or_default();
        migrate_theme_asset(old_dir, new_dir, &mut cfg.hero)?;
        migrate_theme_asset(old_dir, new_dir, &mut cfg.banner)?;
        let s = serde_json::to_string(&cfg).map_err(|e| e.to_string())?;
        std::fs::write(&new_theme, s).map_err(|e| format!("迁移主题配置失败：{}", e))?;
    }
    Ok(())
}
