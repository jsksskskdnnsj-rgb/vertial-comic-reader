// library.js - library management on the real backend commands (shelf_list, folder_*, shelf_move,
// bookmark_counts, episode_info, shelf_rename, book_reset_progress, check_books_missing,
// page_edit_info, reset_page_edits, relink_book, reveal_book).
// Views: ?view=recent (started books) | ?view=marked (bookmarked books), ?q=<search>,
// ?sort=recent|added|name, folder filter in the left panel.
// v0.6: first paint happens as soon as shelf_list/folder_list return; episode labels, bookmark
// counts, missing flags and edit flags arrive afterwards (page_edit_info is capped at 8 parallel
// calls); name sort sorts the book objects (no duplicate/dropped same-name books); a click that
// closes the menu no longer opens the book underneath; keyboard support on cards and folders;
// type badge + progress strip on covers; "details" opens the book page.
// v0.6.3: reading-status filter (All / Unread / In progress / Finished, with counts) and a grid/list view switch; the folder
// filter, status filter and view mode are remembered next to the sort (prefs libFolder / libStatus / libMode / libSort); ?status=
// overrides the saved status like ?sort= does. Filtering and status rules live in library-filters.js (pure, unit-tested).
import { invoke, t, esc, toast, bootPage, prefs, confirmDialog, promptDialog, ago } from './shell.js';
import { onPage } from './page-life.js';
import { lib } from './api.js';
import { mountShell, coverUrl, wireCoverFallback } from './shell-ui.js';
import { fetchEpisodeInfo } from './chapters.js';
import { sortBy, compareTitles } from './naming.js';
import { filterBooks, countStatuses, statusOf, readLibraryState, validFolder, progressPct } from './library-filters.js';
import { libraryCoversToWarm } from './library-cover-warm.js';
import { coverWarmBatch } from './decode-budget.js';
import * as act from './book-actions.js';
import { openMergeDialog } from './merge-ui.js';
import { openGroupEditor } from './group-ui.js';
import { LIBRARY_RENDER_BATCH } from './library-render-window.js';
import { readerOpenHref } from './reader-mode-store.js';
import { readPresets, writePresets, snapshotPreset, applyPreset } from './library-presets.js';
import { cachedInvoke } from './data-cache.js';

const params = new URLSearchParams(location.search);
const view = params.get('view') || '';
const $ = (id) => document.getElementById(id);
bootPage('library');
$('title').textContent = t(view === 'recent' ? 'reader.history' : view === 'marked' ? 'bookmarks.title' : 'lib.title');

let books = [];            // shelf_list rows (snake_case fields)
let folders = [];          // folder_list rows {id,name,created_at,count}
let episodes = new Map();  // book id -> {index,total}
let bmCounts = new Map();
let missing = new Set();
let edited = new Set();
const bust = new Map();    // book id -> cover cache-buster after relink / reset edits
const saved = readLibraryState((k, d) => prefs.get(k, d), location.search);
let filter = saved.folder;          // 'all' | 'none' | folder id (remembered)
let statusFilter = saved.status;    // 'all' | 'unread' | 'progress' | 'finished' (remembered; ?status= overrides)
let viewMode = saved.mode;          // 'grid' | 'list' (remembered)
let sortMode = saved.sort;          // 'recent' | 'added' | 'progress' | 'name' | 'title' (remembered; ?sort= overrides)
let query = (params.get('q') || '').trim();
let loadSeq = 0;
let page = 0;
let extraSeq = 0;
let picking = false;
const PAGE_SIZE = LIBRARY_RENDER_BATCH;
const selected = new Set();
let tags = [];
let bookTags = new Map();
let tagNames = new Map();
let favoriteOnly = !!prefs.get('libFavoriteOnly', false);
let filterTagId = prefs.get('libTagFilter', null);
function tagLabelMap() {
  const m = new Map(tags.map((tg) => [tg.id, tg.name]));
  tagNames = new Map();
  for (const b of books) {
    const ids = bookTags.get(b.id) || [];
    tagNames.set(b.id, ids.map((id) => m.get(id)).filter(Boolean));
  }
}
mountShell(view === 'recent' ? 'recent' : view === 'marked' ? 'marked' : 'library', {
  searchValue: query,
  onSearch: (v) => {
    query = v;
    page = 0;
    const u = new URL(location.href);
    if (v) u.searchParams.set('q', v); else u.searchParams.delete('q');
    if (u.href !== location.href) history.replaceState(null, '', u);
    renderGrid();
  },
});
wireCoverFallback();

const KIND_KEY = { zip: 'type.zip', folder: 'type.folder', images: 'type.images' };
const coverOf = (b) => coverUrl(b.id, bust.get(b.id) ?? 0);

function visibleBooks() {
  const list = filterBooks(books, {
    view, folder: filter, status: statusFilter, query, bmCounts,
    favoriteOnly, tagId: filterTagId, bookTags, tagNames,
  });
  if (sortMode === 'added') return list.sort((a, b) => (b.created_at || 0) - (a.created_at || 0));
  if (sortMode === 'progress') {
    return list.sort((a, b) => {
      const d = progressPct(b) - progressPct(a);
      return d || (b.updated_at || 0) - (a.updated_at || 0);
    });
  }
  if (sortMode === 'name') return sortBy(list, (b) => b.name);
  if (sortMode === 'title') return list.sort((a, b) => compareTitles(a.name, b.name));   // alphabetical, zh-CN collation
  return list.sort((a, b) => (b.updated_at || 0) - (a.updated_at || 0));
}

const warmCovers = new Set();
function prefetchCoversAhead(list, mounted) {
  const ids = libraryCoversToWarm(list, mounted, coverWarmBatch(books.length));
  const base = typeof coverUrl === 'function' ? coverUrl : () => '';
  for (const id of ids) {
    if (warmCovers.has(id)) continue;
    warmCovers.add(id);
    const img = new Image();
    img.decoding = 'async';
    img.src = base(id, 0);
  }
}

function renderFolders() {
  const unfiled = books.filter((b) => b.folder_id == null).length;
  const cur = (key) => String(filter) === String(key);
  const icon = '<svg class="i"><use href="icons.svg#folder"/></svg>';
  const fixed = (key, label, count) => `<div class="folder-row" data-f="${key}" role="button" tabindex="0" aria-current="${cur(key)}">${icon}<span class="fname">${esc(label)}</span><span class="cnt num">${count}</span></div>`;
  $('folders').innerHTML =
    fixed('all', t('lib.allBooks'), books.length) +
    fixed('none', t('lib.noFolder'), unfiled) +
    folders.map((f) => `<div class="folder-row" data-f="${f.id}" role="button" tabindex="0" aria-current="${cur(f.id)}">${icon}<span class="fname">${esc(f.name)}</span>
        <span class="f-act">
          <button data-fup="${f.id}" title="${esc(t('lib.moveUp'))}" aria-label="${esc(t('lib.moveUp'))}"><svg class="i"><use href="icons.svg#chevron-up"/></svg></button>
          <button data-fdown="${f.id}" title="${esc(t('lib.moveDown'))}" aria-label="${esc(t('lib.moveDown'))}"><svg class="i"><use href="icons.svg#chevron-down"/></svg></button>
          <button data-frename="${f.id}" title="${esc(t('lib.rename'))}" aria-label="${esc(t('lib.rename'))}"><svg class="i"><use href="icons.svg#pencil"/></svg></button>
          <button data-fdel="${f.id}" title="${esc(t('common.delete'))}" aria-label="${esc(t('common.delete'))}"><svg class="i"><use href="icons.svg#trash"/></svg></button>
        </span><span class="cnt num">${f.count ?? books.filter((b) => b.folder_id === f.id).length}</span></div>`).join('');
}

const STATUS_KEY = { unread: 'lib.statusUnread', progress: 'lib.statusProgress', finished: 'lib.statusFinished' };
function readMeta(b) {
  if (statusOf(b) === 'unread' || !b.updated_at) return t('lib.notStarted');
  const when = ago(b.updated_at);
  return when ? t('lib.lastRead', { when }) : t('lib.notStarted');
}
function badgeText(st, pct) {
  if (st === 'finished') return t('lib.statusFinished');
  if (st === 'unread') return t('lib.statusUnread');
  return t('lib.progressPct', { n: pct });
}
function rowHtml(b, { cnt, pct, meta, kind, st, read }) {
  const on = picking && selected.has(b.id);
  return `<article class="lib-row${on ? ' is-picked' : ''}" data-id="${b.id}" tabindex="0" role="link">
    ${picking ? '<span class="pick-mark" aria-hidden="true"></span>' : ''}
    <img class="cover" alt="" loading="lazy" data-cover="${b.id}" src="${coverOf(b)}">
    <div class="lib-row__main">
      <b title="${esc(b.name)}">${esc(b.name)}</b>
      <div class="meta num">${esc(meta)}${edited.has(b.id) ? ' · ✎' : ''} · ${esc(read)}</div>
      <div class="bar"><i style="width:${pct}%"></i></div>
    </div>
    <span class="lib-row__chips">
      <span class="lib-chip lib-chip--${st}">${esc(t(STATUS_KEY[st]))} · <span class="num">${pct}%</span></span>
      ${kind ? `<span class="lib-chip">${esc(t(kind))}</span>` : ''}
      ${cnt > 0 ? `<span class="lib-chip" title="${esc(t('bookmarks.title'))}">🔖 <span class="num">${cnt}</span></span>` : ''}
      ${missing.has(b.id) ? `<span class="lib-chip lib-chip--warn">${esc(t('lib.missing'))}</span>` : ''}
    </span>
    <button class="more-btn" data-more="${b.id}" aria-label="${esc(t('lib.menu'))}"><svg class="i"><use href="icons.svg#more"/></svg></button>
  </article>`;
}
function renderStatus() {
  const counts = countStatuses(books, { view, folder: filter, query, bmCounts, favoriteOnly, tagId: filterTagId, bookTags, tagNames });
  document.querySelectorAll('#statusSeg button').forEach((b) => {
    b.setAttribute('aria-pressed', String(b.dataset.status === statusFilter));
    const c = b.querySelector('.cnt'); if (c) c.textContent = counts[b.dataset.status];
  });
  document.querySelectorAll('#modeSeg button').forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.mode === viewMode)));
}

function pageWindow(list) {
  const pages = Math.max(1, Math.ceil(list.length / PAGE_SIZE));
  if (page >= pages) page = pages - 1;
  if (page < 0) page = 0;
  return { slice: list.slice(page * PAGE_SIZE, page * PAGE_SIZE + PAGE_SIZE), pages, total: list.length };
}
let gridHtml = '';
function paintGrid(html) {
  if (html === gridHtml) return;
  gridHtml = html;
  $('grid').innerHTML = html;
}
function renderGrid() {
  $('libLoading').hidden = true;
  const list = visibleBooks();
  const win = pageWindow(list);
  renderStatus();
  $('count').textContent = t('lib.count', { n: list.length });
  $('grid').className = viewMode === 'list' ? 'lib-list' : 'cards-row lib-grid';
  paintGrid(win.slice.map((b) => cardHtml(b)).join(''));
  const empty = $('empty');
  empty.hidden = list.length > 0;
  $('emptyText').textContent = books.length ? t('lib.noBooks') : t('lib.emptyLibrary');
  $('emptyImport').hidden = books.length > 0;
  $('pager').hidden = list.length <= PAGE_SIZE;
  $('pageLabel').textContent = t('lib.page', { n: page + 1, m: win.pages });
  loadExtras(win.slice);
  prefetchCoversAhead(list, (page + 1) * PAGE_SIZE);
}
function render() { renderFolders(); renderGrid(); }

// ---- menus ---------------------------------------------------------------------
let menuEl = null;
let menuWasOpen = false;
function closeMenu() { menuEl?.remove(); menuEl = null; }
function showMenu(x, y, items) {
  closeMenu();
  menuEl = document.createElement('div');
  menuEl.className = 'menu glass-strong';
  menuEl.setAttribute('role', 'menu');
  menuEl.innerHTML = items.map((it) =>
    it === '-' ? '<div class="sep"></div>'
      : `<button role="menuitem" data-a="${it.a}" class="${it.danger ? 'danger' : ''}">${esc(it.label)}</button>`).join('');
  menuEl.style.left = Math.max(8, Math.min(x, innerWidth - 200)) + 'px';
  menuEl.style.top = Math.max(8, Math.min(y, innerHeight - items.length * 40 - 20)) + 'px';
  menuEl.addEventListener('click', (e) => {
    const b = e.target.closest('[data-a]');
    if (!b) return;
    const it = items.find((i) => i !== '-' && i.a === b.dataset.a);
    closeMenu();
    it?.fn?.();
  });
  document.body.append(menuEl);
  menuEl.querySelector('button')?.focus();
}
onPage(document, 'pointerdown', (e) => {
  menuWasOpen = !!menuEl;
  if (menuEl && !menuEl.contains(e.target) && !e.target.closest('[data-more]')) closeMenu();
}, { capture: true });
onPage(document, 'keydown', (e) => { if (e.key === 'Escape') closeMenu(); });

const reload = async (ok, id, bumpCover) => { if (ok) { if (bumpCover) bust.set(id, Date.now()); await load(true); } };

function bookMenu(btn, b) {
  const r = btn.getBoundingClientRect();
  const items = [
    { a: 'open', label: t('lib.open'), fn: () => { location.href = readerOpenHref(b.id); } },
    { a: 'openscroll', label: t('lib.openScroll'), fn: () => { location.href = readerOpenHref(b.id, 'scroll'); } },
    { a: 'openflip', label: t('lib.openFlip'), fn: () => { location.href = readerOpenHref(b.id, 'flip'); } },
    { a: 'fav', label: b.favorite ? t('lib.unfavorite') : t('lib.favorite'), fn: async () => {
      await lib.tags.setFavorite(b.id, !b.favorite).catch(() => {});
      b.favorite = !b.favorite;
      renderGrid();
    } },
    { a: 'details', label: t('lib.details'), fn: () => (location.href = `book.html?book=${b.id}`) },
    { a: 'meta', label: t('meta.matchMenu'), fn: async () => act.matchMetadata(b, { onBound: () => load(true) }) },
    { a: 'metadesk', label: t('meta.deskLink'), fn: () => { location.href = `meta.html?book=${b.id}`; } },
    { a: 'metaopen', label: t('meta.openInBrowser'), fn: () => act.openMetadataPage(b) },
    { a: 'rename', label: t('lib.rename'), fn: async () => reload(await act.renameBook(b)) },
    { a: 'move', label: t('lib.moveTo'), fn: () => moveMenu(b, r.left, r.bottom + 5) },
    '-',
    { a: 'startover', label: t('lib.startOver'), fn: async () => reload(await act.startOver(b)) },
  ];
  if (edited.has(b.id)) items.push({ a: 'resetedit', label: t('lib.resetEdits'), fn: async () => reload(await act.resetEdits(b), b.id, true) });
  if (missing.has(b.id)) items.push({ a: 'relink', label: t('reader.locate'), fn: async () => reload(await act.relinkBook(b), b.id, true) });
  if (b.kind === 'folder' || b.merged) items.push({ a: 'group', label: t('lib.editGroup'), fn: () => openGroupEditor({ bookId: b.id, books, onDone: () => load(true) }) });
  const pend = pendingByBook.get(b.id);
  if (pend) items.push({ a: 'apart', label: t('merge.separateOne'), fn: async () => {
    try {
      await invoke('merge_detach', { id: pend.id, paths: [pend.path] });
      toast(t('merge.separatedOne'));
      await refreshMergeBanner();
      await load(true);
    } catch (e) { toast(errText(e)); }
  } });
  else if (b.merged) items.push({ a: 'apart', label: t('merge.separateOne'), fn: () => openGroupEditor({ bookId: b.id, books, onDone: () => load(true) }) });
  if (b.merged) items.push({ a: 'unmerge', label: t('lib.unmerge'), fn: async () => {
    try { await invoke('unmerge_book', { bookId: b.id }); toast(t('merge.undone')); await load(true); }
    catch (e) { toast(errText(e)); }
  } });
  items.push(
    { a: 'reveal', label: t('lib.reveal'), fn: () => act.revealBook(b) },
    '-',
    { a: 'remove', label: t('lib.remove'), danger: true, fn: async () => reload(await act.removeBook(b)) },
  );
  showMenu(r.left, r.bottom + 5, items);
}
function moveMenu(b, x, y) {
  showMenu(x, y, [
    { a: 'none', label: t('lib.noFolder'), fn: async () => reload(await act.moveBook(b, null)) },
    ...folders.map((f) => ({ a: `f${f.id}`, label: f.name, fn: async () => reload(await act.moveBook(b, f.id)) })),
  ]);
}

// ---- folders ---------------------------------------------------------------------
async function askName(title, value = '') {
  const name = await promptDialog({ title, value, placeholder: t('lib.folderName'), ok: t('common.save'), cancel: t('common.cancel') });
  return typeof name === 'string' && name.trim() ? name.trim() : null;
}
const errText = (e) => String(e?.detail ?? e?.message ?? e);
async function newFolder() {
  const name = await askName(t('lib.newFolder'));
  if (!name) return;
  try { await lib.newFolder(name); toast(t('common.saved')); load(true); } catch (e) { toast(errText(e)); }
}
async function renameFolder(id) {
  const f = folders.find((x) => x.id === id); if (!f) return;
  const name = await askName(t('lib.rename'), f.name);
  if (!name || name === f.name) return;
  try { await lib.renameFolder(id, name); toast(t('common.saved')); load(true); } catch (e) { toast(errText(e)); }
}
async function deleteFolder(id) {
  const f = folders.find((x) => x.id === id);
  if (!await confirmDialog({ title: t('lib.deleteFolder'), message: t('lib.deleteFolderMsg', { name: f?.name ?? '' }), ok: t('common.delete'), danger: true })) return;
  try { await lib.deleteFolder(id); if (filter === id) { filter = 'all'; prefs.set('libFolder', filter); } toast(t('common.saved')); load(true); } catch (e) { toast(errText(e)); }
}
async function moveFolder(id, dir) {
  const ids = folders.map((f) => f.id);
  const i = ids.indexOf(id);
  const j = i + dir;
  if (i < 0 || j < 0 || j >= ids.length) return;
  const swap = ids[i];
  ids[i] = ids[j];
  ids[j] = swap;
  try { await invoke('folder_reorder', { ids }); load(true); } catch (e) { toast(errText(e)); }
}

// ---- events ----------------------------------------------------------------------
onPage(document, 'click', (e) => {
  const closedMenu = menuWasOpen; menuWasOpen = false;
  const more = e.target.closest('[data-more]');
  if (more) { e.stopPropagation(); const b = books.find((x) => x.id === +more.dataset.more); if (b) bookMenu(more, b); return; }
  const fr = e.target.closest('[data-frename]');
  if (fr) { e.stopPropagation(); renameFolder(+fr.dataset.frename); return; }
  const fu = e.target.closest('[data-fup]');
  if (fu) { e.stopPropagation(); moveFolder(+fu.dataset.fup, -1); return; }
  const fdn = e.target.closest('[data-fdown]');
  if (fdn) { e.stopPropagation(); moveFolder(+fdn.dataset.fdown, 1); return; }
  const fd = e.target.closest('[data-fdel]');
  if (fd) { e.stopPropagation(); deleteFolder(+fd.dataset.fdel); return; }
  const frow = e.target.closest('[data-f]');
  if (frow) { const v = frow.dataset.f; filter = v === 'all' || v === 'none' ? v : +v; prefs.set('libFolder', filter); page = 0; render(); return; }
  const card = e.target.closest('.book-card, .lib-row');
  if (card && !closedMenu) {
    if (picking) {
      const id = +card.dataset.id;
      if (selected.has(id)) selected.delete(id); else selected.add(id);
      updateMergeBtn();
      renderGrid();
      return;
    }
    location.href = readerOpenHref(card.dataset.id);
  }
});
onPage(document, 'keydown', (e) => {
  if (e.key !== 'Enter' && e.key !== ' ') return;
  if (e.target.closest('button, select, input')) return;
  const frow = e.target.closest('[data-f]');
  if (frow) { e.preventDefault(); frow.click(); return; }
  const card = e.target.closest('.book-card, .lib-row');
  if (card) {
    e.preventDefault();
    if (picking) { card.click(); return; }
    location.href = readerOpenHref(card.dataset.id);
  }
});
$('sortSel').value = sortMode;
$('sortSel').addEventListener('change', () => { sortMode = $('sortSel').value; prefs.set('libSort', sortMode); page = 0; renderGrid(); });
$('newFolder').addEventListener('click', newFolder);
$('statusSeg').addEventListener('click', (e) => {
  const b = e.target.closest('[data-status]'); if (!b) return;
  statusFilter = b.dataset.status; prefs.set('libStatus', statusFilter); page = 0; renderGrid();
});
$('modeSeg').addEventListener('click', (e) => {
  const b = e.target.closest('[data-mode]'); if (!b) return;
  viewMode = b.dataset.mode; prefs.set('libMode', viewMode); page = 0; renderGrid();
});
function applyLibraryFilters({ fav, tagId, presetId } = {}) {
  if (fav != null) { favoriteOnly = !!fav; prefs.set('libFavoriteOnly', favoriteOnly); }
  if (tagId !== undefined) { filterTagId = tagId; prefs.set('libTagFilter', filterTagId); }
  page = 0;
  renderGrid();
}
function libraryFilterMenu(x, y) {
  const items = [
    { a: 'fav0', label: !favoriteOnly ? `${t('lib.favAll')} ✓` : t('lib.favAll'), fn: () => applyLibraryFilters({ fav: false }) },
    { a: 'fav1', label: favoriteOnly ? `${t('lib.favOnly')} ✓` : t('lib.favOnly'), fn: () => applyLibraryFilters({ fav: true }) },
    { a: 'tag0', label: filterTagId == null ? `${t('lib.tagAll')} ✓` : t('lib.tagAll'), fn: () => applyLibraryFilters({ tagId: null }) },
    '-',
    ...tags.map((tg) => ({
      a: `tag${tg.id}`,
      label: filterTagId === tg.id ? `${tg.name} ✓` : tg.name,
      fn: () => applyLibraryFilters({ tagId: tg.id }),
    })),
    '-',
    ...readPresets().map((p) => ({
      a: `pre${p.id}`,
      label: t('lib.presetApply', { name: p.name }),
      fn: () => applyPreset(p, (st) => {
        sortMode = st.sort; statusFilter = st.status; viewMode = st.mode; filter = st.folder;
        favoriteOnly = !!st.favoriteOnly; filterTagId = st.tagId;
        prefs.set('libSort', sortMode); prefs.set('libStatus', statusFilter); prefs.set('libMode', viewMode);
        prefs.set('libFolder', filter); prefs.set('libFavoriteOnly', favoriteOnly); prefs.set('libTagFilter', filterTagId);
        $('sortSel').value = sortMode;
        page = 0;
        render();
      }),
    })),
    { a: 'presave', label: t('lib.presetSave'), fn: async () => {
      const name = await promptDialog({ title: t('lib.presetSave'), placeholder: t('lib.presetName'), ok: t('common.save') });
      if (!name) return;
      const list = readPresets();
      list.unshift(snapshotPreset(name, { sort: sortMode, status: statusFilter, mode: viewMode, folder: filter, favoriteOnly, tagId: filterTagId }));
      writePresets(list);
      toast(t('common.saved'));
    } },
  ];
  showMenu(x, y, items);
}
$('statusSeg')?.addEventListener('contextmenu', (e) => { e.preventDefault(); libraryFilterMenu(e.clientX, e.clientY); });

// ---- loading ------------------------------------------------------------------------
async function pool(items, n, fn) {
  let i = 0;
  await Promise.all(Array.from({ length: Math.min(n, items.length) }, async () => {
    while (i < items.length) await fn(items[i++]);
  }));
}
async function loadExtras(slice) {
  const my = ++extraSeq;
  const rows = slice || [];
  if (!rows.length) return;
  const ids = new Set(rows.map((b) => b.id));
  const eps = await fetchEpisodeInfo(invoke, rows.map((b) => ({ id: b.id, page: b.last_page ?? 0 }))).catch(() => []);
  if (my !== extraSeq) return;
  rows.forEach((b, i) => episodes.set(b.id, eps?.[i] || null));
  const found = new Set();
  const flags = await lib.pageEditInfos(rows.map((b) => b.id)).catch(() => null);
  if (Array.isArray(flags)) {
    for (const f of flags) if (f.edited) found.add(f.bookId);
  } else {
    await pool(rows, 8, async (b) => {
      const info = await lib.pageEditInfo(b.id).catch(() => null);
      if (info?.edited) found.add(b.id);
    });
  }
  if (my !== extraSeq) return;
  for (const id of ids) edited.delete(id);
  for (const id of found) edited.add(id);
  renderFolders();
  const list = visibleBooks();
  const win = pageWindow(list);
  paintGrid(win.slice.map((b) => cardHtml(b)).join(''));
}
function firstPath(b) {
  try {
    const p = JSON.parse(b.paths || '[]');
    return Array.isArray(p) ? (p[0] || '') : '';
  } catch { return ''; }
}
function updateMergeBtn() {
  const n = selected.size;
  const btn = $('mergeSel');
  btn.hidden = !picking || n < 2;
  btn.textContent = t('merge.selected', { n });
  $('pickMode').setAttribute('aria-pressed', String(picking));
}
async function mergeSelected() {
  const chosen = books.filter((b) => selected.has(b.id) && b.kind === 'folder');
  if (chosen.length < 2) { toast(t(selected.size >= 2 ? 'merge.foldersOnly' : 'merge.needTwo')); return; }
  const title = chosen.map((b) => b.name).sort((a, b) => a.length - b.length)[0] || '';
  openMergeDialog({
    folders,
    groups: [{
      suggested_title: title,
      confident: false,
      members: chosen.map((b) => ({
        name: b.name,
        path: firstPath(b),
        pages: b.page_count || 0,
        chapters: 0,
        order_label: '',
        shelf_id: b.id,
        cover_ver: 0,
      })),
    }],
    onDone: () => { picking = false; selected.clear(); updateMergeBtn(); load(true); },
  });
}
let pendingByBook = new Map();
function indexPending(rows) {
  pendingByBook = new Map();
  for (const g of rows || []) {
    for (const m of g.members || []) {
      const sid = m.shelf_id || m.shelfId;
      if (sid) pendingByBook.set(sid, { id: g.id, path: m.path });
    }
  }
}
async function refreshMergeBanner() {
  const rows = await invoke('list_merge_pending').catch(() => []);
  indexPending(rows);
  const btn = $('mergeBanner');
  const n = rows?.length || 0;
  btn.hidden = n === 0;
  btn.textContent = n ? t('merge.pendingBanner', { n }) : '';
  btn.onclick = () => openMergeDialog({ groups: rows, folders, onDone: () => { refreshMergeBanner(); load(true); } });
}
function cardHtml(b) {
  const ep = episodes.get(b.id);
  const cnt = bmCounts.get(b.id) || 0;
  const st = statusOf(b);
  const pct = st === 'finished' ? 100 : b.page_count ? Math.min(100, Math.round(((b.last_page || 0) / b.page_count) * 100)) : 0;
  const meta = ep
    ? `${t('common.episodeOf', { i: ep.index, n: ep.total })} · ${t('book.pages', { n: b.page_count })}`
    : t('book.pages', { n: b.page_count });
  const kind = KIND_KEY[b.kind];
  const read = readMeta(b);
  if (viewMode === 'list') return rowHtml(b, { cnt, pct, meta, kind: kind || null, st, read });
  const on = picking && selected.has(b.id);
  return `<article class="book-card${on ? ' is-picked' : ''}" data-id="${b.id}" tabindex="0" role="link">
      <div class="cover-wrap">
        ${picking ? '<span class="pick-mark" aria-hidden="true"></span>' : ''}
        <img class="cover" alt="" loading="lazy" data-cover="${b.id}" src="${coverOf(b)}">
        ${cnt > 0 ? `<i class="ribbon" title="${esc(t('bookmarks.title'))}">${cnt}</i>` : ''}
        ${kind ? `<span class="kind-badge">${esc(t(kind))}</span>` : ''}
        ${missing.has(b.id) ? `<span class="missing-badge">${esc(t('lib.missing'))}</span>` : `<span class="prog-badge prog-badge--${st}">${esc(badgeText(st, pct))}</span>`}
        <button class="more-btn" data-more="${b.id}" aria-label="${esc(t('lib.menu'))}"><svg class="i"><use href="icons.svg#more"/></svg></button>
        <div class="bar cover-bar"><i style="width:${pct}%"></i></div>
      </div>
      <b title="${esc(b.name)}">${esc(b.name)}</b>
      <div class="meta num">${esc(meta)}${edited.has(b.id) ? ' · ✎' : ''}</div>
      <div class="meta meta--read">${esc(read)}</div>
    </article>`;
}
async function load(fresh = false) {
  const my = ++loadSeq;
  page = 0;
  const [bk, fl] = await Promise.all([
    cachedInvoke('shelf', () => lib.allBooks(), fresh).catch(() => []),
    cachedInvoke('folders', () => lib.folders(), fresh).catch(() => null),
  ]);
  if (my !== loadSeq) return;
  books = bk; folders = fl ?? [];
  if (fl) { const f = validFolder(filter, folders); if (f !== filter) { filter = f; prefs.set('libFolder', f); } }   // a remembered folder that was deleted -> All (a failed folder_list must not wipe it)
  updateMergeBtn();
  refreshMergeBanner();
  render(); // first paint: this page of covers, names, progress

  const [counts, miss, tagList, tagMap] = await Promise.all([
    books.length ? invoke('bookmark_counts', { bookIds: books.map((b) => b.id) }).catch(() => []) : [],
    lib.missing().catch(() => []),
    lib.tags.list().catch(() => []),
    lib.tags.map().catch(() => []),
  ]);
  if (my !== loadSeq) return;
  bmCounts = new Map((counts || []).map((r) => [r.bookId, r.count]));
  missing = new Set(miss || []);
  tags = tagList || [];
  bookTags = new Map((tagMap || []).map((r) => [r.bookId, r.tagIds || []]));
  tagLabelMap();
  renderFolders();
  renderGrid();
}
$('pickMode').addEventListener('click', () => {
  picking = !picking;
  if (!picking) selected.clear();
  updateMergeBtn();
  renderGrid();
});
$('mergeSel').addEventListener('click', mergeSelected);
$('pagePrev').addEventListener('click', () => { if (page > 0) { page -= 1; renderGrid(); } });
$('pageNext').addEventListener('click', () => { page += 1; renderGrid(); });
load();
