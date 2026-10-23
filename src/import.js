// import.js - source list (add folders / zips / images, reorder, sort), then either
// import_paths (one book per source, duplicates skipped) or import_merged (all sources become
// ONE book, in list order). In merge mode the list is ordered by the numeric range in the names
// ("xxx1-60" before "xxx60-90") until the user reorders by hand. Drag & drop fills the same list
// via the Rust drop queue.
import { T, t, esc, toast, bootPage } from './shell.js';
import { lib } from './legacy-api.js';
import { mountShell, watchDrops } from './shell-ui.js';
import { sortSources } from './naming.js';
import { orderByRange, suggestName } from './merge-order.js';

bootPage('import');
mountShell('import');

const $ = (id) => document.getElementById(id);
const IMG_EXTS = ['jpg', 'jpeg', 'png', 'webp', 'gif', 'avif', 'bmp', 'jfif'];
let sources = [];        // absolute paths, in import order
let merge = false;
let manualOrder = false; // true once the user moved a row by hand: stop auto-sorting
let nameEdited = false;  // true once the user typed their own merged-book name
let lastNewId = null;

function kindOf(p) {
  const low = p.toLowerCase();
  if (/\.(zip|cbz)$/.test(low)) return t('type.zip');
  if (/\.(jpe?g|jfif|jpe|png|webp|gif|bmp|avif)$/.test(low)) return t('type.images');
  return t('type.folder');
}
function autoOrder() {
  if (merge && !manualOrder) sources = orderByRange(sources);
}
function addSources(list) {
  for (const p of list || []) if (p && !sources.includes(p)) sources.push(p);
  autoOrder();
  render();
}
function render() {
  $('srcCount').textContent = sources.length ? t('imp.count', { n: sources.length }) : '';
  $('srcEmpty').hidden = sources.length > 0;
  $('srcList').innerHTML = sources.map((p, i) => `<div class="src-row">
      <span class="idx num">${merge ? i + 1 : ''}</span>
      <span class="kind">${esc(kindOf(p))}</span>
      <span class="path" title="${esc(p)}">${esc(p)}</span>
      <button data-up="${i}" ${i === 0 ? 'disabled' : ''} aria-label="${esc(t('imp.moveUp'))}"><svg class="i"><use href="icons.svg#chevron-up"/></svg></button>
      <button data-down="${i}" ${i === sources.length - 1 ? 'disabled' : ''} aria-label="${esc(t('imp.moveDown'))}"><svg class="i"><use href="icons.svg#chevron-down"/></svg></button>
      <button data-rm="${i}" aria-label="${esc(t('common.remove'))}"><svg class="i"><use href="icons.svg#trash"/></svg></button>
    </div>`).join('');
  document.querySelectorAll('#mergeSeg button').forEach((b) =>
    b.setAttribute('aria-pressed', String((b.dataset.merge === 'on') === merge)));
  $('nameRow').hidden = !merge;
  $('mergeName').placeholder = t('imp.mergeNamePh');
  if (merge && !nameEdited) $('mergeName').value = suggestName(sources);
}
$('srcList').addEventListener('click', (e) => {
  const up = e.target.closest('[data-up]'), down = e.target.closest('[data-down]'), rm = e.target.closest('[data-rm]');
  if (up) { const i = +up.dataset.up; if (i > 0) { [sources[i - 1], sources[i]] = [sources[i], sources[i - 1]]; manualOrder = true; } }
  if (down) { const i = +down.dataset.down; if (i < sources.length - 1) { [sources[i + 1], sources[i]] = [sources[i], sources[i + 1]]; manualOrder = true; } }
  if (rm) sources.splice(+rm.dataset.rm, 1);
  render();
});
$('sortBtn').addEventListener('click', () => {
  sources = merge ? orderByRange(sources) : sortSources(sources);
  manualOrder = false;
  render();
});
$('clearBtn').addEventListener('click', () => { sources = []; manualOrder = false; nameEdited = false; $('mergeName').value = ''; render(); });
$('mergeSeg').addEventListener('click', (e) => {
  const b = e.target.closest('[data-merge]'); if (!b) return;
  merge = b.dataset.merge === 'on';
  autoOrder();
  render();
});
$('mergeName').addEventListener('input', () => { nameEdited = $('mergeName').value.trim() !== ''; });

$('addZips').addEventListener('click', async () => {
  const picked = await T.dialog.open({
    multiple: true,
    filters: [
      { name: t('imp.zipFilter'), extensions: ['zip', 'cbz'] },
      { name: t('imp.imageFilter'), extensions: IMG_EXTS },
    ],
  }).catch(() => null);
  addSources([].concat(picked ?? []));
});
$('addFolders').addEventListener('click', async () => {
  const picked = await T.dialog.open({ multiple: true, directory: true }).catch(() => null);
  addSources([].concat(picked ?? []));
});

// Drag-over highlight: enter/leave fire for every child element, so count them.
const zone = $('zone');
let dragDepth = 0;
addEventListener('dragenter', (e) => { e.preventDefault(); dragDepth++; zone.classList.add('is-over'); });
addEventListener('dragover', (e) => e.preventDefault());
addEventListener('dragleave', () => { dragDepth = Math.max(0, dragDepth - 1); if (!dragDepth) zone.classList.remove('is-over'); });
addEventListener('drop', (e) => { e.preventDefault(); dragDepth = 0; zone.classList.remove('is-over'); });
watchDrops(addSources);

async function loadFolders() {
  const folders = await lib.folders().catch(() => []);
  $('folderSel').innerHTML = `<option value="">${esc(t('lib.noFolder'))}</option>` +
    folders.map((f) => `<option value="${f.id}">${esc(f.name)}</option>`).join('');
}
loadFolders();

$('goBtn').addEventListener('click', async () => {
  if (!sources.length) { toast(t('imp.empty')); return; }
  if (merge && sources.length < 2) { toast(t('imp.mergeNeedTwo')); return; }
  const folderId = $('folderSel').value ? +$('folderSel').value : null;
  $('goBtn').disabled = true;
  $('summary').textContent = t('imp.working');
  $('failList').innerHTML = '';
  $('openNew').hidden = true;
  try {
    if (merge) {
      const name = $('mergeName').value.trim() || null;
      lastNewId = await lib.importMerged([...sources], name, folderId);
      $('summary').textContent = t('imp.mergedToast');
      toast(t('imp.mergedToast'));
      $('openNew').hidden = false;
    } else {
      const r = await lib.importPaths([...sources], folderId);
      $('summary').textContent = t('imp.summary', { a: r.added, s: r.skipped, f: r.failed.length });
      if (r.added) toast(t('imp.importedToast', { n: r.added }));
      else if (!r.failed.length) toast(t('imp.nothingNew'));
      $('failList').innerHTML = (r.failed || []).map((f) =>
        `<li>${esc(t('imp.failedRow', { p: f.path, r: f.reason }))}</li>`).join('');
    }
    sources = [];
    manualOrder = false;
    nameEdited = false;
    $('mergeName').value = '';
    render();
  } catch (e) {
    $('summary').textContent = '';
    toast(String(e?.detail ?? e?.message ?? e));
  } finally {
    $('goBtn').disabled = false;
  }
});
$('openNew').addEventListener('click', () => {
  if (lastNewId) location.href = `reader.html?book=${lastNewId}`;
});
render();
