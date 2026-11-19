// The sidebar entry embeds one fixed official URL. It does not fetch the site,
// parse it, or save pictures into the library.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const URL = 'https://coserslove.com/cosers';
const shell = fs.readFileSync(path.join(root, 'src/shell-ui.js'), 'utf8');
const page = fs.readFileSync(path.join(root, 'src/cosers.js'), 'utf8');
const html = fs.readFileSync(path.join(root, 'src/cosers.html'), 'utf8');
const rust = fs.readFileSync(path.join(root, 'src-tauri/src/site_view.rs'), 'utf8');
const caps = fs.readFileSync(path.join(root, 'src-tauri/capabilities/default.json'), 'utf8');

const rail = shell.slice(shell.indexOf('const RAIL'), shell.indexOf('const searchCtx'));
const keys = [...rail.matchAll(/key: '(\w+)'/g)].map((m) => m[1]);
assert.deepEqual(keys, ['home', 'library', 'meta', 'recent', 'marked', 'import', 'cosers']);
assert.match(rail, /href: 'cosers\.html'/);
assert.match(rail, /icon: 'image'/);
assert.match(shell, /base === 'cosers\.html'/);
assert.match(html, /id="cosersHost"/);
assert.match(page, /site_view_place/);
assert.match(page, /site_view_close/);
assert.equal(page.includes(URL), true);
assert.equal(rust.includes(URL), true);
assert.match(rust, /fn site_view_place\([\s\S]*?\) -> Result/);
assert.equal(/url\s*:/.test(rust.slice(rust.indexOf('fn site_view_place'), rust.indexOf('fn site_view_close'))), false);
assert.equal(page.includes('fetch('), false);
assert.equal(/18comic|adblock|scrape|gallery/i.test(page + rust), false);
assert.equal(caps.includes('coserslove.com'), false);
assert.equal(caps.includes('"remote"'), false);

function chromeShell() {
  const chrome = ['google-chrome', 'chromium', 'chromium-browser'].find((b) => spawnSync('which', [b]).status === 0);
  if (!chrome) return null;
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'mh-cosers-'));
  const open = (file) => {
    const run = spawnSync(chrome, [
      '--headless=new', '--disable-gpu', '--no-sandbox', '--disable-dev-shm-usage',
      '--allow-file-access-from-files', `--user-data-dir=${path.join(dir, 'profile')}`,
      '--window-size=1100,800', '--virtual-time-budget=2500', '--dump-dom', file,
    ], { encoding: 'utf8', timeout: 20000 });
    return run.stdout || '';
  };
  const cosers = open(pathToFileURL(path.join(root, 'src/cosers.html')).href);
  const home = open(pathToFileURL(path.join(root, 'src/home.html')).href);
  return { cosers, home };
}

const dom = chromeShell();
if (dom) {
  assert.match(dom.cosers, /href="cosers\.html"/);
  assert.match(dom.cosers, /href="home\.html"/);
  assert.match(dom.cosers, /href="library\.html"/);
  assert.match(dom.cosers, /id="cosersHost"/);
  assert.match(dom.cosers, /在应用内打开官网/);
  assert.equal(dom.cosers.includes('18comic'), false);
  assert.match(dom.home, /class="hero/);
  assert.match(dom.home, /href="cosers\.html"/);
  assert.equal(dom.home.includes('id="cosersHost"'), false);
  console.log('cosers shell checked in chrome');
} else {
  console.log('cosers shell chrome check skipped');
}

console.log('test-cosers-view ok');
