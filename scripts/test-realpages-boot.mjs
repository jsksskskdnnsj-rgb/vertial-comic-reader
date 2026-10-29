#!/usr/bin/env node
// Regression: open-time onPosition must not TypeError on book.realPages / thumb urls
// when `book` is still null (the bug behind "无法读取这本书" + Cannot read ... 'realPages').
import assert from 'node:assert/strict';
import { ThumbStrip } from '../src/reader-panels.js';

// Mirror the null-safe helpers from reader-boot.js (kept in sync by this comment).
let book = null;
let view = null;
const pageText = (p, n = book?.realPages ?? 0) => `page ${p + 1} of ${n}`;
const epText = (p, n) => pageText(p, n);
const pageThumb = (p) => {
  const i = book && view?.layout?.gFirst?.[p];
  return i == null ? '' : `thumb:${book.bookId}:${i}`;
};

// 1) pageText / epText during open: book still null, realPages comes from position event
assert.equal(epText(0, 42), 'page 1 of 42');
assert.equal(pageText(5), 'page 6 of 0'); // no explicit n, book null -> 0, no throw

// 2) ThumbStrip.update while book/view not ready: empty src, no throw
const host = {
  innerHTML: '',
  addEventListener() {},
};
const strip = new ThumbStrip(host, { thumb: pageThumb, onJump() {} });
assert.doesNotThrow(() => strip.update(0, 10));
assert.match(host.innerHTML, /src=""/);
strip.reset();
assert.doesNotThrow(() => strip.update(0, 0)); // realPages not ready
assert.equal(host.innerHTML, '');

// 3) after beforeMount-style assign, thumbs resolve
book = { bookId: 7, realPages: 10, sig: 'x' };
view = { layout: { gFirst: [0, 1, 2, 3, 4, 5, 6, 7, 8, 9] }, info: book };
strip.reset();
strip.update(3, 10);
assert.match(host.innerHTML, /thumb:7:3/);
assert.equal(pageText(3), 'page 4 of 10');

// 4) classic bug shape: reading book.realPages when book is null must be avoided
book = null;
assert.equal(book?.realPages, undefined);
assert.throws(() => { void book.realPages; }, TypeError); // documents the old crash

console.log('ok: realPages boot timing / null-safety');
