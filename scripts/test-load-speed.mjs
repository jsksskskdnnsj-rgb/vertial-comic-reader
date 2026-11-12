import assert from 'node:assert/strict';
import { beginPage, onPage, pageSignal } from '../src/page-life.js';
import { mapPool } from '../src/folder-scan.js';
import { createListenerBag } from '../src/listener-bag.js';

const target = new EventTarget();
let n = 0;
beginPage();
onPage(target, 'ping', () => { n += 1; });
target.dispatchEvent(new Event('ping'));
assert.equal(n, 1);
const first = pageSignal();
beginPage();
target.dispatchEvent(new Event('ping'));
assert.equal(n, 1, 'the previous page listener must not stay attached');
assert.ok(pageSignal());
assert.notEqual(pageSignal(), first);

const order = [];
const out = await mapPool([1, 2, 3, 4, 5], 2, async (x) => {
  await new Promise((r) => setTimeout(r, x === 1 ? 30 : 1));
  order.push(x);
  return x * 10;
});
assert.deepEqual(out, [10, 20, 30, 40, 50]);
assert.deepEqual(order.slice().sort((a, b) => a - b), [1, 2, 3, 4, 5]);

const bagTarget = new EventTarget();
let hits = 0;
const bag = createListenerBag();
const a = () => { hits += 1; };
const b = () => { hits += 1; };
bag.add(bagTarget, 'ping', a);
bag.add(bagTarget, 'ping', b);
assert.equal(bag.size, 2);
bagTarget.dispatchEvent(new Event('ping'));
assert.equal(hits, 2);
bag.clear();
assert.equal(bag.size, 0);
bagTarget.dispatchEvent(new Event('ping'));
assert.equal(hits, 2, 'cleared listeners do not stack onto the next page');

console.log('test-load-speed ok');
