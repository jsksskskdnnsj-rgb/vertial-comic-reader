// Lightweight page-turning view sharing Reader's backend and position contract.
import { api, pageUrl } from './reader.js';
import { createListenerBag } from './listener-bag.js';
import { Layout, clampWidthPct } from './layout.js';
import { applyCropStyle, cropCacheKey, detectCropFromImage, mergeCropInsets } from './reader-crop.js';
import { flipWarmPages, flipTapZone } from './flip-adjacent.js';
import { rtlStep, rtlTapZone } from './reader-rtl.js';
import { createFlipWheelState, flipSwipeDir, flipVisualSign, flipWheelAdvance } from './reader-feel.js';

export class FlipView {
  constructor(host, { onPosition = () => {}, scrubber = null } = {}) {
    this.host = host;
    this.onPosition = onPosition;
    this.scrubber = scrubber;
    this.mode = 'flip';
    this.page = 0;
    this.spread = false;
    this.crop = { enabled: false, auto: true, manualTrim: 0, cache: new Map(), detectOpts: {} };
    this.rtl = false;
    this.info = null;
    this.layout = new Layout();
    this.params = { widthPct: 100, spreadGap: 8 };
    this.viewH = host.clientHeight;
    this._warm = new Map();
    this._wheelState = createFlipWheelState();
    this._ptr = null;
    this._suppressClick = false;
    this._anim = null;
    this._aimedPage = -1;
    this._lastDir = 1;
    this._keyAt = 0;
    this._key = (e) => {
      if (document.documentElement.dataset.modal === '1' || e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
      const forward = e.key === 'ArrowRight' || e.key === 'PageDown' || (e.key === ' ' && !e.shiftKey);
      const back = e.key === 'ArrowLeft' || e.key === 'PageUp' || (e.key === ' ' && e.shiftKey);
      if (!forward && !back) return;
      e.preventDefault();
      // OS key-repeat is faster than a page can settle. The first press is
      // immediate; repeats are capped so a hold turns pages instead of flashing.
      const now = performance.now();
      if (e.repeat && now - this._keyAt < 160) return;
      this._keyAt = now;
      if (forward) this.next();
      else this.prev();
    };
    this._onWheel = (e) => {
      if (e.ctrlKey) return;
      // Flip has nothing to scroll. Keep the gesture, including the part of a
      // flick that has not yet crossed the page threshold.
      e.preventDefault();
      const now = e.timeStamp || performance.now();
      const step = flipWheelAdvance(this._wheelState, e, now);
      this._wheelState = step.state;
      if (step.dir) this.stepPage(step.dir);
    };
    this._tap = (e) => {
      if (this._suppressClick) { this._suppressClick = false; return; }
      const stage = e.currentTarget;
      const z = rtlTapZone(flipTapZone(e.clientX - stage.getBoundingClientRect().left, stage.clientWidth), this.rtl);
      if (z < 0) this.prev();
      else if (z > 0) this.next();
    };
  }

  async open(bookId, opts = {}) {
    this.info = await api.openBook(bookId);
    this.bookId = bookId;
    this.params = { widthPct: clampWidthPct(opts.widthPct ?? 100), spreadGap: opts.spreadGap ?? 8 };
    this.spread = !!opts.spread;
    if (opts.beforeMount) {
      const r = await opts.beforeMount(this.info);
      if (r === false) { await api.closeBook(bookId).catch(() => {}); throw { code: 'cancelled' }; }
    }
    this.layout.setBook(this.info.dims || [], this.info.src || []);
    this.layout.compute({ viewW: this.host.clientWidth || 800, widthPct: this.params.widthPct, gap: 0, spread: false });
    const start = opts.startPos?.page ?? this.info.resume?.page ?? 0;
    this.page = this.spread ? this.layout.spreadStart(start) : Math.max(0, Math.min(this.layout.realPages - 1, start));
    this.mountDom();
    this._listen?.clear();
    this._listen = createListenerBag();
    this._listen.add(window, 'keydown', this._key);
    this._listen.add(this.host, 'wheel', this._onWheel, { passive: false });
    this.render();
    return this.info;
  }

  mountDom() {
    const spreadCls = this.spread ? ' flip-view--spread' : '';
    this.host.innerHTML = `<div class="flip-view${spreadCls}">
      <button class="flip-arrow flip-prev" type="button" aria-label="Previous">‹</button>
      <div class="flip-stage">
        <img class="flip-image flip-image--a" alt="">
        <img class="flip-image flip-image--b" alt="" hidden>
      </div>
      <button class="flip-arrow flip-next" type="button" aria-label="Next">›</button>
    </div>`;
    this.imgA = this.host.querySelector('.flip-image--a');
    this.imgB = this.host.querySelector('.flip-image--b');
    this.host.querySelector('.flip-prev').onclick = () => this.prev();
    this.host.querySelector('.flip-next').onclick = () => this.next();
    const stage = this.host.querySelector('.flip-stage');
    stage.addEventListener('click', this._tap);
    stage.addEventListener('pointerdown', (e) => this._pointerDown(e));
    stage.addEventListener('pointerup', (e) => this._pointerUp(e));
    stage.addEventListener('pointercancel', () => { this._ptr = null; });
    this._stageEl = stage;
  }

  refresh() { this.render(); }

  _pointerDown(e) {
    if (e.button !== 0) return;
    this._ptr = { id: e.pointerId, x: e.clientX, y: e.clientY, t: performance.now() };
    try { e.currentTarget.setPointerCapture(e.pointerId); } catch { /* not a captor */ }
  }

  _pointerUp(e) {
    const p = this._ptr;
    this._ptr = null;
    if (!p || p.id !== e.pointerId) return;
    const dir = flipSwipeDir(e.clientX - p.x, e.clientY - p.y, performance.now() - p.t, this.rtl);
    if (!dir) return;
    this._suppressClick = true;
    this.stepPage(dir);
  }

  _reduceMotion() {
    return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
  }

  _play(keyframes, duration) {
    const stage = this._stageEl;
    if (!stage || this._reduceMotion()) return;
    this._anim?.cancel();
    this._anim = stage.animate(keyframes, { duration, easing: 'cubic-bezier(0.16, 0.84, 0.22, 1)', fill: 'none' });
  }

  _settle(indexDir) {
    const sign = flipVisualSign(indexDir, this.rtl);
    if (!sign) return;
    const from = sign * 26;
    this._play(
      [{ transform: `translate3d(${from}px,0,0)` }, { transform: 'translate3d(0,0,0)' }],
      190,
    );
  }

  _resist(indexDir) {
    const sign = flipVisualSign(indexDir, this.rtl);
    if (!sign) return;
    const px = sign * 12;
    this._play(
      [
        { transform: 'translate3d(0,0,0)' },
        { transform: `translate3d(${px}px,0,0)` },
        { transform: 'translate3d(0,0,0)' },
      ],
      230,
    );
  }

  _aimExtractor() {
    if (!this.info || this._aimedPage === this.page) return;
    this._aimedPage = this.page;
    // Same index the <img> requests. Aiming the queue there lets the next
    // turn reuse an extracted bitmap instead of stalling on the click.
    api.setWindow({
      bookId: this.info.bookId,
      variant: 0,
      center: this.page,
      direction: this._lastDir < 0 ? -1 : 1,
      ahead: 5,
      behind: 2,
    }).catch(() => {});
  }

  /** Put a decoded warm bitmap into the visible slot. Returns true when it painted immediately. */
  _place(slot, page, maxW) {
    if (!slot || page == null || page >= this.layout.realPages) {
      if (slot) { slot.hidden = true; slot.removeAttribute('src'); delete slot.dataset.page; }
      return true;
    }
    const url = pageUrl(this.info, page, 0);
    slot.hidden = false;
    slot.style.maxWidth = maxW;
    slot.draggable = false;
    const warm = this._warm.get(page);
    if (warm && warm.complete && warm.naturalWidth > 0 && warm !== slot) {
      warm.className = slot.className;
      warm.alt = '';
      warm.hidden = false;
      warm.draggable = false;
      warm.style.maxWidth = maxW;
      warm.dataset.page = String(page);
      slot.replaceWith(warm);
      if (slot === this.imgA) this.imgA = warm;
      else if (slot === this.imgB) this.imgB = warm;
      this._warm.delete(page);
      this.decorateImg(page, warm);
      return true;
    }
    if (slot.dataset.page !== String(page)) {
      slot.dataset.page = String(page);
      if ('fetchPriority' in slot) slot.fetchPriority = 'high';
      slot.src = url;
      slot.onload = () => this.decorateImg(page, slot);
    }
    const hot = slot.complete && slot.naturalWidth > 0;
    if (hot) this.decorateImg(page, slot);
    return hot;
  }

  render() {
    if (!this.info) return false;
    this._aimExtractor();
    this.viewH = this.host.clientHeight;
    const maxW = this.spread ? '48%' : `${clampWidthPct(this.params.widthPct)}%`;
    let hot = true;
    if (this.spread) {
      hot = this._place(this.imgA, this.page, maxW) && hot;
      const pair = this.page + 1 < this.layout.realPages ? this.page + 1 : null;
      hot = this._place(this.imgB, pair, maxW) && hot;
    } else {
      hot = this._place(this.imgA, this.page, maxW) && hot;
      this.imgB.hidden = true;
      this.imgB.removeAttribute('src');
      delete this.imgB.dataset.page;
    }
    const shown = this.spread ? this.page : this.page;
    const maxStart = this.layout.spreadStart(this.layout.realPages - 1);
    const atEnd = this.spread ? this.page >= maxStart : this.page === this.layout.realPages - 1;
    this.onPosition({
      page: shown,
      frac: 0,
      shownPage: shown,
      realPages: this.layout.realPages,
      atEnd,
    });
    this.warmAdjacent();
    return hot;
  }

  warmAdjacent() {
    if (!this.info) return;
    const want = new Set(flipWarmPages(this.page, this.layout.realPages, this.spread));
    for (const [p, img] of this._warm) {
      if (!want.has(p)) { img.removeAttribute('src'); this._warm.delete(p); }
    }
    for (const p of want) {
      if (this._warm.has(p)) continue;
      const img = new Image();
      img.decoding = 'async';
      img.draggable = false;
      if ('fetchPriority' in img) img.fetchPriority = 'low';
      img.src = pageUrl(this.info, p, 0);
      this._warm.set(p, img);
    }
  }

  decorateImg(page, img) {
    const crop = this.crop;
    if (!crop.enabled) { applyCropStyle(img, null, false); return; }
    const vi = this.layout.gFirst[page];
    const key = cropCacheKey(this.info.bookId, this.info.sig, vi);
    let base = crop.cache.get(key);
    if (base === undefined && crop.auto) {
      base = detectCropFromImage(img, crop.detectOpts);
      crop.cache.set(key, base);
    }
    base = base || { top: 0, right: 0, bottom: 0, left: 0 };
    applyCropStyle(img, mergeCropInsets(base, crop.manualTrim), true);
  }

  topPos() { return { page: this.page, frac: 0 }; }
  centerPos() { return { page: this.spread ? Math.min(this.page, this.layout.realPages - 1) : this.page, frac: 0 }; }
  jumpTo(pos) {
    const p = Math.max(0, Math.min(this.layout.realPages - 1, Math.floor(pos?.page || 0)));
    this.page = this.spread ? this.layout.spreadStart(p) : p;
    this.render();
  }
  stepPage(dir) {
    const before = this.page;
    const indexDir = Math.sign(dir) || 0;
    if (this.spread) {
      const n = this.layout.spreadStart(this.page) + (dir > 0 ? 2 : -2);
      this.page = Math.max(0, Math.min(this.layout.realPages - 1, n));
      this.page = this.layout.spreadStart(this.page);
    } else {
      this.page = Math.max(0, Math.min(this.layout.realPages - 1, this.page + dir));
    }
    if (indexDir) this._lastDir = indexDir;
    const changed = this.page !== before;
    const hot = this.render();
    if (!changed) this._resist(indexDir);
    else if (hot) this._settle(indexDir);
  }
  setRtl(on) { this.rtl = !!on; }
  prev() { this.stepPage(rtlStep(-1, this.rtl)); }
  next() { this.stepPage(rtlStep(1, this.rtl)); }
  scrollable() { return false; }
  setFit() {
    this.params.widthPct = 100;
    if (this.imgA) this.imgA.style.maxWidth = this.spread ? '48%' : '100%';
    if (this.imgB) this.imgB.style.maxWidth = '48%';
  }
  setWidthPct(p) {
    this.params.widthPct = clampWidthPct(p);
    const maxW = this.spread ? '48%' : `${this.params.widthPct}%`;
    if (this.imgA) this.imgA.style.maxWidth = maxW;
  }
  setGap() {}
  setSpread(on) {
    const was = this.page;
    this.spread = !!on;
    this.mountDom();
    this.page = this.spread ? this.layout.spreadStart(was) : was;
    this.render();
  }
  setCrop(opts = {}) {
    Object.assign(this.crop, opts);
    if (opts.enabled === false) this.crop.cache.clear();
    this.render();
  }
  async close() {
    this._anim?.cancel();
    this._anim = null;
    this._listen?.clear();
    this._listen = null;
    this._stageEl?.removeEventListener('click', this._tap);
    for (const img of this._warm.values()) img.removeAttribute('src');
    this._warm.clear();
    if (this.bookId) await api.closeBook(this.bookId).catch(() => {});
    this.host.replaceChildren();
  }
}
