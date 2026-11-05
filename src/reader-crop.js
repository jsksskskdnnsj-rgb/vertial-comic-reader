// reader-crop.js - white-margin detection (pure) + clip styles for runtime crop (no source rewrite).

const clamp = (v, a, b) => Math.min(b, Math.max(a, v));

/** Scan RGBA buffer (row-major) for near-white borders. Returns fractions 0..maxCrop. */
export function detectWhiteInsets(rgba, width, height, {
  threshold = 248,
  maxCrop = 0.18,
  sampleStep = 4,
  minContentRatio = 0.015,
} = {}) {
  if (!rgba?.length || width < 8 || height < 8) return { top: 0, right: 0, bottom: 0, left: 0 };

  const isWhite = (x, y) => {
    const i = (y * width + x) * 4;
    const r = rgba[i], g = rgba[i + 1], b = rgba[i + 2], a = rgba[i + 3];
    return a > 8 && r >= threshold && g >= threshold && b >= threshold;
  };

  const rowWhiteRatio = (y) => {
    let w = 0, n = 0;
    for (let x = 0; x < width; x += sampleStep) { n++; if (isWhite(x, y)) w++; }
    return n ? w / n : 0;
  };
  const colWhiteRatio = (x) => {
    let w = 0, n = 0;
    for (let y = 0; y < height; y += sampleStep) { n++; if (isWhite(x, y)) w++; }
    return n ? w / n : 0;
  };

  let top = 0, bottom = 0, left = 0, right = 0;
  for (let y = 0; y < height; y += sampleStep) {
    if (rowWhiteRatio(y) > 1 - minContentRatio) top = y + sampleStep;
    else break;
  }
  for (let y = height - sampleStep; y >= 0; y -= sampleStep) {
    if (rowWhiteRatio(y) > 1 - minContentRatio) bottom = height - y;
    else break;
  }
  for (let x = 0; x < width; x += sampleStep) {
    if (colWhiteRatio(x) > 1 - minContentRatio) left = x + sampleStep;
    else break;
  }
  for (let x = width - sampleStep; x >= 0; x -= sampleStep) {
    if (colWhiteRatio(x) > 1 - minContentRatio) right = width - x;
    else break;
  }

  const toFrac = (px) => clamp(px / Math.max(1, width), 0, maxCrop);
  const toFracY = (px) => clamp(px / Math.max(1, height), 0, maxCrop);
  return {
    top: toFracY(top),
    right: toFrac(right),
    bottom: toFracY(bottom),
    left: toFrac(left),
  };
}

/** Merge auto insets with manual trim (0–12 → extra fraction up to ~6%). */
export function mergeCropInsets(auto, manualTrim = 0) {
  const extra = clamp(Number(manualTrim) || 0, 0, 12) * 0.005;
  const add = (v) => clamp(v + extra, 0, 0.22);
  return {
    top: add(auto?.top || 0),
    right: add(auto?.right || 0),
    bottom: add(auto?.bottom || 0),
    left: add(auto?.left || 0),
  };
}

/** CSS variables for one page image. */
export function cropVars(insets) {
  const t = insets?.top || 0, r = insets?.right || 0, b = insets?.bottom || 0, l = insets?.left || 0;
  return {
    '--crop-t': String(t),
    '--crop-r': String(r),
    '--crop-b': String(b),
    '--crop-l': String(l),
  };
}

/** Cache key for per-virtual-page crop (book sig + index). */
export function cropCacheKey(bookId, sig, virtualIndex) {
  return `${bookId}:${sig}:${virtualIndex}`;
}

/** Apply crop inset variables on a mounted page image (DOM). */
export function applyCropStyle(img, insets, enabled) {
  if (!img) return;
  if (!enabled || !insets) {
    img.classList.remove('pg-crop');
    for (const k of ['--crop-t', '--crop-r', '--crop-b', '--crop-l']) img.style.removeProperty(k);
    return;
  }
  img.classList.add('pg-crop');
  for (const [k, v] of Object.entries(cropVars(insets))) img.style.setProperty(k, v);
}

/** Sample a loaded bitmap for white margins (browser only). Returns inset fractions. */
export function detectCropFromImage(img, opts) {
  const w = img?.naturalWidth || 0, h = img?.naturalHeight || 0;
  if (!w || !h) return { top: 0, right: 0, bottom: 0, left: 0 };
  const maxSide = 320;
  const scale = Math.min(1, maxSide / Math.max(w, h));
  const cw = Math.max(8, Math.round(w * scale));
  const ch = Math.max(8, Math.round(h * scale));
  const canvas = document.createElement('canvas');
  canvas.width = cw;
  canvas.height = ch;
  const ctx = canvas.getContext('2d', { willReadFrequently: true });
  if (!ctx) return { top: 0, right: 0, bottom: 0, left: 0 };
  ctx.drawImage(img, 0, 0, cw, ch);
  const data = ctx.getImageData(0, 0, cw, ch).data;
  return detectWhiteInsets(data, cw, ch, opts);
}
