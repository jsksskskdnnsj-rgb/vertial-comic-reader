// Loose "maybe the same book" grouping for folder names.
// A pair is flagged for review when the series bases share a run of at least
// 3 characters that is not only a volume word, a counter, or digits.
// Confident auto-merge stays in Rust (merge.rs). This path is the fast hint.
import { parseComicName } from './name-parse.js';

const GENERIC = ['volume', 'chapter', 'episode', 'chap', 'vol', 'part', 'ep', 'ch', 'jpeg', 'jpg', 'png', 'webp', 'gif', 'avif', 'cbz', 'cbr', 'zip', 'rar', 'pdf', '7z'];
const GENERIC_CHAR = /[0-9第话話章卷集回部期节節篇册冊零〇一二两三四五六七八九十百千]/;

function tokenAt(chars, i, tok) {
  if (i + tok.length > chars.length) return false;
  for (let k = 0; k < tok.length; k++) if (chars[i + k] !== tok[k]) return false;
  return true;
}

export function isOnlyGeneric(text) {
  const chars = [...String(text ?? '')];
  if (!chars.length) return true;
  const reach = new Array(chars.length + 1).fill(false);
  reach[0] = true;
  for (let i = 0; i < chars.length; i++) {
    if (!reach[i]) continue;
    if (GENERIC_CHAR.test(chars[i])) reach[i + 1] = true;
    for (const tok of GENERIC) {
      if (tokenAt(chars, i, tok)) reach[i + tok.length] = true;
    }
  }
  return reach[chars.length];
}

function pack3(a, b, c) {
  return (BigInt(a.codePointAt(0)) << 42n) | (BigInt(b.codePointAt(0)) << 21n) | BigInt(c.codePointAt(0));
}

function gramsOf(chars) {
  const out = [];
  const seen = new Set();
  const n = Math.min(chars.length, 96);
  if (n < 3) return out;
  for (let i = 0; i <= n - 3; i++) {
    const w = chars.slice(i, i + 3);
    if (isOnlyGeneric(w.join(''))) continue;
    const key = pack3(w[0], w[1], w[2]);
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(key);
  }
  return out;
}

function hasMeaningfulCommon(av, bv) {
  const a = av.slice(0, 96);
  const b = bv.slice(0, 96);
  if (a.length < 3 || b.length < 3) return false;
  let prev = new Array(b.length + 1).fill(0);
  let cur = new Array(b.length + 1).fill(0);
  for (let i = 0; i < a.length; i++) {
    for (let j = 0; j < b.length; j++) {
      if (a[i] !== b[j]) { cur[j + 1] = 0; continue; }
      const len = prev[j] + 1;
      cur[j + 1] = len;
      if (len < 3) continue;
      const extendsRun = i + 1 < a.length && j + 1 < b.length && a[i + 1] === b[j + 1];
      if (extendsRun) continue;
      const start = i + 1 - len;
      if (!isOnlyGeneric(a.slice(start, i + 1).join(''))) return true;
    }
    const swap = prev; prev = cur; cur = swap;
    cur.fill(0);
  }
  return false;
}

function longestCommon(a, b) {
  const av = [...a];
  const bv = [...b];
  let prev = new Array(bv.length + 1).fill(0);
  let cur = new Array(bv.length + 1).fill(0);
  let best = 0;
  let end = 0;
  for (let i = 0; i < av.length; i++) {
    for (let j = 0; j < bv.length; j++) {
      if (av[i] !== bv[j]) { cur[j + 1] = 0; continue; }
      const len = prev[j] + 1;
      cur[j + 1] = len;
      if (len > best) { best = len; end = i + 1; }
    }
    const swap = prev; prev = cur; cur = swap;
    cur.fill(0);
  }
  return av.slice(end - best, end).join('');
}

function suggest(bases) {
  if (!bases.length) return '';
  let stem = bases[0];
  for (let i = 1; i < bases.length; i++) {
    stem = longestCommon(stem, bases[i]).trim();
    if ([...stem].length < 3) break;
  }
  stem = stem.replace(/^[\s_\-–—.·]+|[\s_\-–—.·]+$/g, '');
  if ([...stem].length >= 3 && !isOnlyGeneric(stem) && bases.some((d) => [...d].length > [...stem].length)) return stem;
  return bases.slice().sort((a, b) => [...a].length - [...b].length || (a < b ? -1 : 1))[0] || '';
}

/**
 * names: folder or file display names.
 * Returns review groups only (never an auto-merge decision).
 * @returns {{title:string, members:string[]}[]}
 */
export function reviewGroups(names) {
  const items = (names || []).map((name) => {
    const parsed = parseComicName(name);
    const base = parsed.base || '';
    return { name: String(name), base, chars: [...base] };
  });
  const parent = items.map((_, i) => i);
  const find = (x) => {
    while (parent[x] !== x) { parent[x] = parent[parent[x]]; x = parent[x]; }
    return x;
  };
  const buckets = new Map();
  items.forEach((it, i) => {
    if (it.chars.length < 3) return;
    for (const g of gramsOf(it.chars)) {
      const list = buckets.get(g);
      if (list) list.push(i);
      else buckets.set(g, [i]);
    }
  });
  const linked = new Array(items.length).fill(false);
  const seen = new Set();
  for (const bucket of buckets.values()) {
    if (bucket.length < 2 || bucket.length > 48) continue;
    for (let a = 0; a < bucket.length; a++) {
      for (let b = a + 1; b < bucket.length; b++) {
        const ia = bucket[a];
        const ib = bucket[b];
        const lo = Math.min(ia, ib);
        const hi = Math.max(ia, ib);
        const pair = `${lo}:${hi}`;
        if (seen.has(pair)) continue;
        seen.add(pair);
        const pa = find(ia);
        const pb = find(ib);
        if (pa === pb) { linked[ia] = true; linked[ib] = true; continue; }
        if (!hasMeaningfulCommon(items[ia].chars, items[ib].chars)) continue;
        parent[pb] = pa;
        linked[ia] = true;
        linked[ib] = true;
      }
    }
  }
  const clusters = new Map();
  items.forEach((_, i) => {
    if (!linked[i]) return;
    const p = find(i);
    const list = clusters.get(p);
    if (list) list.push(i);
    else clusters.set(p, [i]);
  });
  const groups = [];
  for (const ids of clusters.values()) {
    if (ids.length < 2) continue;
    const members = ids.map((i) => items[i].name);
    const title = suggest(ids.map((i) => items[i].base).filter(Boolean));
    groups.push({ title: title || members[0], members });
  }
  groups.sort((a, b) => (a.title < b.title ? -1 : a.title > b.title ? 1 : 0));
  return groups;
}
