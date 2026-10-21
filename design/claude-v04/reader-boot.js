// reader-boot.js - glue between reader.html and reader.js / layout.js / chapters.js.
// Chapters are a NAVIGATION/DISPLAY layer: this file only reads the strip's position and calls
// reader.jumpTo(); it never touches layout, slicing or the image window.
import { Reader, thumbUrl } from './reader.js';
import { T, invoke, win, t, esc, toast, applyI18n, applyTheme, wireWindow } from './shell.js';
import { centerPosition, findNearby, thumbVirtualIndex, bookmarkTicks, BookmarkPanel } from './bookmarks.js';
import { resolve, episodeAt, prevTarget, nextTarget, tickFractions, episodeLabel, sanePages, DEFAULT_PAGES } from './chapters.js';

const $ = (id) => document.getElementById(id);
const root = $('reader');
const bookId = Number(new URLSearchParams(location.search).get('book'));

// ---- reader settings (move into your settings table; localStorage keeps the files standalone) ----
const DEF = { widthPct: 100, gap: 0, bg: 'lavender', dim: 100, saver: false };
const cfg = { ...DEF, ...JSON.parse(localStorage.getItem('mh.reader') || '{}') };
const save = () => localStorage.setItem('mh.reader', JSON.stringify(cfg));

applyTheme(); applyI18n(); wireWindow();
root.dataset.bg = cfg.bg;
root.style.setProperty('--dim', cfg.dim / 100);

// ---- chapter state -------------------------------------------------------------------------------
let book = null;                                     // reader.info
let chDefaults = { mode: 'folder', pages: DEFAULT_PAGES };
let chMode = 'folder', chN = DEFAULT_PAGES;          // working values for THIS session
let chs = [];                                        // effective episodes for (chMode, chN)
let shown = 0;                                       // page at viewport middle (for labels)

const curPos = () => reader.strip.layout.getPosition(reader.strip.scroller.scrollTop);   // viewport TOP, same as what is saved

function renderChapterUi() {
  $('epnav').hidden = chs.length === 0;                                          // None / no chapters => no chapter UI at all
  $('chapter').textContent = episodeLabel(t, chs, shown, chMode);
  const rail = $('track').querySelector('.scrub__rail');
  rail.querySelectorAll('.tick').forEach(n => n.remove());
  const fr = tickFractions(chs, book?.realPages ?? 0, rail.clientWidth || 600);
  for (const f of fr) { const i = document.createElement('i'); i.className = 'tick'; i.style.left = f * 100 + '%'; rail.append(i); }
  for (const f of bookmarkTicks(bms, book?.realPages ?? 0)) { const i = document.createElement('i'); i.className = 'tick tick--bm'; i.style.left = f * 100 + '%'; rail.append(i); }
  // popover controls
  document.querySelectorAll('#chmode button').forEach(b => b.setAttribute('aria-pressed', String(b.dataset.chmode === chMode)));
  $('chnRow').hidden = chMode !== 'pages';
  if (document.activeElement !== $('chn')) $('chn').value = chN;
  const noFolders = chMode === 'folder' && (book?.folderChapters?.length ?? 0) === 0;
  $('chhint').hidden = !noFolders; $('chhint').textContent = noFolders ? t('noFolders') : '';
  $('chreset').hidden = !($('chrem').checked || book?.chapterOverridden);
}
function recompute() {                               // instant: no invoke, no rescan, no relayout
  chs = resolve(chMode, chN, book);
  renderChapterUi();
}
let persistTimer = 0;
function persistIfRemembered() {
  if (!$('chrem').checked) return;                   // unchecked => this session only
  clearTimeout(persistTimer);
  persistTimer = setTimeout(() => invoke('set_book_chapter_settings', { bookId, mode: chMode, pages: chN }).catch(() => {}), 400);
}
const goEpisode = (target) => { if (target) reader.jumpTo(target); };           // same strip, same jump as the scrubber

// ---- reader --------------------------------------------------------------------------------------
const reader = new Reader($('host'), {
  onPosition: (p) => {
    shown = p.shownPage;
    $('cnt').textContent = `${p.shownPage + 1} / ${p.realPages}`;
    $('mini').style.setProperty('--p', (p.shownPage + 1) / p.realPages);
    $('chapter').textContent = episodeLabel(t, chs, shown, chMode);
    updateBmButton();
  },
  scrubber: [$('track'), $('thumb'), $('tiplabel')],          // Scrubber(track, thumbImg, labelEl)
});

async function start() {
  $('err').hidden = true;
  try {
    chDefaults = await invoke('get_chapter_defaults').catch(() => chDefaults);
    await reader.open(bookId, { perf: cfg.saver ? 'saver' : 'balanced', widthPct: cfg.widthPct, gap: cfg.gap });
    book = reader.info;
    $('title').textContent = book.title;
    chMode = book.chapterMode; chN = sanePages(book.chapterPages);      // effective values from the backend
    $('chrem').checked = !!book.chapterOverridden;
    reader.tracker.tick();                                     // fill counter immediately
    recompute();
    await loadBookmarks();                                     // resolved BY NAME against the current page list
    poke();                                                    // the resume scroll would have hidden the bars
    new ResizeObserver(() => renderChapterUi()).observe($('track'));    // tick thinning depends on width
  } catch (e) {                                                // e = ReaderError {code, detail}
    $('errTitle').textContent = e?.code === 'source_missing' ? t('errMissing') : t('errCorrupt');
    $('errDetail').textContent = e?.detail ?? String(e);
    $('errLocate').hidden = e?.code !== 'source_missing';
    $('err').hidden = false;
  }
}

// ---- overlay: show on pointer move, hide after 2.5 s idle / on scroll; cursor hides after 2 s ----
let hideTimer = 0, cursorTimer = 0;
const ui = (show) => root.classList.toggle('hide-ui', !show);
const popOpen = () => !$('pop').hidden;
function poke() {
  ui(true); root.classList.remove('idle');
  clearTimeout(hideTimer); clearTimeout(cursorTimer);
  hideTimer = setTimeout(() => { if (!popOpen() && !sideOpen() && !$('cnt').querySelector('input') && !$('track').classList.contains('is-drag')) ui(false); }, 2500);
  cursorTimer = setTimeout(() => root.classList.add('idle'), 2000);
}
addEventListener('pointermove', (e) => { if (Math.abs(e.movementX) + Math.abs(e.movementY) > 0) poke(); });
$('host').addEventListener('scroll', () => { if (!popOpen() && !sideOpen()) ui(false); }, { capture: true, passive: true });
$('host').addEventListener('click', () => {
  if (popOpen()) return setPop(false);
  root.classList.contains('hide-ui') ? poke() : ui(false);
});
const track = $('track');
track.addEventListener('pointerdown', () => track.classList.add('is-drag'));
addEventListener('pointerup', () => track.classList.remove('is-drag'));
track.addEventListener('pointermove', (e) => { const r = track.getBoundingClientRect(); track.style.setProperty('--tx', Math.min(Math.max(e.clientX - r.left, 64), r.width - 64) + 'px'); });

// ---- jump: click the counter, type a page, Enter ---------------------------------------------------
$('cnt').addEventListener('click', () => {
  const c = $('cnt'); if (c.querySelector('input')) return;
  const old = c.textContent, input = Object.assign(document.createElement('input'), { type: 'number', min: 1, max: book.realPages, placeholder: old.split('/')[0].trim() });
  c.textContent = ''; c.append(input); input.focus();
  const done = (go) => { const n = +input.value; c.textContent = old; if (go && n >= 1) reader.jumpTo({ page: Math.min(n, book.realPages) - 1, frac: 0 }); };
  input.addEventListener('keydown', (e) => { if (e.key === 'Enter') done(true); if (e.key === 'Escape') { e.stopPropagation(); done(false); } });
  input.addEventListener('blur', () => done(false), { once: true });
});

// ---- episode navigation ------------------------------------------------------------------------------
$('epPrev').addEventListener('click', () => goEpisode(prevTarget(chs, curPos())));
$('epNext').addEventListener('click', () => goEpisode(nextTarget(chs, curPos())));

// ---- quick settings popover ---------------------------------------------------------------------------
function setPop(open) { $('pop').hidden = !open; $('gear').setAttribute('aria-expanded', open); if (open) ui(true); }
$('gear').addEventListener('click', (e) => { e.stopPropagation(); setPop(!popOpen()); });
$('pop').addEventListener('click', (e) => e.stopPropagation());

const pressed = (sel, attr, val) => document.querySelectorAll(sel).forEach(b => b.setAttribute('aria-pressed', String(b.dataset[attr] == val)));
$('w').value = cfg.widthPct; $('dim').value = cfg.dim; $('saver').checked = cfg.saver; pressed('#gap button', 'gap', cfg.gap); pressed('#bgsw button', 'bg', cfg.bg);
$('w').addEventListener('input',  (e) => { cfg.widthPct = +e.target.value; reader.setWidthPct(cfg.widthPct); save(); });
$('dim').addEventListener('input', (e) => { cfg.dim = +e.target.value; root.style.setProperty('--dim', cfg.dim / 100); save(); });
$('gap').addEventListener('click',  (e) => { const b = e.target.closest('button'); if (!b) return; cfg.gap = +b.dataset.gap; reader.setGap(cfg.gap); pressed('#gap button', 'gap', cfg.gap); save(); });
$('bgsw').addEventListener('click', (e) => { const b = e.target.closest('button'); if (!b) return; cfg.bg = b.dataset.bg; root.dataset.bg = cfg.bg; pressed('#bgsw button', 'bg', cfg.bg); save(); });
$('saver').addEventListener('change', async (e) => { cfg.saver = e.target.checked; save(); await reader.close(); location.reload(); });

// chapter controls: mode / N recompute instantly; persisted only when "Remember for this book" is on
$('chmode').addEventListener('click', (e) => {
  const b = e.target.closest('button'); if (!b) return;
  chMode = b.dataset.chmode; recompute(); persistIfRemembered();
});
$('chn').addEventListener('input', (e) => {
  if (e.target.value === '' || +e.target.value < 1) return;       // wait for a valid number while typing
  chN = sanePages(e.target.value); recompute(); persistIfRemembered();
});
$('chn').addEventListener('blur', () => { $('chn').value = chN; });
$('chrem').addEventListener('change', async (e) => {
  if (e.target.checked) await invoke('set_book_chapter_settings', { bookId, mode: chMode, pages: chN }).catch(() => {});
  else await invoke('clear_book_chapter_settings', { bookId }).catch(() => {});   // back to the global default for future opens
  book.chapterOverridden = e.target.checked; renderChapterUi();
});
$('chreset').addEventListener('click', async () => {
  await invoke('clear_book_chapter_settings', { bookId }).catch(() => {});
  $('chrem').checked = false; book.chapterOverridden = false;
  chMode = chDefaults.mode; chN = sanePages(chDefaults.pages); recompute();
});

// ---- navigation / window --------------------------------------------------------------------------------
// ---- bookmarks: anchored to the IMAGE (entry name), listed in the side panel ----------------------------
let bms = [];                                                      // as returned by list_bookmarks (resolved by name)
const sideOpen = () => !$('side').hidden;
function setSide(open) { $('side').hidden = !open; $('bmPanel').setAttribute('aria-expanded', open); if (open) ui(true); }
$('bmPanel').addEventListener('click', (e) => { e.stopPropagation(); setSide(!sideOpen()); });
$('sideClose').addEventListener('click', () => setSide(false));
$('side').addEventListener('click', (e) => e.stopPropagation());

const centerNow = () => { const sc = reader.strip.scroller; return { sc, pos: centerPosition(reader.strip.layout, sc.scrollTop, sc.clientHeight) }; };
function updateBmButton() {                                        // filled when a bookmark is within half a screen of the centre
  if (!book) return;
  const { sc, pos } = centerNow();
  $('bmAdd').classList.toggle('is-on', !!findNearby(bms, pos, reader.strip.layout, sc.clientHeight));
}
async function loadBookmarks() {
  bms = await invoke('list_bookmarks', { bookId }).catch(() => []);
  panel.render(bms); renderChapterUi(); updateBmButton();         // renderChapterUi redraws the scrubber ticks (chapters + bookmarks)
}
async function toggleBookmark() {                                  // B or the button: bookmark the image at the viewport CENTER, or remove the one there
  if (!book) return;
  const { sc, pos } = centerNow();                                 // pos = { page: REAL page, frac: offset within the whole image }
  const near = findNearby(bms, pos, reader.strip.layout, sc.clientHeight);
  try {
    if (near) { await invoke('delete_bookmark', { bookmarkId: near.id }); toast(t('bmRemoved')); }
    else { await invoke('add_bookmark', { bookId, page: pos.page, frac: pos.frac, note: null }); toast(t('bmAdded')); }
  } catch (e) { toast(String(e?.detail ?? e?.code ?? e)); }
  await loadBookmarks();
}
$('bmAdd').addEventListener('click', (e) => { e.stopPropagation(); toggleBookmark(); });

const hintPage = (bm) => Math.min(bm.pageHint, book.realPages - 1);
const panel = new BookmarkPanel($('bmList'), {
  t, esc,
  thumb: (bm) => thumbUrl(book, thumbVirtualIndex(reader.strip.layout, bm)),     // the slice that contains the bookmarked spot
  label: (bm) => [episodeLabel(t, chs, bm.page, chMode), t('pageOfN', { p: bm.page + 1, n: book.realPages })].filter(Boolean).join(' · '),
  onJump: (bm) => reader.jumpTo({ page: bm.page, frac: bm.frac }, { atCenter: true }),   // exact image + offset, same strip
  onJumpHint: (bm) => reader.jumpTo({ page: hintPage(bm), frac: 0 }),                  // broken bookmark, EXPLICIT user choice
  onReanchor: async (bm) => { await invoke('reanchor_bookmark', { bookmarkId: bm.id, page: hintPage(bm) }).catch(() => {}); loadBookmarks(); },
  onDelete: async (id) => { await invoke('delete_bookmark', { bookmarkId: id }).catch(() => {}); loadBookmarks(); },
  onNote: async (id, note) => { await invoke('update_bookmark_note', { bookmarkId: id, note }).catch(() => {}); loadBookmarks(); },
});

async function leave() { await reader.close(); history.length > 1 ? history.back() : (location.href = 'home.html'); }
$('back').addEventListener('click', leave); $('errBack').addEventListener('click', leave); $('errRetry').addEventListener('click', start);
$('errLocate').addEventListener('click', async () => {        // assumed command: relink_book(bookId, path) -> () ; re-points books.path and clears scan_sig
  const p = await T.dialog.open({ multiple: false, filters: [{ name: 'Comics', extensions: ['zip', 'cbz'] }] });
  if (p) { await invoke('relink_book', { bookId, path: p }); start(); }
});
const toggleFs = async () => win.setFullscreen(!(await win.isFullscreen()));
$('fs').addEventListener('click', toggleFs);
addEventListener('keydown', (e) => {
  if (e.target instanceof HTMLInputElement && e.key !== 'Escape') return;
  if (e.key === 'Escape') { popOpen() ? setPop(false) : sideOpen() ? setSide(false) : win.isFullscreen().then(f => f ? win.setFullscreen(false) : leave()); }
  else if (e.key === 'F11' || e.key === 'f') { e.preventDefault(); toggleFs(); }
  else if (e.key === 'g' || (e.ctrlKey && e.key === 'g')) { e.preventDefault(); ui(true); $('cnt').click(); }
  else if (e.key === 'b' || e.key === 'B') { e.preventDefault(); toggleBookmark(); }
  else if (e.key === 't' || e.key === 'T') { e.preventDefault(); setSide(!sideOpen()); }
  else if (e.key === '[') { e.preventDefault(); goEpisode(prevTarget(chs, curPos())); }    // [ / ] = previous / next episode (no-ops without chapters)
  else if (e.key === ']') { e.preventDefault(); goEpisode(nextTarget(chs, curPos())); }
});
win.onCloseRequested(async (e) => { e.preventDefault(); await reader.close(); await win.destroy(); });

start(); poke();
