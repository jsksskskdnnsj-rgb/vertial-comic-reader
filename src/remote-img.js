// Remote pictures in the window are fetched by the app (mhremote), so they follow the in-app proxy.

export function remoteImgSrcWith(base, url) {
  const u = String(url || '').trim();
  if (!/^https?:\/\//i.test(u)) return '';
  const root = String(base || '').replace(/\/$/, '');
  if (!root) return '';
  return `${root}/img?u=${encodeURIComponent(u)}`;
}

export function remoteImgSrc(url, convertFileSrc) {
  const convert = convertFileSrc
    || (typeof window !== 'undefined' ? window.__TAURI__?.core?.convertFileSrc : null);
  if (typeof convert !== 'function') return '';
  return remoteImgSrcWith(convert('', 'mhremote'), url);
}
