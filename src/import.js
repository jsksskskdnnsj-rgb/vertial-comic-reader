// import.js - pickers + whole-window drag-drop, all funnelled into import_paths.
import { T, invoke, t, esc, toast, applyI18n, applyTheme, wireWindow } from './shell.js';

const $ = (id) => document.getElementById(id);
const api = {
  import: (paths) => invoke('import_paths', { paths, folderId: null }),
};

function showResult(r) {
  $('noResults')?.remove();
  const failed = r.failed || [];
  const el = Object.assign(document.createElement('section'), { className: 'panel imp-result' });
  el.innerHTML = `<div class="sum">已添加 ${r.added ?? 0} 本 · 跳过 ${r.skipped ?? 0} 本 · 失败 ${failed.length} 项</div>` +
    (failed.length
      ? `<ul>${failed.map((f) => `<li>${esc(f.path)} — ${esc(f.reason)}</li>`).join('')}</ul>`
      : '');
  $('results').prepend(el);
}

async function doImport(paths) {
  if (!paths?.length) return;
  try {
    const r = await api.import(paths);
    showResult(r);
    toast(r.added ? t('imported', { n: r.added }) : t('importNone'));
  } catch (e) { toast(t('importFail', { e: e?.detail ?? e?.message ?? e })); }
}

// pick_images / pick_zips -> [{name, path, size}]; pick_folder_paths -> [path strings]
$('pickZips').addEventListener('click', async () => {
  const items = await invoke('pick_zips').catch(() => []);
  doImport((items || []).map((x) => x.path).filter(Boolean));
});
$('pickImages').addEventListener('click', async () => {
  const items = await invoke('pick_images').catch(() => []);
  doImport((items || []).map((x) => x.path).filter(Boolean));
});
$('pickFolder').addEventListener('click', async () => {
  const paths = await invoke('pick_folder_paths').catch(() => []);
  doImport([].concat(paths ?? []));
});

// whole-window drag & drop (same pattern as home.js)
const zone = $('zone');
T.webview.getCurrentWebview().onDragDropEvent(({ payload: p }) => {
  if (p.type === 'enter' || p.type === 'over') zone.classList.add('is-over');
  else { zone.classList.remove('is-over'); if (p.type === 'drop') doImport(p.paths); }
});

// topbar search behaves like home: Enter goes to the library with ?q=
const q = $('q');
q.addEventListener('keydown', (e) => { if (e.key === 'Enter' && q.value.trim()) location.href = `library.html?q=${encodeURIComponent(q.value.trim())}`; });
addEventListener('keydown', (e) => { if ((e.key === '/' || (e.ctrlKey && e.key === 'f')) && document.activeElement !== q) { e.preventDefault(); q.focus(); } });

applyTheme(); applyI18n(); wireWindow();
