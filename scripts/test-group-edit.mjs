import test from 'node:test';
import assert from 'node:assert/strict';
import { eachAlone, moveIndex, pathsToSplit, removePlan, splitPlan } from '../src/group-edit.js';
import { applySourcePack, exportSourcePack, parseHeaders } from '../src/source-pack.js';
import { remoteImgSrcWith } from '../src/remote-img.js';

test('unchecked review rows are the ones split out of the set', () => {
  const members = [
    { path: 'a', keep: true },
    { path: 'b', keep: false },
    { path: 'c' },
    { path: '', keep: false },
  ];
  assert.deepEqual(pathsToSplit(members), ['b']);
  assert.deepEqual(pathsToSplit(members.map((m) => ({ ...m, keep: true }))), []);
});

test('reorder and removal plans keep a volume in the group', () => {
  const parts = [
    { key: 'a', paths: ['A'] },
    { key: 'b', paths: ['B'] },
    { key: 'c', paths: ['C'] },
  ];
  assert.deepEqual(moveIndex(parts, 2, 0).map((p) => p.key), ['c', 'a', 'b']);
  assert.deepEqual(moveIndex(parts, 0, 0), parts);
  const removed = removePlan(parts, ['b']);
  assert.equal(removed.ok, true);
  assert.deepEqual(removed.stay.map((p) => p.key), ['a', 'c']);
  assert.equal(removePlan(parts, ['a', 'b', 'c']).ok, false);
  const split = splitPlan(parts, ['a']);
  assert.deepEqual(split.groups, [['a'], ['b', 'c']]);
  assert.equal(splitPlan(parts, []).ok, false);
  assert.equal(eachAlone(parts).groups.length, 3);
  assert.equal(eachAlone([parts[0]]).ok, false);
});

test('source export keeps lookup rules and drops proxy secrets', () => {
  const cfg = {
    proxyPassword: 'hidden-pass',
    localSecret: 'hidden-secret',
    bangumiToken: 'hidden-token',
    bases: { mangadexApi: 'https://api.example' },
    customSources: [{
      id: 's1',
      name: 'Demo',
      searchUrl: 'https://ex/s?q={title}',
      headers: [{ name: 'X-Token', value: 'abc' }],
      format: 'json',
      titlePath: 'title',
    }],
  };
  const pack = exportSourcePack(cfg);
  const text = JSON.stringify(pack);
  assert.equal(text.includes('hidden-pass'), false);
  assert.equal(text.includes('hidden-secret'), false);
  assert.equal(text.includes('hidden-token'), false);
  assert.equal(pack.bases.mangadexApi, 'https://api.example');
  assert.equal(pack.customSources[0].headers[0].value, 'abc');
  const next = applySourcePack({ proxyPassword: 'hidden-pass', customSources: [] }, pack);
  assert.equal(next.proxyPassword, 'hidden-pass');
  assert.equal(next.customSources[0].name, 'Demo');
  assert.deepEqual(parseHeaders('Accept: text/html\nbad\nX-Test: 1'), [
    { name: 'Accept', value: 'text/html' },
    { name: 'X-Test', value: '1' },
  ]);
});

test('remote image urls go through the local protocol and reject other schemes', () => {
  const src = remoteImgSrcWith('http://mhremote.localhost', 'https://cdn.example/a.jpg');
  assert.equal(src, 'http://mhremote.localhost/img?u=' + encodeURIComponent('https://cdn.example/a.jpg'));
  assert.equal(remoteImgSrcWith('mhremote://localhost', 'file:///tmp/a.jpg'), '');
  assert.equal(remoteImgSrcWith('', 'https://cdn.example/a.jpg'), '');
});
