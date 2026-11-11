// Caps how many page bitmaps the reader keeps hot on very long books.
// Small books keep the existing balanced / saver numbers.

export function tunePerf(perf, pageCount) {
  const src = perf || {};
  const next = { ...src };
  const n = Number(pageCount) || 0;
  if (n >= 4000) {
    next.warm = Math.min(src.warm ?? 0, 1);
    next.maxMounted = Math.min(src.maxMounted ?? 8, 8);
    next.maxAhead = Math.min(src.maxAhead ?? 6, 6);
    next.maxBehind = Math.min(src.maxBehind ?? 2, 2);
  } else if (n >= 1500) {
    next.warm = Math.min(src.warm ?? 0, 2);
    next.maxMounted = Math.min(src.maxMounted ?? 12, 12);
    next.maxAhead = Math.min(src.maxAhead ?? 8, 8);
  }
  return next;
}

/** How many shelf covers to start decoding after the rows already on screen. */
export function coverWarmBatch(librarySize, batch = 20) {
  const n = Number(librarySize) || 0;
  const cap = Math.max(0, Math.floor(Number(batch) || 0));
  if (n >= 5000) return Math.min(cap, 8);
  if (n >= 2000) return Math.min(cap, 12);
  return cap;
}
