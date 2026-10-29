// settings.js - every control writes to the store its consumer reads:
//   language   -> localStorage 'mh.lang' (shell.js t())
//   accent     -> localStorage 'mh.accent' (shell-ui.js applyAccent)
//   background -> theme config `hero` via set_hero_image / reset_hero_image (shell-ui.js applyBackground)
//   bg dim     -> prefs 'bgDim' -> --bg-dim (works for the default AND a custom image)
//   reading    -> prefs keys reader-boot.js reads: 'mode', 'reader.widthPct', 'reader.gap',
//                 'reader.dim', 'saver', 'autoSpeed'
//   chapters   -> get/set_chapter_defaults
//   cache      -> cache_stats / cache_set_limit / cache_clear
//   data dir   -> get_data_dir / pick_data_dir / set_data_dir / open_data_dir (behaviour unchanged)
// v0.6 fixes: esc() was used but never imported, so the stats grid silently stayed empty; the
// "clear library" button was labelled "clear cache" with a "no books" confirmation; the data-folder
// row used the "Folders" and "Change image" strings; swatch labels are translated; stat label
// "Pages" comes from the dictionaries; the dim slider also affects the default background.
import { T, invoke, t, esc, toast, bootPage, prefs, confirmDialog, choiceDialog, lang, setLang } from './shell.js';
import { lib } from './legacy-api.js';
import { mountShell, applyBackground, applyAppearance, applyExtras } from './shell-ui.js';
import { BORDER_PRESETS, BG_COLOR_PRESETS, BG_COLOR_DEFAULT, isHex } from './appearance-extras.js';
import { DEFAULTS, PANEL_MIN, PANEL_MAX } from './appearance.js';
import { sanePages, DEFAULT_PAGES } from './chapters.js';
import { setBackground, resetBackground, setBanner, resetBanner } from './theme-api.js';
import { imageFilters } from './image-exts.js';

bootPage('settings');
mountShell('settings');
const $ = (id) => document.getElementById(id);
const errText = (e) => String(e?.detail ?? e?.message ?? e);

// ---- language ----------------------------------------------------------------------
$('langSel').value = lang();
$('langSel').addEventListener('change', (e) => { setLang(e.target.value); location.reload(); });

// ---- accent -------------------------------------------------------------------------
const ACCENTS = [
  ['#80e5b6', 'accent.emerald'], ['#b7a6f5', 'accent.lavender'], ['#7cc4f0', 'accent.sky'], ['#f2a0b5', 'accent.rose'], ['#eec27f', 'accent.amber'],
];
$('accents').innerHTML = ACCENTS.map(([hex, key]) =>
  `<button class="swatch" data-accent="${hex}" style="background:${hex}" title="${esc(t(key))}" aria-label="${esc(t(key))}"></button>`).join('');
function setAccent(hex) {
  const root = document.documentElement.style;
  if (hex) { localStorage.setItem('mh.accent', hex); root.setProperty('--accent', hex); root.setProperty('--accent2', hex); }
  else { localStorage.removeItem('mh.accent'); root.removeProperty('--accent'); root.removeProperty('--accent2'); }
  syncAccent();
}
function syncAccent() {
  const cur = localStorage.getItem('mh.accent') || '#80e5b6';
  document.querySelectorAll('#accents .swatch').forEach((b) =>
    b.setAttribute('aria-pressed', String(b.dataset.accent.toLowerCase() === cur.toLowerCase())));
  $('accentCustom').value = cur;
}
$('accents').addEventListener('click', (e) => { const b = e.target.closest('[data-accent]'); if (b) setAccent(b.dataset.accent); });
$('accentCustom').addEventListener('input', (e) => setAccent(e.target.value));
$('accentReset').addEventListener('click', () => setAccent(null));
syncAccent();

// ---- background and banner: two independent images (theme config `hero` and `banner`) -------------------
async function pickImage() {
  return T.dialog.open({ multiple: false, filters: imageFilters(t) }).catch(() => null);
}
async function imageAction(fn, okKey) {
  try { await fn(); toast(t(okKey)); applyBackground(); }
  catch (e) { toast(t('settings.imageFailed', { e: errText(e) })); }
}
$('bgChange').addEventListener('click', async () => { const p = await pickImage(); if (p) imageAction(() => setBackground(p), 'settings.imageSaved'); });
$('bgReset').addEventListener('click', () => imageAction(resetBackground, 'settings.imageRestored'));
$('bannerChange').addEventListener('click', async () => { const p = await pickImage(); if (p) imageAction(() => setBanner(p), 'settings.imageSaved'); });
$('bannerReset').addEventListener('click', () => imageAction(resetBanner, 'settings.imageRestored'));
$('bgDim').value = prefs.get('bgDim', 55);
$('bgDimVal').textContent = $('bgDim').value + '%';
$('bgDim').addEventListener('input', (e) => {
  prefs.set('bgDim', +e.target.value);
  $('bgDimVal').textContent = e.target.value + '%';
  applyBackground();
});

// ---- reading defaults -------------------------------------------------------------------
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
$('modeSeg').addEventListener('click', (e) => { const b = e.target.closest('[data-mode]'); if (b) { prefs.set('mode', b.dataset.mode); syncReading(); } });
$('widthRange').addEventListener('input', (e) => { prefs.set('reader.widthPct', +e.target.value); syncReading(); });
$('gapSeg').addEventListener('click', (e) => { const b = e.target.closest('[data-gap]'); if (b) { prefs.set('reader.gap', +b.dataset.gap); syncReading(); } });
$('dimRange').addEventListener('input', (e) => { prefs.set('reader.dim', +e.target.value); syncReading(); });
$('saverChk').addEventListener('change', (e) => prefs.set('saver', e.target.checked));
$('autoRange').addEventListener('input', (e) => { prefs.set('autoSpeed', +e.target.value); syncReading(); });
syncReading();

// ---- panel opacity + wallpaper fit (CSS variables only; applyAppearance writes --panel-a, --panel2-a, --bg-size) ------------
function syncAppearance() {
  const op = Math.min(PANEL_MAX, Math.max(PANEL_MIN, +prefs.get('panelOpacity', DEFAULTS.panelOpacity) || DEFAULTS.panelOpacity));
  $('panelOpacity').value = op;
  $('panelOpacityVal').textContent = $('panelOpacity').value + '%';
  pressed('#fitSeg button', 'fit', prefs.get('bgFit', DEFAULTS.bgFit));
}
$('panelOpacity').addEventListener('input', (e) => { prefs.set('panelOpacity', +e.target.value); syncAppearance(); applyAppearance(); });
$('fitSeg').addEventListener('click', (e) => {
  const b = e.target.closest('[data-fit]'); if (!b) return;
  prefs.set('bgFit', b.dataset.fit); syncAppearance(); applyAppearance();
});
$('appearanceReset').addEventListener('click', () => {
  prefs.set('panelOpacity', DEFAULTS.panelOpacity); prefs.set('bgFit', DEFAULTS.bgFit); prefs.set('bgDim', DEFAULTS.bgDim);
  $('bgDim').value = DEFAULTS.bgDim; $('bgDimVal').textContent = DEFAULTS.bgDim + '%';
  syncAppearance(); applyAppearance(); applyBackground();
  toast(t('common.saved'));
});
syncAppearance();

// ---- background style + colour, border colour, close-to-tray (v0.6.4). Added after the panel-opacity / fit code; that code is untouched. ----
$('bgSwatches').innerHTML = BG_COLOR_PRESETS.map((hex) => `<button class="swatch" data-bgc="${hex}" style="background:${hex}" title="${hex}" aria-label="${hex}"></button>`).join('');
$('borderSwatches').innerHTML = BORDER_PRESETS.map(([v, key]) =>
  `<button class="swatch${v === 'accent' ? ' swatch--accent' : ''}" data-border="${v}"${v === 'accent' ? '' : ` style="background:${v}"`} title="${esc(t(key))}" aria-label="${esc(t(key))}"></button>`).join('');
function syncExtras() {
  pressed('#bgModeSeg button', 'bgmode', prefs.get('bgMode', 'image') === 'color' ? 'color' : 'image');
  const bgc = prefs.get('bgColor', null); const bgHex = isHex(bgc) ? bgc.toLowerCase() : BG_COLOR_DEFAULT;
  $('bgColor').value = bgHex;
  document.querySelectorAll('#bgSwatches .swatch').forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.bgc === bgHex)));
  const bd = prefs.get('borderColor', null);
  document.querySelectorAll('#borderSwatches .swatch').forEach((b) => b.setAttribute('aria-pressed', String(bd !== null && b.dataset.border.toLowerCase() === String(bd).toLowerCase())));
  if (isHex(bd)) $('borderColor').value = bd;
}
$('bgModeSeg').addEventListener('click', (e) => {
  const b = e.target.closest('[data-bgmode]'); if (!b) return;
  prefs.set('bgMode', b.dataset.bgmode); syncExtras(); applyBackground();
});
$('bgSwatches').addEventListener('click', (e) => {
  const b = e.target.closest('[data-bgc]'); if (!b) return;
  prefs.set('bgColor', b.dataset.bgc); syncExtras(); applyBackground();
});
$('bgColor').addEventListener('input', (e) => { prefs.set('bgColor', e.target.value); syncExtras(); applyBackground(); });
$('borderSwatches').addEventListener('click', (e) => {
  const b = e.target.closest('[data-border]'); if (!b) return;
  prefs.set('borderColor', b.dataset.border); syncExtras(); applyExtras();
});
$('borderColor').addEventListener('input', (e) => { prefs.set('borderColor', e.target.value); syncExtras(); applyExtras(); });
$('borderReset').addEventListener('click', () => { prefs.set('borderColor', null); syncExtras(); applyExtras(); toast(t('common.saved')); });
// "Match accent" follows the accent colour: re-apply when the accent changes (these run after the accent handlers above)
$('accents').addEventListener('click', applyExtras); $('accentCustom').addEventListener('input', applyExtras); $('accentReset').addEventListener('click', applyExtras);
// "Restore appearance defaults" (panel opacity / fit / dim, handled above) also resets the three new prefs
$('appearanceReset').addEventListener('click', () => {
  prefs.set('bgMode', 'image'); prefs.set('bgColor', null); prefs.set('borderColor', null); syncExtras(); applyBackground();
});
syncExtras();

// close-to-tray: stored in the existing reader config (get_reader_config / set_reader_config), read by main.rs when the window is closed
$('closeToTray').addEventListener('change', async (e) => {
  const on = e.target.checked;
  try { const cfg = await invoke('get_reader_config'); await invoke('set_reader_config', { cfg: { ...cfg, close_to_tray: on } }); toast(t('common.saved')); }
  catch (err) { e.target.checked = !on; toast(errText(err)); }
});
invoke('get_reader_config').then((c) => { $('closeToTray').checked = c?.close_to_tray !== false; }).catch(() => {});

// ---- chapter defaults ----------------------------------------------------------------------
let chMode = 'folder', chPages = DEFAULT_PAGES, chTimer = 0;
const CH_HELP = { none: 'chapters.noneHelp', folder: 'chapters.folderHelp', pages: 'chapters.pagesHelp' };
function syncCh() {
  pressed('#chSeg button', 'chmode', chMode);
  $('chNRow').hidden = chMode !== 'pages'; // only "pages" mode has an N
  $('chHelp').textContent = t(CH_HELP[chMode]);
  if (document.activeElement !== $('chN')) $('chN').value = chPages;
}
const saveCh = () => {
  clearTimeout(chTimer);
  chTimer = setTimeout(() => invoke('set_chapter_defaults', { mode: chMode, pages: chPages })
    .then(() => toast(t('common.saved'))).catch((e) => toast(errText(e))), 400);
};
$('chSeg').addEventListener('click', (e) => { const b = e.target.closest('[data-chmode]'); if (b) { chMode = b.dataset.chmode; syncCh(); saveCh(); } });
$('chN').addEventListener('input', (e) => {
  if (e.target.value === '' || +e.target.value < 1) return;
  chPages = sanePages(e.target.value); syncCh(); saveCh();
});
invoke('get_chapter_defaults').then((d) => { chMode = d.mode; chPages = sanePages(d.pages); syncCh(); }).catch(syncCh);
syncCh();

// ---- cache -----------------------------------------------------------------------------------
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
  catch (e) { toast(errText(e)); }
});
$('cacheClearBtn').addEventListener('click', async () => {
  try {
    await invoke('cache_clear', { bookId: null });
    toast(t('settings.cacheCleared'));
    refreshCache();
  } catch (e) { toast(errText(e)); }
});
refreshCache();

// ---- data directory (default = the app's own folder; user-changeable; explicit "restore default") --------------
// set_data_dir COPIES the database, reader config and page cache into the chosen folder (files that already exist
// there are NOT overwritten, old files are kept) and writes data-dir.txt; choosing the app's own folder deletes
// data-dir.txt, which is what "restore default" does. get_default_data_dir returns that folder.
let defaultDir = null;
async function refreshDataDir() {
  let cur = '';
  try { cur = await invoke('get_data_dir'); } catch {}
  $('dataDir').textContent = cur || '—';
  try { defaultDir = await invoke('get_default_data_dir'); } catch { defaultDir = null; }
  const known = defaultDir !== null && cur !== '';
  const isDefault = known && cur === defaultDir;
  $('dataDirState').hidden = !known;
  $('dataDirState').textContent = known ? t(isDefault ? 'settings.dataDirDefault' : 'settings.dataDirCustom') : '';
  $('dataDirState').classList.toggle('is-custom', known && !isDefault);
  $('dataDirRestore').hidden = !known || isDefault;
}
async function switchDataDir(path, confirmKey) {
  if (!await confirmDialog({ title: t('settings.dataDir'), message: t(confirmKey, { path }), ok: t('common.ok') })) return;
  try {
    toast(t('settings.dataDirWorking'));
    await invoke('set_data_dir', { path });
    await refreshDataDir();
    toast(t('common.saved'));
    // main.rs opens the reader database, cache and covers ONCE at start-up from the old folder, while the shelf commands
    // re-resolve the folder on every call: until the app restarts the two halves disagree. Keep a visible notice and
    // offer to close the app (win_close).
    $('dataDirRestart').hidden = false;
    const choice = await choiceDialog({
      title: t('settings.dataDirRestartTitle'), message: t('settings.dataDirRestartMsg'),
      choices: [{ label: t('settings.dataDirCloseNow'), value: 'close', kind: 'primary' }, { label: t('settings.dataDirLater'), value: 'later' }],
    });
    // a real exit: with "close hides to the tray" on, win_close would only hide the window and the restart would never happen
    if (choice === 'close') invoke('app_exit').catch(() => invoke('win_close').catch(() => {}));
  } catch (e) { toast(errText(e)); }
}
$('dataDirChange').addEventListener('click', async () => {
  try { const picked = await invoke('pick_data_dir'); if (picked) await switchDataDir(picked, 'settings.dataDirConfirm'); }
  catch (e) { toast(errText(e)); }
});
$('dataDirRestore').addEventListener('click', () => { if (defaultDir) switchDataDir(defaultDir, 'settings.dataDirRestoreConfirm'); });
$('dataDirOpen').addEventListener('click', () => invoke('open_data_dir').catch(() => {}));
refreshDataDir();

// ---- about / stats / clear library -------------------------------------------------------------------
T.app?.getVersion?.().then((v) => { $('ver').textContent = `v${v}`; }).catch(() => {});
lib.stats().then((s) => {
  const cell = (v, label) => `<div class="stat"><b class="num">${v}</b><span>${esc(label)}</span></div>`;
  $('statsGrid').innerHTML =
    cell(s.books, t('lib.title')) + cell(s.pages, t('settings.statPages')) +
    cell(s.folders, t('lib.folders')) + cell(s.inProgress, t('reader.history')) +
    cell(s.finished, t('end.title')) + cell(s.bookmarkedBooks, t('bookmarks.title'));
}).catch(() => {});
$('clearShelf').addEventListener('click', async () => {
  if (!await confirmDialog({ title: t('settings.clearLibrary'), message: t('settings.clearLibraryConfirm'), ok: t('common.delete'), danger: true })) return;
  try { await invoke('shelf_clear'); toast(t('common.saved')); setTimeout(() => location.reload(), 600); }
  catch (e) { toast(errText(e)); }
});
