// In-app proxy, bundled-core nodes, custom metadata sources, and built-in base URLs.
// Rendered into Settings and the Online metadata page. Nothing here changes the system proxy.
import { T, invoke, t, esc, toast } from './shell.js';
import { lib } from './api.js';
import { applySourcePack, exportSourcePack, formatHeaders, parseHeaders } from './source-pack.js';

const errText = (e) => String(e?.message || e || '');
const state = { host: null, custom: [], opts: {} };

const MIRRORS = [
  ['bangumiApi', 'meta.mirrorBangumi'],
  ['mangadexApi', 'meta.mirrorMangadex'],
  ['jikanApi', 'meta.mirrorJikan'],
  ['moegirlApi', 'meta.mirrorMoegirl'],
  ['mangaupdatesApi', 'meta.mirrorMangaupdates'],
  ['anilistApi', 'meta.mirrorAnilist'],
];

function blankSource() {
  return {
    id: `c${Date.now().toString(36)}`,
    name: '',
    enabled: true,
    useProxy: true,
    searchUrl: '',
    headers: [],
    format: 'json',
    list: '',
    idPath: 'id',
    titlePath: 'title',
    authorPath: 'author',
    summaryPath: 'summary',
    tagsPath: 'tags',
    coverPath: 'cover',
    pagePath: 'page',
    sample: '',
  };
}

function readCustom(host) {
  return [...host.querySelectorAll('[data-source]')].map((card) => ({
    id: card.dataset.source,
    name: card.querySelector('[data-f="name"]').value.trim(),
    enabled: card.querySelector('[data-f="enabled"]').checked,
    useProxy: true,
    searchUrl: card.querySelector('[data-f="searchUrl"]').value.trim(),
    headers: parseHeaders(card.querySelector('[data-f="headers"]').value),
    format: card.querySelector('[data-f="format"]').value,
    list: card.querySelector('[data-f="list"]').value.trim(),
    idPath: card.querySelector('[data-f="idPath"]').value.trim(),
    titlePath: card.querySelector('[data-f="titlePath"]').value.trim(),
    authorPath: card.querySelector('[data-f="authorPath"]').value.trim(),
    summaryPath: card.querySelector('[data-f="summaryPath"]').value.trim(),
    tagsPath: card.querySelector('[data-f="tagsPath"]').value.trim(),
    coverPath: card.querySelector('[data-f="coverPath"]').value.trim(),
    pagePath: card.querySelector('[data-f="pagePath"]').value.trim(),
    sample: card.querySelector('[data-f="sample"]').value,
  }));
}

export function readMountedNetwork() {
  const host = state.host;
  if (!host || !host.querySelector('#appProxy')) return {};
  const bases = {};
  for (const [key] of MIRRORS) {
    bases[key] = host.querySelector(`[data-base="${key}"]`)?.value.trim() || '';
  }
  const manual = document.getElementById('metaProxy');
  const own = host.querySelector('#metaProxyUrl');
  const out = {
    appProxy: host.querySelector('#appProxy').checked,
    proxyMode: host.querySelector('#proxyMode').value,
    proxyUsername: host.querySelector('#proxyUser').value,
    proxyPassword: host.querySelector('#proxyPass').value,
    localProxy: host.querySelector('#localProxy').value.trim(),
    localController: host.querySelector('#localCtl').value.trim(),
    localSecret: host.querySelector('#localSecret').value,
    localNode: host.querySelector('#localNode').value.trim(),
    localGroup: host.querySelector('#localGroup').value.trim(),
    coreNode: host.querySelector('#coreNode').value,
    bases,
    customSources: readCustom(host).map(({ sample, ...src }) => src),
  };
  if (own) out.proxyUrl = own.value.trim();
  else if (manual) out.proxyUrl = manual.value.trim();
  return out;
}

function modeLabel(mode) {
  if (mode === 'system') return t('meta.proxySystem');
  if (mode === 'manual') return t('meta.proxyManual');
  if (mode === 'local') return t('meta.proxyLocal');
  if (mode === 'core') return t('meta.proxyCore');
  return t('meta.proxyOff');
}

async function paintStatus() {
  const host = state.host;
  const line = host?.querySelector('#proxyState');
  if (!line) return;
  try {
    const st = await invoke('metadata_proxy_status');
    const node = st.node ? `${t('meta.nodePicked')}: ${st.node}` : '';
    const listen = st.listen ? st.listen : '';
    line.textContent = [st.enabled ? t('meta.appProxyOn') : t('meta.appProxyOff'), modeLabel(st.mode), node, listen, st.message]
      .filter(Boolean).join(' · ');
  } catch (e) {
    line.textContent = errText(e);
  }
}

function sourceCard(src) {
  const card = document.createElement('div');
  card.className = 'panel';
  card.dataset.source = src.id;
  card.innerHTML = `
    <div class="set-row">
      <label class="check"><input type="checkbox" data-f="enabled" ${src.enabled ? 'checked' : ''}> ${esc(t('meta.customEnabled'))}</label>
      <input class="field grow" data-f="name" value="${esc(src.name)}" placeholder="${esc(t('meta.customName'))}">
      <button type="button" class="btn btn--ghost btn--sm" data-del>${esc(t('meta.customDelete'))}</button>
    </div>
    <label class="set-row"><span>${esc(t('meta.customUrl'))}</span><input class="field grow" data-f="searchUrl" value="${esc(src.searchUrl)}"></label>
    <p class="hint muted">${esc(t('meta.customUrlHelp'))}</p>
    <label class="set-row"><span>${esc(t('meta.customFormat'))}</span>
      <select class="select" data-f="format"><option value="json" ${src.format === 'html' ? '' : 'selected'}>JSON</option><option value="html" ${src.format === 'html' ? 'selected' : ''}>HTML</option></select>
    </label>
    <label class="set-row"><span>${esc(t('meta.customList'))}</span><input class="field grow" data-f="list" value="${esc(src.list)}"></label>
    <label class="set-row"><span>${esc(t('meta.customId'))}</span><input class="field grow" data-f="idPath" value="${esc(src.idPath)}"></label>
    <label class="set-row"><span>${esc(t('meta.customTitlePath'))}</span><input class="field grow" data-f="titlePath" value="${esc(src.titlePath)}"></label>
    <label class="set-row"><span>${esc(t('meta.customAuthor'))}</span><input class="field grow" data-f="authorPath" value="${esc(src.authorPath)}"></label>
    <label class="set-row"><span>${esc(t('meta.customSummary'))}</span><input class="field grow" data-f="summaryPath" value="${esc(src.summaryPath)}"></label>
    <label class="set-row"><span>${esc(t('meta.customTags'))}</span><input class="field grow" data-f="tagsPath" value="${esc(src.tagsPath)}"></label>
    <label class="set-row"><span>${esc(t('meta.customCover'))}</span><input class="field grow" data-f="coverPath" value="${esc(src.coverPath)}"></label>
    <label class="set-row"><span>${esc(t('meta.customPage'))}</span><input class="field grow" data-f="pagePath" value="${esc(src.pagePath)}"></label>
    <label class="set-row"><span>${esc(t('meta.customHeaders'))}</span></label>
    <textarea class="field meta-area" data-f="headers">${esc(formatHeaders(src.headers))}</textarea>
    <label class="set-row"><span>${esc(t('meta.customSample'))}</span></label>
    <textarea class="field meta-area" data-f="sample">${esc(src.sample || '')}</textarea>
    <div class="meta-tools">
      <button type="button" class="btn btn--ghost btn--sm" data-parse>${esc(t('meta.customSample'))}</button>
      <button type="button" class="btn btn--ghost btn--sm" data-test>${esc(t('meta.customTest'))}</button>
    </div>
    <p class="hint muted" data-out></p>`;
  return card;
}

function paint(cfg) {
  const host = state.host;
  if (!host) return;
  state.custom = (cfg?.customSources || []).map((s, i) => ({
    ...blankSource(),
    ...s,
    id: s.id || `c${i}-${Date.now().toString(36)}`,
    sample: '',
  }));
  const bases = cfg?.bases || {};
  const mode = cfg?.proxyMode || 'core';
  const manualField = document.getElementById('metaProxy')
    ? ''
    : `<label class="set-row"><span>${esc(t('meta.proxy'))}</span><input class="field grow" id="metaProxyUrl" value="${esc(cfg?.proxyUrl || '')}" placeholder="http://127.0.0.1:7890"></label>`;
  host.innerHTML = `
    <div class="set-row">
      <div class="grow"><span>${esc(t('meta.appProxy'))}</span><span class="sub muted">${esc(t('meta.appProxyHelp'))}</span></div>
      <label class="check"><input type="checkbox" id="appProxy" ${cfg?.appProxy ? 'checked' : ''}></label>
    </div>
    <p class="hint" id="proxyState"></p>
    <label class="set-row"><span>${esc(t('meta.proxyMode'))}</span>
      <select class="select" id="proxyMode">
        <option value="system">${esc(t('meta.proxySystem'))}</option>
        <option value="manual">${esc(t('meta.proxyManual'))}</option>
        <option value="local">${esc(t('meta.proxyLocal'))}</option>
        <option value="core">${esc(t('meta.proxyCore'))}</option>
      </select>
    </label>
    ${manualField}
    <label class="set-row"><span>${esc(t('meta.proxyUser'))}</span><input class="field grow" id="proxyUser" value="${esc(cfg?.proxyUsername || '')}" autocomplete="off"></label>
    <label class="set-row"><span>${esc(t('meta.proxyPass'))}</span><input class="field grow" id="proxyPass" type="password" value="${esc(cfg?.proxyPassword || '')}" autocomplete="off"></label>
    <label class="set-row"><span>${esc(t('meta.localAddr'))}</span><input class="field grow" id="localProxy" value="${esc(cfg?.localProxy || 'http://127.0.0.1:7897')}"></label>
    <label class="set-row"><span>${esc(t('meta.localCtl'))}</span><input class="field grow" id="localCtl" value="${esc(cfg?.localController || '')}" placeholder="127.0.0.1:9090"></label>
    <label class="set-row"><span>${esc(t('meta.localSecret'))}</span><input class="field grow" id="localSecret" type="password" value="${esc(cfg?.localSecret || '')}" autocomplete="off"></label>
    <label class="set-row"><span>${esc(t('meta.localNode'))}</span><input class="field grow" id="localNode" value="${esc(cfg?.localNode || '')}"></label>
    <label class="set-row"><span>${esc(t('meta.localGroup'))}</span><input class="field grow" id="localGroup" value="${esc(cfg?.localGroup || '')}"></label>
    <p class="hint muted">${esc(t('meta.proxyAll'))}</p>
    <p class="hint muted">${esc(t('meta.noTun'))}</p>
    <p class="hint muted">${esc(t('meta.webviewNote'))}</p>
    <p class="hint muted">${esc(t('meta.gplNote'))}</p>
    <h3>${esc(t('meta.clashImport'))}</h3>
    <p class="hint muted">${esc(t('meta.subHelp'))}</p>
    <textarea class="field meta-area" id="clashPaste" placeholder="${esc(t('meta.clashPaste'))}"></textarea>
    <div class="meta-tools">
      <button type="button" class="btn btn--ghost btn--sm" id="clashImport">${esc(t('meta.clashImport'))}</button>
      <button type="button" class="btn btn--ghost btn--sm" id="clashFile">${esc(t('meta.clashFile'))}</button>
      <button type="button" class="btn btn--ghost btn--sm" id="clashReload">${esc(t('meta.clashNodes'))}</button>
      <button type="button" class="btn btn--ghost btn--sm" id="proxyTest">${esc(t('meta.proxyTest'))}</button>
      <button type="button" class="btn btn--ghost btn--sm" id="proxyStop">${esc(t('meta.proxyStop'))}</button>
    </div>
    <label class="set-row"><span>${esc(t('meta.coreNode'))}</span><select class="select" id="coreNode"></select></label>
    <div id="clashNodes"></div>
    <p class="hint muted">${esc(t('meta.clashNeedsCore'))}</p>
    <h3>${esc(t('meta.mirrors'))}</h3>
    <p class="hint muted">${esc(t('meta.mirrorHelp'))}</p>
    <div id="mirrorFields"></div>
    <h3>${esc(t('meta.customTitle'))}</h3>
    <div id="customSources"></div>
    <div class="meta-tools">
      <button type="button" class="btn btn--ghost btn--sm" id="customAdd">${esc(t('meta.customAdd'))}</button>
      <button type="button" class="btn btn--ghost btn--sm" id="customExport">${esc(t('meta.customExport'))}</button>
      <button type="button" class="btn btn--ghost btn--sm" id="customImport">${esc(t('meta.customImport'))}</button>
      <button type="button" class="btn btn--primary btn--sm" id="netSave">${esc(t('common.save'))}</button>
    </div>`;
  host.dataset.core = cfg?.coreNode || '';
  host.querySelector('#proxyMode').value = ['system', 'manual', 'local', 'core'].includes(mode) ? mode : 'core';
  const mirrors = host.querySelector('#mirrorFields');
  for (const [key, label] of MIRRORS) {
    const row = document.createElement('label');
    row.className = 'set-row';
    row.innerHTML = `<span>${esc(t(label))}</span><input class="field grow" data-base="${key}" value="${esc(bases[key] || '')}">`;
    mirrors.append(row);
  }
  const box = host.querySelector('#customSources');
  for (const src of state.custom) box.append(sourceCard(src));
  host.querySelector('#appProxy').addEventListener('change', () => saveNetwork(false));
  host.querySelector('#clashImport').addEventListener('click', () => importClash(host.querySelector('#clashPaste').value, null));
  host.querySelector('#clashFile').addEventListener('click', async () => {
    const picked = await T.dialog.open({ filters: [{ name: 'YAML', extensions: ['yml', 'yaml'] }] }).catch(() => null);
    const path = Array.isArray(picked) ? picked[0] : picked;
    if (!path) return;
    importClash(null, path);
  });
  host.querySelector('#clashReload').addEventListener('click', () => loadNodes());
  host.querySelector('#proxyTest').addEventListener('click', testProxy);
  host.querySelector('#proxyStop').addEventListener('click', stopCore);
  host.querySelector('#customAdd').addEventListener('click', () => {
    state.custom = readCustom(host);
    state.custom.push(blankSource());
    repaintCustom();
  });
  host.querySelector('#customExport').addEventListener('click', exportPack);
  host.querySelector('#customImport').addEventListener('click', importPack);
  host.querySelector('#netSave').addEventListener('click', () => saveNetwork(true));
  host.querySelector('#customSources').addEventListener('click', onCustomClick);
  paintStatus();
  loadNodes();
}

function repaintCustom() {
  const box = state.host.querySelector('#customSources');
  box.innerHTML = '';
  for (const src of state.custom) box.append(sourceCard(src));
}

async function onCustomClick(e) {
  const card = e.target.closest('[data-source]');
  if (!card) return;
  if (e.target.closest('[data-del]')) {
    state.custom = readCustom(state.host).filter((s) => s.id !== card.dataset.source);
    repaintCustom();
    return;
  }
  const src = readCustom(state.host).find((s) => s.id === card.dataset.source);
  if (!src) return;
  const out = card.querySelector('[data-out]');
  try {
    if (e.target.closest('[data-parse]')) {
      const items = await invoke('metadata_custom_parse', { sample: src.sample, source: stripSample(src) });
      out.textContent = items.map((item) => item.title).join(' · ') || t('meta.noHits');
    } else if (e.target.closest('[data-test]')) {
      const bookId = Number(state.opts.getBookId?.() || 0);
      if (!bookId) { toast(t('meta.noBook')); return; }
      const items = await invoke('metadata_custom_test', { bookId, source: stripSample(src) });
      out.textContent = (items || []).map((item) => item.title).join(' · ') || t('meta.noHits');
    }
  } catch (err) { out.textContent = errText(err); }
}

function stripSample(src) {
  const { sample, ...rest } = src;
  return rest;
}

async function importClash(text, path) {
  try {
    const nodes = await invoke('metadata_clash_import', { text: text || null, path: path || null });
    toast(t('common.saved'));
    fillNodes(nodes || []);
  } catch (e) { toast(errText(e)); }
}

async function loadNodes() {
  try { fillNodes(await invoke('metadata_clash_nodes')); }
  catch { fillNodes([]); }
}

function fillNodes(nodes) {
  const host = state.host;
  const sel = host.querySelector('#coreNode');
  const current = sel.value || host.dataset.core || '';
  sel.innerHTML = (nodes || []).map((n) => `<option value="${esc(n.name)}">${esc(n.name)} · ${esc(n.kind)}</option>`).join('');
  if (current && [...sel.options].some((o) => o.value === current)) sel.value = current;
  const list = host.querySelector('#clashNodes');
  list.innerHTML = '';
  for (const node of nodes || []) {
    const row = document.createElement('div');
    row.className = 'set-row';
    row.innerHTML = `<span class="grow">${esc(node.name)} · ${esc(node.kind)} · ${esc(node.server || '')}</span>`;
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'btn btn--ghost btn--sm';
    btn.textContent = t('meta.clashDelay');
    btn.addEventListener('click', async () => {
      btn.disabled = true;
      try {
        const r = await invoke('metadata_proxy_delay', { name: node.name });
        btn.textContent = r.message || t('meta.clashDelay');
      } catch (e) { toast(errText(e)); }
      btn.disabled = false;
    });
    const use = document.createElement('button');
    use.type = 'button';
    use.className = 'btn btn--ghost btn--sm';
    use.textContent = t('meta.coreNode');
    use.addEventListener('click', () => { sel.value = node.name; host.querySelector('#proxyMode').value = 'core'; });
    row.append(btn, use);
    list.append(row);
  }
}

async function testProxy() {
  try {
    const r = await invoke('metadata_proxy_test');
    toast(r.message || t('meta.proxyTest'));
    paintStatus();
  } catch (e) { toast(errText(e)); }
}

async function stopCore() {
  try {
    await invoke('metadata_proxy_stop');
    toast(t('meta.proxyStopped'));
    paintStatus();
  } catch (e) { toast(errText(e)); }
}

function overlaySettingsForm(next) {
  const token = document.getElementById('metaBgmToken');
  if (!token) return next;
  next.bangumiToken = token.value.trim();
  const proxy = document.getElementById('metaProxy');
  if (proxy) next.proxyUrl = proxy.value.trim();
  next.sources = {
    bangumi: document.getElementById('metaSrcBgm').checked,
    mangaupdates: document.getElementById('metaSrcMu').checked,
    anilist: document.getElementById('metaSrcAni').checked,
    comic18: document.getElementById('metaSrcC18').checked,
    moegirl: document.getElementById('metaSrcMoe').checked,
    mangadex: document.getElementById('metaSrcMdx').checked,
    jikan: document.getElementById('metaSrcJikan').checked,
  };
  next.proxyPerSource = {
    bangumi: document.getElementById('metaProxyBgm').checked,
    mangaupdates: document.getElementById('metaProxyMu').checked,
    anilist: document.getElementById('metaProxyAni').checked,
    comic18: document.getElementById('metaProxyC18').checked,
    moegirl: document.getElementById('metaProxyMoe').checked,
    mangadex: document.getElementById('metaProxyMdx').checked,
    jikan: document.getElementById('metaProxyJk').checked,
  };
  next.comic18Domain = document.getElementById('metaC18Domain').value.trim() || '18comic.vip';
  next.rateLimitMs = Math.min(5000, Math.max(200, Number(document.getElementById('metaRate').value) || 400));
  next.autoBindConfident = document.getElementById('metaAutoBind').checked;
  return next;
}

async function saveNetwork(announce) {
  try {
    const base = await lib.metadata.getConfig();
    const next = overlaySettingsForm({ ...base, ...readMountedNetwork() });
    await lib.metadata.setConfig(next);
    state.opts.onSaved?.(next);
    if (announce) toast(t('common.saved'));
    paintStatus();
  } catch (e) { toast(errText(e)); }
}

async function exportPack() {
  const base = await lib.metadata.getConfig().catch(() => ({}));
  const pack = exportSourcePack({ ...base, ...readMountedNetwork() });
  const blob = new Blob([JSON.stringify(pack, null, 2)], { type: 'application/json' });
  const link = document.createElement('a');
  link.href = URL.createObjectURL(blob);
  link.download = 'mh-sources.json';
  link.click();
  URL.revokeObjectURL(link.href);
  toast(t('common.saved'));
}

async function importPack() {
  const input = document.createElement('input');
  input.type = 'file';
  input.accept = 'application/json,.json';
  input.addEventListener('change', async () => {
    const file = input.files?.[0];
    if (!file) return;
    try {
      const pack = JSON.parse(await file.text());
      const base = await lib.metadata.getConfig();
      const next = overlaySettingsForm(applySourcePack({ ...base, ...readMountedNetwork() }, pack));
      await lib.metadata.setConfig(next);
      state.opts.onSaved?.(next);
      paint(next);
      toast(t('common.saved'));
    } catch (e) { toast(errText(e)); }
  });
  input.click();
}

export function mountNetworkEditor(host, opts = {}) {
  state.host = host;
  state.opts = opts;
  if (opts.cfg?.coreNode) host.dataset.core = opts.cfg.coreNode;
  paint(opts.cfg || {});
}

export function refreshNetwork(cfg) {
  if (!state.host) return;
  if (state.host.contains(document.activeElement)) return;
  if (cfg?.coreNode) state.host.dataset.core = cfg.coreNode;
  paint(cfg || {});
}
