// chapters.js - chapter modes as a pure NAVIGATION / DISPLAY layer. No DOM, no Tauri, no imports.
//
// A chapter ("episode") is { index (1-based), page (0-based first REAL page), len, title }.
// Nothing here touches layout.js, the strip, virtual pages, slices or cache keys: switching mode or N
// only changes labels, scrubber ticks and prev/next targets, so the vertical strip never reflows.
//
// Modes:  'none'   one book, no chapter UI at all
//         'folder' episodes = parent-folder changes inside the archive/folder (computed once by Rust)
//         'pages'  episode k covers pages (k-1)*N+1 .. k*N (last one may be shorter); computed here, instantly

export const MODES = ['none', 'folder', 'pages'];
export const DEFAULT_PAGES = 20;
export const MAX_PAGES = 9999;

const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
export const sanePages = (n) => clamp(Math.trunc(Number(n)) || DEFAULT_PAGES, 1, MAX_PAGES);

/** Virtual episodes from page count. Fewer than 2 episodes => [] (a single episode is not a chapter list). */
export function byPages(total, n) {
  if (!(total > 0)) return [];
  n = clamp(sanePages(n), 1, total);
  const count = Math.ceil(total / n);
  if (count < 2) return [];
  return Array.from({ length: count }, (_, k) => ({ index: k + 1, page: k * n, len: Math.min(n, total - k * n), title: '' }));
}

/** Effective list for a mode. `book` = { realPages, folderChapters } as returned by open_book. */
export function resolve(mode, n, book) {
  if (mode === 'pages') return byPages(book.realPages, n);
  if (mode === 'folder') return book.folderChapters ?? [];
  return [];
}

/** Episode containing `page` (binary search; null when there are no chapters). */
export function episodeAt(chs, page) {
  if (!chs.length) return null;
  let lo = 0, hi = chs.length - 1;
  while (lo < hi) { const mid = (lo + hi + 1) >> 1; if (chs[mid].page <= page) lo = mid; else hi = mid - 1; }
  return chs[lo];
}

/** "Previous episode": first press goes to the START of the current episode if you're past it, else to the previous one. */
export function prevTarget(chs, pos) {
  const e = episodeAt(chs, pos.page);
  if (!e) return null;
  if (pos.page > e.page || pos.frac > 0.02) return { page: e.page, frac: 0 };
  const p = chs[e.index - 2];
  return p ? { page: p.page, frac: 0 } : null;
}

export function nextTarget(chs, pos) {
  const e = episodeAt(chs, pos.page);
  const n = e && chs[e.index];                 // index is 1-based and contiguous, so chs[index] is the next one
  return n ? { page: n.page, frac: 0 } : null;
}

/** Scrubber tick positions as fractions 0..1, matching the thumb mapping page/(realPages-1). First episode has no tick. */
export function tickFractions(chs, realPages, trackPx, minGapPx = 4) {
  if (chs.length < 2 || realPages < 2) return [];
  const stride = Math.max(1, Math.ceil((minGapPx * chs.length) / Math.max(1, trackPx)));   // thin out dense ticks
  return chs.filter((e, i) => i > 0 && i % stride === 0).map(e => e.page / (realPages - 1));
}

/** Display text: "Episode 3 / 12", plus the folder name in folder mode. `t` is shell.js's translator. */
export function episodeLabel(t, chs, page, mode) {
  const e = episodeAt(chs, page);
  if (!e) return '';
  const base = t('episodeOf', { i: e.index, n: chs.length });
  const name = mode === 'folder' && e.title ? e.title.split('/').pop() : '';
  return name ? `${base} · ${name}` : base;
}

/** For lists (Recent reading, Library): one round trip for many books. -> [{index,total}|null, ...] aligned with `books`. */
export async function fetchEpisodeInfo(invoke, books) {
  if (!books.length) return [];
  return invoke('episode_info', { items: books.map(b => ({ bookId: b.id, page: b.page ?? 0 })) });
}
