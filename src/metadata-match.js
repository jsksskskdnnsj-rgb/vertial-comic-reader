// metadata-match.js — pick a metadata hit for a book already on the shelf, or type the fields and lock them.
import { t, esc, toast, openModal, promptDialog, confirmDialog } from './shell.js';
import { lib } from './api.js';
import { scoreTitle } from './title-score.js';
import { remoteImgSrc } from './remote-img.js';

const errText = (e) => String(e?.detail ?? e?.message ?? e);

function providerLabel(p) {
  const key = `meta.provider.${p}`;
  const s = t(key);
  return s === key ? p : s;
}

function splitList(s) {
  return String(s || '').split(/[、,，]/).map((x) => x.trim()).filter(Boolean);
}

function confidenceOf(query, c) {
  if (c?.confidence != null && c.confidence !== '') return Number(c.confidence) || 0;
  return scoreTitle(query, {
    title: c?.title,
    originalTitle: c?.originalTitle,
    aliases: c?.aliases || [],
  }).confidence;
}

export async function openMetadataMatch(book, { onBound } = {}) {
  if (!book?.id) return false;
  const query = await promptDialog({
    title: t('meta.matchTitle'),
    message: t('meta.searchPrompt'),
    value: book.name || '',
    ok: t('meta.search'),
    cancel: t('common.cancel'),
  });
  if (typeof query !== 'string' || !query.trim()) return false;
  const q = query.trim();

  const wrap = document.createElement('div');
  wrap.className = 'meta-match';
  const listHost = document.createElement('div');
  listHost.className = 'meta-list';
  const status = document.createElement('p');
  status.className = 'hint muted';
  status.textContent = t('common.loading');
  const optsRow = document.createElement('div');
  optsRow.className = 'meta-opts';
  optsRow.innerHTML = `
    <label class="check"><input type="checkbox" id="metaImportTags"> ${esc(t('meta.importTags'))}</label>
    <label class="check"><input type="checkbox" id="metaUseCover"> ${esc(t('meta.useRemoteCover'))}</label>
    <button class="btn btn--ghost btn--sm" id="metaHand" type="button">${esc(t('meta.handEdit'))}</button>`;
  wrap.append(status, listHost, optsRow);
  const modal = openModal({ title: t('meta.matchTitle'), body: wrap, wide: true });
  wrap.querySelector('#metaHand').onclick = async () => {
    modal.close();
    if (await openMetadataEditor(book, { onBound })) onBound?.();
  };

  async function pickCandidate(provider, externalId, matchKind) {
    try {
      const detail = await lib.metadata.detail(book.id, provider, externalId);
      const lines = [
        detail.title,
        detail.titleCn,
        detail.authors?.length ? detail.authors.join(' · ') : '',
        detail.year ? String(detail.year) : '',
        detail.status || '',
        detail.rating != null ? t('meta.rating', { n: Number(detail.rating).toFixed(1) }) : '',
        detail.summary ? detail.summary.slice(0, 180) : '',
      ].filter(Boolean).join('\n');
      if (!await confirmDialog({
        title: t('meta.confirmBind'),
        message: lines,
        ok: t('meta.bind'),
        cancel: t('common.cancel'),
      })) return;
      const opts = {
        importTags: document.getElementById('metaImportTags')?.checked ?? false,
        useRemoteCover: document.getElementById('metaUseCover')?.checked ?? false,
      };
      await lib.metadata.bind(book.id, provider, externalId, matchKind || 'manual', opts);
      toast(t('meta.bound'));
      modal.close();
      onBound?.();
    } catch (e) {
      toast(errText(e));
    }
  }

  function renderGroups(groups) {
    listHost.replaceChildren();
    let any = false;
    for (const g of groups || []) {
      const block = document.createElement('section');
      block.className = 'meta-group';
      const head = document.createElement('h3');
      head.textContent = g.label || providerLabel(g.provider);
      block.append(head);
      if (g.error) {
        const err = document.createElement('p');
        err.className = 'hint warn';
        err.textContent = g.error;
        block.append(err);
      }
      const cands = [...(g.candidates || [])].sort((a, b) => confidenceOf(q, b) - confidenceOf(q, a));
      if (cands.length) {
        any = true;
        const ul = document.createElement('div');
        ul.className = 'meta-cands';
        for (const c of cands) {
          const row = document.createElement('button');
          row.type = 'button';
          row.className = 'meta-cand btn btn--ghost';
          const conf = confidenceOf(q, c);
          const extra = [c.originalTitle, c.releaseDate].filter(Boolean).join(' · ');
          const cover = remoteImgSrc(c.coverUrl);
          row.innerHTML = `
            ${cover ? `<img class="meta-cover" alt="" src="${esc(cover)}">` : ''}
            <span class="grow"><b>${esc(c.title)}</b>
              ${extra ? `<span class="muted"> · ${esc(extra)}</span>` : ''}
            </span>
            <span class="meta-score">${esc(t('meta.confidence', { n: conf }))}</span>
            <span class="muted">${esc(providerLabel(c.provider))}</span>`;
          row.onclick = () => pickCandidate(c.provider, c.externalId, c.matchKind);
          ul.append(row);
        }
        block.append(ul);
      } else if (!g.error) {
        const empty = document.createElement('p');
        empty.className = 'muted';
        empty.textContent = t('meta.noHits');
        block.append(empty);
      }
      listHost.append(block);
    }
    status.textContent = any ? t('meta.pickOne') : t('meta.noResults');
  }

  try {
    const groups = await lib.metadata.search(book.id, q);
    renderGroups(groups);
  } catch (e) {
    status.textContent = errText(e);
    toast(errText(e));
  }
  return true;
}

export async function openMetadataEditor(book, { bound = null, onBound } = {}) {
  if (!book?.id) return false;
  const current = bound || await lib.metadata.get(book.id).catch(() => null);
  const wrap = document.createElement('form');
  wrap.className = 'meta-form';
  const field = (id, label, value, opts = {}) => {
    const lab = document.createElement('label');
    lab.textContent = label;
    const input = document.createElement(opts.area ? 'textarea' : 'input');
    input.id = id;
    input.className = 'field';
    if (!opts.area) input.type = opts.type || 'text';
    input.value = value ?? '';
    if (opts.placeholder) input.placeholder = opts.placeholder;
    lab.append(input);
    wrap.append(lab);
    return input;
  };
  field('mfTitle', t('meta.fieldTitle'), current?.titleDisplay || book.name || '');
  field('mfCn', t('meta.fieldTitleCn'), current?.titleCn || '');
  field('mfOrig', t('meta.fieldOriginal'), current?.titleOriginal || '');
  field('mfAuthors', t('meta.fieldAuthors'), (current?.authors || []).join('、'), { placeholder: t('meta.authorsPh') });
  field('mfTags', t('meta.fieldTags'), (current?.tags || []).join('、'), { placeholder: t('meta.tagsPh') });
  field('mfStatus', t('meta.fieldStatus'), current?.status || '');
  field('mfYear', t('meta.fieldYear'), current?.year ?? '', { type: 'number' });
  field('mfCh', t('meta.fieldChapters'), current?.chapterCount ?? '', { type: 'number' });
  field('mfVol', t('meta.fieldVolumes'), current?.volumeCount ?? '', { type: 'number' });
  field('mfDate', t('meta.fieldDate'), current?.releaseDate || '');
  field('mfSum', t('meta.fieldSummary'), current?.summary || '', { area: true });
  const lockLab = document.createElement('label');
  lockLab.className = 'check';
  lockLab.innerHTML = `<input type="checkbox" id="mfLock" checked> ${esc(t('meta.lock'))}`;
  wrap.append(lockLab);
  const actions = document.createElement('div');
  actions.className = 'modal__actions';
  const cancel = document.createElement('button');
  cancel.type = 'button';
  cancel.className = 'btn btn--ghost';
  cancel.textContent = t('common.cancel');
  const ok = document.createElement('button');
  ok.type = 'submit';
  ok.className = 'btn btn--primary';
  ok.textContent = t('common.save');
  actions.append(cancel, ok);
  wrap.append(actions);
  const modal = openModal({ title: t('meta.editTitle'), body: wrap, wide: true });
  cancel.onclick = () => modal.close();
  wrap.onsubmit = async (e) => {
    e.preventDefault();
    const num = (id) => {
      const v = wrap.querySelector('#' + id).value.trim();
      if (!v) return null;
      const n = Number(v);
      return Number.isFinite(n) ? n : null;
    };
    const patch = {
      titleDisplay: wrap.querySelector('#mfTitle').value.trim(),
      titleCn: wrap.querySelector('#mfCn').value.trim(),
      titleOriginal: wrap.querySelector('#mfOrig').value.trim(),
      authors: splitList(wrap.querySelector('#mfAuthors').value),
      tags: splitList(wrap.querySelector('#mfTags').value),
      status: wrap.querySelector('#mfStatus').value.trim(),
      year: num('mfYear'),
      chapterCount: num('mfCh'),
      volumeCount: num('mfVol'),
      releaseDate: wrap.querySelector('#mfDate').value.trim(),
      summary: wrap.querySelector('#mfSum').value.trim(),
      lock: wrap.querySelector('#mfLock').checked,
    };
    try {
      await lib.metadata.saveManual(book.id, patch);
      toast(t('meta.manualSaved'));
      modal.close();
      onBound?.();
    } catch (err) {
      toast(errText(err));
    }
  };
  return true;
}

export async function refreshMetadata(book, { onBound } = {}) {
  if (!book?.id) return false;
  try {
    const res = await lib.metadata.refresh(book.id);
    toast(res?.refreshed ? t('meta.refreshDone') : (res?.message || t('meta.refreshLocked')));
    onBound?.();
    return true;
  } catch (e) {
    toast(errText(e));
    return false;
  }
}

export async function unbindMetadata(book, { onBound } = {}) {
  if (!book?.id) return false;
  if (!await confirmDialog({ title: t('meta.unbind'), message: t('meta.unbindMsg'), ok: t('meta.unbind'), danger: true })) return false;
  try {
    await lib.metadata.unbind(book.id);
    toast(t('common.saved'));
    onBound?.();
    return true;
  } catch (e) {
    toast(errText(e));
    return false;
  }
}
