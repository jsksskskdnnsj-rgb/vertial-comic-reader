//! Data directory resolution, SQLite bootstrap, and data-dir migration.

use rusqlite::Connection;
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

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

pub(crate) fn exe_dir() -> Result<PathBuf, String> {
    std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| "无法确定程序目录".to_string())
}

/// 在文件资源管理器中打开本机数据目录（mh-reader.db、cache、covers）
#[tauri::command]
pub fn open_data_dir(_app: AppHandle) -> Result<(), String> {
    let dir = data_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .arg(&dir)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_data_dir() -> Result<String, String> {
    Ok(data_dir()?.to_string_lossy().to_string())
}

/// 默认数据目录（exe 所在目录）。前端用它判断当前是否为默认位置，并作为"恢复默认位置"的目标：
/// set_data_dir 选中 exe 目录即删除 data-dir.txt、恢复默认
#[tauri::command]
pub fn get_default_data_dir() -> Result<String, String> {
    Ok(exe_dir()?.to_string_lossy().to_string())
}

#[tauri::command]
pub fn pick_data_dir(app: AppHandle) -> Result<Option<String>, String> {
    Ok(app
        .dialog()
        .file()
        .blocking_pick_folder()
        .and_then(crate::paths::fp_to_string))
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

/// 切换数据目录：把现有数据（数据库、阅读配置、页面缓存）复制到新位置（不删除旧文件，
/// 新目录已有同名文件不覆盖），写入 exe 旁的 data-dir.txt；
/// 若选的就是 exe 目录则删除配置文件，即恢复默认
#[tauri::command]
pub fn set_data_dir(path: String) -> Result<String, String> {
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

        crate::config::migrate_theme_config(&old_dir, &new_dir)?;
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
    crate::shelf_scan::ensure_scan_tables(&conn)?;
    crate::merge::ensure_tables(&conn)?;
    Ok(conn)
}

pub(crate) fn install_panic_log() {
    let dir = data_dir().ok();
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(dir) = &dir {
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("panic.log"))
            {
                use std::io::Write;
                let _ = writeln!(f, "----\n{info}");
            }
        }
        prev(info);
    }));
}
