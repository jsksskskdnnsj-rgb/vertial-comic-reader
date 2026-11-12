//! Book tags + favorites (local library metadata; inspired by Yomikiru tag catalog, MIT — reimplemented).

use rusqlite::params;
use serde::Serialize;

use crate::appdata::{db_conn, now_secs};

pub const TAGS_DDL: &str = "
CREATE TABLE IF NOT EXISTS tags(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  name TEXT NOT NULL COLLATE NOCASE UNIQUE,
  color TEXT NOT NULL DEFAULT '#80e5b6',
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS book_tags(
  book_id INTEGER NOT NULL,
  tag_id INTEGER NOT NULL,
  PRIMARY KEY(book_id, tag_id)
);
CREATE INDEX IF NOT EXISTS idx_book_tags_tag ON book_tags(tag_id);
";

pub fn migrate(conn: &rusqlite::Connection) -> Result<(), String> {
    conn.execute_batch(TAGS_DDL).map_err(|e| e.to_string())?;
    let has: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('shelf') WHERE name='favorite'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has == 0 {
        crate::appdata::add_column(
            conn,
            "ALTER TABLE shelf ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0",
        )?;
    }
    Ok(())
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TagRow {
    pub id: i64,
    pub name: String,
    pub color: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookTagsMap {
    pub book_id: i64,
    pub tag_ids: Vec<i64>,
}

#[tauri::command]
pub fn list_tags() -> Result<Vec<TagRow>, String> {
    let conn = db_conn()?;
    let mut stmt = conn
        .prepare("SELECT id, name, color FROM tags ORDER BY name COLLATE NOCASE")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(TagRow {
                id: r.get(0)?,
                name: r.get(1)?,
                color: r.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn tag_create(name: String, color: Option<String>) -> Result<i64, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("标签名不能为空".into());
    }
    let conn = db_conn()?;
    let now = now_secs();
    let color = color.unwrap_or_else(|| "#80e5b6".to_string());
    conn.execute(
        "INSERT INTO tags(name,color,created_at) VALUES(?1,?2,?3)",
        params![name, color, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

#[tauri::command]
pub fn tag_rename(id: i64, name: String) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("标签名不能为空".into());
    }
    let conn = db_conn()?;
    conn.execute("UPDATE tags SET name=?2 WHERE id=?1", params![id, name])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn tag_delete(id: i64) -> Result<(), String> {
    let conn = db_conn()?;
    conn.execute("DELETE FROM book_tags WHERE tag_id=?1", [id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM tags WHERE id=?1", [id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn book_set_tags(book_id: i64, tag_ids: Vec<i64>) -> Result<(), String> {
    let mut conn = db_conn()?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM book_tags WHERE book_id=?1", [book_id])
        .map_err(|e| e.to_string())?;
    for tid in tag_ids {
        tx.execute(
            "INSERT OR IGNORE INTO book_tags(book_id,tag_id) VALUES(?1,?2)",
            params![book_id, tid],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn book_set_favorite(book_id: i64, favorite: bool) -> Result<(), String> {
    let conn = db_conn()?;
    conn.execute(
        "UPDATE shelf SET favorite=?2 WHERE id=?1",
        params![book_id, if favorite { 1 } else { 0 }],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn books_tags_bulk() -> Result<Vec<BookTagsMap>, String> {
    let conn = db_conn()?;
    let mut stmt = conn
        .prepare("SELECT book_id, tag_id FROM book_tags ORDER BY book_id, tag_id")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut out: Vec<BookTagsMap> = Vec::new();
    let mut cur_id = 0_i64;
    let mut cur_tags: Vec<i64> = Vec::new();
    for row in rows {
        let (bid, tid) = row.map_err(|e| e.to_string())?;
        if bid != cur_id {
            if cur_id != 0 {
                out.push(BookTagsMap {
                    book_id: cur_id,
                    tag_ids: cur_tags.clone(),
                });
            }
            cur_id = bid;
            cur_tags = vec![tid];
        } else {
            cur_tags.push(tid);
        }
    }
    if cur_id != 0 {
        out.push(BookTagsMap {
            book_id: cur_id,
            tag_ids: cur_tags,
        });
    }
    Ok(out)
}

#[tauri::command]
pub fn find_duplicate_books() -> Result<Vec<Vec<i64>>, String> {
    let conn = db_conn()?;
    let mut stmt = conn
        .prepare("SELECT id, lower(trim(name)), kind, paths FROM shelf ORDER BY name")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?))
        })
        .map_err(|e| e.to_string())?;
    use std::collections::HashMap;
    let mut by_key: HashMap<String, Vec<i64>> = HashMap::new();
    for row in rows {
        let (id, name, kind, paths) = row.map_err(|e| e.to_string())?;
        let key = format!("{name}|{kind}|{paths}");
        by_key.entry(key).or_default().push(id);
    }
    Ok(by_key
        .into_values()
        .filter(|v| v.len() > 1)
        .collect())
}
