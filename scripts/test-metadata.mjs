#!/usr/bin/env node
// Metadata module wiring: i18n parity, invoke names, HTML hooks. No live network.
import fs from 'node:fs';
import path from 'node:path';

const ROOT = process.cwd();
const SRC = path.join(ROOT, 'src');
const zh = fs.readFileSync(path.join(SRC, 'zh-CN.js'), 'utf8');
const en = fs.readFileSync(path.join(SRC, 'en.js'), 'utf8');
const api = fs.readFileSync(path.join(SRC, 'api.js'), 'utf8');
const lib = fs.readFileSync(path.join(SRC, 'library.js'), 'utf8');
const book = fs.readFileSync(path.join(SRC, 'book.html'), 'utf8');
const settings = fs.readFileSync(path.join(SRC, 'settings.html'), 'utf8');

const metaKeys = [...zh.matchAll(/'meta\.[^']+'/g)].map((m) => m[0].slice(1, -1));
const missingEn = metaKeys.filter((k) => !en.includes(`'${k}'`));
if (missingEn.length) {
  console.error('missing en keys', missingEn);
  process.exit(1);
}

const commands = [
  'metadata_get_config', 'metadata_set_config', 'metadata_search', 'metadata_detail', 'metadata_get',
  'metadata_bind', 'metadata_unbind', 'metadata_automatch', 'metadata_list_pending', 'metadata_cover_local_path',
  'metadata_save_manual', 'metadata_refresh',
];
for (const c of commands) {
  if (!api.includes(`'${c}'`)) {
    console.error('api.js missing invoke', c);
    process.exit(1);
  }
}

if (!lib.includes("t('meta.matchMenu')")) {
  console.error('library menu missing metadata entry');
  process.exit(1);
}
if (!book.includes('metaFold')) {
  console.error('book.html missing metadata fold');
  process.exit(1);
}

if (!zh.includes('meta.openInBrowser') || !settings.includes('metaC18Domain')) {
  console.error('comic18 settings hooks missing');
  process.exit(1);
}
if (!lib.includes('metaopen')) {
  console.error('library menu missing open in browser');
  process.exit(1);
}

console.log('test-metadata: ok', metaKeys.length, 'keys');
