// bookmarks.js - image-anchored bookmarks: pure helpers + the side-panel list. No imports (deps are injected),
// so the helpers are unit-testable in Node. Bookmarks never touch the strip; they only read positions and call jumpTo().
//
// A bookmark from the backend: { id, bookId, entryName, fileName, pageHint, page|null, frac, note|null, createdAt, status:'ok'|'broken' }
//   page  = CURRENT real page of that image, found by NAME (null => broken: the image is gone from the book)
//   frac  = 0..1 down the WHOLE image (all slices of a tall image), same unit as layout.js positions

export const NEAR_MIN_FRAC = 0.01;

/** Position at the viewport CENTER: the real page under it + the offset within that whole image (slices included). */
export function centerPosition(layout, scrollTop, viewH) {
  return layout.getPosition(scrollTop + viewH / 2);
}

/** Existing OK bookmark on the same image within half a viewport of `pos` (used to toggle B / the button). */
export function findNearby(bms, pos, layout, viewH) {
  if (!layout.n || pos.page >= layout.realPages) return null;
  const tol = Math.max(NEAR_MIN_FRAC, viewH / 2 / layout.groupBounds(pos.page).height);
  return bms.find(b => b.status === 'ok' && b.page === pos.page && Math.abs(b.frac - pos.frac) <= tol) ?? null;
}

/** Virtual page (slice) that contains the bookmarked spot - the thumbnail shows the part you bookmarked. */
export function thumbVirtualIndex(layout, bm) {
  return layout.findVirtual(layout.posToY({ page: bm.page, frac: bm.frac }));
}

/** Scrubber tick fractions, matching the thumb mapping page/(realPages-1). Broken bookmarks get no tick. */
export function bookmarkTicks(bms, realPages) {
  if (realPages < 2) return [];
  const fr = bms.filter(b => b.status === 'ok' && b.page != null).map(b => b.page / (realPages - 1));
  return [...new Set(fr)];
}

// ---------------------------------------------------------------------------------------------------------
// BookmarkPanel: the "Bookmarks" tab body. Each row: thumbnail OF THAT IMAGE, entry filename, episode/page label, note.
// Click row (OK)  -> onJump(bm): exact image + offset.
// Broken row      -> never jumps on click. Offers explicit choices: go to the OLD position, or anchor to it.
// deps = { t, esc, thumb(bm)->url, label(bm)->string, onJump, onJumpHint, onReanchor, onDelete(id), onNote(id, note|null) }
export class BookmarkPanel {
  constructor(host, deps) {
    this.host = host; this.d = deps; this.bms = [];
    host.addEventListener('click', (e) => this.onClick(e));
  }

  render(bms) {
    const { t, esc } = this.d;
    this.bms = bms;
    this.host.innerHTML = bms.length ? bms.map(b => this.row(b)).join('') : `<p class="empty">${esc(t('bmEmpty'))}</p>`;
  }

  row(b) {
    const { t, esc } = this.d, broken = b.status === 'broken';
    const thumb = broken
      ? `<div class="bm__thumb bm__thumb--broken" aria-hidden="true">!</div>`
      : `<img class="bm__thumb" alt="" loading="lazy" src="${esc(this.d.thumb(b))}">`;
    const where = broken ? `<span class="bm__warn">${esc(t('bmBroken'))}</span>` : esc(this.d.label(b));
    const note = `<div class="bm__note" data-act="note">${b.note ? esc(b.note) : `<i>${esc(t('bmNote'))}</i>`}</div>`;
    const actions = broken
      ? `<button data-act="hint">${esc(t('bmGoOld', { p: b.pageHint + 1 }))}</button><button data-act="reanchor">${esc(t('bmReanchor', { p: b.pageHint + 1 }))}</button>`
      : '';
    return `<article class="bm${broken ? ' bm--broken' : ''}" data-id="${b.id}">${thumb}
      <div class="bm__main"><div class="bm__file" title="${esc(b.entryName)}">${esc(b.fileName)}</div>
      <div class="bm__where">${where}</div>${note}
      <div class="bm__actions">${actions}<button data-act="delete">${esc(t('bmDelete'))}</button></div></div></article>`;
  }

  onClick(e) {
    const card = e.target.closest('.bm'); if (!card) return;
    const bm = this.bms.find(b => b.id === +card.dataset.id); if (!bm) return;
    const act = e.target.closest('[data-act]')?.dataset.act;
    if (act === 'delete') return this.d.onDelete(bm.id);
    if (act === 'hint') return this.d.onJumpHint(bm);                 // explicit user choice
    if (act === 'reanchor') return this.d.onReanchor(bm);             // explicit user choice
    if (act === 'note') return this.editNote(e.target.closest('[data-act="note"]'), bm);
    if (bm.status === 'ok') this.d.onJump(bm);                         // broken rows never jump on a plain click
  }

  editNote(el, bm) {
    const input = Object.assign(document.createElement('input'), { className: 'field bm__input', value: bm.note ?? '', maxLength: 500, placeholder: this.d.t('bmNote') });
    el.replaceWith(input); input.focus();
    let done = false;
    const commit = (save) => { if (done) return; done = true; save ? this.d.onNote(bm.id, input.value.trim() || null) : this.render(this.bms); };
    input.addEventListener('keydown', (e) => { if (e.key === 'Enter') commit(true); if (e.key === 'Escape') { e.stopPropagation(); commit(false); } });
    input.addEventListener('blur', () => commit(true));
  }
}
