// appearance-glass.js — chrome backdrop blur + wallpaper blur as CSS variables (pure, no DOM).
// Inspired by KoiPage src/lib/glass-appearance.ts and src/lib/backdrop.ts (GPL-3.0);
// MH uses independent constants and clamp logic (MIT).

export const DEFAULT_CHROME_BLUR = 14;
export const DEFAULT_BG_BLUR = 0;
export const CHROME_BLUR_MIN = 0;
export const CHROME_BLUR_MAX = 24;
export const BG_BLUR_MIN = 0;
export const BG_BLUR_MAX = 40;

function bounded(raw, min, max, fallback) {
  if (raw === null || raw === undefined || raw === '') return fallback;
  const n = Number(raw);
  return Number.isFinite(n) ? Math.min(max, Math.max(min, Math.round(n))) : fallback;
}

/** prefs chromeBlur / bgBlur -> { vars, chromeBlur, bgBlur } */
export function glassVars({ chromeBlur, bgBlur } = {}) {
  const chrome = bounded(chromeBlur, CHROME_BLUR_MIN, CHROME_BLUR_MAX, DEFAULT_CHROME_BLUR);
  const bg = bounded(bgBlur, BG_BLUR_MIN, BG_BLUR_MAX, DEFAULT_BG_BLUR);
  return {
    chromeBlur: chrome,
    bgBlur: bg,
    vars: {
      '--chrome-blur': `${chrome}px`,
      '--bg-blur': `${bg}px`,
    },
  };
}
