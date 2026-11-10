// Lightweight page-turning view sharing Reader's backend and position contract.
import { api, pageUrl } from './reader.js';
import { Layout } from './layout.js';
import { applyCropStyle, cropCacheKey, detectCropFromImage, mergeCropInsets } from './reader-crop.js';
import { flipWarmPages, flipTapZone } from './flip-adjacent.js';
import { rtlStep, rtlTapZone } from './reader-rtl.js';

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
    this._key = (e) => {
      if (document.documentElement.dataset.modal === '1' || e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
      if (e.key === 'ArrowRight' || e.key === 'PageDown' || (e.key === ' ' && !e.shiftKey)) { e.preventDefault(); this.next(); }
      else if (e.key === 'ArrowLeft' || e.key === 'PageUp' || (e.key === ' ' && e.shiftKey)) { e.preventDefault(); this.prev(); }
    };
    this._tap = (e) => {
      const stage = e.currentTarget;
      const z = rtlTapZone(flipTapZone(e.clientX - stage.getBoundingClientRect().left, stage.clientWidth), this.rtl);
      if (z < 0) this.prev();
      else if (z > 0) this.next();
    };
  }

  async open(bookId, opts = {}) {
    this.info = await api.openBook(bookId);
    this.bookId = bookId;
    this.params = { widthPct: opts.widthPct || 100, spreadGap: opts.spreadGap ?? 8 };
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
    window.addEventListener('keydown', this._key);
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
    this._stageEl = stage;
  }

  refresh() { this.render(); }

  render() {
    if (!this.info) return;
    this.viewH = this.host.clientHeight;
    const maxW = this.spread ? '48%' : `${this.params.widthPct || 100}%`;
    const setImg = (img, p) => {
      if (p == null || p >= this.layout.realPages) { img.hidden = true; img.removeAttribute('src'); return; }
      img.hidden = false;
      img.style.maxWidth = maxW;
      img.src = pageUrl(this.info, p, 0);
      img.onload = () => this.decorateImg(p, img);
    };
    if (this.spread) {
      setImg(this.imgA, this.page);
      setImg(this.imgB, this.page + 1 < this.layout.realPages ? this.page + 1 : null);
    } else {
      setImg(this.imgA, this.page);
      this.imgB.hidden = true;
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
    if (this.spread) {
      const n = this.layout.spreadStart(this.page) + (dir > 0 ? 2 : -2);
      this.page = Math.max(0, Math.min(this.layout.realPages - 1, n));
      if (this.spread) this.page = this.layout.spreadStart(this.page);
    } else {
      this.page = Math.max(0, Math.min(this.layout.realPages - 1, this.page + dir));
    }
    this.render();
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
    this.params.widthPct = p;
    const maxW = this.spread ? '48%' : `${p}%`;
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
    window.removeEventListener('keydown', this._key);
    this._stageEl?.removeEventListener('click', this._tap);
    for (const img of this._warm.values()) img.removeAttribute('src');
    this._warm.clear();
    if (this.bookId) await api.closeBook(this.bookId).catch(() => {});
    this.host.replaceChildren();
  }
}
