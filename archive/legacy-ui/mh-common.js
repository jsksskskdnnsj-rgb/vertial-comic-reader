/* MH阅读器 - 前端共享库（library 与 reader 共用） */
(function () {
  var T = (typeof window !== 'undefined' && window.__TAURI__) || null;
  var core = T ? T.core : null;

  function invoke(cmd, args) {
    if (!core) return Promise.reject(new Error('not in tauri'));
    return core.invoke(cmd, args || {});
  }
  function fileSrc(absPath) {
    if (!core || !core.convertFileSrc) return absPath;
    return core.convertFileSrc(absPath);
  }

  /* ---- 模糊文件名排序（与阅读器内 fuzzyCompare 同算法） ---- */
  var CN_DIGIT = { 零: 0, 一: 1, 二: 2, 三: 3, 四: 4, 五: 5, 六: 6, 七: 7, 八: 8, 九: 9 };
  function normName(s) {
    s = String(s || '');
    s = s.replace(/[０-９]/g, function (c) { return String.fromCharCode(c.charCodeAt(0) - 0xfee0); });
    s = s.replace(/[零一二三四五六七八九]/g, function (c) { return CN_DIGIT[c]; });
    // “第12话” 这类中文数字组合里的十/百：简单处理成阿拉伯数字
    s = s.replace(/十([一二三四五六七八九])/g, function (m, d) { return '1' + CN_DIGIT[d]; });
    s = s.replace(/([一二三四五六七八九])十([一二三四五六七八九])?/g, function (m, a, b) {
      return String(CN_DIGIT[a] * 10 + (b ? CN_DIGIT[b] : 0));
    });
    return s;
  }
  function numChunks(s) {
    s = normName(s);
    var parts = [], re = /(\d+)|(\D+)/g, m;
    while ((m = re.exec(s))) {
      if (m[1] !== undefined) parts.push({ n: parseInt(m[1], 10) });
      else parts.push({ s: m[2] });
    }
    return parts;
  }
  function hasNum(parts) { return parts.some(function (p) { return p.n !== undefined; }); }
  function fuzzyCompare(a, b) {
    var pa = numChunks(a), pb = numChunks(b);
    var ha = hasNum(pa), hb = hasNum(pb);
    if (ha !== hb) return ha ? 1 : -1; // 无数字的文件（封面）排前面
    var len = Math.max(pa.length, pb.length);
    for (var i = 0; i < len; i++) {
      var x = pa[i], y = pb[i];
      if (!x) return -1;
      if (!y) return 1;
      var xn = x.n !== undefined, yn = y.n !== undefined;
      if (xn && yn) { if (x.n !== y.n) return x.n - y.n; }
      else if (xn) return -1;
      else if (yn) return 1;
      else if (x.s !== y.s) return x.s < y.s ? -1 : 1;
    }
    return 0;
  }

  /* ---- 格式化 ---- */
  function fmtBytes(n) {
    n = +n || 0;
    if (n < 1024) return n + ' B';
    if (n < 1048576) return (n / 1024).toFixed(1) + ' KB';
    if (n < 1073741824) return (n / 1048576).toFixed(1) + ' MB';
    return (n / 1073741824).toFixed(2) + ' GB';
  }
  function fmtTime(ts) {
    if (!ts) return '—';
    var d = new Date(ts * 1000), now = new Date();
    var p = function (x) { return (x < 10 ? '0' : '') + x; };
    var sameDay = d.toDateString() === now.toDateString();
    var yest = new Date(now - 864e5).toDateString() === d.toDateString();
    var hm = p(d.getHours()) + ':' + p(d.getMinutes());
    if (sameDay) return '今天 ' + hm;
    if (yest) return '昨天 ' + hm;
    return d.getFullYear() + '-' + p(d.getMonth() + 1) + '-' + p(d.getDate()) + ' ' + hm;
  }
  function baseName(p) {
    return String(p || '').split('/').pop().split('\\').pop();
  }
  function stripExt(n) { return baseName(n).replace(/\.[^.]+$/, ''); }

  var IMG_RE = /\.(jpe?g|jfif|png|apng|webp|gif|bmp|dib|avif|svg|ico|tif|tiff|heic|heif|jxl)$/i;

  /* ---- 封面缩略图：把图片 URL 压成 dataURL
        先取成 blob（不依赖服务端 MIME）；优先用 createImageBitmap 直接降采样解码，
        大图不解全分辨率、不爆内存、不卡主线程 ---- */
  function makeCover(url, width) {
    width = width || 300;
    function viaImage(objUrl) {
      return new Promise(function (resolve) {
        var img = new Image();
        img.onload = function () {
          URL.revokeObjectURL(objUrl);
          try {
            var w = width, h = Math.max(1, Math.round(width * img.naturalHeight / Math.max(1, img.naturalWidth)));
            var c = document.createElement('canvas');
            c.width = w; c.height = h;
            c.getContext('2d').drawImage(img, 0, 0, w, h);
            resolve(c.toDataURL('image/jpeg', 0.72));
          } catch (e) { resolve(''); }
        };
        img.onerror = function () { URL.revokeObjectURL(objUrl); resolve(''); };
        img.src = objUrl;
      });
    }
    return fetch(url).then(function (r) {
      if (!r.ok) throw new Error('fetch failed');
      return r.blob();
    }).then(function (b) {
      if (typeof createImageBitmap === 'function') {
        return createImageBitmap(b, { resizeWidth: width, resizeQuality: 'high' }).then(function (bm) {
          try {
            var c = document.createElement('canvas');
            c.width = bm.width; c.height = bm.height;
            c.getContext('2d').drawImage(bm, 0, 0);
            var u = c.toDataURL('image/jpeg', 0.72);
            if (bm.close) bm.close();
            return u;
          } catch (e) { return ''; }
        }).catch(function () { return viaImage(URL.createObjectURL(b)); });
      }
      return viaImage(URL.createObjectURL(b));
    }).catch(function () { return ''; });
  }

  function esc(s) {
    return String(s == null ? '' : s).replace(/[&<>"']/g, function (c) {
      return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c];
    });
  }

  window.MH = {
    tauri: !!T,
    invoke: invoke,
    fileSrc: fileSrc,
    fuzzyCompare: fuzzyCompare,
    fmtBytes: fmtBytes,
    fmtTime: fmtTime,
    baseName: baseName,
    stripExt: stripExt,
    imgRe: IMG_RE,
    makeCover: makeCover,
    esc: esc
  };
})();
