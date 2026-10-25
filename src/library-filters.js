// library-filters.js - pure helpers for the library page (no imports, no DOM, no Tauri; unit-tested in Node).
// Reading status, filtering, status counts and the remembered library state (sort, status, folder, view mode).
export const SORTS = ['recent', 'added', 'name', 'title'];
export const STATUSES = ['all', 'unread', 'progress', 'finished'];
export const MODES = ['grid', 'list'];

/** Finished: the `finished` column when shelf_list exposes it (today it does not), otherwise "last page reached".
 *  A one-page book cannot be told apart from an unopened one by the page number, so without the column it counts as unread. */
export const isFinished = (b) => (b.finished != null ? !!b.finished : (b.page_count || 0) > 1 && (b.last_page || 0) >= b.page_count - 1);
/** 'unread' (still on page 1) | 'progress' | 'finished'. Same "started" rule as the Recent view: last_page > 0. */
export const statusOf = (b) => (isFinished(b) ? 'finished' : (b.last_page || 0) > 0 ? 'progress' : 'unread');

/** Everything except the status filter. o = { view, folder, query, bmCounts }. */
export function matchesBase(b, o) {
  if (o.view === 'recent' && !(b.last_page > 0)) return false;
  if (o.view === 'marked' && !((o.bmCounts?.get(b.id) || 0) > 0)) return false;
  if (o.folder === 'none' && b.folder_id != null) return false;
  if (typeof o.folder === 'number' && b.folder_id !== o.folder) return false;
  const q = String(o.query || '').trim().toLowerCase();
  if (q && !String(b.name || '').toLowerCase().includes(q)) return false;
  return true;
}
/** o additionally has `status` ('all' or missing = no status filter). */
export const filterBooks = (books, o) => books.filter((b) => matchesBase(b, o) && (!o.status || o.status === 'all' || statusOf(b) === o.status));
/** Counts for the status buttons: books that pass every OTHER filter, split by status. */
export function countStatuses(books, o) {
  const c = { all: 0, unread: 0, progress: 0, finished: 0 };
  for (const b of books) if (matchesBase(b, o)) { c.all++; c[statusOf(b)]++; }
  return c;
}

/** Initial library state. getPref(key, default) reads the saved prefs; `search` is location.search.
 *  ?sort= and ?status= override the saved values; anything invalid falls back to the default. */
export function readLibraryState(getPref, search) {
  const params = new URLSearchParams(search || '');
  const pick = (v, list, d) => (list.includes(v) ? v : d);
  const folderPref = getPref('libFolder', 'all');
  return {
    sort: pick(params.get('sort'), SORTS, pick(getPref('libSort', 'recent'), SORTS, 'recent')),
    status: pick(params.get('status'), STATUSES, pick(getPref('libStatus', 'all'), STATUSES, 'all')),
    folder: folderPref === 'all' || folderPref === 'none' ? folderPref : Number.isInteger(folderPref) && folderPref > 0 ? folderPref : 'all',
    mode: pick(getPref('libMode', 'grid'), MODES, 'grid'),
  };
}
/** A remembered folder that no longer exists falls back to 'all'. */
export const validFolder = (folder, folders) => (folder === 'all' || folder === 'none' || folders.some((f) => f.id === folder) ? folder : 'all');
