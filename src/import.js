// import.js - source list (add folders / zips / images, reorder, sort), then either
// import_paths (one book per source, duplicates skipped) or import_merged (all sources become
// ONE book, in list order). In merge mode the list is ordered by the numeric range in the names
// ("xxx1-60" before "xxx60-90") until the user reorders by hand. Drag & drop fills the same list
// via the Rust drop queue.
// Subfolders: a picked folder is imported with ALL its nested subfolders (the backend scan is recursive); before a folder
// is listed it is inspected with list_images, the row shows how many images / subfolders it holds, and a folder with no
// images at any depth is refused (reported under the list) instead of being added silently.
import { T, t, esc, toast, bootPage } from './shell.js';
import { lib } from './legacy-api.js';
import { mountShell, watchDrops } from './shell-ui.js';
import { sortSources } from './naming.js';
import { orderByRange, suggestName, compareSources, baseName } from './merge-order.js';
import { inspectFolder, looksLikeFile } from './folder-scan.js';

bootPage('import');
mountShell('import');

const $ = (id) => document.getElementById(id);
const IMG_EXTS = ['jpg', 'jpeg', 'png', 'webp', 'gif', 'avif', 'bmp', 'jfif'];
let sources = [];        // absolute paths, in import order
let merge = false;
let manualOrder = false; // true once the user moved a row by hand: stop auto-sorting
let nameEdited = false;  // true once the user typed their own merged-book name
let lastNewId = null;
const info = new Map();  // folder path -> scan summary from list_images (images, nested, subfolders, ...)
let skipped = [];        // folders refused because they hold no images, even in subfolders: { path, archives }
let addChain = Promise.resolve(); // adds run one after another (each folder scan awaits the backend)
const pkey = (p) => String(p).replace(/\//g, '\\').replace(/\\+$/, '').toLowerCase();
const plural = (n, one, many) => t(n === 1 ? one : many, { n });

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
  addChain = addChain.then(() => doAdd(list)).catch(() => {});
  return addChain;
}
async function doAdd(list) {
  const fresh = [...new Set((list || []).filter((p) => p && !sources.includes(p)))];
  if (!fresh.length) return;
  const scanning = fresh.some((p) => !looksLikeFile(p));
  if (scanning) $('summary').textContent = t('imp.scanning');
  let refused = 0;
  for (const p of fresh) {
    if (looksLikeFile(p)) { sources.push(p); continue; }
    const r = await inspectFolder(p, lib.listImages);
    if (!r.ok) { sources.push(p); continue; }           // not readable as a folder (e.g. an unsupported file): the importer reports it as before
    if (r.empty) { skipped.push({ path: p, archives: r.archives }); refused++; continue; }
    info.set(p, r); sources.push(p);
  }
  if (scanning) $('summary').textContent = '';
  if (refused) toast(t('imp.emptySkipped'));
  autoOrder();
  render();
}
function chipFor(p) {
  const r = info.get(p); if (!r) return '';
  const parts = [plural(r.images, 'imp.image1', 'imp.imagesCount')];
  if (r.subfolders > 0) parts.push(plural(r.subfolders, 'imp.subfolder1', 'imp.subfoldersCount'));
  const names = [...r.subfolderNames].sort(compareSources);
  const tip = names.slice(0, 12).join('\n') + (names.length > 12 ? '\n' + t('imp.andMore', { n: names.length - 12 }) : '');
  return `<span class="scan-chip${r.subfolders ? ' has-sub' : ''}" title="${esc(tip)}">${esc(parts.join(' · '))}</span>`;
}
function noteFor(p) {            // picking a folder AND one of its subfolders would import those images twice
  if (looksLikeFile(p)) return '';
  const parent = sources.find((q) => q !== p && !looksLikeFile(q) && pkey(p).startsWith(pkey(q) + '\\'));
  return parent ? `<div class="src-note">${esc(t('imp.overlap', { a: baseName(p), b: baseName(parent) }))}</div>` : '';
}
function renderScan() {
  let images = 0, nested = 0;
  for (const p of sources) { const r = info.get(p); if (r) { images += r.images; nested += r.nested; } }
  $('scanTotals').hidden = images === 0;
  $('scanTotals').textContent = images ? t('imp.totals', { images, nested }) : '';
  $('skipList').innerHTML = skipped.map((s) =>
    `<li>${esc(t(s.archives ? 'imp.emptyFolderZips' : 'imp.emptyFolder', { name: baseName(s.path), n: s.archives }))}</li>`).join('');
}
function render() {
  $('srcCount').textContent = sources.length ? t('imp.count', { n: sources.length }) : '';
  $('srcEmpty').hidden = sources.length > 0;
  $('srcList').innerHTML = sources.map((p, i) => `<div class="src-item"><div class="src-row">
      <span class="idx num">${merge ? i + 1 : ''}</span>
      <span class="kind">${esc(kindOf(p))}</span>
      ${chipFor(p)}
      <span class="path" title="${esc(p)}">${esc(p)}</span>
      <button data-up="${i}" ${i === 0 ? 'disabled' : ''} aria-label="${esc(t('imp.moveUp'))}"><svg class="i"><use href="icons.svg#chevron-up"/></svg></button>
      <button data-down="${i}" ${i === sources.length - 1 ? 'disabled' : ''} aria-label="${esc(t('imp.moveDown'))}"><svg class="i"><use href="icons.svg#chevron-down"/></svg></button>
      <button data-rm="${i}" aria-label="${esc(t('common.remove'))}"><svg class="i"><use href="icons.svg#trash"/></svg></button>
    </div>${noteFor(p)}</div>`).join('');
  renderScan();
  document.querySelectorAll('#mergeSeg button').forEach((b) => {
    b.setAttribute('aria-pressed', String((b.dataset.merge === 'on') === merge));
    // imp.goMerge / imp.goSeparate contain {n}; applyI18n cannot fill it in, so the live source count goes in here
    b.textContent = t(b.dataset.merge === 'on' ? 'imp.goMerge' : 'imp.goSeparate', { n: sources.length });
  });
  $('nameRow').hidden = !merge;
  $('mergeName').placeholder = t('imp.mergeNamePh');
  if (merge && !nameEdited) $('mergeName').value = suggestName(sources);
}
$('srcList').addEventListener('click', (e) => {
  const up = e.target.closest('[data-up]'), down = e.target.closest('[data-down]'), rm = e.target.closest('[data-rm]');
  if (up) { const i = +up.dataset.up; if (i > 0) { [sources[i - 1], sources[i]] = [sources[i], sources[i - 1]]; manualOrder = true; } }
  if (down) { const i = +down.dataset.down; if (i < sources.length - 1) { [sources[i + 1], sources[i]] = [sources[i], sources[i + 1]]; manualOrder = true; } }
  if (rm) { const [gone] = sources.splice(+rm.dataset.rm, 1); info.delete(gone); }
  render();
});
$('sortBtn').addEventListener('click', () => {
  sources = merge ? orderByRange(sources) : sortSources(sources);
  manualOrder = false;
  render();
});
$('clearBtn').addEventListener('click', () => { sources = []; info.clear(); skipped = []; manualOrder = false; nameEdited = false; $('mergeName').value = ''; render(); });
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
    info.clear(); skipped = [];
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
