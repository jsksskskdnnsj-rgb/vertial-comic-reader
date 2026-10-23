#!/usr/bin/env node
// Applies the whole v0.6 set to a repo, idempotently. Run from the REPO ROOT:
//   node <path-to-set>/scripts/apply-v06.mjs [--dry]
// 1 copy src/*.js|html, assets/*.jpg -> src/assets/, scripts/*.mjs (existing different files are first saved to .v06-backup/)
// 2 replace/append the "v0.6 append" block at the end of src/mh-ui.css
// 3 add / refresh the 39 locale keys in src/en.js and src/zh-CN.js (flat `export default {...}`)
// 4 add the icon symbols to src/icons.svg when missing
// 5 patch reader-boot.js / reader-tools.js (apply-reader-patches.mjs)
// 6 patch src-tauri/src/main.rs (apply-rust-patch.mjs: separate banner image, 3 new commands, set_data_dir migration); skip with --no-rust
// then run:  node scripts/verify-refs.mjs --manifest
import fs from 'node:fs'; import path from 'node:path'; import { fileURLToPath } from 'node:url'; import { spawnSync } from 'node:child_process';
const SET = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const ROOT = process.cwd(); const DRY = process.argv.includes('--dry');
if (path.resolve(SET) === path.resolve(ROOT)) { console.error('Run this from the repo root, not from inside the set.'); process.exit(2); }
const say = (m) => console.log(m);
const rd = (p) => fs.readFileSync(p, 'utf8');
function put(dest, data) { if (DRY) return; fs.mkdirSync(path.dirname(dest), { recursive: true }); fs.writeFileSync(dest, data); }
function copyFile(from, to) {
  const buf = fs.readFileSync(from);
  if (fs.existsSync(to)) {
    const old = fs.readFileSync(to);
    if (Buffer.compare(old, buf) === 0) return 'same';
    const bk = path.join(ROOT, '.v06-backup', path.relative(ROOT, to));
    if (!fs.existsSync(bk)) put(bk, old);
    put(to, buf); return 'overwritten';
  }
  put(to, buf); return 'new';
}
// 1 files
const tally = { same: 0, overwritten: 0, new: 0 };
const list = (dir, re) => fs.existsSync(dir) ? fs.readdirSync(dir).filter((f) => re.test(f)) : [];
for (const f of list(path.join(SET, 'src'), /\.(js|html)$/)) { const r = copyFile(path.join(SET, 'src', f), path.join(ROOT, 'src', f)); tally[r]++; if (r !== 'same') say(`  ${r}: src/${f}`); }
for (const f of list(path.join(SET, 'assets'), /\.jpg$/)) { const r = copyFile(path.join(SET, 'assets', f), path.join(ROOT, 'src', 'assets', f)); tally[r]++; if (r !== 'same') say(`  ${r}: src/assets/${f}`); }
for (const f of list(path.join(SET, 'scripts'), /\.(mjs|allow\.example)$/)) { const r = copyFile(path.join(SET, 'scripts', f), path.join(ROOT, 'scripts', f)); tally[r]++; if (r !== 'same') say(`  ${r}: scripts/${f}`); }
say(`files: ${tally.new} new, ${tally.overwritten} overwritten, ${tally.same} unchanged`);
// 2 css
{
  const cssP = path.join(ROOT, 'src', 'mh-ui.css'), block = rd(path.join(SET, 'src', 'mh-ui.v06-append.css')).replace(/\s+$/, '') + '\n';
  if (!fs.existsSync(cssP)) { say('css: src/mh-ui.css missing'); process.exit(1); }
  let css = rd(cssP); const i = css.indexOf('/* ==== v0.6 append');
  const base = (i >= 0 ? css.slice(0, i) : css).replace(/\s+$/, '') + '\n\n';
  put(cssP, base + block); say(`css: v0.6 block ${i >= 0 ? 'replaced' : 'appended'}`);
}
// 3 locale
{
  const add = rd(path.join(SET, 'src', 'locale-additions.txt'));
  const blocks = { 'en.js': add.split('=== en.js ===')[1].split('=== zh-CN.js ===')[0], 'zh-CN.js': add.split('=== zh-CN.js ===')[1] };
  for (const [file, block] of Object.entries(blocks)) {
    const p = path.join(ROOT, 'src', file); let s = rd(p), added = 0, refreshed = 0;
    const lines = block.split(/\r?\n/).filter((l) => /^\s*'[\w.]+'\s*:/.test(l));
    const fresh = [];
    for (const line of lines) {
      const key = line.match(/'([\w.]+)'/)[1];
      const re = new RegExp(`^[ \\t]*['"]${key.replace(/\./g, '\\.')}['"][ \\t]*:.*$`, 'm');
      if (re.test(s)) { const before = s; s = s.replace(re, () => line.replace(/\s+$/, '')); if (s !== before) refreshed++; } else fresh.push(line.replace(/\s+$/, ''));
    }
    if (fresh.length) {
      const end = s.lastIndexOf('}'); if (end < 0) { say(`locale: no closing brace in ${file}`); process.exit(1); }
      let head = s.slice(0, end).replace(/\s+$/, ''); if (!head.endsWith(',') && !head.endsWith('{')) head += ',';
      s = head + '\n' + fresh.join('\n') + '\n' + s.slice(end); added = fresh.length;
    }
    put(p, s); say(`locale ${file}: ${added} added, ${refreshed} refreshed (${lines.length} keys in the set)`);
  }
}
// 4 icons
{
  const p = path.join(ROOT, 'src', 'icons.svg'); let s = rd(p), n = 0;
  for (const m of rd(path.join(SET, 'src', 'icons-additions.txt')).matchAll(/<symbol id="([\w-]+)"[^\n]*<\/symbol>/g)) {
    if (!s.includes(`id="${m[1]}"`)) { const e = s.lastIndexOf('</svg>'); s = s.slice(0, e) + m[0] + '\n' + s.slice(e); n++; }
  }
  put(p, s); say(`icons: ${n} symbol(s) added`);
}
// 5 reader patches
if (!DRY) { const r = spawnSync(process.execPath, [path.join(ROOT, 'scripts', 'apply-reader-patches.mjs')], { cwd: ROOT, encoding: 'utf8' }); process.stdout.write(r.stdout || ''); process.stderr.write(r.stderr || ''); if (r.status) process.exitCode = 1; }
// 6 rust
if (!DRY && !process.argv.includes('--no-rust')) {
  if (fs.existsSync(path.join(ROOT, 'src-tauri', 'src', 'main.rs'))) {
    const r = spawnSync(process.execPath, [path.join(ROOT, 'scripts', 'apply-rust-patch.mjs')], { cwd: ROOT, encoding: 'utf8' });
    process.stdout.write(r.stdout || ''); process.stderr.write(r.stderr || ''); if (r.status) process.exitCode = 1;
  } else say('rust: src-tauri/src/main.rs not found, skipped');
}
say(DRY ? 'dry run: nothing written' : 'done. Next: (cd src-tauri && cargo check), then node scripts/verify-refs.mjs --manifest');
