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
import { invoke, t, esc, toast, bootPage, prefs, confirmDialog, promptDialog } from './shell.js';
import { lib } from './legacy-api.js';
import { mountShell, coverUrl, wireCoverFallback } from './shell-ui.js';
import { fetchEpisodeInfo } from './chapters.js';
import { sortBy } from './naming.js';
import * as act from './book-actions.js';

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
let filter = 'all';        // 'all' | 'none' | folder id
const SORTS = ['recent', 'added', 'name'];
let sortMode = SORTS.includes(params.get('sort')) ? params.get('sort') : prefs.get('libSort', 'recent');
if (!SORTS.includes(sortMode)) sortMode = 'recent';
let query = (params.get('q') || '').trim();
let loadSeq = 0;

mountShell(view === 'recent' ? 'recent' : view === 'marked' ? 'marked' : 'library', {
  searchValue: query,
  onSearch: (v) => { query = v; renderGrid(); },
});
wireCoverFallback();

const KIND_KEY = { zip: 'type.zip', folder: 'type.folder', images: 'type.images' };
const coverOf = (b) => coverUrl(b.id, bust.get(b.id) ?? 0);

function visibleBooks() {
  const q = query.toLowerCase();
  const list = books.filter((b) => {
    if (view === 'recent' && !(b.last_page > 0)) return false;
    if (view === 'marked' && !(bmCounts.get(b.id) > 0)) return false;
    if (filter === 'none' && b.folder_id != null) return false;
    if (typeof filter === 'number' && b.folder_id !== filter) return false;
    if (q && !String(b.name || '').toLowerCase().includes(q)) return false;
    return true;
  });
  if (sortMode === 'added') return list.sort((a, b) => (b.created_at || 0) - (a.created_at || 0));
  if (sortMode === 'name') return sortBy(list, (b) => b.name);
  return list.sort((a, b) => (b.updated_at || 0) - (a.updated_at || 0));
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
          <button data-frename="${f.id}" title="${esc(t('lib.rename'))}" aria-label="${esc(t('lib.rename'))}"><svg class="i"><use href="icons.svg#pencil"/></svg></button>
          <button data-fdel="${f.id}" title="${esc(t('common.delete'))}" aria-label="${esc(t('common.delete'))}"><svg class="i"><use href="icons.svg#trash"/></svg></button>
        </span><span class="cnt num">${f.count ?? books.filter((b) => b.folder_id === f.id).length}</span></div>`).join('');
}

function renderGrid() {
  const list = visibleBooks();
  $('count').textContent = t('lib.count', { n: list.length });
  $('grid').innerHTML = list.map((b) => {
    const ep = episodes.get(b.id);
    const cnt = bmCounts.get(b.id) || 0;
    const pct = b.page_count ? Math.min(100, Math.round(((b.last_page || 0) / b.page_count) * 100)) : 0;
    const meta = ep
      ? `${t('common.episodeOf', { i: ep.index, n: ep.total })} · ${t('book.pages', { n: b.page_count })}`
      : t('book.pages', { n: b.page_count });
    const kind = KIND_KEY[b.kind];
    return `<article class="book-card" data-id="${b.id}" tabindex="0" role="link">
      <div class="cover-wrap">
        <img class="cover" alt="" loading="lazy" data-cover="${b.id}" src="${coverOf(b)}">
        ${cnt > 0 ? `<i class="ribbon" title="${esc(t('bookmarks.title'))}">${cnt}</i>` : ''}
        ${kind ? `<span class="kind-badge">${esc(t(kind))}</span>` : ''}
        ${missing.has(b.id) ? `<span class="missing-badge">${esc(t('lib.missing'))}</span>` : ''}
        <button class="more-btn" data-more="${b.id}" aria-label="${esc(t('lib.menu'))}"><svg class="i"><use href="icons.svg#more"/></svg></button>
        <div class="bar cover-bar"><i style="width:${pct}%"></i></div>
      </div>
      <b title="${esc(b.name)}">${esc(b.name)}</b>
      <div class="meta num">${esc(meta)}${edited.has(b.id) ? ' · ✎' : ''}</div>
    </article>`;
  }).join('');
  const empty = $('empty');
  empty.hidden = list.length > 0;
  empty.textContent = books.length ? t('lib.noBooks') : t('lib.emptyLibrary');
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
document.addEventListener('pointerdown', (e) => {
  menuWasOpen = !!menuEl;
  if (menuEl && !menuEl.contains(e.target) && !e.target.closest('[data-more]')) closeMenu();
}, true);
document.addEventListener('keydown', (e) => { if (e.key === 'Escape') closeMenu(); });

const reload = async (ok, id, bumpCover) => { if (ok) { if (bumpCover) bust.set(id, Date.now()); await load(); } };

function bookMenu(btn, b) {
  const r = btn.getBoundingClientRect();
  const items = [
    { a: 'open', label: t('lib.open'), fn: () => (location.href = `reader.html?book=${b.id}`) },
    { a: 'details', label: t('lib.details'), fn: () => (location.href = `book.html?book=${b.id}`) },
    { a: 'rename', label: t('lib.rename'), fn: async () => reload(await act.renameBook(b)) },
    { a: 'move', label: t('lib.moveTo'), fn: () => moveMenu(b, r.left, r.bottom + 5) },
    '-',
    { a: 'startover', label: t('lib.startOver'), fn: async () => reload(await act.startOver(b)) },
  ];
  if (edited.has(b.id)) items.push({ a: 'resetedit', label: t('lib.resetEdits'), fn: async () => reload(await act.resetEdits(b), b.id, true) });
  if (missing.has(b.id)) items.push({ a: 'relink', label: t('reader.locate'), fn: async () => reload(await act.relinkBook(b), b.id, true) });
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
  try { await lib.newFolder(name); toast(t('common.saved')); load(); } catch (e) { toast(errText(e)); }
}
async function renameFolder(id) {
  const f = folders.find((x) => x.id === id); if (!f) return;
  const name = await askName(t('lib.rename'), f.name);
  if (!name || name === f.name) return;
  try { await lib.renameFolder(id, name); toast(t('common.saved')); load(); } catch (e) { toast(errText(e)); }
}
async function deleteFolder(id) {
  const f = folders.find((x) => x.id === id);
  if (!await confirmDialog({ title: t('lib.deleteFolder'), message: t('lib.deleteFolderMsg', { name: f?.name ?? '' }), ok: t('common.delete'), danger: true })) return;
  try { await lib.deleteFolder(id); if (filter === id) filter = 'all'; toast(t('common.saved')); load(); } catch (e) { toast(errText(e)); }
}

// ---- events ----------------------------------------------------------------------
document.addEventListener('click', (e) => {
  const closedMenu = menuWasOpen; menuWasOpen = false;
  const more = e.target.closest('[data-more]');
  if (more) { e.stopPropagation(); const b = books.find((x) => x.id === +more.dataset.more); if (b) bookMenu(more, b); return; }
  const fr = e.target.closest('[data-frename]');
  if (fr) { e.stopPropagation(); renameFolder(+fr.dataset.frename); return; }
  const fd = e.target.closest('[data-fdel]');
  if (fd) { e.stopPropagation(); deleteFolder(+fd.dataset.fdel); return; }
  const frow = e.target.closest('[data-f]');
  if (frow) { const v = frow.dataset.f; filter = v === 'all' || v === 'none' ? v : +v; render(); return; }
  const card = e.target.closest('.book-card');
  if (card && !closedMenu) location.href = `reader.html?book=${card.dataset.id}`;
});
document.addEventListener('keydown', (e) => {
  if (e.key !== 'Enter' && e.key !== ' ') return;
  if (e.target.closest('button, select, input')) return;
  const frow = e.target.closest('[data-f]');
  if (frow) { e.preventDefault(); frow.click(); return; }
  const card = e.target.closest('.book-card');
  if (card) { e.preventDefault(); location.href = `reader.html?book=${card.dataset.id}`; }
});
$('sortSel').value = sortMode;
$('sortSel').addEventListener('change', () => { sortMode = $('sortSel').value; prefs.set('libSort', sortMode); renderGrid(); });
$('newFolder').addEventListener('click', newFolder);

// ---- loading ------------------------------------------------------------------------
async function pool(items, n, fn) {
  let i = 0;
  await Promise.all(Array.from({ length: Math.min(n, items.length) }, async () => {
    while (i < items.length) await fn(items[i++]);
  }));
}
async function load() {
  const my = ++loadSeq;
  const [bk, fl] = await Promise.all([lib.allBooks().catch(() => []), lib.folders().catch(() => [])]);
  if (my !== loadSeq) return;
  books = bk; folders = fl;
  if (filter !== 'all' && filter !== 'none' && !folders.some((f) => f.id === filter)) filter = 'all';
  render(); // first paint: covers, names, progress

  const [eps, counts, miss] = await Promise.all([
    fetchEpisodeInfo(invoke, books.map((b) => ({ id: b.id, page: b.last_page ?? 0 }))).catch(() => []),
    books.length ? invoke('bookmark_counts', { bookIds: books.map((b) => b.id) }).catch(() => []) : [],
    lib.missing().catch(() => []),
  ]);
  if (my !== loadSeq) return;
  episodes = new Map(books.map((b, i) => [b.id, eps?.[i] || null]));
  bmCounts = new Map((counts || []).map((r) => [r.bookId, r.count]));
  missing = new Set(miss || []);
  render();

  const next = new Set();
  await pool(books, 8, async (b) => {
    const info = await lib.pageEditInfo(b.id).catch(() => null);
    if (info?.edited) next.add(b.id);
  });
  if (my !== loadSeq) return;
  edited = next;
  renderGrid();
}
load();
