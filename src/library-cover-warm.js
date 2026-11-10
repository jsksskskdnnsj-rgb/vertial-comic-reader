// library-cover-warm.js — pick book ids whose covers should be warmed after a library paint.
// Yomikiru warms library thumbnails ahead of the visible window (MIT, Copyright (c) 2023 mienaiyami);
// ComiGo batches cover loads on the shelf (MIT, Copyright (c) 2023 yumenaka). MH uses lightweight Image() fetches.

export const LIBRARY_COVER_WARM_BATCH = 20;

/**
 * @param {Array<{id:number}>} books — full filtered list, in display order
 * @param {number} mounted — how many rows are already on screen
 * @param {number} [batch]
 */
export function libraryCoversToWarm(books, mounted, batch = LIBRARY_COVER_WARM_BATCH) {
  const list = Array.isArray(books) ? books : [];
  const start = Math.max(0, Math.floor(Number(mounted) || 0));
  const n = Math.max(0, Math.floor(Number(batch) || 0));
  return list.slice(start, start + n).map((b) => b?.id).filter((id) => id != null);
}
