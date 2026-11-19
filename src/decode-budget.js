// Caps how many page bitmaps the reader keeps hot on very long books.
// Small books keep the existing balanced / saver numbers.

export function tunePerf(perf, pageCount) {
  const src = perf || {};
  const next = { ...src };
  const n = Number(pageCount) || 0;
  // The viewport is mounted separately. Shrinking warm / ahead used to leave
  // a long book as empty slabs past the first screen, so only the bitmap cap
  // tightens here.
  if (n >= 4000) next.maxMounted = Math.min(src.maxMounted ?? 8, 14);
  else if (n >= 1500) next.maxMounted = Math.min(src.maxMounted ?? 12, 16);
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
