import { test } from 'node:test';
import assert from 'node:assert/strict';
import { glassVars, DEFAULT_CHROME_BLUR, DEFAULT_BG_BLUR, CHROME_BLUR_MAX, BG_BLUR_MAX } from '../src/appearance-glass.js';

test('glassVars clamps chrome and wallpaper blur', () => {
  const d = glassVars({});
  assert.equal(d.chromeBlur, DEFAULT_CHROME_BLUR);
  assert.equal(d.bgBlur, DEFAULT_BG_BLUR);
  assert.equal(d.vars['--chrome-blur'], `${DEFAULT_CHROME_BLUR}px`);
  const hi = glassVars({ chromeBlur: 99, bgBlur: -5 });
  assert.equal(hi.chromeBlur, CHROME_BLUR_MAX);
  assert.equal(hi.bgBlur, DEFAULT_BG_BLUR);
  const mid = glassVars({ bgBlur: 12 });
  assert.equal(mid.bgBlur, 12);
  assert.equal(mid.vars['--bg-blur'], '12px');
});
