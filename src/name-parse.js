// Filename / folder title parsing for comics. Pure (no DOM, no Tauri).
// Chinese numerals count only inside 第N话-style counters, same idea as naming.js.
import { chineseNumber } from './naming.js';

const COUNTER = new Set([...'话話章卷集回部期节節篇册冊']);
const VOLUME_COUNTER = new Set([...'卷册冊']);
const SEP = new Set([...' -~～—–－到至']);

function isDigit(c) {
  return c >= '0' && c <= '9';
}
function isCnNum(c) {
  return '零〇一二两三四五六七八九十百千'.includes(c);
}

function readNum(chars, i) {
  if (i >= chars.length) return null;
  if (isDigit(chars[i])) {
    let j = i + 1;
    while (j < chars.length && isDigit(chars[j])) j++;
    if (j - i > 9) return null;
    return { n: Number(chars.slice(i, j).join('')), end: j };
  }
  if (isCnNum(chars[i])) {
    let j = i + 1;
    while (j < chars.length && isCnNum(chars[j])) j++;
    const n = chineseNumber(chars.slice(i, j).join(''));
    if (n == null) return null;
    return { n, end: j };
  }
  return null;
}

function skipSpace(chars, i) {
  while (i < chars.length && /\s/.test(chars[i])) i++;
  return i;
}

/** Letters and CJK only, lowercased, digits dropped. Used as the series key. */
export function seriesBase(title) {
  let out = '';
  for (const c of String(title ?? '').normalize('NFKC').toLowerCase()) {
    if (isDigit(c)) continue;
    if ((c >= 'a' && c <= 'z') || (c >= '\u3400' && c <= '\u9fff') || (c >= '\u3040' && c <= '\u30ff') || (c >= '\uac00' && c <= '\ud7af')) {
      out += c;
    }
  }
  return out;
}

/**
 * @returns {{raw:string,title:string,base:string,volume:number|null,chapter:number|null,chapterEnd:number|null}}
 */
export function parseComicName(raw) {
  const original = String(raw ?? '').trim();
  let text = original.normalize('NFKC');
  text = text.replace(/\.(zip|cbz|rar|7z|jpe?g|jfif|jpe|png|webp|gif|bmp|avif)$/i, '');
  const chars = [...text];
  const cover = new Array(chars.length).fill(false);
  let volume = null;
  let chapter = null;
  let chapterEnd = null;
  const mark = (a, b) => { for (let i = a; i < b && i < cover.length; i++) cover[i] = true; };

  for (let i = 0; i < chars.length; i++) {
    if (chars[i] !== '第') continue;
    const m = readNum(chars, i + 1);
    if (!m) continue;
    let end = m.end;
    let n1 = m.n;
    let n2 = m.n;
    const sepAt = skipSpace(chars, end);
    if (sepAt < chars.length && SEP.has(chars[sepAt])) {
      const m2 = readNum(chars, skipSpace(chars, sepAt + 1));
      if (m2) { n2 = m2.n; end = m2.end; }
    }
    const counter = chars[end];
    if (!counter || !COUNTER.has(counter)) continue;
    end += 1;
    if (VOLUME_COUNTER.has(counter)) {
      if (volume == null) volume = n1;
    } else if (chapter == null) {
      chapter = Math.min(n1, n2);
      chapterEnd = Math.max(n1, n2);
    }
    mark(i, end);
    i = end - 1;
  }

  const latin = /(?:^|[^A-Za-z])((?:volume|chapter|episode|chap|vol|part|ep|ch))[._\-\s]*(\d{1,6})(?:\s*[-~～—–]\s*(\d{1,6}))?/gi;
  let lm;
  while ((lm = latin.exec(text))) {
    const key = lm[1].toLowerCase();
    const a = Number(lm[2]);
    const b = lm[3] ? Number(lm[3]) : a;
    const from = lm.index + lm[0].toLowerCase().indexOf(key);
    const to = lm.index + lm[0].length;
    if (/^vol/.test(key)) {
      if (volume == null) volume = a;
    } else if (chapter == null) {
      chapter = Math.min(a, b);
      chapterEnd = Math.max(a, b);
    }
    mark(from, to);
  }

  if (volume == null && chapter == null) {
    const bracket = text.match(/(^|[^0-9])[(（\[【](\d{1,4})[)）\]】]\s*$/);
    if (bracket) {
      const n = Number(bracket[2]);
      const at = text.lastIndexOf(bracket[2]);
      chapter = n;
      chapterEnd = n;
      const open = text[at - 1];
      mark(open && '([【（'.includes(open) ? at - 1 : at, text.length);
    } else {
      const tail = text.match(/(^|[^\d])(\d{1,4})\s*$/);
      if (tail) {
        const n = Number(tail[2]);
        const year = tail[2].length === 4 && n >= 1900 && n <= 2099;
        if (!year && text.length !== tail[2].length) {
          chapter = n;
          chapterEnd = n;
          mark(text.length - tail[2].length, text.length);
        }
      }
    }
  }

  const title = chars.filter((_, i) => !cover[i]).join('')
    .replace(/^[\s_\-–—.·・:：|+/\\【】\[\]()（）「」『』]+|[\s_\-–—.·・:：|+/\\【】\[\]()（）「」『』]+$/g, '')
    .replace(/\s{2,}/g, ' ')
    .trim();
  return {
    raw: original,
    title,
    base: seriesBase(title),
    volume,
    chapter,
    chapterEnd,
  };
}
