// flip-adjacent.js — which real page indices to warm in flip mode (pure, no DOM).
// MH reader.js ImageWindow.warmAhead uses the same “decode ahead of the viewport” idea.
// Reading UX patterns informed by ComiGo assets/static/js/reader.js (MIT, Copyright (c) 2023 yumenaka)
// and Yomikiru manga reader preload flow (MIT, Copyright (c) 2023 mienaiyami); reimplemented here.

/** Default how many single-page steps to warm on each side of the current page. */
export const FLIP_WARM_STEPS = 2;

/**
 * @param {number} page — current real page index
 * @param {number} realPages
 * @param {boolean} spread — two-page spread mode
 * @param {number} [steps]
 * @returns {number[]} page indices to prefetch (never includes the visible page(s))
 */
export function flipWarmPages(page, realPages, spread, steps = FLIP_WARM_STEPS) {
  const n = Math.max(0, Math.floor(Number(realPages) || 0));
  if (n <= 0) return [];
  const cur = Math.max(0, Math.min(n - 1, Math.floor(Number(page) || 0)));
  const step = spread ? 2 : 1;
  const shown = new Set(spread ? [cur, Math.min(n - 1, cur + 1)] : [cur]);
  const out = new Set();
  for (let d = 1; d <= steps; d++) {
    const back = cur - d * step;
    if (back >= 0) out.add(back);
    const fwd = cur + d * step;
    if (fwd < n) out.add(fwd);
  }
  for (const i of shown) out.delete(i);
  return [...out].sort((a, b) => a - b);
}

/** Hit zone for tap-to-turn (OpenComic-style edge zones; reimplemented, no GPL code). */
export const FLIP_TAP_EDGE = 0.32;

/**
 * @param {number} clientX
 * @param {number} width — element clientWidth
 * @returns {-1 | 0 | 1} prev / ignore / next
 */
export function flipTapZone(clientX, width) {
  const w = Math.max(1, Number(width) || 1);
  const x = Number(clientX) || 0;
  const edge = FLIP_TAP_EDGE;
  if (x <= w * edge) return -1;
  if (x >= w * (1 - edge)) return 1;
  return 0;
}
