// home.js - banner (its own custom image, independent of the window background), stacked reading-history
// cards (list_recent_books; click resumes), drop import, quick tiles, recently added.
// v0.6: history is a stack of progress cards (the chapter-list panel is gone); drag-over highlight
// no longer flickers; reset errors are reported; banner changes re-apply without a reload.
import { T, invoke, t, esc, ago, toast, bootPage } from './shell.js';
import { lib } from './legacy-api.js';
import { mountShell, coverUrl, wireCoverFallback, watchDrops, applyBackground } from './shell-ui.js';
import { fetchEpisodeInfo } from './chapters.js';
import { setBanner, resetBanner } from './theme-api.js';
import { inspectFolder, looksLikeFile } from './folder-scan.js';
import { archiveAndImageFilters, imageFilters } from './image-exts.js';

bootPage('home');
mountShell('home');
wireCoverFallback();

const $ = (id) => document.getElementById(id);
const errText = (e) => String(e?.detail ?? e?.message ?? e);
const skeletons = (n, kind) => Array.from({ length: n }, () => `<div class="skel skel--${kind}" aria-hidden="true"></div>`).join('');

async function doImport(paths) {
  if (!paths?.length) return;
  // folders are imported with all their subfolders; one with no images at any depth is refused, not added silently
  const keep = []; let empty = 0;
  for (const p of paths) {
    if (looksLikeFile(p)) { keep.push(p); continue; }
    const r = await inspectFolder(p, lib.listImages);
    if (r.ok && r.empty) empty++; else keep.push(p);
  }
  const note = empty ? ' · ' + t('imp.emptySkipped') : '';
  if (!keep.length) { toast(t('imp.emptySkipped')); return; }
  try {
    const r = await lib.importPaths(keep, null);
    if (r.added) toast(t('imp.importedToast', { n: r.added }) + note);
    else if (!r.failed?.length) toast(t('imp.nothingNew') + note);
    if (r.failed?.length) toast(t('imp.failed', { e: r.failed[0].reason }) + note);
    loadLists();
  } catch (e) { toast(errText(e)); }
}
async function pickFiles() {
  const picked = await T.dialog.open({
    multiple: true,
    filters: archiveAndImageFilters(t),
  }).catch(() => null);
  doImport([].concat(picked ?? []));
}
async function pickFolders() {
  const picked = await T.dialog.open({ multiple: true, directory: true }).catch(() => null);
  doImport([].concat(picked ?? []));
}
$('pickFiles').addEventListener('click', pickFiles);
$('pickFolders').addEventListener('click', pickFolders);
$('heroImport').addEventListener('click', () => (location.href = 'import.html'));

// drag & drop anywhere on the page (native queue filled by Rust); enter/leave fire per child, so count
const zone = $('zone');
let dragDepth = 0;
addEventListener('dragenter', (e) => { e.preventDefault(); dragDepth++; zone.classList.add('is-over'); });
addEventListener('dragover', (e) => e.preventDefault());
addEventListener('dragleave', () => { dragDepth = Math.max(0, dragDepth - 1); if (!dragDepth) zone.classList.remove('is-over'); });
addEventListener('drop', (e) => { e.preventDefault(); dragDepth = 0; zone.classList.remove('is-over'); });
watchDrops(doImport);

// ---- custom home banner (independent of the window background, which lives in Settings) -----------------
$('heroBg').addEventListener('click', async () => {
  const picked = await T.dialog.open({ multiple: false, filters: imageFilters(t) }).catch(() => null);
  if (!picked) return;
  try { await setBanner(picked); toast(t('settings.imageSaved')); applyBackground(); }
  catch (e) { toast(t('settings.imageFailed', { e: errText(e) })); }
});
$('heroBgReset').addEventListener('click', async () => {
  try { await resetBanner(); toast(t('settings.imageRestored')); applyBackground(); }
  catch (e) { toast(t('settings.imageFailed', { e: errText(e) })); }
});

// ---- lists --------------------------------------------------------------------------------------
function hcard(b, sub, pct) {
  return `<article class="hcard" data-open="${b.id}" tabindex="0" role="link">
    <img class="cover" alt="" loading="lazy" data-cover="${b.id}" src="${coverUrl(b.id, b.coverVer)}">
    <div class="hcard__body">
      <div class="hcard__title" title="${esc(b.title)}">${esc(b.title)}</div>
      <div class="hcard__ep">${esc(sub)}</div>
      <div class="bar"><i style="width:${pct}%"></i></div>
      <div class="hcard__meta"><span class="num">${pct}%</span><span>${esc(ago(b.lastReadAt))}</span></div>
    </div>
  </article>`;
}
function card(b, sub) {
  return `<article class="book-card" data-open="${b.id}" tabindex="0" role="link">
    <div class="cover-wrap"><img class="cover" alt="" loading="lazy" data-cover="${b.id}" src="${coverUrl(b.id, b.coverVer)}"></div>
    <b title="${esc(b.title)}">${esc(b.title)}</b>
    <div class="meta num">${esc(sub)}</div>
  </article>`;
}

async function loadLists() {
  $('noRecent').hidden = true;
  $('noAdded').hidden = true;
  $('recent').setAttribute('aria-busy', 'true');
  $('added').setAttribute('aria-busy', 'true');
  $('recent').innerHTML = skeletons(3, 'row');
  $('added').innerHTML = skeletons(4, 'card');
  const recent = await lib.recentBooks(8).catch(() => []);
  const eps = await fetchEpisodeInfo(invoke, recent).catch(() => []);
  $('recent').innerHTML = recent.map((b, i) => {
    const ep = eps?.[i];
    const pct = b.pageCount ? Math.min(100, Math.round(((b.page + (b.frac || 0)) / b.pageCount) * 100)) : 0;
    const sub = ep
      ? t('common.episodeOf', { i: ep.index, n: ep.total })
      : t('common.pageOf', { p: (b.page ?? 0) + 1, n: b.pageCount ?? 0 });
    return hcard(b, sub, pct);
  }).join('');
  $('recent').removeAttribute('aria-busy');
  $('noRecent').hidden = recent.length > 0;

  const added = await lib.recentAdded(12).catch(() => []);
  $('added').innerHTML = added.map((b) =>
    card(b, `${t('book.pages', { n: b.pageCount ?? 0 })} · ${ago(b.addedAt)}`)).join('');
  $('added').removeAttribute('aria-busy');
  $('noAdded').hidden = added.length > 0;
}
const open = (el) => { const c = el?.closest('[data-open]'); if (c) location.href = `reader.html?book=${c.dataset.open}`; };
document.addEventListener('click', (e) => open(e.target));
document.addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { const c = e.target.closest?.('[data-open]'); if (c) { e.preventDefault(); open(c); } } });
loadLists();
