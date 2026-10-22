// shell-ui.js - the app shell the repaired package left empty:
// reader.html (and the library pages) contain <header id="shell-top"> and
// <nav id="shell-rail"> placeholders; this module fills them, matching the
// target UI: brand + search + language + window controls on top, an icon
// rail (home / library / history / favourites / import / settings) on the
// left, the user's custom background image behind the glass, and the
// chosen accent colour.
import { T, invoke, t, esc, prefs, applyTheme, applyI18n, wireWindow } from './shell.js';

export const COVER_BASE = T.core?.convertFileSrc
  ? T.core.convertFileSrc('', 'mhcover').replace(/\/$/, '')
  : '';
export const coverUrl = (id, v = 0) => `${COVER_BASE}/${id}?v=${v ?? 0}`;

/** Cover <img data-cover="id"> fallback: a 404 means "not generated yet" ->
 *  ask Rust once (ensure_cover), then reload the image. Call once per page. */
export function wireCoverFallback() {
  document.addEventListener('error', async (e) => {
    const img = e.target;
    if (!(img instanceof HTMLImageElement) || !img.dataset.cover || img.dataset.retried) return;
    img.dataset.retried = '1';
    try {
      await invoke('ensure_cover', { bookId: +img.dataset.cover });
      img.src = coverUrl(img.dataset.cover, Date.now());
    } catch {}
  }, true);
}

const RAIL = [
  { key: 'home', href: 'home.html', icon: 'home', label: 'nav.home' },
  { key: 'library', href: 'library.html', icon: 'shelf', label: 'nav.library' },
  { key: 'recent', href: 'library.html?view=recent', icon: 'clock', label: 'reader.history' },
  { key: 'marked', href: 'library.html?view=marked', icon: 'star', label: 'bookmarks.title' },
  { key: 'import', href: 'import.html', icon: 'download', label: 'nav.import' },
];

/** Fill #shell-top / #shell-rail (either may be absent), apply accent + bg.
 *  opts: { onSearch(q), searchValue } */
export function mountShell(active = '', opts = {}) {
  applyTheme();
  applyAccent();
  applyBackground();

  const top = document.getElementById('shell-top');
  if (top) {
    top.className = 'shell-top';
    top.setAttribute('data-tauri-drag-region', '');
    top.innerHTML = `
      <a class="brand" href="home.html">
        <span class="brand__logo"><svg class="i"><use href="icons.svg#book-open"/></svg></span>
        <span class="brand__txt"><b>${esc(t('app.name'))}</b><small>${esc(t('app.tagline'))}</small></span>
      </a>
      <label class="search shell-search">
        <svg class="i"><use href="icons.svg#search"/></svg>
        <input id="shellSearch" type="search" autocomplete="off" placeholder="${esc(t('search.placeholder'))}">
      </label>
      <span class="spacer" data-tauri-drag-region></span>
      <button class="ibtn" id="shellLang" aria-label="Language"><svg class="i"><use href="icons.svg#globe"/></svg></button>
      <div class="wc">
        <button data-window-minimize aria-label="${esc(t('win.min'))}"><svg class="i"><use href="icons.svg#min"/></svg></button>
        <button data-window-maximize aria-label="${esc(t('win.max'))}"><svg class="i"><use href="icons.svg#max"/></svg></button>
        <button data-window-close aria-label="${esc(t('win.close'))}"><svg class="i"><use href="icons.svg#close"/></svg></button>
      </div>`;
    const q = top.querySelector('#shellSearch');
    if (opts.searchValue) q.value = opts.searchValue;
    q.addEventListener('keydown', (e) => {
      if (e.key !== 'Enter') return;
      const v = q.value.trim();
      if (opts.onSearch) opts.onSearch(v);
      else if (v) location.href = `library.html?q=${encodeURIComponent(v)}`;
    });
    top.querySelector('#shellLang').addEventListener('click', () => {
      const cur = localStorage.getItem('mh.lang') || 'zh-CN';
      localStorage.setItem('mh.lang', cur === 'en' ? 'zh-CN' : 'en');
      location.reload();
    });
  }

  const rail = document.getElementById('shell-rail');
  if (rail) {
    rail.className = 'shell-rail';
    rail.innerHTML =
      RAIL.map((r) => `<a href="${r.href}" class="${active === r.key ? 'is-on' : ''}">
          <svg class="i"><use href="icons.svg#${r.icon}"/></svg><span>${esc(t(r.label))}</span></a>`).join('') +
      `<span class="rail-spacer"></span>
       <a href="settings.html" class="${active === 'settings' ? 'is-on' : ''}">
         <svg class="i"><use href="icons.svg#settings"/></svg><span>${esc(t('nav.settings'))}</span></a>`;
  }
  applyI18n();
  wireWindow();
}

function applyAccent() {
  const hex = localStorage.getItem('mh.accent');
  if (hex && /^#[0-9a-f]{6}$/i.test(hex)) {
    document.documentElement.style.setProperty('--accent', hex);
    document.documentElement.style.setProperty('--accent2', hex);
  }
}

/** Custom whole-window background (Settings -> set_hero_image stores the file
 *  in the data dir and theme config's `hero` points at it). */
export async function applyBackground() {
  try {
    const cfg = await invoke('get_theme_config');
    if (!cfg || !cfg.hero || cfg.hero === 'default') return;
    let layer = document.querySelector('.bg-layer');
    if (!layer) {
      layer = document.createElement('div');
      layer.className = 'bg-layer';
      document.body.prepend(layer);
    }
    const dim = prefs.get('bgDim', 55); // 0..100: strength of the dark overlay
    layer.style.backgroundImage =
      `linear-gradient(rgba(4,14,15,${dim / 100}), rgba(4,14,15,${Math.min(0.92, dim / 100 + 0.12)})), url("${T.core.convertFileSrc(cfg.hero)}")`;
    document.body.classList.add('has-bg');
  } catch {}
}

/** Poll the Rust drop queue (native drag-drop writes paths there). */
export function watchDrops(onPaths) {
  setInterval(async () => {
    try {
      const q = await invoke('take_drop_queue');
      if (Array.isArray(q) && q.length) onPaths(q);
    } catch {}
  }, 900);
}
