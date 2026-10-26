// appearance-extras.js - pure mapping (no DOM; unit-tested in Node) from three prefs to CSS variables:
//   bgMode   'image' (default: the default or custom picture) | 'color' (plain colour, picture hidden)
//   bgColor  '#rrggbb' - the plain colour; it also fills the empty area when the wallpaper fit is Contain
//   borderColor  null (built-in soft border) | '#rrggbb' | 'accent' (follow the accent colour)
// With nothing set every variable is removed and the stylesheet fallbacks apply, so the look is exactly the old one.
export const BG_COLOR_DEFAULT = '#061315';
export const BORDER_PRESETS = [   // [value, label key]; 'accent' = follow the accent colour; the five colours reuse the accent preset names
  ['accent', 'settings.borderMatch'], ['#ffffff', 'settings.borderWhite'], ['#80e5b6', 'accent.emerald'],
  ['#b7a6f5', 'accent.lavender'], ['#7cc4f0', 'accent.sky'], ['#f2a0b5', 'accent.rose'], ['#eec27f', 'accent.amber'],
];
export const BG_COLOR_PRESETS = ['#061315', '#0b1020', '#140b1a', '#101010', '#1a1410'];   // dark presets for the dark glass UI
export const BORDER_ALPHA = 0.5;          // custom colours are drawn translucent, like the built-in .13 border but visible
export const BORDER_STRONG_ALPHA = 0.6;   // overlays (.glass-strong) and the window frame
export const isHex = (v) => typeof v === 'string' && /^#[0-9a-f]{6}$/i.test(v);
export function rgba(hex, a) { const n = parseInt(hex.slice(1), 16); return `rgba(${(n >> 16) & 255},${(n >> 8) & 255},${n & 255},${a})`; }

/** prefs -> { set: {var: value}, remove: [var], colorMode }. accent = the current accent colour ('#rrggbb' or empty). */
export function extrasVars({ bgMode, bgColor, borderColor } = {}, accent) {
  const colorMode = bgMode === 'color';
  const color = isHex(bgColor) ? bgColor.toLowerCase() : BG_COLOR_DEFAULT;
  const set = {}, remove = [];
  if (colorMode || color !== BG_COLOR_DEFAULT) set['--bg-color'] = color; else remove.push('--bg-color');
  if (colorMode) set['--bg-image'] = 'none'; else remove.push('--bg-image');
  const b = borderColor === 'accent' ? (isHex(accent) ? accent : '#80e5b6') : isHex(borderColor) ? borderColor : null;
  if (b) { set['--line'] = rgba(b, BORDER_ALPHA); set['--line-strong'] = rgba(b, BORDER_STRONG_ALPHA); set['--frame-color'] = rgba(b, BORDER_STRONG_ALPHA); }
  else remove.push('--line', '--line-strong', '--frame-color');
  return { set, remove, colorMode };
}
