#!/usr/bin/env node
// MH Reader reference verifier. Run from the repo root:  node scripts/verify-refs.mjs [--manifest]
// Checks: Rust command surface, invoke() names, JS imports/exports, HTML/CSS references,
// icons.svg symbol ids, tauri.conf.json paths, i18n key parity and usage. No dependencies.
import fs from 'node:fs'; import path from 'node:path'; import crypto from 'node:crypto';
import { pathToFileURL } from 'node:url';

const ROOT = process.cwd();
const SRC = path.join(ROOT, 'src');
const RS = path.join(ROOT, 'src-tauri', 'src');
const COMMANDS = `pick_folder_paths list_images open_data_dir
shelf_list shelf_get shelf_remove shelf_clear get_data_dir pick_data_dir set_data_dir
get_reader_config set_reader_config get_theme_config set_theme_config set_hero_image reset_hero_image win_minimize
win_toggle_maximize win_close take_drop_queue folder_add folder_list folder_rename folder_remove shelf_move open_book
rescan_book close_book set_reading_window save_progress load_progress fix_page_dims cache_stats cache_set_limit
cache_clear get_chapter_defaults set_chapter_defaults set_book_chapter_settings clear_book_chapter_settings
episode_info add_bookmark list_bookmarks delete_bookmark update_bookmark_note reanchor_bookmark bookmark_counts
ensure_cover list_recent_books list_recent_added import_paths reveal_book relink_book reorder_pages delete_pages
append_pages reset_page_edits page_edit_info shelf_rename book_reset_progress check_books_missing import_merged
library_stats set_banner_image reset_banner_image get_default_data_dir win_start_drag win_hide_if_tray app_exit
scan_begin scan_status scan_cancel list_scan_issues folder_reorder import_scan_books
merge_auto_get merge_auto_set list_merge_pending resolve_merge_pending merge_books unmerge_book`.split(/\s+/);

const results = []; let failures = 0, warnings = 0;
// Optional allow-list scripts/verify-refs.allow: lines "<text in message> :: <more text>" downgrade matching
// FAILs (known, accepted repo conditions) to ALLW warnings; # starts a comment.
const ALLOW = (() => { const p = path.join(ROOT, 'scripts', 'verify-refs.allow'); return fs.existsSync(p) ? fs.readFileSync(p, 'utf8').split(/\r?\n/).map((l) => l.trim()).filter((l) => l && !l.startsWith('#')).map((l) => l.split('::').map((x) => x.trim())) : []; })();
const fail = (sec, msg) => {
  if (ALLOW.some(([a, b]) => msg.includes(a) && (!b || msg.includes(b)))) { warnings++; results.push(['ALLW', sec, msg + '  (allowed)']); return; }
  failures++; results.push(['FAIL', sec, msg]);
};
const warn = (sec, msg) => { warnings++; results.push(['WARN', sec, msg]); };
const ok = (sec, msg) => results.push(['OK  ', sec, msg]);
const rd = p => fs.readFileSync(p, 'utf8');
const walk = (d, filt) => !fs.existsSync(d) ? [] : fs.readdirSync(d, { withFileTypes: true }).flatMap(e =>
  e.isDirectory() ? (['node_modules', 'target', '.git'].includes(e.name) ? [] : walk(path.join(d, e.name), filt))
    : filt(e.name) ? [path.join(d, e.name)] : []);
const rel = p => path.relative(ROOT, p).split(path.sep).join('/');
const stripComments = s => s.replace(/\/\*[\s\S]*?\*\//g, '').replace(/(^|[^:'"`\\])\/\/.*$/gm, '$1');

// ---- 1. Rust command surface --------------------------------------------------------------
{
  const sec = 'rust-commands';
  const files = walk(RS, n => n.endsWith('.rs'));
  const defined = new Set();
  for (const f of files) {
    const s = stripComments(rd(f));
    for (const m of s.matchAll(/#\[\s*(?:tauri::)?command[^\]]*\]\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(\w+)/g)) defined.add(m[1]);
  }
  const mainPath = path.join(RS, 'main.rs'); const lib = path.join(RS, 'lib.rs');
  const reg = new Set();
  for (const f of [mainPath, lib].filter(fs.existsSync)) {
    const s = stripComments(rd(f));
    const m = s.match(/generate_handler!\s*\[([\s\S]*?)\]/);
    if (m) for (const e of m[1].split(',')) { const n = e.trim().split('::').pop(); if (n) reg.add(n); }
  }
  if (!files.length) fail(sec, 'no .rs files under src-tauri/src');
  else {
    const missingDef = COMMANDS.filter(c => !defined.has(c));
    const missingReg = COMMANDS.filter(c => !reg.has(c));
    const extraReg = [...reg].filter(c => !COMMANDS.includes(c));
    const unregistered = [...defined].filter(c => !reg.has(c));
    missingDef.forEach(c => fail(sec, `command "${c}" has no #[tauri::command] fn`));
    missingReg.forEach(c => fail(sec, `command "${c}" not in generate_handler!`));
    extraReg.forEach(c => warn(sec, `registered command "${c}" is outside the agreed command surface (72 + win_start_drag, win_hide_if_tray, app_exit)`));
    unregistered.forEach(c => warn(sec, `#[tauri::command] fn "${c}" is not registered`));
    ok(sec, `${defined.size} command fns, ${reg.size} registered, surface=${COMMANDS.length}`);
  }
}

// ---- 2. Frontend files --------------------------------------------------------------------
const jsFiles = walk(SRC, n => /\.(m?js)$/.test(n));
const htmlFiles = walk(SRC, n => n.endsWith('.html'));
const cssFiles = walk(SRC, n => n.endsWith('.css'));
const exportsOf = new Map();
function exportsFor(file) {
  if (exportsOf.has(file)) return exportsOf.get(file);
  const s = stripComments(rd(file)); const set = new Set();
  for (const m of s.matchAll(/export\s+(?:async\s+)?(?:function\*?|class|const|let|var)\s+([\w$]+)/g)) set.add(m[1]);
  for (const m of s.matchAll(/export\s*\{([^}]*)\}(?:\s*from\s*['"][^'"]+['"])?/g))
    for (const part of m[1].split(',')) { const p = part.trim(); if (!p) continue; const as = p.split(/\s+as\s+/); set.add((as[1] || as[0]).trim()); }
  if (/export\s+default\b/.test(s)) set.add('default');
  if (/export\s*\*\s*from/.test(s)) set.add('*');
  exportsOf.set(file, set); return set;
}
{
  const sec = 'js-imports';
  let n = 0;
  for (const f of jsFiles) {
    const s = stripComments(rd(f));
    const re = /import\s*(?:([\w$*\s{},]+?)\s*from\s*)?['"]([^'"]+)['"]|import\(\s*['"]([^'"]+)['"]\s*\)/g;
    for (const m of s.matchAll(re)) {
      const spec = m[2] || m[3]; if (!spec || !spec.startsWith('.')) continue; n++;
      const target = path.resolve(path.dirname(f), spec);
      if (!fs.existsSync(target)) { fail(sec, `${rel(f)} imports "${spec}" -> file not found`); continue; }
      const clause = m[1]; if (!clause) continue;
      const ex = exportsFor(target); if (ex.has('*')) continue;
      const named = clause.match(/\{([^}]*)\}/);
      if (named) for (const part of named[1].split(',')) {
        const nm = part.trim().split(/\s+as\s+/)[0].trim(); if (!nm) continue;
        if (!ex.has(nm)) fail(sec, `${rel(f)} imports { ${nm} } from "${spec}" but it is not exported`);
      }
      const def = clause.replace(/\{[^}]*\}/, '').replace(/\*\s*as\s+[\w$]+/, '').replace(/,/g, '').trim();
      if (def && !ex.has('default')) fail(sec, `${rel(f)} default-imports from "${spec}" which has no default export`);
    }
  }
  ok(sec, `${n} relative imports checked across ${jsFiles.length} JS files`);
}
{
  const sec = 'invoke-names';
  const known = new Set(COMMANDS); const used = new Map();
  for (const f of [...jsFiles, ...htmlFiles]) {
    const s = stripComments(rd(f));
    for (const m of s.matchAll(/\b(?:invoke|call|rpc)\s*\(\s*['"`]([a-z_0-9]+)['"`]/g)) {
      if (!used.has(m[1])) used.set(m[1], new Set()); used.get(m[1]).add(rel(f));
    }
    for (const m of s.matchAll(/\bapi\s*\.\s*([a-z_0-9]+)\s*\(/g)) if (known.has(m[1])) {
      if (!used.has(m[1])) used.set(m[1], new Set()); used.get(m[1]).add(rel(f));
    }
  }
  for (const [name, files] of used) if (!known.has(name) && name.includes('_'))
    fail(sec, `"${name}" is invoked in ${[...files].join(', ')} but is not in the agreed command surface`);
  const unused = COMMANDS.filter(c => !used.has(c));
  ok(sec, `${used.size} distinct command names found in frontend; ${unused.length} surface commands never referenced literally (may be wrapped dynamically)`);
  if (unused.length) warn(sec, `not referenced literally: ${unused.join(', ')}`);
}
{
  const sec = 'html-css-refs';
  const isExt = u => /^(https?:|data:|blob:|asset:|mailto:|#|about:|tauri:|\/\/)/i.test(u) || u.includes('://');
  const check = (from, u, what) => {
    if (!u || isExt(u)) return;
    const clean = u.split(/[?#]/)[0]; if (!clean) return;
    const target = clean.startsWith('/') ? path.join(SRC, clean) : path.resolve(path.dirname(from), clean);
    if (!fs.existsSync(target)) fail(sec, `${rel(from)} ${what} "${u}" not found`);
    return target;
  };
  const iconIds = new Map();
  const iconIdsFor = f => { if (!iconIds.has(f)) iconIds.set(f, new Set([...rd(f).matchAll(/<(?:symbol|g|svg)[^>]*\sid=["']([^"']+)["']/g)].map(m => m[1]))); return iconIds.get(f); };
  let n = 0;
  for (const f of htmlFiles) {
    const s = rd(f);
    for (const m of s.matchAll(/<(?:script|img|source|audio|video)\b[^>]*\ssrc=["']([^"']+)["']/gi)) { n++; check(f, m[1], 'src'); }
    for (const m of s.matchAll(/<link\b[^>]*\shref=["']([^"']+)["']/gi)) { n++; check(f, m[1], 'link href'); }
    for (const m of s.matchAll(/<a\b[^>]*\shref=["']([^"']+\.html[^"']*)["']/gi)) { n++; check(f, m[1], 'a href'); }
    for (const m of s.matchAll(/<use\b[^>]*href=["']([^"']+)["']/gi)) {
      n++; const [u, id] = m[1].split('#'); if (!u || !id) continue;
      const t = check(f, u, 'use href'); if (t && fs.existsSync(t) && !iconIdsFor(t).has(id)) fail(sec, `${rel(f)} uses icon "#${id}" missing in ${rel(t)}`);
    }
  }
  for (const f of jsFiles) {
    const s = stripComments(rd(f));
    for (const m of s.matchAll(/href=\\?["']([^"'\\]*icons\.svg)#([\w-]+)/g)) {
      const t = path.resolve(SRC, m[1]); if (fs.existsSync(t) && !iconIdsFor(t).has(m[2])) fail(sec, `${rel(f)} uses icon "#${m[2]}" missing in icons.svg`);
    }
    for (const m of s.matchAll(/\bicon\s*:\s*['"]([\w-]+)['"]/g)) { const t = path.join(SRC, 'icons.svg'); if (fs.existsSync(t) && !iconIdsFor(t).has(m[1])) warn(sec, `${rel(f)} icon: '${m[1]}' not found in icons.svg`); }
    for (const m of s.matchAll(/(?:location(?:\.href)?\s*=|\.assign\()\s*['"`]([\w./-]+\.html[^'"`]*)['"`]/g)) { n++; check(f, m[1], 'navigation'); }
  }
  for (const f of cssFiles) {
    const s = stripComments(rd(f));
    for (const m of s.matchAll(/url\(\s*['"]?([^'")]+)['"]?\s*\)/g)) { n++; check(f, m[1], 'url()'); }
    for (const m of s.matchAll(/@import\s+(?:url\()?['"]?([^'")\s;]+)/g)) { n++; check(f, m[1], '@import'); }
  }
  ok(sec, `${n} references checked`);
}
{
  const sec = 'tauri-conf';
  const p = path.join(ROOT, 'src-tauri', 'tauri.conf.json');
  if (!fs.existsSync(p)) fail(sec, 'src-tauri/tauri.conf.json missing');
  else {
    let c; try { c = JSON.parse(rd(p)); } catch (e) { fail(sec, 'invalid JSON: ' + e.message); }
    if (c) {
      const dist = c.build?.frontendDist ?? c.build?.distDir;
      if (dist) { const d = path.resolve(path.dirname(p), dist); fs.existsSync(d) ? ok(sec, `frontend dist "${dist}" exists`) : fail(sec, `frontend dist "${dist}" not found`); }
      for (const ic of c.bundle?.icon ?? c.tauri?.bundle?.icon ?? []) if (!fs.existsSync(path.resolve(path.dirname(p), ic))) fail(sec, `icon "${ic}" missing (are icons gitignored?)`);
      const gi = path.join(ROOT, '.gitignore');
      if (fs.existsSync(gi) && /icons/.test(rd(gi))) warn(sec, '.gitignore mentions icons: a fresh clone may not build');
    }
  }
}

// ---- 3. i18n ------------------------------------------------------------------------------
{
  const sec = 'i18n';
  const find = n => [path.join(SRC, 'locales', n), path.join(SRC, n)].find(fs.existsSync);
  const enP = find('en.js'), zhP = find('zh-CN.js');
  if (!enP || !zhP) fail(sec, 'en.js / zh-CN.js not found under src/ or src/locales/');
  else {
    const flat = (o, pre = '', out = {}) => { for (const [k, v] of Object.entries(o)) typeof v === 'object' && v && !Array.isArray(v) ? flat(v, pre + k + '.', out) : out[pre + k] = v; return out; };
    const load = async p => { const m = await import(pathToFileURL(p).href); const o = m.default ?? m.en ?? m.zh ?? m.dict ?? m.messages ?? Object.values(m).find(v => v && typeof v === 'object'); return flat(o); };
    try {
      const en = await load(enP), zh = await load(zhP);
      const ek = new Set(Object.keys(en)), zk = new Set(Object.keys(zh));
      for (const k of ek) if (!zk.has(k)) fail(sec, `key "${k}" missing in zh-CN`);
      for (const k of zk) if (!ek.has(k)) fail(sec, `key "${k}" missing in en`);
      for (const k of ek) { if (typeof en[k] === 'string' && !en[k].trim()) fail(sec, `en "${k}" is empty`); if (zk.has(k) && typeof zh[k] === 'string' && !zh[k].trim()) fail(sec, `zh-CN "${k}" is empty`); }
      for (const k of ek) if (zk.has(k) && typeof zh[k] === 'string' && zh[k] === en[k] && /[A-Za-z]{4,}/.test(en[k]) && !/^[\w.{}%\s/-]+$/.test('') ) warn(sec, `zh-CN "${k}" equals English text`);
      for (const k of ek) if (typeof zh[k] === 'string' && !/[\u4e00-\u9fff]/.test(zh[k]) && /[A-Za-z]{5,}/.test(zh[k]) && zh[k] !== en[k]) warn(sec, `zh-CN "${k}" has no Chinese characters`);
      for (const k of zk) if (typeof en[k] === 'string' && /[\u4e00-\u9fff]/.test(en[k])) fail(sec, `en "${k}" contains Chinese text`);
      const used = new Map();
      for (const f of [...jsFiles, ...htmlFiles]) {
        if (f === enP || f === zhP) continue; const s = rd(f);
        for (const m of s.matchAll(/data-i(?:18n)?(?:-[a-z]+)?=["']([\w.-]+)["']/g)) (used.get(m[1]) ?? used.set(m[1], new Set()).get(m[1])).add(rel(f));
        for (const m of s.matchAll(/\bt\(\s*['"`]([\w.-]+)['"`]/g)) (used.get(m[1]) ?? used.set(m[1], new Set()).get(m[1])).add(rel(f));
      }
      for (const [k, fs_] of used) if (!ek.has(k) && k.includes('.')) fail(sec, `key "${k}" used in ${[...fs_].join(', ')} is not defined`);
      ok(sec, `${ek.size} en keys, ${zk.size} zh-CN keys, ${used.size} keys used in code`);
    } catch (e) { fail(sec, 'could not load locale modules: ' + e.message); }
  }
}

// ---- 4. Manifest --------------------------------------------------------------------------
if (process.argv.includes('--manifest')) {
  const all = [...walk(SRC, () => true), ...walk(RS, () => true), ...['src-tauri/tauri.conf.json', 'src-tauri/Cargo.toml'].map(p => path.join(ROOT, p)).filter(fs.existsSync)];
  const m = all.sort().map(f => ({ path: rel(f), bytes: fs.statSync(f).size, sha256: crypto.createHash('sha256').update(fs.readFileSync(f)).digest('hex') }));
  fs.writeFileSync(path.join(ROOT, 'MANIFEST.json'), JSON.stringify({ generated: new Date().toISOString(), files: m }, null, 2));
  ok('manifest', `MANIFEST.json written with ${m.length} files`);
}
for (const [lv, sec, msg] of results) console.log(`${lv} [${sec}] ${msg}`);
console.log(`\n${failures} failure(s), ${warnings} warning(s)`);
process.exit(failures ? 1 : 0);
