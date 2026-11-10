import { test } from 'node:test';
import assert from 'node:assert/strict';
import { libraryRenderCount, LIBRARY_RENDER_BATCH, libraryScopeKey } from '../src/library-render-window.js';

test('large libraries initially mount one batch and can reveal every result', () => {
  const ids = Array.from({ length: 10000 }, (_, id) => id);
  assert.equal(libraryRenderCount(ids.length), LIBRARY_RENDER_BATCH);
  let count = libraryRenderCount(ids.length);
  while (count < ids.length) count = libraryRenderCount(ids.length, count + LIBRARY_RENDER_BATCH);
  assert.equal(count, ids.length);
});

test('restored counts shrink safely after filtering', () => {
  assert.equal(libraryRenderCount(10000, 600), 600);
  assert.equal(libraryRenderCount(23, 600), 23);
  assert.equal(libraryRenderCount(0, 120), 0);
  for (const invalid of [NaN, Infinity, -1]) {
    assert.equal(libraryRenderCount(1000, invalid), LIBRARY_RENDER_BATCH);
  }
});

test('libraryScopeKey is stable for the same filters', () => {
  const a = libraryScopeKey({ view: '', filter: 'all', status: 'all', sort: 'recent', query: '', mode: 'grid' });
  const b = libraryScopeKey({ view: '', filter: 'all', status: 'all', sort: 'recent', query: '', mode: 'grid' });
  assert.equal(a, b);
  assert.notEqual(a, libraryScopeKey({ view: '', filter: 'all', status: 'all', sort: 'recent', query: 'naruto', mode: 'grid' }));
});
