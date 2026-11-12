import assert from 'node:assert/strict';
import { compareRows, filterShelf, createQueue, enqueue, nextPending, markRunning, cancelItem, cancelPending, retryItem, finishItem } from '../src/meta-desk.js';

const rows = [
  { bookId: 1, name: 'Alpha', kind: 'folder', provider: 'bangumi', locked: false, titleDisplay: 'Alpha' },
  { bookId: 2, name: 'Beta', kind: 'zip', provider: null, locked: false },
  { bookId: 3, name: 'Gamma', kind: 'folder', provider: 'manual', locked: true, externalId: 'm-3' },
];

assert.deepEqual(filterShelf(rows, { query: 'alp' }).map((r) => r.bookId), [1]);
assert.deepEqual(filterShelf(rows, { status: 'unbound' }).map((r) => r.bookId), [2]);
assert.deepEqual(filterShelf(rows, { status: 'locked' }).map((r) => r.bookId), [3]);
assert.deepEqual(filterShelf(rows, { status: 'bound', kind: 'folder' }).map((r) => r.bookId), [1, 3]);
assert.deepEqual(filterShelf(rows, { status: 'pending', pendingIds: new Set([2]) }).map((r) => r.bookId), [2]);
assert.deepEqual(filterShelf(rows, { query: 'm-3' }).map((r) => r.bookId), [3]);

const cmp = compareRows(
  { titleDisplay: '旧', authors: ['甲'], chapterCount: 3, summary: '一样' },
  { title: '新', authors: ['甲', '乙'], chapterCount: 3, summary: '一样' },
);
const byKey = Object.fromEntries(cmp.map((row) => [row.key, row]));
assert.equal(byKey.title.current, '旧');
assert.equal(byKey.title.next, '新');
assert.equal(byKey.title.same, false);
assert.equal(byKey.authors.next, '甲、乙');
assert.equal(byKey.chapters.same, true);
assert.equal(byKey.summary.same, true);

const q = createQueue();
const added = enqueue(q, [
  { bookId: 1, title: 'Alpha', action: 'search' },
  { bookId: 2, title: 'Beta', action: 'refresh' },
  { bookId: null, title: 'skip' },
]);
assert.equal(added.length, 2);
assert.equal(nextPending(q).bookId, 1);
markRunning(q, added[0].id);
assert.equal(nextPending(q).bookId, 2);
cancelItem(q, added[0].id);
finishItem(q, added[0].id, true, 'late');
assert.equal(q.items[0].status, 'cancelled');
cancelPending(q);
assert.equal(q.items[1].status, 'cancelled');
assert.equal(nextPending(q), null);
retryItem(q, added[1].id);
assert.equal(nextPending(q).bookId, 2);
markRunning(q, added[1].id);
finishItem(q, added[1].id, false, 'offline');
assert.equal(q.items[1].status, 'failed');
assert.equal(q.items[1].message, 'offline');
retryItem(q, added[1].id);
assert.equal(q.items[1].status, 'pending');
assert.equal(q.items[1].message, '');

console.log('test-meta-desk ok');
