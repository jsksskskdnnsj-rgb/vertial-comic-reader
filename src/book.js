// book.js - book details page (?book=<id>): cover, progress, type, size, path, bookmarks, folder,
// and the same actions as the library menu (continue, start over, rename, locate, reveal, remove,
// reset page edits, move to folder). Data: shelf_get, folder_list, episode_info (chapters.js),
// bookmark_counts, check_books_missing, page_edit_info.
import { invoke, t, esc, ago, fmtBytes, bootPage, lang } from './shell.js';
import { lib } from './legacy-api.js';
import { mountShell, coverUrl, wireCoverFallback } from './shell-ui.js';
import { fetchEpisodeInfo } from './chapters.js';
import * as act from './book-actions.js';

const $ = (id) => document.getElementById(id);
bootPage('book');
mountShell('library');
wireCoverFallback();

const id = +new URLSearchParams(location.search).get('book');
const KIND_KEY = { zip: 'type.zip', folder: 'type.folder', images: 'type.images' };
let book = null;
let coverBust = 0;

function parsePaths(b) { try { const p = JSON.parse(b.paths); return Array.isArray(p) ? p : []; } catch { return []; } }

async function load() {
  book = id ? await lib.shelfGet(id).catch(() => null) : null;
  $('bookLoading').hidden = true;
  if (!book) {
    $('hero').hidden = true; $('details').hidden = true; $('notFound').hidden = false;
    return;
  }
  $('notFound').hidden = true; $('hero').hidden = false; $('details').hidden = false;
  const [folders, eps, counts, miss, info] = await Promise.all([
    lib.folders().catch(() => []),
    fetchEpisodeInfo(invoke, [{ id: book.id, page: book.last_page ?? 0 }]).catch(() => []),
    invoke('bookmark_counts', { bookIds: [book.id] }).catch(() => []),
    lib.missing().catch(() => []),
    lib.pageEditInfo(book.id).catch(() => null),
  ]);
  const ep = eps?.[0] || null;
  const bm = (counts || []).find((r) => r.bookId === book.id)?.count || 0;
  const isMissing = (miss || []).includes(book.id);
  const started = book.last_page > 0;
  const pct = book.page_count ? Math.min(100, Math.round(((book.last_page || 0) / book.page_count) * 100)) : 0;

  $('cover').dataset.cover = String(book.id);
  delete $('cover').dataset.retried;
  $('cover').src = coverUrl(book.id, coverBust);
  $('name').textContent = book.name;
  document.title = `${book.name} · ${t('app.name')}`;
  $('chips').innerHTML = [KIND_KEY[book.kind] ? t(KIND_KEY[book.kind]) : '', t('book.pages', { n: book.page_count })]
    .filter(Boolean).map((s) => `<span>${esc(s)}</span>`).join('');
  $('barFill').style.width = pct + '%';
  $('progText').textContent = started ? t('common.pageOf', { p: book.last_page + 1, n: book.page_count }) : t('lib.notStarted');
  $('epText').textContent = ep ? t('common.episodeOf', { i: ep.index, n: ep.total }) : '';
  $('continueLbl').textContent = t(started ? 'book.continue' : 'book.start');
  $('missingHint').hidden = !isMissing;
  $('relinkBtn').hidden = !isMissing;

  const paths = parsePaths(book);
  const pathHtml = paths.length
    ? esc(paths[0]) + (paths.length > 1 ? ` <span class="muted">${esc(t('book.pathMore', { n: paths.length - 1 }))}</span>` : '')
    : '—';
  const row = (k, v) => `<dt>${esc(t(k))}</dt><dd>${v}</dd>`;
  $('kv').innerHTML =
    row('book.type', esc(KIND_KEY[book.kind] ? t(KIND_KEY[book.kind]) : '—')) +
    row('book.size', `<span class="num">${esc(fmtBytes(book.total_size))}</span>`) +
    row('book.lastRead', started ? esc(ago(book.updated_at)) : esc(t('lib.notStarted'))) +
    row('book.added', esc(book.created_at ? new Date(book.created_at * 1000).toLocaleDateString(lang() === 'en' ? 'en-US' : 'zh-CN') : '—')) +
    row('book.bookmarks', `<span class="num">${bm}</span>`) +
    row('book.path', pathHtml);
  $('folderSel').innerHTML = `<option value="">${esc(t('lib.noFolder'))}</option>` +
    folders.map((f) => `<option value="${f.id}"${f.id === book.folder_id ? ' selected' : ''}>${esc(f.name)}</option>`).join('');
  $('resetEditRow').hidden = !info?.edited;
}

const after = async (ok, bump) => { if (ok) { if (bump) coverBust = Date.now(); await load(); } };
$('continueBtn').addEventListener('click', () => { location.href = `reader.html?book=${book.id}`; });
$('startOverBtn').addEventListener('click', async () => after(await act.startOver(book)));
$('renameBtn').addEventListener('click', async () => after(await act.renameBook(book)));
$('relinkBtn').addEventListener('click', async () => after(await act.relinkBook(book), true));
$('revealBtn').addEventListener('click', () => act.revealBook(book));
$('removeBtn').addEventListener('click', async () => { if (await act.removeBook(book)) location.href = 'library.html'; });
$('resetEditBtn').addEventListener('click', async () => after(await act.resetEdits(book), true));
$('folderSel').addEventListener('change', async () => {
  const v = $('folderSel').value;
  if (!await act.moveBook(book, v ? +v : null)) await load();
});
load();
