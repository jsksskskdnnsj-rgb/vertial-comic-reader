#!/usr/bin/env node
import assert from 'node:assert/strict';
import { rtlPageDelta, rtlTapZone } from '../src/reader-rtl.js';

assert.equal(rtlPageDelta('ArrowRight', true), -1);
assert.equal(rtlTapZone(-1, true), 1);
console.log('test-reader-rtl: ok');
