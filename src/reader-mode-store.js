// reader-mode-store.js — default + per-book reading view (scroll vs flip); pure, no DOM.

export const VIEW_MODES = ['scroll', 'flip'];

/** Map legacy reader-config value `page` → flip. */
export function normalizeViewMode(mode) {
  const m = String(mode || '').toLowerCase();
  if (m === 'flip' || m === 'page') return 'flip';
  return 'scroll';
}

function bookModeKey(bookId) {
  return `bookView.${Math.floor(Number(bookId) || 0)}`;
}

/**
 * @param {number} bookId
 * @param {string|null|undefined} urlOverride — `view` query: scroll | flip
 * @param {() => string} getDefault — settings default (scroll | flip)
 * @param {{ get: Function, set: Function }} prefs — mh.pref store
 */
export function resolveBookViewMode(bookId, urlOverride, getDefault, prefs) {
  const fromUrl = urlOverride != null && urlOverride !== '' ? normalizeViewMode(urlOverride) : null;
  if (fromUrl) return fromUrl;
  const saved = prefs.get(bookModeKey(bookId), null);
  if (saved != null) return normalizeViewMode(saved);
  return normalizeViewMode(getDefault());
}

export function persistBookViewMode(bookId, mode, prefs) {
  const m = normalizeViewMode(mode);
  if (!VIEW_MODES.includes(m)) return;
  prefs.set(bookModeKey(bookId), m);
}

/** Build reader.html URL; optional `view` forces scroll/flip for this open only. */
export function readerOpenHref(bookId, view = null) {
  const id = Math.floor(Number(bookId) || 0);
  if (!id) return 'reader.html';
  const q = new URLSearchParams();
  q.set('book', String(id));
  if (view != null && view !== '') q.set('view', normalizeViewMode(view));
  return `reader.html?${q.toString()}`;
}
