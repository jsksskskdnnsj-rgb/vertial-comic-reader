// merge-ui.js — confirm or rearrange a group of folders that look like one comic.
// Reordering is local until Merge; the backend then rewrites shelf rows only.
import { invoke, t, esc, toast, openModal } from './shell.js';
import { coverUrl, COVER_BASE } from './shell-ui.js';

function thumb(m) {
  if (m.shelf_id && COVER_BASE) {
    return `<img alt="" src="${coverUrl(m.shelf_id, m.cover_ver || 0)}" data-cover="${m.shelf_id}">`;
  }
  const ch = esc((m.name || '?').trim().slice(0, 1) || '?');
  return `<span class="merge-thumb" aria-hidden="true">${ch}</span>`;
}

function metaLine(m) {
  const bits = [t('merge.pages', { n: m.pages || 0 })];
  if (m.chapters) bits.push(t('merge.chapters', { n: m.chapters }));
  if (m.order_label) bits.push(m.order_label);
  return bits.join(' · ');
}

/**
 * groups: pending rows from list_merge_pending, or one handmade group (no id) for a manual merge.
 * folders: {id, name}[]. preview: reorder locally and skip invoke (static screenshots).
 */
export function openMergeDialog({ groups = [], folders = [], preview = false, onDone = null } = {}) {
  const state = (groups || []).map((g) => ({
    id: g.id || 0,
    confident: !!g.confident,
    title: g.suggested_title || '',
    folderId: '',
    members: (g.members || []).map((m, i) => ({ ...m, _i: i })),
  }));
  const root = document.createElement('div');
  root.className = 'merge-dialog';
  const help = document.createElement('p');
  help.className = 'hint';
  help.textContent = t('merge.help');
  const list = document.createElement('div');
  list.className = 'merge-list';
  root.append(help, list);

  const folderOpts = `<option value="">${esc(t('lib.noFolder'))}</option>` +
    (folders || []).map((f) => `<option value="${f.id}">${esc(f.name)}</option>`).join('');

  function paintRows(g, box) {
    box.innerHTML = '';
    g.members.forEach((m, idx) => {
      const row = document.createElement('div');
      row.className = 'merge-row';
      row.draggable = true;
      row.innerHTML = `
        ${thumb(m)}
        <span class="merge-row__main">
          <b title="${esc(m.name)}">${esc(m.name)}</b>
          <span class="meta num">${esc(metaLine(m))}</span>
        </span>
        <span class="merge-row__move">
          <button type="button" class="ibtn" data-up="${idx}" aria-label="${esc(t('merge.up'))}" title="${esc(t('merge.up'))}"><svg class="i"><use href="icons.svg#chevron-up"/></svg></button>
          <button type="button" class="ibtn" data-down="${idx}" aria-label="${esc(t('merge.down'))}" title="${esc(t('merge.down'))}"><svg class="i"><use href="icons.svg#chevron-down"/></svg></button>
        </span>`;
      row.addEventListener('dragstart', (e) => {
        row.classList.add('is-drag');
        e.dataTransfer?.setData('text/plain', String(idx));
        if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
      });
      row.addEventListener('dragend', () => row.classList.remove('is-drag'));
      row.addEventListener('dragover', (e) => { e.preventDefault(); if (e.dataTransfer) e.dataTransfer.dropEffect = 'move'; });
      row.addEventListener('drop', (e) => {
        e.preventDefault();
        const src = Number(e.dataTransfer?.getData('text/plain'));
        if (!Number.isFinite(src) || src === idx || src < 0 || src >= g.members.length) return;
        const [item] = g.members.splice(src, 1);
        const dest = src < idx ? idx - 1 : idx;
        g.members.splice(dest, 0, item);
        paintRows(g, box);
      });
      box.append(row);
    });
  }

  function removeCard(card, g) {
    const i = state.indexOf(g);
    if (i >= 0) state.splice(i, 1);
    card.remove();
    if (!state.length) modal.close();
  }

  state.forEach((g) => {
    const card = document.createElement('section');
    card.className = 'merge-card';
    const badge = document.createElement('p');
    badge.className = 'merge-flag';
    badge.textContent = t(g.confident ? 'merge.confident' : 'merge.uncertain');
    const title = document.createElement('label');
    title.className = 'merge-field';
    title.innerHTML = `<span>${esc(t('merge.rename'))}</span>`;
    const input = document.createElement('input');
    input.className = 'field';
    input.value = g.title;
    input.addEventListener('input', () => { g.title = input.value; });
    title.append(input);
    const cat = document.createElement('label');
    cat.className = 'merge-field';
    cat.innerHTML = `<span>${esc(t('merge.category'))}</span>`;
    const sel = document.createElement('select');
    sel.className = 'select';
    sel.innerHTML = folderOpts;
    sel.addEventListener('change', () => { g.folderId = sel.value; });
    cat.append(sel);
    const rows = document.createElement('div');
    rows.className = 'merge-rows';
    rows.addEventListener('click', (e) => {
      const up = e.target.closest('[data-up]');
      const down = e.target.closest('[data-down]');
      if (!up && !down) return;
      e.preventDefault();
      const idx = Number((up || down).dataset.up ?? (up || down).dataset.down);
      const j = up ? idx - 1 : idx + 1;
      if (j < 0 || j >= g.members.length) return;
      const tmp = g.members[idx];
      g.members[idx] = g.members[j];
      g.members[j] = tmp;
      paintRows(g, rows);
    });
    paintRows(g, rows);
    const actions = document.createElement('div');
    actions.className = 'modal__actions';
    const mergeBtn = document.createElement('button');
    mergeBtn.type = 'button';
    mergeBtn.className = 'btn btn--primary btn--sm';
    mergeBtn.textContent = t('merge.merge');
    const sepBtn = document.createElement('button');
    sepBtn.type = 'button';
    sepBtn.className = 'btn btn--ghost btn--sm';
    sepBtn.textContent = t('merge.separate');
    sepBtn.hidden = !g.id;
    mergeBtn.addEventListener('click', async () => {
      if (preview) { toast(t('merge.done')); return; }
      mergeBtn.disabled = true;
      const folderId = g.folderId ? Number(g.folderId) : null;
      try {
        if (g.id) {
          await invoke('resolve_merge_pending', {
            id: g.id,
            action: 'merge',
            title: g.title.trim(),
            folderId,
            order: g.members.map((m) => m._i),
          });
        } else {
          await invoke('merge_books', {
            title: g.title.trim(),
            folderId,
            members: g.members.map((m) => ({
              shelfId: m.shelf_id,
              path: m.path || '',
              name: m.name || '',
              pages: m.pages || 0,
            })),
          });
        }
        toast(t('merge.done'));
        removeCard(card, g);
      } catch (e) {
        toast(String(e?.message || e));
        mergeBtn.disabled = false;
      }
    });
    sepBtn.addEventListener('click', async () => {
      if (preview) { removeCard(card, g); return; }
      sepBtn.disabled = true;
      try {
        await invoke('resolve_merge_pending', {
          id: g.id,
          action: 'separate',
          title: g.title.trim(),
          folderId: g.folderId ? Number(g.folderId) : null,
          order: [],
        });
        toast(t('merge.done'));
        removeCard(card, g);
      } catch (e) {
        toast(String(e?.message || e));
        sepBtn.disabled = false;
      }
    });
    actions.append(sepBtn, mergeBtn);
    card.append(badge, title, cat, rows, actions);
    list.append(card);
  });

  const modal = openModal({ title: t('merge.title'), body: root, wide: true, onClose: () => onDone?.() });
  modal.element.classList.add('modal--merge');
  return modal;
}
