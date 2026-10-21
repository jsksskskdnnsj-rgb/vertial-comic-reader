// settings.js - Reading defaults (localStorage 'mh.reader'), chapter defaults
// (get/set_chapter_defaults, folded in from chapter-settings.html), theme
// (localStorage 'mh.theme' via shell.js applyTheme), data dir, cache, about.
import { T, invoke, t, toast, applyI18n, applyTheme, wireWindow } from './shell.js';
import { sanePages, DEFAULT_PAGES } from './chapters.js';

const $ = (id) => document.getElementById(id);

// ---- 阅读：显示默认值，键/默认值与 reader-boot.js 的 DEF 完全一致 ---------------
const DEF = { widthPct: 100, gap: 0, bg: 'lavender', dim: 100, saver: false };
let cfg = { ...DEF };
try { cfg = { ...DEF, ...JSON.parse(localStorage.getItem('mh.reader') || '{}') }; } catch {}
const saveCfg = () => localStorage.setItem('mh.reader', JSON.stringify(cfg));

const pressed = (sel, attr, val) => document.querySelectorAll(sel).forEach((b) => b.setAttribute('aria-pressed', String(b.dataset[attr] == val)));
function renderReaderCfg() {
  $('setW').value = cfg.widthPct;
  $('setDim').value = cfg.dim;
  $('setSaver').checked = !!cfg.saver;
  pressed('#setGap button', 'gap', cfg.gap);
  pressed('#setBg button', 'bg', cfg.bg);
}
$('setW').addEventListener('input', (e) => { cfg.widthPct = +e.target.value; saveCfg(); });
$('setDim').addEventListener('input', (e) => { cfg.dim = +e.target.value; saveCfg(); });
$('setGap').addEventListener('click', (e) => { const b = e.target.closest('button'); if (!b) return; cfg.gap = +b.dataset.gap; pressed('#setGap button', 'gap', cfg.gap); saveCfg(); });
$('setBg').addEventListener('click', (e) => { const b = e.target.closest('button'); if (!b) return; cfg.bg = b.dataset.bg; pressed('#setBg button', 'bg', cfg.bg); saveCfg(); });
$('setSaver').addEventListener('change', (e) => { cfg.saver = e.target.checked; saveCfg(); });
renderReaderCfg();

// ---- 分话默认值（chapter-settings.html 的逻辑，现由 settings.js 统一持有） -----
const HELP = { none: 'chNoneHelp', folder: 'chFolderHelp', pages: 'chPagesHelp' };
let chMode = 'folder', chPages = DEFAULT_PAGES, chTimer = 0;
function renderCh() {
  document.querySelectorAll('#dMode button').forEach((b) => b.setAttribute('aria-pressed', String(b.dataset.m === chMode)));
  $('dNRow').hidden = chMode !== 'pages';
  $('dHelp').textContent = t(HELP[chMode]);
  if (document.activeElement !== $('dN')) $('dN').value = chPages;
}
const saveCh = () => { clearTimeout(chTimer); chTimer = setTimeout(() => invoke('set_chapter_defaults', { mode: chMode, pages: chPages }).then(() => toast(t('saved'))).catch(() => {}), 400); };
$('dMode').addEventListener('click', (e) => { const b = e.target.closest('button'); if (!b) return; chMode = b.dataset.m; renderCh(); saveCh(); });
$('dN').addEventListener('input', (e) => { if (e.target.value === '' || +e.target.value < 1) return; chPages = sanePages(e.target.value); renderCh(); saveCh(); });
$('dN').addEventListener('blur', () => { $('dN').value = chPages; });
invoke('get_chapter_defaults').then((d) => { chMode = d.mode; chPages = sanePages(d.pages); renderCh(); }).catch(renderCh);

// ---- 主题：localStorage 'mh.theme'（'light' 默认 / 'night'），applyTheme 读它 --
$('themeSel').value = localStorage.getItem('mh.theme') || 'light';
$('themeSel').addEventListener('change', (e) => { localStorage.setItem('mh.theme', e.target.value); applyTheme(); });

// ---- 数据目录 -------------------------------------------------------------------
async function refreshDataDir() {
  try { $('dataDir').textContent = await invoke('get_data_dir'); } catch { $('dataDir').textContent = '读取失败'; }
}
$('changeDataDir').addEventListener('click', async () => {
  try {
    const picked = await invoke('pick_data_dir');
    if (!picked) return;
    const applied = await invoke('set_data_dir', { path: picked });
    $('dataDir').textContent = applied || picked;
    toast('数据目录已更换');
  } catch (e) { toast(`更换失败：${e?.detail ?? e?.message ?? e}`); }
});
$('openDataDir').addEventListener('click', () => invoke('open_data_dir').catch(() => {}));
refreshDataDir();

// ---- 缓存 ------------------------------------------------------------------------
const fmtMB = (bytes) => `${((bytes || 0) / 1048576).toFixed(1)} MB`;
async function refreshCache() {
  try {
    const s = await invoke('cache_stats');
    $('cacheStats').textContent = `阅读缓存已用 ${fmtMB(s.bytes)} / 上限 ${fmtMB(s.limitBytes)} · 共 ${s.entries} 项`;
    if (document.activeElement !== $('cacheLimitMb')) $('cacheLimitMb').value = Math.round((s.limitBytes || 0) / 1048576);
  } catch { $('cacheStats').textContent = '缓存信息读取失败'; }
}
$('setCacheLimit').addEventListener('click', async () => {
  const mb = +$('cacheLimitMb').value;
  if (!(mb > 0)) return;
  try { await invoke('cache_set_limit', { bytes: Math.round(mb * 1048576) }); toast(t('saved')); refreshCache(); }
  catch (e) { toast(`保存失败：${e?.detail ?? e?.message ?? e}`); }
});
$('clearReaderCache').addEventListener('click', async () => {
  try { await invoke('cache_clear', { bookId: null }); toast('阅读缓存已清理'); refreshCache(); }
  catch (e) { toast(`清理失败：${e?.detail ?? e?.message ?? e}`); }
});
$('clearLegacyCache').addEventListener('click', async () => {
  try { await invoke('clear_cache'); toast('旧页面缓存已清理'); }
  catch (e) { toast(`清理失败：${e?.detail ?? e?.message ?? e}`); }
});
refreshCache();

// ---- 关于 + 危险区 ---------------------------------------------------------------
T.app.getVersion().then((v) => { $('ver').textContent = `v${v}`; }).catch(() => {});
$('clearShelf').addEventListener('click', async () => {
  if (!confirm('确定要清空书架吗？全部书籍记录将被移除（源文件不会被删除），且无法撤销。')) return;
  try { await invoke('shelf_clear'); toast('书架已清空'); }
  catch (e) { toast(`清空失败：${e?.detail ?? e?.message ?? e}`); }
});

// topbar search behaves like home: Enter goes to the library with ?q=
const q = $('q');
q.addEventListener('keydown', (e) => { if (e.key === 'Enter' && q.value.trim()) location.href = `library.html?q=${encodeURIComponent(q.value.trim())}`; });

applyTheme(); applyI18n(); wireWindow();
renderCh();
