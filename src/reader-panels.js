// reader-panels.js - the two small panels around the reading area:
//   HistoryStack : stacked reading-history cards in the left book panel (recently read books/episodes + progress, click to resume)
//   ThumbStrip   : page thumbnails in the bottom bar (current page highlighted, click to jump)
// No imports: everything it needs is injected, so it stays testable.

/** deps: { t, esc, coverUrl(id, ver), onOpen(bookId) } */
export class HistoryStack {
  constructor(host, deps) {
    this.host = host; this.d = deps;
    host.addEventListener('click', (e) => { const c = e.target.closest('.hcard'); if (c && !c.classList.contains('is-current')) this.d.onOpen(Number(c.dataset.id)); });
    host.addEventListener('keydown', (e) => { if ((e.key === 'Enter' || e.key === ' ') && e.target.classList?.contains('hcard') && !e.target.classList.contains('is-current')) { e.preventDefault(); this.d.onOpen(Number(e.target.dataset.id)); } });
  }
  card(b, ep, current) {
    const { t, esc } = this.d, pct = this.pct(b);
    const where = ep ? t('common.episodeOf', { i: ep.index, n: ep.total }) : (b.page != null ? t('common.pageOf', { p: b.page + 1, n: b.pageCount }) : t('lib.notStarted'));
    return `<article class="hcard${current ? ' is-current' : ''}" data-id="${b.id}" tabindex="${current ? -1 : 0}" role="button" aria-label="${esc(b.title)}">
      <img class="cover" alt="" src="${this.d.coverUrl(b.id, b.coverVer)}">
      <div class="hcard__body"><div class="hcard__title">${esc(b.title)}</div><div class="hcard__ep num" data-role="where">${esc(where)}</div>
        <div class="bar"><i data-role="bar" style="width:${pct}%"></i></div>
        <div class="hcard__meta"><span class="${current ? 'hcard__tag' : ''}">${esc(current ? t('reader.readingNow') : (b.lastReadAt != null ? this.d.ago(b.lastReadAt) : ''))}</span><span class="num" data-role="pct">${pct}%</span></div></div></article>`;
  }
  pct(b) { return b.page == null ? 0 : Math.min(100, Math.round(((b.page + (b.frac ?? 0)) / Math.max(1, b.pageCount)) * 100)); }
  /** current: {id,title,page,frac,pageCount,coverVer}; recent: other books (already without the current one); eps: episode info aligned with [current, ...recent] */
  render(current, recent, eps) {
    const { t } = this.d;
    this.host.innerHTML = this.card(current, eps[0], true) + (recent.length ? recent.map((b, i) => this.card(b, eps[i + 1], false)).join('') : `<p class="empty">${t('reader.historyEmpty')}</p>`);
  }
  /** Live update of the "reading now" card while the user reads (no re-render). */
  updateCurrent({ page, realPages, epText }) {
    const c = this.host.querySelector('.hcard.is-current'); if (!c) return;
    const pct = Math.min(100, Math.round(((page + 1) / Math.max(1, realPages)) * 100));
    const width = pct + '%';
    const bar = c.querySelector('[data-role="bar"]');
    const pctEl = c.querySelector('[data-role="pct"]');
    const where = c.querySelector('[data-role="where"]');
    if (bar && bar.style.width !== width) bar.style.width = width;
    if (pctEl && pctEl.textContent !== width) pctEl.textContent = width;
    if (where && where.textContent !== epText) where.textContent = epText;
  }
}

/** deps: { thumb(page) -> url, onJump(page) }. Shows pages [cur-2 .. cur+4]. */
export class ThumbStrip {
  constructor(host, deps) { this.host = host; this.d = deps; this.cur = -1; this._key = ''; host.addEventListener('click', (e) => { const b = e.target.closest('button'); if (b) this.d.onJump(Number(b.dataset.page)); }); }
  _range(page, realPages) {
    const from = Math.max(0, Math.min(page - 2, realPages - 7));
    const to = Math.min(realPages - 1, from + 6);
    return { from, to, key: `${from}:${to}:${realPages}` };
  }
  _html(from, to, page) {
    const out = [];
    for (let p = from; p <= to; p++) {
      const src = this.d.thumb(p) || '';
      out.push(`<button data-page="${p}" class="${p === page ? 'is-active' : ''}" aria-label="${p + 1}"><img alt="" loading="lazy" decoding="async" src="${src}"></button>`);
    }
    return out.join('');
  }
  update(page, realPages) {
    if (!(realPages > 0)) { this.reset(); return; }   // not ready / empty book: skip thumbs, don't throw
    const { from, to, key } = this._range(page, realPages);
    if (page === this.cur && this._key === key) return;
    const reuse = typeof document !== 'undefined' && typeof this.host.querySelector === 'function';
    if (reuse && this._key === key) {
      this.cur = page;
      for (const b of this.host.querySelectorAll('button')) b.classList.toggle('is-active', Number(b.dataset.page) === page);
      return;
    }
    if (reuse) {
      const have = new Map();
      for (const b of this.host.querySelectorAll('button')) have.set(Number(b.dataset.page), b);
      const frag = document.createDocumentFragment();
      for (let p = from; p <= to; p++) {
        let b = have.get(p);
        if (!b) {
          b = document.createElement('button');
          b.dataset.page = String(p);
          b.setAttribute('aria-label', String(p + 1));
          const img = document.createElement('img');
          img.alt = '';
          img.loading = 'lazy';
          img.decoding = 'async';
          const src = this.d.thumb(p) || '';
          if (src) img.src = src;
          b.append(img);
        }
        b.classList.toggle('is-active', p === page);
        frag.append(b);
        have.delete(p);
      }
      this.host.replaceChildren(frag);
    } else {
      this.host.innerHTML = this._html(from, to, page);
    }
    this.cur = page;
    this._key = key;
  }
  reset() { this.cur = -1; this._key = ''; this.host.innerHTML = ''; }
}
