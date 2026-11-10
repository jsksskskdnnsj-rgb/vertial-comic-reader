#!/usr/bin/env node
import assert from 'node:assert/strict';
import {
  normalizeViewMode,
  resolveBookViewMode,
  persistBookViewMode,
  readerOpenHref,
} from '../src/reader-mode-store.js';

assert.equal(normalizeViewMode('page'), 'flip');
assert.equal(normalizeViewMode('PAGE'), 'flip');
assert.equal(normalizeViewMode('scroll'), 'scroll');
assert.equal(normalizeViewMode(''), 'scroll');

const mem = new Map();
const prefs = {
  get(k, d) {
    return mem.has(k) ? mem.get(k) : d;
  },
  set(k, v) {
    mem.set(k, v);
  },
};

assert.equal(resolveBookViewMode(5, 'flip', () => 'scroll', prefs), 'flip');
assert.equal(resolveBookViewMode(5, null, () => 'scroll', prefs), 'scroll');
persistBookViewMode(5, 'flip', prefs);
assert.equal(resolveBookViewMode(5, null, () => 'scroll', prefs), 'flip');
assert.equal(resolveBookViewMode(9, 'scroll', () => 'flip', prefs), 'scroll');

assert.equal(readerOpenHref(12), 'reader.html?book=12');
assert.equal(readerOpenHref(12, 'flip'), 'reader.html?book=12&view=flip');
assert.equal(readerOpenHref(0), 'reader.html');

console.log('test-reader-mode-store: ok');
