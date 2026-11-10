#!/usr/bin/env node
import assert from 'node:assert/strict';
import { flipWarmPages, flipTapZone, FLIP_WARM_STEPS } from '../src/flip-adjacent.js';

assert.equal(FLIP_WARM_STEPS, 2);
assert.deepEqual(flipWarmPages(5, 20, false), [3, 4, 6, 7]);
assert.deepEqual(flipWarmPages(0, 10, false), [1, 2]);
assert.ok(!flipWarmPages(3, 10, false).includes(3));
assert.deepEqual(flipTapZone(10, 100), -1);
assert.deepEqual(flipTapZone(90, 100), 1);
assert.deepEqual(flipTapZone(50, 100), 0);
console.log('test-flip-adjacent: ok');
