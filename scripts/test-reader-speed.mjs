// Zoom stays on one transform until the gesture ends, and the strip only
// builds boxes for the viewport window. Both are pure enough to assert in
// node; a headless Chrome pass times a real DOM when a browser is installed.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { Layout } from '../src/layout.js';
import {
  PAGE_WINDOW_CAP, ZOOM_COMMIT_MS, clampZoom, pageWindow, zoomOriginY, zoomPreviewScale,
} from '../src/reader.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

assert.equal(ZOOM_COMMIT_MS, 140);
assert.equal(clampZoom(9), 3);
assert.equal(clampZoom(0.01), 0.5);
assert.equal(clampZoom(1.234), 1.23);
assert.equal(clampZoom(Number.NaN), 1);
assert.equal(zoomPreviewScale(1, 1.1), 1.1);
assert.equal(zoomPreviewScale(2, 2), 1);
assert.equal(zoomOriginY(100, 40), 140);
assert.equal(zoomOriginY(-30, 10), 0);

function book(n) {
  const L = new Layout();
  const dims = Array.from({ length: n }, () => [800, 1200]);
  const src = Array.from({ length: n }, (_, i) => i);
  L.setBook(dims, src);
  L.compute({ viewW: 900, widthPct: 100, maxW: Infinity, zoom: 1, gap: 0, endPad: 0, spread: false });
  return L;
}

{
  const L = book(1500);
  const viewH = 900;
  const scroll = 40000;
  const win = pageWindow(L, scroll, viewH, 4.2);
  assert.ok(win.count > 0 && win.count <= PAGE_WINDOW_CAP, JSON.stringify(win));
  assert.ok(win.count < L.n / 4, `window ${win.count} should be far smaller than ${L.n}`);
  const mid = L.findVirtual(scroll + viewH / 2);
  assert.ok(win.from <= mid && mid <= win.to, `viewport page ${mid} left the window ${win.from}..${win.to}`);
  assert.deepEqual(pageWindow(L, 0, 0, 4.2), { from: 0, to: L.n - 1, count: L.n });
  assert.deepEqual(pageWindow({ n: 0 }, 0, 800, 4.2), { from: 0, to: -1, count: 0 });

  const reps = 40;
  let sink = 0;
  const tAll = performance.now();
  for (let k = 0; k < reps; k++) {
    for (let i = 0; i < L.n; i++) sink += L.vTop[i] + L.vH[i];
  }
  const allMs = performance.now() - tAll;
  const tWin = performance.now();
  for (let k = 0; k < reps; k++) {
    const w = pageWindow(L, scroll, viewH, 4.2);
    for (let i = w.from; i <= w.to; i++) sink += L.vTop[i] + L.vH[i];
  }
  const winMs = performance.now() - tWin;
  console.log(`BENCH layout-math pages=${L.n} window=${win.count} full_ms=${allMs.toFixed(2)} window_ms=${winMs.toFixed(2)} sink=${sink}`);
  assert.ok(winMs < allMs, `window walk ${winMs}ms should beat a full walk ${allMs}ms`);
}

function chromeBench() {
  const chrome = ['google-chrome', 'chromium', 'chromium-browser'].find((b) => spawnSync('which', [b]).status === 0);
  if (!chrome) return null;
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'mh-speed-'));
  const file = path.join(dir, 'index.html');
  const readerUrl = pathToFileURL(path.join(root, 'src/reader.js')).href;
  const cssUrl = pathToFileURL(path.join(root, 'src/mh-ui.css')).href;
  const n = 400;
  fs.writeFileSync(file, `<!doctype html><html><head>
<link rel="stylesheet" href="${cssUrl}">
</head><body>
<div id="host" style="position:relative;width:420px;height:520px"></div>
<pre id="out"></pre>
<script>
window.__frames = [];
window.requestAnimationFrame = (fn) => { window.__frames.push(fn); return window.__frames.length; };
window.cancelAnimationFrame = () => {};
window.__timers = [];
window.setTimeout = (fn) => { window.__timers.push(fn); return window.__timers.length; };
window.clearTimeout = () => {};
</script>
<script type="module">
import { Reader } from ${JSON.stringify(readerUrl)};
const host = document.getElementById('host');
const reader = new Reader(host);
const dims = Array.from({ length: ${n} }, () => [800, 1200]);
const src = Array.from({ length: ${n} }, (_, i) => i);
reader.params.maxW = Infinity;
reader.params.gap = 0;
reader.strip.padVh = 4.2;
reader.strip.build(dims, src);
reader.strip.apply(reader.layoutParams());
const live = reader.strip.liveCount;
const sample = reader.strip.pages.find((p) => p);
const topBefore = sample ? sample.style.top : '';
const tPreview = performance.now();
for (let k = 0; k < 8; k++) {
  reader.previewZoom(1 + (k + 1) * 0.08, 80);
  const batch = window.__frames.splice(0);
  for (const fn of batch) fn(performance.now());
}
const previewMs = performance.now() - tPreview;
const transform = reader.strip.sizer.style.transform;
const topDuring = sample.style.top;
const tCommit = performance.now();
reader.commitZoom();
const commitMs = performance.now() - tCommit;
reader.setZoom(1, 0);
const tFull = performance.now();
for (let k = 0; k < 8; k++) reader.setZoom(1 + (k + 1) * 0.08, 80);
const fullMs = performance.now() - tFull;
const nodes = [];
for (let i = 0; i < ${n}; i++) {
  const d = document.createElement('div');
  d.className = 'page';
  host.append(d);
  nodes.push(d);
}
const tOld = performance.now();
for (let k = 0; k < 8; k++) {
  for (const d of nodes) {
    d.style.top = (k * 10) + 'px';
    d.style.height = (600 + k) + 'px';
    d.style.left = '0px';
    d.style.width = '400px';
  }
}
const oldWriteMs = performance.now() - tOld;
const tNew = performance.now();
for (let k = 0; k < 8; k++) reader.strip.sizer.style.transform = 'scale(' + (1 + k * 0.08) + ')';
const newWriteMs = performance.now() - tNew;
document.getElementById('out').textContent = JSON.stringify({
  live, n: ${n}, topBefore, topDuring, transform,
  previewMs, commitMs, fullMs, oldWriteMs, newWriteMs,
  zoom: reader.params.zoom,
  boxes: reader.strip.liveCount,
});
</script></body></html>`);
  const run = spawnSync(chrome, [
    '--headless=new', '--disable-gpu', '--no-sandbox', '--disable-dev-shm-usage',
    '--allow-file-access-from-files', `--user-data-dir=${path.join(dir, 'profile')}`,
    '--window-size=800,700', '--virtual-time-budget=1000', '--dump-dom', `file://${file}`,
  ], { encoding: 'utf8', timeout: 20000 });
  const text = run.stdout || '';
  const m = text.match(/<pre id="out">([^<]*)<\/pre>/);
  if (!m) throw new Error('chrome speed fixture produced no measurements:\n' + text.slice(0, 500));
  return JSON.parse(m[1].replaceAll('&quot;', '"').replaceAll('&amp;', '&'));
}

const dom = chromeBench();
if (dom) {
  console.log(`BENCH dom live=${dom.live}/${dom.n} preview_ms=${dom.previewMs.toFixed(2)} commit_ms=${dom.commitMs.toFixed(2)} relayout8_ms=${dom.fullMs.toFixed(2)} writeAll_ms=${dom.oldWriteMs.toFixed(2)} writeTransform_ms=${dom.newWriteMs.toFixed(2)} transform=${dom.transform}`);
  assert.ok(dom.live > 0 && dom.live < dom.n / 2, `expected a window of boxes, got ${dom.live}/${dom.n}`);
  assert.equal(dom.topBefore, dom.topDuring, 'a zoom preview must not rewrite page boxes');
  assert.match(dom.transform, /^scale\(/);
  assert.ok(dom.newWriteMs < dom.oldWriteMs, `one transform ${dom.newWriteMs}ms should beat rewriting every page ${dom.oldWriteMs}ms`);
  assert.ok(Math.abs(dom.zoom - 1.64) < 0.02, `commit should land on the last live zoom, got ${dom.zoom}`);
} else {
  console.log('BENCH dom skipped (no chrome)');
}

console.log('test-reader-speed ok');
