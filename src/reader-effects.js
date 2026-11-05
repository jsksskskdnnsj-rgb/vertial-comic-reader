// reader-effects.js - map reading-surface prefs to CSS variables (no DOM; unit-tested in Node).
// Applied on #host so chrome panels are not filtered.

export const FILTER_PRESETS = ['none', 'warm', 'night', 'paper'];
export const EFFECT_DEFAULTS = {
  dim: 100,
  contrast: 100,
  filter: 'none',
  filterStrength: 70,
};

const clamp = (v, a, b) => Math.min(b, Math.max(a, v));

/** Normalise stored prefs. */
export function normalizeEffects(raw = {}) {
  const dim = clamp(Math.round(Number(raw.dim ?? EFFECT_DEFAULTS.dim) || EFFECT_DEFAULTS.dim), 60, 100);
  const contrast = clamp(Math.round(Number(raw.contrast ?? EFFECT_DEFAULTS.contrast) || EFFECT_DEFAULTS.contrast), 80, 120);
  const filter = FILTER_PRESETS.includes(raw.filter) ? raw.filter : EFFECT_DEFAULTS.filter;
  const filterStrength = clamp(Math.round(Number(raw.filterStrength ?? EFFECT_DEFAULTS.filterStrength) || EFFECT_DEFAULTS.filterStrength), 0, 100);
  return { dim, contrast, filter, filterStrength };
}

/** Build filter() chain parts for preset + strength (0–100). */
export function filterParts({ filter, filterStrength } = {}) {
  const f = FILTER_PRESETS.includes(filter) ? filter : 'none';
  const t = clamp(Number(filterStrength) || 0, 0, 100) / 100;
  if (f === 'none' || t <= 0) return [];
  if (f === 'warm') return [`sepia(${+(0.35 * t).toFixed(3)})`, `hue-rotate(-${Math.round(8 * t)}deg)`];
  if (f === 'night') return [`brightness(${+(1 - 0.08 * t).toFixed(3)})`, `saturate(${+(1 - 0.25 * t).toFixed(3)})`, `hue-rotate(${Math.round(12 * t)}deg)`];
  if (f === 'paper') return [`sepia(${+(0.18 * t).toFixed(3)})`, `contrast(${+(1 + 0.06 * t).toFixed(3)})`];
  return [];
}

/** -> { vars, effects } */
export function readerEffectVars(raw = {}) {
  const effects = normalizeEffects(raw);
  const bright = (effects.dim / 100).toFixed(3);
  const contrast = (effects.contrast / 100).toFixed(3);
  const parts = [`brightness(${bright})`, `contrast(${contrast})`, ...filterParts(effects)];
  return {
    effects,
    vars: { '--reader-filter': parts.join(' ') },
  };
}
