// reader.js - vertical-scroll reader core for MH Reader (ES module, no bundler needed).
// Requires tauri.conf.json: app.withGlobalTauri = true   (so window.__TAURI__ exists)
//
// Objects:  Strip (DOM + layout)   ViewportTracker (scroll -> position/direction/speed)
//           ImageWindow (IO-driven mount/unmount)   Prefetcher (-> Rust set_reading_window)
//           ProgressSaver (debounced save)   Scrubber (bottom slider)   Reader (glue)
import { Layout } from './layout.js';

const T = window.__TAURI__;
const invoke = T.core.invoke;
// Windows/WebView2: "http://mhpage.localhost"   (macOS/Linux would be "mhpage://localhost")
const PAGE_BASE = T.core.convertFileSrc('', 'mhpage').replace(/\/$/, '');
/** 160px-wide preview of virtual page `i` (a slice for tall images). Used by the bookmarks panel; same URL shape as the scrubber. */
export const thumbUrl = (info, i) => `${PAGE_BASE}/${info.bookId}/${i}?v=160&s=${info.sig}`;

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

const PERF = {
  balanced: { nearMargin: '150% 0px', farMargin: '300% 0px', maxMounted: 14,
              aheadVh: 5, behindVh: 1.5, maxAhead: 10, maxBehind: 3, variant: 0 },
  saver:    { nearMargin: '80% 0px',  farMargin: '160% 0px', maxMounted: 6,
              aheadVh: 2.5, behindVh: 0, maxAhead: 4, maxBehind: 0, variant: 1600 },
};
const CFG = {
  prefetchMinGapMs: 120,       // at most ~8 set_reading_window calls/sec
  fastVhPerSec: 3,             // faster than 3 viewport-heights/sec = "flinging": hold off mounting
  settleMs: 90,                // ...until scrolling has been calm this long
  saveDebounceMs: 1500, saveMaxWaitMs: 4000,
  endPad: 360,                 // room under the last page for the end card
};
const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
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
    sc.scrollTop = this.layout.posToY(pos) - anchorY;              // 3. restore in the same task
  }
}

// ---------------------------------------------------------------- ViewportTracker
// rAF-coalesced scroll handler. Computes: position at viewport top (what we SAVE),
// page at viewport middle (what we SHOW), direction and speed.
class ViewportTracker {
  constructor(reader) {
    this.r = reader; this.raf = 0; this.settle = 0;
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
    const dy = y - this.lastY, dt = Math.max(1, now - this.lastT);
    this.lastY = y; this.lastT = now;
    if (Math.abs(dy) > 1) this.dir = dy > 0 ? 1 : -1;

    // fling control: while very fast, don't mount; mount once calm
    const fast = Math.abs(dy) / dt * 1000 > vh * CFG.fastVhPerSec;
    iw.setGate(fast);
    clearTimeout(this.settle);
    this.settle = setTimeout(() => iw.setGate(false), CFG.settleMs);

    const pos = L.getPosition(y);                              // viewport TOP -> saved
    const mid = L.findVirtual(y + vh * 0.5);                   // viewport MIDDLE -> shown
    const atEnd = y + vh >= sc.scrollHeight - 4;
    this.r.emitPosition({ ...pos, shownPage: L.src[mid], realPages: L.realPages, atEnd });

    prefetcher.update(this.r.computeWindow(y, this.dir));
    saver.mark(pos, atEnd);
    if (iw.near.size > iw.mounted.size) iw.flush();            // cheap top-up when budget freed
  }
  destroy() { this.r.strip.scroller.removeEventListener('scroll', this.onScroll); clearTimeout(this.settle); }
}

// ---------------------------------------------------------------- ImageWindow
// Two IntersectionObservers over every page box = hysteresis:
//   near (rootMargin 150% top/bottom): entering -> candidate for mounting
//   far  (rootMargin 300% top/bottom): leaving  -> unmount
// A page is mounted from 1.5 screens before it becomes visible until it is 3 screens past,
// so scrolling back and forth around the edge never thrashes img elements.
class ImageWindow {
  constructor(reader, perf) {
    this.r = reader; this.perf = perf;
    this.mounted = new Map();   // virtual idx -> { img, tries }
    this.near = new Set();      // virtual idx currently inside the near margin
    this.gated = false;
    const root = reader.strip.scroller;
    this.nearIO = new IntersectionObserver(es => this.onNear(es), { root, rootMargin: perf.nearMargin, threshold: 0 });
    this.farIO  = new IntersectionObserver(es => this.onFar(es),  { root, rootMargin: perf.farMargin,  threshold: 0 });
    for (const p of reader.strip.pages) { this.nearIO.observe(p); this.farIO.observe(p); }
  }

  onNear(entries) {
    for (const e of entries) {
      const i = +e.target.dataset.i;
      if (e.isIntersecting) this.near.add(i); else this.near.delete(i);
    }
    if (!this.gated) this.flush();
  }
  onFar(entries) { for (const e of entries) if (!e.isIntersecting) this.release(+e.target.dataset.i); }

  setGate(closed) {
    if (this.gated === closed) return;
    this.gated = closed;
    if (!closed) this.flush();
  }

  // Mount the nearest-to-viewport-centre pages first (and only up to the budget), so the
  // page being read is always the first request Rust sees.
  flush() {
    const { strip } = this.r, L = strip.layout;
    const c = strip.scroller.scrollTop + strip.viewH / 2;
    const dist = (i) => Math.abs(L.vTop[i] + L.vH[i] / 2 - c);
    const want = [...this.near].sort((a, b) => dist(a) - dist(b)).slice(0, this.perf.maxMounted);
    for (const i of want) this.mount(i);
  }

  url(i, tries = 0) {
    const { bookId, sig } = this.r.info;
    return `${PAGE_BASE}/${bookId}/${i}?v=${this.perf.variant}&s=${sig}` + (tries ? `&r=${tries}` : '');
  }

  mount(i) {
    if (this.mounted.has(i)) return;
    const box = this.r.strip.pages[i];
    const img = new Image();
    img.className = 'pg'; img.decoding = 'async'; img.draggable = false; img.alt = '';
    const rec = { img, tries: 0 };
    this.mounted.set(i, rec);
    img.src = this.url(i);
    box.append(img);                       // opacity:0 until decoded -> no half-painted tiles
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
class ProgressSaver {
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
class Scrubber {
  constructor(track, thumbImg, label, reader) {
    this.track = track; this.thumb = thumbImg; this.label = label; this.r = reader;
    this.dragging = false; this.thumbTimer = 0;
    track.addEventListener('pointerdown', (e) => { this.dragging = true; track.setPointerCapture(e.pointerId); this.move(e); });
    track.addEventListener('pointermove', (e) => { if (this.dragging) this.move(e); });
    track.addEventListener('pointerup',   (e) => { if (!this.dragging) return; this.dragging = false; this.r.jumpTo({ page: this.pageAt(e), frac: 0 }); });
  }
  pageAt(e) {
    const rc = this.track.getBoundingClientRect();
    const t = clamp((e.clientX - rc.left) / rc.width, 0, 1);
    return Math.min(this.r.strip.layout.realPages - 1, Math.floor(t * this.r.strip.layout.realPages));
  }
  move(e) {
    const page = this.pageAt(e), L = this.r.strip.layout;
    this.label.textContent = `${page + 1} / ${L.realPages}`;
    clearTimeout(this.thumbTimer);                          // debounce: one thumb request per pause
    this.thumbTimer = setTimeout(() => {
      const { bookId, sig } = this.r.info;
      this.thumb.src = `${PAGE_BASE}/${bookId}/${L.gFirst[page]}?v=160&s=${sig}`;
    }, 60);
  }
  setValue(page, realPages) {                               // from tracker, when not dragging
    if (this.dragging) return;
    this.track.style.setProperty('--p', realPages > 1 ? page / (realPages - 1) : 0);
    this.label.textContent = `${page + 1} / ${realPages}`;
  }
}

// ---------------------------------------------------------------- Reader (glue)
export class Reader {
  constructor(host, { onPosition = () => {}, scrubber = null } = {}) {
    this.host = host; this.onPosition = onPosition; this.scrubberEls = scrubber;
    this.strip = new Strip(host);
    this.params = { widthPct: 100, maxW: 1000, zoom: 1, gap: 0, endPad: CFG.endPad };
    this.perfName = 'balanced'; this.perf = PERF.balanced;
    this.info = null; this.fixes = new Map(); this.fixRaf = 0;
  }

  layoutParams() { return { viewW: this.strip.viewW, ...this.params }; }

  async open(bookId, { perf = 'balanced', widthPct = 100, gap = 0 } = {}) {
    this.perfName = perf; this.perf = PERF[perf];
    Object.assign(this.params, { widthPct, gap });
    this.info = await api.openBook(bookId);                 // { bookId, sig, dims, src, realPages, chapters, resume, ... }
    const { dims, src, resume } = this.info;

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

  emitPosition(p) {
    this.onPosition(p);
    this.scrubber?.setValue(p.shownPage, p.realPages);
  }

  jumpTo(pos, { smooth = false, atCenter = false } = {}) {   // atCenter: put the position at the viewport CENTER (bookmarks)
    const sc = this.strip.scroller, L = this.strip.layout;
    const y = clamp(L.posToY(pos) - (atCenter ? sc.clientHeight / 2 : 0), 0, sc.scrollHeight - sc.clientHeight);
    const center = L.findVirtual(y + sc.clientHeight * 0.5);
    this.prefetcher.jumpNow({ ...this.computeWindow(y, y >= sc.scrollTop ? 1 : -1), center });
    if (smooth) { sc.scrollTo({ top: y, behavior: 'smooth' }); return; }
    this.tracker.expectJump(y);                              // not a fling: don't gate mounting
    this.imageWindow.setGate(false);
    sc.scrollTop = y;                                        // observers fire next frame -> demand requests
  }

  // Ctrl+wheel zoom anchored on the pointer; plain wheel is left to the browser.
  onWheel(e) {
    if (!e.ctrlKey) return;
    e.preventDefault();
    this.setZoom(this.params.zoom * (e.deltaY < 0 ? 1.1 : 1 / 1.1), e.clientY - this.strip.scroller.getBoundingClientRect().top);
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
    const sc = this.strip.scroller, vh = sc.clientHeight;
    if (e.target instanceof HTMLInputElement) return;
    const page = this.strip.layout.getPosition(sc.scrollTop).page;
    switch (e.key) {
      case ' ': case 'PageDown': sc.scrollBy({ top: (e.shiftKey ? -1 : 1) * vh * 0.9, behavior: 'smooth' }); break;
      case 'PageUp':  sc.scrollBy({ top: -vh * 0.9, behavior: 'smooth' }); break;
      case 'ArrowDown': sc.scrollBy({ top: 80 }); break;
      case 'ArrowUp':   sc.scrollBy({ top: -80 }); break;
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
    await this.saver?.flush();
    window.removeEventListener('keydown', this.keyHandler);
    document.removeEventListener('visibilitychange', this.visHandler);
    window.removeEventListener('blur', this.blurHandler);
    this.ro?.disconnect(); this.tracker?.destroy(); this.imageWindow?.destroy(); this.prefetcher?.destroy();
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
