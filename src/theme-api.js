// theme-api.js - theme config helpers shared by Home and Settings. ThemeConfig (main.rs) holds the
// window background (`hero`, set_hero_image) and, since v0.6, an independent home banner (`banner`,
// set_banner_image). Saving always reads the current config first so no field is clobbered; if the
// read fails the save is aborted (the old code fell back to defaults and would have overwritten
// the user's theme).
import { invoke } from './shell.js';

const FALLBACK = { theme: 'dark', custom_pri: '', hero: 'default', banner: 'default' };
/** Tolerant read for display purposes. */
export const readTheme = () => invoke('get_theme_config').then((c) => ({ ...FALLBACK, ...(c || {}) })).catch(() => ({ ...FALLBACK }));
/** Strict read-modify-write: throws if the current config cannot be read. */
export async function saveTheme(patch) {
  const cfg = await invoke('get_theme_config');
  await invoke('set_theme_config', { cfg: { ...FALLBACK, ...cfg, ...patch } });
}
export async function setBackground(src) { const dest = await invoke('set_hero_image', { src }); await saveTheme({ hero: dest }); return dest; }
export async function resetBackground() { await invoke('reset_hero_image'); await saveTheme({ hero: 'default' }); }
export async function setBanner(src) { const dest = await invoke('set_banner_image', { src }); await saveTheme({ banner: dest }); return dest; }
export async function resetBanner() { await invoke('reset_banner_image'); await saveTheme({ banner: 'default' }); }
