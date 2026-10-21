// home.js - Home page behaviour. Pure DOM + invoke(); no framework.
import { T, invoke, t, esc, ago, toast, applyI18n, applyTheme, wireWindow } from './shell.js';
import { fetchEpisodeInfo } from './chapters.js';

// =============================================================================
// ADAPTER CONTRACT - the only place Home touches your backend. Map these to your
// existing commands (names below are what I assumed; rename freely).
//
//  list_recent_books({limit}) -> [{id,title,pageCount,page,frac,lastReadAt(ms),coverVer}]
//     SELECT b.id,b.title,b.page_count,p.page,p.frac,b.last_read_at
//       FROM books b JOIN progress p ON p.book_id=b.id
//      ORDER BY p.updated_at DESC LIMIT ?1;           -- `progress` table from the reader backend
//  list_recent_added({limit}) -> [{id,title,pageCount,addedAt,coverVer}]
//     SELECT id,title,page_count,added_at FROM books ORDER BY added_at DESC LIMIT ?1;
//  import_paths({paths,folderId}) -> {added,skipped,failed:[{path,reason}]}   (your existing import)
//  reveal_book({bookId})      -> opens Explorer with the file selected
//  ensure_cover({bookId})     -> generates covers/{id}.jpg if missing (see cover_protocol.rs)
// =============================================================================
const api = {
  recent:  (limit = 3) => invoke('list_recent_books', { limit }),
  added:   (limit = 4) => invoke('list_recent_added', { limit }),
  import:  (paths)     => invoke('import_paths', { paths, folderId: null }),
  reveal:  (bookId)    => invoke('reveal_book', { bookId }),
  cover:   (bookId)    => invoke('ensure_cover', { bookId }),
};
const COVER_BASE = T.core.convertFileSrc('', 'mhcover').replace(/\/$/, '');   // http://mhcover.localhost on Windows

// ---- rendering ---------------------------------------------------------------
const pct = (b) => Math.min(100, Math.max(0, Math.round(((b.page + b.frac) / Math.max(1, b.pageCount)) * 100)));
const coverImg = (b, cls = '') => `<img class="cover ${cls}" alt="" loading="lazy" data-cover="${b.id}" src="${COVER_BASE}/${b.id}?v=${b.coverVer ?? 0}">`;

const recentItem = (b, ep) => `
  <article class="recent" data-id="${b.id}">
    ${coverImg(b)}
    <div class="recent__body">
      <a class="recent__title" href="reader.html?book=${b.id}">${esc(b.title)}</a>
      <div class="recent__meta num" title="${esc(t('pageOf', { p: b.page + 1, n: b.pageCount }))}">${ep ? t('episodeOf', { i: ep.index, n: ep.total }) : t('pageOf', { p: b.page + 1, n: b.pageCount })}</div>
      <div class="bar"><i style="width:${pct(b)}%"></i></div>
    </div>
    <div class="recent__side">
      <button class="more" data-more="${b.id}" aria-label="More"><svg class="i"><use href="icons.svg#more"/></svg></button>
      <span>${esc(ago(b.lastReadAt))}</span><span class="num">${pct(b)}%</span>
    </div>
  </article>`;
const addedItem = (b) => `<div class="added" data-id="${b.id}" title="${esc(b.title)}">${coverImg(b)}<b>${esc(b.title)}</b></div>`;

async function refresh() {
  const [recent, added] = await Promise.all([api.recent(3).catch(() => []), api.added(4).catch(() => [])]);
  // "Episode 12 / 128": null for books whose chapter mode is None / that have no chapters => falls back to the page line
  const eps = await fetchEpisodeInfo(invoke, recent).catch(() => []);
  document.getElementById('recent').innerHTML = recent.length ? recent.map((b, i) => recentItem(b, eps[i])).join('') : `<p class="empty">${t('noRecent')}</p>`;
  document.getElementById('added').innerHTML  = added.length  ? added.map(addedItem).join('')  : `<p class="empty" style="grid-column:1/-1;padding:0">${t('noAdded')}</p>`;
}

// covers: a 404 means "not generated yet" -> ask Rust once, then retry
document.addEventListener('error', async (e) => {
  const img = e.target; if (!(img instanceof HTMLImageElement) || !img.dataset.cover || img.dataset.retried) return;
  img.dataset.retried = '1';
  try { await api.cover(+img.dataset.cover); img.src = img.src.replace(/v=\d+/, 'v=' + Date.now()); } catch {}
}, true);

// open a book from either list (title is a real link for keyboard users; the card is a mouse convenience)
document.addEventListener('click', (e) => {
  const more = e.target.closest('[data-more]');
  if (more) { e.stopPropagation(); return openMenu(more, +more.dataset.more); }
  const card = e.target.closest('.recent, .added');
  if (card && !e.target.closest('a')) location.href = `reader.html?book=${card.dataset.id}`;
});

// ---- row menu ----------------------------------------------------------------
let menu;
function openMenu(btn, id) {
  menu?.remove();
  menu = Object.assign(document.createElement('div'), { className: 'menu' });
  menu.innerHTML = `<button data-a="open">${t('open')}</button><button data-a="reveal">${t('reveal')}</button>`;
  const r = btn.getBoundingClientRect();
  menu.style.top = r.bottom + 4 + 'px'; menu.style.left = Math.min(r.right - 168, innerWidth - 180) + 'px';
  menu.onclick = (e) => { const a = e.target.closest('button')?.dataset.a; if (a === 'open') location.href = `reader.html?book=${id}`; if (a === 'reveal') api.reveal(id).catch(() => {}); menu.remove(); };
  document.body.append(menu);
}
addEventListener('pointerdown', (e) => { if (menu && !menu.contains(e.target) && !e.target.closest('[data-more]')) { menu.remove(); menu = null; } });
addEventListener('keydown', (e) => { if (e.key === 'Escape') { menu?.remove(); menu = null; } });

// ---- import: picker + drag & drop --------------------------------------------
async function doImport(paths) {
  if (!paths?.length) return;
  try {
    const r = await api.import(paths);
    toast(r.added ? t('imported', { n: r.added }) : t('importNone'));
    await refresh();
  } catch (e) { toast(t('importFail', { e: e?.detail ?? e?.message ?? e })); }
}
const dialog = T.dialog;
const pickFiles  = async () => { const p = await dialog.open({ multiple: true, filters: [
  { name: 'Comics', extensions: ['zip', 'cbz'] }, { name: 'Images', extensions: ['jpg', 'jpeg', 'png', 'webp', 'gif', 'avif', 'bmp'] }] }); doImport([].concat(p ?? [])); };
const pickFolder = async () => { const p = await dialog.open({ directory: true, multiple: true }); doImport([].concat(p ?? [])); };

document.getElementById('pick').addEventListener('click', (e) => {
  menu?.remove();
  menu = Object.assign(document.createElement('div'), { className: 'menu' });
  menu.innerHTML = `<button data-a="f"><svg class="i"><use href="icons.svg#image"/></svg>${t('pickFiles')}</button><button data-a="d"><svg class="i"><use href="icons.svg#folder"/></svg>${t('pickFolder')}</button>`;
  const r = e.currentTarget.getBoundingClientRect();
  menu.style.top = r.bottom + 6 + 'px'; menu.style.left = r.left + r.width / 2 - 100 + 'px';
  menu.onclick = (ev) => { const a = ev.target.closest('button')?.dataset.a; menu.remove(); menu = null; if (a === 'f') pickFiles(); if (a === 'd') pickFolder(); };
  document.body.append(menu); e.stopPropagation();
});

const zone = document.getElementById('zone');
T.webview.getCurrentWebview().onDragDropEvent(({ payload: p }) => {
  if (p.type === 'enter' || p.type === 'over') zone.classList.add('is-over');
  else { zone.classList.remove('is-over'); if (p.type === 'drop') doImport(p.paths); }   // dropped ANYWHERE in the window imports
});

// ---- search: Enter goes to Library with ?q= ; "/" or Ctrl+F focuses ------------
const q = document.getElementById('q');
q.addEventListener('keydown', (e) => { if (e.key === 'Enter' && q.value.trim()) location.href = `library.html?q=${encodeURIComponent(q.value.trim())}`; });
addEventListener('keydown', (e) => { if ((e.key === '/' || (e.ctrlKey && e.key === 'f')) && document.activeElement !== q) { e.preventDefault(); q.focus(); } });

// ---- boot --------------------------------------------------------------------
applyTheme(); applyI18n(); wireWindow(); refresh();
T.app.getVersion().then(v => { document.getElementById('ver').textContent = `v${v}  |  mh-reader`; });
