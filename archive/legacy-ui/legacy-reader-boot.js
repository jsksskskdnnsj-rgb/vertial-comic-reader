/* MH阅读器 - 阅读页启动脚本（v2）
 * reader.html?id=<shelf_id>：从 SQLite 书架加载漫画，
 * 页面数据经 Rust 按需提供（ZIP 单页解压 / 文件夹直接读），
 * 复用页面原有的 openComic / goToPage / state.current。
 */
(function () {
  if (!window.__TAURI__ || !window.MH) return;
  var invoke = MH.invoke, cfs = MH.fileSrc;

  var id = new URLSearchParams(location.search).get('id');
  var iv = document.getElementById('importView');
  if (iv) iv.style.display = 'none'; // v2 阅读页不再用旧导入页
  if (!id) { location.href = 'legacy.html'; return; }
  var sid = +id;

  function blobOf(url) {
    return fetch(url).then(function (r) {
      if (!r.ok) throw new Error('read failed');
      return r.blob();
    });
  }
  function baseName(p) { return MH.baseName(p); }

  // 懒加载 entry：阅读器管线调 entry.async("blob") 时才真正取数
  function zipPageEntry(zip_id, index) {
    return {
      async: function () {
        return invoke('zip_extract_page', { zip_id: zip_id, index: index })
          .then(function (p) { return blobOf(cfs(p)); });
      }
    };
  }
  function fileEntry(abs) {
    return {
      async: function () { return blobOf(cfs(abs)); }
    };
  }

  function buildPages(entry) {
    var paths;
    try { paths = JSON.parse(entry.paths || '[]'); } catch (e) { paths = []; }
    if (entry.kind === 'zip' && paths[0]) {
      return invoke('zip_open', { path: paths[0] }).then(function (info) {
        var es = info.entries
          .filter(function (e) { return MH.imgRe.test(e.name); })
          .sort(function (a, b) { return MH.fuzzyCompare(a.name, b.name); });
        return es.map(function (e) {
          return { name: baseName(e.name), sortName: e.name, entry: zipPageEntry(info.zip_id, e.index) };
        });
      });
    }
    var filesP = entry.kind === 'folder' && paths[0]
      ? invoke('list_images', { path: paths[0] }).then(function (fs) {
          return fs.filter(function (f) { return MH.imgRe.test(f.name); });
        })
      : Promise.resolve(paths.map(function (p) { return { name: p, path: p }; }));
    return filesP.then(function (files) {
      files.sort(function (a, b) { return MH.fuzzyCompare(a.name, b.name); });
      return files.map(function (f) {
        return { name: baseName(f.name), sortName: f.name, entry: fileEntry(f.path) };
      });
    });
  }

  function saveProgress() {
    try {
      var cur = (window.state && window.state.current) || 0;
      invoke('shelf_progress', { id: sid, page: cur });
    } catch (e) {}
  }

  invoke('shelf_get', { id: sid }).then(function (entry) {
    if (!entry) {
      toast('找不到这本漫画');
      setTimeout(function () { location.href = 'legacy.html'; }, 1200);
      return;
    }
    toast('正在打开…');
    var cfgP = invoke('get_reader_config').catch(function(){ return null; });
    var themeP = invoke('get_theme_config').catch(function(){ return null; });
    Promise.all([buildPages(entry), cfgP, themeP]).then(function (res) {
      var pages = res[0], cfg = res[1] || {}, tc = res[2];
      if (!pages.length) { toast('没有找到图片'); return; }
      // 应用阅读配置（默认阅读模式 / 省内存模式）
      try {
        if (cfg.mode === 'page' || cfg.mode === 'scroll') window.state.mode = cfg.mode;
        window.state.memorySaver = !!cfg.memory_saver;
      } catch (e) {}
      // 应用主题主色
      try {
        var PRI = { lavender: '#8b5cf6', pink: '#ec4899', blue: '#3b82f6', mint: '#10b981', dark: '#a78bfa' };
        var pri = tc ? (tc.theme === 'custom' ? tc.custom_pri : (PRI[tc.theme] || PRI.lavender)) : null;
        if (pri) {
          var h = String(pri).replace('#', ''), r, g, b;
          if (h.length === 3) h = h.split('').map(function (c) { return c + c; }).join('');
          var n = parseInt(h, 16) || 0;
          r = n >> 16 & 255; g = n >> 8 & 255; b = n & 255;
          var rs = document.documentElement.style;
          rs.setProperty('--accent', pri);
          rs.setProperty('--accent-ink', '#ffffff');
          rs.setProperty('--accent-soft', 'rgba(' + r + ',' + g + ',' + b + ',0.12)');
        }
      } catch (e) {}
      openComic(pages, entry.name);
      var start = Math.max(0, Math.min(entry.last_page || 0, pages.length - 1));
      if (start > 0) {
        setTimeout(function () {
          goToPage(start);
          toast('已回到第 ' + (start + 1) + ' 页');
        }, 500);
      }
      setInterval(saveProgress, 4000);
      window.addEventListener('pagehide', saveProgress);
    }).catch(function () { toast('打开失败，文件可能被移动了'); });
  }).catch(function () { toast('读取书架失败'); });

  // 无边框窗口按钮（阅读页顶栏，走 Rust 命令）
  function winAct(a) {
    var cmd = a === 'min' ? 'win_minimize' : a === 'max' ? 'win_toggle_maximize' : 'win_close';
    invoke(cmd).catch(function () {});
  }
  [['wMinR','min'],['wMaxR','max'],['wCloseR','close']].forEach(function (x) {
    var b = document.getElementById(x[0]);
    if (b) b.addEventListener('click', function () { winAct(x[1]); });
  });
})();
