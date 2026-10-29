#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// v0.4 reader backend (split from design/claude-v04/reader_backend.rs; see
// design/claude-v04/INTEGRATION_NOTES.md §3 for the shelf-unification decisions).
mod adapters;
mod bookmarks;
mod cache;
mod chapters;
mod commands;
mod cover_protocol;
mod db;
mod error;
mod extractor;
mod library;
mod merge;
mod model;
mod protocol;
mod scan;
mod shelf_scan;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, FilePath};

/// Extensions admitted at import and recognised by `extractor::sniff`.
/// `avif` is passthrough: the webview displays the original bytes. The `image`
/// crate is not built with an AVIF decoder, so covers, slices, and width
/// variants use `DECODE_EXTS` only. svg/tif/heic/jxl are rejected at import —
/// they used to succeed and then fail every page as "not a recognised image".
pub(crate) const IMAGE_EXTS: &[&str] = &[
    "jpg", "jpeg", "jfif", "jpe", "png", "webp", "gif", "bmp", "avif",
];
/// Formats `image` 0.25 is compiled to decode (see Cargo.toml features).
pub(crate) const DECODE_EXTS: &[&str] = &[
    "jpg", "jpeg", "jfif", "jpe", "png", "webp", "gif", "bmp",
];
const ZIP_EXTS: &[&str] = &["zip", "cbz"];

/* ================= 通用 ================= */

#[derive(Serialize, Clone)]
struct PickedFile {
    name: String, // 显示/排序用（文件夹场景为相对路径）
    path: String, // 本地绝对路径
    size: u64,
}

fn ext_of(p: &Path) -> String {
    p.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn kind_of(p: &Path) -> Option<&'static str> {
    let e = ext_of(p);
    if IMAGE_EXTS.contains(&e.as_str()) {
        Some("image")
    } else if ZIP_EXTS.contains(&e.as_str()) {
        Some("zip")
    } else {
        None
    }
}

fn fp_to_string(fp: FilePath) -> Option<String> {
    match fp {
        FilePath::Path(p) => Some(p.to_string_lossy().to_string()),
        FilePath::Url(u) => Some(u.to_string()),
    }
}

fn to_picked(path_str: String, rel: String) -> Option<PickedFile> {
    let p = PathBuf::from(&path_str);
    if kind_of(&p).is_none() {
        return None;
    }
    let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
    Some(PickedFile {
        name: rel,
        path: path_str,
        size,
    })
}

fn walk_dir(dir: &Path, base: &Path, out: &mut Vec<PickedFile>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut subs: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            subs.push(p);
            continue;
        }
        if kind_of(&p).is_none() {
            continue;
        }
        let rel = p
            .strip_prefix(base)
            .unwrap_or(&p)
            .to_string_lossy()
            .replace('\\', "/");
        let abs = p.to_string_lossy().to_string();
        if let Some(pf) = to_picked(abs, rel) {
            out.push(pf);
        }
    }
    subs.sort();
    for s in subs {
        walk_dir(&s, base, out);
    }
}

/* ================= 文件选择 ================= */

/// 选择文件夹（可多选），只返回文件夹路径
#[tauri::command]
fn pick_folder_paths(app: AppHandle) -> Vec<String> {
    app.dialog()
        .file()
        .blocking_pick_folders()
        .unwrap_or_default()
        .into_iter()
        .filter_map(fp_to_string)
        .collect()
}

/// 列出文件夹下的图片/压缩包（供书架打开时用）
#[tauri::command]
fn list_images(path: String) -> Result<Vec<PickedFile>, String> {
    let base = PathBuf::from(&path);
    if !base.is_dir() {
        return Err("文件夹不存在".to_string());
    }
    let mut out = Vec::new();
    walk_dir(&base, &base, &mut out);
    Ok(out)
}

/// 在文件资源管理器中打开本机数据目录（mh-reader.db、cache、covers）
#[tauri::command]
fn open_data_dir(_app: AppHandle) -> Result<(), String> {
    let dir = data_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .arg(&dir)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

/* ================= SQLite 书架 ================= */

#[derive(Serialize, Clone)]
struct ShelfEntry {
    id: i64,
    name: String,
    kind: String, // zip | folder | images
    paths: String, // JSON 数组
    cover: String, // dataURL 缩略图
    page_count: i64,
    total_size: i64,
    last_page: i64, // 0-based
    created_at: i64,
    updated_at: i64,
    folder_id: Option<i64>, // 所属目录，NULL=未分类
    /// True when this row is a folder merge that can be split back apart.
    merged: bool,
}

pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/* ================= 无边框窗口控制（走 Rust，避开前端 API 差异） ================= */

#[tauri::command]
fn win_minimize(window: tauri::Window) -> Result<(), String> {
    window.minimize().map_err(|e| e.to_string())
}

#[tauri::command]
fn win_toggle_maximize(window: tauri::Window) -> Result<(), String> {
    if window.is_maximized().map_err(|e| e.to_string())? {
        window.unmaximize().map_err(|e| e.to_string())
    } else {
        window.maximize().map_err(|e| e.to_string())
    }
}

#[tauri::command]
fn win_close(window: tauri::Window) -> Result<(), String> {
    window.close().map_err(|e| e.to_string())
}

/// 原生文件拖放：事件把路径写入队列文件，前端轮询 take_drop_queue 取走
/// （全程走 invoke 通道，不依赖前端 webview/event API）
fn drop_queue_path() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("drop-queue.json"))
}

fn push_drop_queue(paths: &[PathBuf]) -> Result<(), String> {
    let p = drop_queue_path()?;
    let mut q: Vec<String> = std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    q.extend(paths.iter().map(|x| x.to_string_lossy().to_string()));
    std::fs::write(&p, serde_json::to_string(&q).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn take_drop_queue() -> Result<Vec<String>, String> {
    let p = drop_queue_path()?;
    let q: Vec<String> = std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let _ = std::fs::remove_file(&p);
    Ok(q)
}

/* ================= 数据目录（可自定义，默认 exe 所在目录） ================= */

/// 数据目录解析：exe 旁的 data-dir.txt 可指定自定义目录；默认就是 exe 所在目录
pub(crate) fn data_dir() -> Result<PathBuf, String> {
    let exe_dir = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "无法确定程序目录".to_string())?;
    let mut dir = exe_dir.clone();
    let cfg = exe_dir.join("data-dir.txt");
    if cfg.exists() {
        if let Ok(s) = std::fs::read_to_string(&cfg) {
            let s = s.trim();
            if !s.is_empty() {
                let p = PathBuf::from(s);
                if p.is_dir() {
                    dir = p;
                }
            }
        }
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn exe_dir() -> Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "无法确定程序目录".to_string())
}

#[tauri::command]
fn get_data_dir() -> Result<String, String> {
    Ok(data_dir()?.to_string_lossy().to_string())
}

/* ================= 阅读配置（省内存模式 / 默认阅读模式） ================= */

#[derive(Serialize, Deserialize, Clone)]
struct ReaderConfig {
    memory_saver: bool,
    mode: String, // scroll | page
    #[serde(default = "default_close_to_tray")]
    close_to_tray: bool,
}

fn default_close_to_tray() -> bool {
    true
}
impl Default for ReaderConfig {
    fn default() -> Self {
        ReaderConfig { memory_saver: false, mode: "scroll".to_string(), close_to_tray: default_close_to_tray() }
    }
}

#[tauri::command]
fn get_reader_config() -> Result<ReaderConfig, String> {
    let p = data_dir()?.join("reader-config.json");
    if let Ok(s) = std::fs::read_to_string(&p) {
        if let Ok(c) = serde_json::from_str::<ReaderConfig>(&s) {
            return Ok(c);
        }
    }
    Ok(ReaderConfig::default())
}

#[tauri::command]
fn set_reader_config(cfg: ReaderConfig) -> Result<(), String> {
    let p = data_dir()?.join("reader-config.json");
    let s = serde_json::to_string(&cfg).map_err(|e| e.to_string())?;
    std::fs::write(&p, s).map_err(|e| e.to_string())
}

/* ================= 主题配置（整体配色 + 首页横幅） ================= */

#[derive(Serialize, Deserialize, Clone)]
struct ThemeConfig {
    theme: String,      // lavender | pink | blue | mint | dark | custom
    custom_pri: String, // 自定义主色 hex
    hero: String,       // "default" 或自定义横幅绝对路径
    /// "default" 或自定义首页横幅绝对路径（数据目录下的 banner-custom.<ext>），与整体背景 hero 相互独立
    #[serde(default = "default_banner")]
    banner: String,
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
fn get_theme_config() -> Result<ThemeConfig, String> {
    let p = data_dir()?.join("theme-config.json");
    if let Ok(s) = std::fs::read_to_string(&p) {
        if let Ok(c) = serde_json::from_str::<ThemeConfig>(&s) {
            return Ok(c);
        }
    }
    Ok(ThemeConfig::default())
}

#[tauri::command]
fn set_theme_config(cfg: ThemeConfig) -> Result<(), String> {
    let p = data_dir()?.join("theme-config.json");
    let s = serde_json::to_string(&cfg).map_err(|e| e.to_string())?;
    std::fs::write(&p, s).map_err(|e| e.to_string())
}

/// 把用户选的图复制为数据目录下的横幅（hero-custom.<ext>），返回目标路径
#[tauri::command]
fn set_hero_image(src: String) -> Result<String, String> {
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
fn reset_hero_image() -> Result<(), String> {
    clear_hero_files(&data_dir()?);
    Ok(())
}

/// 把用户选的图复制为数据目录下的首页横幅（banner-custom.<ext>），返回目标路径
#[tauri::command]
fn set_banner_image(src: String) -> Result<String, String> {
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
fn reset_banner_image() -> Result<(), String> {
    clear_banner_files(&data_dir()?);
    Ok(())
}
/// 默认数据目录（exe 所在目录）。前端用它判断当前是否为默认位置，并作为"恢复默认位置"的目标：
/// set_data_dir 选中 exe 目录即删除 data-dir.txt、恢复默认
#[tauri::command]
fn get_default_data_dir() -> Result<String, String> {
    Ok(exe_dir()?.to_string_lossy().to_string())
}
#[tauri::command]
fn pick_data_dir(app: AppHandle) -> Result<Option<String>, String> {
    Ok(app
        .dialog()
        .file()
        .blocking_pick_folder()
        .and_then(fp_to_string))
}

/// 递归复制目录：只复制目标不存在的文件，已存在的跳过（不覆盖）
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    if !src.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(dst).map_err(|e| format!("创建目录失败：{}", e))?;
    let entries = std::fs::read_dir(src).map_err(|e| format!("读取目录失败：{}", e))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("读取目录项失败：{}", e))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let ft = entry
            .file_type()
            .map_err(|e| format!("读取文件类型失败：{}", e))?;
        if ft.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if ft.is_file() && !dst_path.exists() {
            std::fs::copy(&src_path, &dst_path).map_err(|e| format!("复制文件失败：{}", e))?;
        }
        // 符号链接等其他类型跳过
    }
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
/// 切换数据目录：把现有数据（数据库、阅读配置、页面缓存）复制到新位置（不删除旧文件，
/// 新目录已有同名文件不覆盖），写入 exe 旁的 data-dir.txt；
/// 若选的就是 exe 目录则删除配置文件，即恢复默认
#[tauri::command]
fn set_data_dir(path: String) -> Result<String, String> {
    let new_dir = PathBuf::from(path.trim());
    if !new_dir.is_dir() {
        return Err("文件夹不存在".to_string());
    }
    let old_dir = data_dir()?;
    if new_dir != old_dir {
        // 数据库
        let old_db = old_dir.join("mh-reader.db");
        let new_db = new_dir.join("mh-reader.db");
        if old_db.exists() && !new_db.exists() {
            std::fs::copy(&old_db, &new_db).map_err(|e| format!("迁移数据库失败：{}", e))?;
        }
        // 阅读配置（默认阅读模式/省内存开关等）
        let old_cfg = old_dir.join("reader-config.json");
        let new_cfg = new_dir.join("reader-config.json");
        if old_cfg.exists() && !new_cfg.exists() {
            std::fs::copy(&old_cfg, &new_cfg).map_err(|e| format!("迁移阅读配置失败：{}", e))?;
        }
        // 页面缓存（已解压的 ZIP 页面，递归复制，已存在的跳过）
        let old_cache = old_dir.join("page_cache");
        let new_cache = new_dir.join("page_cache");
        if old_cache.is_dir() {
            copy_dir_recursive(&old_cache, &new_cache)?;
        }

        // 主题配置 + 自定义背景图/横幅（路径改指向新目录）
        let old_theme = old_dir.join("theme-config.json");
        let new_theme = new_dir.join("theme-config.json");
        if old_theme.exists() && !new_theme.exists() {
            let mut cfg: ThemeConfig = std::fs::read_to_string(&old_theme)
                .ok()
                .and_then(|s| serde_json::from_str::<ThemeConfig>(&s).ok())
                .unwrap_or_default();
            migrate_theme_asset(&old_dir, &new_dir, &mut cfg.hero)?;
            migrate_theme_asset(&old_dir, &new_dir, &mut cfg.banner)?;
            let s = serde_json::to_string(&cfg).map_err(|e| e.to_string())?;
            std::fs::write(&new_theme, s).map_err(|e| format!("迁移主题配置失败：{}", e))?;
        }
        // 封面缓存（可再生，复制只为省去重新生成）
        copy_dir_recursive(&old_dir.join("covers"), &new_dir.join("covers"))?;
    }
    let exe = exe_dir()?;
    let cfg = exe.join("data-dir.txt");
    if new_dir == exe {
        let _ = std::fs::remove_file(&cfg);
    } else {
        std::fs::write(&cfg, new_dir.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(new_dir.to_string_lossy().to_string())
}

pub(crate) fn db_conn() -> Result<Connection, String> {
    let dir = data_dir()?;
    let conn = Connection::open(dir.join("mh-reader.db")).map_err(|e| e.to_string())?;
    // v0.4: the design backend holds one long-lived connection to this same file;
    // wait on contention instead of failing with SQLITE_BUSY (both sides set this).
    conn.busy_timeout(std::time::Duration::from_millis(5000))
        .map_err(|e| e.to_string())?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS shelf (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            kind TEXT NOT NULL,
            paths TEXT NOT NULL,
            cover TEXT NOT NULL DEFAULT '',
            page_count INTEGER NOT NULL DEFAULT 0,
            total_size INTEGER NOT NULL DEFAULT 0,
            last_page INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            folder_id INTEGER
        );
        CREATE TABLE IF NOT EXISTS folders (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL,
            created_at INTEGER NOT NULL
        )",
    )
    .map_err(|e| e.to_string())?;
    // 轻量迁移：给旧库补 folder_id 列
    let has_col: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('shelf') WHERE name='folder_id'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has_col == 0 {
        conn.execute("ALTER TABLE shelf ADD COLUMN folder_id INTEGER", [])
            .map_err(|e| e.to_string())?;
    }
    // v0.4 轻量迁移（同一 pragma 检查风格）：reader 后端需要的三个新列
    // scan_sig: 扫描签名（变更检测）；finished: 是否读完；cover_ver: 封面版本号（mhcover 缓存失效）
    for (col, ddl) in [
        ("scan_sig", "ALTER TABLE shelf ADD COLUMN scan_sig TEXT"),
        (
            "finished",
            "ALTER TABLE shelf ADD COLUMN finished INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "cover_ver",
            "ALTER TABLE shelf ADD COLUMN cover_ver INTEGER NOT NULL DEFAULT 1",
        ),
    ] {
        let has: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('shelf') WHERE name=?1",
                [col],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if has == 0 {
            conn.execute(ddl, []).map_err(|e| e.to_string())?;
        }
    }
    let has_order: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('folders') WHERE name='sort_order'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has_order == 0 {
        conn.execute("ALTER TABLE folders ADD COLUMN sort_order INTEGER", [])
            .map_err(|e| e.to_string())?;
        conn.execute("UPDATE folders SET sort_order = id WHERE sort_order IS NULL", [])
            .map_err(|e| e.to_string())?;
    }
    // WAL keeps a large shelf responsive while a scan writes its checkpoint.
    // A failure here (some network drives) leaves the default journal in place.
    let _ = conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;");
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_shelf_folder ON shelf(folder_id);
         CREATE INDEX IF NOT EXISTS idx_shelf_updated ON shelf(updated_at);
         CREATE INDEX IF NOT EXISTS idx_shelf_name ON shelf(name);",
    )
    .map_err(|e| e.to_string())?;
    shelf_scan::ensure_scan_tables(&conn)?;
    merge::ensure_tables(&conn)?;
    Ok(conn)
}

fn row_to_entry(r: &rusqlite::Row) -> rusqlite::Result<ShelfEntry> {
    Ok(ShelfEntry {
        id: r.get(0)?,
        name: r.get(1)?,
        kind: r.get(2)?,
        paths: r.get(3)?,
        cover: r.get(4)?,
        page_count: r.get(5)?,
        total_size: r.get(6)?,
        last_page: r.get(7)?,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
        folder_id: r.get(10)?,
        merged: r.get::<_, i64>(11)? != 0,
    })
}

const SHELF_COLS: &str = "id,name,kind,paths,cover,page_count,total_size,last_page,created_at,updated_at,folder_id";
const SHELF_MERGED: &str = "EXISTS(SELECT 1 FROM book_merges m WHERE m.book_id = shelf.id AND m.undone=0)";

#[tauri::command]
fn shelf_list(_app: AppHandle) -> Result<Vec<ShelfEntry>, String> {
    let conn = db_conn()?;
    let mut stmt = conn
        .prepare(&format!("SELECT {SHELF_COLS}, {SHELF_MERGED} FROM shelf ORDER BY updated_at DESC"))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], row_to_entry)
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn shelf_get(_app: AppHandle, id: i64) -> Result<Option<ShelfEntry>, String> {
    let conn = db_conn()?;
    let mut stmt = conn
        .prepare(&format!("SELECT {SHELF_COLS}, {SHELF_MERGED} FROM shelf WHERE id=?"))
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query_map([id], row_to_entry).map_err(|e| e.to_string())?;
    match rows.next() {
        Some(r) => Ok(Some(r.map_err(|e| e.to_string())?)),
        None => Ok(None),
    }
}

#[tauri::command]
fn shelf_remove(_app: AppHandle, id: i64) -> Result<(), String> {
    let conn = db_conn()?;
    conn.execute("DELETE FROM shelf WHERE id=?", [id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn shelf_clear(_app: AppHandle) -> Result<(), String> {
    let conn = db_conn()?;
    conn.execute("DELETE FROM shelf", [])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/* ================= 书架目录（图书分类） ================= */

#[derive(Serialize)]
struct FolderInfo {
    id: i64,
    name: String,
    created_at: i64,
    count: i64,
}

#[tauri::command]
fn folder_add(name: String) -> Result<i64, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("分类名不能为空".to_string());
    }
    let conn = db_conn()?;
    conn.execute(
        "INSERT INTO folders (name, created_at, sort_order) VALUES (?1, ?2, COALESCE((SELECT MAX(sort_order) FROM folders), 0) + 1)",
        params![name, now_secs()],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

#[tauri::command]
fn folder_list() -> Result<Vec<FolderInfo>, String> {
    let conn = db_conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT f.id, f.name, f.created_at, COUNT(s.id)
             FROM folders f LEFT JOIN shelf s ON s.folder_id = f.id
             GROUP BY f.id ORDER BY COALESCE(f.sort_order, f.id), f.id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(FolderInfo {
                id: r.get(0)?,
                name: r.get(1)?,
                created_at: r.get(2)?,
                count: r.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn folder_rename(id: i64, name: String) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("分类名不能为空".to_string());
    }
    let conn = db_conn()?;
    conn.execute("UPDATE folders SET name=? WHERE id=?", params![name, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn folder_remove(id: i64) -> Result<(), String> {
    let conn = db_conn()?;
    conn.execute("UPDATE shelf SET folder_id=NULL WHERE folder_id=?", [id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM folders WHERE id=?", [id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// ids are the category ids in the order the shelf should show them.
#[tauri::command]
fn folder_reorder(ids: Vec<i64>) -> Result<(), String> {
    let conn = db_conn()?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    for (i, id) in ids.iter().enumerate() {
        tx.execute("UPDATE folders SET sort_order=?1 WHERE id=?2", params![i as i64, id])
            .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// 把图书移入/移出目录（folder_id 为 null 时移回未分类）
/// 注意：Tauri 默认要求前端用 camelCase 传参，此命令显式声明 snake_case
#[tauri::command(rename_all = "snake_case")]
fn shelf_move(id: i64, folder_id: Option<i64>) -> Result<(), String> {
    let conn = db_conn()?;
    conn.execute("UPDATE shelf SET folder_id=? WHERE id=?", params![folder_id, id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/* ================= 入口 ================= */

// ---- v0.6.4: system tray, close-to-tray, window drag, real exit ----
// Hiding to the tray is only allowed once the tray icon exists (otherwise a hidden window could never be brought back).
static TRAY_OK: AtomicBool = AtomicBool::new(false);
fn hide_to_tray_enabled() -> bool {
    TRAY_OK.load(Ordering::SeqCst) && get_reader_config().map(|c| c.close_to_tray).unwrap_or(false)
}
fn show_main_window(app: &tauri::AppHandle) {
    use tauri::Manager;
    let w = app.get_webview_window("main").or_else(|| app.webview_windows().into_values().next());
    if let Some(w) = w {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}
fn init_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    // no icon -> no tray, and hide-to-tray stays off (see TRAY_OK)
    let icon = match app.default_window_icon() {
        Some(i) => i.clone(),
        None => return Ok(()),
    };
    let show = MenuItem::with_id(app, "tray_show", "显示 / Show", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "tray_quit", "退出 / Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    #[allow(deprecated)] // renamed show_menu_on_left_click in tauri 2.2; the old name compiles on every 2.x
    let builder = TrayIconBuilder::with_id("mh-tray")
        .icon(icon)
        .tooltip("MH Reader")
        .menu(&menu)
        .menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray_show" => show_main_window(app),
            "tray_quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main_window(tray.app_handle());
            }
        });
    builder.build(app)?;
    TRAY_OK.store(true, Ordering::SeqCst);
    Ok(())
}
/// 拖动窗口（自定义标题栏按下鼠标时调用）
#[tauri::command]
fn win_start_drag(window: tauri::Window) -> Result<(), String> {
    window.start_dragging().map_err(|e| e.to_string())
}
/// 关闭到托盘开启时隐藏窗口并返回 true；否则返回 false（调用方自行退出）。阅读页用它代替 destroy()
#[tauri::command]
fn win_hide_if_tray(window: tauri::Window) -> Result<bool, String> {
    if hide_to_tray_enabled() {
        window.hide().map_err(|e| e.to_string())?;
        Ok(true)
    } else {
        Ok(false)
    }
}
/// 真正退出程序（设置页"立即关闭程序"用；win_close 在关闭到托盘时只会隐藏窗口）
#[tauri::command]
fn app_exit(app: AppHandle) -> Result<(), String> {
    app.exit(0);
    Ok(())
}

fn install_panic_log() {
    let dir = data_dir().ok();
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(dir) = &dir {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("panic.log")) {
                use std::io::Write;
                let _ = writeln!(f, "----\n{info}");
            }
        }
        prev(info);
    }));
}

fn main() {
    install_panic_log();
    tauri::Builder::default()
        // Must be the first plugin. A second launch is handed to this process,
        // which unminimizes and focuses the window already on screen.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(&app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            use tauri::Manager;
            // v0.4 reader backend wiring (design reader_backend.rs wiring block,
            // adapted to this crate's data_dir()/db_conn()).
            app.manage(shelf_scan::ScanHost::default());
            let dir = data_dir().map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            // db_conn() runs the base DDL + shelf column migrations (incl. the new
            // scan_sig/finished/cover_ver columns the design DDL expects).
            let _ = db_conn().map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            let conn = Connection::open(dir.join("mh-reader.db"))?;
            conn.busy_timeout(std::time::Duration::from_millis(5000))?;
            // Design migrations once, in order: db DDL -> chapters DDL -> bookmarks
            // (bookmarks::migrate also runs bookmarks::DDL). All idempotent.
            conn.execute_batch(db::DDL)?;
            conn.execute_batch(db::EDITED_DDL)?;
            conn.execute_batch(chapters::DDL)?;
            bookmarks::migrate(&conn)?;
            let rdb: db::Db =
                std::sync::Arc::new(parking_lot::Mutex::new(conn));
            let cache = std::sync::Arc::new(cache::PageCache::open(
                dir.join("cache"),
                512 << 20,
            )?);
            let extractor = extractor::Extractor::start(cache.clone(), extractor::worker_count());
            app.manage(commands::ReaderState {
                db: rdb,
                cache,
                extractor,
            });
            let covers = dir.join("covers");
            cover_protocol::trim_covers(&covers, 512 << 20);
            app.manage(cover_protocol::CoversDir(std::sync::Arc::new(covers)));
            // v0.6.4 system tray; a failure here must not stop the app, it only keeps close-to-tray off
            if let Err(e) = init_tray(app) {
                eprintln!("tray: {}", e);
            }
            Ok(())
        })
        .register_asynchronous_uri_scheme_protocol("mhpage", |ctx, req, responder| {
            use tauri::Manager;
            let ex = ctx
                .app_handle()
                .state::<commands::ReaderState>()
                .extractor
                .clone();
            protocol::handle(ex, req, responder);
        })
        .register_asynchronous_uri_scheme_protocol("mhcover", |ctx, req, responder| {
            use tauri::Manager;
            let dir = ctx
                .app_handle()
                .state::<cover_protocol::CoversDir>()
                .0
                .clone();
            cover_protocol::cover_handle(dir, req, responder);
        })
        .register_asynchronous_uri_scheme_protocol("mhfile", |_ctx, req, responder| {
            tauri::async_runtime::spawn(async move {
                let resp = tauri::async_runtime::spawn_blocking(move || protocol::serve_theme_file(&req))
                    .await
                    .unwrap_or_else(|_| {
                        http::Response::builder().status(500).body(Vec::new()).unwrap()
                    });
                responder.respond(resp);
            });
        })
        .invoke_handler(tauri::generate_handler![
            pick_folder_paths,
            list_images,
            shelf_scan::scan_begin,
            shelf_scan::scan_status,
            shelf_scan::scan_cancel,
            shelf_scan::list_scan_issues,
            open_data_dir,
            shelf_list,
            shelf_get,
            shelf_remove,
            shelf_clear,
            get_data_dir,
            pick_data_dir,
            set_data_dir,
            set_banner_image,
            reset_banner_image,
            get_default_data_dir,
            get_reader_config,
            set_reader_config,
            get_theme_config,
            set_theme_config,
            set_hero_image,
            reset_hero_image,
            win_minimize,
            win_toggle_maximize,
            win_close,
            win_start_drag,
            win_hide_if_tray,
            app_exit,
            take_drop_queue,
            folder_add,
            folder_list,
            folder_rename,
            folder_remove,
            folder_reorder,
            shelf_move,
            // v0.4 reader core (design backend)
            commands::open_book,
            commands::rescan_book,
            commands::close_book,
            commands::set_reading_window,
            commands::save_progress,
            commands::load_progress,
            commands::fix_page_dims,
            commands::cache_stats,
            commands::cache_set_limit,
            commands::cache_clear,
            // v0.4 chapters
            chapters::get_chapter_defaults,
            chapters::set_chapter_defaults,
            chapters::set_book_chapter_settings,
            chapters::clear_book_chapter_settings,
            chapters::episode_info,
            // v0.4 bookmarks
            bookmarks::add_bookmark,
            bookmarks::list_bookmarks,
            bookmarks::delete_bookmark,
            bookmarks::update_bookmark_note,
            bookmarks::reanchor_bookmark,
            bookmarks::bookmark_counts,
            // v0.4 covers
            cover_protocol::ensure_cover,
            // v0.4 adapters (INTEGRATION_NOTES §2.3)
            adapters::list_recent_books,
            adapters::list_recent_added,
            adapters::import_paths,
            adapters::import_scan_books,
            merge::merge_auto_get,
            merge::merge_auto_set,
            merge::list_merge_pending,
            merge::resolve_merge_pending,
            merge::merge_books,
            merge::unmerge_book,
            adapters::reveal_book,
            adapters::relink_book,
            // v0.5 library management + page edits (library.rs)
            library::reorder_pages,
            library::delete_pages,
            library::append_pages,
            library::reset_page_edits,
            library::page_edit_info,
            library::shelf_rename,
            library::book_reset_progress,
            library::check_books_missing,
            library::import_merged,
            library::library_stats
        ])
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) => {
                let _ = push_drop_queue(paths);
            }
            tauri::WindowEvent::CloseRequested { api, .. } => {
                if hide_to_tray_enabled() {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
            _ => {}
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod image_ext_contract {
    #[test]
    fn dialog_and_decoder_lists_stay_aligned() {
        for ext in crate::DECODE_EXTS {
            assert!(
                crate::IMAGE_EXTS.contains(ext),
                "{ext} is decodable but not importable"
            );
        }
        assert!(crate::IMAGE_EXTS.contains(&"avif"));
        assert!(
            !crate::DECODE_EXTS.contains(&"avif"),
            "avif is passthrough-only until the image crate avif feature is enabled"
        );
        let n = crate::extractor::worker_count();
        assert!((2..=4).contains(&n), "extract workers {n} out of 2..=4");
        for banned in ["svg", "tif", "tiff", "heic", "heif", "jxl", "ico", "apng", "dib"] {
            assert!(!crate::IMAGE_EXTS.contains(&banned), "{banned} must not be imported");
            assert!(!crate::DECODE_EXTS.contains(&banned), "{banned} must not be a cover candidate");
        }
    }
}
