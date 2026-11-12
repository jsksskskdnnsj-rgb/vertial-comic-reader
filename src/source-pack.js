// Import and export of custom metadata sources and built-in base URLs.
// Proxy passwords, controller secrets, and the Bangumi token stay out of the file.

export function parseHeaders(text) {
  return String(text || '').split(/\r?\n/).map((line) => {
    const i = line.indexOf(':');
    if (i <= 0) return null;
    const name = line.slice(0, i).trim();
    const value = line.slice(i + 1).trim();
    if (!name) return null;
    return { name, value };
  }).filter(Boolean);
}

export function formatHeaders(list) {
  return (list || []).map((h) => `${h.name}: ${h.value}`).join('\n');
}

function copySource(s) {
  return {
    id: s.id || '',
    name: s.name || '',
    enabled: s.enabled !== false,
    useProxy: s.useProxy !== false,
    searchUrl: s.searchUrl || '',
    headers: (s.headers || []).map((h) => ({ name: h.name || '', value: h.value || '' })),
    format: s.format === 'html' ? 'html' : 'json',
    list: s.list || '',
    idPath: s.idPath || '',
    titlePath: s.titlePath || '',
    authorPath: s.authorPath || '',
    summaryPath: s.summaryPath || '',
    tagsPath: s.tagsPath || '',
    coverPath: s.coverPath || '',
    pagePath: s.pagePath || '',
  };
}

export function exportSourcePack(cfg) {
  const bases = cfg?.bases || {};
  return {
    bases: {
      bangumiApi: bases.bangumiApi || '',
      mangadexApi: bases.mangadexApi || '',
      jikanApi: bases.jikanApi || '',
      moegirlApi: bases.moegirlApi || '',
      mangaupdatesApi: bases.mangaupdatesApi || '',
      anilistApi: bases.anilistApi || '',
    },
    customSources: (cfg?.customSources || []).map(copySource),
  };
}

export function applySourcePack(cfg, pack) {
  const next = { ...(cfg || {}) };
  if (pack?.bases && typeof pack.bases === 'object') {
    next.bases = { ...(next.bases || {}), ...pack.bases };
  }
  if (Array.isArray(pack?.customSources)) {
    next.customSources = pack.customSources.map(copySource);
  }
  return next;
}
