//! Shelf CRUD and user-defined category (folder) commands.

use rusqlite::params;
use serde::Serialize;
use tauri::AppHandle;

use crate::appdata::{db_conn, now_secs};

#[derive(Serialize, Clone)]
pub struct ShelfEntry {
    pub id: i64,
    pub name: String,
    pub kind: String, // zip | folder | images
    pub paths: String, // JSON 数组
    pub cover: String, // dataURL 缩略图
    pub page_count: i64,
    pub total_size: i64,
    pub last_page: i64, // 0-based
    pub created_at: i64,
    pub updated_at: i64,
    pub folder_id: Option<i64>, // 所属目录，NULL=未分类
    /// True when this row is a folder merge that can be split back apart.
    pub merged: bool,
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

const SHELF_COLS: &str =
    "id,name,kind,paths,cover,page_count,total_size,last_page,created_at,updated_at,folder_id";
const SHELF_MERGED: &str =
    "EXISTS(SELECT 1 FROM book_merges m WHERE m.book_id = shelf.id AND m.undone=0)";

#[tauri::command]
pub fn shelf_list(_app: AppHandle) -> Result<Vec<ShelfEntry>, String> {
    let conn = db_conn()?;
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {SHELF_COLS}, {SHELF_MERGED} FROM shelf ORDER BY updated_at DESC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], row_to_entry)
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn shelf_get(_app: AppHandle, id: i64) -> Result<Option<ShelfEntry>, String> {
    let conn = db_conn()?;
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {SHELF_COLS}, {SHELF_MERGED} FROM shelf WHERE id=?"
        ))
        .map_err(|e| e.to_string())?;
    let mut rows = stmt.query_map([id], row_to_entry).map_err(|e| e.to_string())?;
    match rows.next() {
        Some(r) => Ok(Some(r.map_err(|e| e.to_string())?)),
        None => Ok(None),
    }
}

#[tauri::command]
pub fn shelf_remove(_app: AppHandle, id: i64) -> Result<(), String> {
    let conn = db_conn()?;
    conn.execute("DELETE FROM shelf WHERE id=?", [id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn shelf_clear(_app: AppHandle) -> Result<(), String> {
    let conn = db_conn()?;
    conn.execute("DELETE FROM shelf", [])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Serialize)]
pub struct FolderInfo {
    pub id: i64,
    pub name: String,
    pub created_at: i64,
    pub count: i64,
}

#[tauri::command]
pub fn folder_add(name: String) -> Result<i64, String> {
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
pub fn folder_list() -> Result<Vec<FolderInfo>, String> {
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
pub fn folder_rename(id: i64, name: String) -> Result<(), String> {
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
pub fn folder_remove(id: i64) -> Result<(), String> {
    let conn = db_conn()?;
    conn.execute("UPDATE shelf SET folder_id=NULL WHERE folder_id=?", [id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM folders WHERE id=?", [id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// ids are the category ids in the order the shelf should show them.
#[tauri::command]
pub fn folder_reorder(ids: Vec<i64>) -> Result<(), String> {
    let conn = db_conn()?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    for (i, id) in ids.iter().enumerate() {
        tx.execute(
            "UPDATE folders SET sort_order=?1 WHERE id=?2",
            params![i as i64, id],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// 把图书移入/移出目录（folder_id 为 null 时移回未分类）
/// 注意：Tauri 默认要求前端用 camelCase 传参，此命令显式声明 snake_case
#[tauri::command(rename_all = "snake_case")]
pub fn shelf_move(id: i64, folder_id: Option<i64>) -> Result<(), String> {
    let conn = db_conn()?;
    conn.execute(
        "UPDATE shelf SET folder_id=? WHERE id=?",
        params![folder_id, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
