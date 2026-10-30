// shell-ui.js - fills <header id="shell-top"> and <nav id="shell-rail">, applies the accent colour,
// the whole-window background and the home banner (separate images since v0.6). Changes vs v0.5.1: the dim strength is a CSS variable (--bg-dim) that
// drives BOTH the default background (painted by mh-ui.css) and a custom image, so the Settings
// slider works either way; a custom image also becomes --banner-image for the Home banner;
// resetting to the default removes the layer; language button label is translated; the drop-queue
// poll pauses while the window is hidden.
import { T, invoke, t, esc, toast, prefs, applyTheme, applyI18n, wireWindow, lang, setLang } from './shell.js';
import { appearanceVars } from './appearance.js';
import { extrasVars } from './appearance-extras.js';

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
    top.innerHTML = `
      <a class="brand" href="home.html">
        <span class="brand__logo"><svg class="i"><use href="icons.svg#book-open"/></svg></span>
        <span class="brand__txt"><b>${esc(t('app.name'))}</b><small>${esc(t('app.tagline'))}</small></span>
      </a>
      <label class="search shell-search">
        <svg class="i"><use href="icons.svg#search"/></svg>
        <input id="shellSearch" type="search" autocomplete="off" placeholder="${esc(t('search.placeholder'))}">
      </label>
      <span class="spacer"></span>
      <button class="ibtn" id="shellLang" aria-label="${esc(t('settings.language'))}" title="${esc(t('settings.language'))}"><svg class="i"><use href="icons.svg#globe"/></svg></button>
      <div class="wc">
        <button data-window-minimize aria-label="${esc(t('win.min'))}"><svg class="i"><use href="icons.svg#min"/></svg></button>
        <button data-window-maximize aria-label="${esc(t('win.max'))}"><svg class="i"><use href="icons.svg#max"/></svg></button>
        <button data-window-close aria-label="${esc(t('win.close'))}"><svg class="i"><use href="icons.svg#close"/></svg></button>
      </div>`;
    const q = top.querySelector('#shellSearch');
    if (opts.searchValue) q.value = opts.searchValue;
    q.title = t('search.shortcut');
    let searchTimer = 0;
    const runSearch = () => {
      const v = q.value.trim();
      if (opts.onSearch) opts.onSearch(v);
      else if (v) location.href = `library.html?q=${encodeURIComponent(v)}`;
    };
    q.addEventListener('input', () => {
      if (!opts.onSearch) return;
      clearTimeout(searchTimer);
      searchTimer = setTimeout(runSearch, 80);
    });
    q.addEventListener('keydown', (e) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        if (q.value) { q.value = ''; if (opts.onSearch) opts.onSearch(''); }
        else q.blur();
        return;
      }
      if (e.key !== 'Enter') return;
      e.preventDefault();
      clearTimeout(searchTimer);
      runSearch();
    });
    document.addEventListener('keydown', (e) => {
      if (e.isComposing) return;
      const tag = e.target?.tagName;
      if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || e.target?.isContentEditable) return;
      if (document.documentElement.dataset.modal === '1' || document.querySelector('.modal-backdrop')) return;
      if (document.body.classList.contains('reader-chrome-away')) return;
      const slash = e.key === '/' && !e.ctrlKey && !e.metaKey && !e.altKey;
      const find = (e.key === 'k' || e.key === 'K') && (e.ctrlKey || e.metaKey) && !e.altKey;
      if (!slash && !find) return;
      e.preventDefault();
      q.focus();
      q.select();
    });
    // Window drag: the whole top bar except its controls drags the window; double-click toggles maximize. Done through our own
    // Rust commands (like the window buttons), so it does not depend on the webview's window-API permissions or on which
    // elements carry Tauri's drag-region attribute (only the element carrying it counted, which left just the thin spacer).
    top.addEventListener('mousedown', (e) => {
      if (e.button !== 0) return;
      if (e.target.closest?.('a, button, input, select, textarea, label, [data-no-drag]')) return;
      e.preventDefault();
      invoke(e.detail === 2 ? 'win_toggle_maximize' : 'win_start_drag').catch(() => {});
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
  mountScanFloat();
}

/** Corner pill while a folder scan is running, on every page. Click opens the scan page.
 *  The Rust scan keeps its own state, so moving between pages does not restart it. */
function mountScanFloat() {
  if (document.getElementById('scanFloat')) return;
  const btn = document.createElement('button');
  btn.type = 'button';
  btn.id = 'scanFloat';
  btn.className = 'scan-float';
  btn.hidden = true;
  btn.innerHTML = `<span class="spin" aria-hidden="true"></span><span id="scanFloatText"></span>`;
  btn.addEventListener('click', () => { location.href = 'scan.html'; });
  document.body.append(btn);
  const text = btn.querySelector('#scanFloatText');
  const preview = new URLSearchParams(location.search).get('preview');
  if (preview === 'float') {
    btn.hidden = false;
    text.textContent = t('scan.float', { folders: 128, books: 14 });
    return;
  }
  const onScan = /(?:^|\/)scan\.html$/.test(location.pathname);
  let noted = '';
  let pendingCache = null;
  let pendingAt = 0;
  const showRunning = (s) => {
    btn.hidden = false;
    btn.classList.remove('is-idle');
    text.textContent = t('scan.float', { folders: s.folders_scanned || 0, books: s.books_found || 0 });
  };
  const showChip = (n) => {
    btn.hidden = false;
    btn.classList.add('is-idle');
    text.textContent = t('scan.pendingChip', { n });
  };
  const hide = () => { btn.hidden = true; };
  async function pendingCount() {
    const now = Date.now();
    if (pendingCache != null && now - pendingAt < 4000) return pendingCache;
    pendingAt = now;
    try {
      const rows = await invoke('list_merge_pending');
      pendingCache = Array.isArray(rows) ? rows.length : 0;
    } catch {
      if (pendingCache == null) pendingCache = 0;
    }
    return pendingCache;
  }
  async function tick() {
    let s = null;
    try { s = await invoke('scan_status'); } catch { s = null; }
    const running = s?.phase === 'running';
    if (running) {
      if (!onScan) showRunning(s);
      else hide();
      return;
    }
    if (s?.phase === 'done') {
      const key = `${s.root}|${s.phase}|${s.folders_scanned}|${s.merged}|${s.pending}|${s.added}`;
      if (sessionStorage.getItem('mh.scanNote') !== key) {
        sessionStorage.setItem('mh.scanNote', key);
        const msg = t('scan.finishedNote', { added: s.added || 0, merged: s.merged || 0, pending: s.pending || 0 });
        toast(msg);
        notifyDone(msg);
      }
    }
    const n = await pendingCount();
    if (!onScan && n > 0) showChip(n);
    else hide();
  }
  async function notifyDone(body) {
    if (typeof Notification === 'undefined') return;
    let perm = Notification.permission;
    if (perm === 'default') {
      try { perm = await Notification.requestPermission(); } catch { return; }
    }
    if (perm !== 'granted') return;
    try { new Notification(t('app.name'), { body }); } catch { /* the toast already showed */ }
  }
  tick();
  setInterval(tick, 1000);
}

function applyAccent() {
  const hex = localStorage.getItem('mh.accent');
  if (hex && /^#[0-9a-f]{6}$/i.test(hex)) {
    document.documentElement.style.setProperty('--accent', hex);
    document.documentElement.style.setProperty('--accent2', hex);
  }
}

/** Panel opacity and wallpaper fit (prefs panelOpacity / bgFit) -> CSS variables --panel-a, --panel2-a, --bg-size on <html>.
 *  mh-ui.css derives --panel / --panel2 and the wallpaper sizing from them. Variables only: no blur is added anywhere. */
export function applyAppearance() {
  const { vars } = appearanceVars({ panelOpacity: prefs.get('panelOpacity', null), bgFit: prefs.get('bgFit', null) });
  for (const [k, v] of Object.entries(vars)) document.documentElement.style.setProperty(k, v);
}

/** Background style, background colour and border colour (prefs bgMode / bgColor / borderColor) -> CSS variables on <html>
 *  (--bg-color, --bg-image, --line, --line-strong, --frame-color; see appearance-extras.js). Returns true in plain-colour mode. */
export function applyExtras() {
  const { set, remove, colorMode } = extrasVars(
    { bgMode: prefs.get('bgMode', 'image'), bgColor: prefs.get('bgColor', null), borderColor: prefs.get('borderColor', null) },
    localStorage.getItem('mh.accent'));
  const st = document.documentElement.style;
  for (const [k, v] of Object.entries(set)) st.setProperty(k, v);
  for (const k of remove) st.removeProperty(k);
  return colorMode;
}

/** Whole-window background (theme config `hero`) and home banner (`banner`), independent of each other.
 *  The default background is painted by mh-ui.css; a custom background is drawn by .bg-layer above it.
 *  A custom banner becomes the CSS variable --banner-image used by .hero. Call again after changing either
 *  image or the dim slider. */
export async function applyBackground() {
  applyAppearance();
  const colorMode = applyExtras();            // plain-colour mode: no picture, and the dim overlay would only tint the colour
  const root = document.documentElement;
  const dim = colorMode ? 0 : Math.min(90, Math.max(0, +prefs.get('bgDim', 55))) / 100;
  root.style.setProperty('--bg-dim', String(dim)); // runs before the first await: no flash
  let cfg = {};
  try { cfg = (await invoke('get_theme_config')) || {}; } catch {}
  // Custom pictures are copied into the data dir and served by the mhfile protocol
  // (hero-custom.* / banner-custom.* only). The asset protocol is disabled.
  const fileUrl = (abs) => {
    if (!abs || abs === 'default' || !T.core?.convertFileSrc) return '';
    return T.core.convertFileSrc(abs, 'mhfile').replace(/["\\\n\r]/g, '');
  };
  const hero = cfg.hero || 'default', banner = cfg.banner || 'default'; // `banner` is absent on a backend without v0.6
  const heroUrl = colorMode ? '' : fileUrl(hero);
  let layer = document.querySelector('.bg-layer');
  if (!heroUrl) {
    layer?.remove();
    document.body.classList.remove('has-bg');
  } else {
    if (!layer) {
      layer = document.createElement('div');
      layer.className = 'bg-layer';
      document.body.prepend(layer);
    }
    layer.style.backgroundImage =
      `linear-gradient(rgba(250,253,252,calc(var(--bg-dim) * .45)), rgba(250,253,252,calc(var(--bg-dim) * .55))), url("${heroUrl}")`;   // soft white veil (v0.7); the slider value still scales it
    document.body.classList.add('has-bg');
  }
  const bannerUrl = fileUrl(banner);
  if (!bannerUrl) root.style.removeProperty('--banner-image');
  else root.style.setProperty('--banner-image', `url("${bannerUrl}")`);
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
