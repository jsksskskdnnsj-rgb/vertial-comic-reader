// settings.js - every control writes to the store its consumer actually reads:
//   language  -> localStorage 'mh.lang' (shell.js t())
//   accent    -> localStorage 'mh.accent' (shell-ui.js applyAccent)
//   background-> theme config hero via set_hero_image/reset_hero_image (shell-ui.js applyBackground)
//   bg dim    -> prefs 'bgDim' (shell-ui.js)
//   reading   -> prefs keys reader-boot.js reads: 'mode', 'reader.widthPct',
//                'reader.gap', 'reader.dim', 'saver', 'autoSpeed'
//   chapters  -> backend get/set_chapter_defaults
//   cache     -> cache_stats / cache_set_limit / cache_clear (+ legacy clear_cache)
//   data dir  -> get_data_dir / pick_data_dir / set_data_dir / open_data_dir
import { T, invoke, t, toast, bootPage, prefs, confirmDialog, lang } from './shell.js';
import { lib } from './legacy-api.js';
import { mountShell, applyBackground } from './shell-ui.js';
import { sanePages, DEFAULT_PAGES } from './chapters.js';

bootPage('settings');
mountShell('settings');
const $ = (id) => document.getElementById(id);
const IMG_EXTS = ['jpg', 'jpeg', 'png', 'webp', 'gif', 'avif', 'bmp', 'jfif'];

// ---- language ------------------------------------------------------------------
$('langSel').value = localStorage.getItem('mh.lang') || 'zh-CN';
$('langSel').addEventListener('change', (e) => {
  localStorage.setItem('mh.lang', e.target.value);
  location.reload();
});

// ---- accent ---------------------------------------------------------------------
function syncAccent() {
  const cur = localStorage.getItem('mh.accent') || '#80e5b6';
  document.querySelectorAll('#accents .swatch').forEach((b) =>
    b.setAttribute('aria-pressed', String(b.dataset.accent.toLowerCase() === cur.toLowerCase())));
  $('accentCustom').value = cur;
}
$('accents').addEventListener('click', (e) => {
  const b = e.target.closest('[data-accent]'); if (!b) return;
  localStorage.setItem('mh.accent', b.dataset.accent);
  document.documentElement.style.setProperty('--accent', b.dataset.accent);
  document.documentElement.style.setProperty('--accent2', b.dataset.accent);
  syncAccent();
});
$('accentCustom').addEventListener('input', (e) => {
  localStorage.setItem('mh.accent', e.target.value);
  document.documentElement.style.setProperty('--accent', e.target.value);
  document.documentElement.style.setProperty('--accent2', e.target.value);
  syncAccent();
});
$('accentReset').addEventListener('click', () => {
  localStorage.removeItem('mh.accent');
  document.documentElement.style.removeProperty('--accent');
  document.documentElement.style.removeProperty('--accent2');
  syncAccent();
});
syncAccent();

// ---- background ------------------------------------------------------------------
$('bgChange').addEventListener('click', async () => {
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
    applyBackground();
  } catch (e) {
    toast(t('settings.imageFailed', { e: String(e?.detail ?? e?.message ?? e) }));
  }
});
$('bgReset').addEventListener('click', async () => {
  try {
    await invoke('reset_hero_image');
    const cfg = await invoke('get_theme_config').catch(() => ({ theme: 'dark', custom_pri: '', hero: 'default' }));
    await invoke('set_theme_config', { cfg: { ...cfg, hero: 'default' } });
    toast(t('settings.imageRestored'));
    document.querySelector('.bg-layer')?.remove();
    document.body.classList.remove('has-bg');
  } catch {}
});
$('bgDim').value = prefs.get('bgDim', 55);
$('bgDimVal').textContent = $('bgDim').value + '%';
$('bgDim').addEventListener('input', (e) => {
  prefs.set('bgDim', +e.target.value);
  $('bgDimVal').textContent = e.target.value + '%';
  applyBackground();
});

// ---- reading defaults --------------------------------------------------------------
const pressed = (sel, attr, val) =>
  document.querySelectorAll(sel).forEach((b) => b.setAttribute('aria-pressed', String(b.dataset[attr] == val)));
function syncReading() {
  pressed('#modeSeg button', 'mode', prefs.get('mode', 'scroll'));
  $('widthRange').value = prefs.get('reader.widthPct', 100);
  $('widthVal').textContent = $('widthRange').value + '%';
  pressed('#gapSeg button', 'gap', prefs.get('reader.gap', 0));
  $('dimRange').value = prefs.get('reader.dim', 100);
  $('dimVal').textContent = $('dimRange').value + '%';
  $('saverChk').checked = !!prefs.get('saver', false);
  $('autoRange').value = prefs.get('autoSpeed', 4);
  $('autoVal').textContent = $('autoRange').value;
}
$('modeSeg').addEventListener('click', (e) => {
  const b = e.target.closest('[data-mode]'); if (!b) return;
  prefs.set('mode', b.dataset.mode); syncReading();
});
$('widthRange').addEventListener('input', (e) => { prefs.set('reader.widthPct', +e.target.value); syncReading(); });
$('gapSeg').addEventListener('click', (e) => {
  const b = e.target.closest('[data-gap]'); if (!b) return;
  prefs.set('reader.gap', +b.dataset.gap); syncReading();
});
$('dimRange').addEventListener('input', (e) => { prefs.set('reader.dim', +e.target.value); syncReading(); });
$('saverChk').addEventListener('change', (e) => prefs.set('saver', e.target.checked));
$('autoRange').addEventListener('input', (e) => { prefs.set('autoSpeed', +e.target.value); syncReading(); });
syncReading();

// ---- chapter defaults ---------------------------------------------------------------
let chMode = 'folder', chPages = DEFAULT_PAGES, chTimer = 0;
const CH_HELP = { none: 'chapters.noneHelp', folder: 'chapters.folderHelp', pages: 'chapters.pagesHelp' };
function syncCh() {
  pressed('#chSeg button', 'chmode', chMode);
  $('chNRow').style.display = chMode === 'pages' ? '' : 'none';
  $('chHelp').textContent = t(CH_HELP[chMode]);
  if (document.activeElement !== $('chN')) $('chN').value = chPages;
}
const saveCh = () => {
  clearTimeout(chTimer);
  chTimer = setTimeout(() =>
    invoke('set_chapter_defaults', { mode: chMode, pages: chPages })
      .then(() => toast(t('common.saved'))).catch(() => {}), 400);
};
$('chSeg').addEventListener('click', (e) => {
  const b = e.target.closest('[data-chmode]'); if (!b) return;
  chMode = b.dataset.chmode; syncCh(); saveCh();
});
$('chN').addEventListener('input', (e) => {
  if (e.target.value === '' || +e.target.value < 1) return;
  chPages = sanePages(e.target.value); syncCh(); saveCh();
});
invoke('get_chapter_defaults')
  .then((d) => { chMode = d.mode; chPages = sanePages(d.pages); syncCh(); })
  .catch(syncCh);
syncCh();

// ---- cache ---------------------------------------------------------------------------
const fmtMB = (bytes) => `${((bytes || 0) / 1048576).toFixed(1)} MB`;
async function refreshCache() {
  try {
    const s = await invoke('cache_stats');
    $('cacheStats').textContent = t('settings.cacheUsed', { used: fmtMB(s.bytes), limit: fmtMB(s.limitBytes) }) + ` · ${s.entries}`;
    if (document.activeElement !== $('cacheLimitMb')) $('cacheLimitMb').value = Math.round((s.limitBytes || 0) / 1048576);
  } catch { $('cacheStats').textContent = '—'; }
}
$('cacheLimitBtn').addEventListener('click', async () => {
  const mb = +$('cacheLimitMb').value;
  if (!(mb >= 64)) return;
  try { await invoke('cache_set_limit', { bytes: Math.round(mb * 1048576) }); toast(t('common.saved')); refreshCache(); }
  catch (e) { toast(String(e?.detail ?? e?.message ?? e)); }
});
$('cacheClearBtn').addEventListener('click', async () => {
  try {
    await invoke('cache_clear', { bookId: null });
    await invoke('clear_cache').catch(() => {});
    toast(t('settings.cacheCleared'));
    refreshCache();
  } catch (e) { toast(String(e?.detail ?? e?.message ?? e)); }
});
refreshCache();

// ---- data dir ---------------------------------------------------------------------------
async function refreshDataDir() {
  try { $('dataDir').textContent = await invoke('get_data_dir'); } catch { $('dataDir').textContent = '—'; }
}
$('dataDirChange').addEventListener('click', async () => {
  try {
    const picked = await invoke('pick_data_dir');
    if (!picked) return;
    $('dataDir').textContent = await invoke('set_data_dir', { path: picked });
    toast(t('common.saved'));
  } catch (e) { toast(String(e?.detail ?? e?.message ?? e)); }
});
$('dataDirOpen').addEventListener('click', () => invoke('open_data_dir').catch(() => {}));
refreshDataDir();

// ---- about / stats / danger ----------------------------------------------------------------
T.app?.getVersion?.().then((v) => { $('ver').textContent = `v${v}`; }).catch(() => {});
lib.stats().then((s) => {
  const cell = (v, label) => `<div class="stat"><b class="num">${v}</b><span>${esc(label)}</span></div>`;
  $('statsGrid').innerHTML =
    cell(s.books, t('lib.title')) + cell(s.pages, lang() === 'en' ? 'Pages' : '总页数') +
    cell(s.folders, t('lib.folders')) + cell(s.inProgress, t('reader.history')) +
    cell(s.finished, t('end.title')) + cell(s.bookmarkedBooks, t('bookmarks.title'));
}).catch(() => {});
$('clearShelf').addEventListener('click', async () => {
  if (!await confirmDialog({ title: t('lib.remove'), message: t('lib.noBooks'), ok: t('common.delete'), danger: true })) return;
  try { await invoke('shelf_clear'); toast(t('common.saved')); }
  catch (e) { toast(String(e?.detail ?? e?.message ?? e)); }
});
