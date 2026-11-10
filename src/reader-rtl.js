// reader-rtl.js — manga right-to-left (Yomikiru reader direction, MIT — reimplemented; pure).

/** Flip prev/next direction when RTL is on. */
export function rtlStep(dir, rtl) {
  return rtl ? -dir : dir;
}

/** Map horizontal key to page delta in scroll mode. */
export function rtlPageDelta(key, rtl) {
  if (key === 'ArrowRight') return rtl ? -1 : 1;
  if (key === 'ArrowLeft') return rtl ? 1 : -1;
  return 0;
}

/** Tap zone under RTL: visual left still means "back" in RTL mode (= next page). */
export function rtlTapZone(zone, rtl) {
  if (!rtl) return zone;
  if (zone < 0) return 1;
  if (zone > 0) return -1;
  return 0;
}
