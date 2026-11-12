//! Manual grouping rules.
//!
//! `merge_pin` is an exact membership and order chosen by the user. Auto-merge does not
//! write pins, so a later volume can still join a group nobody has rearranged.
//! `merge_apart` is a pair of folder path keys that must not share a book. Removing a
//! member, splitting a group, or keeping a pending group separate writes those pairs.
//! Rescans call `enforce_pins` and then drop auto groups that would cross an apart pair
//! or swallow a pinned path. Undo stores the shelf shape and the rules, not the files.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::merge::{apply_parts, auto_on, member_from_parts, propose_groups, store_pending, unmerge_conn};
use crate::shelf_scan::path_key;

const UNDO_CAP: i64 = 20;

#[derive(Clone, Debug)]
pub(crate) struct Rules {
    pub pins: HashMap<String, (String, i64)>,
    pub tokens: HashMap<String, Vec<(i64, String)>>,
    pub apart: HashSet<(String, String)>,
}

impl Rules {
    pub(crate) fn is_empty(&self) -> bool {
        self.pins.is_empty() && self.apart.is_empty()
    }

    pub(crate) fn pinned(&self, key: &str) -> bool {
        self.pins.contains_key(key)
    }

    pub(crate) fn apart(&self, a: &str, b: &str) -> bool {
        if a.is_empty() || b.is_empty() || a == b {
            return false;
        }
        let (a, b) = pair(a, b);
        self.apart.contains(&(a, b))
    }
}

pub(crate) fn load_rules(conn: &Connection) -> Result<Rules, String> {
    let mut pins = HashMap::new();
    let mut tokens: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    let mut stmt = conn
        .prepare("SELECT path_key, token, ord FROM merge_pin")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))
        .map_err(|e| e.to_string())?;
    for row in rows {
        let (key, token, ord) = row.map_err(|e| e.to_string())?;
        pins.insert(key.clone(), (token.clone(), ord));
        tokens.entry(token).or_default().push((ord, key));
    }
    let mut apart = HashSet::new();
    let mut stmt = conn
        .prepare("SELECT a, b FROM merge_apart")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    for row in rows {
        let (a, b) = row.map_err(|e| e.to_string())?;
        apart.insert((a, b));
    }
    Ok(Rules { pins, tokens, apart })
}

/// Indexes of members that auto-merge may still join. Pinned paths are left out
/// (enforce_pins already placed them). Apart pairs are split, and clusters of one are dropped.
pub(crate) fn auto_clusters(member_keys: &[Vec<String>], rules: &Rules) -> Vec<Vec<usize>> {
    let mut buckets: Vec<Vec<usize>> = Vec::new();
    for i in 0..member_keys.len() {
        if member_keys[i].iter().any(|k| rules.pinned(k)) {
            continue;
        }
        let mut placed = None;
        for (b, bucket) in buckets.iter().enumerate() {
            let clash = bucket.iter().any(|&j| keys_apart(&member_keys[i], &member_keys[j], rules));
            if !clash {
                placed = Some(b);
                break;
            }
        }
        if let Some(b) = placed {
            buckets[b].push(i);
        } else {
            buckets.push(vec![i]);
        }
    }
    buckets.into_iter().filter(|b| b.len() >= 2).collect()
}

fn keys_apart(a: &[String], b: &[String], rules: &Rules) -> bool {
    a.iter().any(|x| b.iter().any(|y| rules.apart(x, y)))
}

fn pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

fn norm_keys(paths: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for p in paths {
        let k = path_key(p);
        if !k.is_empty() && !out.contains(&k) {
            out.push(k);
        }
    }
    out
}

pub(crate) fn pin_exact(conn: &Connection, paths: &[String]) -> Result<(), String> {
    let keys = norm_keys(paths);
    if keys.len() < 2 {
        return Ok(());
    }
    clear_pin_tokens(conn, &keys)?;
    clear_apart_among(conn, &keys)?;
    let token = new_token();
    for (i, key) in keys.iter().enumerate() {
        conn.execute(
            "INSERT INTO merge_pin(path_key, token, ord) VALUES(?1,?2,?3)",
            params![key, token, i as i64],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(crate) fn apart_between_paths(conn: &Connection, left: &[String], right: &[String]) -> Result<(), String> {
    apart_between(conn, &norm_keys(left), &norm_keys(right))
}

/// Keep these paths from being auto-grouped with each other. Pins on other books stay.
pub(crate) fn apart_among(conn: &Connection, paths: &[String]) -> Result<(), String> {
    let keys = norm_keys(paths);
    for i in 0..keys.len() {
        for j in (i + 1)..keys.len() {
            apart_between(conn, &[keys[i].clone()], &[keys[j].clone()])?;
        }
    }
    Ok(())
}

pub(crate) fn apart_all(conn: &Connection, paths: &[String]) -> Result<(), String> {
    let keys = norm_keys(paths);
    clear_pin_tokens(conn, &keys)?;
    for i in 0..keys.len() {
        for j in (i + 1)..keys.len() {
            let (a, b) = pair(&keys[i], &keys[j]);
            conn.execute(
                "INSERT INTO merge_apart(a, b) VALUES(?1,?2) ON CONFLICT(a, b) DO NOTHING",
                params![a, b],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn clear_apart_among(conn: &Connection, keys: &[String]) -> Result<(), String> {
    for i in 0..keys.len() {
        for j in (i + 1)..keys.len() {
            let (a, b) = pair(&keys[i], &keys[j]);
            conn.execute("DELETE FROM merge_apart WHERE a=?1 AND b=?2", params![a, b])
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn clear_pin_tokens(conn: &Connection, keys: &[String]) -> Result<(), String> {
    if keys.is_empty() {
        return Ok(());
    }
    let mut tokens = Vec::new();
    for key in keys {
        if let Some(token) = conn
            .query_row("SELECT token FROM merge_pin WHERE path_key=?1", [key], |r| r.get::<_, String>(0))
            .optional()
            .map_err(|e| e.to_string())?
        {
            if !tokens.contains(&token) {
                tokens.push(token);
            }
        }
    }
    for token in tokens {
        conn.execute("DELETE FROM merge_pin WHERE token=?1", [token])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn clear_rules(conn: &Connection, keys: &[String]) -> Result<(), String> {
    clear_pin_tokens(conn, keys)?;
    for key in keys {
        conn.execute("DELETE FROM merge_apart WHERE a=?1 OR b=?1", [key])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn new_token() -> String {
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("p{n}-{}", std::process::id())
}

#[derive(Clone, Debug)]
struct FolderRow {
    id: i64,
    name: String,
    paths: Vec<String>,
    pages: i64,
    folder_id: Option<i64>,
    merged: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct UndoPart {
    name: String,
    paths: Vec<String>,
    pages: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct UndoBook {
    title: String,
    folder_id: Option<i64>,
    parts: Vec<UndoPart>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PinRow {
    path_key: String,
    token: String,
    ord: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PendingSnap {
    id: i64,
    resolved: i64,
    payload: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct UndoBody {
    books: Vec<UndoBook>,
    pins: Vec<PinRow>,
    apart: Vec<(String, String)>,
    /// Open or resolved suggestion rows that mentioned these paths, so undo can put a
    /// detached book back into the same review group.
    #[serde(default)]
    pending: Vec<PendingSnap>,
}

#[derive(Clone, Debug)]
struct Part {
    name: String,
    paths: Vec<String>,
    pages: i64,
}

struct Planned {
    title: String,
    folder_id: Option<i64>,
    parts: Vec<Part>,
}

fn folder_rows(conn: &Connection) -> Result<Vec<FolderRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, paths, page_count, folder_id,
                    EXISTS(SELECT 1 FROM book_merges m WHERE m.book_id = shelf.id AND m.undone=0)
             FROM shelf WHERE kind='folder'",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows {
        let (id, name, paths, pages, folder_id, merged) = row.map_err(|e| e.to_string())?;
        let paths: Vec<String> = serde_json::from_str(&paths).unwrap_or_default();
        out.push(FolderRow {
            id,
            name,
            paths,
            pages,
            folder_id,
            merged: merged != 0,
        });
    }
    Ok(out)
}

fn book_row(conn: &Connection, id: i64) -> Result<FolderRow, String> {
    folder_rows(conn)?
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| "书架上没有这本文件夹漫画".to_string())
}

fn parts_of(conn: &Connection, id: i64) -> Result<(String, Option<i64>, Vec<Part>), String> {
    let row = book_row(conn, id)?;
    let merge_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM book_merges WHERE book_id=?1 AND undone=0 ORDER BY id DESC LIMIT 1",
            [id],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some(mid) = merge_id {
        let mut stmt = conn
            .prepare("SELECT name, paths, page_count FROM book_merge_parts WHERE merge_id=?1 ORDER BY ord")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([mid], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))
            .map_err(|e| e.to_string())?;
        let mut parts = Vec::new();
        for row in rows {
            let (name, paths, pages) = row.map_err(|e| e.to_string())?;
            let paths: Vec<String> = serde_json::from_str(&paths).unwrap_or_default();
            if paths.is_empty() {
                continue;
            }
            parts.push(Part { name, paths, pages });
        }
        if parts.len() < 2 {
            return Err("合并记录不完整".into());
        }
        return Ok((row.name, row.folder_id, parts));
    }
    if row.paths.is_empty() {
        return Err("这本没有文件夹路径".into());
    }
    Ok((
        row.name.clone(),
        row.folder_id,
        vec![Part {
            name: row.name,
            paths: row.paths,
            pages: row.pages,
        }],
    ))
}

fn part_keys(part: &Part) -> Vec<String> {
    norm_keys(&part.paths)
}

fn part_key(part: &Part) -> String {
    part_keys(part).into_iter().next().unwrap_or_default()
}

fn rows_touching(rows: &[FolderRow], keys: &HashSet<String>) -> Vec<FolderRow> {
    rows.iter()
        .filter(|r| r.paths.iter().any(|p| keys.contains(&path_key(p))))
        .cloned()
        .collect()
}

fn closure(rules: &Rules, seeds: &[String]) -> Vec<String> {
    let mut set: HashSet<String> = seeds.iter().filter(|s| !s.is_empty()).cloned().collect();
    loop {
        let n = set.len();
        let tokens: Vec<String> = set
            .iter()
            .filter_map(|k| rules.pins.get(k).map(|(t, _)| t.clone()))
            .collect();
        for (key, (token, _)) in &rules.pins {
            if tokens.iter().any(|t| t == token) {
                set.insert(key.clone());
            }
        }
        for (a, b) in &rules.apart {
            if set.contains(a) {
                set.insert(b.clone());
            } else if set.contains(b) {
                set.insert(a.clone());
            }
        }
        if set.len() == n {
            break;
        }
    }
    let mut out: Vec<String> = set.into_iter().collect();
    out.sort();
    out
}

fn books_touching(conn: &Connection, keys: &[String]) -> Result<Vec<UndoBook>, String> {
    let want: HashSet<String> = keys.iter().cloned().collect();
    let rows = folder_rows(conn)?;
    let mut books = Vec::new();
    for row in rows_touching(&rows, &want) {
        let (_, folder_id, parts) = parts_of(conn, row.id)?;
        books.push(UndoBook {
            title: row.name,
            folder_id,
            parts: parts
                .into_iter()
                .map(|p| UndoPart {
                    name: p.name,
                    paths: p.paths,
                    pages: p.pages,
                })
                .collect(),
        });
    }
    Ok(books)
}

pub(crate) fn push_undo(conn: &Connection, paths: &[String]) -> Result<(), String> {
    let rules = load_rules(conn)?;
    let keys = closure(&rules, &norm_keys(paths));
    if keys.is_empty() {
        return Ok(());
    }
    let books = books_touching(conn, &keys)?;
    let keyset: HashSet<&str> = keys.iter().map(|s| s.as_str()).collect();
    let pins = rules
        .pins
        .iter()
        .filter(|(k, _)| keyset.contains(k.as_str()))
        .map(|(k, (token, ord))| PinRow {
            path_key: k.clone(),
            token: token.clone(),
            ord: *ord,
        })
        .collect();
    let apart = rules
        .apart
        .iter()
        .filter(|(a, b)| keyset.contains(a.as_str()) || keyset.contains(b.as_str()))
        .cloned()
        .collect();
    let pending = pending_snaps(conn, &keys);
    let body = serde_json::to_string(&UndoBody { books, pins, apart, pending }).map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO merge_undo(created_at, body) VALUES(?1,?2)",
        params![crate::now_secs(), body],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM merge_undo WHERE id NOT IN (SELECT id FROM merge_undo ORDER BY id DESC LIMIT ?1)",
        [UNDO_CAP],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn pending_snaps(conn: &Connection, keys: &[String]) -> Vec<PendingSnap> {
    let want: HashSet<&str> = keys.iter().map(|s| s.as_str()).collect();
    if want.is_empty() {
        return Vec::new();
    }
    let Ok(mut stmt) = conn.prepare("SELECT id, resolved, payload FROM merge_pending") else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?))) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for row in rows.flatten() {
        let (id, resolved, payload) = row;
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload) else {
            continue;
        };
        let hit = value
            .get("members")
            .and_then(|m| m.as_array())
            .map(|members| {
                members.iter().any(|member| {
                    member
                        .get("path")
                        .and_then(|p| p.as_str())
                        .map(|p| want.contains(path_key(p).as_str()))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false);
        if hit {
            out.push(PendingSnap { id, resolved, payload });
        }
    }
    out
}

fn path_set(paths: &[String]) -> HashSet<String> {
    paths.iter().map(|p| path_key(p)).filter(|k| !k.is_empty()).collect()
}

fn row_for_part<'a>(rows: &'a [FolderRow], part: &Part) -> Result<&'a FolderRow, String> {
    let want = path_set(&part.paths);
    rows.iter()
        .find(|r| path_set(&r.paths) == want)
        .ok_or_else(|| "分组里的文件夹已经不在书架上".to_string())
}

fn rematerialize(conn: &Connection, groups: &[Planned]) -> Result<Vec<i64>, String> {
    let mut keys = HashSet::new();
    for g in groups {
        for part in &g.parts {
            for p in &part.paths {
                let k = path_key(p);
                if !k.is_empty() {
                    keys.insert(k);
                }
            }
        }
    }
    let rows = folder_rows(conn)?;
    let mut merged_ids = Vec::new();
    for row in rows_touching(&rows, &keys) {
        if row.merged && !merged_ids.contains(&row.id) {
            merged_ids.push(row.id);
        }
    }
    for id in merged_ids {
        unmerge_conn(conn, id)?;
    }
    let rows = folder_rows(conn)?;
    let mut ids = Vec::new();
    for g in groups {
        if g.parts.len() < 2 {
            if let Some(part) = g.parts.first() {
                if let Ok(row) = row_for_part(&rows, part) {
                    ids.push(row.id);
                }
            }
            continue;
        }
        let mut members = Vec::new();
        for part in &g.parts {
            let row = row_for_part(&rows, part)?;
            members.push(member_from_parts(row.id, &part.name, row.paths.clone(), part.pages.max(row.pages)));
        }
        let id = apply_parts(conn, &g.title, g.folder_id, &members, false)?;
        ids.push(id);
    }
    Ok(ids)
}

fn apart_between(conn: &Connection, left: &[String], right: &[String]) -> Result<(), String> {
    for a in left {
        for b in right {
            if a == b {
                continue;
            }
            let (a, b) = pair(a, b);
            conn.execute(
                "INSERT INTO merge_apart(a, b) VALUES(?1,?2) ON CONFLICT(a, b) DO NOTHING",
                params![a, b],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupPartView {
    pub key: String,
    pub name: String,
    pub paths: Vec<String>,
    pub pages: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupView {
    pub book_id: i64,
    pub title: String,
    pub manual: bool,
    pub merged: bool,
    pub parts: Vec<GroupPartView>,
    pub can_undo: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupEditResult {
    pub book_ids: Vec<i64>,
    pub message: String,
}

fn view_parts(parts: &[Part]) -> Vec<GroupPartView> {
    parts
        .iter()
        .map(|p| GroupPartView {
            key: part_key(p),
            name: p.name.clone(),
            paths: p.paths.clone(),
            pages: p.pages,
        })
        .collect()
}

fn manual_for(rules: &Rules, parts: &[Part]) -> bool {
    let keys: Vec<String> = parts.iter().flat_map(|p| part_keys(p)).collect();
    keys.iter().any(|k| rules.pinned(k)) || keys.iter().any(|a| keys.iter().any(|b| rules.apart(a, b)))
}

#[tauri::command]
pub fn group_describe(book_id: i64) -> Result<GroupView, String> {
    let conn = crate::db_conn()?;
    let (title, _, parts) = parts_of(&conn, book_id)?;
    let rules = load_rules(&conn)?;
    let can_undo: i64 = conn
        .query_row("SELECT COUNT(*) FROM merge_undo", [], |r| r.get(0))
        .unwrap_or(0);
    let merged = parts.len() >= 2;
    Ok(GroupView {
        book_id,
        title,
        manual: manual_for(&rules, &parts),
        merged,
        parts: view_parts(&parts),
        can_undo: can_undo > 0,
    })
}

fn select_parts(parts: &[Part], keys: &[String]) -> Result<Vec<Part>, String> {
    let want: HashSet<String> = keys.iter().cloned().collect();
    let mut out = Vec::new();
    for key in keys {
        if out.iter().any(|p: &Part| part_key(p) == *key) {
            continue;
        }
        let part = parts
            .iter()
            .find(|p| part_keys(p).iter().any(|k| k == key) || part_key(p) == *key)
            .ok_or_else(|| "没有这一册".to_string())?;
        if !want.is_empty() {
            out.push(part.clone());
        }
    }
    Ok(out)
}

#[tauri::command]
pub fn group_remove_members(book_id: i64, path_keys: Vec<String>) -> Result<GroupEditResult, String> {
    let conn = crate::db_conn()?;
    remove_members_conn(&conn, book_id, path_keys)
}

pub(crate) fn remove_members_conn(
    conn: &Connection,
    book_id: i64,
    path_keys: Vec<String>,
) -> Result<GroupEditResult, String> {
    let (title, folder_id, parts) = parts_of(&conn, book_id)?;
    if parts.len() < 2 {
        return Err("这本还没有分册可移出".into());
    }
    let drop_keys: HashSet<String> = path_keys.into_iter().collect();
    if drop_keys.is_empty() {
        return Err("请选择要移出的分册".into());
    }
    let mut stay = Vec::new();
    let mut gone = Vec::new();
    for part in parts {
        if part_keys(&part).iter().any(|k| drop_keys.contains(k)) {
            gone.push(part);
        } else {
            stay.push(part);
        }
    }
    if gone.is_empty() {
        return Err("没有这一册".into());
    }
    if stay.is_empty() {
        return Err("请至少留一册，或改用拆组".into());
    }
    let all: Vec<String> = stay
        .iter()
        .chain(gone.iter())
        .flat_map(|p| p.paths.clone())
        .collect();
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    push_undo(&tx, &all)?;
    let mut planned = Vec::new();
    if stay.len() >= 2 {
        planned.push(Planned {
            title: title.clone(),
            folder_id,
            parts: stay.clone(),
        });
    }
    for part in &gone {
        planned.push(Planned {
            title: part.name.clone(),
            folder_id,
            parts: vec![part.clone()],
        });
    }
    if stay.len() == 1 {
        planned.push(Planned {
            title: stay[0].name.clone(),
            folder_id,
            parts: stay.clone(),
        });
    }
    let ids = rematerialize(&tx, &planned)?;
    let stay_keys: Vec<String> = stay.iter().flat_map(|p| part_keys(p)).collect();
    let gone_keys: Vec<String> = gone.iter().flat_map(|p| part_keys(p)).collect();
    let rules = load_rules(&tx)?;
    let was_pinned = stay_keys.iter().chain(gone_keys.iter()).any(|k| rules.pinned(k));
    if stay.len() >= 2 && was_pinned {
        pin_exact(&tx, &stay.iter().flat_map(|p| p.paths.clone()).collect::<Vec<_>>())?;
    } else if stay.len() >= 2 {
        clear_pin_tokens(&tx, &stay_keys)?;
    }
    apart_between(&tx, &stay_keys, &gone_keys)?;
    apart_all_pairs_only(&tx, &gone_keys)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(GroupEditResult {
        book_ids: ids,
        message: "已移出。漫画文件还在原来的文件夹里。".into(),
    })
}

fn apart_all_pairs_only(conn: &Connection, keys: &[String]) -> Result<(), String> {
    for i in 0..keys.len() {
        for j in (i + 1)..keys.len() {
            let (a, b) = pair(&keys[i], &keys[j]);
            conn.execute(
                "INSERT INTO merge_apart(a, b) VALUES(?1,?2) ON CONFLICT(a, b) DO NOTHING",
                params![a, b],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn group_reorder(book_id: i64, path_keys: Vec<String>) -> Result<GroupEditResult, String> {
    let conn = crate::db_conn()?;
    let (title, folder_id, parts) = parts_of(&conn, book_id)?;
    if parts.len() < 2 {
        return Err("至少两册才能调整顺序".into());
    }
    let ordered = select_parts(&parts, &path_keys)?;
    if ordered.len() != parts.len() {
        return Err("顺序里要包含全部分册".into());
    }
    let paths: Vec<String> = ordered.iter().flat_map(|p| p.paths.clone()).collect();
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    push_undo(&tx, &paths)?;
    let ids = rematerialize(
        &tx,
        &[Planned {
            title,
            folder_id,
            parts: ordered,
        }],
    )?;
    pin_exact(&tx, &paths)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(GroupEditResult {
        book_ids: ids,
        message: "已按这个顺序保存。之后重新扫描也会保持。".into(),
    })
}

#[tauri::command]
pub fn group_move_member(path_keys: Vec<String>, to_book_id: i64) -> Result<GroupEditResult, String> {
    let conn = crate::db_conn()?;
    let rows = folder_rows(&conn)?;
    let want: HashSet<String> = path_keys.iter().cloned().collect();
    let source = rows
        .iter()
        .find(|r| r.paths.iter().any(|p| want.contains(&path_key(p))) || parts_hit(&conn, r.id, &want).unwrap_or(false))
        .ok_or_else(|| "没有找到要移动的分册".to_string())?;
    let (src_title, src_folder, src_parts) = parts_of(&conn, source.id)?;
    let moving: Vec<Part> = src_parts
        .iter()
        .filter(|p| part_keys(p).iter().any(|k| want.contains(k)))
        .cloned()
        .collect();
    if moving.is_empty() {
        return Err("没有这一册".into());
    }
    if source.id == to_book_id {
        return Err("已经在这个分组里".into());
    }
    let (dst_title, dst_folder, mut dst_parts) = parts_of(&conn, to_book_id)?;
    let stay: Vec<Part> = src_parts
        .iter()
        .filter(|p| !part_keys(p).iter().any(|k| want.contains(k)))
        .cloned()
        .collect();
    dst_parts.extend(moving.iter().cloned());
    let all: Vec<String> = src_parts.iter().chain(dst_parts.iter()).flat_map(|p| p.paths.clone()).collect();
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    push_undo(&tx, &all)?;
    let mut planned = vec![Planned {
        title: dst_title,
        folder_id: dst_folder,
        parts: dst_parts.clone(),
    }];
    if stay.len() >= 2 {
        planned.push(Planned {
            title: src_title,
            folder_id: src_folder,
            parts: stay.clone(),
        });
    } else if let Some(part) = stay.first() {
        planned.push(Planned {
            title: part.name.clone(),
            folder_id: src_folder,
            parts: vec![part.clone()],
        });
    }
    let ids = rematerialize(&tx, &planned)?;
    pin_exact(&tx, &dst_parts.iter().flat_map(|p| p.paths.clone()).collect::<Vec<_>>())?;
    if stay.len() >= 2 {
        pin_exact(&tx, &stay.iter().flat_map(|p| p.paths.clone()).collect::<Vec<_>>())?;
    }
    let stay_keys: Vec<String> = stay.iter().flat_map(|p| part_keys(p)).collect();
    let move_keys: Vec<String> = moving.iter().flat_map(|p| part_keys(p)).collect();
    apart_between(&tx, &stay_keys, &move_keys)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(GroupEditResult {
        book_ids: ids,
        message: "已移到另一个分组。文件没有搬动。".into(),
    })
}

fn parts_hit(conn: &Connection, id: i64, want: &HashSet<String>) -> Result<bool, String> {
    let (_, _, parts) = parts_of(conn, id)?;
    Ok(parts.iter().any(|p| part_keys(p).iter().any(|k| want.contains(k))))
}

#[tauri::command]
pub fn group_add_books(book_id: i64, book_ids: Vec<i64>) -> Result<GroupEditResult, String> {
    let conn = crate::db_conn()?;
    let (title, folder_id, mut parts) = parts_of(&conn, book_id)?;
    if book_ids.is_empty() {
        return Err("请选择要加入的书".into());
    }
    for id in &book_ids {
        if *id == book_id {
            continue;
        }
        let (_, _, extra) = parts_of(&conn, *id)?;
        parts.extend(extra);
    }
    if parts.len() < 2 {
        return Err("至少两本才能组成一组".into());
    }
    let paths: Vec<String> = parts.iter().flat_map(|p| p.paths.clone()).collect();
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    push_undo(&tx, &paths)?;
    let ids = rematerialize(
        &tx,
        &[Planned {
            title,
            folder_id,
            parts,
        }],
    )?;
    pin_exact(&tx, &paths)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(GroupEditResult {
        book_ids: ids,
        message: "已加入这个分组。文件还在原处。".into(),
    })
}

#[tauri::command]
pub fn group_split(book_id: i64, groups: Vec<Vec<String>>) -> Result<GroupEditResult, String> {
    let conn = crate::db_conn()?;
    let (title, folder_id, parts) = parts_of(&conn, book_id)?;
    if parts.len() < 2 {
        return Err("这本还没有可以拆开的分册".into());
    }
    if groups.is_empty() {
        return Err("请指定拆开后的分组".into());
    }
    let mut used = HashSet::new();
    let mut planned_parts = Vec::new();
    for g in &groups {
        let chunk = select_parts(&parts, g)?;
        if chunk.is_empty() {
            return Err("拆分组里没有分册".into());
        }
        for part in &chunk {
            let key = part_key(part);
            if !used.insert(key) {
                return Err("同一册不能同时放进两个分组".into());
            }
        }
        planned_parts.push(chunk);
    }
    if used.len() != parts.len() {
        return Err("拆分时要列出全部分册".into());
    }
    let paths: Vec<String> = parts.iter().flat_map(|p| p.paths.clone()).collect();
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    push_undo(&tx, &paths)?;
    let planned: Vec<Planned> = planned_parts
        .iter()
        .map(|chunk| Planned {
            title: if chunk.len() == parts.len() {
                title.clone()
            } else if chunk.len() == 1 {
                chunk[0].name.clone()
            } else {
                title.clone()
            },
            folder_id,
            parts: chunk.clone(),
        })
        .collect();
    let ids = rematerialize(&tx, &planned)?;
    let mut all_keys = Vec::new();
    for chunk in &planned_parts {
        let keys: Vec<String> = chunk.iter().flat_map(|p| part_keys(p)).collect();
        all_keys.extend(keys.iter().cloned());
        if chunk.len() >= 2 {
            pin_exact(&tx, &chunk.iter().flat_map(|p| p.paths.clone()).collect::<Vec<_>>())?;
        }
    }
    for i in 0..planned_parts.len() {
        for j in (i + 1)..planned_parts.len() {
            let a: Vec<String> = planned_parts[i].iter().flat_map(|p| part_keys(p)).collect();
            let b: Vec<String> = planned_parts[j].iter().flat_map(|p| part_keys(p)).collect();
            apart_between(&tx, &a, &b)?;
        }
    }
    let _ = all_keys;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(GroupEditResult {
        book_ids: ids,
        message: "已拆开。磁盘上的文件都还在。".into(),
    })
}

#[tauri::command]
pub fn group_undo() -> Result<GroupEditResult, String> {
    let conn = crate::db_conn()?;
    undo_last_conn(&conn)
}

pub(crate) fn undo_last_conn(conn: &Connection) -> Result<GroupEditResult, String> {
    let row: Option<(i64, String)> = conn
        .query_row(
            "SELECT id, body FROM merge_undo ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    let Some((id, body)) = row else {
        return Err("没有可以撤销的分组操作".into());
    };
    let snap: UndoBody = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let planned: Vec<Planned> = snap
        .books
        .iter()
        .map(|b| Planned {
            title: b.title.clone(),
            folder_id: b.folder_id,
            parts: b
                .parts
                .iter()
                .map(|p| Part {
                    name: p.name.clone(),
                    paths: p.paths.clone(),
                    pages: p.pages,
                })
                .collect(),
        })
        .collect();
    let ids = rematerialize(&tx, &planned)?;
    let keys: Vec<String> = snap
        .books
        .iter()
        .flat_map(|b| b.parts.iter().flat_map(|p| norm_keys(&p.paths)))
        .collect();
    clear_rules(&tx, &keys)?;
    for pin in &snap.pins {
        conn.execute(
            "INSERT INTO merge_pin(path_key, token, ord) VALUES(?1,?2,?3)",
            params![pin.path_key, pin.token, pin.ord],
        )
        .map_err(|e| e.to_string())?;
    }
    for (a, b) in &snap.apart {
        conn.execute(
            "INSERT INTO merge_apart(a, b) VALUES(?1,?2) ON CONFLICT(a, b) DO NOTHING",
            params![a, b],
        )
        .map_err(|e| e.to_string())?;
    }
    for row in &snap.pending {
        let updated = tx
            .execute(
                "UPDATE merge_pending SET payload=?1, resolved=?2 WHERE id=?3",
                params![row.payload, row.resolved, row.id],
            )
            .map_err(|e| e.to_string())?;
        if updated == 0 {
            tx.execute(
                "INSERT INTO merge_pending(id, created_at, resolved, payload) VALUES(?1,?2,?3,?4)",
                params![row.id, crate::now_secs(), row.resolved, row.payload],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    tx.execute("DELETE FROM merge_undo WHERE id=?1", [id])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(GroupEditResult {
        book_ids: ids,
        message: "已撤销上一次分组调整。".into(),
    })
}

#[tauri::command]
pub fn group_restore_auto(book_id: i64) -> Result<GroupEditResult, String> {
    let conn = crate::db_conn()?;
    let (_, _, parts) = parts_of(&conn, book_id)?;
    let rules = load_rules(&conn)?;
    let seeds: Vec<String> = parts.iter().flat_map(|p| part_keys(p)).collect();
    let keys = closure(&rules, &seeds);
    let books = books_touching(&conn, &keys)?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    push_undo(&tx, &keys)?;
    clear_rules(&tx, &keys)?;
    let keyset: HashSet<String> = keys.iter().cloned().collect();
    let merged_ids: Vec<i64> = rows_touching(&folder_rows(&tx)?, &keyset)
        .into_iter()
        .filter(|r| r.merged)
        .map(|r| r.id)
        .collect();
    for id in merged_ids {
        unmerge_conn(&tx, id)?;
    }
    let rows = folder_rows(&tx)?;
    let mut fresh = Vec::new();
    let mut seen = HashSet::new();
    for book in &books {
        for part in &book.parts {
            if let Some(row) = rows.iter().find(|r| path_set(&r.paths) == path_set(&part.paths)) {
                if seen.insert(row.id) {
                    fresh.push(member_from_parts(row.id, &part.name, row.paths.clone(), part.pages));
                }
            }
        }
    }
    if fresh.is_empty() {
        return Err("没有可以重新检测的文件夹".into());
    }
    let groups = propose_groups(&fresh);
    let auto = auto_on(&tx);
    let mut ids = Vec::new();
    let mut pending = Vec::new();
    for g in groups {
        if g.confident && auto && g.members.len() >= 2 {
            ids.push(apply_parts(&tx, &g.title, None, &g.members, true)?);
        } else if g.members.len() >= 2 {
            pending.push(g);
        }
    }
    if !pending.is_empty() {
        store_pending(&tx, &pending, false)?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(GroupEditResult {
        book_ids: ids,
        message: "已恢复自动检测。手动规则已清掉，文件没有删除。".into(),
    })
}

pub(crate) fn unmerge_keeping_apart(conn: &Connection, book_id: i64) -> Result<(), String> {
    let (_, _, parts) = parts_of(conn, book_id)?;
    let paths: Vec<String> = parts.iter().flat_map(|p| p.paths.clone()).collect();
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    push_undo(&tx, &paths)?;
    apart_all(&tx, &paths)?;
    unmerge_conn(&tx, book_id)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

pub(crate) fn enforce_pins(conn: &Connection) -> Result<(), String> {
    let rules = load_rules(conn)?;
    if rules.tokens.is_empty() {
        return Ok(());
    }
    for entries in rules.tokens.values() {
        let mut entries = entries.clone();
        entries.sort_by_key(|(ord, _)| *ord);
        let rows = folder_rows(conn)?;
        let mut paths = Vec::new();
        for (_, key) in &entries {
            if let Some(p) = rows.iter().flat_map(|r| r.paths.iter()).find(|p| path_key(p) == *key) {
                paths.push(p.clone());
            }
        }
        if paths.len() < 2 {
            continue;
        }
        let keyset: HashSet<String> = paths.iter().map(|p| path_key(p)).collect();
        let owners = rows_touching(&rows, &keyset);
        let exact = owners.len() == 1
            && owners[0].paths.len() == paths.len()
            && owners[0].paths.iter().map(|p| path_key(p)).eq(paths.iter().map(|p| path_key(p)));
        if exact {
            continue;
        }
        let title = owners
            .iter()
            .max_by_key(|r| r.paths.iter().filter(|p| keyset.contains(&path_key(p))).count())
            .map(|r| r.name.clone())
            .unwrap_or_else(|| "合并".into());
        let folder_id = owners.first().and_then(|r| r.folder_id);
        let parts = paths
            .iter()
            .map(|p| {
                let row = owners.iter().find(|r| r.paths.iter().any(|x| path_key(x) == path_key(p)));
                Part {
                    name: row.map(|r| r.name.clone()).unwrap_or_else(|| p.clone()),
                    paths: vec![p.clone()],
                    pages: row.map(|r| r.pages).unwrap_or(0),
                }
            })
            .collect();
        rematerialize(
            conn,
            &[Planned {
                title,
                folder_id,
                parts,
            }],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apart_pair_is_not_auto_joined_and_pinned_paths_are_skipped() {
        let rules = Rules {
            pins: HashMap::from([("b".into(), ("t".into(), 0))]),
            tokens: HashMap::new(),
            apart: HashSet::from([("a".into(), "c".into())]),
        };
        let keys = vec![vec!["a".into()], vec!["c".into()], vec!["d".into()], vec!["b".into()]];
        let clusters = auto_clusters(&keys, &rules);
        assert!(clusters.iter().all(|c| !c.contains(&3)), "pinned member stays out");
        assert!(
            clusters.iter().all(|c| !(c.contains(&0) && c.contains(&1))),
            "apart pair stays apart: {clusters:?}"
        );
        assert!(clusters.iter().any(|c| c.contains(&0) && c.contains(&2)) || clusters.iter().any(|c| c.contains(&1) && c.contains(&2)));
    }
}
