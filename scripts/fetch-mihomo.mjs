#!/usr/bin/env node
// Download the pinned Windows mihomo build and check its size and sha256.
// `tauri build` on Windows runs this and refuses to continue if the check fails.
// On other systems a failed download only prints a warning, because those
// builds do not ship the Windows sidecar (build.rs supplies a placeholder).
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '..');
const PIN = JSON.parse(fs.readFileSync(path.join(HERE, 'mihomo-pin.json'), 'utf8'));
const dest = path.join(ROOT, PIN.dest);
const required = process.platform === 'win32' || process.env.MIHOMO_REQUIRED === '1';

function sha256(buf) {
  return crypto.createHash('sha256').update(buf).digest('hex');
}

function fail(msg) {
  console.error(msg);
  if (required) process.exit(1);
  console.error('mihomo: continuing because this is not a Windows package build');
  process.exit(0);
}

if (fs.existsSync(dest)) {
  const buf = fs.readFileSync(dest);
  if (buf.length > 1_000_000) {
    const zipNote = path.join(ROOT, 'src-tauri/binaries/.mihomo-zip-sha256');
    if (fs.existsSync(zipNote) && fs.readFileSync(zipNote, 'utf8').trim() === PIN.sha256) {
      console.log(`skip ${PIN.dest} (pinned zip already unpacked)`);
      process.exit(0);
    }
  }
}

const tmp = path.join(ROOT, 'src-tauri/binaries/.mihomo-download.zip');
fs.mkdirSync(path.dirname(dest), { recursive: true });
console.log(`fetch ${PIN.url}`);
let zip;
try {
  const res = await fetch(PIN.url, { redirect: 'follow' });
  if (!res.ok) fail(`mihomo: download HTTP ${res.status}`);
  zip = Buffer.from(await res.arrayBuffer());
} catch (err) {
  fail(`mihomo: download failed (${err instanceof Error ? err.message : 'network'})`);
}
if (zip.length !== PIN.bytes) fail(`mihomo: size ${zip.length}, expected ${PIN.bytes}`);
const got = sha256(zip);
if (got !== PIN.sha256) fail(`mihomo: sha256 ${got}, expected ${PIN.sha256}`);
fs.writeFileSync(tmp, zip);

const extract = path.join(ROOT, 'src-tauri/binaries/.mihomo-extract');
fs.rmSync(extract, { recursive: true, force: true });
fs.mkdirSync(extract, { recursive: true });
try {
  execFileSync('python3', ['-c', 'import sys,zipfile; zipfile.ZipFile(sys.argv[1]).extractall(sys.argv[2])', tmp, extract], { stdio: 'inherit' });
} catch {
  fail('mihomo: could not unzip the archive');
}
const found = [];
function walk(dir) {
  for (const name of fs.readdirSync(dir)) {
    const p = path.join(dir, name);
    if (fs.statSync(p).isDirectory()) walk(p);
    else if (name.toLowerCase() === PIN.exeName.toLowerCase() || name.toLowerCase().endsWith('.exe')) found.push(p);
  }
}
walk(extract);
if (!found.length) fail(`mihomo: ${PIN.exeName} was not in the zip`);
fs.copyFileSync(found[0], dest);
fs.writeFileSync(path.join(ROOT, 'src-tauri/binaries/.mihomo-zip-sha256'), PIN.sha256);
fs.rmSync(tmp, { force: true });
fs.rmSync(extract, { recursive: true, force: true });
console.log(`wrote ${PIN.dest} from ${PIN.file}`);
