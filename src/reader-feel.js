// reader-feel.js — pure input/physics helpers for scroll glide and page-flip gestures.
// No DOM. The reader keeps its own pixels; this module only decides distances and timings.

/** Last pixels of a key-scroll coast ease to a stop instead of cutting off. */
export const KEY_SOFT_LAND_PX = 140;

/** Isolated pixel deltas at least this large are mouse-wheel notches, not trackpad ticks. */
export const WHEEL_NOTCH_PX = 40;
/** Trackpad packets arrive closer together than this. A notch is a single spaced event. */
export const WHEEL_NOTCH_GAP_MS = 48;

export const FLIP_WHEEL_TRACK_PX = 88;
export const FLIP_WHEEL_DECAY_MS = 180;
export const FLIP_WHEEL_NOTCH_LOCK_MS = 150;
export const FLIP_WHEEL_TRACK_LOCK_MS = 280;

export const FLIP_SWIPE_DRAG_PX = 56;
export const FLIP_SWIPE_FLING_PX = 22;
export const FLIP_SWIPE_FLING_V = 0.45;

/**
 * 'notch' — one mouse-wheel detent (or line/page delta). The reader should glide it.
 * 'trackpad' — a high-rate pixel stream. Leave it to the platform so inertia survives.
 *
 * @param {{ deltaX?: number, deltaY?: number, deltaMode?: number }} e
 * @param {number} prevT — time of the previous wheel sample (0 if none)
 * @param {number} now
 * @returns {'notch' | 'trackpad'}
 */
export function classifyWheel(e, prevT = 0, now = 0) {
  const mode = e?.deltaMode || 0;
  if (mode === 1 || mode === 2) return 'notch';
  const dy = Number(e?.deltaY) || 0;
  const dx = Number(e?.deltaX) || 0;
  if (dy !== Math.trunc(dy) || dx !== Math.trunc(dx)) return 'trackpad';
  if (dx && dy && Math.abs(dx) > 8 && Math.abs(dy) > 8) return 'trackpad';
  const big = Math.abs(dy) >= WHEEL_NOTCH_PX || Math.abs(dx) >= WHEEL_NOTCH_PX;
  const dt = prevT ? now - prevT : 1000;
  if (big && dt >= WHEEL_NOTCH_GAP_MS) return 'notch';
  return 'trackpad';
}

/**
 * Exponential chase. Stacked notches move the target; the position follows
 * without restarting an ease (which stutters on a fast spin) and without overshoot.
 * Snaps when the remainder is sub-pixel.
 */
export function glideChase(current, target, dtMs, tauMs) {
  const tau = Math.max(1, Number(tauMs) || 1);
  const dt = Math.max(0, Number(dtMs) || 0);
  if (dt === 0) return current;
  const next = current + (target - current) * (1 - Math.exp(-dt / tau));
  if (Math.abs(target - next) < 0.35) return target;
  return next;
}

/**
 * Tau in ms. Short travel (one notch) settles quickly; a long page jump
 * gets a longer tau so it eases instead of blinking across the strip.
 * Capped so a very tall page still arrives.
 */
export function glideTauMs(distancePx) {
  const d = Math.abs(Number(distancePx) || 0);
  const tau = (d / 4200) * 1000;
  if (!Number.isFinite(tau)) return 70;
  return Math.max(70, Math.min(240, Math.round(tau)));
}

/**
 * Chromium `ScrollableArea::PageStep`: max(7/8 of the scrollport,
 * scrollport − 40px). Tall reader viewports therefore move almost a full
 * screen and keep a 40px overlap. Space / PageDown / PageUp use this.
 * Arrow nudges do not.
 */
export const KEY_PAGE_FRACTION = 0.875;
export const KEY_PAGE_OVERLAP_PX = 40;
/** One page step. Inside the 150–250ms range a browser uses for the gesture. */
export const KEY_PAGE_DURATION_MS = 200;

export function pageKeyDistance(viewH) {
  const h = Math.max(1, Number(viewH) || 1);
  const step = Math.max(h * KEY_PAGE_FRACTION, h - KEY_PAGE_OVERLAP_PX);
  return Math.max(1, Math.round(step));
}

export function easeOutCubic(t) {
  const u = Math.min(1, Math.max(0, Number(t) || 0));
  return 1 - (1 - u) ** 3;
}

/**
 * Cubic from `from` to `to` that arrives at rest.
 * v0 ≈ 0 (a fresh press) is ease-out: the slope starts at 3× the average
 * and ends at 0, over `duration` ms. A chain passes the current velocity
 * so the new segment does not drop to 0 and start again.
 */
export function pageScrollSegment({ from, to, v0 = 0, duration = KEY_PAGE_DURATION_MS }) {
  const dist = to - from;
  const D = Math.max(1, Number(duration) || 1);
  if (!dist) return { a: from, b: 0, c: 0, d: 0, D, to, from };
  const easeV = (3 * dist) / D;
  let b = easeV;
  if (v0 && Math.sign(v0) === Math.sign(dist)) {
    const mag = Math.abs(v0);
    const easeMag = Math.abs(easeV);
    // Keep speed only while it is still the same gesture. A near-stop uses
    // the ease-out impulse (fast), not a crawl away from rest.
    if (mag >= easeMag * 0.45 && mag <= easeMag * 1.02) b = Math.sign(dist) * mag;
  }
  const d = (b * D - 2 * dist) / (D ** 3);
  const c = -b / (2 * D) - 1.5 * d * D;
  return { a: from, b, c, d, D, to, from };
}

export function samplePageScroll(seg, elapsedMs) {
  if (!seg) return { y: 0, v: 0, done: true };
  const t = Math.min(seg.D, Math.max(0, Number(elapsedMs) || 0));
  let y = seg.a + seg.b * t + seg.c * t * t + seg.d * t * t * t;
  const v = seg.b + 2 * seg.c * t + 3 * seg.d * t * t;
  if (t >= seg.D - 0.001) return { y: seg.to, v: 0, done: true };
  const lo = Math.min(seg.a, seg.to);
  const hi = Math.max(seg.a, seg.to);
  if (y < lo) y = lo;
  else if (y > hi) y = hi;
  return { y, v, done: false };
}

/**
 * Add `delta` px onto an in-flight page scroll. Time scales with the new
 * distance so the pace stays one viewport-step per KEY_PAGE_DURATION_MS.
 * The sample at `elapsedMs` is the start of the new segment — position and
 * (when it still matters) velocity carry over.
 */
export function chainPageScroll(seg, elapsedMs, delta, page, duration = KEY_PAGE_DURATION_MS) {
  const now = samplePageScroll(seg, elapsedMs);
  const target = (seg?.to ?? now.y) + delta;
  const dist = target - now.y;
  const pageAbs = Math.max(1, Math.abs(page) || 1);
  const pages = Math.max(Math.abs(dist) / pageAbs, 0.35);
  const D = Math.max(1, duration * pages);
  return pageScrollSegment({ from: now.y, to: target, v0: now.v, duration: D });
}

function clampScroll(v, maxY) {
  return Math.min(Math.max(0, maxY), Math.max(0, v));
}

/** Fresh Space / PageDown. `to` may be shorter than a full page at the ends. */
export function createPageKey({ y, to, dir, page, duration = KEY_PAGE_DURATION_MS, now = 0 }) {
  const D = Math.max(1, duration);
  return {
    mode: 'ease',
    dir,
    page,
    duration: D,
    y,
    seg: pageScrollSegment({ from: y, to, v0: 0, duration: D }),
    segT: now,
  };
}

/**
 * Another Space while the first is still moving — a second tap or an OS
 * key-repeat. Both add one page step and keep the current speed. A hold is
 * just this, once per repeat, until the key comes up.
 */
export function extendPageKey(state, now, maxY) {
  const elapsed = Math.max(0, now - state.segT);
  const sample = samplePageScroll(state.seg, elapsed);
  const delta = state.dir * state.page;
  let seg = chainPageScroll(state.seg, elapsed, delta, state.page, state.duration);
  const to = clampScroll(seg.to, maxY);
  if (to !== seg.to) {
    const dist = to - sample.y;
    const pages = Math.abs(dist) / Math.max(1, state.page);
    seg = pageScrollSegment({
      from: sample.y,
      to,
      v0: sample.v,
      duration: Math.max(1, state.duration * Math.max(pages, 0.35)),
    });
  }
  return { ...state, mode: 'ease', seg, segT: now, y: sample.y };
}

/** One animation frame of the ease-out. Done when the chained target is reached. */
export function stepPageKey(state, now, maxY) {
  const max = Math.max(0, Number(maxY) || 0);
  const sample = samplePageScroll(state.seg, now - state.segT);
  const y = clampScroll(sample.y, max);
  const atEnd = state.dir > 0 ? y >= max - 0.5 : y <= 0.5;
  if (atEnd || sample.done) return { y, state: { ...state, y }, done: true };
  return { y, state: { ...state, y }, done: false };
}

/**
 * Speed during the release coast of an arrow-key nudge.
 * Full speed until the last `softPx`, then a linear ease down to 0
 * so a tap and a hold share one top speed and neither stops dead.
 */
export function keyReleaseSpeed(pxPerSec, remainPx, softPx = KEY_SOFT_LAND_PX) {
  const v = Math.max(0, Number(pxPerSec) || 0);
  const remain = Math.max(0, Number(remainPx) || 0);
  if (remain <= 0 || v <= 0) return 0;
  const soft = Math.max(1, Number(softPx) || 1);
  if (remain >= soft) return v;
  return v * (remain / soft);
}

/**
 * 1 when this virtual page's top edge sits exactly on an earlier page
 * (gap 0, including slices of one tall image). The strip grows this box
 * 1px upward so the later page — already painted on top — covers the join.
 * A moving compositor otherwise flashes the page fill between abutting boxes.
 * No z-index: a stacking context per page would make the scroll heavier.
 * @returns {0 | 1}
 */
export function pageSeamCover(layout, index) {
  const n = layout?.n | 0;
  if (!n || index <= 0 || index >= n) return 0;
  const top = layout.vTop[index];
  const left = layout.vLeft?.[index] ?? layout.left ?? 0;
  const width = layout.vW?.[index] || layout.colW || 0;
  const right = left + width;
  const stop = Math.max(0, index - 48);
  for (let j = index - 1; j >= stop; j--) {
    const bottom = layout.vTop[j] + layout.vH[j];
    if (bottom !== top) continue;
    const l2 = layout.vLeft?.[j] ?? layout.left ?? 0;
    const w2 = layout.vW?.[j] || layout.colW || 0;
    if (right > l2 && left < l2 + w2) return 1;
  }
  return 0;
}

export function createFlipWheelState() {
  return { acc: 0, t: 0, lockUntil: 0 };
}

/**
 * One flip step from a wheel/trackpad sample.
 * Positive dir advances the page index (wheel down / scroll right).
 * A notch turns immediately. A trackpad stream must cross a threshold,
 * then locks so one flick is one page.
 *
 * @returns {{ state: { acc: number, t: number, lockUntil: number }, dir: -1 | 0 | 1 }}
 */
export function flipWheelAdvance(state, e, now) {
  const prev = state || createFlipWheelState();
  const dy = Number(e?.deltaY) || 0;
  const dx = Number(e?.deltaX) || 0;
  const axis = Math.abs(dy) >= Math.abs(dx) ? dy : dx;
  if (!axis || now < prev.lockUntil) {
    return { state: { acc: 0, t: now, lockUntil: prev.lockUntil }, dir: 0 };
  }
  const discrete = classifyWheel(e, prev.t, now) === 'notch';
  if (discrete) {
    const dir = /** @type {-1 | 1} */ (Math.sign(axis));
    return { state: { acc: 0, t: now, lockUntil: now + FLIP_WHEEL_NOTCH_LOCK_MS }, dir };
  }
  let acc = now - prev.t > FLIP_WHEEL_DECAY_MS ? 0 : prev.acc;
  acc += axis;
  if (Math.abs(acc) < FLIP_WHEEL_TRACK_PX) {
    return { state: { acc, t: now, lockUntil: 0 }, dir: 0 };
  }
  const dir = /** @type {-1 | 1} */ (Math.sign(acc));
  return { state: { acc: 0, t: now, lockUntil: now + FLIP_WHEEL_TRACK_LOCK_MS }, dir };
}

/**
 * Horizontal swipe / fling on the flip stage.
 * LTR: swipe left advances. RTL: swipe right advances (page-turn direction).
 * Mostly-vertical drags are ignored so a shaky finger does not turn the page.
 *
 * @returns {-1 | 0 | 1} index delta
 */
export function flipSwipeDir(dx, dy, dtMs, rtl = false) {
  const x = Number(dx) || 0;
  const y = Number(dy) || 0;
  const adx = Math.abs(x);
  const ady = Math.abs(y);
  if (adx < 18) return 0;
  if (ady > adx * 1.15) return 0;
  const dt = Math.max(1, Number(dtMs) || 1);
  const fling = adx / dt >= FLIP_SWIPE_FLING_V && adx >= FLIP_SWIPE_FLING_PX;
  const drag = adx >= FLIP_SWIPE_DRAG_PX;
  if (!fling && !drag) return 0;
  const towardLeft = x < 0;
  const forward = rtl ? !towardLeft : towardLeft;
  return forward ? 1 : -1;
}

/** Screen-space offset for a settle animation. +1 enters from the right. */
export function flipVisualSign(indexDir, rtl = false) {
  const d = Math.sign(Number(indexDir) || 0);
  if (!d) return 0;
  return rtl ? -d : d;
}
