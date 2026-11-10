#!/usr/bin/env node
// Mirrors comic18.rs challenge detection and URL building (no network).
const BLOCKED_HINT =
  '该站需要浏览器验证，已跳过自动抓取；可在书中使用「在浏览器打开资料页」手动查看';

function isChallengePage(status, html) {
  if (status === 403 || status === 503) return true;
  const probe = html.toLowerCase();
  return (
    probe.includes('challenges.cloudflare.com') ||
    probe.includes('cf-mitigated') ||
    probe.includes('just a moment') ||
    (probe.includes('cloudflare') &&
      probe.includes('challenge') &&
      !probe.includes('/album/') &&
      !probe.includes('book-name'))
  );
}

function searchUrl(host, query) {
  const q = encodeURIComponent(query.trim()).replace(/%20/g, '+');
  return `https://${host}/search/photos?main_tag=0&search_query=${q}`;
}

if (!isChallengePage(403, '')) process.exit(1);
if (isChallengePage(200, '<h1 id="book-name">x</h1>')) process.exit(1);
if (!isChallengePage(200, 'challenges.cloudflare.com')) process.exit(1);
const u = searchUrl('18comic.vip', '韩漫 测试');
if (!u.includes('18comic.vip') || !u.includes('search_query')) process.exit(1);
console.log('test-comic18-parse: ok', BLOCKED_HINT.length > 0);
