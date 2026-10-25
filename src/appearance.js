// appearance.js - pure mapping from the two appearance prefs to CSS variables (no DOM; unit-tested in Node).
// Panel opacity scales the two glass levels (.glass uses --panel, overlays use --panel2); wallpaper fit sets --bg-size.
// CSS variables only: nothing here adds blur, so large panels stay free of backdrop-filter.
export const FIT = { cover: 'cover', contain: 'contain', stretch: '100% 100%' };
export const DEFAULTS = { panelOpacity: 82, bgFit: 'cover', bgDim: 55 };
export const PANEL_MIN = 40;   // below 40 % text over a bright wallpaper stops being readable
export const PANEL_MAX = 100;

/** -> { vars, panelOpacity, bgFit } with clamped / validated values. */
export function appearanceVars({ panelOpacity, bgFit } = {}) {
  const n = Number(panelOpacity);
  const op = Number.isFinite(n) && panelOpacity !== null && panelOpacity !== '' ? Math.min(PANEL_MAX, Math.max(PANEL_MIN, Math.round(n))) : DEFAULTS.panelOpacity;
  const fit = Object.prototype.hasOwnProperty.call(FIT, bgFit) ? bgFit : DEFAULTS.bgFit;
  return {
    panelOpacity: op, bgFit: fit,
    vars: { '--panel-a': String(op / 100), '--panel2-a': String(Math.min(1, +(op / 100 + 0.12).toFixed(2))), '--bg-size': FIT[fit] },
  };
}
