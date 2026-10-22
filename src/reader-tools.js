// reader-tools.js - reader features restored from v0.3.2:
//   AutoScroll        : continuous scroll (strip) / timed page turns (flip), speed 1..10
//   openPageDirectory : thumbnail grid of every page: tap to jump, select + delete, smart sort, reverse order, append pages
// Backend calls arrive through `ctx.lib` (legacy-api.js), so the real command names live in ONE file.
import { openModal, confirmDialog, choiceDialog, toast, T, esc, t } from './shell.js';
import { sortSources } from './naming.js';

// ---------------------------------------------------------------------------------------------------- AutoScroll
const PX_PER_SEC = [20, 35, 55, 80, 115, 160, 220, 300, 400, 520];          // strip mode, speed 1..10
const SEC_PER_PAGE = [12, 9, 7, 5.5, 4.5, 3.5, 2.8, 2.2, 1.6, 1.1];          // flip mode, speed 1..10
export class AutoScroll {
  /** deps: { getView(), onChange(running) } */
  constructor(deps) { this.d = deps; this.speed = 4; this.running = false; this.raf = 0; this.timer = 0; }
  setSpeed(n) { this.speed = Math.min(10, Math.max(1, Math.round(n))); if (this.running) { this.stop(false); this.start(); } }
  toggle() { this.running ? this.stop() : this.start(); }
  start() {
    const v = this.d.getView(); if (!v || this.running) return; this.running = true; this.d.onChange(true);
    if (v.mode === 'flip') { this.timer = setInterval(() => { const p = v.topPos(); v.next(); if (v.page === v.layout.realPages - 1 && !v.scrollable()) this.stop(); }, SEC_PER_PAGE[this.speed - 1] * 1000); return; }
    const sc = v.strip.scroller; let last = performance.now(), pos = sc.scrollTop;
    const tick = (now) => {
      if (!this.running) return;
      if (Math.abs(sc.scrollTop - Math.round(pos)) > 3) pos = sc.scrollTop;                       // the user moved it: follow, don't fight
      pos += PX_PER_SEC[this.speed - 1] * (now - last) / 1000; last = now; sc.scrollTop = pos;
      if (sc.scrollTop + sc.clientHeight >= sc.scrollHeight - 2) return this.stop();               // end of book
      this.raf = requestAnimationFrame(tick);
    };
    this.raf = requestAnimationFrame(tick);
  }
  stop(notify = true) { if (!this.running) return; this.running = false; cancelAnimationFrame(this.raf); clearInterval(this.timer); if (notify) this.d.onChange(false); }
}

// ------------------------------------------------------------------------------------------------ Page directory
/** ctx: { bookId, realPages, current, thumb(page)->url, lib, onJump(page), onChanged(), title } */
export function openPageDirectory(ctx) {
  const body = document.createElement('div'); body.className = 'dir';
  const bar = document.createElement('div'); bar.className = 'dir__bar';
  const mk = (icon, label, kind = 'ghost') => { const b = document.createElement('button'); b.className = `btn btn--${kind} btn--sm`; b.innerHTML = `<svg class="i"><use href="icons.svg#${icon}"/></svg><span>${esc(label)}</span>`; return b; };
  const bSel = mk('check', t('dir.select')), bSmart = mk('wand', t('dir.smartSort')), bRev = mk('reverse', t('dir.reverse')), bAdd = mk('plus', t('dir.append')), bDel = mk('trash', t('dir.deleteSel', { n: 0 }), 'danger');
  bDel.hidden = true; const info = document.createElement('span'); info.className = 'muted num'; info.style.marginLeft = 'auto'; info.textContent = t('dir.count', { n: ctx.realPages });
  bar.append(bSel, bSmart, bRev, bAdd, bDel, info);
  const grid = document.createElement('div'); grid.className = 'dir__grid';
  grid.innerHTML = Array.from({ length: ctx.realPages }, (_, p) => `<div class="dir__item${p === ctx.current ? ' is-current' : ''}" data-p="${p}"><img alt="" loading="lazy" src="${ctx.thumb(p)}"><span class="dir__chk"><svg class="i"><use href="icons.svg#check"/></svg></span><span class="dir__n num">${p + 1}</span></div>`).join('');
  body.append(bar, grid);
  const m = openModal({ title: t('dir.title', { title: ctx.title }), body, wide: true });
  const sel = new Set();
  const upd = () => { bDel.hidden = !body.classList.contains('selecting'); bDel.querySelector('span').textContent = t('dir.deleteSel', { n: sel.size }); bDel.disabled = sel.size === 0; };
  bSel.addEventListener('click', () => { body.classList.toggle('selecting'); sel.clear(); grid.querySelectorAll('.is-sel').forEach(e => e.classList.remove('is-sel')); upd(); });
  grid.addEventListener('click', (e) => {
    const it = e.target.closest('.dir__item'); if (!it) return; const p = Number(it.dataset.p);
    if (body.classList.contains('selecting')) { sel.has(p) ? sel.delete(p) : sel.add(p); it.classList.toggle('is-sel', sel.has(p)); upd(); }
    else { m.close(); ctx.onJump(p); }                                              // tap a thumbnail = jump there
  });
  const run = async (fn, doneKey) => { try { await fn(); m.close(); toast(t(doneKey)); await ctx.onChanged(); } catch (e) { toast(t('dir.failed', { e: e?.detail ?? e?.message ?? e })); } };
  bSmart.addEventListener('click', async () => { if (await confirmDialog({ title: t('dir.smartSort'), message: t('dir.smartSortMsg'), ok: t('dir.smartSort') })) run(() => ctx.lib.reorderPages(ctx.bookId, 'smart'), 'dir.sorted'); });
  bRev.addEventListener('click', async () => { if (await confirmDialog({ title: t('dir.reverse'), message: t('dir.reverseMsg'), ok: t('dir.reverse') })) run(() => ctx.lib.reorderPages(ctx.bookId, 'reverse'), 'dir.reversed'); });
  bDel.addEventListener('click', async () => { if (sel.size && await confirmDialog({ title: t('dir.delete'), message: t('dir.deleteMsg', { n: sel.size }), ok: t('common.delete'), danger: true })) run(() => ctx.lib.deletePages(ctx.bookId, [...sel].sort((a, b) => a - b)), 'dir.deleted'); });
  bAdd.addEventListener('click', async () => {                                       // append-merge: more pages, ordered by the numeric range in their names
    const kind = await choiceDialog({ title: t('dir.append'), message: t('dir.appendMsg'), choices: [{ label: t('common.cancel'), value: null }, { label: t('imp.addFolders'), value: 'folder' }, { label: t('imp.addZips'), value: 'files', kind: 'primary' }] });
    if (!kind) return;
    const picked = [].concat(await T.dialog.open(kind === 'folder' ? { directory: true, multiple: true } : { multiple: true, filters: [{ name: t('imp.zipFilter'), extensions: ['zip', 'cbz'] }, { name: t('imp.imageFilter'), extensions: ['jpg', 'jpeg', 'png', 'webp', 'gif', 'avif', 'bmp'] }] }) ?? []);
    if (picked.length) run(() => ctx.lib.appendPages(ctx.bookId, sortSources(picked)), 'dir.appended');
  });
  return m;
}
