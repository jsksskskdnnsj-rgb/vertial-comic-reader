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
    c.querySelector('[data-role="bar"]').style.width = pct + '%'; c.querySelector('[data-role="pct"]').textContent = pct + '%'; c.querySelector('[data-role="where"]').textContent = epText;
  }
}

/** deps: { thumb(page) -> url, onJump(page) }. Shows pages [cur-2 .. cur+4]. */
export class ThumbStrip {
  constructor(host, deps) { this.host = host; this.d = deps; this.cur = -1; host.addEventListener('click', (e) => { const b = e.target.closest('button'); if (b) this.d.onJump(Number(b.dataset.page)); }); }
  update(page, realPages) {
    if (page === this.cur) return; this.cur = page;
    const from = Math.max(0, Math.min(page - 2, realPages - 7)), to = Math.min(realPages - 1, from + 6), out = [];
    for (let p = from; p <= to; p++) out.push(`<button data-page="${p}" class="${p === page ? 'is-active' : ''}" aria-label="${p + 1}"><img alt="" loading="lazy" src="${this.d.thumb(p)}"></button>`);
    this.host.innerHTML = out.join('');
  }
  reset() { this.cur = -1; this.host.innerHTML = ''; }
}
