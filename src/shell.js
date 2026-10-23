// shell.js - shared page runtime (v0.6). Changes vs v0.5.1: t() no longer falls back to the other
// language (a missing key shows the key itself and is logged once); every user-visible string
// that was hardcoded (dialog buttons, modal close label, connection errors, "just now"/"x ago")
// now comes from the dictionaries; dialogs resolve when closed with x/backdrop/Escape; prompt
// dialogs can be prefilled and submit on Enter; data-i-placeholder is supported; fmtBytes and
// setLang helpers added. Existing exports are unchanged.
import zh from './zh-CN.js';
import en from './en.js';

export const T = window.__TAURI__ || {};
export const invoke = (cmd, args = {}) => {
  if (!T.core?.invoke) return Promise.reject(new Error(t('err.noDesktop')));
  return T.core.invoke(cmd, args);
};
export const win = T.window?.getCurrentWindow?.() ||
  { setFullscreen: async () => {}, isFullscreen: async () => false, onCloseRequested: async () => {}, destroy: async () => {} };

export const lang = () => (localStorage.getItem('mh.lang') === 'en' ? 'en' : 'zh-CN');
export const setLang = (l) => localStorage.setItem('mh.lang', l === 'en' ? 'en' : 'zh-CN');

const warned = new Set();
export function t(key, vars = {}) {
  const dict = lang() === 'en' ? en : zh;
  const s = dict[key];
  if (s === undefined) {
    if (!warned.has(lang() + key)) { warned.add(lang() + key); console.warn('[i18n] missing key', key, lang()); }
    return key;
  }
  return String(s)
    .replace(/\{(\w+)\}/g, (_, k) => vars[k] ?? `{${k}}`)
    .replace(/\\u([\da-f]{4})/gi, (_, h) => String.fromCharCode(parseInt(h, 16)));
}

export const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

export function ago(v) {
  if (!v) return '';
  const n = typeof v === 'number' ? (v < 1e12 ? v * 1000 : v) : Date.parse(v);
  if (!Number.isFinite(n)) return '';
  const d = Math.max(0, Date.now() - n), m = Math.floor(d / 60000);
  if (m < 1) return t('time.justNow');
  if (m < 60) return t('time.minutes', { n: m });
  const h = Math.floor(m / 60);
  if (h < 24) return t('time.hours', { n: h });
  const days = Math.floor(h / 24);
  return days < 30 ? t('time.days', { n: days }) : new Date(n).toLocaleDateString(lang() === 'en' ? 'en-US' : 'zh-CN');
}

export function fmtBytes(n) {
  n = +n || 0;
  if (!n) return '—';
  const u = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0, v = n;
  while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(v >= 100 || i === 0 ? 0 : 1)} ${u[i]}`;
}

export const prefs = {
  get(k, d) { try { const v = localStorage.getItem('mh.pref.' + k); return v === null ? d : JSON.parse(v); } catch { return d; } },
  set(k, v) { try { localStorage.setItem('mh.pref.' + k, JSON.stringify(v)); } catch {} },
};

export function toast(msg) {
  let n = document.querySelector('.toast');
  if (!n) { n = document.createElement('div'); n.className = 'toast'; document.body.append(n); }
  n.textContent = String(msg ?? '');
  n.classList.add('show');
  clearTimeout(n._timer);
  n._timer = setTimeout(() => n.classList.remove('show'), 2600);
}

export function applyI18n(root = document) {
  root.querySelectorAll('[data-i]').forEach((e) => { e.textContent = t(e.dataset.i); });
  root.querySelectorAll('[data-i-label]').forEach((e) => e.setAttribute('aria-label', t(e.dataset.iLabel)));
  root.querySelectorAll('[data-i-title]').forEach((e) => e.setAttribute('title', t(e.dataset.iTitle)));
  root.querySelectorAll('[data-i-placeholder]').forEach((e) => e.setAttribute('placeholder', t(e.dataset.iPlaceholder)));
  document.documentElement.lang = lang() === 'en' ? 'en' : 'zh-CN';
  if (root === document) document.title = t('app.name');
}
export function bootPage(section) { applyI18n(); }

export const isModalOpen = () => !!document.querySelector('.modal-backdrop');
export function openModal({ title = '', body = null, wide = false, onClose = null } = {}) {
  const backdrop = document.createElement('div');
  backdrop.className = 'modal-backdrop';
  const panel = document.createElement('section');
  panel.className = 'modal glass-strong' + (wide ? ' modal--wide' : '');
  panel.setAttribute('role', 'dialog');
  panel.setAttribute('aria-modal', 'true');
  const head = document.createElement('header');
  head.className = 'modal__head';
  const h = document.createElement('h2');
  h.textContent = title;
  const close = document.createElement('button');
  close.className = 'ibtn';
  close.textContent = '×';
  close.setAttribute('aria-label', t('common.close'));
  head.append(h, close);
  const content = document.createElement('div');
  content.className = 'modal__body';
  if (typeof body === 'string') content.innerHTML = body; else if (body) content.append(body);
  panel.append(head, content);
  backdrop.append(panel);
  document.body.append(backdrop);
  document.documentElement.dataset.modal = '1'; // reader.js onKey() stops handling keys while a dialog is open
  const onKey = (e) => { if (e.key === 'Escape') done(); };
  let closed = false;
  function done() {
    if (closed) return;
    closed = true;
    document.removeEventListener('keydown', onKey);
    backdrop.remove();
    if (!document.querySelector('.modal-backdrop')) delete document.documentElement.dataset.modal;
    onClose?.();
  }
  document.addEventListener('keydown', onKey);
  close.onclick = done;
  backdrop.addEventListener('click', (e) => { if (e.target === backdrop) done(); });
  return { close: done, element: panel, body: content };
}

function dialog({ title = '', message = '', label = '', ok = t('common.ok'), cancel = t('common.cancel'), danger = false, placeholder = '', value = '' }) {
  return new Promise((resolve) => {
    const body = document.createElement('div');
    body.className = 'dialog';
    const p = document.createElement('p');
    p.textContent = message || label;
    if (message || label) body.append(p);
    let input = null;
    if (placeholder || value) {
      input = document.createElement('input');
      input.className = 'field';
      input.placeholder = placeholder;
      input.value = value;
      body.append(input);
    }
    const actions = document.createElement('div');
    actions.className = 'modal__actions';
    const no = document.createElement('button');
    no.className = 'btn btn--ghost';
    no.textContent = cancel;
    const yes = document.createElement('button');
    yes.className = 'btn ' + (danger ? 'btn--danger' : 'btn--primary');
    yes.textContent = ok;
    actions.append(no, yes);
    body.append(actions);
    const m = openModal({ title, body, onClose: () => resolve(false) });
    no.onclick = () => m.close();
    const submit = () => { resolve(input ? input.value : true); m.close(); };
    yes.onclick = submit;
    if (input) { input.addEventListener('keydown', (e) => { if (e.key === 'Enter') submit(); }); input.focus(); input.select(); }
  });
}
export const confirmDialog = (o) => dialog(o);
export const promptDialog = (o) => dialog({ ...o, placeholder: o?.placeholder || o?.value || ' ' });

export function choiceDialog({ title, message = '', choices = [] } = {}) {
  return new Promise((resolve) => {
    const body = document.createElement('div');
    body.className = 'dialog';
    const p = document.createElement('p');
    p.textContent = message;
    body.append(p);
    const actions = document.createElement('div');
    actions.className = 'modal__actions';
    const m = openModal({ title, body, onClose: () => resolve(null) });
    for (const c of choices) {
      const b = document.createElement('button');
      b.className = 'btn ' + (c.kind === 'primary' ? 'btn--primary' : 'btn--ghost');
      b.textContent = c.label;
      b.onclick = () => { resolve(c.value); m.close(); };
      actions.append(b);
    }
    body.append(actions);
  });
}

export function applyTheme(theme = prefs.get('theme', 'dark')) {
  const safe = ['dark', 'light', 'system'].includes(theme) ? theme : 'dark';
  document.documentElement.dataset.theme = safe;
  document.documentElement.classList.toggle('theme-light', safe === 'light');
  return safe;
}

export function wireWindow() {
  const w = T.window?.getCurrentWindow?.();
  if (!w) return false;
  document.querySelectorAll('[data-window-minimize]').forEach((b) => b.addEventListener('click', () => w.minimize?.()));
  document.querySelectorAll('[data-window-maximize]').forEach((b) => b.addEventListener('click', () => w.toggleMaximize?.()));
  document.querySelectorAll('[data-window-close]').forEach((b) => b.addEventListener('click', () => w.close?.()));
  return true;
}
