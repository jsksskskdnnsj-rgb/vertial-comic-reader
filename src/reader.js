// reader.js - vertical-scroll reader core for MH Reader (ES module, no bundler needed).
// Requires tauri.conf.json: app.withGlobalTauri = true   (so window.__TAURI__ exists)
//
// Objects:  Strip (DOM + layout)   ViewportTracker (scroll -> position/direction/speed)
//           ImageWindow (IO-driven mount/unmount)   Prefetcher (-> Rust set_reading_window)
//           ProgressSaver (debounced save)   Scrubber (bottom slider)   Reader (glue)
import { Layout } from './layout.js';

const T = globalThis.window?.__TAURI__;
const invoke = (cmd, args) => (T?.core?.invoke ? T.core.invoke(cmd, args) : Promise.reject(new Error('no backend')));
// Windows/WebView2: "http://mhpage.localhost"   (macOS/Linux would be "mhpage://localhost")
const PAGE_BASE = T?.core?.convertFileSrc ? T.core.convertFileSrc('', 'mhpage').replace(/\/$/, '') : '';
/** 160px-wide preview of virtual page `i` (a slice for tall images). Used by the bookmarks panel; same URL shape as the scrubber. */
export const thumbUrl = (info, i) => `${PAGE_BASE}/${info.bookId}/${i}?v=160&s=${info.sig}`;
/** URL of virtual page `i` at a given width variant (0 = full size). Shared with flip.js. */
export const pageUrl = (info, i, variant = 0) => `${PAGE_BASE}/${info.bookId}/${i}?v=${variant}&s=${info.sig}`;

export const api = {
  openBook:      (bookId) => invoke('open_book', { bookId }),
  rescanBook:    (bookId) => invoke('rescan_book', { bookId }),
  closeBook:     (bookId) => invoke('close_book', { bookId }),
  setWindow:     (a) => invoke('set_reading_window', a),
  saveProgress:  (bookId, pos, reachedEnd) => invoke('save_progress', { bookId, pos, reachedEnd }),
  fixDims:       (bookId, fixes) => invoke('fix_page_dims', { bookId, fixes }),
  cacheStats:    () => invoke('cache_stats'),
  cacheSetLimit: (bytes) => invoke('cache_set_limit', { bytes }),
  cacheClear:    (bookId = null) => invoke('cache_clear', { bookId }),
};

// Margins are IntersectionObserver rootMargins. A page mounts inside `near` and stays
// mounted until it leaves `far`, so a short scroll back does not drop the bitmap.
// v0.7.6 closed the mount gate above ~3 viewport-heights/sec and waited 90ms of calm,
// so a fast scroll showed empty boxes on the first open and on every re-read, ZIP or
// folder. Mounting now tracks the viewport the whole time. `warm` is how many
// not-yet-mounted pages in the reading direction get a browser fetch started early.
// Saver still uses the width-variant decode path; balanced non-slice pages do not.
export const PERF = {
  balanced: { nearVh: 2, farVh: 4.2, maxMounted: 18,
              aheadVh: 7, behindVh: 2, maxAhead: 14, maxBehind: 4, variant: 0, warm: 4 },
  saver:    { nearVh: 1, farVh: 2.2, maxMounted: 8,
              aheadVh: 3.5, behindVh: 0.5, maxAhead: 6, maxBehind: 1, variant: 1600, warm: 2 },
};
const rootMargin = (vh) => `${Math.round(vh * 100)}% 0px`;
// Virtual pages overlapping the viewport expanded by `nearVh` screens.
// Used on every scroll frame so mounting does not wait for IntersectionObserver.
export function virtualsNear(layout, scrollTop, viewH, nearVh) {
  if (!layout.n || viewH <= 0) return [];
  const i0 = layout.findVirtual(Math.max(0, scrollTop - viewH * nearVh));
  const i1 = layout.findVirtual(scrollTop + viewH * (1 + nearVh));
  const out = [];
  for (let i = i0; i <= i1 && i < layout.n; i++) out.push(i);
  return out;
}
const CFG = {
  prefetchMinGapMs: 40,        // trailing send still lets the final position win
  saveDebounceMs: 1500, saveMaxWaitMs: 4000,
  endPad: 360,                 // room under the last page for the end card
};
export const WHEEL_GEAR_MIN = 1;
export const WHEEL_GEAR_MAX = 5;
export const WHEEL_GEAR_DEFAULT = 2;   // one step above native; multiplier 2
const WHEEL_MUL = [1, 2, 3, 4, 6];     // gear 1 keeps the browser's own wheel distance
export function wheelGear(v) {
  const n = Math.round(Number(v));
  if (!Number.isFinite(n)) return WHEEL_GEAR_DEFAULT;
  return Math.min(WHEEL_GEAR_MAX, Math.max(WHEEL_GEAR_MIN, n));
}
export function wheelMultiplier(gear) { return WHEEL_MUL[wheelGear(gear) - 1]; }
// Gear 1 returns null: the caller must leave that notch to the browser.
// `page` is {w,h} in px, used only for deltaMode page (2).
export function wheelScrollDelta(e, gear, page) {
  const mul = wheelMultiplier(gear);
  if (mul === 1) return null;
  let dx = e.deltaX || 0, dy = e.deltaY || 0;
  if (e.deltaMode === 1) { dx *= 32; dy *= 32; }
  else if (e.deltaMode === 2) { dx *= page?.w || 0; dy *= page?.h || 0; }
  if (!dx && !dy) return null;
  return { dx: dx * mul, dy: dy * mul };
}
// Space / PageDown used to call scrollBy({ behavior:'smooth' }) on every keydown,
// including OS key-repeat. A tap was one CSS smooth jump; a hold stacked many more
// at a different cadence. One rAF velocity covers both: tap travels minTravel at
// that speed, hold keeps the same px/s until keyup.
export const KEY_PAGE_PX_PER_SEC = 1800;
export const KEY_NUDGE_PX_PER_SEC = 560;
export const KEY_PAGE_TRAVEL_VH = 0.9;
export const KEY_NUDGE_MIN_PX = 72;
export function keyScrollPlan(key, { shift = false, viewH = 800 } = {}) {
  const pageTravel = Math.max(1, viewH * KEY_PAGE_TRAVEL_VH);
  if (key === 'ArrowDown') return { dir: 1, pxPerSec: KEY_NUDGE_PX_PER_SEC, minTravel: KEY_NUDGE_MIN_PX, key };
  if (key === 'ArrowUp') return { dir: -1, pxPerSec: KEY_NUDGE_PX_PER_SEC, minTravel: KEY_NUDGE_MIN_PX, key };
  if (key === 'PageUp') return { dir: -1, pxPerSec: KEY_PAGE_PX_PER_SEC, minTravel: pageTravel, key };
  if (key === 'PageDown') return { dir: 1, pxPerSec: KEY_PAGE_PX_PER_SEC, minTravel: pageTravel, key };
  if (key === ' ') return { dir: shift ? -1 : 1, pxPerSec: KEY_PAGE_PX_PER_SEC, minTravel: pageTravel, key };
  return null;
}
const clamp = (v, a, b) => Math.min(b, Math.max(a, v));

// --p on the scrub track is a 0–1 fraction. CSS turns it into a length with
// calc(var(--p) * 100%). A bare number is not a valid width/left, which left
// the thumb pinned to the start while the labels kept moving.
export function scrubProgress(page, realPages, frac = 0) {
  const n = Math.max(0, Math.floor(Number(realPages) || 0));
  if (n <= 1) return 0;
  const p = Math.min(n - 1, Math.max(0, Math.floor(Number(page) || 0)));
  const f = Math.min(1, Math.max(0, Number(frac) || 0));
  return Math.min(1, (p + f) / (n - 1));
}

/** Inverse of scrubProgress(..., 0): the real page under a 0–1 track fraction. */
export function pageFromScrub(t, realPages) {
  const n = Math.max(0, Math.floor(Number(realPages) || 0));
  if (n <= 1) return 0;
  const u = Math.min(1, Math.max(0, Number(t) || 0));
  return Math.min(n - 1, Math.round(u * (n - 1)));
}

/** 1-based page pair shared by the top counter and the bottom bar. */
export function pageReadout(page, realPages) {
  const n = Math.max(0, Math.floor(Number(realPages) || 0));
  if (!n) return { page: 0, total: 0, text: '- / -' };
  const p = Math.min(n - 1, Math.max(0, Math.floor(Number(page) || 0)));
  return { page: p + 1, total: n, text: `${p + 1} / ${n}` };
}

const READER_BLANK_BLOCK = [
  'button', 'a', 'input', 'textarea', 'select', 'label',
  '.scrub', '.drawer', '.flyout', '.endcard', '.reader-err', '.help-overlay',
  '.thumbstrip', '.page', '.flip-arrow', '.flip-image', '.chrome-toggle',
  '.modal', '.modal-backdrop', '.mp__head', '.tb', '.bb', '.bp',
  '.shell-top', '.shell-rail', '.vp__auto', '.reader-loading', '.hcard',
].join(', ');

/** True for empty reading surface (margins, gaps). Comic pages and controls are not blank. */
export function isReaderBlankTarget(target) {
  if (!target || typeof target.closest !== 'function') return false;
  if (target.closest(READER_BLANK_BLOCK)) return false;
  return !!target.closest('.viewport, .scroller, .sizer, #host, #vp');
}

/** Sticky show/hide for the reader shell. Scroll position does not call this. */
export function applyReaderChromeClasses(doc, hidden) {
  const on = !!hidden;
  doc.body?.classList.toggle('reader-chrome-away', on);
  const rg = doc.querySelector?.('#rg') || doc.querySelector?.('.rg');
  rg?.classList.toggle('chrome-away', on);
  return on;
}
const el = (tag, cls) => { const e = document.createElement(tag); if (cls) e.className = cls; return e; };

// ---------------------------------------------------------------- Strip
// Owns the scroller, the sizer and one absolutely-positioned box per virtual page.
// Every box gets an explicit integer top/height from Layout, so reserved space is exact and
// image loading can never shift the content (no layout shift, no scroll-anchoring surprises).
class Strip {
  constructor(host) {
    this.scroller = el('div', 'scroller');
    this.sizer = el('div', 'sizer');
    this.scroller.append(this.sizer);
    host.append(this.scroller);
    this.layout = new Layout();
    this.pages = [];
  }
  get viewW() { return this.scroller.clientWidth; }  // excludes the scrollbar
  get viewH() { return this.scroller.clientHeight; }

  build(dims, src) {
    this.layout.setBook(dims, src);
    const frag = document.createDocumentFragment();
    this.pages = dims.map((_, i) => {
      const d = el('div', 'page'); d.dataset.i = i;
      const label = el('span', 'page-label'); label.textContent = src[i] + 1; // faint number while loading
      d.append(label); frag.append(d); return d;
    });
    this.sizer.replaceChildren(frag);
  }

  apply(params) {
    const L = this.layout;
    L.compute(params);
    this.sizer.style.width = L.sizerW + 'px';
    this.sizer.style.height = L.sizerH + 'px';
    for (let i = 0; i < L.n; i++) {
      const s = this.pages[i].style;
      s.top = L.vTop[i] + 'px'; s.left = L.left + 'px'; s.width = L.colW + 'px'; s.height = L.vH[i] + 'px';
    }
  }

  // Re-layout while keeping the same (page, frac) under viewport line `anchorY`
  // (0 = top edge: resize / dim fixes; pointer y: Ctrl+wheel zoom).
  relayout(params, anchorY = 0) {
    const sc = this.scroller;
    const pos = this.layout.getPosition(sc.scrollTop + anchorY);   // 1. read BEFORE touching anything
    this.apply(params);                                            // 2. new tops/heights/sizer
    // The scroll event this assignment fires is not the reader scrolling.
    this.ignoreChromeScroll = true;
    sc.scrollTop = this.layout.posToY(pos) - anchorY;              // 3. restore in the same task
    this.ignoreChromeScroll = false;
  }
}

// ---------------------------------------------------------------- ViewportTracker
// rAF-coalesced scroll handler. Computes: position at viewport top (what we SAVE),
// page at viewport middle (what we SHOW), direction and speed.
class ViewportTracker {
  constructor(reader) {
    this.r = reader; this.raf = 0;
    this.lastY = reader.strip.scroller.scrollTop; this.lastT = performance.now(); this.dir = 1;
    this.onScroll = () => { if (!this.raf) this.raf = requestAnimationFrame(() => this.tick()); };
    reader.strip.scroller.addEventListener('scroll', this.onScroll, { passive: true });
  }
  // Call right before a programmatic jump so the jump isn't mistaken for a fling.
  expectJump(toY) { this.lastY = toY; this.lastT = performance.now(); }

  tick() {
    this.raf = 0;
    const { strip, imageWindow: iw, prefetcher, saver } = this.r;
    const sc = strip.scroller, L = strip.layout;
    const y = sc.scrollTop, vh = sc.clientHeight, now = performance.now();
    const dy = y - this.lastY;
    this.lastY = y; this.lastT = now;
    if (Math.abs(dy) > 1) this.dir = dy > 0 ? 1 : -1;

    const pos = L.getPosition(y);                              // viewport TOP -> saved
    const midPos = L.getPosition(y + vh * 0.5);                // viewport MIDDLE -> shown
    const atEnd = y + vh >= sc.scrollHeight - 4;
    this.r.emitPosition({ ...pos, shownPage: midPos.page, shownFrac: midPos.frac, realPages: L.realPages, atEnd });

    prefetcher.update(this.r.computeWindow(y, this.dir));
    saver.mark(pos, atEnd);
    iw.flush();                                                // nearest pages, including while the strip is still moving
    iw.warmAhead(this.dir);
  }
  destroy() { this.r.strip.scroller.removeEventListener('scroll', this.onScroll); }
}

// ---------------------------------------------------------------- ImageWindow
// Two IntersectionObservers over every page box = hysteresis:
//   near (PERF.nearVh): candidate for mounting
//   far  (PERF.farVh): leaving -> unmount
// flush() also mounts from layout math on each scroll frame, so a fast fling does not
// wait for the observer. Far stays larger than near, so those pages are not dropped
// on the next observer callback.
class ImageWindow {
  constructor(reader, perf) {
    this.r = reader; this.perf = perf;
    this.mounted = new Map();   // virtual idx -> { img, tries }
    this.near = new Set();      // virtual idx currently inside the near margin
    this.warm = new Map();      // virtual idx -> Image not in the document yet
    const root = reader.strip.scroller;
    this.nearIO = new IntersectionObserver(es => this.onNear(es), { root, rootMargin: rootMargin(perf.nearVh), threshold: 0 });
    this.farIO  = new IntersectionObserver(es => this.onFar(es),  { root, rootMargin: rootMargin(perf.farVh), threshold: 0 });
    for (const p of reader.strip.pages) { this.nearIO.observe(p); this.farIO.observe(p); }
  }

  onNear(entries) {
    for (const e of entries) {
      const i = +e.target.dataset.i;
      if (e.isIntersecting) this.near.add(i); else this.near.delete(i);
    }
    this.flush();
  }
  onFar(entries) { for (const e of entries) if (!e.isIntersecting) this.release(+e.target.dataset.i); }

  // Mount the nearest-to-viewport-centre pages first (and only up to the budget), so the
  // page being read is always the first request Rust sees. Called on every scroll frame:
  // a fast fling used to skip this until the wheel went quiet.
  flush() {
    const { strip } = this.r, L = strip.layout;
    if (!L.n) return;
    const vh = strip.viewH;
    const top = strip.scroller.scrollTop;
    const c = top + vh / 2;
    const dist = (i) => Math.abs(L.vTop[i] + L.vH[i] / 2 - c);
    const candidates = new Set(this.near);
    for (const i of virtualsNear(L, top, vh, this.perf.nearVh)) candidates.add(i);
    const want = [...candidates].sort((a, b) => dist(a) - dist(b)).slice(0, this.perf.maxMounted);
    let synced = false;
    for (const i of want) {
      if (this.mounted.has(i)) continue;
      const sync = !synced && dist(i) <= vh;
      this.mount(i, sync);
      if (sync) synced = true;
    }
  }

  // Start fetches for the next few pages before they enter the mount margin.
  // The element is reused by mount(), so the bitmap is not decoded twice.
  warmAhead(dir) {
    const { strip } = this.r, L = strip.layout;
    const nWarm = this.perf.warm || 0;
    if (!nWarm || !L.n) return;
    const y = strip.scroller.scrollTop, vh = strip.viewH;
    const center = L.findVirtual(y + vh * 0.5);
    const step = dir < 0 ? -1 : 1;
    const want = [];
    for (let k = 1; want.length < nWarm && k < L.n; k++) {
      const i = center + step * k;
      if (i < 0 || i >= L.n) break;
      if (!this.mounted.has(i)) want.push(i);
    }
    const keep = new Set(want);
    for (const [i, img] of this.warm) {
      if (!keep.has(i)) { img.removeAttribute('src'); this.warm.delete(i); }
    }
    for (const i of want) {
      if (this.warm.has(i)) continue;
      const img = new Image();
      img.className = 'pg'; img.decoding = 'async'; img.draggable = false; img.alt = '';
      if ('fetchPriority' in img) img.fetchPriority = 'low';
      img.src = this.url(i);
      this.warm.set(i, img);
    }
  }

  takeWarm(i) {
    const img = this.warm.get(i);
    if (!img) return null;
    this.warm.delete(i);
    if (img.complete && !img.naturalWidth) { img.removeAttribute('src'); return null; }
    return img;
  }

  url(i, tries = 0) {
    const { bookId, sig } = this.r.info;
    return `${PAGE_BASE}/${bookId}/${i}?v=${this.perf.variant}&s=${sig}` + (tries ? `&r=${tries}` : '');
  }

  mount(i, syncDecode = false) {
    if (this.mounted.has(i)) return;
    const box = this.r.strip.pages[i];
    let img = this.takeWarm(i);
    if (!img) {
      img = new Image();
      img.className = 'pg'; img.draggable = false; img.alt = '';
      img.decoding = syncDecode ? 'sync' : 'async';
      if ('fetchPriority' in img) img.fetchPriority = syncDecode ? 'high' : 'auto';
      img.src = this.url(i);
    }
    const rec = { img, tries: 0 };
    this.mounted.set(i, rec);
    box.append(img);
    this.afterDecode(i, rec);
  }

  afterDecode(i, rec) {
    rec.img.decode().then(() => {
      if (this.mounted.get(i) !== rec) return;           // unmounted meanwhile
      rec.img.classList.add('ready');
      this.checkDims(i, rec.img);
    }).catch(() => { if (this.mounted.get(i) === rec) this.onError(i, rec); });
  }

  release(i) {
    const rec = this.mounted.get(i);
    if (!rec) return;
    this.mounted.delete(i);
    rec.img.removeAttribute('src');        // cancels an in-flight request and frees the bitmap
    rec.img.remove();
    this.r.strip.pages[i].classList.remove('failed');
  }

  async onError(i, rec) {
    const box = this.r.strip.pages[i];
    if (rec.tries < 2) {                   // transient (e.g. 503 cancelled by a jump): retry
      rec.tries++;
      await new Promise(r => setTimeout(r, 400 * rec.tries));
      if (this.mounted.get(i) !== rec) return;
      rec.img.src = this.url(i, rec.tries);
      this.afterDecode(i, rec);
      return;
    }
    // Permanent: ask the protocol WHY (img.onerror gives no body), show it in the placeholder.
    let why = 'failed to load';
    try { const r = await fetch(this.url(i)); if (!r.ok) { const j = await r.json(); why = j.code || why; } } catch {}
    box.classList.add('failed'); box.dataset.error = why;   // CSS: .page.failed::after{content:"Page " attr(data-n) " - " attr(data-error)}
    rec.img.remove();
  }

  // Real size differs from what the header said (EXIF-rotated JPEG, unknown dims, lying header)
  checkDims(i, img) {
    const L = this.r.strip.layout, [w, h] = L.dims[i];
    const nw = img.naturalWidth, nh = img.naturalHeight;
    if (!nw || !nh) return;
    const known = w > 0 && h > 0;
    if (!known || Math.abs((nh / nw) / (h / w) - 1) > 0.01) this.r.fixDim(i, nw, nh);
  }

  destroy() {
    this.nearIO.disconnect(); this.farIO.disconnect();
    for (const img of this.warm.values()) img.removeAttribute('src');
    this.warm.clear();
    for (const i of [...this.mounted.keys()]) this.release(i);
  }
}

// ---------------------------------------------------------------- Prefetcher
// Tells Rust which pages the extraction queue should work on. Sends only when the window
// changed, at most every prefetchMinGapMs, with a trailing send so the final position wins.
class Prefetcher {
  constructor(reader) { this.r = reader; this.lastKey = ''; this.pending = null; this.timer = 0; this.lastAt = 0; }

  update(w) {
    const key = `${w.center}:${w.direction}:${w.ahead}:${w.behind}`;
    if (key === this.lastKey) return;
    this.pending = w;
    if (this.timer) return;
    const wait = Math.max(0, CFG.prefetchMinGapMs - (performance.now() - this.lastAt));
    this.timer = setTimeout(() => this.send(), wait);
  }
  send() {
    this.timer = 0;
    const w = this.pending; if (!w) return;
    this.lastKey = `${w.center}:${w.direction}:${w.ahead}:${w.behind}`;
    this.lastAt = performance.now();
    api.setWindow({ bookId: this.r.info.bookId, variant: this.r.perf.variant, ...w }).catch(() => {});
  }
  // Jumps (scrubber commit, chapter click, resume) bypass the throttle: Rust must re-aim
  // the queue BEFORE the <img> requests for the new place arrive.
  jumpNow(w) { clearTimeout(this.timer); this.timer = 0; this.pending = w; this.send(); }
  destroy() { clearTimeout(this.timer); }
}

// ---------------------------------------------------------------- ProgressSaver
// Writes (page, frac) when it changed: 1.5 s after scrolling pauses, but never less often
// than every 4 s during continuous reading. flush() is awaited on close/blur/hide.
export class ProgressSaver {
  constructor(bookId) { this.bookId = bookId; this.dirty = null; this.saved = null; this.lastAt = performance.now(); this.timer = 0; }

  mark(pos, reachedEnd) {
    const s = this.saved;
    if (!reachedEnd && s && s.page === pos.page && Math.abs(s.frac - pos.frac) < 0.002) return;
    this.dirty = { pos, reachedEnd: !!(reachedEnd || this.dirty?.reachedEnd) };
    const now = performance.now();
    const delay = Math.max(0, Math.min(CFG.saveDebounceMs, this.lastAt + CFG.saveMaxWaitMs - now));
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.flush(), delay);
  }
  async flush() {
    clearTimeout(this.timer);
    const d = this.dirty; if (!d) return;
    this.dirty = null;
    try {
      await api.saveProgress(this.bookId, d.pos, d.reachedEnd);
      this.saved = d.pos; this.lastAt = performance.now();
    } catch (e) { this.dirty = this.dirty ?? d; }          // keep it, next mark/flush retries
  }
}

// ---------------------------------------------------------------- Scrubber
// Track maps linearly to REAL pages. While dragging we only preview (label + 160px thumb);
// nothing is mounted or requested for intermediate pages. The jump happens once, on release.
export class Scrubber {
  constructor(track, thumbImg, label, reader) {
    this.track = track; this.thumb = thumbImg; this.label = label; this.r = reader;
    this.dragging = false; this.thumbTimer = 0;
    track.addEventListener('pointerdown', (e) => { this.dragging = true; track.setPointerCapture(e.pointerId); this.move(e); });
    track.addEventListener('pointermove', (e) => { if (this.dragging) this.move(e); });
    track.addEventListener('pointerup',   (e) => { if (!this.dragging) return; this.dragging = false; this.r.jumpTo({ page: this.pageAt(e), frac: 0 }); });
  }
  pageAt(e) {
    const n = this.r.strip?.layout?.realPages || 0;
    const rc = this.track.getBoundingClientRect();
    const t = clamp((e.clientX - rc.left) / Math.max(1, rc.width), 0, 1);
    this.track.style.setProperty('--p', String(Math.min(1, Math.max(0, t))));
    return pageFromScrub(t, n);
  }
  move(e) {
    const page = this.pageAt(e), L = this.r.strip.layout, n = L.realPages;
    const txt = pageReadout(page, n).text;
    this.label.textContent = txt;
    this.r.onScrubPreview?.(page, n);
    clearTimeout(this.thumbTimer);                          // debounce: one thumb request per pause
    this.thumbTimer = setTimeout(() => {
      const { bookId, sig } = this.r.info;
      const vi = L.gFirst[page];
      if (vi == null) return;
      this.thumb.src = `${PAGE_BASE}/${bookId}/${vi}?v=160&s=${sig}`;
    }, 60);
  }
  setValue(page, realPages, frac = 0) {                     // from tracker, when not dragging
    if (this.dragging) return;
    this.track.style.setProperty('--p', String(scrubProgress(page, realPages, frac)));
    this.label.textContent = pageReadout(page, realPages).text;
  }
}

// ---------------------------------------------------------------- Reader (glue)
export class Reader {
  constructor(host, { onPosition = () => {}, scrubber = null } = {}) {
    this.host = host; this.onPosition = onPosition; this.scrubberEls = scrubber;
    this.strip = new Strip(host);
    this.params = { widthPct: 100, maxW: Infinity, zoom: 1, gap: 0, endPad: CFG.endPad };
    this.wheelGear = WHEEL_GEAR_DEFAULT;
    this.perfName = 'balanced'; this.perf = PERF.balanced;
    this.info = null; this.fixes = new Map(); this.fixRaf = 0;
    this._keyScroll = null;
  }

  layoutParams() { return { viewW: this.strip.viewW, ...this.params }; }

  // opts: perf, widthPct, gap, startPos ({page,frac}: overrides the saved resume point),
  //       beforeMount(info) -> false (cancel) | {perf} | undefined   (used for the large-book warning)
  async open(bookId, { perf = 'balanced', widthPct = 100, gap = 0, startPos = null, beforeMount = null, wheelSpeed = WHEEL_GEAR_DEFAULT } = {}) {
    this.perfName = perf; this.perf = PERF[perf];
    this.wheelGear = wheelGear(wheelSpeed);
    Object.assign(this.params, { widthPct, gap });
    this.info = await api.openBook(bookId);                 // { bookId, sig, dims, src, realPages, chapters, resume, ... }
    if (beforeMount) {
      const r = await beforeMount(this.info);
      if (r === false) { await api.closeBook(bookId).catch(() => {}); throw { code: 'cancelled' }; }
      if (r?.perf) { this.perfName = r.perf; this.perf = PERF[r.perf]; }
    }
    const { dims, src } = this.info, resume = startPos ?? this.info.resume;

    this.strip.build(dims, src);
    this.strip.apply(this.layoutParams());
    const sc = this.strip.scroller, L = this.strip.layout;
    sc.scrollTop = resume ? L.posToY(resume) : 0;           // exact resume, before any image is requested

    this.imageWindow = new ImageWindow(this, this.perf);
    this.prefetcher = new Prefetcher(this);
    this.saver = new ProgressSaver(bookId);
    this.tracker = new ViewportTracker(this);
    if (this.scrubberEls) this.scrubber = new Scrubber(...this.scrubberEls, this);

    // Re-aim Rust first, then let the observers fire (first IO callbacks arrive next frame)
    this.prefetcher.jumpNow(this.computeWindow(sc.scrollTop, 1));
    this.tracker.tick();

    // width changes -> relayout with (page, frac) preserved. Height-only changes need nothing.
    let lastW = this.strip.viewW;
    this.ro = new ResizeObserver(() => {
      const w = this.strip.viewW; if (w === lastW) return; lastW = w;
      this.strip.relayout(this.layoutParams());             // ResizeObserver runs before paint: no flash
    });
    this.ro.observe(sc);

    sc.addEventListener('wheel', (e) => this.onWheel(e), { passive: false });
    window.addEventListener('keydown', this.keyHandler = (e) => this.onKey(e));
    document.addEventListener('visibilitychange', this.visHandler = () => { if (document.hidden) this.saver.flush(); });
    window.addEventListener('blur', this.blurHandler = () => this.saver.flush());
  }

  // Which virtual pages should Rust keep warm? Sized in viewport heights, not page counts,
  // so 8000px slices and 300px strips both get sensible windows.
  computeWindow(y, dir) {
    const L = this.strip.layout, vh = this.strip.viewH, p = this.perf;
    const center = L.findVirtual(y + vh * 0.5);
    const ahead  = clamp(L.findVirtual(y + vh * (1 + p.aheadVh)) - center, 1, p.maxAhead);
    const behind = clamp(center - L.findVirtual(Math.max(0, y - vh * p.behindVh)), 0, p.maxBehind);
    return { center, direction: dir >= 0 ? 1 : -1, ahead, behind };
  }

  // ---- common view API (flip.js implements the same surface) ----
  get mode() { return 'scroll'; }
  get layout() { return this.strip.layout; }
  get viewH() { return this.strip.viewH; }
  topPos() { return this.strip.layout.getPosition(this.strip.scroller.scrollTop); }                       // what is saved
  centerPos() { return this.strip.layout.getPosition(this.strip.scroller.scrollTop + this.strip.viewH / 2); }   // what B bookmarks
  stepPage(dir) { const p = this.topPos(); const to = clamp(p.page + dir, 0, this.strip.layout.realPages - 1); this.jumpTo({ page: to, frac: 0 }, { smooth: true }); }
  refresh() { this.tracker?.tick(); }
  setFit() { this.params.zoom = 1; this.params.widthPct = 100; this.strip.relayout(this.layoutParams()); }   // "fit to panel"
  keysEnabled = true;

  emitPosition(p) {
    this.onPosition(p);
    this.scrubber?.setValue(p.shownPage, p.realPages, p.shownFrac || 0);
  }

  jumpTo(pos, { smooth = false, atCenter = false } = {}) {   // atCenter: put the position at the viewport CENTER (bookmarks)
    const sc = this.strip.scroller, L = this.strip.layout;
    const y = clamp(L.posToY(pos) - (atCenter ? sc.clientHeight / 2 : 0), 0, sc.scrollHeight - sc.clientHeight);
    const center = L.findVirtual(y + sc.clientHeight * 0.5);
    this.prefetcher.jumpNow({ ...this.computeWindow(y, y >= sc.scrollTop ? 1 : -1), center });
    if (smooth) { sc.scrollTo({ top: y, behavior: 'smooth' }); return; }
    this.tracker.expectJump(y);
    sc.scrollTop = y;                                        // observers fire next frame -> demand requests
  }

  // Ctrl+wheel zoom anchored on the pointer. Plain wheel uses the saved gear;
  // gear 1 is the browser's own distance. Auto-scroll and the page keys do not come through here.
  onWheel(e) {
    if (e.ctrlKey) {
      e.preventDefault();
      this.setZoom(this.params.zoom * (e.deltaY < 0 ? 1.1 : 1 / 1.1), e.clientY - this.strip.scroller.getBoundingClientRect().top);
      return;
    }
    this.stopKeyScroll();
    const d = wheelScrollDelta(e, this.wheelGear, { w: this.strip.viewW, h: this.strip.viewH });
    if (!d) return;
    e.preventDefault();
    this.strip.scroller.scrollBy({ left: d.dx, top: d.dy, behavior: 'auto' });
  }
  setWheelSpeed(gear) { this.wheelGear = wheelGear(gear); }

  beginKeyScroll(key, shift = false) {
    const plan = keyScrollPlan(key, { shift, viewH: this.strip.viewH });
    if (!plan) return;
    this.stopKeyScroll();
    const hold = {
      ...plan,
      traveled: 0,
      holding: true,
      raf: 0,
      lastT: performance.now(),
      onUp: null,
    };
    this._keyScroll = hold;
    hold.onUp = (ev) => {
      if (ev.key !== hold.key) return;
      hold.holding = false;
      window.removeEventListener('keyup', hold.onUp);
      hold.onUp = null;
    };
    window.addEventListener('keyup', hold.onUp);
    const tick = (now) => {
      if (this._keyScroll !== hold) return;
      const dt = Math.min(0.05, Math.max(0, (now - hold.lastT) / 1000));
      hold.lastT = now;
      const sc = this.strip.scroller;
      const before = sc.scrollTop;
      sc.scrollTop = before + hold.dir * hold.pxPerSec * dt;
      hold.traveled += Math.abs(sc.scrollTop - before);
      const atEnd = (hold.dir > 0 && sc.scrollTop + sc.clientHeight >= sc.scrollHeight - 1)
        || (hold.dir < 0 && sc.scrollTop <= 0);
      if (atEnd || (!hold.holding && hold.traveled >= hold.minTravel)) {
        this.stopKeyScroll();
        return;
      }
      hold.raf = requestAnimationFrame(tick);
    };
    hold.raf = requestAnimationFrame(tick);
  }

  stopKeyScroll() {
    const hold = this._keyScroll;
    if (!hold) return;
    this._keyScroll = null;
    cancelAnimationFrame(hold.raf);
    if (hold.onUp) window.removeEventListener('keyup', hold.onUp);
  }
  setZoom(z, anchorY = 0) {
    this.params.zoom = clamp(Math.round(z * 100) / 100, 0.5, 3);
    this.strip.relayout(this.layoutParams(), anchorY);
  }
  setWidthPct(pct) { this.params.widthPct = clamp(pct, 40, 100); this.strip.relayout(this.layoutParams()); }
  setGap(px)       { this.params.gap = px;                       this.strip.relayout(this.layoutParams()); }

  // Corrections for pages whose real size differs from the header (batched per frame).
  fixDim(i, w, h) {
    const L = this.strip.layout;
    L.setDim(i, w, h);
    const g = L.src[i];
    if (L.gFirst[g] === L.gLast[g]) this.fixes.set(g, { page: g, w, h });   // sliced pages: ignore
    if (this.fixRaf) return;
    this.fixRaf = requestAnimationFrame(() => {
      this.fixRaf = 0;
      this.strip.relayout(this.layoutParams());              // anchored on viewport top (page, frac)
      const batch = [...this.fixes.values()]; this.fixes.clear();
      if (batch.length) api.fixDims(this.info.bookId, batch).catch(() => {});
    });
  }

  onKey(e) {
    if (!this.keysEnabled || document.documentElement.dataset.modal === '1') return;     // dialogs own the keyboard
    if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;

    // Space / Page / arrows: one constant velocity for tap and hold. Ignore OS repeat.
    if (keyScrollPlan(e.key, { shift: e.shiftKey })) {
      e.preventDefault();
      if (e.repeat) return;
      this.beginKeyScroll(e.key, e.shiftKey);
      return;
    }

    if (e.repeat) return;                                      // don't stack Home/End/side jumps
    const page = this.strip.layout.getPosition(this.strip.scroller.scrollTop).page;
    switch (e.key) {
      case 'ArrowRight': this.jumpTo({ page: page + 1, frac: 0 }, { smooth: true }); break;
      case 'ArrowLeft':  this.jumpTo({ page: Math.max(0, page - 1), frac: 0 }, { smooth: true }); break;
      case 'Home': this.jumpTo({ page: 0, frac: 0 }); break;
      case 'End':  this.jumpTo({ page: this.strip.layout.realPages - 1, frac: 0.9999 }); break;
      default:
        if (e.ctrlKey && (e.key === '+' || e.key === '=')) this.setZoom(this.params.zoom * 1.1);
        else if (e.ctrlKey && e.key === '-') this.setZoom(this.params.zoom / 1.1);
        else if (e.ctrlKey && e.key === '0') this.setZoom(1);
        else return;
    }
    e.preventDefault();
  }

  async close() {
    this.stopKeyScroll();
    await this.saver?.flush();
    window.removeEventListener('keydown', this.keyHandler);
    document.removeEventListener('visibilitychange', this.visHandler);
    window.removeEventListener('blur', this.blurHandler);
    this.ro?.disconnect(); this.tracker?.destroy(); this.imageWindow?.destroy(); this.prefetcher?.destroy();
    this.strip.scroller.remove();                              // leave the host empty for the next view
    if (this.info) await api.closeBook(this.info.bookId).catch(() => {});
  }
}

// ---------------------------------------------------------------- boot (reader.html)
// const reader = new Reader(document.getElementById('host'), {
//   onPosition: p => { counter.textContent = `${p.shownPage + 1} / ${p.realPages}`; },
//   scrubber: [track, thumbImg, labelEl],
// });
// await reader.open(Number(new URLSearchParams(location.search).get('book')));
// const win = window.__TAURI__.window.getCurrentWindow();
// await win.onCloseRequested(async (e) => { e.preventDefault(); await reader.close(); await win.destroy(); });
