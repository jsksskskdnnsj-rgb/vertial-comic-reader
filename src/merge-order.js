// merge-order.js - pure helpers for multi-source merge import (no DOM, no Tauri; unit-testable in node).
// Orders sources by the numeric range in their names so "xxx1-60" comes before "xxx60-90" and
// "xxx61-90" comes before "xxx100-120", and suggests a name for the merged book.
const ARCHIVE_EXT = /\.(zip|cbz)$/i;
const FULLWIDTH = /[\uff10-\uff19]/g;

export const baseName = (p) => String(p || '').replace(/[\\/]+$/, '').split(/[\\/]/).pop();
export const stemOf = (p) => baseName(p).replace(ARCHIVE_EXT, '').replace(FULLWIDTH, (c) => String.fromCharCode(c.charCodeAt(0) - 0xfee0));

function keyOf(p) {
  const stem = stemOf(p);
  const first = stem.search(/\d/);
  const prefix = (first < 0 ? stem : stem.slice(0, first)).toLowerCase().replace(/[\s_\-–—.·]+$/g, '');
  const nums = [...stem.matchAll(/\d+/g)].map((m) => parseInt(m[0], 10));
  return { prefix, nums, stem };
}

/** Stable ordering: series prefix, then every number in the name left to right
 *  (a missing number sorts first), then the full name as a last tie-break. */
export function compareSources(a, b) {
  const x = keyOf(a), y = keyOf(b);
  if (x.prefix !== y.prefix) return x.prefix < y.prefix ? -1 : 1;
  const n = Math.max(x.nums.length, y.nums.length);
  for (let i = 0; i < n; i++) {
    const u = x.nums[i], v = y.nums[i];
    if (u === undefined) return -1;
    if (v === undefined) return 1;
    if (u !== v) return u - v;
  }
  return x.stem < y.stem ? -1 : x.stem > y.stem ? 1 : 0;
}

export function orderByRange(paths) {
  return paths.map((p, i) => [p, i]).sort((a, b) => compareSources(a[0], b[0]) || a[1] - b[1]).map((e) => e[0]);
}

/** Suggested merged-book name: the common leading text of all names, minus trailing digits and
 *  separators. One source -> its own stem. Returns '' when nothing sensible is shared. */
export function suggestName(paths) {
  const stems = paths.map(stemOf).filter(Boolean);
  if (!stems.length) return '';
  if (stems.length === 1) return stems[0];
  let pre = stems[0];
  for (const s of stems) { let i = 0; while (i < pre.length && i < s.length && pre[i] === s[i]) i++; pre = pre.slice(0, i); }
  pre = pre.replace(/\d+$/, '').replace(/[\s_\-–—.·(（【[]+$/g, '').replace(/^[\s_\-–—.·)）】\]]+/, '');
  return pre.length >= 2 ? pre : '';
}
