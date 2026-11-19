// Continuity: neighbor aspects, real dims without re-slicing, constant zoom gap,
// viewport windows, and mount sets that always include the screen.
import assert from 'node:assert/strict';
import { Layout, applyRealDims, fillAspects } from '../src/layout.js';
import { PAGE_WINDOW_CAP, pageWindow, pagesToMount, sameScroll } from '../src/reader.js';

function layout(dims, src, opts = {}) {
  const L = new Layout();
  L.setBook(dims, src ?? dims.map((_, i) => i));
  L.compute({ viewW: 800, widthPct: 100, maxW: Infinity, zoom: 1, gap: 0, endPad: 0, spread: false, ...opts });
  return L;
}

{
  const ar = fillAspects([[0, 0], [800, 1600], [0, 0], [900, 900]]);
  assert.equal(ar[0], 2);
  assert.equal(ar[1], 2);
  assert.equal(ar[2], 2);
  assert.equal(ar[3], 1);
  assert.deepEqual(fillAspects([[0, 0], [0, 0]]), [1.5, 1.5]);
}

{
  const L = layout([[0, 0], [800, 1600], [0, 0], [900, 900]]);
  assert.equal(L.vH[0], 1600);
  assert.equal(L.vH[1], 1600);
  assert.equal(L.vH[2], 1600);
  assert.equal(L.vH[3], 800);
}

{
  const L = layout([[800, 1600], [0, 0], [0, 0]]);
  assert.equal(L.vH[1], 1600);
  assert.equal(L.vH[2], 1600);
  L.setDim(1, 800, 800);
  L.compute({ viewW: 800, widthPct: 100, maxW: Infinity, zoom: 1, gap: 0, endPad: 0 });
  assert.equal(L.vH[1], 800, 'the decoded page takes its real ratio');
  assert.equal(L.vH[2], 1600, 'later unknown pages keep the height already on screen');
  L.setBook([[0, 0], [0, 0]], [0, 1]);
  L.compute({ viewW: 800, widthPct: 100, maxW: Infinity, zoom: 1, gap: 0, endPad: 0 });
  assert.equal(L.vH[0], 1200);
  assert.equal(L.vH[1], 1200);
}

{
  const L = layout([[800, 1000], [800, 1000], [800, 1000]], null, { gap: 16 });
  const pos = L.getPosition(L.vTop[2] + 40);
  assert.equal(L.shiftHeight(0, 500), -500);
  assert.equal(L.vH[0], 500);
  assert.equal(L.vTop[1] - (L.vTop[0] + L.vH[0]), 16, 'the gap stays the configured pixels');
  assert.equal(L.vTop[0], 0);
  const y = L.posToY(pos);
  const back = L.getPosition(y);
  assert.equal(back.page, pos.page);
  assert.ok(Math.abs(back.frac - pos.frac) < 0.02, `${pos.frac} -> ${back.frac}`);
  assert.equal(L.shiftHeight(1, L.vH[1]), 0);
  const spread = layout([[800, 1000], [800, 1000]], [0, 1], { spread: true });
  assert.equal(spread.shiftHeight(0, 400), null);
}

{
  const L = layout([[800, 8000], [800, 8000], [0, 0]], [0, 0, 1]);
  assert.equal(applyRealDims(L, [[800, 20000], [900, 1200]]), 1);
  assert.deepEqual(L.dims[0], [800, 8000]);
  assert.deepEqual(L.dims[1], [800, 8000]);
  assert.deepEqual(L.dims[2], [900, 1200]);
  assert.equal(L.n, 3, 'a tall real size does not re-slice the open session');
  assert.equal(applyRealDims(L, [[1, 1]]), 0);
  assert.equal(applyRealDims(L, null), 0);
}

{
  const L = layout([[800, 1200], [800, 1200]], [0, 1], { gap: 16, zoom: 1 });
  const gapAt = (zoom) => {
    L.compute({ viewW: 800, widthPct: 100, maxW: Infinity, zoom, gap: 16, endPad: 0 });
    return L.vTop[1] - (L.vTop[0] + L.vH[0]);
  };
  assert.equal(gapAt(1), 16);
  assert.equal(gapAt(1.8), 16);
  const anchorY = 40;
  const scroll = 200;
  const before = L.getPosition(scroll + anchorY);
  L.compute({ viewW: 800, widthPct: 100, maxW: Infinity, zoom: 1, gap: 16, endPad: 0 });
  const pos = L.getPosition(scroll + anchorY);
  L.compute({ viewW: 800, widthPct: 100, maxW: Infinity, zoom: 1.8, gap: 16, endPad: 0 });
  const y = L.posToY(pos) - anchorY;
  const back = L.getPosition(y + anchorY);
  assert.equal(back.page, pos.page);
  assert.ok(Math.abs(back.frac - pos.frac) < 0.02, `${before.frac} -> ${back.frac}`);
}

{
  const n = 200;
  const L = layout(Array.from({ length: n }, () => [100, 20]), null, { viewW: 100 });
  const viewH = 400;
  const scroll = 1000;
  const win = pageWindow(L, scroll, viewH, 10);
  const visFrom = L.findVirtual(scroll);
  const visTo = L.findVirtual(scroll + viewH);
  assert.ok(win.count <= PAGE_WINDOW_CAP, JSON.stringify(win));
  assert.ok(win.from <= visFrom && win.to >= visTo, `window ${win.from}..${win.to} missed ${visFrom}..${visTo}`);
  const mounted = pagesToMount(L, scroll, viewH, { nearVh: 2, maxMounted: 4 }, []);
  const visible = [];
  for (let i = visFrom; i <= visTo; i++) visible.push(i);
  for (const i of visible) assert.ok(mounted.includes(i), `visible page ${i} was not mounted`);
  assert.ok(mounted.length >= visible.length);
}

assert.equal(sameScroll(10, 10.4), true);
assert.equal(sameScroll(10, 12), false);

console.log('test-reader-continuity ok');
