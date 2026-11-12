// Pure helpers for the 联网资料 desk: local shelf filter, field comparison, lookup queue.
// No network and no DOM. A job always names a book id that is already on the shelf.

export function textList(value) {
  if (Array.isArray(value)) return value.map((x) => String(x || '').trim()).filter(Boolean).join('、');
  if (value == null) return '';
  return String(value);
}

const FIELDS = [
  ['title', (row) => row.titleDisplay || row.title || ''],
  ['titleCn', (row) => row.titleCn || ''],
  ['original', (row) => row.titleOriginal || ''],
  ['authors', (row) => textList(row.authors)],
  ['tags', (row) => textList(row.tags)],
  ['year', (row) => row.year ?? ''],
  ['status', (row) => row.status || ''],
  ['chapters', (row) => row.chapterCount ?? ''],
  ['volumes', (row) => row.volumeCount ?? ''],
  ['rating', (row) => row.rating ?? ''],
  ['summary', (row) => row.summary || ''],
];

/** Side-by-side rows. `current` is a bound record (or null). `detail` is a candidate detail. */
export function compareRows(current, detail) {
  const left = current || {};
  const right = detail || {};
  return FIELDS.map(([key, pick]) => {
    const cur = String(pick(left) ?? '');
    const next = String(pick(right) ?? '');
    return { key, current: cur, next, same: cur === next };
  });
}

export function filterShelf(rows, opts = {}) {
  const q = String(opts.query || '').trim().toLowerCase();
  const status = opts.status || 'all';
  const kind = opts.kind || 'all';
  const pending = opts.pendingIds instanceof Set ? opts.pendingIds : new Set(opts.pendingIds || []);
  return (rows || []).filter((row) => {
    if (!row) return false;
    if (kind !== 'all' && row.kind !== kind) return false;
    const id = row.bookId ?? row.id;
    if (status === 'bound' && !row.provider) return false;
    if (status === 'unbound' && row.provider) return false;
    if (status === 'locked' && !row.locked) return false;
    if (status === 'pending' && !pending.has(id)) return false;
    if (!q) return true;
    const hay = [row.name, row.titleDisplay, row.provider, row.externalId].filter(Boolean).join('\n').toLowerCase();
    return hay.includes(q);
  });
}

export function createQueue() {
  return { items: [], seq: 0, stop: new Set() };
}

export function enqueue(queue, jobs) {
  const added = [];
  for (const job of jobs || []) {
    if (job?.bookId == null) continue;
    const item = {
      id: ++queue.seq,
      bookId: job.bookId,
      title: job.title || '',
      query: job.query || job.title || '',
      action: job.action || 'search',
      status: 'pending',
      message: '',
    };
    queue.items.push(item);
    added.push(item);
  }
  return added;
}

export function nextPending(queue) {
  return queue.items.find((item) => item.status === 'pending') || null;
}

export function markRunning(queue, id) {
  const item = queue.items.find((row) => row.id === id);
  if (item && item.status === 'pending') item.status = 'running';
  return item || null;
}

/** Drop a waiting job, or ask a running job to be discarded when it returns. */
export function cancelItem(queue, id) {
  const item = queue.items.find((row) => row.id === id);
  if (!item) return null;
  if (item.status === 'pending') {
    item.status = 'cancelled';
    item.message = '';
    return item;
  }
  if (item.status === 'running') queue.stop.add(id);
  return item;
}

export function cancelPending(queue) {
  let n = 0;
  for (const item of queue.items) {
    if (item.status === 'pending') {
      item.status = 'cancelled';
      n++;
    } else if (item.status === 'running') {
      queue.stop.add(item.id);
    }
  }
  return n;
}

export function retryItem(queue, id) {
  const item = queue.items.find((row) => row.id === id);
  if (!item || (item.status !== 'failed' && item.status !== 'cancelled')) return null;
  item.status = 'pending';
  item.message = '';
  queue.stop.delete(id);
  return item;
}

export function finishItem(queue, id, ok, message) {
  const item = queue.items.find((row) => row.id === id);
  if (!item) return null;
  if (queue.stop.has(id)) {
    queue.stop.delete(id);
    item.status = 'cancelled';
    item.message = '';
    return item;
  }
  item.status = ok ? 'done' : 'failed';
  item.message = message || '';
  return item;
}
