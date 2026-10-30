// v0.7.8: key tap/hold share one velocity. Chrome is a sticky toggle, not a scroll timer.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  KEY_NUDGE_MIN_PX, KEY_NUDGE_PX_PER_SEC, KEY_PAGE_PX_PER_SEC, KEY_PAGE_TRAVEL_VH,
  keyScrollPlan,
} from '../src/reader.js';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const src = fs.readFileSync(path.join(root, 'src/reader.js'), 'utf8');
const boot = fs.readFileSync(path.join(root, 'src/reader-boot.js'), 'utf8');
const css = fs.readFileSync(path.join(root, 'src/mh-ui.css'), 'utf8');

// Tap and hold must use the same plan. OS key-repeat must not queue CSS smooth jumps.
{
  const tap = keyScrollPlan(' ', { viewH: 800 });
  const hold = keyScrollPlan('PageDown', { viewH: 800 });
  assert.equal(tap.pxPerSec, hold.pxPerSec);
  assert.equal(tap.pxPerSec, KEY_PAGE_PX_PER_SEC);
  assert.equal(tap.minTravel, 800 * KEY_PAGE_TRAVEL_VH);
  assert.equal(tap.dir, 1);
  assert.equal(keyScrollPlan(' ', { shift: true, viewH: 800 }).dir, -1);
  assert.equal(keyScrollPlan('PageUp', { viewH: 900 }).dir, -1);
  const nudge = keyScrollPlan('ArrowDown', { viewH: 800 });
  assert.equal(nudge.pxPerSec, KEY_NUDGE_PX_PER_SEC);
  assert.equal(nudge.minTravel, KEY_NUDGE_MIN_PX);
  assert.equal(keyScrollPlan('ArrowUp').dir, -1);
  assert.equal(keyScrollPlan('Home'), null);
}

assert.match(src, /if \(e\.repeat\) return/);
assert.match(src, /beginKeyScroll/);
assert.match(src, /requestAnimationFrame\(tick\)/);
assert.match(src, /stopKeyScroll/);
assert.equal(/case ' ':.*behavior:\s*'smooth'/.test(src), false);
assert.equal(/case 'PageDown':.*behavior:\s*'smooth'/.test(src), false);
assert.equal(/case 'ArrowDown':\s*sc\.scrollBy/.test(src), false);

assert.equal(boot.includes('CHROME_HIDE_AFTER_MS'), false);
assert.equal(boot.includes('CHROME_SHOW_AFTER_MS'), false);
assert.equal(boot.includes('noteChromeScroll'), false);
assert.equal(boot.includes('onChromeScrollActivity'), false);
assert.match(boot, /toggleReaderChrome/);
assert.match(boot, /isReaderBlankTarget/);
assert.match(boot, /applyReaderChromeClasses/);
assert.match(css, /calc\(var\(--p, 0\) \* 100%\)/);
assert.match(css, /body\.reader-chrome-away \.shell-top/);
assert.match(css, /\.rg\.chrome-away \.mp__head\{display:none\}/);

console.log('scroll v0.7.8 checks passed');
