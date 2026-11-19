// Sidebar page that shows https://coserslove.com/cosers in a child webview.
// The webview loads the official document itself. This file only reserves a rectangle
// and does not fetch, parse, or restyle that page.
import { invoke, bootPage } from './shell.js';
import { onPage } from './page-life.js';
import { mountShell } from './shell-ui.js';
import { cosersViewRect } from './cosers-rect.js';

bootPage('cosers');
mountShell('cosers');

const host = document.getElementById('cosersHost');
const fallback = document.getElementById('cosersFallback');
let epoch = 0;
let raf = 0;
let watch = null;

function rectOf(el) {
  if (!el) return null;
  const r = el.getBoundingClientRect();
  return { left: r.left, top: r.top, right: r.right, bottom: r.bottom, width: r.width, height: r.height };
}

function currentRect() {
  if (!host?.isConnected) return null;
  const flo = document.getElementById('scanFloat');
  const scan = flo && !flo.hidden ? { ...rectOf(flo), visible: true } : null;
  const modal = document.documentElement.dataset.modal === '1' || !!document.querySelector('.modal-backdrop');
  return cosersViewRect(rectOf(host), {
    rail: rectOf(document.getElementById('shell-rail')),
    top: rectOf(document.getElementById('shell-top')),
    scanFloat: scan,
    modal,
  });
}

function setFallback(on) {
  if (!fallback) return;
  fallback.hidden = !on;
}

async function place() {
  const ticket = epoch;
  const rect = currentRect();
  try {
    if (!rect) {
      await invoke('cosers_hide');
    } else {
      await invoke('cosers_show', rect);
    }
  } catch {
    if (ticket !== epoch) return;
    setFallback(true);
    return;
  }
  if (ticket !== epoch) {
    await invoke('cosers_close').catch(() => {});
    return;
  }
  setFallback(false);
}

function schedule() {
  if (epoch === -1 || raf) return;
  raf = requestAnimationFrame(() => {
    raf = 0;
    if (epoch === -1) return;
    void place();
  });
}

function stop() {
  epoch = -1;
  if (raf) cancelAnimationFrame(raf);
  raf = 0;
  watch?.disconnect();
  watch = null;
}

onPage(document, 'mh-cosers-release', () => { stop(); });
onPage(window, 'pagehide', () => {
  stop();
  void invoke('cosers_close').catch(() => {});
});
onPage(window, 'resize', () => { schedule(); });

if (host && typeof ResizeObserver !== 'undefined') {
  const ro = new ResizeObserver(() => { schedule(); });
  ro.observe(host);
  onPage(document, 'mh-cosers-release', () => { ro.disconnect(); });
}

watch = new MutationObserver(() => { schedule(); });
watch.observe(document.documentElement, { attributes: true, attributeFilter: ['data-modal'] });
watch.observe(document.body, { childList: true });
const flo = document.getElementById('scanFloat');
if (flo) watch.observe(flo, { attributes: true, attributeFilter: ['hidden'] });

document.getElementById('cosersRetry')?.addEventListener('click', () => {
  setFallback(false);
  schedule();
});

schedule();
