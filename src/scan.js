// scan.js - pick one folder, review the books found in its subfolders, then add the ones you want.
// The walk runs in the background. Cancelling or leaving the page keeps a checkpoint, so the next
// scan of the same folder continues. Problems (empty files, unreadable directories, links) are listed.
import { T, invoke, t, esc, toast, bootPage } from './shell.js';
import { lib } from './legacy-api.js';
import { mountShell } from './shell-ui.js';
import { openMergeDialog } from './merge-ui.js';

bootPage('import');
mountShell('import');

const $ = (id) => document.getElementById(id);
const ERR = { drive_root: 'scan.driveRoot', missing: 'scan.missing', not_folder: 'scan.notFolder' };
const PAGE = 40;
let snap = null;
let timer = 0;
let adding = false;
let listPage = 0;
let picked = null;

function errText(e) {
  const raw = String(e?.message ?? e?.detail ?? e ?? '');
  return ERR[raw] ? t(ERR[raw]) : raw || t('err.noDesktop');
}
function chapterLine(book) {
  const ch = book.chapters || [];
  if (!ch.length) return t('scan.noChapters');
  const shown = ch.slice(0, 4).map((c) => c.title).join(' · ');
  const extra = ch.length > 4 ? ` · +${ch.length - 4}` : '';
  return `${t('scan.chapters', { n: ch.length })} · ${shown}${extra}`;
}
function issueReason(reason) {
  if (reason === 'empty') return t('scan.issueEmpty');
  if (reason === 'symlink') return t('scan.issueSymlink');
  if (reason === 'non_utf8') return t('scan.issueNonUtf8');
  if (reason === 'missing') return t('scan.issueMissing');
  if (String(reason).startsWith('unreadable')) return t('scan.issueUnreadable');
  return reason;
}
function paintIssues(rows) {
  const list = rows || [];
  $('scanIssueBox').hidden = list.length === 0;
  $('scanIssueTitle').textContent = t('scan.issues', { n: snap?.issue_count || list.length });
  $('scanIssues').innerHTML = list.map((row) => `<div class="scan-issue"><b>${esc(issueReason(row.reason))}</b> · ${esc(row.path)}</div>`).join('');
}
function checked(book, idx) {
  if (!book || book.already) return false;
  if (!picked) return true;
  return picked.has(idx);
}
function ensurePicked() {
  if (picked) return picked;
  picked = new Set();
  (snap?.books || []).forEach((b, i) => { if (!b.already) picked.add(i); });
  return picked;
}
function render(s) {
  const prevPhase = snap?.phase;
  snap = s;
  const running = s?.phase === 'running';
  if (running && prevPhase !== 'running') { listPage = 0; picked = null; }
  if ((s?.phase === 'done' || s?.phase === 'cancelled') && prevPhase === 'running') listPage = 0;
  $('scanProgress').hidden = !running;
  $('scanCancel').hidden = !running;
  $('scanStat').textContent = running ? t('scan.scanning', { folders: s.folders_scanned || 0, books: s.books_found || 0 }) : '';
  const cur = s?.current_path || '';
  $('scanPath').hidden = !cur;
  $('scanPath').textContent = cur ? t('scan.current', { path: cur }) : '';
  $('scanResume').hidden = !s?.resumed;
  $('scanResume').textContent = s?.resumed ? t('scan.resumed') : '';
  const books = s?.books || [];
  const finished = s && (s.phase === 'done' || s.phase === 'cancelled');
  $('scanTools').hidden = !finished || !books.length;
  $('scanLib').hidden = !finished;
  if (s?.phase === 'done') $('scanNote').textContent = t('scan.done', { books: s.books_found || books.length, folders: s.folders_scanned || 0 });
  else if (s?.phase === 'cancelled') $('scanNote').textContent = t('scan.cancelled', { books: s.books_found || books.length });
  else if (s?.phase === 'error') $('scanNote').textContent = s.error || '';
  else if (!running) $('scanNote').textContent = '';
  $('scanLoose').hidden = !(finished && s.loose_pages > 0);
  if (s?.loose_pages) $('scanLoose').textContent = t('scan.loose', { n: s.loose_pages });
  if (finished && !books.length && s.phase !== 'error') $('scanNote').textContent = t('scan.empty');
  const pages = Math.max(1, Math.ceil(books.length / PAGE));
  if (listPage >= pages) listPage = pages - 1;
  if (listPage < 0) listPage = 0;
  const slice = books.slice(listPage * PAGE, listPage * PAGE + PAGE);
  const offset = listPage * PAGE;
  $('scanList').innerHTML = slice.map((b, i) => `<label class="scan-row${b.already ? ' is-already' : ''}">
      <input type="checkbox" data-idx="${offset + i}" ${b.already ? 'disabled' : checked(b, offset + i) ? 'checked' : ''}>
      <span class="scan-row__main">
        <b>${esc(b.title)}</b>
        <span class="meta num">${esc(t('scan.pages', { n: b.pages }))} · ${esc(chapterLine(b))}</span>
        <span class="scan-path">${esc(b.path)}</span>
      </span>
      ${b.already ? `<span class="lib-chip">${esc(t('scan.already'))}</span>` : ''}
    </label>`).join('');
  $('scanPager').hidden = books.length <= PAGE;
  $('scanPageLabel').textContent = t('lib.page', { n: listPage + 1, m: pages });
  if (s?.issues) paintIssues(s.issues);
  if (s?.phase === 'done') maybePrompt(s);
  if (s && s.phase !== 'running') refreshScanPending();
}
function schedule() {
  clearTimeout(timer);
  timer = setTimeout(poll, 200);
}
async function poll() {
  try {
    const s = await invoke('scan_status');
    render(s);
    if (s?.phase === 'running') schedule();
    else if (s?.root) {
      const rows = await invoke('list_scan_issues', { root: s.root }).catch(() => null);
      if (rows) paintIssues(rows);
    }
  } catch (e) {
    if (snap) $('scanNote').textContent = errText(e);
  }
}
async function loadCategories() {
  const folders = await lib.folders().catch(() => []);
  const sel = $('scanFolder');
  const cur = sel.value;
  sel.innerHTML = `<option value="">${esc(t('lib.noFolder'))}</option>` +
    (folders || []).map((f) => `<option value="${f.id}">${esc(f.name)}</option>`).join('');
  if (cur) sel.value = cur;
}
$('scanPick').addEventListener('click', async () => {
  if (!T.dialog?.open) { toast(t('err.noDesktop')); return; }
  let picked;
  try { picked = await T.dialog.open({ directory: true, multiple: false }); }
  catch (e) { toast(errText(e)); return; }
  const root = Array.isArray(picked) ? picked[0] : picked;
  if (!root) return;
  try {
    await invoke('scan_begin', { root });
  } catch (e) { toast(errText(e)); return; }
  listPage = 0;
  picked = null;
  render({ phase: 'running', folders_scanned: 0, books_found: 0, books: [], loose_pages: 0, current_path: root, issues: [], resumed: false });
  schedule();
});
$('scanCancel').addEventListener('click', () => { invoke('scan_cancel').catch(() => {}); });
addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && snap?.phase === 'running') invoke('scan_cancel').catch(() => {});
});
$('scanList').addEventListener('change', (e) => {
  const box = e.target.closest('input[type="checkbox"]');
  if (!box || box.disabled) return;
  const set = ensurePicked();
  const idx = Number(box.dataset.idx);
  if (box.checked) set.add(idx); else set.delete(idx);
});
$('scanAll').addEventListener('click', () => { picked = null; if (snap) render(snap); });
$('scanPrev').addEventListener('click', () => { if (listPage > 0) { listPage -= 1; render(snap); } });
$('scanNext').addEventListener('click', () => { listPage += 1; render(snap); });
$('scanAdd').addEventListener('click', async () => {
  if (adding || !snap) return;
  const chosen = (snap.books || []).filter((b, i) => checked(b, i));
  if (!chosen.length) { toast(t('scan.noneSelected')); return; }
  adding = true;
  $('scanAdd').disabled = true;
  $('scanAdd').textContent = t('scan.adding');
  const folderId = $('scanFolder').value ? Number($('scanFolder').value) : null;
  try {
    const r = await invoke('import_scan_books', {
      items: chosen.map((b) => ({ path: b.path, title: b.title, pages: b.pages })),
      folderId,
    });
    const added = new Set(chosen.map((b) => b.path));
    for (const b of snap.books) if (added.has(b.path)) b.already = true;
    render(snap);
    toast(t('scan.added', { n: r?.added || 0, skipped: r?.skipped || 0 }));
    if (r?.failed?.length) toast(r.failed[0].reason);
    $('scanLib').hidden = false;
  } catch (e) { toast(errText(e)); }
  adding = false;
  $('scanAdd').disabled = false;
  $('scanAdd').textContent = t('scan.add');
});
async function refreshScanPending() {
  const rows = await invoke('list_merge_pending').catch(() => null);
  const btn = $('scanPending');
  if (!btn || !rows) return;
  btn.hidden = rows.length === 0;
  btn.textContent = rows.length ? t('merge.pendingBanner', { n: rows.length }) : '';
  btn.onclick = async () => {
    const folders = await lib.folders().catch(() => []);
    openMergeDialog({ groups: rows, folders, onDone: refreshScanPending });
  };
}
function maybePrompt(s) {
  if (s?.phase !== 'done' || !(s.pending > 0)) return;
  const key = `${s.root}|${s.folders_scanned}|${s.pending}|${s.merged}|${s.added}`;
  if (sessionStorage.getItem('mh.mergeDialog') === key) return;
  sessionStorage.setItem('mh.mergeDialog', key);
  invoke('list_merge_pending').then(async (rows) => {
    if (!rows?.length) return;
    const folders = await lib.folders().catch(() => []);
    openMergeDialog({ groups: rows, folders, onDone: refreshScanPending });
  }).catch(() => {});
}
loadCategories();
poll();
if (new URLSearchParams(location.search).get('preview') === 'merge') {
  openMergeDialog({
    preview: true,
    folders: [{ id: 1, name: '连载' }],
    groups: [
      {
        id: 1,
        suggested_title: '海贼王',
        confident: true,
        members: [
          { name: '海贼王 第1-50话', path: 'D:/comics/海贼王 第1-50话', pages: 980, chapters: 50, order_label: '1–50', shelf_id: null },
          { name: '海贼王 第51-100话', path: 'D:/comics/海贼王 第51-100话', pages: 1020, chapters: 50, order_label: '51–100', shelf_id: null },
        ],
      },
      {
        id: 2,
        suggested_title: '海贼王',
        confident: false,
        members: [
          { name: '海贼王', path: 'D:/comics/海贼王', pages: 2000, chapters: 100, order_label: '', shelf_id: null },
          { name: '海贼王 彩色版', path: 'D:/comics/海贼王 彩色版', pages: 400, chapters: 12, order_label: '', shelf_id: null },
        ],
      },
    ],
  });
}
invoke('list_scan_issues', { root: null }).then((rows) => { if (!snap || snap.phase === 'idle' || !snap.phase) paintIssues(rows || []); }).catch(() => {});
