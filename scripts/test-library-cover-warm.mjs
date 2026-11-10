#!/usr/bin/env node
import assert from 'node:assert/strict';
import { libraryCoversToWarm, LIBRARY_COVER_WARM_BATCH } from '../src/library-cover-warm.js';

const books = [{ id: 1 }, { id: 2 }, { id: 3 }, { id: 4 }];
assert.equal(LIBRARY_COVER_WARM_BATCH, 20);
assert.deepEqual(libraryCoversToWarm(books, 2, 2), [3, 4]);
assert.deepEqual(libraryCoversToWarm(books, 10), []);
console.log('test-library-cover-warm: ok');
