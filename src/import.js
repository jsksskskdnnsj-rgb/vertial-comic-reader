// import.js - source list (add folders / zips / images, reorder, sort by name),
// then either import_paths (one book per source, duplicates skipped) or
// import_merged (library.rs: all sources become ONE book, in list order).
// Drag & drop fills the same list via the Rust drop queue.
import { T, t, esc, toast, bootPage } from './shell.js';
import { lib } from './legacy-api.js';
import { mountShell, watchDrops } from './shell-ui.js';
import { sortSources } from './naming.js';

bootPage('import');
mountShell('import');

const $ = (id) => document.getElementById(id);
const IMG_EXTS = ['jpg', 'jpeg', 'png', 'webp', 'gif', 'avif', 'bmp', 'jfif'];
let sources = [];       // absolute paths, in import order
let merge = false;
let lastNewId = null;

function kindOf(p) {
  const low = p.toLowerCase();
  if (/\.(zip|cbz)$/.test(low)) return 'ZIP';
  if (/\.(jpe?g|jfif|png|apng|webp|gif|bmp|avif|svg|ico|tiff?|heic|jxl)$/.test(low)) return t('type.images');
  return t('type.folder');
}
function addSources(list) {
  for (const p of list || []) if (p && !sources.includes(p)) sources.push(p);
  render();
}
function render() {
  $('srcCount').textContent = sources.length ? t('imp.count', { n: sources.length }) : '';
  $('srcEmpty').hidden = sources.length > 0;
  $('srcList').innerHTML = sources.map((p, i) => `<div class="src-row">
      <span class="kind">${esc(kindOf(p))}</span>
      <span class="path" title="${esc(p)}">${esc(p)}</span>
      <button data-up="${i}" aria-label="${esc(t('imp.moveUp'))}"><svg class="i"><use href="icons.svg#skip-prev"/></svg></button>
      <button data-down="${i}" aria-label="${esc(t('imp.moveDown'))}"><svg class="i"><use href="icons.svg#skip-next"/></svg></button>
      <button data-rm="${i}" aria-label="${esc(t('common.remove'))}"><svg class="i"><use href="icons.svg#trash"/></svg></button>
    </div>`).join('');
  document.querySelectorAll('#mergeSeg button').forEach((b) =>
    b.setAttribute('aria-pressed', String((b.dataset.merge === 'on') === merge)));
}
$('srcList').addEventListener('click', (e) => {
  const up = e.target.closest('[data-up]'), down = e.target.closest('[data-down]'), rm = e.target.closest('[data-rm]');
  if (up) { const i = +up.dataset.up; if (i > 0) [sources[i - 1], sources[i]] = [sources[i], sources[i - 1]]; }
  if (down) { const i = +down.dataset.down; if (i < sources.length - 1) [sources[i + 1], sources[i]] = [sources[i], sources[i + 1]]; }
  if (rm) sources.splice(+rm.dataset.rm, 1);
  render();
});
$('sortBtn').addEventListener('click', () => { sources = sortSources(sources); render(); });
$('clearBtn').addEventListener('click', () => { sources = []; render(); });
$('mergeSeg').addEventListener('click', (e) => {
  const b = e.target.closest('[data-merge]'); if (!b) return;
  merge = b.dataset.merge === 'on';
  render();
});

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
const zone = $('zone');
addEventListener('dragover', (e) => { e.preventDefault(); zone.classList.add('is-over'); });
addEventListener('dragleave', () => zone.classList.remove('is-over'));
addEventListener('drop', (e) => { e.preventDefault(); zone.classList.remove('is-over'); });
watchDrops(addSources);

async function loadFolders() {
  const folders = await lib.folders().catch(() => []);
  $('folderSel').innerHTML = `<option value="">${esc(t('lib.noFolder'))}</option>` +
    folders.map((f) => `<option value="${f.id}">${esc(f.name)}</option>`).join('');
}
loadFolders();

$('goBtn').addEventListener('click', async () => {
  if (!sources.length) { toast(t('imp.empty')); return; }
  const folderId = $('folderSel').value ? +$('folderSel').value : null;
  $('summary').textContent = t('imp.working');
  $('failList').innerHTML = '';
  $('openNew').hidden = true;
  try {
    if (merge) {
      lastNewId = await lib.importMerged([...sources], null, folderId);
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
    render();
  } catch (e) {
    $('summary').textContent = '';
    toast(String(e?.detail ?? e?.message ?? e));
  }
});
$('openNew').addEventListener('click', () => {
  if (lastNewId) location.href = `reader.html?book=${lastNewId}`;
});
render();
