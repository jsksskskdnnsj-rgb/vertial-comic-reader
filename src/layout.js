// layout.js - pure layout/position math for the vertical strip. No DOM, no Tauri.
//
// Terms
//   virtual page : one DOM box. A normal page, or one slice of a very tall image.
//   page / group : a REAL page as the user counts it (1..N). May span several virtual pages.
//
// Position model (stable across resize, zoom, width changes, re-slicing):
//   { page, frac }  page = real page index, frac in [0,1) = how far down the page's whole
//   height (all its slices, no gaps) the viewport top is.

const FALLBACK_AR = 1.5; // h/w used while a page's true size is unknown (dims 0,0)

export class Layout {
  constructor() {
    this.n = 0;
    this.dims = [];                 // [[w,h], ...] per virtual page (0,0 = unknown)
    this.src = [];                  // virtual index -> real page index (non-decreasing, contiguous)
    this.gFirst = []; this.gLast = []; // real page -> first/last virtual index
    this.vTop = new Float64Array(0); // top (px) of each virtual page inside the sizer
    this.vH = new Float64Array(0);   // height (px), integer
    this.vLeft = new Float64Array(0);
    this.vW = new Float64Array(0);
    this.colW = 0; this.left = 0; this.sizerW = 0; this.contentH = 0; this.sizerH = 0; this.gap = 0;
    this.spread = false;
    this.spreadGap = 8;
  }

  get realPages() { return this.gFirst.length; }

  setBook(dims, src) {
    this.dims = dims.map(d => [d[0], d[1]]);
    this.src = src;
    this.n = dims.length;
    this.gFirst = []; this.gLast = [];
    for (let i = 0; i < this.n; i++) {
      const g = src[i];
      if (this.gFirst[g] === undefined) this.gFirst[g] = i;
      this.gLast[g] = i;
    }
    this.vTop = new Float64Array(this.n);
    this.vH = new Float64Array(this.n);
    this.vLeft = new Float64Array(this.n);
    this.vW = new Float64Array(this.n);
  }

  isGroupEnd(i) { return i === this.n - 1 || this.src[i + 1] !== this.src[i]; }

  _sliceHeight(i, colW) {
    const [w, h] = this.dims[i];
    const ar = w > 0 && h > 0 ? h / w : FALLBACK_AR;
    return Math.max(1, Math.round(colW * ar));
  }

  _groupStackHeight(g, colW) {
    let h = 0;
    const a = this.gFirst[g], b = this.gLast[g];
    if (a === undefined) return 0;
    for (let i = a; i <= b; i++) h += this._sliceHeight(i, colW);
    return h;
  }

  _placeGroupColumn(g, colW, xLeft, yStart, rowH) {
    const a = this.gFirst[g], b = this.gLast[g];
    if (a === undefined) return;
    const stackH = this._groupStackHeight(g, colW);
    let y = yStart + Math.round((rowH - stackH) / 2);
    for (let i = a; i <= b; i++) {
      const H = this._sliceHeight(i, colW);
      this.vTop[i] = y;
      this.vH[i] = H;
      this.vLeft[i] = xLeft;
      this.vW[i] = colW;
      y += H;
    }
  }

  // Recompute all tops/heights. Heights are rounded to integers HERE and the DOM is given
  // exactly these numbers (absolute positioning), so JS math and the browser never disagree.
  // maxW defaults to 1000 so flip mode (which omits it) stays as before.
  // Scroll mode passes Infinity and the column tracks the pane, including after
  // the side bars tuck away. widthPct still scales that pane (the 40–100 slider).
  // spread: pairs of real pages side by side (vertical scroll only).
  compute({ viewW, widthPct = 100, maxW = 1000, zoom = 1, gap = 0, endPad = 0, spread = false, spreadGap = 8 }) {
    const scaled = Math.round(viewW * widthPct / 100);
    const base = Number.isFinite(maxW) ? Math.min(scaled, maxW) : scaled;
    this.colW = Math.max(120, Math.round(base * zoom));
    this.gap = gap;
    this.spread = !!spread;
    this.spreadGap = Math.max(0, Math.round(spreadGap));

    if (!this.spread) {
      this.sizerW = Math.max(this.colW, viewW);
      this.left = Math.round((this.sizerW - this.colW) / 2);
      let y = 0;
      for (let i = 0; i < this.n; i++) {
        const H = this._sliceHeight(i, this.colW);
        this.vTop[i] = y;
        this.vH[i] = H;
        this.vLeft[i] = this.left;
        this.vW[i] = this.colW;
        y += H + (this.isGroupEnd(i) && i < this.n - 1 ? gap : 0);
      }
      this.contentH = this.n ? this.vTop[this.n - 1] + this.vH[this.n - 1] : 0;
      this.sizerH = this.contentH + endPad;
      return;
    }

    const pairW = this.colW;
    const halfW = Math.max(60, Math.floor((pairW - this.spreadGap) / 2));
    this.sizerW = Math.max(pairW, viewW);
    this.left = Math.round((this.sizerW - pairW) / 2);
    const xL = this.left;
    const xR = this.left + halfW + this.spreadGap;
    let y = 0;
    for (let g = 0; g < this.realPages; g += 2) {
      const rowH = Math.max(this._groupStackHeight(g, halfW), g + 1 < this.realPages ? this._groupStackHeight(g + 1, halfW) : 0);
      this._placeGroupColumn(g, halfW, xL, y, rowH);
      if (g + 1 < this.realPages) this._placeGroupColumn(g + 1, halfW, xR, y, rowH);
      y += rowH + (g + 2 < this.realPages ? gap : 0);
    }
    this.contentH = y;
    this.sizerH = this.contentH + endPad;
  }

  setDim(i, w, h) { this.dims[i] = [w, h]; }

  // Largest i with vTop[i] <= y  (y inside a gap -> the page above it)
  findVirtual(y) {
    let lo = 0, hi = this.n - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (this.vTop[mid] <= y) lo = mid; else hi = mid - 1;
    }
    return lo;
  }

  groupBounds(g) {
    const a = this.gFirst[g], b = this.gLast[g];
    const top = this.vTop[a], bottom = this.vTop[b] + this.vH[b];
    return { top, bottom, height: Math.max(1, bottom - top) };
  }

  /** Align a real page index to the first page of its spread pair. */
  spreadStart(page) {
    const p = Math.max(0, Math.floor(Number(page) || 0));
    return p - (p % 2);
  }

  // scrollTop (or any y) -> { page, frac }
  getPosition(y) {
    if (!this.n) return { page: 0, frac: 0 };
    const v = this.findVirtual(Math.max(0, y));
    const g = this.src[v];
    const { top, height } = this.groupBounds(g);
    return { page: g, frac: Math.min(0.9999, Math.max(0, (y - top) / height)) };
  }

  // { page, frac } -> scrollTop
  posToY({ page, frac }) {
    if (!this.n) return 0;
    const g = Math.min(Math.max(0, page), this.realPages - 1);
    const { top, height } = this.groupBounds(g);
    return Math.round(top + frac * height);
  }
}
