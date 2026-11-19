// layout.js - pure layout/position math for the vertical strip. No DOM, no Tauri.
//
// Terms
//   virtual page : one DOM box. A normal page, or one slice of a very tall image.
//   page / group : a REAL page as the user counts it (1..N). May span several virtual pages.
//
// Position model (stable across resize, zoom, width changes, re-slicing):
//   { page, frac }  page = real page index, frac in [0,1) = how far down the page's whole
//   height (all its slices, no gaps) the viewport top is.

const FALLBACK_AR = 1.5; // h/w used only when a page and its neighbors are all still unknown

/**
 * Aspect (h/w) for every virtual page. A known header wins. A run of unknown
 * pages keeps the previous known page, and pages before the first known size
 * take that first known size. 1.5 is only the last resort, so a long book does
 * not jump from a square placeholder to the real strip as each file decodes.
 *
 * `prev` is the aspect already on screen. Unknown pages keep that estimate, so
 * decoding one page does not restretch every later page that is still 0×0.
 * A different page count (a new book) ignores `prev`.
 */
export function fillAspects(dims, prev) {
  const n = dims?.length || 0;
  const sticky = prev && prev.length === n ? prev : null;
  const ar = new Array(n).fill(0);
  for (let i = 0; i < n; i++) {
    const [w, h] = dims[i] || [0, 0];
    if (w > 0 && h > 0) ar[i] = h / w;
    else if (sticky && sticky[i] > 0) ar[i] = sticky[i];
  }
  let carry = 0;
  for (let i = 0; i < n; i++) {
    if (ar[i] > 0) carry = ar[i];
    else if (carry > 0) ar[i] = carry;
  }
  let back = 0;
  for (let i = n - 1; i >= 0; i--) {
    if (ar[i] > 0) back = ar[i];
    else if (back > 0) ar[i] = back;
  }
  for (let i = 0; i < n; i++) if (!(ar[i] > 0)) ar[i] = FALLBACK_AR;
  return ar;
}

/**
 * Copy known real-page sizes onto virtual boxes that are still one box per page.
 * A sliced page (several boxes, one file) keeps the slicing from open. Returns
 * how many virtual boxes changed. A length mismatch means the page list itself
 * changed; the caller reopens instead of splicing boxes mid-session.
 */
export function applyRealDims(layout, realDims) {
  if (!layout || !Array.isArray(realDims)) return 0;
  if (realDims.length !== layout.realPages) return 0;
  let changed = 0;
  for (let g = 0; g < realDims.length; g++) {
    const a = layout.gFirst[g];
    const b = layout.gLast[g];
    if (a === undefined || a !== b) continue;
    const wh = realDims[g];
    if (!wh || !(wh[0] > 0) || !(wh[1] > 0)) continue;
    const cur = layout.dims[a];
    if (cur && cur[0] === wh[0] && cur[1] === wh[1]) continue;
    layout.setDim(a, wh[0], wh[1]);
    changed++;
  }
  return changed;
}

/** Page-width slider. 100% is the full pane. 20% is a narrow strip. */
export const WIDTH_PCT_MIN = 20;
export const WIDTH_PCT_MAX = 100;
/** Smallest column, so a tiny window cannot collapse the page to a sliver. */
export const COL_MIN_PX = 48;

/** Persisted width. Values already inside the range (including old 40–100 saves) pass through. */
export function clampWidthPct(pct, fallback = WIDTH_PCT_MAX) {
  const n = Math.round(Number(pct));
  if (!Number.isFinite(n)) return fallback;
  return Math.min(WIDTH_PCT_MAX, Math.max(WIDTH_PCT_MIN, n));
}

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
    this.ar = [];
    this._arDirty = true;
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
    this.ar = [];
    this._arDirty = true;
  }

  isGroupEnd(i) { return i === this.n - 1 || this.src[i + 1] !== this.src[i]; }

  _rebuildAr() {
    this.ar = fillAspects(this.dims, this.ar);
    this._arDirty = false;
  }

  _sliceHeight(i, colW) {
    const ar = this.ar[i] > 0 ? this.ar[i] : FALLBACK_AR;
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
  // the side bars tuck away. widthPct still scales that pane (the 20–100 slider).
  // spread: pairs of real pages side by side (vertical scroll only).
  compute({ viewW, widthPct = 100, maxW = 1000, zoom = 1, gap = 0, endPad = 0, spread = false, spreadGap = 8 }) {
    if (this._arDirty) this._rebuildAr();
    const scaled = Math.round(viewW * widthPct / 100);
    const base = Number.isFinite(maxW) ? Math.min(scaled, maxW) : scaled;
    this.colW = Math.max(COL_MIN_PX, Math.round(base * zoom));
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
    const halfW = Math.max(1, Math.floor((pairW - this.spreadGap) / 2));
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

  setDim(i, w, h) {
    this.dims[i] = [w, h];
    this._arDirty = true;
  }

  /**
   * Change one virtual page's box height and slide every page after it by the
   * same amount. Gaps already stored between tops stay put. Spread rows share
   * a height, so that mode returns null and the caller recomputes the row.
   * @returns {number | null} pixel delta, or null when a slide is not valid
   */
  shiftHeight(i, nextH) {
    if (this.spread || i < 0 || i >= this.n) return null;
    const h = Math.max(1, Math.round(Number(nextH) || 0));
    const prev = this.vH[i] || 0;
    if (prev === h) return 0;
    const delta = h - prev;
    this.vH[i] = h;
    for (let j = i + 1; j < this.n; j++) this.vTop[j] += delta;
    this.contentH += delta;
    this.sizerH += delta;
    return delta;
  }

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
