// library-presets.js — saved filter/sort snapshots (KoiPage preferences presets idea, GPL — reimplemented).
import { SORTS, STATUSES, MODES } from './library-filters.js';

const KEY = 'mh.lib.presets';

/** @typedef {{ id: string, name: string, sort: string, status: string, mode: string, folder: string|number, favoriteOnly?: boolean, tagId?: number|null }} LibPreset */

export function readPresets() {
  try {
    const raw = localStorage.getItem(KEY);
    const list = raw ? JSON.parse(raw) : [];
    return Array.isArray(list) ? list.filter((p) => p && p.id && p.name) : [];
  } catch {
    return [];
  }
}

export function writePresets(list) {
  try { localStorage.setItem(KEY, JSON.stringify(list.slice(0, 24))); } catch { /* private */ }
}

export function snapshotPreset(name, state) {
  const id = `${Date.now()}-${Math.random().toString(36).slice(2, 7)}`;
  const pick = (v, list, d) => (list.includes(v) ? v : d);
  return {
    id,
    name: String(name || '').trim().slice(0, 40) || '预设',
    sort: pick(state.sort, SORTS, 'recent'),
    status: pick(state.status, STATUSES, 'all'),
    mode: pick(state.mode, MODES, 'grid'),
    folder: state.folder ?? 'all',
    favoriteOnly: !!state.favoriteOnly,
    tagId: state.tagId ?? null,
  };
}

export function applyPreset(p, apply) {
  if (!p) return;
  apply({
    sort: p.sort,
    status: p.status,
    mode: p.mode,
    folder: p.folder,
    favoriteOnly: !!p.favoriteOnly,
    tagId: p.tagId ?? null,
  });
}
