// shell.js - shared by home.html, reader.html (and your library/import/settings pages).
// Needs tauri.conf.json: app.withGlobalTauri = true, window: decorations:false, transparent:true.
// Capabilities (src-tauri/capabilities/*.json): core:window:allow-start-dragging, allow-minimize,
//   allow-toggle-maximize, allow-close, allow-destroy, allow-is-maximized, allow-set-fullscreen,
//   allow-is-fullscreen, dialog:allow-open, core:webview:allow-... (drag-drop is on by default).
export const T = window.__TAURI__;
export const invoke = T.core.invoke;
export const win = T.window.getCurrentWindow();

// ---- i18n: the reference UI is zh-CN, so zh is the default. Persist `lang` in your settings table
//      (localStorage is used here only so the files run standalone; WebView2 keeps it in the data dir).
const S = {
  zh: {
    tagline: '更好的漫画阅读体验', home: '首页', library: '书架', import: '本地导入', settings: '设置',
    quote: '漫画不只是图像，<br>而是另一个世界。', search: '搜索漫画、作者、标签...',
    welcome: '欢迎使用 mh-reader', welcomeSub: '支持多种格式 · 本地阅读 · 纯净体验',
    dropTitle: '拖放文件夹或压缩包到这里', dropSub: '支持 .zip、.cbz 压缩包、图片文件夹和散图',
    pick: '选择文件 / 文件夹', pickFiles: '选择压缩包或图片', pickFolder: '选择文件夹',
    dropNote: '本地读取 · 不联网 · 按需解压', quick: '快速开始',
    qImport: '本地导入', qImportD: '支持压缩包与文件夹<br>一键导入漫画', qLib: '我的书架', qLibD: '管理你的漫画资源<br>与阅读进度',
    qSet: '阅读设置', qSetD: '自定义阅读模式<br>享受舒适体验', qImg: '图片模式', qImgD: '支持卷轴与翻页<br>多种阅读方式',
    recent: '最近阅读', added: '最近添加', more: '查看更多', noRecent: '还没有阅读记录，导入一本漫画开始吧。', noAdded: '导入后会显示在这里。',
    pageOf: '第 {p} 页 / 共 {n} 页', open: '继续阅读', reveal: '在资源管理器中显示', imported: '已导入 {n} 本', importNone: '没有新增（已存在或格式不支持）',
    importFail: '导入失败：{e}', back: '返回', settingsQuick: '阅读设置', width: '页面宽度', gap: '页间距', bg: '背景', dim: '亮度', saver: '省内存模式',
    paper: '纸白', lavender: '薰衣草', night: '夜间', black: '纯黑', jump: '跳转到页', errMissing: '找不到文件', errCorrupt: '文件无法读取', retry: '重试',
    locate: '重新定位文件…', fullscreen: '全屏', min: '最小化', max: '最大化', close: '关闭',
  },
  en: {
    tagline: 'A better way to read comics', home: 'Home', library: 'Library', import: 'Local import', settings: 'Settings',
    quote: 'Comics are more than images,<br>they are another world.', search: 'Search titles, authors, tags...',
    welcome: 'Welcome to mh-reader', welcomeSub: 'Many formats · Local reading · Clean experience',
    dropTitle: 'Drop folders or archives here', dropSub: 'Supports .zip, .cbz, image folders and loose images',
    pick: 'Choose files / folder', pickFiles: 'Choose archives or images', pickFolder: 'Choose a folder',
    dropNote: 'Read locally · Offline · Extracted on demand', quick: 'Quick start',
    qImport: 'Local import', qImportD: 'Archives and folders<br>imported in one step', qLib: 'My library', qLibD: 'Manage your comics<br>and reading progress',
    qSet: 'Reading settings', qSetD: 'Customize reading mode<br>for comfort', qImg: 'Image mode', qImgD: 'Scroll or page-flip<br>reading styles',
    recent: 'Recent reading', added: 'Recently added', more: 'See more', noRecent: 'No reading history yet. Import a comic to start.', noAdded: 'Imported books show up here.',
    pageOf: 'Page {p} of {n}', open: 'Continue reading', reveal: 'Show in Explorer', imported: 'Imported {n}', importNone: 'Nothing new (already added or unsupported)',
    importFail: 'Import failed: {e}', back: 'Back', settingsQuick: 'Reading settings', width: 'Page width', gap: 'Page gap', bg: 'Background', dim: 'Brightness', saver: 'Memory saver',
    paper: 'Paper', lavender: 'Lavender', night: 'Night', black: 'Black', jump: 'Jump to page', errMissing: 'File not found', errCorrupt: 'File could not be read', retry: 'Retry',
    locate: 'Locate file…', fullscreen: 'Fullscreen', min: 'Minimize', max: 'Maximize', close: 'Close',
    // chapters (English only; the zh table has no entries, so t() falls back to these)
    episodeOf: 'Episode {i} / {n}', chapters: 'Chapters', chNone: 'None', chFolder: 'Folder', chPages: 'Pages',
    pagesPerEp: 'Pages per episode', rememberBook: 'Remember for this book', resetDefault: 'Use default',
    noFolders: 'No sub-folders found, so this book has no chapters.', prevEp: 'Previous episode', nextEp: 'Next episode',
    chDefaultTitle: 'Chapter mode (default)', chDefaultHelp: 'Applies to every book unless you change it for one book in the reader.',
    chNoneHelp: 'Continuous: one folder or ZIP is one book, with no chapter UI.', chFolderHelp: 'Episodes follow the folders inside the book.',
    chPagesHelp: 'Every N pages is one episode. The last one may be shorter.', saved: 'Saved',
    // bookmarks
    bookmarks: 'Bookmarks', bmAdd: 'Bookmark this image (B)', bmPanel: 'Bookmarks panel (T)', bmAdded: 'Bookmark added', bmRemoved: 'Bookmark removed',
    bmEmpty: 'No bookmarks yet. Press B to bookmark the image in the middle of the screen.', bmBroken: 'This image is no longer in the book.',
    bmGoOld: 'Go to old position (page {p})', bmReanchor: 'Anchor to page {p}', bmDelete: 'Delete', bmNote: 'Add a note', pageOfN: 'Page {p} / {n}',
  },
};
export const lang = localStorage.getItem('mh.lang') || 'zh';
export const t = (k, v = {}) => (S[lang][k] ?? S.en[k] ?? k).replace(/\{(\w+)\}/g, (_, n) => v[n] ?? '');
export const esc = (s) => String(s).replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

export function applyI18n(root = document) {
  document.documentElement.lang = lang === 'zh' ? 'zh-CN' : 'en';
  root.querySelectorAll('[data-i]').forEach(e => { e.innerHTML = t(e.dataset.i); });      // strings are ours, not user data
  root.querySelectorAll('[data-i-ph]').forEach(e => { e.placeholder = t(e.dataset.iPh); });
  root.querySelectorAll('[data-i-label]').forEach(e => { e.setAttribute('aria-label', t(e.dataset.iLabel)); e.title = t(e.dataset.iLabel); });
}
export const applyTheme = () => { document.documentElement.dataset.theme = localStorage.getItem('mh.theme') || 'light'; };

export function ago(ms) {
  const rtf = new Intl.RelativeTimeFormat(lang === 'zh' ? 'zh-CN' : 'en', { numeric: 'auto' });
  const s = Math.round((ms - Date.now()) / 1000), a = Math.abs(s);
  if (a < 3600) return rtf.format(Math.round(s / 60), 'minute');
  if (a < 86400) return rtf.format(Math.round(s / 3600), 'hour');
  return rtf.format(Math.round(s / 86400), 'day');
}

let toastTimer = 0;
export function toast(msg) {
  let el = document.getElementById('toast');
  if (!el) { el = Object.assign(document.createElement('div'), { id: 'toast', className: 'toast', hidden: true }); document.body.append(el); }
  el.textContent = msg; el.hidden = false; clearTimeout(toastTimer); toastTimer = setTimeout(() => { el.hidden = true; }, 3200);
}

export function wireWindow(root = document) {
  root.querySelectorAll('[data-win]').forEach(b => b.addEventListener('click', () => {
    const a = b.dataset.win; if (a === 'min') win.minimize(); else if (a === 'max') win.toggleMaximize(); else win.close();
  }));
  const sync = async () => document.querySelector('.window')?.classList.toggle('is-max', await win.isMaximized());
  win.onResized(sync); sync();
}
