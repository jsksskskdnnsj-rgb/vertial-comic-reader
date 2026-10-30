// reader-boot.js - wires reader.html to the two views (vertical strip: reader.js, page flip: flip.js) and to every reader feature.
// Chapters and bookmarks are a navigation/display layer: this file only reads positions from the active view and calls view.jumpTo().
import { T, invoke, win, t, esc, ago, toast, bootPage, prefs, isModalOpen, confirmDialog, choiceDialog, promptDialog } from './shell.js';
import { Reader, thumbUrl, wheelGear, scrubProgress, pageReadout, isReaderBlankTarget, applyReaderChromeClasses } from './reader.js';
import { FlipView } from './flip.js';
import { lib } from './legacy-api.js';
import { resolve, episodeAt, prevTarget, nextTarget, tickFractions, episodeLabel, sanePages, DEFAULT_PAGES, fetchEpisodeInfo } from './chapters.js';
import { findNearby, thumbVirtualIndex, bookmarkTicks, BookmarkPanel } from './bookmarks.js';
import { HistoryStack, ThumbStrip } from './reader-panels.js';
import { AutoScroll, openPageDirectory } from './reader-tools.js';
import { mountShell } from './shell-ui.js';

const $ = (id) => document.getElementById(id);
bootPage('library');
mountShell('library');

const bookId = Number(new URLSearchParams(location.search).get('book'));
const COVER_BASE = T.core?.convertFileSrc ? T.core.convertFileSrc('', 'mhcover').replace(/\/$/, '') : '';
const coverUrl = (id, v = 0) => `${COVER_BASE}/${id}?v=${v ?? 0}`;
const LARGE_BOOK_PAGES = 1200;                         // above this the large-book warning appears (once per book per session)

// ---- state -----------------------------------------------------------------------------------------------
const rp = { widthPct: prefs.get('reader.widthPct', 100), gap: prefs.get('reader.gap', 0), dim: prefs.get('reader.dim', 100), wheel: wheelGear(prefs.get('reader.wheelSpeed', 2)) };
let modeNow = prefs.get('mode', 'scroll');             // default reading mode (Settings); the toolbar toggles it for this session only
let saverNow = prefs.get('saver', false);
let view = null, book = null, bms = [], chs = [], chMode = 'folder', chN = DEFAULT_PAGES, chDefaults = { mode: 'folder', pages: DEFAULT_PAGES };
let shown = 0, lastPos = null, endDismissed = false, endArmAt = 0;

// ---- small ui helpers ------------------------------------------------------------------------------------
// Prefer an explicit page count (from onPosition) so open-time callbacks work before `book` is assigned.
const pageText = (p, n = book?.realPages ?? 0) => t('common.pageOf', { p: p + 1, n });
const epText = (p, n) => (chs.length ? episodeLabel(t, chs, p, chMode) : pageText(p, n));
const applyDim = () => $('host').style.setProperty('--dim', rp.dim / 100);
/** Thumb URL for real page `p`; empty string while book/layout are not ready yet. */
const pageThumb = (p) => {
  const i = book && view?.layout?.gFirst?.[p];
  return i == null ? '' : thumbUrl(book, i);
};

// ---- panels ----------------------------------------------------------------------------------------------
const history_ = new HistoryStack($('hist'), { t, esc, ago, coverUrl, onOpen: async (id) => { await leave(`reader.html?book=${id}`); } });
const strip = new ThumbStrip($('tstrip'), { thumb: pageThumb, onJump: (p) => view?.jumpTo({ page: p, frac: 0 }) });
const auto = new AutoScroll({ getView: () => view, onChange: onAutoChange });

async function loadHistory() {
  if (!book) return;
  const recent = (await lib.recentBooks(12).catch(() => [])).filter(b => b.id !== bookId);
  const cur = { id: bookId, title: book.title, page: shown, frac: 0, pageCount: book.realPages, coverVer: 0 };
  const eps = await fetchEpisodeInfo(invoke, [{ id: bookId, page: shown }, ...recent]).catch(() => []);
  history_.render(cur, recent, eps); history_.updateCurrent({ page: shown, realPages: book.realPages, epText: epText(shown) });
  $('histCount').textContent = recent.length ? t('reader.historyCount', { n: recent.length + 1 }) : '';
}

// ---- position -> every readout -------------------------------------------------------------------------
// Left (bbEp) and right (bbCount) share one page index. The short "8 / 2634" form
// is what both numeric spots show; the long "第 8 页 / 共 2634 页" form is the
// bottom-left title so a narrow column cannot look like a different page.
function positionLabels(page, realPages) {
  const info = pageReadout(page, realPages);
  const long = info.total ? t('common.pageOf', { p: info.page, n: info.total }) : info.text;
  const ep = info.total && chs.length ? episodeLabel(t, chs, info.page - 1, chMode) : '';
  // Bottom-left uses the same "248 / 2634" text as the right counter, then the
  // episode name. A long "第 248 页 / 共 …" string was clipping into a different page.
  return { ...info, long, crumb: ep || long, side: ep ? `${info.text} · ${ep}` : info.text };
}
function paintScrub(page, realPages, frac = 0) {
  const track = $('track');
  if (!track || track.classList.contains('is-drag')) return;
  track.style.setProperty('--p', String(scrubProgress(page, realPages, frac)));
  const tip = $('tiplabel');
  if (tip) tip.textContent = pageReadout(page, realPages).text;
}
function paintPageReadout(page, realPages, { commit = true } = {}) {
  const lab = positionLabels(page, realPages);
  $('pgCount').textContent = lab.text; $('pgCount').title = lab.long;
  $('vpText').textContent = lab.text;
  $('bbCount').textContent = lab.text; $('bbCount').title = lab.long;
  $('bbCount').dataset.page = String(lab.page); $('bbCount').dataset.total = String(lab.total);
  $('bcEp').textContent = lab.crumb;
  $('bbEp').textContent = lab.side; $('bbEp').title = lab.long;
  $('bbEp').dataset.page = String(lab.page); $('bbEp').dataset.total = String(lab.total);
  if (!commit) return lab;
  shown = lab.total ? lab.page - 1 : 0;
  history_.updateCurrent({ page: shown, realPages: lab.total, epText: lab.crumb });
  strip.update(shown, lab.total);
  return lab;
}
function onPosition(p) {
  // open() emits position before it returns; sync book from view.info if boot has not assigned it yet
  if (!book && view?.info) book = view.info;
  lastPos = p;
  paintPageReadout(p.shownPage, p.realPages);
  paintScrub(p.shownPage, p.realPages, p.shownFrac || 0);
  updateBmButton();
  if (p.atEnd && !endDismissed && performance.now() > endArmAt && $('endCard').hidden) showEnd();
}

// ---- scrubber track: a fresh element per view (drops the old Scrubber's listeners) -----------------------
function wireTrack() {
  const tr = $('track');
  tr.addEventListener('pointerdown', () => tr.classList.add('is-drag'));
  tr.addEventListener('pointermove', (e) => { const r = tr.getBoundingClientRect(); tr.style.setProperty('--tx', Math.min(Math.max(e.clientX - r.left, 64), r.width - 64) + 'px'); });
}
function resetTrack() { const old = $('track'), fresh = old.cloneNode(true); fresh.classList.remove('is-drag'); old.replaceWith(fresh); wireTrack(); trackObserver.disconnect(); trackObserver.observe(fresh); }
addEventListener('pointerup', () => $('track').classList.remove('is-drag'));
const trackObserver = new ResizeObserver(() => renderChapterUi());

// ---- opening / switching views ----------------------------------------------------------------------------
async function largeBookGuard(info) {
  if (info.realPages < LARGE_BOOK_PAGES || sessionStorage.getItem('mh.big.' + bookId)) return undefined;
  const v = await choiceDialog({ title: t('big.title'), message: t('big.msg', { n: info.realPages }),
    choices: [{ label: t('common.cancel'), value: 'cancel' }, { label: t('big.normal'), value: 'normal' }, { label: t('big.saver'), value: 'saver', kind: 'primary' }] });
  if (!v || v === 'cancel') return false;
  sessionStorage.setItem('mh.big.' + bookId, '1');
  return v === 'saver' ? { perf: 'saver' } : undefined;
}

function showOpening(on) {
  const n = $('readerLoading');
  if (n) n.hidden = !on;
}
async function startView({ startPos = null } = {}) {
  auto.stop(); $('endCard').hidden = true; $('err').hidden = true;
  showOpening(true);
  if (view) { await view.close(); view = null; }
  book = null;                                                  // drop stale info while the next open is in flight
  resetTrack(); strip.reset(); applyDim();
  const Mode = modeNow === 'flip' ? FlipView : Reader;
  view = new Mode($('host'), { onPosition, scrubber: [$('track'), $('thumb'), $('tiplabel')] });
  view.onScrubPreview = (page, n) => paintPageReadout(page, n, { commit: false });
  try {
    // Assign `book` in beforeMount (runs after openBook, before first onPosition from tick/render)
    // so pageText / ThumbStrip never see null during open.
    await view.open(bookId, {
      perf: saverNow ? 'saver' : 'balanced', widthPct: rp.widthPct, gap: rp.gap, startPos, wheelSpeed: rp.wheel,
      beforeMount: async (info) => { book = info; return largeBookGuard(info); },
    });
  } catch (e) { book = null; view = null; showOpening(false); return openFailed(e); }
  book = view.info; endDismissed = false; endArmAt = performance.now() + 1500;
  chMode = book.chapterMode; chN = sanePages(book.chapterPages); $('chrem').checked = !!book.chapterOverridden;
  $('bpCover').src = coverUrl(bookId); $('bbThumb').src = coverUrl(bookId);
  $('bpTitle').textContent = book.title; $('bcBook').textContent = book.title; $('bbBook').textContent = book.title;
  $('bpChips').innerHTML = `<span class="chip">${esc(t('book.pages', { n: book.realPages }))}</span>`;
  view.refresh(); recompute(); syncControls(); showOpening(false); await Promise.all([loadBookmarks(), loadHistory()]);
}
function openFailed(e) {
  if (e?.code === 'cancelled') return leave('library.html');
  // Surface real open failures; avoid treating boot TypeErrors as "corrupt file" when possible
  const msg = e?.detail ?? e?.message ?? String(e);
  $('errTitle').textContent = t(e?.code === 'source_missing' ? 'reader.errMissing' : 'reader.errCorrupt');
  $('errDetail').textContent = msg; $('errLocate').hidden = e?.code !== 'source_missing'; $('err').hidden = false;
}
async function leave(url) {
  auto.stop(); await view?.close();
  if (url) location.href = url; else history.length > 1 ? history.back() : (location.href = 'library.html');
}

// ---- chapters (navigation + display only) -----------------------------------------------------------------
function renderChapterUi() {
  $('epnav').hidden = chs.length === 0;
  const rail = $('track').querySelector('.scrub__rail');
  rail.querySelectorAll('.tick').forEach(n => n.remove());
  for (const f of tickFractions(chs, book?.realPages ?? 0, rail.clientWidth || 600)) { const i = document.createElement('i'); i.className = 'tick'; i.style.left = f * 100 + '%'; rail.append(i); }
  for (const f of bookmarkTicks(bms, book?.realPages ?? 0)) { const i = document.createElement('i'); i.className = 'tick tick--bm'; i.style.left = f * 100 + '%'; rail.append(i); }
  document.querySelectorAll('#chmode button').forEach(b => b.setAttribute('aria-pressed', String(b.dataset.chmode === chMode)));
  $('chnRow').hidden = chMode !== 'pages';                                   // the N-pages control exists ONLY in "pages" mode
  if (document.activeElement !== $('chn')) $('chn').value = chN;
  const none = chMode === 'folder' && (book?.folderChapters?.length ?? 0) === 0;
  $('chhint').hidden = !none; $('chhint').textContent = none ? t('chapters.noFolders') : '';
  $('chreset').hidden = !($('chrem').checked || book?.chapterOverridden);
  if (book) { const lab = positionLabels(shown, book.realPages); $('bcEp').textContent = lab.crumb; $('bbEp').textContent = lab.side; $('bbEp').title = lab.long; }
}
function recompute() { chs = resolve(chMode, chN, book); renderChapterUi(); }
let persistTimer = 0;
function persistIfRemembered() {
  if (!$('chrem').checked) return;
  clearTimeout(persistTimer);
  persistTimer = setTimeout(() => invoke('set_book_chapter_settings', { bookId, mode: chMode, pages: chN }).catch(() => {}), 400);
}
const goEpisode = (target) => { if (target) view.jumpTo(target); };
$('epPrev').addEventListener('click', () => goEpisode(prevTarget(chs, view.topPos())));
$('epNext').addEventListener('click', () => goEpisode(nextTarget(chs, view.topPos())));
$('chmode').addEventListener('click', (e) => { const b = e.target.closest('button'); if (!b) return; chMode = b.dataset.chmode; recompute(); persistIfRemembered(); });
$('chn').addEventListener('input', (e) => { if (e.target.value === '' || +e.target.value < 1) return; chN = sanePages(e.target.value); recompute(); persistIfRemembered(); });
$('chn').addEventListener('blur', () => { $('chn').value = chN; });
$('chrem').addEventListener('change', async (e) => {
  if (e.target.checked) await invoke('set_book_chapter_settings', { bookId, mode: chMode, pages: chN }).catch(() => {});
  else await invoke('clear_book_chapter_settings', { bookId }).catch(() => {});
  book.chapterOverridden = e.target.checked; renderChapterUi();
});
$('chreset').addEventListener('click', async () => {
  await invoke('clear_book_chapter_settings', { bookId }).catch(() => {});
  $('chrem').checked = false; book.chapterOverridden = false; chMode = chDefaults.mode; chN = sanePages(chDefaults.pages); recompute();
});

// ---- bookmarks (anchored to the image; broken ones keep the manual two-choice behaviour) --------------------
const sideBm = (open) => { $('drawer').hidden = !open; $('tbMarks').classList.toggle('is-on', open); };
const drawerOpen = () => !$('drawer').hidden;
function updateBmButton() { if (!book || !view) return; $('bmHere').classList.toggle('is-on', !!findNearby(bms, view.centerPos(), view.layout, view.viewH)); }
async function loadBookmarks() { bms = await invoke('list_bookmarks', { bookId }).catch(() => []); bmPanel.render(bms); renderChapterUi(); updateBmButton(); }
async function toggleBookmark() {
  if (!book || !view) return;
  const pos = view.centerPos(), near = findNearby(bms, pos, view.layout, view.viewH);
  try {
    if (near) { await invoke('delete_bookmark', { bookmarkId: near.id }); toast(t('bookmarks.removed')); }
    else { await invoke('add_bookmark', { bookId, page: pos.page, frac: pos.frac, note: null }); toast(t('bookmarks.added')); }
  } catch (e) { toast(String(e?.detail ?? e?.code ?? e)); }
  await loadBookmarks();
}
const hintPage = (bm) => Math.min(bm.pageHint, book.realPages - 1);
const bmPanel = new BookmarkPanel($('bmList'), {
  t, esc,
  thumb: (bm) => thumbUrl(book, thumbVirtualIndex(view.layout, bm)),
  label: (bm) => [chs.length ? episodeLabel(t, chs, bm.page, chMode) : '', t('bookmarks.pageOf', { p: bm.page + 1, n: book.realPages })].filter(Boolean).join(' · '),
  onJump: (bm) => view.jumpTo({ page: bm.page, frac: bm.frac }, { atCenter: true }),
  onJumpHint: (bm) => view.jumpTo({ page: hintPage(bm), frac: 0 }),                                  // explicit user choice
  onReanchor: async (bm) => { await invoke('reanchor_bookmark', { bookmarkId: bm.id, page: hintPage(bm) }).catch(() => {}); loadBookmarks(); },
  onDelete: async (id) => { await invoke('delete_bookmark', { bookmarkId: id }).catch(() => {}); loadBookmarks(); },
  onNote: async (id, note) => { await invoke('update_bookmark_note', { bookmarkId: id, note }).catch(() => {}); loadBookmarks(); },
});
$('bmHere').addEventListener('click', toggleBookmark);
$('tbMarks').addEventListener('click', () => sideBm(!drawerOpen()));
$('drawerClose').addEventListener('click', () => sideBm(false));

// ---- toolbar, flyouts ---------------------------------------------------------------------------------------
const FLY = { flySettings: 'tbSettings', flyAuto: 'tbAuto' };
function closeFlies(except) {
  for (const [id, btn] of Object.entries(FLY)) { if (id === except) continue; $(id).hidden = true; $(btn).classList.toggle('is-on', id === 'flyAuto' && auto.running); }
}
function toggleFly(id) {
  const open = $(id).hidden; closeFlies(open ? id : null);
  $(id).hidden = !open; $(FLY[id]).classList.toggle('is-on', open || (id === 'flyAuto' && auto.running));
  if (open && id === 'flyAuto') { const rg = $('rg').getBoundingClientRect(), b = $('tbAuto').getBoundingClientRect(); $('flyAuto').style.top = Math.max(8, b.top - rg.top - 40) + 'px'; }
}
$('tbSettings').addEventListener('click', () => toggleFly('flySettings'));
$('tbAuto').addEventListener('click', () => toggleFly('flyAuto'));
let chromeSuppressClick = false;
addEventListener('pointerdown', (e) => {
  const open = !$('flySettings').hidden || !$('flyAuto').hidden;
  const inside = !!e.target.closest('.flyout, #tb');
  if (!inside) closeFlies(null);
  chromeSuppressClick = open && !inside;
});

function syncControls() {
  $('w').value = rp.widthPct; $('dim').value = rp.dim; $('saver').checked = saverNow;
  $('wheel').value = rp.wheel; $('wheelVal').textContent = rp.wheel;
  document.querySelectorAll('#gap button').forEach(b => b.setAttribute('aria-pressed', String(+b.dataset.gap === rp.gap)));
  document.querySelectorAll('#fMode button').forEach(b => b.setAttribute('aria-pressed', String(b.dataset.mode === modeNow)));
  document.querySelectorAll('#flySettings [data-for="scroll"]').forEach(r => { r.hidden = modeNow !== 'scroll'; });
  $('tbModeIcon').innerHTML = `<use href="icons.svg#${modeNow === 'flip' ? 'flip' : 'scroll'}"/>`; $('tbModeText').textContent = t(modeNow === 'flip' ? 'reader.modeFlipShort' : 'reader.modeScrollShort');
  $('autoSpeed').value = auto.speed; $('autoVal').textContent = auto.speed;
}
$('w').addEventListener('input', (e) => { rp.widthPct = +e.target.value; prefs.set('reader.widthPct', rp.widthPct); if (view?.mode === 'scroll') view.setWidthPct(rp.widthPct); });
$('wheel').addEventListener('input', (e) => { rp.wheel = wheelGear(+e.target.value); prefs.set('reader.wheelSpeed', rp.wheel); $('wheelVal').textContent = rp.wheel; if (view?.mode === 'scroll') view.setWheelSpeed(rp.wheel); });
$('dim').addEventListener('input', (e) => { rp.dim = +e.target.value; prefs.set('reader.dim', rp.dim); applyDim(); });
$('gap').addEventListener('click', (e) => { const b = e.target.closest('button'); if (!b) return; rp.gap = +b.dataset.gap; prefs.set('reader.gap', rp.gap); if (view?.mode === 'scroll') view.setGap(rp.gap); syncControls(); });
$('saver').addEventListener('change', async (e) => { saverNow = e.target.checked; prefs.set('saver', saverNow); await startView({ startPos: view?.topPos() }); });
const setMode = async (m) => { if (m === modeNow) return; modeNow = m; await startView({ startPos: view?.topPos() }); };
$('fMode').addEventListener('click', (e) => { const b = e.target.closest('button'); if (b) setMode(b.dataset.mode); });
$('tbMode').addEventListener('click', () => setMode(modeNow === 'scroll' ? 'flip' : 'scroll'));
$('tbFit').addEventListener('click', () => { view?.setFit(); if (view?.mode === 'scroll') { rp.widthPct = 100; $('w').value = 100; prefs.set('reader.widthPct', 100); } });

// ---- auto-scroll -----------------------------------------------------------------------------------------------
function onAutoChange(running) {
  $('autoBadge').hidden = !running; $('autoText').textContent = t('reader.autoOn', { n: auto.speed });
  $('tbAuto').querySelector('use').setAttribute('href', `icons.svg#${running ? 'pause' : 'play'}`); $('tbAuto').classList.toggle('is-on', running);
  $('autoToggle').querySelector('use').setAttribute('href', `icons.svg#${running ? 'pause' : 'play'}`); $('autoToggleText').textContent = t(running ? 'reader.autoStop' : 'reader.autoStart');
}
auto.setSpeed(prefs.get('autoSpeed', 4)); onAutoChange(false);
$('autoSpeed').addEventListener('input', (e) => { auto.setSpeed(+e.target.value); prefs.set('autoSpeed', auto.speed); $('autoVal').textContent = auto.speed; $('autoText').textContent = t('reader.autoOn', { n: auto.speed }); });
$('autoToggle').addEventListener('click', () => auto.toggle()); $('autoStop').addEventListener('click', () => auto.stop());
$('host').addEventListener('wheel', () => { auto.stop(); }, { passive: true });

// Sticky chrome. v0.7.7–v0.7.8 hid the side bars from scroll alone, so a wheel
// notch or a Space hold flashed them. They now stay until the reader toggles:
// the floating button, the T key, or a click on blank page margin. Comic pages,
// the scrubber, and the other controls keep their own clicks. .no-bp (c / list)
// still only collapses history.
let chromeHidden = false;
function syncChromeToggle() {
  const btn = $('chromeToggle');
  if (!btn) return;
  const key = chromeHidden ? 'reader.chromeShow' : 'reader.chromeHide';
  btn.setAttribute('aria-pressed', chromeHidden ? 'true' : 'false');
  btn.setAttribute('aria-label', t(key));
  btn.title = t(key);
  const label = $('chromeToggleText');
  if (label) label.textContent = t(key);
}
function applyReaderChrome(hidden, { persist = true } = {}) {
  chromeHidden = applyReaderChromeClasses(document, hidden);
  if (chromeHidden) closeFlies(null);
  syncChromeToggle();
  if (persist) prefs.set('reader.chromeHidden', chromeHidden);
}
function toggleReaderChrome() { applyReaderChrome(!chromeHidden); }
$('chromeToggle')?.addEventListener('click', (e) => {
  e.stopPropagation();
  chromeSuppressClick = false;
  toggleReaderChrome();
});
$('vp').addEventListener('click', (e) => {
  if (chromeSuppressClick) { chromeSuppressClick = false; return; }
  if (e.button !== 0 || e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return;
  if (isModalOpen() || helpOpen()) return;
  if (!isReaderBlankTarget(e.target)) return;
  const sc = e.target.classList?.contains('scroller') ? e.target : null;
  if (sc) {
    const box = sc.getBoundingClientRect();
    if (e.clientX >= box.left + sc.clientWidth || e.clientY >= box.top + sc.clientHeight) return;
  }
  toggleReaderChrome();
});

// ---- page directory, prev/next page, jump ----------------------------------------------------------------------
$('tbDir').addEventListener('click', () => {
  closeFlies(null);
  if (!book || !view) return;
  openPageDirectory({ bookId, realPages: book.realPages, current: shown, title: book.title, thumb: pageThumb, lib,
    onJump: (p) => view.jumpTo({ page: p, frac: 0 }), onChanged: () => startView({ startPos: view.topPos() }) });
});
$('pgPrev').addEventListener('click', () => view?.stepPage(-1)); $('pgNext').addEventListener('click', () => view?.stepPage(1));
$('pgCount').addEventListener('click', async () => {
  if (!book || !view) return;
  const v = await promptDialog({ title: t('reader.jump'), label: t('reader.jumpLabel', { n: book.realPages }), value: String(shown + 1), ok: t('common.go') });
  const n = Number(v); if (v && n >= 1) view.jumpTo({ page: Math.min(n, book.realPages) - 1, frac: 0 });
});

// ---- history panel visibility ----------------------------------------------------------------------------------
const narrow = matchMedia('(max-width:1180px)');
$('bpToggle').addEventListener('click', () => { if (narrow.matches) $('rg').classList.toggle('show-bp'); else { $('rg').classList.toggle('no-bp'); prefs.set('reader.noHistory', $('rg').classList.contains('no-bp')); } });
if (prefs.get('reader.noHistory', false)) $('rg').classList.add('no-bp');

// ---- end-of-book card ---------------------------------------------------------------------------------------------
function showEnd() { auto.stop(); $('endCover').src = coverUrl(bookId); $('endMsg').textContent = t('end.msg', { title: book.title }); $('endCard').hidden = false; }
$('endKeep').addEventListener('click', () => { endDismissed = true; $('endCard').hidden = true; });
$('endAgain').addEventListener('click', () => { $('endCard').hidden = true; endDismissed = false; endArmAt = performance.now() + 1500; view.jumpTo({ page: 0, frac: 0 }); });
$('endLib').addEventListener('click', () => leave('library.html'));

// ---- navigation, fullscreen, window -------------------------------------------------------------------------------
$('back').addEventListener('click', () => leave()); $('errBack').addEventListener('click', () => leave()); $('errRetry').addEventListener('click', () => startView());
$('errLocate').addEventListener('click', async () => {
  const p = await T.dialog.open({ multiple: false, filters: [{ name: t('imp.zipFilter'), extensions: ['zip', 'cbz'] }] });
  if (p) { await lib.relinkBook(bookId, p).catch(() => {}); startView(); }
});
const toggleFs = async () => win.setFullscreen(!(await win.isFullscreen()));
$('fsBtn').addEventListener('click', toggleFs);
const HELP_ROWS = [
  ['Space  ·  PgDn', 'help.space'],
  ['Shift+Space  ·  PgUp', 'help.pageUp'],
  ['↑  ↓', 'help.nudge'],
  ['←  →', 'help.page'],
  ['Home  ·  End', 'help.ends'],
  ['Ctrl  +  −  0', 'help.zoom'],
  ['B', 'help.bookmark'],
  ['M', 'help.mode'],
  ['A', 'help.auto'],
  ['D', 'help.dir'],
  ['C', 'help.history'],
  ['T', 'help.chrome'],
  ['G', 'help.jump'],
  ['F  ·  F11', 'help.full'],
  ['[  ·  ]', 'help.episode'],
  ['Esc', 'help.leave'],
  ['?  ·  H', 'help.toggle'],
];
const helpOpen = () => !$('helpOverlay').hidden;
function renderHelp() {
  $('helpGrid').innerHTML = HELP_ROWS.map(([keys, key]) =>
    `<div class="help-row"><kbd>${esc(keys)}</kbd><span>${esc(t(key))}</span></div>`).join('');
}
function setHelp(open) {
  $('helpOverlay').hidden = !open;
  $('helpBtn')?.classList.toggle('is-on', open);
  if (open) {
    renderHelp();
    document.documentElement.dataset.modal = '1';
    const card = $('helpOverlay')?.querySelector('.help-card');
    if (card && !card.hasAttribute('tabindex')) card.tabIndex = -1;
    card?.focus({ preventScroll: true });
  } else if (!document.querySelector('.modal-backdrop')) {
    delete document.documentElement.dataset.modal;
  }
}
$('helpBtn').addEventListener('click', () => setHelp(!helpOpen()));
$('helpClose').addEventListener('click', () => setHelp(false));
$('helpOverlay').addEventListener('click', (e) => { if (e.target === $('helpOverlay')) setHelp(false); });
addEventListener('keydown', (e) => {
  if (isModalOpen() || e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
  const k = e.key.toLowerCase();
  if (k === 'escape') {
    if (helpOpen()) { setHelp(false); e.preventDefault(); return; }
    if (!$('flySettings').hidden || !$('flyAuto').hidden) closeFlies(null); else if (drawerOpen()) sideBm(false); else if (!$('endCard').hidden) $('endKeep').click();
    else win.isFullscreen().then(f => (f ? win.setFullscreen(false) : leave())); return;
  }
  if (e.key === '?' || k === 'h') {
    if (e.repeat || e.ctrlKey || e.metaKey || e.altKey) return;
    setHelp(!helpOpen()); e.preventDefault(); return;
  }
  if (helpOpen() || e.ctrlKey || e.metaKey || e.altKey) return;
  if (k === 't') { if (e.repeat) return; toggleReaderChrome(); e.preventDefault(); return; }
  if (!view) return;
  if (k === 'b') toggleBookmark(); else if (k === 'm') $('tbMode').click(); else if (k === 'a') auto.toggle(); else if (k === 'd') $('tbDir').click(); else if (k === 'c') $('bpToggle').click();
  else if (k === 'g') $('pgCount').click(); else if (k === 'f' || k === 'f11') toggleFs(); else if (k === '[') goEpisode(prevTarget(chs, view.topPos())); else if (k === ']') goEpisode(nextTarget(chs, view.topPos())); else return;
  e.preventDefault();
});
win.onCloseRequested(async (e) => {
  e.preventDefault(); auto.stop();
  // close-to-tray: hide the window and keep the book open (progress is flushed when the page becomes hidden); otherwise quit as before
  if (await invoke('win_hide_if_tray').catch(() => false)) return;
  await view?.close(); await win.destroy();
});

// ---- go ---------------------------------------------------------------------------------------------------------------
wireTrack(); trackObserver.observe($('track'));
applyReaderChrome(!!prefs.get('reader.chromeHidden', false), { persist: false });
invoke('get_chapter_defaults').then(d => { chDefaults = d; }).catch(() => {});
startView();
if (new URLSearchParams(location.search).get('help') === '1') setHelp(true);
