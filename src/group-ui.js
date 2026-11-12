// Edit a folder group: remove, reorder, move, add, split, undo, restore auto-detect.
// Disk files are never deleted. The existing merge dialog stays as it is.
import { invoke, t, esc, toast, openModal } from './shell.js';
import { eachAlone, moveIndex, removePlan, splitPlan } from './group-edit.js';

const errText = (e) => String(e?.message || e || '');

async function describe(bookId) {
  return invoke('group_describe', { bookId });
}

async function focusResult(result, preferKey) {
  let best = null;
  for (const id of result?.bookIds || []) {
    const view = await describe(id).catch(() => null);
    if (!view) continue;
    if (preferKey && view.parts.some((p) => p.key === preferKey)) return view;
    if (!best || view.parts.length > best.parts.length) best = view;
  }
  return best;
}

export async function openGroupEditor({ bookId, books = null, onDone = null } = {}) {
  let view;
  try { view = await describe(bookId); }
  catch (e) { toast(errText(e)); return null; }
  let shelf = books;
  if (!shelf) shelf = await invoke('shelf_list').catch(() => []);

  const root = document.createElement('div');
  root.className = 'merge-dialog';
  const help = document.createElement('p');
  help.className = 'hint';
  help.textContent = t('group.help');
  const note = document.createElement('p');
  note.className = 'hint muted';
  note.textContent = t('group.filesStay');
  const list = document.createElement('div');
  list.className = 'merge-list';
  const tools = document.createElement('div');
  tools.className = 'meta-tools';
  root.append(help, note, list, tools);

  const selected = () => [...list.querySelectorAll('input[data-key]:checked')].map((el) => el.dataset.key);

  function targets() {
    return (shelf || []).filter((b) => b.id !== view.bookId && (b.kind === 'folder' || b.merged));
  }

  function paint() {
    list.innerHTML = '';
    if (view.manual) {
      const manual = document.createElement('p');
      manual.className = 'hint';
      manual.textContent = t('group.manual');
      list.append(manual);
    }
    view.parts.forEach((part, idx) => {
      const row = document.createElement('div');
      row.className = 'merge-row';
      const path = (part.paths || [])[0] || '';
      row.innerHTML = `
        <label class="check"><input type="checkbox" data-key="${esc(part.key)}"></label>
        <span class="merge-row__main">
          <b title="${esc(part.name)}">${esc(part.name)}</b>
          <span class="meta num">${esc(path)}</span>
        </span>
        <button type="button" class="btn btn--ghost btn--sm merge-apart" data-alone="${idx}" ${view.parts.length < 2 ? 'disabled' : ''}>${esc(t('merge.separateOne'))}</button>
        <span class="merge-row__move">
          <button type="button" class="ibtn" data-up="${idx}" aria-label="${esc(t('merge.up'))}"><svg class="i"><use href="icons.svg#chevron-up"/></svg></button>
          <button type="button" class="ibtn" data-down="${idx}" aria-label="${esc(t('merge.down'))}"><svg class="i"><use href="icons.svg#chevron-down"/></svg></button>
        </span>`;
      list.append(row);
    });
    tools.innerHTML = '';
    const addBtn = button(t('group.remove'), async () => {
      const plan = removePlan(view.parts, selected());
      if (!plan.ok) { toast(t(plan.reason === 'none' ? 'group.pick' : 'group.needPart')); return; }
      const prefer = plan.stay[0]?.key;
      await run(() => invoke('group_remove_members', { bookId: view.bookId, pathKeys: plan.gone.map((p) => p.key) }), prefer, t('group.removed'));
    });
    const splitBtn = button(t('group.split'), async () => {
      const plan = splitPlan(view.parts, selected());
      if (!plan.ok) { toast(t('group.needPart')); return; }
      await run(() => invoke('group_split', { bookId: view.bookId, groups: plan.groups }), plan.groups[1][0], t('group.removed'));
    });
    const aloneBtn = button(t('group.alone'), async () => {
      const plan = eachAlone(view.parts);
      if (!plan.ok) { toast(t('group.needPart')); return; }
      await run(() => invoke('group_split', { bookId: view.bookId, groups: plan.groups }), null, t('group.removed'));
    });
    const undoBtn = button(t('group.undo'), async () => {
      await run(() => invoke('group_undo'), null, t('group.undone'));
    });
    undoBtn.disabled = !view.canUndo;
    const restoreBtn = button(t('group.restore'), async () => {
      await run(() => invoke('group_restore_auto', { bookId: view.bookId }), null, t('group.restored'));
    });
    const grouped = view.parts.length >= 2;
    addBtn.disabled = !grouped;
    splitBtn.disabled = !grouped;
    aloneBtn.disabled = !grouped;
    tools.append(addBtn, splitBtn, aloneBtn, undoBtn, restoreBtn);

    const other = targets();
    if (other.length) {
      const addRow = document.createElement('div');
      addRow.className = 'set-row';
      const sel = document.createElement('select');
      sel.className = 'select';
      sel.innerHTML = other.map((b) => `<option value="${b.id}">${esc(b.name)}</option>`).join('');
      const add = button(t('group.add'), async () => {
        const id = Number(sel.value);
        if (!id) return;
        await run(() => invoke('group_add_books', { bookId: view.bookId, bookIds: [id] }), null, t('common.saved'));
      });
      addRow.append(sel, add);
      const moveRow = document.createElement('div');
      moveRow.className = 'set-row';
      const dest = document.createElement('select');
      dest.className = 'select';
      dest.innerHTML = other.map((b) => `<option value="${b.id}">${esc(b.name)}</option>`).join('');
      const move = button(t('group.move'), async () => {
        const keys = selected();
        if (!keys.length) { toast(t('group.needPart')); return; }
        await run(() => invoke('group_move_member', { pathKeys: keys, toBookId: Number(dest.value) }), keys[0], t('common.saved'));
      });
      moveRow.append(dest, move);
      tools.append(addRow, moveRow);
    }
  }

  function button(label, fn) {
    const el = document.createElement('button');
    el.type = 'button';
    el.className = 'btn btn--ghost btn--sm';
    el.textContent = label;
    el.addEventListener('click', fn);
    return el;
  }

  async function run(fn, preferKey, okText) {
    try {
      const result = await fn();
      toast(result?.message || okText);
      const next = await focusResult(result, preferKey);
      if (!next) { modal.close(); return; }
      view = next;
      paint();
    } catch (e) { toast(errText(e)); }
  }

  list.addEventListener('click', async (e) => {
    const alone = e.target.closest('[data-alone]');
    if (alone && !alone.disabled) {
      const part = view.parts[Number(alone.dataset.alone)];
      if (!part) return;
      const plan = removePlan(view.parts, [part.key]);
      if (!plan.ok) { toast(t(plan.reason === 'none' ? 'group.pick' : 'group.needPart')); return; }
      await run(
        () => invoke('group_remove_members', { bookId: view.bookId, pathKeys: [part.key] }),
        plan.stay[0]?.key,
        t('merge.separatedOne'),
      );
      return;
    }
    const up = e.target.closest('[data-up]');
    const down = e.target.closest('[data-down]');
    if (!up && !down) return;
    const idx = Number((up || down).dataset.up || (up || down).dataset.down);
    const to = up ? idx - 1 : idx + 1;
    const next = moveIndex(view.parts, idx, to);
    if (next === view.parts || next.map((p) => p.key).join() === view.parts.map((p) => p.key).join()) return;
    await run(
      () => invoke('group_reorder', { bookId: view.bookId, pathKeys: next.map((p) => p.key) }),
      null,
      t('group.reorder'),
    );
  });

  paint();
  const modal = openModal({ title: t('group.edit'), body: root, wide: true, onClose: () => onDone?.() });
  modal.element.classList.add('modal--merge');
  return modal;
}
