//! Scan one parent folder into books.
//!
//! Each immediate subfolder that contains images (in that folder or anywhere
//! under it) is one book, titled with the subfolder's name. Folders under the
//! book become chapters, in the same order the reader uses: pages are sorted
//! with [`compare_comic_names`], and a chapter is the parent directory of a
//! page. Two or more such directories are required, matching `scan::chapters`.
//!
//! Images are not opened. Archives, hidden directories, and images sitting
//! directly in the picked folder are not books. Whole-drive scans and system
//! directories are refused or left for a later pass (see `is_drive_root`).

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, DirEntry};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tauri::State;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanChapter {
    pub title: String,
    pub pages: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanBook {
    pub title: String,
    pub path: String,
    pub pages: u64,
    pub chapters: Vec<ScanChapter>,
    pub already: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ScanIssue {
    pub path: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScanSnapshot {
    pub phase: String,
    pub root: String,
    pub folders_scanned: u64,
    pub books_found: u64,
    pub loose_pages: u64,
    pub files_seen: u64,
    pub current_path: String,
    pub books: Vec<ScanBook>,
    pub error: Option<String>,
    /// True when this run continued a cancelled or interrupted scan of the same folder.
    pub resumed: bool,
    pub issue_count: u64,
    /// The latest problems (capped). The full set is in `scan_issues`.
    pub issues: Vec<ScanIssue>,
}

impl ScanSnapshot {
    fn running(root: &Path) -> Self {
        Self {
            phase: "running".into(),
            root: root.to_string_lossy().to_string(),
            folders_scanned: 0,
            books_found: 0,
            loose_pages: 0,
            files_seen: 0,
            current_path: String::new(),
            books: Vec::new(),
            error: None,
            resumed: false,
            issue_count: 0,
            issues: Vec::new(),
        }
    }
}

struct Job {
    cancel: Arc<AtomicBool>,
    snap: Arc<Mutex<ScanSnapshot>>,
}

struct Shared {
    /// Serialises checkpoint writes so a new scan cannot load a half-written row.
    db: Mutex<()>,
    epoch: AtomicU64,
}

pub struct ScanHost {
    job: Mutex<Option<Arc<Job>>>,
    shared: Arc<Shared>,
}

impl Default for ScanHost {
    fn default() -> Self {
        Self {
            job: Mutex::new(None),
            shared: Arc::new(Shared {
                db: Mutex::new(()),
                epoch: AtomicU64::new(0),
            }),
        }
    }
}

// ---- natural order (same rules as src/naming.js compareNames) -----------------------

fn nfkc_filename(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\u{3000}' => ' ',
            c if ('\u{ff01}'..='\u{ff5e}').contains(&c) => {
                char::from_u32(c as u32 - 0xfee0).unwrap_or(c)
            }
            c => c,
        })
        .collect()
}

fn is_cn_numeral(c: char) -> bool {
    matches!(
        c,
        '零' | '〇' | '一' | '二' | '两' | '三' | '四' | '五' | '六' | '七' | '八' | '九' | '十' | '百' | '千'
    )
}

fn is_counter(c: char) -> bool {
    matches!(c, '话' | '話' | '章' | '卷' | '集' | '回' | '部' | '期' | '节' | '節' | '篇')
}

fn cn_digit(c: char) -> Option<u32> {
    Some(match c {
        '零' | '〇' => 0,
        '一' => 1,
        '二' | '两' => 2,
        '三' => 3,
        '四' => 4,
        '五' => 5,
        '六' => 6,
        '七' => 7,
        '八' => 8,
        '九' => 9,
        _ => return None,
    })
}

/// Value of a Chinese integer made of 零〇一二两三四五六七八九十百千. `None` if it isn't one.
fn chinese_number(chars: &[char]) -> Option<u64> {
    if chars.is_empty() {
        return None;
    }
    if chars.iter().all(|c| c.is_ascii_digit()) {
        return chars.iter().collect::<String>().parse().ok();
    }
    let mut section: u64 = 0;
    let mut num: u64 = 0;
    for &c in chars {
        if let Some(d) = cn_digit(c) {
            num = d as u64;
        } else if c == '十' {
            section += num.max(1) * 10;
            num = 0;
        } else if c == '百' {
            section += num.max(1) * 100;
            num = 0;
        } else if c == '千' {
            section += num.max(1) * 1000;
            num = 0;
        } else {
            return None;
        }
    }
    Some(section + num)
}

/// 第十一话 -> 第11话. Other text, including 《二十四孝》, is left alone.
pub fn numerals_to_digits(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '第' {
            let start = i + 1;
            let mut j = start;
            while j < chars.len() && is_cn_numeral(chars[j]) {
                j += 1;
            }
            if j > start && j < chars.len() && is_counter(chars[j]) {
                if let Some(n) = chinese_number(&chars[start..j]) {
                    out.push('第');
                    out.push_str(&n.to_string());
                    i = j;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn pad_digits(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let run: String = chars[start..i].iter().collect();
            if run.len() < 12 {
                out.push_str(&"0".repeat(12 - run.len()));
            }
            out.push_str(&run);
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn sort_key(s: &str) -> String {
    pad_digits(&numerals_to_digits(&nfkc_filename(s)).to_lowercase())
}

/// Comic filename order: 2 before 10, 01 beside 1, 第一话 before 第2话 before 第10话.
pub fn compare_comic_names(a: &str, b: &str) -> std::cmp::Ordering {
    sort_key(a).cmp(&sort_key(b)).then_with(|| a.cmp(b))
}

pub fn sort_comic_names(names: &mut [String]) {
    names.sort_by(|a, b| compare_comic_names(a, b));
}

// ---- path identity -------------------------------------------------------------------

/// Compare folder paths the way Windows users type them: slashes and case don't matter.
pub fn path_key(p: &str) -> String {
    let mut s = p.trim().replace('/', "\\");
    if let Some(rest) = s.strip_prefix("\\\\?\\") {
        s = rest.to_string();
    }
    while s.len() > 1 && s.ends_with('\\') {
        let bare = &s[..s.len() - 1];
        let drive = bare.len() == 2 && bare.as_bytes()[1] == b':';
        if drive || bare == "\\" {
            break;
        }
        s.pop();
    }
    if s.len() == 2 && s.as_bytes().get(1) == Some(&b':') {
        s.push('\\');
    }
    s.to_lowercase()
}

/// `D:\`, `D:`, `/`, and `\` are a whole drive or the filesystem root.
pub fn is_drive_root(p: &str) -> bool {
    let k = path_key(p);
    if k.is_empty() || k == "\\" {
        return true;
    }
    let b = k.as_bytes();
    b.len() == 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\'
}

pub fn mark_already(books: &mut [ScanBook], existing_paths: &[String]) {
    let keys: HashSet<String> = existing_paths.iter().map(|p| path_key(p)).collect();
    for b in books {
        b.already = keys.contains(&path_key(&b.path));
    }
}

// ---- grouping ------------------------------------------------------------------------

fn parent_dir(rel: &str) -> String {
    rel.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default()
}

fn hidden_name(name: &str) -> bool {
    name.starts_with('.') || name.is_empty()
}

/// One book's pages and chapters from image paths relative to that book folder.
/// `None` when there is no image. Chapter titles follow `scan::chapters`.
pub fn group_book(title: &str, path: &str, mut rels: Vec<String>) -> Option<ScanBook> {
    rels.retain(|r| crate::scan::is_image(r));
    if rels.is_empty() {
        return None;
    }
    sort_comic_names(&mut rels);
    let pages = rels.len() as u64;
    let mut chapters: Vec<ScanChapter> = Vec::new();
    for rel in &rels {
        let dir = parent_dir(rel);
        if dir.is_empty() {
            continue;
        }
        if chapters.last().map(|c| c.title == dir).unwrap_or(false) {
            chapters.last_mut().unwrap().pages += 1;
        } else {
            chapters.push(ScanChapter { title: dir, pages: 1 });
        }
    }
    if chapters.len() < 2 {
        chapters.clear();
    }
    Some(ScanBook {
        title: title.to_string(),
        path: path.to_string(),
        pages,
        chapters,
        already: false,
    })
}

fn entry_name(e: &DirEntry) -> String {
    e.file_name().to_string_lossy().to_string()
}

enum Kind {
    Dir,
    File,
    Skip(String),
}

fn classify(e: &DirEntry) -> Kind {
    match e.file_type() {
        Ok(ft) if ft.is_symlink() => Kind::Skip("symlink".into()),
        Ok(ft) if ft.is_dir() => Kind::Dir,
        Ok(ft) if ft.is_file() => Kind::File,
        Ok(_) => Kind::Skip("unreadable".into()),
        Err(err) => Kind::Skip(format!("unreadable: {err}")),
    }
}

fn rel_join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

/// Collect image paths relative to `dir`. `Err(())` if cancelled mid-walk.
const ISSUE_CAP: usize = 100;
const ISSUE_DB_CAP: i64 = 2000;

struct Tally {
    pages: u64,
    dirs: HashMap<String, u64>,
}

struct Flush {
    /// `Some` only on the first write, so the child list is not rewritten every book.
    child_names: Option<Vec<String>>,
    next_index: usize,
    new_book: Option<ScanBook>,
    folders_scanned: u64,
    loose_pages: u64,
    files_seen: u64,
    phase: String,
    new_issues: Vec<ScanIssue>,
}

struct Loaded {
    child_names: Vec<String>,
    next_index: usize,
    books: Vec<ScanBook>,
    folders_scanned: u64,
    loose_pages: u64,
    files_seen: u64,
    issue_count: u64,
}

pub(crate) fn ensure_scan_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS scan_checkpoint (
            root_key TEXT PRIMARY KEY,
            root TEXT NOT NULL,
            child_names TEXT NOT NULL,
            next_index INTEGER NOT NULL,
            folders_scanned INTEGER NOT NULL,
            loose_pages INTEGER NOT NULL,
            files_seen INTEGER NOT NULL,
            phase TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS scan_found (
            root_key TEXT NOT NULL,
            idx INTEGER NOT NULL,
            book_json TEXT NOT NULL,
            PRIMARY KEY (root_key, idx)
        );
        CREATE TABLE IF NOT EXISTS scan_issues (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            root_key TEXT NOT NULL,
            path TEXT NOT NULL,
            reason TEXT NOT NULL,
            created_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_scan_issues_root ON scan_issues(root_key);",
    )
    .map_err(|e| e.to_string())
}

fn insert_issue(conn: &Connection, root_key: &str, path: &str, reason: &str) -> Result<(), String> {
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM scan_issues WHERE root_key=?1 AND path=?2 AND reason=?3",
            params![root_key, path, reason],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if n > 0 {
        return Ok(());
    }
    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM scan_issues WHERE root_key=?1",
            [root_key],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if total >= ISSUE_DB_CAP {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO scan_issues (root_key, path, reason, created_at) VALUES (?1,?2,?3,?4)",
        params![root_key, path, reason, crate::now_secs()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Record a page the reader could not decode. Failures here are ignored.
pub fn note_reader_trouble(path: &str, reason: &str) {
    let Ok(conn) = crate::db_conn() else { return };
    let _ = insert_issue(&conn, "reader", path, reason);
}

fn clear_root(conn: &Connection, root_key: &str) -> Result<(), String> {
    conn.execute("DELETE FROM scan_found WHERE root_key=?1", [root_key])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM scan_issues WHERE root_key=?1", [root_key])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM scan_checkpoint WHERE root_key=?1", [root_key])
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn save_progress(conn: &Connection, root_key: &str, root: &str, flush: &Flush) -> Result<(), String> {
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    if let Some(names) = &flush.child_names {
        let names_json = serde_json::to_string(names).unwrap_or_else(|_| "[]".into());
        tx.execute(
            "INSERT INTO scan_checkpoint (root_key, root, child_names, next_index, folders_scanned, loose_pages, files_seen, phase, updated_at)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(root_key) DO UPDATE SET
                root=excluded.root, child_names=excluded.child_names, next_index=excluded.next_index,
                folders_scanned=excluded.folders_scanned, loose_pages=excluded.loose_pages,
                files_seen=excluded.files_seen, phase=excluded.phase, updated_at=excluded.updated_at",
            params![
                root_key,
                root,
                names_json,
                flush.next_index as i64,
                flush.folders_scanned as i64,
                flush.loose_pages as i64,
                flush.files_seen as i64,
                flush.phase,
                crate::now_secs()
            ],
        )
        .map_err(|e| e.to_string())?;
    } else {
        tx.execute(
            "UPDATE scan_checkpoint SET next_index=?2, folders_scanned=?3, loose_pages=?4, files_seen=?5, phase=?6, updated_at=?7
             WHERE root_key=?1",
            params![
                root_key,
                flush.next_index as i64,
                flush.folders_scanned as i64,
                flush.loose_pages as i64,
                flush.files_seen as i64,
                flush.phase,
                crate::now_secs()
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    if let Some(book) = &flush.new_book {
        let book_json = serde_json::to_string(book).unwrap_or_else(|_| "{}".into());
        let idx: i64 = tx
            .query_row("SELECT COALESCE(MAX(idx), -1) + 1 FROM scan_found WHERE root_key=?1", [root_key], |r| r.get(0))
            .unwrap_or(0);
        tx.execute(
            "INSERT INTO scan_found (root_key, idx, book_json) VALUES (?1,?2,?3)",
            params![root_key, idx, book_json],
        )
        .map_err(|e| e.to_string())?;
    }
    for issue in &flush.new_issues {
        insert_issue(&tx, root_key, &issue.path, &issue.reason)?;
    }
    tx.commit().map_err(|e| e.to_string())
}

fn load_progress(conn: &Connection, root_key: &str) -> Result<Option<Loaded>, String> {
    let row = conn.query_row(
        "SELECT child_names, next_index, folders_scanned, loose_pages, files_seen, phase FROM scan_checkpoint WHERE root_key=?1",
        [root_key],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
            ))
        },
    );
    let (names_json, next_index, folders, loose, files, phase) = match row {
        Ok(v) => v,
        Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    if phase == "done" || phase == "error" {
        return Ok(None);
    }
    let child_names: Vec<String> = serde_json::from_str(&names_json).unwrap_or_default();
    let mut stmt = conn
        .prepare("SELECT book_json FROM scan_found WHERE root_key=?1 ORDER BY idx")
        .map_err(|e| e.to_string())?;
    let books = stmt
        .query_map([root_key], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .flatten()
        .filter_map(|s| serde_json::from_str::<ScanBook>(&s).ok())
        .collect();
    let issue_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM scan_issues WHERE root_key=?1", [root_key], |r| r.get(0))
        .unwrap_or(0);
    Ok(Some(Loaded {
        child_names,
        next_index: next_index.max(0) as usize,
        books,
        folders_scanned: folders.max(0) as u64,
        loose_pages: loose.max(0) as u64,
        files_seen: files.max(0) as u64,
        issue_count: issue_count.max(0) as u64,
    }))
}

fn note(snap: &mut ScanSnapshot, pending: &mut Vec<ScanIssue>, path: String, reason: String) {
    snap.issue_count += 1;
    let issue = ScanIssue { path, reason };
    pending.push(issue.clone());
    snap.issues.push(issue);
    if snap.issues.len() > ISSUE_CAP {
        let extra = snap.issues.len() - ISSUE_CAP;
        snap.issues.drain(0..extra);
    }
}

fn read_entries(dir: &Path, snap: &mut ScanSnapshot, pending: &mut Vec<ScanIssue>) -> Result<Vec<DirEntry>, ()> {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(err) => {
            note(snap, pending, dir.to_string_lossy().to_string(), format!("unreadable: {err}"));
            return Err(());
        }
    };
    let mut out = Vec::new();
    for ent in rd {
        match ent {
            Ok(e) => {
                if e.file_name().to_str().is_none() {
                    note(snap, pending, e.path().to_string_lossy().to_string(), "non_utf8".into());
                }
                out.push(e);
            }
            Err(err) => note(snap, pending, dir.to_string_lossy().to_string(), format!("unreadable: {err}")),
        }
    }
    Ok(out)
}

/// Count images under `dir` without keeping their paths. `Err(())` means cancelled.
fn tally_dir(
    dir: &Path,
    rel: &str,
    cancel: &AtomicBool,
    snap: &mut ScanSnapshot,
    pending: &mut Vec<ScanIssue>,
    hook: &mut dyn FnMut(&ScanSnapshot),
) -> Result<Tally, ()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(());
    }
    snap.folders_scanned += 1;
    snap.current_path = dir.to_string_lossy().to_string();
    hook(snap);
    let entries = match read_entries(dir, snap, pending) {
        Ok(v) => v,
        Err(()) => return Ok(Tally { pages: 0, dirs: HashMap::new() }),
    };
    let mut tally = Tally { pages: 0, dirs: HashMap::new() };
    let mut subs = Vec::new();
    for e in entries {
        let name = entry_name(&e);
        if hidden_name(&name) {
            continue;
        }
        let path = e.path();
        match classify(&e) {
            Kind::Dir => subs.push((path, rel_join(rel, &name))),
            Kind::File => {
                snap.files_seen += 1;
                let rel_path = rel_join(rel, &name);
                if !crate::scan::is_image(&rel_path) {
                    continue;
                }
                match path.metadata() {
                    Ok(meta) if meta.len() == 0 => {
                        note(snap, pending, path.to_string_lossy().to_string(), "empty".into());
                    }
                    Ok(_) => {
                        tally.pages += 1;
                        let parent = parent_dir(&rel_path);
                        if !parent.is_empty() {
                            *tally.dirs.entry(parent).or_insert(0) += 1;
                        }
                    }
                    Err(err) => note(snap, pending, path.to_string_lossy().to_string(), format!("unreadable: {err}")),
                }
            }
            Kind::Skip(reason) => {
                snap.files_seen += 1;
                note(snap, pending, path.to_string_lossy().to_string(), reason);
            }
        }
    }
    for (path, child_rel) in subs {
        if cancel.load(Ordering::Relaxed) {
            return Err(());
        }
        let sub = tally_dir(&path, &child_rel, cancel, snap, pending, hook)?;
        tally.pages += sub.pages;
        for (k, v) in sub.dirs {
            *tally.dirs.entry(k).or_insert(0) += v;
        }
    }
    Ok(tally)
}

fn book_from_tally(title: &str, path: &str, tally: Tally) -> Option<ScanBook> {
    if tally.pages == 0 {
        return None;
    }
    let mut chapters: Vec<ScanChapter> = tally
        .dirs
        .into_iter()
        .map(|(title, pages)| ScanChapter { title, pages })
        .collect();
    chapters.sort_by(|a, b| compare_comic_names(&a.title, &b.title));
    if chapters.len() < 2 {
        chapters.clear();
    }
    Some(ScanBook {
        title: title.to_string(),
        path: path.to_string(),
        pages: tally.pages,
        chapters,
        already: false,
    })
}

fn emit(snap: &ScanSnapshot, pending: &mut Vec<ScanIssue>, on_flush: &mut dyn FnMut(&Flush), phase: &str, next_index: usize, child_names: Option<Vec<String>>, new_book: Option<ScanBook>) {
    on_flush(&Flush {
        child_names,
        next_index,
        new_book,
        folders_scanned: snap.folders_scanned,
        loose_pages: snap.loose_pages,
        files_seen: snap.files_seen,
        phase: phase.to_string(),
        new_issues: std::mem::take(pending),
    });
}

/// Walk `root`. Immediate subfolders with images become books.
///
/// Page paths are counted and discarded. A checkpoint flush runs after the child
/// list is known and after each child, so a cancel or a killed process can
/// continue from `resume.next_index` on the next scan of the same folder.
/// A finished scan (`phase == done`) is not passed back in, so the next run
/// starts over and picks up new folders.
fn scan_walk(
    root: &Path,
    cancel: &AtomicBool,
    existing: &HashSet<String>,
    resume: Option<Loaded>,
    on_flush: &mut dyn FnMut(&Flush),
    hook: &mut dyn FnMut(&ScanSnapshot),
) -> ScanSnapshot {
    let mut snap = ScanSnapshot::running(root);
    let mut pending: Vec<ScanIssue> = Vec::new();
    let (child_names, mut next_index, resumed) = if let Some(prev) = resume {
        snap.resumed = true;
        snap.books = prev.books;
        snap.books_found = snap.books.len() as u64;
        snap.folders_scanned = prev.folders_scanned;
        snap.loose_pages = prev.loose_pages;
        snap.files_seen = prev.files_seen;
        snap.issue_count = prev.issue_count;
        for book in &mut snap.books {
            book.already = existing.contains(&path_key(&book.path));
        }
        (prev.child_names, prev.next_index, true)
    } else {
        if cancel.load(Ordering::Relaxed) {
            snap.phase = "cancelled".into();
            hook(&snap);
            return snap;
        }
        let entries = match read_entries(root, &mut snap, &mut pending) {
            Ok(v) => v,
            Err(()) => {
                snap.phase = "error".into();
                snap.error = Some("unreadable".into());
                emit(&snap, &mut pending, on_flush, "error", 0, Some(Vec::new()), None);
                hook(&snap);
                return snap;
            }
        };
        snap.folders_scanned = 1;
        snap.current_path = root.to_string_lossy().to_string();
        let mut dirs: Vec<(String, PathBuf)> = Vec::new();
        for e in entries {
            let name = entry_name(&e);
            if hidden_name(&name) {
                continue;
            }
            match classify(&e) {
                Kind::Dir => dirs.push((name, e.path())),
                Kind::File => {
                    snap.files_seen += 1;
                    if !crate::scan::is_image(&name) {
                        continue;
                    }
                    match e.path().metadata() {
                        Ok(meta) if meta.len() == 0 => {
                            note(&mut snap, &mut pending, e.path().to_string_lossy().to_string(), "empty".into());
                        }
                        Err(err) => note(&mut snap, &mut pending, e.path().to_string_lossy().to_string(), format!("unreadable: {err}")),
                        Ok(_) => snap.loose_pages += 1,
                    }
                }
                Kind::Skip(reason) => {
                    snap.files_seen += 1;
                    note(&mut snap, &mut pending, e.path().to_string_lossy().to_string(), reason);
                }
            }
        }
        dirs.sort_by(|a, b| compare_comic_names(&a.0, &b.0));
        let names: Vec<String> = dirs.iter().map(|(n, _)| n.clone()).collect();
        emit(&snap, &mut pending, on_flush, "running", 0, Some(names.clone()), None);
        hook(&snap);
        (names, 0usize, false)
    };

    // Rebuild child paths from names so a resumed scan does not depend on the old DirEntry list.
    let children: Vec<(String, PathBuf)> = child_names.iter().cloned().zip(child_names.iter().map(|n| root.join(n))).collect();
    let _ = resumed;

    for (i, (name, path)) in children.iter().enumerate().skip(next_index) {
        if cancel.load(Ordering::Relaxed) {
            snap.phase = "cancelled".into();
            emit(&snap, &mut pending, on_flush, "cancelled", next_index, None, None);
            hook(&snap);
            return snap;
        }
        if !path.is_dir() {
            note(&mut snap, &mut pending, path.to_string_lossy().to_string(), "missing".into());
            next_index = i + 1;
            emit(&snap, &mut pending, on_flush, "running", next_index, None, None);
            continue;
        }
        let start_folders = snap.folders_scanned;
        let start_files = snap.files_seen;
        let start_issues = snap.issue_count;
        let start_issue_list = snap.issues.clone();
        let tally = match tally_dir(path, "", cancel, &mut snap, &mut pending, hook) {
            Ok(t) => t,
            Err(()) => {
                // This child did not finish. Drop its partial counts so the next scan repeats it once.
                snap.folders_scanned = start_folders;
                snap.files_seen = start_files;
                snap.issue_count = start_issues;
                snap.issues = start_issue_list;
                pending.clear();
                snap.phase = "cancelled".into();
                emit(&snap, &mut pending, on_flush, "cancelled", next_index, None, None);
                hook(&snap);
                return snap;
            }
        };
        let abs = path.to_string_lossy().to_string();
        let mut new_book = None;
        if let Some(mut book) = book_from_tally(name, &abs, tally) {
            book.already = existing.contains(&path_key(&abs));
            snap.books.push(book.clone());
            snap.books_found = snap.books.len() as u64;
            new_book = Some(book);
        }
        next_index = i + 1;
        emit(&snap, &mut pending, on_flush, "running", next_index, None, new_book);
        hook(&snap);
    }
    snap.phase = "done".into();
    snap.current_path.clear();
    emit(&snap, &mut pending, on_flush, "done", children.len(), None, None);
    hook(&snap);
    snap
}

/// Walk `root`. Immediate subfolders with images become books. Cancel is checked
/// between directories; a book that was only partly read is left out.
pub fn scan_tree(
    root: &Path,
    cancel: &AtomicBool,
    existing: &HashSet<String>,
    hook: &mut dyn FnMut(&ScanSnapshot),
) -> ScanSnapshot {
    scan_walk(root, cancel, existing, None, &mut |_| {}, hook)
}

fn existing_folder_keys() -> HashSet<String> {
    let mut keys = HashSet::new();
    let Ok(conn) = crate::db_conn() else { return keys };
    let Ok(mut stmt) = conn.prepare("SELECT paths FROM shelf WHERE kind='folder'") else { return keys };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) else { return keys };
    for row in rows.flatten() {
        let Ok(paths) = serde_json::from_str::<Vec<String>>(&row) else { continue };
        for p in paths {
            keys.insert(path_key(&p));
        }
    }
    keys
}

fn blank_snapshot() -> ScanSnapshot {
    ScanSnapshot {
        phase: "idle".into(),
        root: String::new(),
        folders_scanned: 0,
        books_found: 0,
        loose_pages: 0,
        files_seen: 0,
        current_path: String::new(),
        books: Vec::new(),
        error: None,
        resumed: false,
        issue_count: 0,
        issues: Vec::new(),
    }
}

fn publish(slot: &Mutex<ScanSnapshot>, next: &ScanSnapshot) {
    let Ok(mut slot) = slot.lock() else { return };
    let keep_books = next.phase != "running";
    slot.phase = next.phase.clone();
    slot.root = next.root.clone();
    slot.folders_scanned = next.folders_scanned;
    slot.books_found = next.books_found;
    slot.loose_pages = next.loose_pages;
    slot.files_seen = next.files_seen;
    slot.current_path = next.current_path.clone();
    slot.error = next.error.clone();
    slot.resumed = next.resumed;
    slot.issue_count = next.issue_count;
    slot.issues = next.issues.clone();
    if keep_books {
        slot.books = next.books.clone();
    }
}

#[tauri::command]
pub fn scan_begin(state: State<'_, ScanHost>, root: String) -> Result<(), String> {
    let trimmed = root.trim().to_string();
    let path = PathBuf::from(&trimmed);
    if is_drive_root(&trimmed) {
        return Err("drive_root".into());
    }
    if !path.exists() {
        return Err("missing".into());
    }
    if !path.is_dir() {
        return Err("not_folder".into());
    }
    if let Ok(guard) = state.job.lock() {
        if let Some(old) = guard.as_ref() {
            old.cancel.store(true, Ordering::Relaxed);
        }
    }
    let root_key = path_key(&trimmed);
    let (epoch, resume) = {
        let _db = state.shared.db.lock().unwrap_or_else(|e| e.into_inner());
        let epoch = state.shared.epoch.fetch_add(1, Ordering::Relaxed) + 1;
        let conn = crate::db_conn()?;
        let loaded = load_progress(&conn, &root_key)?;
        let resume = if loaded.is_some() {
            loaded
        } else {
            clear_root(&conn, &root_key)?;
            None
        };
        (epoch, resume)
    };
    let mut initial = ScanSnapshot::running(&path);
    if let Some(prev) = &resume {
        initial.resumed = true;
        initial.books_found = prev.books.len() as u64;
        initial.folders_scanned = prev.folders_scanned;
        initial.loose_pages = prev.loose_pages;
        initial.files_seen = prev.files_seen;
        initial.issue_count = prev.issue_count;
    }
    let cancel = Arc::new(AtomicBool::new(false));
    let snap = Arc::new(Mutex::new(initial));
    if let Ok(mut guard) = state.job.lock() {
        if let Some(old) = guard.take() {
            old.cancel.store(true, Ordering::Relaxed);
        }
        *guard = Some(Arc::new(Job { cancel: cancel.clone(), snap: snap.clone() }));
    } else {
        return Err("busy".into());
    }
    let existing = existing_folder_keys();
    let shared = state.shared.clone();
    let root_text = path.to_string_lossy().to_string();
    std::thread::spawn(move || {
        let finished = scan_walk(&path, &cancel, &existing, resume, &mut |flush| {
            let _db = shared.db.lock().unwrap_or_else(|e| e.into_inner());
            if shared.epoch.load(Ordering::Relaxed) != epoch {
                return;
            }
            if let Ok(conn) = crate::db_conn() {
                let _ = save_progress(&conn, &root_key, &root_text, flush);
            }
        }, &mut |next| publish(&snap, next));
        publish(&snap, &finished);
    });
    Ok(())
}

#[tauri::command]
pub fn scan_status(state: State<'_, ScanHost>) -> Result<ScanSnapshot, String> {
    let guard = state.job.lock().map_err(|e| e.to_string())?;
    Ok(match guard.as_ref() {
        Some(job) => job.snap.lock().map(|s| s.clone()).unwrap_or_else(|_| blank_snapshot()),
        None => blank_snapshot(),
    })
}

#[tauri::command]
pub fn scan_cancel(state: State<'_, ScanHost>) -> Result<(), String> {
    if let Ok(guard) = state.job.lock() {
        if let Some(job) = guard.as_ref() {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
    Ok(())
}

#[tauri::command]
pub fn list_scan_issues(root: Option<String>) -> Result<Vec<ScanIssue>, String> {
    let conn = crate::db_conn()?;
    let key = root.as_deref().map(str::trim).unwrap_or("");
    let mut stmt = if key.is_empty() {
        conn.prepare("SELECT path, reason FROM scan_issues ORDER BY id DESC LIMIT 200")
            .map_err(|e| e.to_string())?
    } else {
        conn.prepare("SELECT path, reason FROM scan_issues WHERE root_key=?1 ORDER BY id DESC LIMIT 200")
            .map_err(|e| e.to_string())?
    };
    let map = |r: &rusqlite::Row| {
        Ok(ScanIssue {
            path: r.get(0)?,
            reason: r.get(1)?,
        })
    };
    let rows = if key.is_empty() {
        stmt.query_map([], map).map_err(|e| e.to_string())?
    } else {
        let stored = path_key(key);
        stmt.query_map([stored], map).map_err(|e| e.to_string())?
    };
    let mut list: Vec<ScanIssue> = rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?;
    list.reverse();
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};
    use std::sync::atomic::AtomicBool;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn names(list: &[&str]) -> Vec<String> {
        let mut v: Vec<String> = list.iter().map(|s| (*s).to_string()).collect();
        sort_comic_names(&mut v);
        v
    }

    #[test]
    fn natural_order_digits_padding_and_mixed_names() {
        assert_eq!(names(&["10.jpg", "2.jpg", "1.jpg"]), vec!["1.jpg", "2.jpg", "10.jpg"]);
        assert_eq!(names(&["10.jpg", "2.jpg", "01.jpg"]), vec!["01.jpg", "2.jpg", "10.jpg"]);
        assert_eq!(
            names(&["page10.png", "page2.png", "cover.png"]),
            vec!["cover.png", "page2.png", "page10.png"]
        );
        assert_eq!(names(&["第2话/1.jpg", "第10话/1.jpg", "第2话/10.jpg", "第2话/2.jpg"]), vec![
            "第2话/1.jpg",
            "第2话/2.jpg",
            "第2话/10.jpg",
            "第10话/1.jpg",
        ]);
    }

    #[test]
    fn chinese_chapter_numbers_sort_before_plain_digits_of_the_same_value() {
        assert_eq!(numerals_to_digits("第一话"), "第1话");
        assert_eq!(numerals_to_digits("第十一话"), "第11话");
        assert_eq!(numerals_to_digits("第二十话"), "第20话");
        assert_eq!(numerals_to_digits("《二十四孝》"), "《二十四孝》");
        assert_eq!(
            names(&["第10话", "第2话", "第一话", "第十一话"]),
            vec!["第一话", "第2话", "第10话", "第十一话"]
        );
    }

    #[test]
    fn flat_folder_is_one_book_without_chapters() {
        let book = group_book("Solo", "/comics/Solo", vec!["10.jpg".into(), "2.jpg".into(), "1.jpg".into()]).unwrap();
        assert_eq!(book.pages, 3);
        assert!(book.chapters.is_empty());
        assert_eq!(book.title, "Solo");
    }

    #[test]
    fn nested_folders_become_chapters_in_reading_order() {
        let rels = vec![
            "第10话/1.jpg".into(),
            "第2话/2.jpg".into(),
            "第2话/10.jpg".into(),
            "第一话/1.jpg".into(),
            "notes.txt".into(),
        ];
        let book = group_book("BookName", "/comics/BookName", rels).unwrap();
        assert_eq!(book.pages, 4);
        assert_eq!(
            book.chapters.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(),
            vec!["第一话", "第2话", "第10话"]
        );
        assert_eq!(book.chapters[1].pages, 2);
    }

    #[test]
    fn a_single_chapter_folder_is_not_a_chapter_list() {
        let book = group_book("Only", "/comics/Only", vec!["第1话/1.jpg".into(), "第1话/2.jpg".into()]).unwrap();
        assert_eq!(book.pages, 2);
        assert!(book.chapters.is_empty());
    }

    #[test]
    fn path_keys_ignore_slash_and_case_and_mark_duplicates() {
        assert_eq!(path_key("D:/Comics/Book"), path_key("d:\\comics\\book\\"));
        assert!(is_drive_root("D:\\"));
        assert!(is_drive_root("D:"));
        assert!(is_drive_root("/"));
        assert!(!is_drive_root("D:\\Comics"));
        let mut books = vec![group_book("Book", "D:/Comics/Book", vec!["1.jpg".into()]).unwrap()];
        mark_already(&mut books, &["d:\\comics\\book".into()]);
        assert!(books[0].already);
        mark_already(&mut books, &["D:\\Comics\\Other".into()]);
        assert!(!books[0].already);
    }

    fn scratch(label: &str) -> PathBuf {
        let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("mh-scan-{label}-{n}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, b"x").unwrap();
    }

    #[test]
    fn walk_groups_children_skips_hidden_and_loose_root_files() {
        let root = scratch("walk");
        touch(&root.join("loose.jpg"));
        touch(&root.join("Flat").join("10.jpg"));
        touch(&root.join("Flat").join("2.jpg"));
        touch(&root.join("BookName").join("第10话").join("1.jpg"));
        touch(&root.join("BookName").join("第2话").join("1.jpg"));
        touch(&root.join("BookName").join("第一话").join("1.jpg"));
        touch(&root.join(".hidden").join("1.jpg"));
        touch(&root.join("ZipsOnly").join("pack.zip"));
        let snap = scan_tree(&root, &AtomicBool::new(false), &HashSet::new(), &mut |_| {});
        let _ = fs::remove_dir_all(&root);
        assert_eq!(snap.phase, "done");
        assert_eq!(snap.loose_pages, 1);
        let titles: Vec<_> = snap.books.iter().map(|b| b.title.as_str()).collect();
        assert_eq!(titles, vec!["BookName", "Flat"]);
        assert_eq!(snap.books[0].pages, 3);
        assert_eq!(
            snap.books[0].chapters.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(),
            vec!["第一话", "第2话", "第10话"]
        );
        assert!(snap.books[1].chapters.is_empty());
        assert_eq!(snap.books[1].pages, 2);
    }

    #[test]
    fn cancel_stops_before_the_remaining_books() {
        let root = scratch("cancel");
        for name in ["a-book", "b-book", "c-book"] {
            touch(&root.join(name).join("1.jpg"));
        }
        let cancel = AtomicBool::new(false);
        let snap = scan_tree(&root, &cancel, &HashSet::new(), &mut |s| {
            if s.books_found >= 1 {
                cancel.store(true, Ordering::Relaxed);
            }
        });
        let _ = fs::remove_dir_all(&root);
        assert_eq!(snap.phase, "cancelled");
        assert!(snap.books.len() < 3, "found {:?}", snap.books.iter().map(|b| &b.title).collect::<Vec<_>>());
        assert!(!snap.books.is_empty());
    }

    #[test]
    fn reader_folder_scan_uses_the_same_order() {
        let root = scratch("reader");
        touch(&root.join("10.jpg"));
        touch(&root.join("2.jpg"));
        touch(&root.join("第10话").join("1.jpg"));
        touch(&root.join("第一话").join("1.jpg"));
        let pages = crate::scan::scan_folder(&root);
        let _ = fs::remove_dir_all(&root);
        let names: Vec<String> = pages.iter().map(|p| p.name.replace('\\', "/")).collect();
        assert_eq!(names, vec!["2.jpg", "10.jpg", "第一话/1.jpg", "第10话/1.jpg"]);
    }

    #[test]
    fn empty_files_and_links_are_skipped_and_recorded() {
        let root = scratch("issues");
        touch(&root.join("Flat").join("ok.jpg"));
        fs::write(root.join("Flat").join("empty.jpg"), b"").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join("missing-target"), root.join("broken-link")).unwrap();
        let snap = scan_tree(&root, &AtomicBool::new(false), &HashSet::new(), &mut |_| {});
        let _ = fs::remove_dir_all(&root);
        assert_eq!(snap.phase, "done");
        assert_eq!(snap.books.len(), 1);
        assert_eq!(snap.books[0].pages, 1);
        let reasons: Vec<_> = snap.issues.iter().map(|i| i.reason.as_str()).collect();
        assert!(reasons.iter().any(|r| *r == "empty"), "{reasons:?}");
        #[cfg(unix)]
        assert!(reasons.iter().any(|r| *r == "symlink"), "{reasons:?}");
        assert!(snap.issue_count >= 1);
    }

    #[test]
    fn checkpoint_round_trip_and_resume_continues() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_scan_tables(&conn).unwrap();
        let root = scratch("resume");
        for name in ["a-book", "b-book", "c-book"] {
            touch(&root.join(name).join("1.jpg"));
        }
        let key = path_key(&root.to_string_lossy());
        let root_text = root.to_string_lossy().to_string();
        let cancel = AtomicBool::new(false);
        let snap = scan_walk(&root, &cancel, &HashSet::new(), None, &mut |flush| {
            if flush.next_index >= 1 && flush.phase == "running" {
                cancel.store(true, Ordering::Relaxed);
            }
            save_progress(&conn, &key, &root_text, flush).unwrap();
        }, &mut |_| {});
        assert_eq!(snap.phase, "cancelled");
        assert!(!snap.books.is_empty());
        let loaded = load_progress(&conn, &key).unwrap().expect("checkpoint");
        assert!(loaded.next_index >= 1 && loaded.next_index < 3, "next {}", loaded.next_index);
        assert_eq!(loaded.books.len(), snap.books.len());
        let snap2 = scan_walk(&root, &AtomicBool::new(false), &HashSet::new(), Some(loaded), &mut |flush| {
            save_progress(&conn, &key, &root_text, flush).unwrap();
        }, &mut |_| {});
        let _ = fs::remove_dir_all(&root);
        assert_eq!(snap2.phase, "done");
        assert!(snap2.resumed);
        assert_eq!(snap2.books.len(), 3, "{:?}", snap2.books.iter().map(|b| &b.title).collect::<Vec<_>>());
        let again = load_progress(&conn, &key).unwrap();
        assert!(again.is_none(), "a finished scan starts fresh");
    }

    fn peak_rss_kb() -> u64 {
        let Ok(text) = fs::read_to_string("/proc/self/status") else { return 0 };
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("VmHWM:") {
                return rest.split_whitespace().next().unwrap_or("0").parse().unwrap_or(0);
            }
        }
        0
    }

    #[test]
    #[ignore = "MH_STRESS=1 cargo test -- --ignored stress_scan --nocapture"]
    fn stress_scan_reports_memory_speed_and_problems() {
        if std::env::var("MH_STRESS").ok().as_deref() != Some("1") {
            eprintln!("MH_STRESS is not 1");
            return;
        }
        let root = scratch("stress");
        let n = 2500u32;
        for i in 0..n {
            let dir = root.join(format!("book-{i:05}"));
            if i % 20 == 0 {
                touch(&dir.join("ch1").join("1.jpg"));
                touch(&dir.join("ch2").join("2.jpg"));
            } else {
                touch(&dir.join("1.jpg"));
                touch(&dir.join("2.jpg"));
            }
        }
        touch(&root.join("flat-book").join("ok.jpg"));
        fs::write(root.join("flat-book").join("empty.jpg"), b"").unwrap();
        #[cfg(unix)]
        let _ = std::os::unix::fs::symlink(root.join("missing-target"), root.join("broken-link"));
        let before = peak_rss_kb();
        let started = std::time::Instant::now();
        let snap = scan_tree(&root, &AtomicBool::new(false), &HashSet::new(), &mut |_| {});
        let elapsed = started.elapsed();
        let peak = peak_rss_kb();
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE shelf (id INTEGER PRIMARY KEY, name TEXT, kind TEXT, paths TEXT, page_count INTEGER, folder_id INTEGER, updated_at INTEGER)",
        )
        .unwrap();
        let insert_started = std::time::Instant::now();
        let tx = conn.unchecked_transaction().unwrap();
        for b in &snap.books {
            let paths = serde_json::to_string(&vec![b.path.as_str()]).unwrap_or_else(|_| "[]".into());
            tx.execute(
                "INSERT INTO shelf (name, kind, paths, page_count, folder_id, updated_at) VALUES (?1,'folder',?2,?3,NULL,0)",
                params![b.title, paths, b.pages as i64],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        let insert_ms = insert_started.elapsed().as_millis();
        let page: i64 = conn
            .query_row("SELECT COUNT(*) FROM (SELECT id FROM shelf ORDER BY updated_at DESC LIMIT 48)", [], |r| r.get(0))
            .unwrap();
        let _ = fs::remove_dir_all(&root);
        let secs = elapsed.as_secs_f64().max(0.001);
        println!(
            "STRESS books={} folders_scanned={} files_seen={} elapsed_ms={} folders_per_sec={:.1} files_per_sec={:.1} peak_rss_kb={} rss_before_kb={} problems={} library_insert_ms={} page_rows={} phase={}",
            snap.books.len(),
            snap.folders_scanned,
            snap.files_seen,
            elapsed.as_millis(),
            snap.folders_scanned as f64 / secs,
            snap.files_seen as f64 / secs,
            peak,
            before,
            snap.issue_count,
            insert_ms,
            page,
            snap.phase
        );
        assert_eq!(snap.phase, "done");
        assert!(snap.books.len() >= n as usize, "books {}", snap.books.len());
        assert!(snap.issue_count >= 1, "expected the empty file to be recorded");
        assert_eq!(page, 48);
    }
}
