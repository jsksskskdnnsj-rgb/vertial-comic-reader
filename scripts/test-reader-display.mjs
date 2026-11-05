// v0.8.0: dual spread layout, eye-comfort filters, margin crop helpers.
import assert from 'node:assert/strict';
import { Layout } from '../src/layout.js';
import { readerEffectVars, normalizeEffects, filterParts } from '../src/reader-effects.js';
import { detectWhiteInsets, mergeCropInsets } from '../src/reader-crop.js';

{
  const L = new Layout();
  const dims = [[800, 1200], [800, 1200], [800, 1200], [800, 1200]];
  L.setBook(dims, [0, 1, 2, 3]);
  L.compute({ viewW: 1000, widthPct: 100, maxW: Infinity, gap: 0, spread: false });
  assert.equal(L.vLeft[0], L.left);
  assert.equal(L.vW[0], L.colW);
  const y1 = L.posToY({ page: 1, frac: 0 });
  assert.ok(y1 > L.posToY({ page: 0, frac: 0 }));

  L.compute({ viewW: 1000, widthPct: 100, maxW: Infinity, gap: 0, spread: true, spreadGap: 8 });
  assert.ok(L.spread);
  assert.ok(L.vLeft[L.gFirst[0]] < L.vLeft[L.gFirst[1]]);
  assert.equal(L.vTop[L.gFirst[0]], L.vTop[L.gFirst[1]]);
  assert.equal(L.spreadStart(3), 2);
  assert.equal(L.spreadStart(4), 4);
}

{
  const w = 40, h = 40;
  const data = new Uint8ClampedArray(w * h * 4);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const i = (y * w + x) * 4;
      const border = x < 5 || y < 5 || x >= w - 5 || y >= h - 5;
      data[i] = border ? 255 : 40;
      data[i + 1] = border ? 255 : 40;
      data[i + 2] = border ? 255 : 40;
      data[i + 3] = 255;
    }
  }
  const ins = detectWhiteInsets(data, w, h, { sampleStep: 1, threshold: 240 });
  assert.ok(ins.top > 0.08, ins.top);
  assert.ok(ins.left > 0.08, ins.left);
  const merged = mergeCropInsets(ins, 4);
  assert.ok(merged.top > ins.top);
}

{
  const fx = readerEffectVars({ dim: 80, contrast: 110, filter: 'warm', filterStrength: 100 });
  assert.match(fx.vars['--reader-filter'], /brightness\(0\.800\)/);
  assert.match(fx.vars['--reader-filter'], /contrast\(1\.100\)/);
  assert.ok(filterParts(normalizeEffects({ filter: 'none', filterStrength: 100 })).length === 0);
  assert.ok(filterParts(normalizeEffects({ filter: 'night', filterStrength: 50 })).length >= 2);
}

console.log('reader display v0.8.0 checks passed');
