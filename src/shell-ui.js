// shell-ui.js - fills <header id="shell-top"> and <nav id="shell-rail">, applies the accent colour,
// the whole-window background and the home banner (separate images since v0.6). v0.8.1: one-shot shell
// chrome (rail/top not rebuilt every navigation), cached wallpaper layer, in-app fetch navigation between
// shell pages so the background and frame stay put.
import { T, invoke, t, esc, toast, prefs, applyTheme, applyI18n, wireWindow, lang, setLang } from './shell.js';
import { appearanceVars } from './appearance.js';
import { extrasVars } from './appearance-extras.js';

export const COVER_BASE = T.core?.convertFileSrc
  ? T.core.convertFileSrc('', 'mhcover').replace(/\/$/, '')
  : '';
export const coverUrl = (id, v = 0) => `${COVER_BASE}/${id}?v=${v ?? 0}`;

const SHELL_PAGES = new Set(['home.html', 'library.html', 'import.html', 'settings.html', 'scan.html', 'book.html']);

let coverWired = false;
/** Cover <img data-cover="id"> fallback: a 404 means "not generated yet" -> ask Rust once
 *  (ensure_cover), then reload the image. Call once per page. */
export function wireCoverFallback() {
  if (coverWired) return;
  coverWired = true;
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

const searchCtx = { onSearch: null, searchTimer: 0 };
let navGen = 0;
let navBusy = false;
let themeCfgCache = null;
let themeCfgAt = 0;
let lastHeroLayer = '';
let lastBannerVar = '';

export function railActiveFromUrl(url = location) {
  const base = url.pathname.split('/').pop() || 'home.html';
  const view = url.searchParams.get('view');
  if (base === 'library.html') {
    if (view === 'recent') return 'recent';
    if (view === 'marked') return 'marked';
    return 'library';
  }
  if (base === 'settings.html') return 'settings';
  if (base === 'import.html' || base === 'scan.html') return 'import';
  if (base === 'book.html') return 'library';
  if (base === 'home.html') return 'home';
  return '';
}

function isShellUrl(url) {
  if (url.origin !== location.origin) return false;
  const base = url.pathname.split('/').pop() || '';
  return SHELL_PAGES.has(base);
}

function syncRailActive(active) {
  const rail = document.getElementById('shell-rail');
  if (!rail) return;
  rail.querySelectorAll('a[href]').forEach((a) => {
    const key = a.href.includes('settings.html') ? 'settings'
      : a.href.includes('view=recent') ? 'recent'
        : a.href.includes('view=marked') ? 'marked'
          : a.href.includes('import.html') ? 'import'
            : a.href.includes('library.html') ? 'library'
              : a.href.includes('home.html') ? 'home' : '';
    a.classList.toggle('is-on', key === active);
  });
}

function renderRail(active) {
  const rail = document.getElementById('shell-rail');
  if (!rail) return;
  rail.className = 'shell-rail';
  rail.innerHTML =
    RAIL.map((r) => `<a href="${r.href}" class="${active === r.key ? 'is-on' : ''}">
        <svg class="i"><use href="icons.svg#${r.icon}"/></svg><span>${esc(t(r.label))}</span></a>`).join('') +
    `<span class="rail-spacer"></span>
     <a href="settings.html" class="${active === 'settings' ? 'is-on' : ''}">
       <svg class="i"><use href="icons.svg#settings"/></svg><span>${esc(t('nav.settings'))}</span></a>`;
}

function buildShellTop(opts = {}) {
  const top = document.getElementById('shell-top');
  if (!top || top.dataset.built === '1') return top;
  top.dataset.built = '1';
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
  q.title = t('search.shortcut');
  const runSearch = () => {
    const v = q.value.trim();
    if (searchCtx.onSearch) searchCtx.onSearch(v);
    else if (v) shellNavigate(`library.html?q=${encodeURIComponent(v)}`);
  };
  q.addEventListener('input', () => {
    if (!searchCtx.onSearch) return;
    clearTimeout(searchCtx.searchTimer);
    searchCtx.searchTimer = setTimeout(runSearch, 80);
  });
  q.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      if (q.value) { q.value = ''; if (searchCtx.onSearch) searchCtx.onSearch(''); }
      else q.blur();
      return;
    }
    if (e.key !== 'Enter') return;
    e.preventDefault();
    clearTimeout(searchCtx.searchTimer);
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
  return top;
}

function syncShellSearch(opts = {}) {
  searchCtx.onSearch = opts.onSearch || null;
  const q = document.getElementById('shellSearch');
  if (q && opts.searchValue != null) q.value = opts.searchValue;
}

async function loadPageModule(pathname) {
  const base = pathname.split('/').pop() || 'home.html';
  const stem = base.replace(/\.html$/i, '');
  const gen = ++navGen;
  await import(`./${stem}.js?shell=${gen}`);
}

async function swapShellMain(href, { historyMode = 'push' } = {}) {
  const url = new URL(href, location.href);
  if (!isShellUrl(url)) {
    location.assign(url.href);
    return;
  }
  const main = document.querySelector('.page-main');
  if (!main) {
    location.assign(url.href);
    return;
  }
  const same = url.pathname === location.pathname && url.search === location.search && url.hash === location.hash;
  if (same && historyMode !== 'pop') return;

  navBusy = true;
  main.classList.add('is-swapping');
  try {
    const res = await fetch(url.pathname + url.search);
    if (!res.ok) throw new Error(String(res.status));
    const doc = new DOMParser().parseFromString(await res.text(), 'text/html');
    const incoming = doc.querySelector('.page-main');
    if (!incoming) throw new Error('no main');
    main.innerHTML = incoming.innerHTML;
    main.scrollTop = 0;
    if (historyMode === 'push') history.pushState(null, '', url.pathname + url.search + url.hash);
    else if (historyMode === 'replace') history.replaceState(null, '', url.pathname + url.search + url.hash);
    syncRailActive(railActiveFromUrl(url));
    await loadPageModule(url.pathname);
    applyI18n();
    if (url.hash) document.querySelector(url.hash)?.scrollIntoView({ block: 'start' });
  } catch {
    location.assign(url.href);
  } finally {
    main.classList.remove('is-swapping');
    navBusy = false;
  }
}

/** In-app navigation between shell HTML pages (keeps wallpaper + unified frame). */
export function shellNavigate(href, opts = {}) {
  if (navBusy) return Promise.resolve();
  const url = new URL(href, location.href);
  if (!isShellUrl(url)) {
    location.assign(url.href);
    return Promise.resolve();
  }
  const mode = opts.replace ? 'replace' : 'push';
  return swapShellMain(url.href, { historyMode: mode });
}

function wireShellNavigation() {
  if (!document.querySelector('.work.shell-frame') || document.body.dataset.shellNav === '1') return;
  document.body.dataset.shellNav = '1';
  document.addEventListener('click', (e) => {
    if (navBusy || e.defaultPrevented) return;
    const a = e.target.closest('a[href]');
    if (!a || a.target === '_blank') return;
    if (e.ctrlKey || e.metaKey || e.shiftKey || e.altKey) return;
    const url = new URL(a.href, location.href);
    if (!isShellUrl(url)) return;
    e.preventDefault();
    void shellNavigate(url.href);
  });
  addEventListener('popstate', () => {
    if (navBusy) return;
    void swapShellMain(location.pathname + location.search + location.hash, { historyMode: 'pop' });
  });
  const rail = document.getElementById('shell-rail');
  rail?.addEventListener('mouseover', (e) => {
    const a = e.target.closest('a[href]');
    if (!a) return;
    const u = new URL(a.href, location.href);
    if (!isShellUrl(u)) return;
    const link = document.createElement('link');
    link.rel = 'prefetch';
    link.href = u.pathname + u.search;
    document.head.append(link);
  }, { passive: true });
}

/** Fill #shell-top / #shell-rail (either may be absent), apply accent + background.
 *  opts: { onSearch(q), searchValue } */
export function mountShell(active = '', opts = {}) {
  applyTheme();
  applyAccent();
  applyBackground();

  buildShellTop(opts);
  syncShellSearch(opts);

  const rail = document.getElementById('shell-rail');
  if (rail) {
    if (rail.dataset.built !== '1') {
      renderRail(active);
      rail.dataset.built = '1';
    } else syncRailActive(active);
  }
  applyI18n();
  wireWindow();
  mountScanFloat();
  wireShellNavigation();
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
  btn.addEventListener('click', () => { void shellNavigate('scan.html'); });
  document.body.append(btn);
  const text = btn.querySelector('#scanFloatText');
  const preview = new URLSearchParams(location.search).get('preview');
  if (preview === 'float') {
    btn.hidden = false;
    text.textContent = t('scan.float', { folders: 128, books: 14 });
    return;
  }
  const onScan = /(?:^|\/)scan\.html$/.test(location.pathname);
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

async function readThemeConfig() {
  const now = Date.now();
  if (themeCfgCache != null && now - themeCfgAt < 15000) return themeCfgCache;
  themeCfgAt = now;
  try {
    themeCfgCache = (await invoke('get_theme_config')) || {};
  } catch {
    themeCfgCache = themeCfgCache || {};
  }
  return themeCfgCache;
}

/** Whole-window background (theme config `hero`) and home banner (`banner`), independent of each other.
 *  The default background is painted by mh-ui.css; a custom background is drawn by .bg-layer above it.
 *  A custom banner becomes the CSS variable --banner-image used by .hero. Call again after changing either
 *  image or the dim slider. */
export async function applyBackground() {
  applyAppearance();
  const colorMode = applyExtras();
  const root = document.documentElement;
  const dim = colorMode ? 0 : Math.min(90, Math.max(0, +prefs.get('bgDim', 55))) / 100;
  root.style.setProperty('--bg-dim', String(dim));
  const cfg = await readThemeConfig();
  const fileUrl = (abs) => {
    if (!abs || abs === 'default' || !T.core?.convertFileSrc) return '';
    return T.core.convertFileSrc(abs, 'mhfile').replace(/["\\\n\r]/g, '');
  };
  const hero = cfg.hero || 'default';
  const banner = cfg.banner || 'default';
  const heroUrl = colorMode ? '' : fileUrl(hero);
  let layer = document.querySelector('.bg-layer');
  if (!heroUrl) {
    if (layer) layer.remove();
    document.body.classList.remove('has-bg');
    lastHeroLayer = '';
  } else {
    const layerStyle =
      `linear-gradient(rgba(250,253,252,calc(var(--bg-dim) * .45)), rgba(250,253,252,calc(var(--bg-dim) * .55))), url("${heroUrl}")`;
    if (!layer) {
      layer = document.createElement('div');
      layer.className = 'bg-layer';
      document.body.prepend(layer);
    }
    if (lastHeroLayer !== layerStyle) {
      layer.style.backgroundImage = layerStyle;
      lastHeroLayer = layerStyle;
    }
    document.body.classList.add('has-bg');
  }
  const bannerUrl = fileUrl(banner);
  const bannerCss = bannerUrl ? `url("${bannerUrl}")` : '';
  if (bannerCss !== lastBannerVar) {
    if (!bannerUrl) root.style.removeProperty('--banner-image');
    else root.style.setProperty('--banner-image', bannerCss);
    lastBannerVar = bannerCss;
  }
}

/** Invalidate cached theme paths after the user changes hero/banner in Settings. */
export function invalidateThemeCache() {
  themeCfgCache = null;
  themeCfgAt = 0;
  lastHeroLayer = '';
  lastBannerVar = '';
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
