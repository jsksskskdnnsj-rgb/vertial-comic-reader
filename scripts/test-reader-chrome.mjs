// Sticky reader chrome and the bottom scrubber. The thumb used to stay at the
// left because --p was a 0–1 fraction and CSS used it as a length.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import {
  isReaderBlankTarget, pageFromScrub, pageReadout, scrubProgress,
} from '../src/reader.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

{
  const shown = pageReadout(247, 2634);
  assert.deepEqual(shown, { page: 248, total: 2634, text: '248 / 2634' });
  assert.equal(pageReadout(7, 2634).text, '8 / 2634');
  assert.equal(pageReadout(0, 0).text, '- / -');
  const at = scrubProgress(247, 2634, 0);
  assert.ok(Math.abs(at - 247 / 2633) < 1e-12, at);
  assert.equal(pageFromScrub(at, 2634), 247);
  assert.equal(scrubProgress(0, 2634, 0), 0);
  assert.equal(pageFromScrub(0, 2634), 0);
  assert.equal(scrubProgress(2633, 2634, 0), 1);
  assert.equal(pageFromScrub(1, 2634), 2633);
  assert.ok(scrubProgress(2, 5, 0.5) > scrubProgress(2, 5, 0));
  assert.equal(isReaderBlankTarget(null), false);
}

function chromeSnap() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'mh-chrome-'));
  const file = path.join(dir, 'index.html');
  const css = pathToFileURL(path.join(root, 'src/mh-ui.css')).href;
  const reader = pathToFileURL(path.join(root, 'src/reader.js')).href;
  fs.writeFileSync(file, `<!doctype html><html><head>
<link rel="stylesheet" href="${css}">
<style>html,body{width:1200px;height:800px}</style>
</head><body>
<div class="app" style="height:800px">
  <header class="shell-top" id="shell-top">top</header>
  <div class="work">
    <nav class="shell-rail" id="shell-rail">rail</nav>
    <div class="rg" id="rg">
      <aside class="glass bp" id="bp">history</aside>
      <section class="glass mp" id="mp">
        <header class="mp__head" id="head">nav</header>
        <div class="viewport" id="vp">
          <div id="host"><div class="scroller"><div class="sizer"><div class="page"><img alt=""></div></div></div></div>
          <button class="chrome-toggle" id="chromeToggle" type="button">界面</button>
        </div>
      </section>
      <aside class="glass tb" id="tb">tools</aside>
      <footer class="glass bb" id="bb">
        <div class="bb__ep"><b id="bbEp"></b><span id="bbBook">title</span></div>
        <div class="bb__nav">
          <button class="ibtn" id="pgPrev" type="button">prev</button>
          <div class="scrub" id="track"><div class="scrub__rail"><div class="scrub__fill"></div></div><div class="scrub__thumb"></div></div>
          <button class="ibtn" id="pgNext" type="button">next</button>
          <span class="bb__count" id="bbCount"></span>
        </div>
      </footer>
      <section class="flyout" id="flySettings">settings</section>
    </div>
  </div>
</div>
<pre id="out"></pre>
<script type="module">
import { applyReaderChromeClasses, isReaderBlankTarget, pageReadout, scrubProgress } from ${JSON.stringify(reader)};
const disp = (id) => getComputedStyle(document.getElementById(id)).display;
const track = document.getElementById('track');
const info = pageReadout(247, 2634);
document.getElementById('bbEp').textContent = info.text;
document.getElementById('bbCount').textContent = info.text;
function ratio() {
  const thumb = track.querySelector('.scrub__thumb');
  const fill = track.querySelector('.scrub__fill');
  const rail = track.querySelector('.scrub__rail');
  const tr = track.getBoundingClientRect();
  const th = thumb.getBoundingClientRect();
  const fl = fill.getBoundingClientRect();
  const rl = rail.getBoundingClientRect();
  return {
    thumb: (th.left + th.width / 2 - tr.left) / tr.width,
    fill: rl.width ? fl.width / rl.width : -1,
  };
}
function place(page, n, frac) {
  track.style.setProperty('--p', String(scrubProgress(page, n, frac)));
  return ratio();
}
const count = document.getElementById('bbCount');
const cr = count.getBoundingClientRect();
const mid = place(2, 5, 0);
const midFrac = place(2, 5, 0.5);
const far = place(247, 2634, 0);
let clicks = [];
function toggleIfBlank(el) {
  if (!isReaderBlankTarget(el)) { clicks.push(false); return; }
  const on = !document.body.classList.contains('reader-chrome-away');
  applyReaderChromeClasses(document, on);
  clicks.push(true);
}
toggleIfBlank(document.querySelector('.page img'));
toggleIfBlank(document.querySelector('.scrub__thumb'));
toggleIfBlank(document.getElementById('pgNext'));
toggleIfBlank(document.querySelector('.sizer'));
const hidden = {
  bp: disp('bp'), tb: disp('tb'), bb: disp('bb'), head: disp('head'),
  top: disp('shell-top'), rail: disp('shell-rail'), toggle: disp('chromeToggle'),
  away: document.body.classList.contains('reader-chrome-away'),
};
toggleIfBlank(document.querySelector('.sizer'));
const back = {
  bp: disp('bp'), tb: disp('tb'), bb: disp('bb'), head: disp('head'),
  top: disp('shell-top'), rail: disp('shell-rail'), toggle: disp('chromeToggle'),
  away: document.body.classList.contains('reader-chrome-away'),
};
document.getElementById('out').textContent = JSON.stringify({
  text: info.text,
  same: document.getElementById('bbEp').textContent === count.textContent,
  countW: cr.width, countScroll: count.scrollWidth,
  mid, midFrac, far, clicks, hidden, back,
});
</script>
</body></html>`);
  const chrome = ['google-chrome', 'chromium', 'chromium-browser'].find((b) => spawnSync('which', [b]).status === 0);
  if (!chrome) return null;
  const run = spawnSync(chrome, [
    '--headless=new', '--disable-gpu', '--no-sandbox', '--disable-dev-shm-usage',
    '--allow-file-access-from-files', `--user-data-dir=${path.join(dir, 'profile')}`,
    '--window-size=1200,800', '--virtual-time-budget=2000', '--dump-dom', `file://${file}`,
  ], { encoding: 'utf8', timeout: 15000 });
  const text = run.stdout || '';
  const m = text.match(/<pre id="out">([^<]*)<\/pre>/);
  if (!m) throw new Error('chrome fixture produced no measurements:\n' + text.slice(0, 500) + '\n' + (run.stderr || '').slice(0, 400));
  return JSON.parse(m[1].replaceAll('&quot;', '"').replaceAll('&amp;', '&'));
}

const snap = chromeSnap();
if (!snap) {
  console.log('reader chrome checks skipped (no chrome)');
} else {
  assert.equal(snap.text, '248 / 2634');
  assert.equal(snap.same, true);
  assert.ok(snap.countW > 50, `page count clipped to ${snap.countW}px (scroll ${snap.countScroll})`);
  assert.ok(snap.countScroll - snap.countW < 2, `count overflow ${snap.countScroll} vs ${snap.countW}`);
  assert.ok(Math.abs(snap.mid.thumb - 0.5) < 0.04, `mid thumb ${snap.mid.thumb}`);
  assert.ok(Math.abs(snap.mid.fill - 0.5) < 0.04, `mid fill ${snap.mid.fill}`);
  assert.ok(snap.midFrac.thumb > snap.mid.thumb + 0.08, `frac did not move the thumb (${snap.mid.thumb} -> ${snap.midFrac.thumb})`);
  const expect = 247 / 2633;
  assert.ok(Math.abs(snap.far.thumb - expect) < 0.03, `page 248 thumb ${snap.far.thumb} expected ${expect}`);
  assert.ok(snap.far.thumb > 0.05 && snap.far.thumb < 0.2, `thumb still stuck (${snap.far.thumb})`);
  assert.ok(Math.abs(snap.far.fill - snap.far.thumb) < 0.03, `fill ${snap.far.fill} vs thumb ${snap.far.thumb}`);
  assert.deepEqual(snap.clicks, [false, false, false, true, true]);
  assert.equal(snap.hidden.away, true);
  assert.equal(snap.hidden.bp, 'none');
  assert.equal(snap.hidden.tb, 'none');
  assert.equal(snap.hidden.bb, 'none');
  assert.equal(snap.hidden.head, 'none');
  assert.equal(snap.hidden.top, 'none');
  assert.equal(snap.hidden.rail, 'none');
  assert.notEqual(snap.hidden.toggle, 'none');
  assert.equal(snap.back.away, false);
  assert.notEqual(snap.back.bp, 'none');
  assert.notEqual(snap.back.tb, 'none');
  assert.notEqual(snap.back.bb, 'none');
  assert.notEqual(snap.back.head, 'none');
  assert.notEqual(snap.back.top, 'none');
  assert.notEqual(snap.back.rail, 'none');
  assert.notEqual(snap.back.toggle, 'none');
  console.log('reader chrome', { far: snap.far.thumb, countW: snap.countW });
}

console.log('reader chrome checks passed');
