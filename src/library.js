// library.js - full library management on REAL backend commands:
//   shelf_list / folder_* / shelf_move (main.rs), bookmark_counts,
//   episode_info (chapters.rs), and library.rs: shelf_rename,
//   book_reset_progress, check_books_missing, page_edit_info,
//   reset_page_edits, relink_book / reveal_book (adapters.rs).
// Views: ?view=recent (started books) | ?view=marked (bookmarked books),
// ?q=<search>, ?sort=recent|added|name, folder filter in the left panel.
import { T, invoke, t, esc, toast, bootPage, openModal, confirmDialog, promptDialog } from './shell.js';
import { lib } from './legacy-api.js';
import { mountShell, coverUrl, wireCoverFallback } from './shell-ui.js';
import { fetchEpisodeInfo } from './chapters.js';
import { sortSources } from './naming.js';

const params = new URLSearchParams(location.search);
const view = params.get('view') || '';
bootPage('library');
mountShell(view === 'recent' ? 'recent' : view === 'marked' ? 'marked' : 'library', {
  searchValue: params.get('q') || '',
  onSearch: (v) => { query = v; renderGrid(); },
});
wireCoverFallback();

const $ = (id) => document.getElementById(id);
let books = [];           // shelf_list rows {id,name,kind,paths,page_count,total_size,last_page,created_at,updated_at,folder_id}
let folders = [];         // folder_list rows {id,name,count}
let episodes = [];        // aligned with books
let bmCounts = new Map();
let missing = new Set();
let edited = new Set();
let filter = 'all';       // 'all' | 'none' | folder id
let sortMode = ['added', 'name'].includes(params.get('sort')) ? params.get('sort') : 'recent';
let query = (params.get('q') || '').trim();

const coverOf = (b) => coverUrl(b.id, b.cover_ver ?? 0);
const fmtSize = (n) => {
  if (!n) return '—';
  const u = ['B', 'KB', 'MB', 'GB']; let i = 0; let v = n;
  while (v >= 1024 && i < 3) { v /= 1024; i++; }
  return `${v.toFixed(v >= 100 ? 0 : 1)} ${u[i]}`;
};

function visibleBooks() {
  let list = books.filter((b) => {
    if (view === 'recent' && !(b.last_page > 0)) return false;
    if (view === 'marked' && !(bmCounts.get(b.id) > 0)) return false;
    if (filter === 'none' && b.folder_id != null) return false;
    if (typeof filter === 'number' && b.folder_id !== filter) return false;
    if (query && !String(b.name || '').toLowerCase().includes(query.toLowerCase())) return false;
    return true;
  });
  if (sortMode === 'added') list = [...list].sort((a, b) => (b.created_at || 0) - (a.created_at || 0));
  else if (sortMode === 'name') list = sortSources(list.map((b) => b.name)).map((n) => list.find((b) => b.name === n));
  else list = [...list].sort((a, b) => (b.updated_at || 0) - (a.updated_at || 0));
  return list;
}

function renderFolders() {
  const unfiled = books.filter((b) => b.folder_id == null).length;
  const row = (key, label, count) => `<button class="folder-row" data-f="${key}" aria-current="${String(filter === key || String(filter) === String(key))}">
      <svg class="i" style="width:15px;height:15px"><use href="icons.svg#folder"/></svg><span>${esc(label)}</span><span class="cnt num">${count}</span></button>`;
  $('folders').innerHTML =
    row('all', t('lib.allBooks'), books.length) +
    row('none', t('lib.noFolder'), unfiled) +
    folders.map((f) => `<div class="folder-row" data-f="${f.id}" aria-current="${String(filter === f.id)}" role="button" tabindex="0">
        <svg class="i" style="width:15px;height:15px"><use href="icons.svg#folder"/></svg><span>${esc(f.name)}</span>
        <span class="f-act">
          <button data-frename="${f.id}" title="${esc(t('lib.rename'))}"><svg class="i"><use href="icons.svg#wand"/></svg></button>
          <button data-fdel="${f.id}" title="${esc(t('common.delete'))}"><svg class="i"><use href="icons.svg#trash"/></svg></button>
        </span><span class="cnt num">${f.count ?? books.filter((b) => b.folder_id === f.id).length}</span></div>`).join('');
}

function renderGrid() {
  const list = visibleBooks();
  const idxOf = new Map(books.map((b, i) => [b.id, i]));
  $('count').textContent = t('lib.count', { n: list.length });
  $('grid').innerHTML = list.map((b) => {
    const ep = episodes[idxOf.get(b.id)];
    const cnt = bmCounts.get(b.id) || 0;
    const pct = b.page_count ? Math.min(100, Math.round(((b.last_page || 0) / b.page_count) * 100)) : 0;
    const meta = ep
      ? `${t('common.episodeOf', { i: ep.index, n: ep.total })} · ${t('book.pages', { n: b.page_count })}`
      : t('book.pages', { n: b.page_count });
    return `<article class="book-card" data-id="${b.id}">
      <div class="cover-wrap">
        <img class="cover" alt="" loading="lazy" data-cover="${b.id}" src="${coverOf(b)}">
        ${cnt > 0 ? `<i class="ribbon" title="${esc(t('bookmarks.title'))}">${cnt}</i>` : ''}
        ${missing.has(b.id) ? `<span class="missing-badge">${esc(t('lib.missing'))}</span>` : ''}
        <button class="more-btn" data-more="${b.id}" aria-label="${esc(t('lib.menu'))}"><svg class="i"><use href="icons.svg#more"/></svg></button>
      </div>
      <b title="${esc(b.name)}">${esc(b.name)}</b>
      <div class="meta num">${esc(meta)}${edited.has(b.id) ? ' · ✎' : ''}</div>
      <div class="bar"><i style="width:${pct}%"></i></div>
    </article>`;
  }).join('');
  const empty = $('empty');
  empty.hidden = list.length > 0;
  empty.textContent = books.length ? t('lib.noBooks') : t('home.noRecent');
}
function render() { renderFolders(); renderGrid(); }

// ---- menus -------------------------------------------------------------------
let menuEl = null;
function closeMenu() { menuEl?.remove(); menuEl = null; }
function showMenu(x, y, items) {
  closeMenu();
  menuEl = document.createElement('div');
  menuEl.className = 'menu glass-strong';
  menuEl.innerHTML = items.map((it) =>
    it === '-' ? '<div class="sep"></div>'
      : `<button data-a="${it.a}" class="${it.danger ? 'danger' : ''}">${esc(it.label)}</button>`).join('');
  menuEl.style.left = Math.min(x, innerWidth - 200) + 'px';
  menuEl.style.top = Math.min(y, innerHeight - items.length * 40 - 20) + 'px';
  menuEl.addEventListener('click', (e) => {
    const b = e.target.closest('[data-a]');
    if (!b) return;
    const it = items.find((i) => i !== '-' && i.a === b.dataset.a);
    closeMenu();
    it?.fn?.();
  });
  document.body.append(menuEl);
}
document.addEventListener('pointerdown', (e) => {
  if (menuEl && !menuEl.contains(e.target) && !e.target.closest('[data-more]')) closeMenu();
});

function bookMenu(btn, b) {
  const r = btn.getBoundingClientRect();
  const items = [
    { a: 'open', label: t('lib.open'), fn: () => (location.href = `reader.html?book=${b.id}`) },
    { a: 'details', label: t('lib.details'), fn: () => showDetails(b) },
    { a: 'rename', label: t('lib.rename'), fn: () => renameBook(b) },
    { a: 'move', label: t('lib.moveTo'), fn: (ev) => moveMenu(b) },
    '-',
    { a: 'startover', label: t('lib.startOver'), fn: () => startOver(b) },
  ];
  if (edited.has(b.id)) items.push({ a: 'resetedit', label: t('dir.title', { title: '' }) + ' · ' + t('chapters.useDefault'), fn: () => resetEdits(b) });
  if (missing.has(b.id)) items.push({ a: 'relink', label: t('reader.locate'), fn: () => relink(b) });
  items.push(
    { a: 'reveal', label: t('lib.reveal'), fn: () => lib.revealBook(b.id).catch(() => {}) },
    '-',
    { a: 'remove', label: t('lib.remove'), danger: true, fn: () => removeBook(b) },
  );
  showMenu(r.left, r.bottom + 5, items);
  // move uses a second-level menu; wire it after showMenu replaced menuEl
  menuEl.querySelector('[data-a="move"]')?.addEventListener('click', (e) => {
    e.stopPropagation();
    moveMenu(b);
  });
}
function moveMenu(b) {
  const items = [
    { a: 'none', label: t('lib.noFolder'), fn: async () => { await lib.moveTo(b.id, null); toast(t('common.saved')); load(); } },
    ...folders.map((f) => ({
      a: `f${f.id}`, label: f.name,
      fn: async () => { await lib.moveTo(b.id, f.id); toast(t('common.saved')); load(); },
    })),
  ];
  showMenu(Math.min(innerWidth - 220, 240), Math.min(innerHeight - 260, 140), items);
}

async function renameBook(b) {
  const name = await promptDialog({ title: t('lib.renameTitle'), message: '', placeholder: b.name, ok: t('common.save'), cancel: t('common.cancel') });
  if (!name || !String(name).trim() || name === true) return;
  try { await lib.rename(b.id, String(name)); toast(t('common.saved')); load(); }
  catch (e) { toast(String(e?.detail ?? e?.message ?? e)); }
}
async function startOver(b) {
  if (!await confirmDialog({ title: t('lib.startOver'), message: t('lib.startOverMsg', { title: b.name }), ok: t('lib.startOver') })) return;
  try { await lib.startOver(b.id); toast(t('book.progressReset')); load(); }
  catch (e) { toast(String(e?.detail ?? e?.message ?? e)); }
}
async function resetEdits(b) {
  if (!await confirmDialog({ title: t('lib.details'), message: t('dir.reverseMsg'), ok: t('common.ok') })) return;
  try { await lib.resetPageEdits(b.id); toast(t('common.saved')); load(); }
  catch (e) { toast(String(e?.detail ?? e?.message ?? e)); }
}
async function removeBook(b) {
  if (!await confirmDialog({ title: t('lib.remove'), message: t('lib.removeMsg', { title: b.name }), ok: t('common.delete'), danger: true })) return;
  try { await lib.remove(b.id); toast(t('common.saved')); load(); }
  catch (e) { toast(String(e?.detail ?? e?.message ?? e)); }
}
async function relink(b) {
  const p = await T.dialog.open({ multiple: false }).catch(() => null);
  if (!p) return;
  try { await lib.relinkBook(b.id, p); toast(t('common.saved')); load(); }
  catch (e) { toast(String(e?.detail ?? e?.message ?? e)); }
}

function showDetails(b) {
  const paths = (() => { try { return JSON.parse(b.paths); } catch { return []; } })();
  const ep = episodes[books.findIndex((x) => x.id === b.id)];
  const body = document.createElement('div');
  body.className = 'details';
  body.innerHTML = `<img class="cover" alt="" data-cover="${b.id}" src="${coverOf(b)}">
    <dl>
      <dt>${esc(t('book.pages'))}</dt><dd class="num">${b.page_count}</dd>
      <dt>${esc(t('book.size'))}</dt><dd class="num">${fmtSize(b.total_size)}</dd>
      <dt>${esc(t('book.progress'))}</dt><dd class="num">${b.last_page > 0 ? t('common.pageOf', { p: b.last_page + 1, n: b.page_count }) : esc(t('lib.notStarted'))}${ep ? ` · ${esc(t('common.episodeOf', { i: ep.index, n: ep.total }))}` : ''}</dd>
      <dt>${esc(t('book.bookmarks'))}</dt><dd class="num">${bmCounts.get(b.id) || 0}</dd>
      <dt>${esc(t('book.path'))}</dt><dd>${esc(paths[0] || '—')}</dd>
    </dl>`;
  openModal({ title: b.name, body, wide: false });
}

// ---- events ------------------------------------------------------------------
document.addEventListener('click', (e) => {
  const more = e.target.closest('[data-more]');
  if (more) { e.stopPropagation(); const b = books.find((x) => x.id === +more.dataset.more); if (b) bookMenu(more, b); return; }
  const fr = e.target.closest('[data-frename]');
  if (fr) { e.stopPropagation(); renameFolder(+fr.dataset.frename); return; }
  const fd = e.target.closest('[data-fdel]');
  if (fd) { e.stopPropagation(); deleteFolder(+fd.dataset.fdel); return; }
  const frow = e.target.closest('[data-f]');
  if (frow && !e.target.closest('.f-act')) {
    const v = frow.dataset.f;
    filter = v === 'all' || v === 'none' ? v : +v;
    render();
    return;
  }
  const card = e.target.closest('.book-card');
  if (card && !menuEl) location.href = `reader.html?book=${card.dataset.id}`;
});
$('sortSel').value = sortMode;
$('sortSel').addEventListener('change', () => { sortMode = $('sortSel').value; renderGrid(); });
$('newFolder').addEventListener('click', async () => {
  const name = await promptDialog({ title: t('lib.newFolder'), message: '', placeholder: t('lib.folderName'), ok: t('common.create'), cancel: t('common.cancel') });
  if (!name || name === true || !String(name).trim()) return;
  try { await lib.newFolder(String(name)); toast(t('common.saved')); load(); }
  catch (e) { toast(String(e?.detail ?? e?.message ?? e)); }
});
async function renameFolder(id) {
  const f = folders.find((x) => x.id === id); if (!f) return;
  const name = await promptDialog({ title: t('lib.rename'), message: '', placeholder: f.name, ok: t('common.save'), cancel: t('common.cancel') });
  if (!name || name === true || !String(name).trim()) return;
  try { await lib.renameFolder(id, String(name)); toast(t('common.saved')); load(); }
  catch (e) { toast(String(e?.detail ?? e?.message ?? e)); }
}
async function deleteFolder(id) {
  if (!await confirmDialog({ title: t('lib.deleteFolder'), message: t('lib.deleteFolderMsg', { name: folders.find((x) => x.id === id)?.name ?? '' }), ok: t('common.delete'), danger: true })) return;
  try { await lib.deleteFolder(id); if (filter === id) filter = 'all'; toast(t('common.saved')); load(); }
  catch (e) { toast(String(e?.detail ?? e?.message ?? e)); }
}

// ---- boot ----------------------------------------------------------------------
async function load() {
  [books, folders] = await Promise.all([lib.allBooks().catch(() => []), lib.folders().catch(() => [])]);
  episodes = await fetchEpisodeInfo(invoke, books.map((b) => ({ id: b.id, page: b.last_page ?? 0 })));
  bmCounts = new Map();
  if (books.length) {
    const rows = await invoke('bookmark_counts', { bookIds: books.map((b) => b.id) }).catch(() => []);
    for (const r of rows || []) bmCounts.set(r.bookId, r.count);
  }
  missing = new Set(await lib.missing().catch(() => []));
  edited = new Set();
  await Promise.all(books.map(async (b) => {
    const info = await lib.pageEditInfo(b.id).catch(() => null);
    if (info?.edited) edited.add(b.id);
  }));
  render();
}
load();
