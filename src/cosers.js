// Embedded view of the official site. The address lives in Rust (site_view_place).
// This page only positions that view. It does not fetch, parse, or store the site.
import { invoke, bootPage } from './shell.js';
import { mountShell } from './shell-ui.js';
import { pageSignal } from './page-life.js';

export const COSERS_URL = 'https://coserslove.com/cosers';

bootPage('cosers');
mountShell('cosers');

const signal = pageSignal();
const host = document.getElementById('cosersHost');
let dead = false;
let raf = 0;
let fallback = false;

function bounds() {
  const r = host.getBoundingClientRect();
  return { x: r.left, y: r.top, width: r.width, height: r.height };
}

function showFallback() {
  if (fallback || !host) return;
  fallback = true;
  const note = host.querySelector('.cosers-note');
  if (!note || note.querySelector('a')) return;
  const link = document.createElement('a');
  link.href = COSERS_URL;
  link.target = '_blank';
  link.rel = 'noreferrer';
  link.textContent = COSERS_URL;
  note.append(document.createTextNode(' '));
  note.append(link);
}

async function place() {
  if (dead || !host) return;
  try {
    await invoke('site_view_place', bounds());
  } catch {
    showFallback();
  }
  if (dead) await invoke('site_view_close').catch(() => {});
}

function schedule() {
  if (dead || raf) return;
  raf = requestAnimationFrame(() => {
    raf = 0;
    place();
  });
}

signal?.addEventListener('abort', () => {
  dead = true;
  cancelAnimationFrame(raf);
  invoke('site_view_close').catch(() => {});
});

if (host && typeof ResizeObserver === 'function') {
  const ro = new ResizeObserver(() => schedule());
  ro.observe(host);
  signal?.addEventListener('abort', () => ro.disconnect());
}
window.addEventListener('resize', schedule, { signal });
window.addEventListener('pagehide', () => {
  dead = true;
  invoke('site_view_close').catch(() => {});
}, { signal });
schedule();
