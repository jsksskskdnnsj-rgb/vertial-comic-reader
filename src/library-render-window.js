// library-render-window.js — incremental library grid mounting (pure, no DOM).
// Idea aligned with KoiPage features/library/render-window.ts (GPL-3.0, not copied);
// MH reimplements the batch cap independently for MIT licensing.

export const LIBRARY_RENDER_BATCH = 48;

/** How many cards to mount for `total` rows; `requested` grows when the user reveals more. */
export function libraryRenderCount(total, requested = LIBRARY_RENDER_BATCH) {
  const batch = LIBRARY_RENDER_BATCH;
  const want = Number.isFinite(requested) && requested > 0 ? Math.max(batch, Math.floor(requested)) : batch;
  return Math.min(Math.max(0, total), want);
}

/** Stable key for remembering render count + scroll position per filter set. */
export function libraryScopeKey(parts) {
  const o = parts && typeof parts === 'object' ? parts : {};
  return JSON.stringify([
    o.view ?? '',
    o.filter ?? '',
    o.status ?? '',
    o.sort ?? '',
    o.query ?? '',
    o.mode ?? '',
    o.fav ?? '',
    o.tag ?? '',
  ]);
}
