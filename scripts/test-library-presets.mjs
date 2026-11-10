#!/usr/bin/env node
import assert from 'node:assert/strict';
import { snapshotPreset } from '../src/library-presets.js';

const p = snapshotPreset('测试', { sort: 'progress', status: 'progress', mode: 'list', folder: 'all', favoriteOnly: true, tagId: 3 });
assert.equal(p.sort, 'progress');
assert.equal(p.favoriteOnly, true);
assert.equal(p.tagId, 3);
console.log('test-library-presets: ok');
