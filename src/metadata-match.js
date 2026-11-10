// metadata-match.js — online metadata search/bind dialog (glass modal).
import { t, esc, toast, openModal, promptDialog, confirmDialog } from './shell.js';
import { lib } from './api.js';

const errText = (e) => String(e?.detail ?? e?.message ?? e);

function providerLabel(p) {
  const key = `meta.provider.${p}`;
  const s = t(key);
  return s === key ? p : s;
}

function matchBadge(kind) {
  if (!kind || kind === 'none') return '';
  const label = t(`meta.match.${kind}`);
  return `<span class="chip chip--sm">${esc(label)}</span>`;
}

export async function openMetadataMatch(book, { onBound } = {}) {
  if (!book?.id) return false;
  const defaultQuery = book.name || '';
  const query = await promptDialog({
    title: t('meta.matchTitle'),
    message: t('meta.searchPrompt'),
    value: defaultQuery,
    ok: t('meta.search'),
    cancel: t('common.cancel'),
  });
  if (typeof query !== 'string' || !query.trim()) return false;

  const wrap = document.createElement('div');
  wrap.className = 'meta-match';
  const listHost = document.createElement('div');
  listHost.id = 'metaList';
  listHost.className = 'meta-list';
  const status = document.createElement('p');
  status.className = 'hint muted';
  status.id = 'metaStatus';
  status.textContent = t('common.loading');
  const optsRow = document.createElement('div');
  optsRow.className = 'meta-opts';
  optsRow.innerHTML = `
    <label class="check"><input type="checkbox" id="metaImportTags"> ${esc(t('meta.importTags'))}</label>
    <label class="check"><input type="checkbox" id="metaUseCover"> ${esc(t('meta.useRemoteCover'))}</label>`;
  wrap.append(status, listHost, optsRow);

  const modal = openModal({ title: t('meta.matchTitle'), body: wrap, wide: true });

  async function pickCandidate(provider, externalId, matchKind) {
    try {
      const detail = await lib.metadata.detail(provider, externalId);
      const msg = [
        detail.title,
        detail.titleCn,
        detail.authors?.length ? detail.authors.join(' · ') : '',
        detail.rating != null ? t('meta.rating', { n: detail.rating.toFixed(1) }) : '',
      ].filter(Boolean).join('\n');
      if (!await confirmDialog({
        title: t('meta.confirmBind'),
        message: msg,
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
    for (const g of groups) {
      const block = document.createElement('section');
      block.className = 'meta-group';
      const head = document.createElement('h3');
      head.textContent = providerLabel(g.provider);
      block.append(head);
      if (g.error) {
        const err = document.createElement('p');
        err.className = 'hint warn';
        err.textContent = g.error;
        block.append(err);
      }
      if (g.candidates?.length) {
        any = true;
        const ul = document.createElement('div');
        ul.className = 'meta-cands';
        for (const c of g.candidates) {
          const row = document.createElement('button');
          row.type = 'button';
          row.className = 'meta-cand btn btn--ghost';
          row.innerHTML = `
            <span class="grow"><b>${esc(c.title)}</b>
              ${c.originalTitle ? `<span class="muted"> · ${esc(c.originalTitle)}</span>` : ''}
            </span>
            ${matchBadge(c.matchKind)}
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
    const groups = await lib.metadata.search(query.trim());
    renderGroups(groups);
  } catch (e) {
    status.textContent = errText(e);
    toast(errText(e));
  }
  return true;
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
