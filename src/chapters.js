// chapters.js - chapter (episode) math for the v2 reader, aligned with the REAL
// backend contract (verified against src-tauri/src/chapters.rs + model.rs):
//   open_book returns chapters/folderChapters as Episode { index, page, len, title }
//   episode_info is ONE batch call: invoke('episode_info', { items: [{bookId, page}] })
//     -> [{ index, total } | null, ...] aligned with the request.
// A chapter here is { start, end, index, title } with end exclusive, in REAL pages.
// (The repaired package's version called episode_info per-book with a wrong
// shape and computed folder-episode `end` as the book total for every chapter;
// both are fixed here. prev/next accept a page number OR a {page, frac} pos.)
export const DEFAULT_PAGES = 30;
export const sanePages = (v) => Math.max(1, Math.min(9999, Math.round(Number(v) || DEFAULT_PAGES)));

export function resolve(mode = 'folder', pages = DEFAULT_PAGES, book = {}) {
  const n = Math.max(0, book?.realPages || book?.src?.length || 0);
  if (!n || mode === 'none') return [];
  if (mode === 'folder' && Array.isArray(book.folderChapters) && book.folderChapters.length) {
    return book.folderChapters
      .map((c, i) => {
        const start = Math.max(0, c.page ?? c.start ?? 0);
        const end = Math.min(n, c.end ?? (start + (c.len ?? n - start)));
        return { start, end, index: c.index ?? i + 1, title: c.title || '' };
      })
      .filter((c) => c.start < n && c.end > c.start);
  }
  if (mode === 'folder') return []; // no sub-folders in this book -> no chapters
  const size = sanePages(pages);
  const count = Math.ceil(n / size);
  if (count < 2) return []; // a single episode is not a chapter list
  const out = [];
  for (let start = 0, i = 1; start < n; start += size, i++) {
    out.push({ start, end: Math.min(n, start + size), index: i, title: '' });
  }
  return out;
}

const pageOf = (pos) => (typeof pos === 'number' ? pos : Math.floor(Number(pos?.page) || 0));

export function episodeAt(chs, pos) {
  const page = pageOf(pos);
  return chs.findIndex((c) => page >= c.start && page < c.end);
}

export function prevTarget(chs, pos) {
  const i = episodeAt(chs, pos);
  if (i < 0) return null;
  const page = pageOf(pos);
  const frac = typeof pos === 'object' && pos ? Number(pos.frac) || 0 : 0;
  // Past the very start of the current episode -> go to its start first.
  if (page > chs[i].start || frac > 0.02) return { page: chs[i].start, frac: 0 };
  return i > 0 ? { page: chs[i - 1].start, frac: 0 } : null;
}

export function nextTarget(chs, pos) {
  const i = episodeAt(chs, pos);
  return i >= 0 && i < chs.length - 1 ? { page: chs[i + 1].start, frac: 0 } : null;
}

/** Scrubber ticks: one per episode boundary (the first episode needs none). */
export function tickFractions(chs, total) {
  if (!total || total < 2 || chs.length < 2) return [];
  return chs.slice(1).map((c) => c.start / (total - 1)).filter((f) => f > 0 && f < 1);
}

export function episodeLabel(t, chs, pos, mode) {
  const i = episodeAt(chs, pos);
  if (i < 0) return '';
  const c = chs[i];
  const base = t('common.episodeOf', { i: c.index ?? i + 1, n: chs.length });
  const name = mode === 'folder' && c.title ? c.title.split('/').pop() : '';
  return name ? `${base} · ${name}` : base;
}

/** Batch episode lookup for book lists. -> [{index,total}|null, ...] aligned. */
export async function fetchEpisodeInfo(invoke, books) {
  const list = books || [];
  if (!list.length) return [];
  try {
    const rows = await invoke('episode_info', {
      items: list.map((b) => ({ bookId: b.id, page: Math.max(0, Math.floor(Number(b.page) || 0)) })),
    });
    return list.map((_, i) => rows?.[i] ?? null);
  } catch {
    return list.map(() => null);
  }
}
