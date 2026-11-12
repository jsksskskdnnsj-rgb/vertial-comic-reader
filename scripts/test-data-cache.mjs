// Shelf and metadata lists stay in one module so a shell navigation can reuse them.
import assert from 'node:assert/strict';
import { cachedInvoke, markDirty } from '../src/data-cache.js';

let n = 0;
const load = async () => ++n;

const first = await cachedInvoke('shelf', load, false, 60_000);
const second = await cachedInvoke('shelf', load, false, 60_000);
assert.equal(first, 1);
assert.equal(second, 1);

markDirty('shelf');
const third = await cachedInvoke('shelf', load, false, 60_000);
assert.equal(third, 2);

const forced = await cachedInvoke('shelf', load, true, 60_000);
assert.equal(forced, 3);

markDirty();
const cleared = await cachedInvoke('shelf', load, false, 60_000);
assert.equal(cleared, 4);

console.log('data-cache ok');
