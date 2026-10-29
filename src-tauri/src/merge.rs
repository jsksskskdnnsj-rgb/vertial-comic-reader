//! Group folder books that are the same comic under different volume names.
//!
//! Names are normalised (width, volume/chapter markers, brackets, digits, punctuation).
//! An identical base plus one non-overlapping order key per folder is merged automatically
//! when `settings.merge.auto` is on (the default). Anything less certain is stored in
//! `merge_pending` for the dialog. Merging only rewrites shelf rows: files stay where they are.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::shelf_scan::{
    chinese_number, compare_comic_names, is_cn_numeral, nfkc_filename, path_key, ScanBook,
    ScanSnapshot,
};

const DDL: &str = "
CREATE TABLE IF NOT EXISTS book_merges (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  book_id INTEGER NOT NULL,
  title TEXT NOT NULL,
  auto INTEGER NOT NULL DEFAULT 0,
  undone INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_book_merges_book ON book_merges(book_id);
CREATE TABLE IF NOT EXISTS book_merge_parts (
  merge_id INTEGER NOT NULL,
  ord INTEGER NOT NULL,
  source_id INTEGER,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  paths TEXT NOT NULL,
  page_count INTEGER NOT NULL,
  total_size INTEGER NOT NULL DEFAULT 0,
  last_page INTEGER NOT NULL DEFAULT 0,
  finished INTEGER NOT NULL DEFAULT 0,
  folder_id INTEGER,
  progress_page INTEGER,
  progress_frac REAL,
  progress_updated INTEGER,
  PRIMARY KEY (merge_id, ord)
);
CREATE TABLE IF NOT EXISTS merge_pending (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  created_at INTEGER NOT NULL,
  resolved INTEGER NOT NULL DEFAULT 0,
  payload TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_merge_pending_open ON merge_pending(resolved);
CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

pub fn ensure_tables(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(DDL).map_err(|e| e.to_string())
}

#[derive(Clone, Debug)]
struct Parsed {
    base: String,
    display: String,
    order: Option<(u64, u64)>,
    label: String,
}

struct Span {
    start: usize,
    end: usize,
    ord: (u64, u64),
    label: String,
}

#[derive(Clone, Debug)]
struct Member {
    shelf_id: Option<i64>,
    name: String,
    paths: Vec<String>,
    pages: i64,
    chapters: i64,
    cover_ver: i64,
    parsed: Parsed,
}

struct Group {
    confident: bool,
    title: String,
    members: Vec<Member>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingMember {
    pub name: String,
    pub path: String,
    pub pages: i64,
    #[serde(default)]
    pub chapters: i64,
    #[serde(default)]
    pub order_label: String,
    #[serde(default)]
    pub shelf_id: Option<i64>,
    #[serde(default)]
    pub cover_ver: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingGroup {
    #[serde(default)]
    pub id: i64,
    pub suggested_title: String,
    pub confident: bool,
    pub members: Vec<PendingMember>,
}

#[derive(Deserialize)]
pub struct MergeIn {
    pub shelf_id: i64,
    pub path: String,
    pub name: String,
    /// Sent by the library dialog; the shelf row's page_count is what the merge uses.
    #[allow(dead_code)]
    pub pages: i64,
}

struct SnapPart {
    source_id: Option<i64>,
    name: String,
    kind: String,
    paths: Vec<String>,
    page_count: i64,
    total_size: i64,
    last_page: i64,
    finished: i64,
    folder_id: Option<i64>,
    progress_page: Option<i64>,
    progress_frac: Option<f64>,
    progress_updated: Option<i64>,
}

// ---- names --------------------------------------------------------------------

fn is_counter(c: char) -> bool {
    matches!(
        c,
        '话' | '話' | '章' | '卷' | '集' | '回' | '册' | '冊' | '部' | '期' | '节' | '節' | '篇'
    )
}

fn rank_of(c: char) -> Option<u64> {
    match c {
        '上' => Some(1),
        '中' => Some(2),
        '下' => Some(3),
        _ => None,
    }
}

fn is_trim_char(c: char) -> bool {
    matches!(
        c,
        '_' | '-' | '.' | '~' | '～' | '—' | '–' | '/' | '\\' | ':' | '|' | '+' | '·' | '・' | '('
            | ')' | '[' | ']' | '【' | '】'
    )
}

fn is_edge(c: char) -> bool {
    c.is_whitespace() || !c.is_alphanumeric()
}

fn is_latin_boundary(chars: &[char], i: usize) -> bool {
    i == 0 || !chars[i - 1].is_ascii_alphabetic()
}

fn is_soft_boundary(chars: &[char], i: usize) -> bool {
    i == 0 || !chars[i - 1].is_alphanumeric()
}

fn parse_num(chars: &[char], i: usize) -> Option<(u64, usize)> {
    if i >= chars.len() {
        return None;
    }
    if chars[i].is_ascii_digit() {
        let mut j = i + 1;
        while j < chars.len() && chars[j].is_ascii_digit() {
            j += 1;
        }
        if j - i > 9 {
            return None;
        }
        let s: String = chars[i..j].iter().collect();
        return s.parse().ok().map(|n| (n, j));
    }
    if is_cn_numeral(chars[i]) {
        let mut j = i + 1;
        while j < chars.len() && is_cn_numeral(chars[j]) {
            j += 1;
        }
        return chinese_number(&chars[i..j]).map(|n| (n, j));
    }
    None
}

fn parse_num_ws(chars: &[char], i: usize) -> Option<(u64, usize)> {
    let mut j = i;
    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }
    parse_num(chars, j)
}

fn parse_range_sep(chars: &[char], i: usize) -> Option<usize> {
    let mut j = i;
    while j < chars.len() && chars[j].is_whitespace() {
        j += 1;
    }
    if j >= chars.len() {
        return None;
    }
    match chars[j] {
        '-' | '~' | '～' | '—' | '–' | '－' | '到' | '至' => Some(j + 1),
        _ => None,
    }
}

fn span_range(start: usize, end: usize, n1: u64, n2: u64) -> Span {
    let (a, b) = if n1 <= n2 { (n1, n2) } else { (n2, n1) };
    let label = if a == b {
        a.to_string()
    } else {
        format!("{a}–{b}")
    };
    Span { start, end, ord: (a, b), label }
}

fn try_di(chars: &[char], i: usize) -> Option<Span> {
    if chars.get(i) != Some(&'第') {
        return None;
    }
    let (n1, j) = parse_num(chars, i + 1)?;
    if let Some(k) = parse_range_sep(chars, j) {
        if let Some((n2, m)) = parse_num_ws(chars, k) {
            if m < chars.len() && is_counter(chars[m]) {
                return Some(span_range(i, m + 1, n1, n2));
            }
        }
    }
    if j < chars.len() && is_counter(chars[j]) {
        return Some(span_range(i, j + 1, n1, n1));
    }
    None
}

fn try_range_counter(chars: &[char], i: usize) -> Option<Span> {
    if !is_soft_boundary(chars, i) {
        return None;
    }
    let (n1, j) = parse_num(chars, i)?;
    let k = parse_range_sep(chars, j)?;
    let (n2, m) = parse_num_ws(chars, k)?;
    if m < chars.len() && is_counter(chars[m]) {
        return Some(span_range(i, m + 1, n1, n2));
    }
    None
}

fn try_juan(chars: &[char], i: usize) -> Option<Span> {
    if !matches!(chars.get(i), Some('卷' | '册' | '冊')) {
        return None;
    }
    let (n1, j) = parse_num_ws(chars, i + 1)?;
    if let Some(k) = parse_range_sep(chars, j) {
        if let Some((n2, m)) = parse_num_ws(chars, k) {
            return Some(span_range(i, m, n1, n2));
        }
    }
    Some(span_range(i, j, n1, n1))
}

fn try_latin(chars: &[char], i: usize) -> Option<Span> {
    if !is_latin_boundary(chars, i) {
        return None;
    }
    let rest: String = chars[i..].iter().flat_map(|c| c.to_lowercase()).collect();
    const KEYS: &[&str] = &["volume", "chapter", "episode", "chap", "vol", "part", "ep", "ch"];
    for key in KEYS {
        if !rest.starts_with(key) {
            continue;
        }
        let mut j = i + key.len();
        if j < chars.len() && matches!(chars[j], '.' | '_' | '-') {
            j += 1;
        }
        let Some((n1, m)) = parse_num_ws(chars, j) else { continue };
        let mut end = m;
        let mut n2 = n1;
        if let Some(k) = parse_range_sep(chars, m) {
            if let Some((b, e)) = parse_num_ws(chars, k) {
                n2 = b;
                end = e;
            }
        }
        return Some(span_range(i, end, n1, n2));
    }
    None
}

fn try_bracket_rank(chars: &[char], i: usize) -> Option<Span> {
    let close = match chars.get(i)? {
        '(' => ')',
        '[' => ']',
        '【' => '】',
        '「' => '」',
        '『' => '』',
        '{' => '}',
        _ => return None,
    };
    let c = *chars.get(i + 1)?;
    let r = rank_of(c)?;
    if chars.get(i + 2) != Some(&close) {
        return None;
    }
    Some(Span { start: i, end: i + 3, ord: (r, r), label: c.to_string() })
}

fn try_token_rank(chars: &[char], i: usize) -> Option<Span> {
    let c = *chars.get(i)?;
    let r = rank_of(c)?;
    let prev_ok = i == 0 || is_edge(chars[i - 1]);
    let next_ok = i + 1 >= chars.len() || is_edge(chars[i + 1]);
    if prev_ok && next_ok {
        Some(Span { start: i, end: i + 1, ord: (r, r), label: c.to_string() })
    } else {
        None
    }
}

fn try_trailing(chars: &[char]) -> Option<Span> {
    if chars.is_empty() {
        return None;
    }
    let mut end = chars.len();
    let mut close_at = None;
    if matches!(chars[end - 1], ')' | ']' | '】' | '」' | '』' | '}') {
        close_at = Some(end - 1);
        end -= 1;
        if end == 0 {
            return None;
        }
    }
    if !chars[end - 1].is_ascii_digit() {
        return None;
    }
    let mut start = end;
    while start > 0 && chars[start - 1].is_ascii_digit() {
        start -= 1;
    }
    let text: String = chars[start..end].iter().collect();
    let n: u64 = text.parse().ok()?;
    if n > 10_000 {
        return None;
    }
    if close_at.is_none() && text.len() == 4 && (1900..=2099).contains(&n) {
        return None;
    }
    let (span_start, span_end) = if let Some(ci) = close_at {
        if start == 0 {
            return None;
        }
        let expect = match chars[ci] {
            ')' => '(',
            ']' => '[',
            '】' => '【',
            '」' => '「',
            '』' => '『',
            '}' => '{',
            _ => return None,
        };
        if chars[start - 1] != expect {
            return None;
        }
        (start - 1, ci + 1)
    } else {
        (start, end)
    };
    if span_start == 0 && span_end == chars.len() {
        return None;
    }
    Some(span_range(span_start, span_end, n, n))
}

fn find_spans(chars: &[char]) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let hit = try_di(chars, i)
            .or_else(|| try_range_counter(chars, i))
            .or_else(|| try_juan(chars, i))
            .or_else(|| try_latin(chars, i))
            .or_else(|| try_bracket_rank(chars, i))
            .or_else(|| try_token_rank(chars, i));
        if let Some(sp) = hit {
            if sp.end > i {
                i = sp.end;
                spans.push(sp);
                continue;
            }
        }
        i += 1;
    }
    if spans.is_empty() {
        if let Some(sp) = try_trailing(chars) {
            spans.push(sp);
        }
    }
    spans
}

fn trim_display(s: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in s.chars() {
        if c.is_whitespace() {
            gap = true;
            continue;
        }
        if gap && !out.is_empty() {
            out.push(' ');
        }
        gap = false;
        out.push(c);
    }
    out.trim_matches(|c: char| c.is_whitespace() || is_trim_char(c))
        .to_string()
}

fn parse_name(raw: &str) -> Parsed {
    let folded = nfkc_filename(raw);
    let chars: Vec<char> = folded.chars().collect();
    let spans = find_spans(&chars);
    let mut cover = vec![false; chars.len()];
    for sp in &spans {
        for i in sp.start..sp.end.min(chars.len()) {
            cover[i] = true;
        }
    }
    let mut disp = String::new();
    let mut base = String::new();
    for (i, c) in chars.iter().enumerate() {
        if cover[i] {
            continue;
        }
        disp.push(*c);
        if c.is_alphanumeric() && !c.is_ascii_digit() {
            for low in c.to_lowercase() {
                base.push(low);
            }
        }
    }
    let order = if spans.len() == 1 { Some(spans[0].ord) } else { None };
    let label = if spans.len() == 1 { spans[0].label.clone() } else { String::new() };
    Parsed { base, display: trim_display(&disp), order, label }
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

fn similar(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let ac = char_len(a);
    let bc = char_len(b);
    if ac < 2 || bc < 2 {
        return false;
    }
    let (short, long, sc, lc) = if ac <= bc { (a, b, ac, bc) } else { (b, a, bc, ac) };
    if long.contains(short) && lc - sc <= 4 {
        return true;
    }
    if sc > 64 || lc > 64 {
        return false;
    }
    let av: Vec<char> = a.chars().collect();
    let bv: Vec<char> = b.chars().collect();
    let dist = levenshtein(&av, &bv) as f64;
    1.0 - dist / (lc.max(sc) as f64) >= 0.86
}

fn prefix2(base: &str) -> String {
    base.chars().take(2).collect()
}

fn suggest_title(members: &[Member]) -> String {
    let mut best = String::new();
    for m in members {
        let d = m.parsed.display.trim();
        if d.is_empty() {
            continue;
        }
        if best.is_empty() || char_len(d) < char_len(&best) {
            best = d.to_string();
        }
    }
    if best.is_empty() {
        members.first().map(|m| m.name.clone()).unwrap_or_default()
    } else {
        best
    }
}

fn ranges_conflict(orders: &[(u64, u64)]) -> bool {
    if orders.len() < 2 {
        return false;
    }
    let mut rs = orders.to_vec();
    rs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut max_end = rs[0].1;
    for r in rs.iter().skip(1) {
        if r.0 <= max_end {
            return true;
        }
        if r.1 > max_end {
            max_end = r.1;
        }
    }
    false
}

fn sort_members(members: &mut [Member]) {
    members.sort_by(|a, b| {
        let ao = a.parsed.order.unwrap_or((u64::MAX, u64::MAX));
        let bo = b.parsed.order.unwrap_or((u64::MAX, u64::MAX));
        ao.0.cmp(&bo.0)
            .then(ao.1.cmp(&bo.1))
            .then_with(|| compare_comic_names(&a.name, &b.name))
    });
}

fn group_members(pool: &[Member]) -> Vec<Group> {
    let mut exact: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, m) in pool.iter().enumerate() {
        if char_len(&m.parsed.base) < 2 {
            continue;
        }
        exact.entry(m.parsed.base.clone()).or_default().push(i);
    }
    let mut used = vec![false; pool.len()];
    let mut groups = Vec::new();
    for idxs in exact.values() {
        if idxs.len() < 2 {
            continue;
        }
        let members: Vec<Member> = idxs.iter().map(|&i| pool[i].clone()).collect();
        let orders: Option<Vec<(u64, u64)>> = members.iter().map(|m| m.parsed.order).collect();
        let confident = match &orders {
            Some(v) => !ranges_conflict(v),
            None => false,
        };
        let mut members = members;
        sort_members(&mut members);
        let title = suggest_title(&members);
        for &i in idxs {
            used[i] = true;
        }
        groups.push(Group { confident, title, members });
    }
    let mut buckets: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, m) in pool.iter().enumerate() {
        if used[i] || char_len(&m.parsed.base) < 2 {
            continue;
        }
        buckets.entry(prefix2(&m.parsed.base)).or_default().push(i);
    }
    for idxs in buckets.values() {
        if idxs.len() < 2 || idxs.len() > 48 {
            continue;
        }
        let mut parent: Vec<usize> = (0..idxs.len()).collect();
        fn find(p: &mut [usize], x: usize) -> usize {
            if p[x] != x {
                p[x] = find(p, p[x]);
            }
            p[x]
        }
        for a in 0..idxs.len() {
            for b in (a + 1)..idxs.len() {
                let ma = &pool[idxs[a]];
                let mb = &pool[idxs[b]];
                if ma.parsed.base == mb.parsed.base || !similar(&ma.parsed.base, &mb.parsed.base) {
                    continue;
                }
                let pa = find(&mut parent, a);
                let pb = find(&mut parent, b);
                if pa != pb {
                    parent[pb] = pa;
                }
            }
        }
        let mut clusters: HashMap<usize, Vec<usize>> = HashMap::new();
        for i in 0..idxs.len() {
            clusters.entry(find(&mut parent, i)).or_default().push(idxs[i]);
        }
        for members_i in clusters.into_values() {
            if members_i.len() < 2 {
                continue;
            }
            let mut members: Vec<Member> = members_i.iter().map(|&i| pool[i].clone()).collect();
            sort_members(&mut members);
            let title = suggest_title(&members);
            groups.push(Group { confident: false, title, members });
        }
    }
    groups
}

fn group_relevant(fresh: &[Member], library: &[Member]) -> Vec<Group> {
    let fresh_kept: Vec<&Member> = fresh
        .iter()
        .filter(|m| char_len(&m.parsed.base) >= 2)
        .collect();
    if fresh_kept.is_empty() {
        return Vec::new();
    }
    let mut by_prefix: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, m) in fresh_kept.iter().enumerate() {
        by_prefix.entry(prefix2(&m.parsed.base)).or_default().push(i);
    }
    let mut fresh_paths = HashSet::new();
    for m in fresh {
        for p in &m.paths {
            fresh_paths.insert(path_key(p));
        }
    }
    let mut pool: Vec<Member> = fresh_kept.into_iter().cloned().collect();
    for lib in library {
        if char_len(&lib.parsed.base) < 2 {
            continue;
        }
        if lib.paths.iter().any(|p| fresh_paths.contains(&path_key(p))) {
            continue;
        }
        let Some(cands) = by_prefix.get(&prefix2(&lib.parsed.base)) else { continue };
        let hit = cands.iter().any(|&i| {
            let f = &pool[i];
            f.parsed.base == lib.parsed.base || similar(&f.parsed.base, &lib.parsed.base)
        });
        if hit {
            pool.push(lib.clone());
        }
    }
    group_members(&pool)
}

fn member_from_scan(b: &ScanBook) -> Member {
    Member {
        shelf_id: None,
        name: b.title.clone(),
        paths: vec![b.path.clone()],
        pages: b.pages as i64,
        chapters: b.chapters.len() as i64,
        cover_ver: 0,
        parsed: parse_name(&b.title),
    }
}

fn member_from_pending(m: &PendingMember) -> Member {
    Member {
        shelf_id: m.shelf_id,
        name: m.name.clone(),
        paths: if m.path.is_empty() { Vec::new() } else { vec![m.path.clone()] },
        pages: m.pages,
        chapters: m.chapters,
        cover_ver: m.cover_ver,
        parsed: parse_name(&m.name),
    }
}

fn to_pending(g: &Group) -> PendingGroup {
    PendingGroup {
        id: 0,
        suggested_title: g.title.clone(),
        confident: g.confident,
        members: g
            .members
            .iter()
            .map(|m| PendingMember {
                name: m.name.clone(),
                path: m.paths.first().cloned().unwrap_or_default(),
                pages: m.pages,
                chapters: m.chapters,
                order_label: m.parsed.label.clone(),
                shelf_id: m.shelf_id,
                cover_ver: m.cover_ver,
            })
            .collect(),
    }
}

// ---- shelf --------------------------------------------------------------------

fn auto_on(conn: &Connection) -> bool {
    match conn.query_row(
        "SELECT value FROM settings WHERE key='merge.auto'",
        [],
        |r| r.get::<_, String>(0),
    ) {
        Ok(v) => v != "0",
        Err(_) => true,
    }
}

fn load_folder_members(conn: &Connection) -> Vec<Member> {
    let Ok(mut stmt) = conn.prepare(
        "SELECT id, name, paths, page_count, cover_ver FROM shelf WHERE kind='folder'",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
        ))
    }) else {
        return Vec::new();
    };
    rows.flatten()
        .filter_map(|(id, name, paths_json, pages, cover)| {
            let paths: Vec<String> = serde_json::from_str(&paths_json).unwrap_or_default();
            if paths.is_empty() {
                return None;
            }
            Some(Member {
                shelf_id: Some(id),
                name: name.clone(),
                paths,
                pages,
                chapters: 0,
                cover_ver: cover,
                parsed: parse_name(&name),
            })
        })
        .collect()
}

fn load_snap(conn: &Connection, id: i64) -> Result<SnapPart, String> {
    let row = conn
        .query_row(
            "SELECT name, kind, paths, page_count, total_size, last_page, finished, folder_id, updated_at FROM shelf WHERE id=?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i64>(6)?,
                    r.get::<_, Option<i64>>(7)?,
                    r.get::<_, i64>(8)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?;
    if row.1 != "folder" {
        return Err("只能合并文件夹漫画".into());
    }
    let paths: Vec<String> = serde_json::from_str(&row.2).unwrap_or_default();
    let prog = conn
        .query_row(
            "SELECT page, frac, updated_at FROM progress WHERE book_id=?1",
            [id],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?, r.get::<_, i64>(2)?)),
        )
        .optional()
        .unwrap_or(None);
    let (progress_page, progress_frac, progress_updated) = if let Some((p, f, u)) = prog {
        (Some(p), Some(f), Some(u))
    } else if row.5 > 0 {
        (Some(row.5), Some(0.0), Some(row.8))
    } else {
        (None, None, None)
    };
    Ok(SnapPart {
        source_id: Some(id),
        name: row.0,
        kind: row.1,
        paths,
        page_count: row.3,
        total_size: row.4,
        last_page: row.5,
        finished: row.6,
                folder_id: row.7,
                progress_page,
        progress_frac,
        progress_updated,
    })
}

fn snapshot_member(conn: &Connection, m: &Member) -> Result<SnapPart, String> {
    if let Some(id) = m.shelf_id {
        load_snap(conn, id)
    } else {
        Ok(SnapPart {
            source_id: None,
            name: m.name.clone(),
            kind: "folder".into(),
            paths: m.paths.clone(),
            page_count: m.pages.max(0),
            total_size: 0,
            last_page: 0,
            finished: 0,
            folder_id: None,
            progress_page: None,
            progress_frac: None,
            progress_updated: None,
        })
    }
}

fn file_label(path: &str, ord: usize) -> String {
    Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().replace(['/', '\\'], "／"))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("part{}", ord + 1))
}

fn part_prefix(index: usize, path: &str) -> String {
    format!("{:02}-{}/", index + 1, file_label(path, index))
}

fn index_of(paths: &[String], path: &str) -> usize {
    let key = path_key(path);
    paths.iter().position(|p| path_key(p) == key).unwrap_or(0)
}

fn merged_entry_name(entry: &str, old_paths: &[String], new_paths: &[String]) -> String {
    if old_paths.is_empty() {
        return entry.to_string();
    }
    if old_paths.len() == 1 {
        let prefix = part_prefix(index_of(new_paths, &old_paths[0]), &old_paths[0]);
        if entry.starts_with(&prefix) {
            return entry.to_string();
        }
        return format!("{prefix}{entry}");
    }
    for (i, p) in old_paths.iter().enumerate() {
        let old_pre = part_prefix(i, p);
        if let Some(rel) = entry.strip_prefix(&old_pre) {
            return format!("{}{rel}", part_prefix(index_of(new_paths, p), p));
        }
    }
    entry.to_string()
}

fn restored_entry_name(entry: &str, part_paths: &[String], merged_paths: &[String]) -> Option<String> {
    if part_paths.is_empty() {
        return None;
    }
    if part_paths.len() == 1 {
        let pre = part_prefix(index_of(merged_paths, &part_paths[0]), &part_paths[0]);
        return entry.strip_prefix(&pre).map(|s| s.to_string());
    }
    for (local_i, p) in part_paths.iter().enumerate() {
        let pre = part_prefix(index_of(merged_paths, p), p);
        if let Some(rel) = entry.strip_prefix(&pre) {
            return Some(format!("{}{rel}", part_prefix(local_i, p)));
        }
    }
    None
}

fn bookmark_rows(conn: &Connection, book_id: i64) -> Vec<(i64, String)> {
    let Ok(mut stmt) = conn.prepare("SELECT id, entry_name FROM bookmarks WHERE book_id=?1") else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([book_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))) else {
        return Vec::new();
    };
    rows.flatten().collect()
}

fn rewrite_bookmarks(
    conn: &Connection,
    from_id: i64,
    old_paths: &[String],
    new_paths: &[String],
    to_id: i64,
) {
    for (id, name) in bookmark_rows(conn, from_id) {
        let next = merged_entry_name(&name, old_paths, new_paths);
        let _ = conn.execute(
            "UPDATE bookmarks SET book_id=?1, entry_name=?2 WHERE id=?3",
            params![to_id, next, id],
        );
    }
}

fn ignore_delete(conn: &Connection, sql: &str, id: i64) {
    let _ = conn.execute(sql, [id]);
}

fn concat_paths(snaps: &[SnapPart]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for snap in snaps {
        for p in &snap.paths {
            if p.is_empty() {
                continue;
            }
            if seen.insert(path_key(p)) {
                out.push(p.clone());
            }
        }
    }
    out
}

fn pick_progress(snaps: &[SnapPart]) -> Option<(i64, f64)> {
    let mut best_u = -1i64;
    let mut best: Option<(i64, f64)> = None;
    let mut offset = 0i64;
    for snap in snaps {
        let count = snap.page_count.max(0);
        if let Some(page) = snap.progress_page {
            let updated = snap.progress_updated.unwrap_or(0);
            if updated >= best_u {
                let local = if count == 0 { 0 } else { page.clamp(0, count - 1) };
                best = Some((offset + local, snap.progress_frac.unwrap_or(0.0)));
                best_u = updated;
            }
        }
        offset += count;
    }
    best
}

fn insert_folder_row(
    conn: &Connection,
    name: &str,
    paths: &[String],
    pages: i64,
    total_size: i64,
    last_page: i64,
    folder_id: Option<i64>,
    finished: i64,
) -> Result<i64, String> {
    let json = serde_json::to_string(paths).unwrap_or_else(|_| "[]".into());
    let now = crate::now_secs();
    conn.execute(
        "INSERT INTO shelf (name,kind,paths,cover,page_count,total_size,last_page,created_at,updated_at,folder_id,finished,cover_ver)
         VALUES (?1,'folder',?2,'',?3,?4,?5,?6,?6,?7,?8,1)",
        params![name, json, pages, total_size, last_page, now, folder_id, finished],
    )
    .map_err(|e| e.to_string())?;
    Ok(conn.last_insert_rowid())
}

fn drop_cover(id: i64) {
    if cfg!(test) {
        return;
    }
    if let Ok(dir) = crate::data_dir() {
        let _ = std::fs::remove_file(dir.join("covers").join(format!("{id}.jpg")));
    }
}

fn apply_parts_inner(
    conn: &Connection,
    title: &str,
    folder_override: Option<i64>,
    members: &[Member],
    auto: bool,
) -> Result<i64, String> {
    if members.len() < 2 {
        return Err("至少需要两本".into());
    }
    let snaps: Vec<SnapPart> = members
        .iter()
        .map(|m| snapshot_member(conn, m))
        .collect::<Result<_, _>>()?;
    let new_paths = concat_paths(&snaps);
    if new_paths.len() < 2 && snaps.iter().map(|s| s.paths.len()).sum::<usize>() < 2 {
        return Err("至少需要两个文件夹".into());
    }
    let page_count: i64 = snaps.iter().map(|s| s.page_count.max(0)).sum();
    let total_size: i64 = snaps.iter().map(|s| s.total_size.max(0)).sum();
    let finished = if snaps.iter().all(|s| s.finished == 1) { 1 } else { 0 };
    let folder_id = folder_override.or_else(|| snaps.iter().find_map(|s| s.folder_id));
    let progress = pick_progress(&snaps);
    let last_page = progress.map(|(p, _)| p).unwrap_or(0);
    let name = if title.trim().is_empty() { suggest_title(members) } else { title.trim().to_string() };
    let survivor = if let Some(id) = snaps.iter().find_map(|s| s.source_id) {
        id
    } else {
        insert_folder_row(conn, &name, &new_paths, page_count, total_size, last_page, folder_id, finished)?
    };
    for snap in &snaps {
        if let Some(id) = snap.source_id {
            rewrite_bookmarks(conn, id, &snap.paths, &new_paths, survivor);
        }
    }
    let paths_json = serde_json::to_string(&new_paths).unwrap_or_else(|_| "[]".into());
    let now = crate::now_secs();
    conn.execute(
        "UPDATE shelf SET name=?1, paths=?2, page_count=?3, total_size=?4, folder_id=?5, scan_sig=NULL, last_page=?6, finished=?7, updated_at=?8 WHERE id=?9",
        params![name, paths_json, page_count, total_size, folder_id, last_page, finished, now, survivor],
    )
    .map_err(|e| e.to_string())?;
    for snap in &snaps {
        if let Some(id) = snap.source_id {
            if id == survivor {
                continue;
            }
            ignore_delete(conn, "DELETE FROM pages_meta WHERE book_id=?1", id);
            ignore_delete(conn, "DELETE FROM edited_pages WHERE book_id=?1", id);
            ignore_delete(conn, "DELETE FROM progress WHERE book_id=?1", id);
            conn.execute("DELETE FROM shelf WHERE id=?1", [id]).map_err(|e| e.to_string())?;
            let _ = conn.execute(
                "UPDATE book_merges SET undone=1 WHERE book_id=?1 AND undone=0",
                [id],
            );
        }
    }
    ignore_delete(conn, "DELETE FROM pages_meta WHERE book_id=?1", survivor);
    ignore_delete(conn, "DELETE FROM edited_pages WHERE book_id=?1", survivor);
    ignore_delete(conn, "DELETE FROM progress WHERE book_id=?1", survivor);
    if let Some((page, frac)) = progress {
        let _ = conn.execute(
            "INSERT INTO progress (book_id, page, frac, updated_at) VALUES (?1,?2,?3,?4)",
            params![survivor, page, frac, now],
        );
    }
    let _ = conn.execute(
        "UPDATE book_merges SET undone=1 WHERE book_id=?1 AND undone=0",
        [survivor],
    );
    conn.execute(
        "INSERT INTO book_merges (book_id, title, auto, undone, created_at) VALUES (?1,?2,?3,0,?4)",
        params![survivor, name, if auto { 1 } else { 0 }, now],
    )
    .map_err(|e| e.to_string())?;
    let merge_id = conn.last_insert_rowid();
    for (ord, snap) in snaps.iter().enumerate() {
        let paths_json = serde_json::to_string(&snap.paths).unwrap_or_else(|_| "[]".into());
        conn.execute(
            "INSERT INTO book_merge_parts (merge_id, ord, source_id, name, kind, paths, page_count, total_size, last_page, finished, folder_id, progress_page, progress_frac, progress_updated)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                merge_id,
                ord as i64,
                snap.source_id,
                snap.name,
                snap.kind,
                paths_json,
                snap.page_count,
                snap.total_size,
                snap.last_page,
                snap.finished,
                snap.folder_id,
                snap.progress_page,
                snap.progress_frac,
                snap.progress_updated,
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(survivor)
}

fn apply_parts(
    conn: &Connection,
    title: &str,
    folder_override: Option<i64>,
    members: &[Member],
    auto: bool,
) -> Result<i64, String> {
    conn.execute_batch("SAVEPOINT merge_apply").map_err(|e| e.to_string())?;
    match apply_parts_inner(conn, title, folder_override, members, auto) {
        Ok(id) => {
            conn.execute_batch("RELEASE SAVEPOINT merge_apply").map_err(|e| e.to_string())?;
            drop_cover(id);
            Ok(id)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK TO SAVEPOINT merge_apply; RELEASE SAVEPOINT merge_apply;");
            Err(e)
        }
    }
}

fn load_parts(conn: &Connection, merge_id: i64) -> Result<Vec<SnapPart>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT source_id, name, kind, paths, page_count, total_size, last_page, finished, folder_id, progress_page, progress_frac, progress_updated
             FROM book_merge_parts WHERE merge_id=?1 ORDER BY ord",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([merge_id], |r| {
            let paths_json: String = r.get(3)?;
            Ok(SnapPart {
                source_id: r.get(0)?,
                name: r.get(1)?,
                kind: r.get(2)?,
                paths: serde_json::from_str(&paths_json).unwrap_or_default(),
                page_count: r.get(4)?,
                total_size: r.get(5)?,
                last_page: r.get(6)?,
                finished: r.get(7)?,
                folder_id: r.get(8)?,
                progress_page: r.get(9)?,
                progress_frac: r.get(10)?,
                progress_updated: r.get(11)?,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())
}

fn unmerge_inner(conn: &Connection, book_id: i64) -> Result<(), String> {
    let (merge_id,): (i64,) = conn
        .query_row(
            "SELECT id FROM book_merges WHERE book_id=?1 AND undone=0 ORDER BY id DESC LIMIT 1",
            [book_id],
            |r| Ok((r.get(0)?,)),
        )
        .map_err(|_| "这本书不是合并来的".to_string())?;
    let mut parts = load_parts(conn, merge_id)?;
    if parts.len() < 2 {
        return Err("合并记录不完整".into());
    }
    let merged_paths: Vec<String> = conn
        .query_row("SELECT paths FROM shelf WHERE id=?1", [book_id], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())
        .map(|s| serde_json::from_str(&s).unwrap_or_default())?;
    let merged_finished: i64 = conn
        .query_row("SELECT finished FROM shelf WHERE id=?1", [book_id], |r| r.get(0))
        .unwrap_or(0);
    let current = conn
        .query_row(
            "SELECT page, frac, updated_at FROM progress WHERE book_id=?1",
            [book_id],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, f64>(1)?, r.get::<_, i64>(2)?)),
        )
        .optional()
        .unwrap_or(None);
    if let Some((page, frac, updated)) = current {
        let mut offset = 0i64;
        let mut hit = None;
        for (i, part) in parts.iter().enumerate() {
            let count = part.page_count.max(0);
            let end = offset + count;
            let inside = if count == 0 {
                page == offset
            } else {
                page >= offset && page < end
            };
            if inside || (hit.is_none() && i + 1 == parts.len() && page >= offset) {
                hit = Some((i, offset));
                if inside {
                    break;
                }
            }
            offset = end;
        }
        if let Some((i, offset)) = hit {
            let count = parts[i].page_count.max(0);
            let local = if count == 0 { 0 } else { (page - offset).clamp(0, count - 1) };
            parts[i].progress_page = Some(local);
            parts[i].progress_frac = Some(frac);
            parts[i].progress_updated = Some(updated);
            parts[i].last_page = local;
            if merged_finished == 1 && count > 0 && local >= count - 1 {
                parts[i].finished = 1;
            }
        }
    }
    let marks = bookmark_rows(conn, book_id);
    let mut created: Vec<(i64, Vec<String>)> = Vec::new();
    for part in &parts {
        let page = part.progress_page.unwrap_or(part.last_page);
        let id = insert_folder_row(
            conn,
            &part.name,
            &part.paths,
            part.page_count,
            part.total_size,
            page.max(0),
            part.folder_id,
            part.finished,
        )?;
        if let Some(pp) = part.progress_page {
            let _ = conn.execute(
                "INSERT INTO progress (book_id, page, frac, updated_at) VALUES (?1,?2,?3,?4)",
                params![id, pp, part.progress_frac.unwrap_or(0.0), part.progress_updated.unwrap_or_else(crate::now_secs)],
            );
        }
        created.push((id, part.paths.clone()));
    }
    for (bid, name) in marks {
        let mut placed = false;
        for (new_id, paths) in &created {
            if let Some(next) = restored_entry_name(&name, paths, &merged_paths) {
                let _ = conn.execute(
                    "UPDATE bookmarks SET book_id=?1, entry_name=?2 WHERE id=?3",
                    params![new_id, next, bid],
                );
                placed = true;
                break;
            }
        }
        if !placed {
            let _ = conn.execute("DELETE FROM bookmarks WHERE id=?1", [bid]);
        }
    }
    ignore_delete(conn, "DELETE FROM bookmarks WHERE book_id=?1", book_id);
    ignore_delete(conn, "DELETE FROM pages_meta WHERE book_id=?1", book_id);
    ignore_delete(conn, "DELETE FROM edited_pages WHERE book_id=?1", book_id);
    ignore_delete(conn, "DELETE FROM progress WHERE book_id=?1", book_id);
    conn.execute("DELETE FROM shelf WHERE id=?1", [book_id]).map_err(|e| e.to_string())?;
    conn.execute("UPDATE book_merges SET undone=1 WHERE id=?1", [merge_id]).map_err(|e| e.to_string())?;
    drop_cover(book_id);
    Ok(())
}

fn unmerge_conn(conn: &Connection, book_id: i64) -> Result<(), String> {
    conn.execute_batch("SAVEPOINT merge_undo").map_err(|e| e.to_string())?;
    match unmerge_inner(conn, book_id) {
        Ok(()) => conn.execute_batch("RELEASE SAVEPOINT merge_undo").map_err(|e| e.to_string()),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK TO SAVEPOINT merge_undo; RELEASE SAVEPOINT merge_undo;");
            Err(e)
        }
    }
}

fn folder_ids_by_path(conn: &Connection) -> HashMap<String, i64> {
    let mut out = HashMap::new();
    let Ok(mut stmt) = conn.prepare("SELECT id, paths FROM shelf WHERE kind='folder'") else {
        return out;
    };
    let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))) else {
        return out;
    };
    for (id, json) in rows.flatten() {
        let paths: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
        for p in paths {
            out.entry(path_key(&p)).or_insert(id);
        }
    }
    out
}

fn group_key(members: &[PendingMember]) -> String {
    let mut keys: Vec<String> = members
        .iter()
        .map(|m| path_key(&m.path))
        .filter(|k| !k.is_empty())
        .collect();
    keys.sort();
    keys.dedup();
    keys.join("\n")
}

fn store_pending(conn: &Connection, groups: &[Group], replace_all: bool) -> Result<(), String> {
    if replace_all {
        conn.execute("DELETE FROM merge_pending WHERE resolved=0", [])
            .map_err(|e| e.to_string())?;
    }
    let mut existing: HashMap<String, i64> = HashMap::new();
    if !replace_all {
        if let Ok(mut stmt) = conn.prepare("SELECT id, payload FROM merge_pending WHERE resolved=0") {
            if let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))) {
                for (id, payload) in rows.flatten() {
                    if let Ok(g) = serde_json::from_str::<PendingGroup>(&payload) {
                        existing.insert(group_key(&g.members), id);
                    }
                }
            }
        }
    }
    let now = crate::now_secs();
    for g in groups {
        let payload = serde_json::to_string(&to_pending(g)).map_err(|e| e.to_string())?;
        let key = group_key(&to_pending(g).members);
        if let Some(id) = existing.get(&key) {
            conn.execute(
                "UPDATE merge_pending SET payload=?1, created_at=?2 WHERE id=?3",
                params![payload, now, id],
            )
            .map_err(|e| e.to_string())?;
        } else {
            conn.execute(
                "INSERT INTO merge_pending (created_at, resolved, payload) VALUES (?1, 0, ?2)",
                params![now, payload],
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn mark_already(snap: &mut ScanSnapshot, paths: &[String]) {
    let keys: HashSet<String> = paths.iter().map(|p| path_key(p)).collect();
    for book in &mut snap.books {
        if keys.contains(&path_key(&book.path)) {
            book.already = true;
        }
    }
}

/// After a finished scan: auto-merge confident groups and record the rest.
pub fn finish_scan(conn: &Connection, snap: &mut ScanSnapshot) -> Result<(), String> {
    let fresh: Vec<Member> = snap
        .books
        .iter()
        .filter(|b| !b.already)
        .map(member_from_scan)
        .collect();
    if fresh.is_empty() {
        snap.added = 0;
        snap.merged = 0;
        snap.pending = 0;
        return Ok(());
    }
    let library = load_folder_members(conn);
    let groups = group_relevant(&fresh, &library);
    let auto = auto_on(conn);
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let mut added = 0u64;
    let mut merged = 0u64;
    let mut pending = Vec::new();
    let mut consumed = Vec::new();
    for g in groups {
        if g.confident && auto {
            match apply_parts(&tx, &g.title, None, &g.members, true) {
                Ok(_) => {
                    merged += 1;
                    if g.members.iter().all(|m| m.shelf_id.is_none()) {
                        added += 1;
                    }
                    for m in &g.members {
                        consumed.extend(m.paths.clone());
                    }
                }
                Err(_) => pending.push(g),
            }
        } else {
            pending.push(g);
        }
    }
    store_pending(&tx, &pending, true)?;
    tx.commit().map_err(|e| e.to_string())?;
    mark_already(snap, &consumed);
    snap.added = added;
    snap.merged = merged;
    snap.pending = pending.len() as u64;
    Ok(())
}

/// Group folders just inserted by import. Failures stay on the shelf as separate books.
pub fn absorb_new(conn: &Connection, new_ids: &[i64]) -> Result<(), String> {
    if new_ids.is_empty() {
        return Ok(());
    }
    let want: HashSet<i64> = new_ids.iter().copied().collect();
    let library = load_folder_members(conn);
    let fresh: Vec<Member> = library.iter().filter(|m| m.shelf_id.is_some_and(|id| want.contains(&id))).cloned().collect();
    if fresh.is_empty() {
        return Ok(());
    }
    let groups = group_relevant(&fresh, &library);
    if groups.is_empty() {
        return Ok(());
    }
    let auto = auto_on(conn);
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let mut pending = Vec::new();
    for g in groups {
        if g.confident && auto {
            if apply_parts(&tx, &g.title, None, &g.members, true).is_err() {
                pending.push(g);
            }
        } else {
            pending.push(g);
        }
    }
    store_pending(&tx, &pending, false)?;
    tx.commit().map_err(|e| e.to_string())
}

fn reorder_members(members: Vec<Member>, order: Option<Vec<usize>>) -> Vec<Member> {
    let Some(order) = order else { return members };
    if order.len() != members.len() {
        return members;
    }
    let mut seen = HashSet::new();
    let mut next = Vec::new();
    for i in order {
        if i >= members.len() || !seen.insert(i) {
            return members;
        }
        next.push(members[i].clone());
    }
    next
}

fn attach_shelf(conn: &Connection, members: &mut [Member], folder_id: Option<i64>) -> Result<(), String> {
    let known = folder_ids_by_path(conn);
    for m in members.iter_mut() {
        if m.shelf_id.is_some() {
            continue;
        }
        let Some(path) = m.paths.first().cloned() else { continue };
        if let Some(id) = known.get(&path_key(&path)) {
            m.shelf_id = Some(*id);
            continue;
        }
        let id = insert_folder_row(conn, &m.name, &m.paths, m.pages, 0, 0, folder_id, 0)?;
        m.shelf_id = Some(id);
    }
    Ok(())
}

#[tauri::command]
pub fn merge_auto_get() -> Result<bool, String> {
    let conn = crate::db_conn()?;
    Ok(auto_on(&conn))
}

#[tauri::command]
pub fn merge_auto_set(on: bool) -> Result<(), String> {
    let conn = crate::db_conn()?;
    ensure_tables(&conn)?;
    conn.execute(
        "INSERT INTO settings(key, value) VALUES('merge.auto', ?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![if on { "1" } else { "0" }],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn list_merge_pending() -> Result<Vec<PendingGroup>, String> {
    let conn = crate::db_conn()?;
    ensure_tables(&conn)?;
    let mut stmt = conn
        .prepare("SELECT id, payload FROM merge_pending WHERE resolved=0 ORDER BY id")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows.flatten() {
        let mut g: PendingGroup = match serde_json::from_str(&row.1) {
            Ok(g) => g,
            Err(_) => continue,
        };
        g.id = row.0;
        out.push(g);
    }
    Ok(out)
}

#[tauri::command]
pub fn resolve_merge_pending(
    id: i64,
    action: String,
    title: Option<String>,
    folder_id: Option<i64>,
    order: Option<Vec<usize>>,
) -> Result<i64, String> {
    let conn = crate::db_conn()?;
    let payload: String = conn
        .query_row(
            "SELECT payload FROM merge_pending WHERE id=?1 AND resolved=0",
            [id],
            |r| r.get(0),
        )
        .map_err(|_| "没有这组待确认的合并".to_string())?;
    let group: PendingGroup = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let result = if action == "separate" {
        for m in &group.members {
            if m.shelf_id.is_some() || m.path.is_empty() {
                continue;
            }
            let known = folder_ids_by_path(&tx);
            if known.contains_key(&path_key(&m.path)) {
                continue;
            }
            insert_folder_row(&tx, &m.name, &[m.path.clone()], m.pages, 0, 0, folder_id, 0)?;
        }
        0
    } else if action == "merge" {
        let mut members: Vec<Member> = group.members.iter().map(member_from_pending).collect();
        members = reorder_members(members, order);
        attach_shelf(&tx, &mut members, folder_id)?;
        let title = title.unwrap_or(group.suggested_title);
        apply_parts(&tx, &title, folder_id, &members, false)?
    } else {
        return Err("未知操作".into());
    };
    tx.execute("UPDATE merge_pending SET resolved=1 WHERE id=?1", [id])
        .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(result)
}

#[tauri::command]
pub fn merge_books(title: String, folder_id: Option<i64>, members: Vec<MergeIn>) -> Result<i64, String> {
    if members.len() < 2 {
        return Err("请至少选择两本文件夹漫画".into());
    }
    let conn = crate::db_conn()?;
    let mut parts = Vec::new();
    for m in &members {
        let snap = load_snap(&conn, m.shelf_id)?;
        parts.push(Member {
            shelf_id: Some(m.shelf_id),
            name: if m.name.trim().is_empty() { snap.name.clone() } else { m.name.clone() },
            paths: if snap.paths.is_empty() { vec![m.path.clone()] } else { snap.paths.clone() },
            pages: snap.page_count,
            chapters: 0,
            cover_ver: 0,
            parsed: parse_name(if m.name.trim().is_empty() { &snap.name } else { &m.name }),
        });
    }
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let id = apply_parts(&tx, &title, folder_id, &parts, false)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(id)
}

#[tauri::command]
pub fn unmerge_book(book_id: i64) -> Result<(), String> {
    let conn = crate::db_conn()?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    unmerge_conn(&tx, book_id)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(name: &str) -> Member {
        Member {
            shelf_id: None,
            name: name.to_string(),
            paths: vec![format!("C:/{name}")],
            pages: 10,
            chapters: 1,
            cover_ver: 0,
            parsed: parse_name(name),
        }
    }

    fn grouped(names: &[&str]) -> Vec<Group> {
        let pool: Vec<Member> = names.iter().map(|n| mem(n)).collect();
        group_members(&pool)
    }

    #[test]
    fn chinese_and_english_names_parse_and_group() {
        let a = parse_name("海贼王 第1-50话");
        let b = parse_name("海贼王 第51-100话");
        assert_eq!(a.base, b.base);
        assert_eq!(a.base, "海贼王");
        assert_eq!(a.order, Some((1, 50)));
        assert_eq!(b.order, Some((51, 100)));
        assert_eq!(a.display, "海贼王");
        let g = grouped(&["海贼王 第51-100话", "海贼王 第1-50话"]);
        assert_eq!(g.len(), 1);
        assert!(g[0].confident);
        assert_eq!(g[0].title, "海贼王");
        assert!(g[0].members[0].name.contains("第1-50"));
        assert!(g[0].members[1].name.contains("第51-100"));

        let g = grouped(&["XXX 卷2", "XXX 卷1"]);
        assert!(g[0].confident);
        assert_eq!(g[0].members[0].parsed.order, Some((1, 1)));
        assert_eq!(g[0].members[1].parsed.order, Some((2, 2)));

        let g = grouped(&["XXX (下)", "XXX (上)"]);
        assert!(g[0].confident);
        assert_eq!(g[0].members[0].parsed.order, Some((1, 1)));
        assert_eq!(g[0].members[1].parsed.order, Some((3, 3)));
        assert!(g[0].members[0].name.contains('上'));

        let g = grouped(&["XXX_part2", "XXX_part1"]);
        assert!(g[0].confident);
        assert!(g[0].members[0].name.contains("part1"));
        assert!(g[0].members[1].name.contains("part2"));

        assert_eq!(parse_name("海贼王 第１话").base, parse_name("海贼王 第1话").base);
        assert_eq!(parse_name("海贼王 第１话").order, Some((1, 1)));

        let g = grouped(&["海贼王 第1-50话", "海贼王 第40-60话"]);
        assert_eq!(g.len(), 1);
        assert!(!g[0].confident);

        let g = grouped(&["海贼王", "海贼王 第1话"]);
        assert_eq!(g.len(), 1);
        assert!(!g[0].confident);

        let g = grouped(&["One Piece Vol. 10", "One Piece Vol. 2"]);
        assert!(g[0].confident);
        assert_eq!(g[0].title, "One Piece");
        assert!(g[0].members[0].name.contains("Vol. 2"));
        assert_eq!(g[0].members[0].parsed.order, Some((2, 2)));
        assert_eq!(g[0].members[1].parsed.order, Some((10, 10)));

        let g = grouped(&["Foo (2)", "Foo (1)"]);
        assert!(g[0].confident);
        assert_eq!(g[0].members[0].parsed.order, Some((1, 1)));
        assert_eq!(g[0].members[1].parsed.order, Some((2, 2)));

        assert!(parse_name("《二十四孝》").order.is_none());
        assert_eq!(parse_name("《二十四孝》").base, "二十四孝");
        assert!(parse_name("上海").order.is_none());
        assert_eq!(parse_name("上海").base, "上海");
        assert!(parse_name("vol01-10").base.is_empty());
        assert!(grouped(&["vol01-10", "vol01-10"]).is_empty());
    }

    #[test]
    fn folder_parts_prefix_each_root() {
        let root = std::env::temp_dir().join(format!("mh-parts-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let a = root.join("PartA");
        let b = root.join("PartB");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("1.jpg"), b"x").unwrap();
        std::fs::write(b.join("2.jpg"), b"y").unwrap();
        let pages = crate::scan::scan_folder_parts(&[
            a.to_string_lossy().into_owned(),
            b.to_string_lossy().into_owned(),
        ]);
        assert_eq!(pages.len(), 2);
        assert!(pages[0].name.starts_with("01-PartA/"), "{}", pages[0].name);
        assert!(pages[1].name.starts_with("02-PartB/"), "{}", pages[1].name);
        let s0 = pages[0].src_path.clone().unwrap_or_default();
        let s1 = pages[1].src_path.clone().unwrap_or_default();
        assert!(s0.contains("PartA"), "{s0}");
        assert!(s1.contains("PartB"), "{s1}");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn schema(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE shelf (
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
                folder_id INTEGER,
                scan_sig TEXT,
                finished INTEGER NOT NULL DEFAULT 0,
                cover_ver INTEGER NOT NULL DEFAULT 1
            );
            CREATE TABLE progress(book_id INTEGER PRIMARY KEY, page INTEGER NOT NULL, frac REAL NOT NULL, updated_at INTEGER NOT NULL);
            CREATE TABLE bookmarks(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                book_id INTEGER NOT NULL,
                entry_name TEXT NOT NULL,
                page_hint INTEGER NOT NULL DEFAULT 0,
                frac REAL NOT NULL DEFAULT 0,
                note TEXT,
                created_at INTEGER NOT NULL
            );",
        )
        .unwrap();
        ensure_tables(conn).unwrap();
    }

    #[test]
    fn merge_and_unmerge_keep_the_files() {
        let stamp = std::process::id();
        let root = std::env::temp_dir().join(format!("mh-merge-{stamp}"));
        let _ = std::fs::remove_dir_all(&root);
        let a = root.join("海贼王 第1-50话");
        let b = root.join("海贼王 第51-100话");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let file = a.join("001.jpg");
        std::fs::write(&file, b"still-here").unwrap();
        let conn = Connection::open_in_memory().unwrap();
        schema(&conn);
        let id1 = insert_folder_row(&conn, "海贼王 第1-50话", &[a.to_string_lossy().into_owned()], 10, 0, 0, None, 0).unwrap();
        let id2 = insert_folder_row(&conn, "海贼王 第51-100话", &[b.to_string_lossy().into_owned()], 12, 0, 0, None, 0).unwrap();
        conn.execute(
            "INSERT INTO progress (book_id, page, frac, updated_at) VALUES (?1, 3, 0.2, 50)",
            [id2],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO bookmarks (book_id, entry_name, page_hint, frac, note, created_at) VALUES (?1, '001.jpg', 0, 0, NULL, 1)",
            [id1],
        )
        .unwrap();
        let members = vec![
            Member {
                shelf_id: Some(id1),
                name: "海贼王 第1-50话".into(),
                paths: vec![a.to_string_lossy().into_owned()],
                pages: 10,
                chapters: 50,
                cover_ver: 1,
                parsed: parse_name("海贼王 第1-50话"),
            },
            Member {
                shelf_id: Some(id2),
                name: "海贼王 第51-100话".into(),
                paths: vec![b.to_string_lossy().into_owned()],
                pages: 12,
                chapters: 50,
                cover_ver: 1,
                parsed: parse_name("海贼王 第51-100话"),
            },
        ];
        let id = apply_parts(&conn, "海贼王", None, &members, true).unwrap();
        assert!(file.exists());
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM shelf", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 1);
        let paths: String = conn.query_row("SELECT paths FROM shelf WHERE id=?1", [id], |r| r.get(0)).unwrap();
        assert!(paths.contains("第1-50"));
        assert!(paths.contains("第51-100"));
        let page: i64 = conn.query_row("SELECT page FROM progress WHERE book_id=?1", [id], |r| r.get(0)).unwrap();
        assert_eq!(page, 13);
        let entry: String = conn.query_row("SELECT entry_name FROM bookmarks WHERE book_id=?1", [id], |r| r.get(0)).unwrap();
        assert!(entry.starts_with("01-"), "{entry}");
        assert!(entry.ends_with("001.jpg"), "{entry}");
        unmerge_conn(&conn, id).unwrap();
        assert!(file.exists());
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM shelf", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2);
        let entry: String = conn.query_row("SELECT entry_name FROM bookmarks", [], |r| r.get(0)).unwrap();
        assert_eq!(entry, "001.jpg");
        let restored: i64 = conn
            .query_row(
                "SELECT page FROM progress ORDER BY page DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(restored, 3);
        let _ = std::fs::remove_dir_all(&root);
    }
}
