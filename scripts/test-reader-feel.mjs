// Hand-feel: wheel notches glide, trackpad streams stay native, page joins
// overlap by one pixel, and flip gestures turn one page per flick.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { Layout } from '../src/layout.js';
import {
  FLIP_WHEEL_NOTCH_LOCK_MS, FLIP_WHEEL_TRACK_PX,
  classifyWheel, createFlipWheelState, flipSwipeDir, flipVisualSign, flipWheelAdvance,
  glideChase, glideTauMs, keyReleaseSpeed, pageSeamCover,
} from '../src/reader-feel.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

function book(n, w = 800, h = 1200) {
  const L = new Layout();
  L.setBook(Array.from({ length: n }, () => [w, h]), Array.from({ length: n }, (_, i) => i));
  return L;
}

// A mouse notch is one spaced, large, integer delta. A trackpad is a burst
// of small or fractional deltas, often on both axes.
{
  assert.equal(classifyWheel({ deltaY: 100, deltaX: 0, deltaMode: 0 }, 0, 1000), 'notch');
  assert.equal(classifyWheel({ deltaY: -120, deltaX: 0, deltaMode: 0 }, 1000, 1300), 'notch');
  assert.equal(classifyWheel({ deltaY: 3, deltaX: 0, deltaMode: 1 }, 0, 500), 'notch');
  assert.equal(classifyWheel({ deltaY: 1, deltaX: 0, deltaMode: 2 }, 0, 500), 'notch');
  assert.equal(classifyWheel({ deltaY: 4.5, deltaX: 0, deltaMode: 0 }, 1000, 1016), 'trackpad');
  assert.equal(classifyWheel({ deltaY: 12, deltaX: -6, deltaMode: 0 }, 1000, 1010), 'trackpad');
  assert.equal(classifyWheel({ deltaY: 80, deltaX: 0, deltaMode: 0 }, 1000, 1020), 'trackpad');
  assert.equal(classifyWheel({ deltaY: 16, deltaX: 0, deltaMode: 0 }, 1000, 2000), 'trackpad');
}

// One notch eases most of the way inside a short tau and then snaps.
// A second target update (another notch) does not restart from zero.
{
  const tau = glideTauMs(200);
  assert.equal(tau, 70);
  let y = 0;
  y = glideChase(y, 200, 16, tau);
  assert.ok(y > 30 && y < 80, `first frame ${y}`);
  const mid = y;
  y = glideChase(y, 400, 16, glideTauMs(400 - y));
  assert.ok(y > mid, 'a stacked notch keeps moving forward');
  assert.ok(y < 400, 'chase does not overshoot');
  assert.equal(glideChase(399.8, 400, 16, 70), 400);
  assert.ok(glideTauMs(5000) > glideTauMs(200));
  assert.equal(glideTauMs(5000), 240);
}

// Release coast keeps full speed until the last stretch, then eases to zero.
{
  assert.equal(keyReleaseSpeed(1800, 500), 1800);
  assert.equal(keyReleaseSpeed(1800, 70, 140), 900);
  assert.equal(keyReleaseSpeed(1800, 0), 0);
  assert.equal(keyReleaseSpeed(0, 40), 0);
}

// Seamless pages (and slices of one tall image) cover the join. A chosen gap does not.
{
  const L = book(4);
  L.compute({ viewW: 900, widthPct: 100, maxW: Infinity, gap: 0 });
  assert.equal(pageSeamCover(L, 0), 0);
  assert.equal(pageSeamCover(L, 1), 1);
  assert.equal(pageSeamCover(L, 3), 1);
  assert.equal(L.vTop[1], L.vTop[0] + L.vH[0]);
  const slices = new Layout();
  slices.setBook([[800, 4000], [800, 4000], [800, 1200]], [0, 0, 1]);
  slices.compute({ viewW: 900, widthPct: 100, maxW: Infinity, gap: 12 });
  assert.equal(pageSeamCover(slices, 1), 1, 'slices of one page still meet');
  assert.equal(pageSeamCover(slices, 2), 0, 'the chosen gap stays open');
  L.compute({ viewW: 900, widthPct: 100, maxW: Infinity, gap: 16 });
  assert.equal(pageSeamCover(L, 1), 0);
  L.compute({ viewW: 1000, widthPct: 100, maxW: Infinity, gap: 0, spread: true, spreadGap: 8 });
  assert.equal(L.vTop[0], L.vTop[1]);
  assert.equal(pageSeamCover(L, 2), 1, 'the next row covers the join above it');
  assert.equal(pageSeamCover(L, 0), 0);
}

// Flip: a notch turns once and then locks. A trackpad flick sums to one page.
{
  let state = createFlipWheelState();
  let step = flipWheelAdvance(state, { deltaY: 100, deltaX: 0, deltaMode: 0 }, 1000);
  assert.equal(step.dir, 1);
  step = flipWheelAdvance(step.state, { deltaY: 100, deltaX: 0, deltaMode: 0 }, 1000 + FLIP_WHEEL_NOTCH_LOCK_MS - 10);
  assert.equal(step.dir, 0);
  step = flipWheelAdvance(step.state, { deltaY: -100, deltaX: 0, deltaMode: 0 }, 2000);
  assert.equal(step.dir, -1);

  state = createFlipWheelState();
  let acc = state;
  let turned = 0;
  for (let i = 0; i < 12; i++) {
    const r = flipWheelAdvance(acc, { deltaY: 10, deltaX: 0, deltaMode: 0 }, 3000 + i * 12);
    acc = r.state;
    turned += r.dir;
  }
  assert.equal(turned, 1);
  assert.ok(Math.abs(acc.acc) < FLIP_WHEEL_TRACK_PX);
  const during = flipWheelAdvance(acc, { deltaY: 40, deltaX: 0, deltaMode: 0 }, 3200);
  assert.equal(during.dir, 0);
}

// Swipe follows the page-turn direction. A vertical drag does not turn.
{
  assert.equal(flipSwipeDir(-80, 6, 140, false), 1);
  assert.equal(flipSwipeDir(80, 6, 140, false), -1);
  assert.equal(flipSwipeDir(80, 6, 140, true), 1);
  assert.equal(flipSwipeDir(-80, 6, 140, true), -1);
  assert.equal(flipSwipeDir(-30, 2, 40, false), 1);
  assert.equal(flipSwipeDir(-12, 0, 80, false), 0);
  assert.equal(flipSwipeDir(-40, 90, 100, false), 0);
  assert.equal(flipVisualSign(1, false), 1);
  assert.equal(flipVisualSign(1, true), -1);
  assert.equal(flipVisualSign(-1, true), 1);
}

const reader = fs.readFileSync(path.join(root, 'src/reader.js'), 'utf8');
const flip = fs.readFileSync(path.join(root, 'src/flip.js'), 'utf8');
const css = fs.readFileSync(path.join(root, 'src/mh-ui.css'), 'utf8');

assert.match(reader, /classifyWheel/);
assert.match(reader, /glideBy/);
assert.match(reader, /pageSeamCover/);
assert.match(reader, /keyReleaseSpeed/);
assert.equal(reader.includes("decoding = syncDecode ? 'sync' : 'async'"), false);
assert.equal(/scrollTo\(\{[^}]*behavior:\s*'smooth'/.test(reader), false);
assert.match(flip, /flipWheelAdvance/);
assert.match(flip, /flipSwipeDir/);
assert.match(flip, /set_reading_window|setWindow/);
assert.match(css, /\.scroller\{overflow-anchor:none\}/);
assert.match(css, /#host \.scroller\{filter:none\}/);
assert.match(css, /#host \.sizer\{filter:var\(--reader-filter\)\}/);
assert.match(css, /\.page img\.pg\{transition:none\}/);
assert.match(css, /\.flip-stage\{touch-action:none\}/);

// Headless Chrome: a real wheel notch glides the strip, a trackpad sample does
// not cancel the platform scroll, and abutting pages overlap by one pixel.
function chromeFeel() {
  const chrome = ['google-chrome', 'chromium', 'chromium-browser'].find((b) => spawnSync('which', [b]).status === 0);
  if (!chrome) return null;
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'mh-feel-'));
  const file = path.join(dir, 'index.html');
  const readerUrl = pathToFileURL(path.join(root, 'src/reader.js')).href;
  const cssUrl = pathToFileURL(path.join(root, 'src/mh-ui.css')).href;
  fs.writeFileSync(file, `<!doctype html><html><head>
<link rel="stylesheet" href="${cssUrl}">
</head><body>
<div id="host" style="position:relative;width:420px;height:520px"></div>
<pre id="out"></pre>
<script>
window.__frames = [];
window.requestAnimationFrame = (fn) => { window.__frames.push(fn); return window.__frames.length; };
window.cancelAnimationFrame = () => {};
</script>
<script type="module">
import { Reader } from ${JSON.stringify(readerUrl)};
const host = document.getElementById('host');
const reader = new Reader(host);
const sc = reader.strip.scroller;
const box = document.createElement('div');
box.style.height = '6000px';
reader.strip.sizer.append(box);
reader.wheelGear = 2;
const notch = new WheelEvent('wheel', { deltaY: 100, deltaX: 0, deltaMode: 0, bubbles: true, cancelable: true });
reader.onWheel(notch);
const track = new WheelEvent('wheel', { deltaY: 8.5, deltaX: 1.25, deltaMode: 0, bubbles: true, cancelable: true });
reader.onWheel(track);
let now = performance.now();
for (let i = 0; i < 24 && window.__frames.length; i++) {
  const batch = window.__frames.splice(0);
  now += 16;
  for (const fn of batch) fn(now);
}
const y = Math.round(sc.scrollTop);
reader.strip.build([[800, 1200], [800, 1200], [800, 1200]], [0, 1, 2]);
reader.params.gap = 0;
reader.params.maxW = Infinity;
reader.strip.apply(reader.layoutParams());
const page = (i) => reader.strip.pages[i].style;
document.getElementById('out').textContent = JSON.stringify({
  y,
  notchPrevented: notch.defaultPrevented,
  trackPrevented: track.defaultPrevented,
  scrollerFilter: getComputedStyle(sc).filter,
  sizerFilter: getComputedStyle(reader.strip.sizer).filter,
  anchor: getComputedStyle(sc).overflowAnchor,
  top1: page(1).top,
  h1: page(1).height,
  h0: page(0).height,
  vTop1: reader.strip.layout.vTop[1],
  vH1: reader.strip.layout.vH[1],
});
</script></body></html>`);
  const run = spawnSync(chrome, [
    '--headless=new', '--disable-gpu', '--no-sandbox', '--disable-dev-shm-usage',
    '--allow-file-access-from-files', `--user-data-dir=${path.join(dir, 'profile')}`,
    '--window-size=800,700', '--virtual-time-budget=1000', '--dump-dom', `file://${file}`,
  ], { encoding: 'utf8', timeout: 8000 });
  const text = run.stdout || '';
  const m = text.match(/<pre id="out">([^<]*)<\/pre>/);
  if (!m) throw new Error('chrome feel fixture produced no measurements:\n' + text.slice(0, 500));
  return JSON.parse(m[1].replaceAll('&quot;', '"').replaceAll('&amp;', '&'));
}

const feel = chromeFeel();
if (feel) {
  assert.equal(feel.notchPrevented, true);
  assert.equal(feel.trackPrevented, false);
  assert.ok(feel.y > 40 && feel.y < 220, `glide distance ${feel.y}`);
  assert.equal(feel.scrollerFilter, 'none');
  assert.equal(feel.anchor, 'none');
  assert.equal(feel.top1, (feel.vTop1 - 1) + 'px');
  assert.equal(feel.h1, (feel.vH1 + 1) + 'px');
  assert.equal(feel.h0, feel.vH1 + 'px');
  console.log('chrome feel', { y: feel.y, sizerFilter: feel.sizerFilter });
}

console.log('reader feel checks passed');
