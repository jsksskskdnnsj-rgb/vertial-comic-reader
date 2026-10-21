// library.js - full shelf: folder filter, search (?q=), sort (?sort=recent|added),
// covers via mhcover (same pattern as home.js), episode labels, bookmark ribbons, card menu.
import { T, invoke, t, esc, toast, applyI18n, applyTheme, wireWindow } from './shell.js';
import { fetchEpisodeInfo } from './chapters.js';

const COVER_BASE = T.core.convertFileSrc('', 'mhcover').replace(/\/$/, '');   // http://mhcover.localhost on Windows
const params = new URLSearchParams(location.search);

let books = [];          // raw shelf_list rows
let folders = [];        // folder_list rows
let episodes = [];       // [{index,total}|null] aligned with `books`
let bmCounts = new Map(); // bookId -> count
let filter = 'all';      // 'all' | 'none' | folder id (number)
let sortMode = params.get('sort') === 'added' ? 'added' : 'recent';
let query = (params.get('q') || '').trim();

const $ = (id) => document.getElementById(id);
const coverVer = (b) => b.coverVer ?? b.cover_ver ?? 0;
const coverImg = (b, cls = '') => `<img class="cover ${cls}" alt="" loading="lazy" data-cover="${b.id}" src="${COVER_BASE}/${b.id}?v=${coverVer(b)}">`;

// covers: a 404 means "not generated yet" -> ask Rust once, then retry (copied from home.js)
document.addEventListener('error', async (e) => {
  const img = e.target; if (!(img instanceof HTMLImageElement) || !img.dataset.cover || img.dataset.retried) return;
  img.dataset.retried = '1';
  try { await invoke('ensure_cover', { bookId: +img.dataset.cover }); img.src = img.src.replace(/v=\d+/, 'v=' + Date.now()); } catch {}
}, true);

function visibleBooks() {
  let list = books.filter((b) => {
    if (filter === 'none' && b.folder_id != null) return false;
    if (typeof filter === 'number' && b.folder_id !== filter) return false;
    if (query && !String(b.name || '').toLowerCase().includes(query.toLowerCase())) return false;
    return true;
  });
  list = [...list].sort((a, b2) => sortMode === 'added'
    ? (b2.created_at || 0) - (a.created_at || 0)
    : (b2.updated_at || 0) - (a.updated_at || 0));
  return list;
}

function renderFolders() {
  const unfiled = books.filter((b) => b.folder_id == null).length;
  const row = (key, label, count, icon) => `<button data-f="${key}" aria-current="${String(filter === key || String(filter) === String(key))}">
    <svg class="i"><use href="icons.svg#${icon}"/></svg><span>${esc(label)}</span><span class="cnt num">${count}</span></button>`;
  $('folders').innerHTML =
    row('all', '全部', books.length, 'shelf') +
    row('none', '未分类', unfiled, 'folder') +
    folders.map((f) => row(f.id, f.name, f.count ?? books.filter((b) => b.folder_id === f.id).length, 'folder')).join('');
}

function renderGrid() {
  const list = visibleBooks();
  const idxOf = new Map(books.map((b, i) => [b.id, i]));
  $('grid').innerHTML = list.map((b) => {
    const ep = episodes[idxOf.get(b.id)];
    const cnt = bmCounts.get(b.id) || 0;
    const meta = ep
      ? `${t('episodeOf', { i: ep.index, n: ep.total })} · 共 ${b.page_count} 页`
      : `共 ${b.page_count} 页`;
    return `<article class="lib-card" data-id="${b.id}">
      <div class="lib-cover-wrap">${coverImg(b)}
        ${cnt > 0 ? `<i class="ribbon" title="书签 ${cnt}">${cnt}</i>` : ''}
        <button class="more" data-more="${b.id}" aria-label="更多操作"><svg class="i"><use href="icons.svg#more"/></svg></button>
      </div>
      <b title="${esc(b.name)}">${esc(b.name)}</b>
      <div class="meta num">${esc(meta)}</div>
    </article>`;
  }).join('');
  const empty = $('empty');
  if (!list.length) {
    empty.hidden = false;
    empty.textContent = books.length
      ? '没有找到符合条件的漫画，换个关键词或文件夹试试。'
      : '书架还是空的，去「本地导入」添加一本漫画吧。';
  } else empty.hidden = true;
}

function render() { renderFolders(); renderGrid(); }

// ---- card menu: reveal / move to folder (submenu) / remove -------------------
let menu, subMenu;
function closeMenus() { menu?.remove(); subMenu?.remove(); menu = null; subMenu = null; }

function openMoveSubmenu(anchorBtn, id) {
  subMenu?.remove();
  subMenu = Object.assign(document.createElement('div'), { className: 'menu' });
  const items = [`<button data-mv="none">未分类</button>`,
    ...folders.map((f) => `<button data-mv="${f.id}">${esc(f.name)}</button>`)];
  subMenu.innerHTML = items.join('') || '<button data-mv="none">未分类</button>';
  const r = anchorBtn.getBoundingClientRect();
  subMenu.style.top = r.top + 'px';
  subMenu.style.left = Math.min(r.right + 4, innerWidth - 180) + 'px';
  subMenu.onclick = async (e) => {
    const b = e.target.closest('[data-mv]'); if (!b) return;
    const folder_id = b.dataset.mv === 'none' ? null : +b.dataset.mv;
    closeMenus();
    try { await invoke('shelf_move', { id, folder_id }); toast('已移动'); await load(); }
    catch (err) { toast(`移动失败：${err?.detail ?? err?.message ?? err}`); }
  };
  document.body.append(subMenu);
}

function openMenu(btn, id) {
  closeMenus();
  menu = Object.assign(document.createElement('div'), { className: 'menu' });
  menu.innerHTML = `<button data-a="open">${t('open')}</button>
    <button data-a="reveal">${t('reveal')}</button>
    <button data-a="move">移动到文件夹<svg class="i" style="margin-left:auto;width:14px;height:14px"><use href="icons.svg#chevron"/></svg></button>
    <div class="sep"></div>
    <button data-a="remove" class="danger">从书架移除</button>`;
  const r = btn.getBoundingClientRect();
  menu.style.top = r.bottom + 4 + 'px';
  menu.style.left = Math.max(8, Math.min(r.left, innerWidth - 190)) + 'px';
  menu.onclick = async (e) => {
    const b = e.target.closest('button'); if (!b) return;
    const a = b.dataset.a;
    if (a === 'move') { e.stopPropagation(); return openMoveSubmenu(b, id); }
    closeMenus();
    if (a === 'open') location.href = `reader.html?book=${id}`;
    if (a === 'reveal') invoke('reveal_book', { bookId: id }).catch(() => {});
    if (a === 'remove') {
      const book = books.find((x) => x.id === id);
      if (!confirm(`确定要从书架移除《${book?.name ?? ''}》吗？源文件不会被删除。`)) return;
      try { await invoke('shelf_remove', { id }); toast('已移除'); await load(); }
      catch (err) { toast(`移除失败：${err?.detail ?? err?.message ?? err}`); }
    }
  };
  document.body.append(menu);
}

document.addEventListener('click', (e) => {
  const more = e.target.closest('[data-more]');
  if (more) { e.stopPropagation(); return openMenu(more, +more.dataset.more); }
  if (menu && !menu.contains(e.target) && !(subMenu && subMenu.contains(e.target))) closeMenus();
  const card = e.target.closest('.lib-card');
  if (card && !e.target.closest('.menu')) location.href = `reader.html?book=${card.dataset.id}`;
});
addEventListener('pointerdown', (e) => { if (menu && !menu.contains(e.target) && !(subMenu && subMenu.contains(e.target)) && !e.target.closest('[data-more]')) closeMenus(); });
addEventListener('keydown', (e) => { if (e.key === 'Escape') closeMenus(); });

$('folders').addEventListener('click', (e) => {
  const b = e.target.closest('[data-f]'); if (!b) return;
  const v = b.dataset.f;
  filter = v === 'all' || v === 'none' ? v : +v;
  render();
});

// ---- search & sort ------------------------------------------------------------
const q = $('q');
q.value = query;
q.addEventListener('input', () => { query = q.value.trim(); renderGrid(); });
q.addEventListener('keydown', (e) => { if (e.key === 'Enter') renderGrid(); });
addEventListener('keydown', (e) => { if ((e.key === '/' || (e.ctrlKey && e.key === 'f')) && document.activeElement !== q) { e.preventDefault(); q.focus(); } });

const sortSel = $('sortSel');
sortSel.value = sortMode;
sortSel.addEventListener('change', () => {
  sortMode = sortSel.value;
  const u = new URL(location.href); u.searchParams.set('sort', sortMode); history.replaceState(null, '', u);
  renderGrid();
});

// ---- boot ----------------------------------------------------------------------
async function load() {
  [books, folders] = await Promise.all([
    invoke('shelf_list').catch(() => []),
    invoke('folder_list').catch(() => []),
  ]);
  episodes = books.length
    ? await fetchEpisodeInfo(invoke, books.map((b) => ({ id: b.id, page: b.last_page ?? 0 }))).catch(() => books.map(() => null))
    : [];
  bmCounts = new Map();
  if (books.length) {
    const rows = await invoke('bookmark_counts', { bookIds: books.map((b) => b.id) }).catch(() => []);
    for (const r of rows || []) bmCounts.set(r.bookId, r.count);
  }
  render();
}

applyTheme(); applyI18n(); wireWindow(); load();
