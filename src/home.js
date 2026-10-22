// home.js - welcome banner (custom background), drop import, quick tiles,
// recent reading / recently added (adapters: list_recent_books/list_recent_added).
import { T, invoke, t, esc, toast, bootPage } from './shell.js';
import { lib } from './legacy-api.js';
import { mountShell, coverUrl, wireCoverFallback, watchDrops } from './shell-ui.js';
import { fetchEpisodeInfo } from './chapters.js';

bootPage('home');
mountShell('home');
wireCoverFallback();

const $ = (id) => document.getElementById(id);
const IMG_EXTS = ['jpg', 'jpeg', 'png', 'webp', 'gif', 'avif', 'bmp', 'jfif'];

async function doImport(paths) {
  if (!paths?.length) return;
  try {
    const r = await lib.importPaths(paths, null);
    if (r.added) toast(t('imp.importedToast', { n: r.added }));
    else if (!r.failed?.length) toast(t('imp.nothingNew'));
    if (r.failed?.length) toast(t('imp.failed', { e: r.failed[0].reason }));
    loadLists();
  } catch (e) {
    toast(String(e?.detail ?? e?.message ?? e));
  }
}

async function pickFiles() {
  const picked = await T.dialog.open({
    multiple: true,
    filters: [
      { name: t('imp.zipFilter'), extensions: ['zip', 'cbz'] },
      { name: t('imp.imageFilter'), extensions: IMG_EXTS },
    ],
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

// drag & drop anywhere on the home page (native queue filled by Rust)
const zone = $('zone');
addEventListener('dragover', (e) => { e.preventDefault(); zone.classList.add('is-over'); });
addEventListener('dragleave', () => zone.classList.remove('is-over'));
addEventListener('drop', (e) => { e.preventDefault(); zone.classList.remove('is-over'); });
watchDrops(doImport);

// ---- custom background -------------------------------------------------------
$('heroBg').addEventListener('click', async () => {
  const picked = await T.dialog.open({
    multiple: false,
    filters: [{ name: t('settings.imageFilter'), extensions: IMG_EXTS }],
  }).catch(() => null);
  if (!picked) return;
  try {
    const dest = await invoke('set_hero_image', { src: picked });
    const cfg = await invoke('get_theme_config').catch(() => ({ theme: 'dark', custom_pri: '', hero: 'default' }));
    await invoke('set_theme_config', { cfg: { ...cfg, hero: dest } });
    toast(t('settings.imageSaved'));
    setTimeout(() => location.reload(), 450);
  } catch (e) {
    toast(t('settings.imageFailed', { e: String(e?.detail ?? e?.message ?? e) }));
  }
});
$('heroBgReset').addEventListener('click', async () => {
  try {
    await invoke('reset_hero_image');
    const cfg = await invoke('get_theme_config').catch(() => ({ theme: 'dark', custom_pri: '', hero: 'default' }));
    await invoke('set_theme_config', { cfg: { ...cfg, hero: 'default' } });
    toast(t('settings.imageRestored'));
    setTimeout(() => location.reload(), 450);
  } catch {}
});

// ---- lists -------------------------------------------------------------------
function card(b, sub, pct) {
  return `<article class="book-card" data-open="${b.id}">
    <div class="cover-wrap"><img class="cover" alt="" loading="lazy" data-cover="${b.id}" src="${coverUrl(b.id, b.coverVer)}"></div>
    <b title="${esc(b.title)}">${esc(b.title)}</b>
    <div class="meta num">${esc(sub)}</div>
    ${pct != null ? `<div class="bar"><i style="width:${pct}%"></i></div>` : ''}
  </article>`;
}

async function loadLists() {
  const recent = await lib.recentBooks(12).catch(() => []);
  const eps = await fetchEpisodeInfo(invoke, recent);
  $('recent').innerHTML = recent.map((b, i) => {
    const ep = eps[i];
    const pct = b.pageCount ? Math.min(100, Math.round(((b.page + (b.frac || 0)) / b.pageCount) * 100)) : 0;
    const sub = ep
      ? t('common.episodeOf', { i: ep.index, n: ep.total })
      : t('common.pageOf', { p: (b.page ?? 0) + 1, n: b.pageCount ?? 0 });
    return card(b, sub, pct);
  }).join('');
  $('noRecent').hidden = recent.length > 0;

  const added = await lib.recentAdded(12).catch(() => []);
  $('added').innerHTML = added.map((b) =>
    card(b, t('book.pages', { n: b.pageCount ?? 0 }), null)).join('');
  $('noAdded').hidden = added.length > 0;
}
document.addEventListener('click', (e) => {
  const c = e.target.closest('[data-open]');
  if (c) location.href = `reader.html?book=${c.dataset.open}`;
});
loadLists();
