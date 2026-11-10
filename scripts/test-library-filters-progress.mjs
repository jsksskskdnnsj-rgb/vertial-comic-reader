#!/usr/bin/env node
import assert from 'node:assert/strict';
import { progressPct, SORTS } from '../src/library-filters.js';

assert.ok(SORTS.includes('progress'));
assert.equal(progressPct({ page_count: 1, last_page: 0 }), 0);
assert.equal(progressPct({ page_count: 101, last_page: 50 }), 50);
assert.equal(progressPct({ page_count: 11, last_page: 10 }), 100);
console.log('test-library-filters-progress: ok');
