// v0.7.7 scroll-mode checks: column width, wheel gear, and which pages mount
// while the strip is still moving. Chrome layout of .chrome-away is checked
// when google-chrome is on PATH (see the bottom of this file).
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { Layout } from '../src/layout.js';
import {
  PERF, WHEEL_GEAR_DEFAULT, virtualsNear, wheelGear, wheelMultiplier, wheelScrollDelta,
} from '../src/reader.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

function book(n, w = 1000, h = 1400) {
  const L = new Layout();
  L.setBook(Array.from({ length: n }, () => [w, h]), Array.from({ length: n }, (_, i) => i));
  return L;
}

// Flip mode omits maxW, so the historical 1000px cap stays there.
{
  const L = book(4);
  L.compute({ viewW: 1800, widthPct: 100 });
  assert.equal(L.colW, 1000);
}

// Scroll mode passes Infinity: 100% fills the pane, the slider still scales it,
// and a narrow pane is not forced wider than itself (except the 120px floor).
{
  const L = book(4);
  L.compute({ viewW: 1800, widthPct: 100, maxW: Infinity });
  assert.equal(L.colW, 1800);
  L.compute({ viewW: 1800, widthPct: 70, maxW: Infinity });
  assert.equal(L.colW, 1260);
  L.compute({ viewW: 360, widthPct: 100, maxW: Infinity });
  assert.equal(L.colW, 360);
  L.compute({ viewW: 200, widthPct: 40, maxW: Infinity });
  assert.equal(L.colW, 120);
}

// Width changes keep {page, frac}. That is the saved reading position.
{
  const L = book(12);
  L.compute({ viewW: 1000, widthPct: 100, maxW: Infinity, gap: 16 });
  const mid = L.getPosition(L.sizerH * 0.37);
  L.compute({ viewW: 1680, widthPct: 100, maxW: Infinity, gap: 16 });
  const back = L.getPosition(L.posToY(mid));
  assert.equal(back.page, mid.page);
  assert.ok(Math.abs(back.frac - mid.frac) < 0.01, `${back.frac} vs ${mid.frac}`);
}

// Default gear is one step faster than native. Gear 1 leaves the browser alone.
assert.equal(WHEEL_GEAR_DEFAULT, 2);
assert.equal(wheelGear(undefined), 2);
assert.equal(wheelGear(9), 5);
assert.equal(wheelMultiplier(1), 1);
assert.equal(wheelMultiplier(2), 2);
assert.equal(wheelMultiplier(5), 6);
assert.equal(wheelScrollDelta({ deltaX: 0, deltaY: 100, deltaMode: 0 }, 1), null);
assert.deepEqual(wheelScrollDelta({ deltaX: 0, deltaY: 100, deltaMode: 0 }, 2), { dx: 0, dy: 200 });
assert.deepEqual(wheelScrollDelta({ deltaX: 0, deltaY: 4, deltaMode: 1 }, 3, { w: 800, h: 600 }), { dx: 0, dy: 384 });
assert.deepEqual(
  wheelScrollDelta({ deltaX: 0, deltaY: 1, deltaMode: 2 }, 2, { w: 800, h: 600 }),
  { dx: 0, dy: 1200 },
);

// A page under the viewport is in the mount set immediately, including after a
// jump of several screens. Nothing here waits for a settle timer.
{
  const L = book(40, 800, 1200);
  const vh = 700;
  L.compute({ viewW: 900, widthPct: 100, maxW: Infinity });
  const at = (screens) => virtualsNear(L, screens * vh, vh, PERF.balanced.nearVh);
  const top = at(0);
  assert.ok(top.includes(0), 'first page mounts at the top');
  const far = at(12);
  assert.ok(!far.includes(0), 'a page many screens back is not required');
  const centerPage = L.findVirtual(12 * vh + vh / 2);
  assert.ok(far.includes(centerPage), `page ${centerPage} under the viewport must mount during the fling`);
  assert.ok(PERF.balanced.farVh > PERF.balanced.nearVh);
  assert.ok(PERF.saver.maxMounted < PERF.balanced.maxMounted);
  assert.equal(PERF.saver.variant, 1600);
  assert.equal(PERF.balanced.variant, 0);
}

const src = fs.readFileSync(path.join(root, 'src/reader.js'), 'utf8');
assert.match(src, /maxW: Infinity/);
assert.equal(src.includes('setGate'), false);
assert.match(src, /vh \* 0\.9/);
assert.match(src, /scrollBy\(\{ top: 80 \}\)/);
assert.match(src, /scrollBy\(\{ top: -80 \}\)/);

function chromeSnap(windowSize) {
  const [w, h] = windowSize.split(',').map(Number);
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'mh-scroll-'));
  const file = path.join(dir, 'index.html');
  fs.writeFileSync(file, `<!doctype html><html><head>
<link rel="stylesheet" href="file://${path.join(root, 'src/mh-ui.css')}">
<style>html,body{width:${w}px;height:${h}px}</style>
</head><body>
<div class="app" style="height:${h}px"><div class="work">
  <nav class="shell-rail"></nav>
  <div class="rg" id="rg">
    <aside class="glass bp" id="bp">history</aside>
    <section class="glass mp" id="mp"><header class="mp__head">head</header>
      <div class="viewport" id="vp"><div id="host"></div>
        <aside class="drawer" id="drawer"><div>bookmarks</div></aside>
      </div></section>
    <aside class="glass tb" id="tb">tools</aside>
    <footer class="glass bb" id="bb">bottom</footer>
    <section class="flyout" id="flySettings">settings</section>
    <section class="flyout" id="flyAuto">auto</section>
  </div></div></div>
<pre id="out"></pre>
<script>
const rg = document.getElementById('rg');
const st = (id) => getComputedStyle(document.getElementById(id));
const snap = (name) => {
  const mp = document.getElementById('mp').getBoundingClientRect();
  const g = rg.getBoundingClientRect();
  return { name, mpW: Math.round(mp.width), rgW: Math.round(g.width), mpH: Math.round(mp.height), rgH: Math.round(g.height),
    bp: st('bp').display, tb: st('tb').display, bb: st('bb').display,
    fly: st('flySettings').visibility, drawer: st('drawer').display };
};
const rows = [];
rows.push(snap('rest'));
rg.classList.add('show-bp');
rows.push(snap('show-bp'));
rg.classList.add('chrome-away');
rows.push(snap('away'));
rg.classList.add('no-bp');
rows.push(snap('away-nobp'));
rg.classList.remove('chrome-away');
rows.push(snap('nobp'));
document.getElementById('out').textContent = JSON.stringify(rows);
</script></body></html>`);
  const chrome = ['google-chrome', 'chromium', 'chromium-browser'].find((b) => spawnSync('which', [b]).status === 0);
  if (!chrome) return null;
  // Headless Chrome in this environment prints the DOM and then waits on D-Bus.
  // Kill it once the measurements are in stdout; a non-zero status is fine.
  const run = spawnSync(chrome, [
    '--headless=new', '--disable-gpu', '--no-sandbox', '--disable-dev-shm-usage',
    '--allow-file-access-from-files', `--user-data-dir=${path.join(dir, 'profile')}`,
    `--window-size=${windowSize}`, '--virtual-time-budget=1500', '--dump-dom', `file://${file}`,
  ], { encoding: 'utf8', timeout: 8000 });
  const text = run.stdout || '';
  const m = text.match(/<pre id="out">([^<]*)<\/pre>/);
  if (!m) throw new Error('chrome fixture produced no measurements:\n' + text.slice(0, 400) + '\n' + (run.stderr || '').slice(0, 400));
  return JSON.parse(m[1].replaceAll('&quot;', '"').replaceAll('&amp;', '&'));
}

const wide = chromeSnap('1600,900');
if (wide) {
  const rest = wide.find((r) => r.name === 'rest');
  const away = wide.find((r) => r.name === 'away');
  const nobp = wide.find((r) => r.name === 'nobp');
  assert.equal(rest.bp, 'flex');
  assert.notEqual(rest.tb, 'none');
  assert.notEqual(rest.bb, 'none');
  assert.equal(rest.fly, 'visible');
  assert.equal(rest.drawer, 'flex');
  assert.ok(rest.mpW < rest.rgW - 200, `resting pane ${rest.mpW} should leave room for the side bars inside ${rest.rgW}`);
  assert.equal(away.bp, 'none');
  assert.equal(away.tb, 'none');
  assert.equal(away.bb, 'none');
  assert.equal(away.fly, 'hidden');
  assert.equal(away.drawer, 'flex', 'bookmark drawer must stay openable');
  assert.ok(away.mpW >= away.rgW - 4, `tucked pane ${away.mpW} should fill the grid ${away.rgW}`);
  assert.ok(away.mpH > rest.mpH, 'hiding the bottom bar should give the page more height');
  assert.equal(nobp.bp, 'none', 'manual no-bp stays collapsed after scroll chrome returns');
  assert.notEqual(nobp.tb, 'none');
  assert.equal(nobp.fly, 'visible');
  console.log('chrome wide', { rest: rest.mpW, away: away.mpW, rg: away.rgW });
}

const narrow = chromeSnap('700,800');
if (narrow) {
  const rest = narrow.find((r) => r.name === 'rest');
  const shown = narrow.find((r) => r.name === 'show-bp');
  const away = narrow.find((r) => r.name === 'away');
  const back = narrow.find((r) => r.name === 'nobp');
  assert.equal(rest.bp, 'none', 'narrow history stays an overlay, hidden until asked');
  assert.equal(shown.bp, 'flex');
  assert.equal(away.bp, 'none', 'scroll chrome hides a narrow history overlay');
  assert.equal(away.drawer, 'flex');
  assert.equal(away.tb, 'none');
  assert.ok(away.mpW >= away.rgW - 4);
  assert.equal(back.bp, 'flex', 'show-bp comes back when scroll chrome does');
  console.log('chrome narrow ok');
}

console.log('scroll v0.7.7 checks passed');
