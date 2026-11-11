#!/usr/bin/env node
// Core algorithm locks: name parsing, same-book review (3-character run),
// natural sort order, and large-library decode caps. No DOM, no network.
import assert from 'node:assert/strict';
import { compareNames, sortBy } from '../src/naming.js';
import { parseComicName } from '../src/name-parse.js';
import { reviewGroups } from '../src/same-book.js';
import { tunePerf, coverWarmBatch } from '../src/decode-budget.js';
import { summarizeFolder } from '../src/folder-scan.js';
import { scoreTitle } from '../src/title-score.js';

const one = parseComicName('海贼王 第1-50话');
assert.equal(one.base, '海贼王');
assert.equal(one.chapter, 1);
assert.equal(one.chapterEnd, 50);
assert.equal(one.volume, null);

const classic = parseComicName('《二十四孝》');
assert.equal(classic.base, '二十四孝');
assert.equal(classic.chapter, null);
assert.equal(classic.volume, null);

const vol = parseComicName('One Piece Vol. 10');
assert.equal(vol.volume, 10);
assert.equal(vol.base, 'onepiece');
assert.equal(parseComicName('第一话').chapter, 1);
assert.equal(parseComicName('Foo (2)').chapter, 2);
assert.equal(parseComicName('作品2024').chapter, null);
assert.equal(parseComicName('上海').chapter, null);

const ordered = sortBy(['10.jpg', '2.jpg', '第10话', '第2话', '第一话'], (x) => x);
assert.deepEqual(ordered, ['2.jpg', '10.jpg', '第一话', '第2话', '第10话']);
const books = [{ id: 1, name: '10' }, { id: 2, name: '2' }, { id: 3, name: '2' }];
assert.deepEqual(sortBy(books, (b) => b.name).map((b) => b.id), [2, 3, 1]);
assert.ok(compareNames('《二十四孝》', '第24话') !== 0);

const pair = reviewGroups(['海贼王外传', '海贼王特别篇']);
assert.equal(pair.length, 1);
assert.equal(pair[0].title, '海贼王');
assert.deepEqual(pair[0].members.slice().sort(), ['海贼王外传', '海贼王特别篇']);
assert.equal(reviewGroups(['东京喰种', '东京复仇者']).length, 0);
assert.equal(reviewGroups(['volcano', 'voltage']).length, 0);
assert.equal(reviewGroups(['aaachapter', 'bbbchapter']).length, 0);
assert.equal(reviewGroups(['第1话', '第2话']).length, 0);
assert.equal(reviewGroups(['AlphaBetaOne', 'AlphaBetaTwoExtra', 'AlphaBetaThreeStory']).length, 1);

const names = [];
for (let i = 0; i < 500; i++) names.push(`独立${String(i).padStart(4, '0')}甲乙`);
names.push('海贼王外传', '海贼王特别篇');
const t0 = performance.now();
const wide = reviewGroups(names);
const reviewMs = performance.now() - t0;
assert.ok(wide.some((g) => g.title === '海贼王'), 'shared title kept in a large list');
assert.ok(reviewMs < 1500, `reviewGroups ${reviewMs.toFixed(1)}ms`);

const sortNames = Array.from({ length: 3000 }, (_, i) => (i % 2 ? `第${i}话` : `${i}.jpg`));
const t1 = performance.now();
const fast = sortBy(sortNames, (x) => x);
const fastMs = performance.now() - t1;
const sample = sortNames.slice(0, 400);
assert.deepEqual(sortBy(sample, (x) => x), sample.slice().sort(compareNames));
assert.ok(fast.length === sortNames.length);
assert.ok(fastMs < 1500, `sortBy ${fastMs.toFixed(1)}ms`);

const big = tunePerf({ warm: 4, maxMounted: 24, maxAhead: 16, maxBehind: 6 }, 4000);
assert.equal(big.warm, 1);
assert.equal(big.maxMounted, 8);
assert.equal(big.maxAhead, 6);
assert.equal(big.maxBehind, 2);
const mid = tunePerf({ warm: 4, maxMounted: 24, maxAhead: 16, maxBehind: 6 }, 1500);
assert.equal(mid.warm, 2);
assert.equal(mid.maxMounted, 12);
assert.equal(mid.maxAhead, 8);
assert.equal(mid.maxBehind, 6);
const small = tunePerf({ warm: 4, maxMounted: 24, maxAhead: 16, maxBehind: 6 }, 40);
assert.equal(small.warm, 4);
assert.equal(coverWarmBatch(100, 20), 20);
assert.equal(coverWarmBatch(2000, 20), 12);
assert.equal(coverWarmBatch(5000, 20), 8);

const files = [];
for (let i = 0; i < 30; i++) files.push({ name: `d${i}/1.jpg`, size: 10 });
const summary = summarizeFolder(files);
assert.equal(summary.subfolders, 30);
assert.equal(summary.subfolderNames.length, 24);
assert.equal(summary.images, 30);

const han = scoreTitle('進擊的巨人', { title: '进击的巨人', aliases: [] });
assert.equal(han.kind, 'exact');
assert.ok(han.confidence >= 90);
const romaji = scoreTitle('Ōkami', { title: 'Okami' });
assert.equal(romaji.kind, 'exact');
const local = scoreTitle(' solo leveling 汉化 ', { title: 'Solo Leveling', aliases: ['나 혼자만 레벨업'] });
assert.ok(local.kind === 'exact' || local.kind === 'strong');
assert.ok(local.confidence >= 80);

console.log('test-core-algo: ok', { reviewMs: Math.round(reviewMs), fastMs: Math.round(fastMs) });
