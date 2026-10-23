// shell-ui.js - fills <header id="shell-top"> and <nav id="shell-rail">, applies the accent colour,
// the whole-window background and the home banner (separate images since v0.6). Changes vs v0.5.1: the dim strength is a CSS variable (--bg-dim) that
// drives BOTH the default background (painted by mh-ui.css) and a custom image, so the Settings
// slider works either way; a custom image also becomes --banner-image for the Home banner;
// resetting to the default removes the layer; language button label is translated; the drop-queue
// poll pauses while the window is hidden.
import { T, invoke, t, esc, prefs, applyTheme, applyI18n, wireWindow, lang, setLang } from './shell.js';

export const COVER_BASE = T.core?.convertFileSrc
  ? T.core.convertFileSrc('', 'mhcover').replace(/\/$/, '')
  : '';
export const coverUrl = (id, v = 0) => `${COVER_BASE}/${id}?v=${v ?? 0}`;

/** Cover <img data-cover="id"> fallback: a 404 means "not generated yet" -> ask Rust once
 *  (ensure_cover), then reload the image. Call once per page. */
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

/** Fill #shell-top / #shell-rail (either may be absent), apply accent + background.
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
      <button class="ibtn" id="shellLang" aria-label="${esc(t('settings.language'))}" title="${esc(t('settings.language'))}"><svg class="i"><use href="icons.svg#globe"/></svg></button>
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
      setLang(lang() === 'en' ? 'zh-CN' : 'en');
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

/** Whole-window background (theme config `hero`) and home banner (`banner`), independent of each other.
 *  The default background is painted by mh-ui.css; a custom background is drawn by .bg-layer above it.
 *  A custom banner becomes the CSS variable --banner-image used by .hero. Call again after changing either
 *  image or the dim slider. */
export async function applyBackground() {
  const root = document.documentElement;
  const dim = Math.min(90, Math.max(0, +prefs.get('bgDim', 55))) / 100;
  root.style.setProperty('--bg-dim', String(dim)); // runs before the first await: no flash
  let cfg = {};
  try { cfg = (await invoke('get_theme_config')) || {}; } catch {}
  const conv = T.core?.convertFileSrc;
  const hero = cfg.hero || 'default', banner = cfg.banner || 'default'; // `banner` is absent on a backend without v0.6
  let layer = document.querySelector('.bg-layer');
  if (hero === 'default' || !conv) {
    layer?.remove();
    document.body.classList.remove('has-bg');
  } else {
    if (!layer) {
      layer = document.createElement('div');
      layer.className = 'bg-layer';
      document.body.prepend(layer);
    }
    layer.style.backgroundImage =
      `linear-gradient(rgba(4,14,15,var(--bg-dim)), rgba(4,14,15,calc(var(--bg-dim) + .12))), url("${conv(hero)}")`;
    document.body.classList.add('has-bg');
  }
  if (banner === 'default' || !conv) root.style.removeProperty('--banner-image');
  else root.style.setProperty('--banner-image', `url("${conv(banner)}")`);
}

/** Poll the Rust drop queue (native drag-drop writes paths there). */
export function watchDrops(onPaths) {
  setInterval(async () => {
    if (document.hidden) return;
    try {
      const q = await invoke('take_drop_queue');
      if (Array.isArray(q) && q.length) onPaths(q);
    } catch {}
  }, 900);
}
