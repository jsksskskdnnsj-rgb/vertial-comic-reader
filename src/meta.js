// meta.js — 联网资料 desk. Shelf books only: filter, compare, queue. The per-book dialog stays.
import { t, esc, toast, bootPage } from './shell.js';
import { onPage, pageSignal } from './page-life.js';
import { lib } from './api.js';
import { mountShell } from './shell-ui.js';
import { compareRows, filterShelf, createQueue, enqueue, nextPending, markRunning, cancelItem, cancelPending, retryItem, finishItem } from './meta-desk.js';
import { openMetadataEditor } from './metadata-match.js';
import { mountNetworkEditor, readMountedNetwork, refreshNetwork } from './meta-net.js';
import { remoteImgSrc } from './remote-img.js';

bootPage('meta');

const $ = (id) => document.getElementById(id);
const errText = (e) => String(e?.detail ?? e?.message ?? e);
const FIELD_KEY = {
  title: 'meta.fieldTitle',
  titleCn: 'meta.fieldTitleCn',
  original: 'meta.fieldOriginal',
  authors: 'meta.fieldAuthors',
  tags: 'meta.fieldTags',
  year: 'meta.fieldYear',
  status: 'meta.fieldStatus',
  chapters: 'meta.fieldChapters',
  volumes: 'meta.fieldVolumes',
  rating: 'meta.fieldRating',
  summary: 'meta.fieldSummary',
};
const SOURCES = [
  ['bangumi', 'Bangumi'],
  ['moegirl', '萌娘百科'],
  ['mangadex', 'MangaDex'],
  ['jikan', 'MAL'],
  ['mangaupdates', 'MangaUpdates'],
  ['anilist', 'AniList'],
  ['comic18', '18comic'],
];

let rows = [];
let pendingIds = new Set();
let query = '';
let focusId = 0;
let picked = null;
let hits = new Map();
let bound = null;
let detail = null;
const selected = new Set();
const queue = createQueue();
let pumping = false;
let cfg = null;

const params = new URLSearchParams(location.search);
const wanted = Number(params.get('book')) || 0;

mountShell('meta', {
  searchValue: '',
  onSearch: (v) => {
    query = v;
    renderBooks();
  },
});

function providerLabel(p) {
  const key = `meta.provider.${p}`;
  const s = t(key);
  return s === key ? p : s;
}
function statusLabel(status) {
  const key = `meta.status.${status}`;
  const s = t(key);
  return s === key ? status : s;
}
function cacheLabel(kind) {
  const key = {
    fresh: 'meta.cacheFresh',
    stale: 'meta.cacheStale',
    miss: 'meta.cacheMiss',
    off: 'meta.cacheOff',
    na: 'meta.cacheNa',
  }[kind] || 'meta.cacheMiss';
  return t(key);
}
function visible() {
  return filterShelf(rows, {
    query,
    status: $('metaStatus').value,
    kind: $('metaKind').value,
    pendingIds,
  });
}
function focused() {
  return rows.find((row) => row.bookId === focusId) || null;
}

function renderSources() {
  const host = $('metaSources');
  const sources = cfg?.sources || {};
  host.innerHTML = SOURCES.map(([key, label]) => {
    const on = key === 'comic18' ? sources.comic18 === true : sources[key] !== false;
    return `<label class="check"><input type="checkbox" data-src="${key}" ${on ? 'checked' : ''}> ${esc(label)}</label>`;
  }).join('');
}

function renderBooks() {
  const list = visible();
  $('metaSelected').textContent = t('meta.selected', { n: [...selected].filter((id) => list.some((row) => row.bookId === id)).length });
  $('metaBooks').innerHTML = list.map((row) => {
    const mark = row.provider ? providerLabel(row.provider) : t('meta.notBound');
    const on = row.bookId === focusId ? ' is-on' : '';
    return `<div class="meta-book${on}" data-book="${row.bookId}">
      <input type="checkbox" data-check="${row.bookId}" ${selected.has(row.bookId) ? 'checked' : ''}>
      <span class="grow"><b>${esc(row.name)}</b><span class="muted">${esc(mark)}${row.locked ? ' · ' + esc(t('meta.locked')) : ''}</span></span>
    </div>`;
  }).join('');
}

function renderQueue() {
  const host = $('metaQueue');
  host.innerHTML = queue.items.map((item) => `<div class="meta-job">
      <span class="grow"><b>${esc(item.title)}</b>
        <span class="muted">${esc(statusLabel(item.status))}${item.message ? ' · ' + esc(item.message) : ''}</span>
      </span>
      ${item.status === 'pending' || item.status === 'running' ? `<button class="btn btn--ghost btn--sm" type="button" data-cancel="${item.id}">${esc(t('common.cancel'))}</button>` : ''}
      ${item.status === 'failed' || item.status === 'cancelled' ? `<button class="btn btn--ghost btn--sm" type="button" data-retry="${item.id}">${esc(t('common.retry'))}</button>` : ''}
    </div>`).join('');
  $('metaQueueEmpty').hidden = queue.items.length > 0;
}

function renderCompare() {
  const host = $('metaTable');
  if (!detail) {
    host.innerHTML = '';
    $('metaApply').hidden = true;
    $('metaCompareHint').hidden = false;
    return;
  }
  $('metaCompareHint').hidden = true;
  const body = compareRows(bound, detail).map((row) => `<tr>
      <th>${esc(t(FIELD_KEY[row.key] || row.key))}</th>
      <td>${esc(row.current)}</td>
      <td class="${row.same ? '' : 'changed'}">${esc(row.next)}</td>
    </tr>`).join('');
  host.innerHTML = `<table class="meta-compare"><thead><tr><th></th><th>${esc(t('meta.current'))}</th><th>${esc(t('meta.candidate'))}</th></tr></thead><tbody>${body}</tbody></table>`;
  $('metaApply').hidden = !picked;
}

function renderCands() {
  const groups = hits.get(focusId) || [];
  const host = $('metaCands');
  host.replaceChildren();
  for (const group of groups) {
    const block = document.createElement('section');
    const head = document.createElement('h3');
    head.textContent = group.label || providerLabel(group.provider);
    block.append(head);
    if (group.error) {
      const err = document.createElement('p');
      err.className = 'hint warn';
      err.textContent = group.error;
      block.append(err);
    }
    for (const cand of group.candidates || []) {
      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'meta-cand btn btn--ghost';
      const on = picked && picked.provider === group.provider && picked.externalId === cand.externalId;
      if (on) btn.classList.add('is-on');
      btn.dataset.provider = group.provider;
      btn.dataset.external = cand.externalId;
      btn.dataset.kind = cand.matchKind || 'manual';
      const conf = cand.confidence != null ? t('meta.confidence', { n: cand.confidence }) : '';
      const cover = remoteImgSrc(cand.coverUrl);
      btn.innerHTML = `${cover ? `<img class="meta-cover" alt="" src="${esc(cover)}">` : ''}<span class="grow"><b>${esc(cand.title || '')}</b></span><span class="meta-score">${esc(conf)}</span>`;
      block.append(btn);
    }
    host.append(block);
  }
}

async function renderCache() {
  const host = $('metaCache');
  if (!focusId) { host.innerHTML = ''; return; }
  const q = $('metaQuery')?.value?.trim() || focused()?.name || '';
  const rowsNow = await lib.metadata.cacheStatus(focusId, q).catch(() => []);
  host.innerHTML = (rowsNow || []).map((row) =>
    `<span class="muted">${esc(providerLabel(row.provider))} · ${esc(cacheLabel(row.search))} / ${esc(cacheLabel(row.detail))}</span>`
  ).join('');
}

async function showBook(id, { search = false } = {}) {
  focusId = id;
  picked = null;
  detail = null;
  const row = focused();
  $('metaEmpty').hidden = !!row;
  $('metaWork').hidden = !row;
  if (!row) {
    $('metaBookTitle').textContent = t('meta.noBook');
    renderBooks();
    renderCands();
    renderCompare();
    return;
  }
  $('metaBookTitle').textContent = row.name;
  $('metaQuery').value = row.name || '';
  $('metaOpenBook').href = `book.html?book=${row.bookId}`;
  bound = await lib.metadata.get(row.bookId).catch(() => null);
  renderBooks();
  renderCands();
  renderCompare();
  renderCache();
  if (search) enqueueAndPump([{ bookId: row.bookId, title: row.name, query: row.name, action: 'search' }]);
}

function chosenIds() {
  const listed = new Set(visible().map((row) => row.bookId));
  const ids = [...selected].filter((id) => listed.has(id));
  return ids.length ? ids : (focusId ? [focusId] : []);
}

function enqueueAndPump(jobs) {
  enqueue(queue, jobs);
  renderQueue();
  pump();
}

async function pump() {
  if (pumping) return;
  pumping = true;
  const signal = pageSignal();
  try {
    while (!signal?.aborted) {
      const job = nextPending(queue);
      if (!job) break;
      markRunning(queue, job.id);
      renderQueue();
      let ok = false;
      let message = '';
      try {
        if (job.action === 'search') {
          const groups = await lib.metadata.search(job.bookId, job.query || job.title, null, 12);
          hits.set(job.bookId, groups || []);
          const n = (groups || []).reduce((sum, group) => sum + (group.candidates?.length || 0), 0);
          ok = true;
          message = t('meta.queueHits', { n });
          if (job.bookId === focusId) {
            renderCands();
            renderCache();
          }
        } else if (job.action === 'automatch') {
          const result = await lib.metadata.automatch([job.bookId], false);
          const row = result?.[0];
          ok = row?.status !== 'error';
          message = row?.message || row?.status || '';
        } else if (job.action === 'refresh') {
          const result = await lib.metadata.refresh(job.bookId);
          ok = true;
          message = result?.message || (result?.refreshed ? t('meta.refreshDone') : t('meta.refreshLocked'));
        }
      } catch (e) {
        ok = false;
        message = errText(e);
      }
      finishItem(queue, job.id, ok, message);
      renderQueue();
      if (job.bookId === focusId) {
        bound = await lib.metadata.get(job.bookId).catch(() => bound);
        renderCompare();
      }
    }
  } finally {
    pumping = false;
    if (!signal?.aborted && nextPending(queue)) pump();
  }
}

$('metaStatus').addEventListener('change', renderBooks);
$('metaKind').addEventListener('change', renderBooks);
$('metaSelectAll').addEventListener('click', () => {
  for (const row of visible()) selected.add(row.bookId);
  renderBooks();
});
$('metaSelectNone').addEventListener('click', () => {
  selected.clear();
  renderBooks();
});
$('metaBooks').addEventListener('change', (e) => {
  const box = e.target.closest('[data-check]');
  if (!box) return;
  const id = Number(box.dataset.check);
  if (box.checked) selected.add(id); else selected.delete(id);
  renderBooks();
});
$('metaBooks').addEventListener('click', (e) => {
  if (e.target.closest('[data-check]')) return;
  const card = e.target.closest('[data-book]');
  if (!card) return;
  showBook(Number(card.dataset.book));
});
$('metaBatchSearch').addEventListener('click', () => {
  const jobs = chosenIds().map((id) => {
    const row = rows.find((item) => item.bookId === id);
    return row ? { bookId: id, title: row.name, query: row.name, action: 'search' } : null;
  }).filter(Boolean);
  if (!jobs.length) { toast(t('meta.noBook')); return; }
  enqueueAndPump(jobs);
});
$('metaBatchAuto').addEventListener('click', () => {
  const jobs = chosenIds().map((id) => {
    const row = rows.find((item) => item.bookId === id);
    return row ? { bookId: id, title: row.name, query: row.name, action: 'automatch' } : null;
  }).filter(Boolean);
  if (!jobs.length) { toast(t('meta.noBook')); return; }
  enqueueAndPump(jobs);
});
$('metaBatchRefresh').addEventListener('click', () => {
  const jobs = chosenIds().map((id) => {
    const row = rows.find((item) => item.bookId === id);
    return row?.provider ? { bookId: id, title: row.name, query: row.name, action: 'refresh' } : null;
  }).filter(Boolean);
  if (!jobs.length) { toast(t('meta.filterBound')); return; }
  enqueueAndPump(jobs);
});
$('metaSearchOne').addEventListener('click', () => {
  const row = focused();
  if (!row) return;
  const q = $('metaQuery').value.trim();
  if (!q) return;
  enqueueAndPump([{ bookId: row.bookId, title: row.name, query: q, action: 'search' }]);
});
$('metaRefreshOne').addEventListener('click', () => {
  const row = focused();
  if (!row) return;
  enqueueAndPump([{ bookId: row.bookId, title: row.name, query: row.name, action: 'refresh' }]);
});
$('metaEditOne').addEventListener('click', () => {
  const row = focused();
  if (!row) return;
  openMetadataEditor({ id: row.bookId, name: row.name }, {
    onBound: async () => {
      await loadShelf();
      showBook(row.bookId);
    },
  });
});
$('metaCands').addEventListener('click', async (e) => {
  const btn = e.target.closest('[data-provider]');
  if (!btn || !focusId) return;
  picked = { provider: btn.dataset.provider, externalId: btn.dataset.external, matchKind: btn.dataset.kind || 'manual' };
  try {
    detail = await lib.metadata.detail(focusId, picked.provider, picked.externalId);
  } catch (err) {
    detail = null;
    toast(errText(err));
  }
  renderCands();
  renderCompare();
});
$('metaApply').addEventListener('click', async () => {
  const row = focused();
  if (!row || !picked) return;
  try {
    await lib.metadata.bind(row.bookId, picked.provider, picked.externalId, picked.matchKind || 'manual', {
      importTags: $('metaImportTags').checked,
      useRemoteCover: $('metaUseCover').checked,
    });
    toast(t('meta.bound'));
    await loadShelf();
    showBook(row.bookId);
  } catch (err) { toast(errText(err)); }
});
$('metaSources').addEventListener('change', async (e) => {
  const box = e.target.closest('[data-src]');
  if (!box || !cfg) return;
  cfg = { ...cfg, ...readMountedNetwork() };
  cfg.sources = cfg.sources || {};
  cfg.sources[box.dataset.src] = box.checked;
  try {
    await lib.metadata.setConfig(cfg);
    toast(t('common.saved'));
    renderCache();
  } catch (err) {
    toast(errText(err));
    renderSources();
  }
});
$('metaQueue').addEventListener('click', (e) => {
  const cancel = e.target.closest('[data-cancel]');
  if (cancel) { cancelItem(queue, Number(cancel.dataset.cancel)); renderQueue(); return; }
  const retry = e.target.closest('[data-retry]');
  if (retry) { retryItem(queue, Number(retry.dataset.retry)); renderQueue(); pump(); }
});
$('metaCancelWait').addEventListener('click', () => { cancelPending(queue); renderQueue(); });
onPage(window, 'keydown', (e) => {
  if (e.key !== 'Escape') return;
  if (document.documentElement.dataset.modal === '1') return;
  const tag = e.target?.tagName;
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;
  cancelPending(queue);
  renderQueue();
});

async function loadShelf() {
  const [index, pending] = await Promise.all([
    lib.metadata.shelfIndex().catch(() => []),
    lib.metadata.listPending().catch(() => []),
  ]);
  rows = index || [];
  pendingIds = new Set((pending || []).map((row) => row.bookId));
  cfg = await lib.metadata.getConfig().catch(() => ({}));
  const netHost = $('metaNetExtra');
  if (netHost && !netHost.querySelector('#appProxy')) {
    mountNetworkEditor(netHost, { cfg, getBookId: () => focusId, onSaved: (next) => { cfg = next; } });
  } else {
    refreshNetwork(cfg);
  }
  renderSources();
  renderBooks();
}

loadShelf().then(() => {
  renderQueue();
  if (wanted && rows.some((row) => row.bookId === wanted)) showBook(wanted);
});
