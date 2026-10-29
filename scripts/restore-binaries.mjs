#!/usr/bin/env node
// Restores the binary files that cannot be pushed as text.
// The GitHub snapshot push used for this repo is UTF-8 text only, so the
// app icons and the default wallpaper/banner are stored base64-encoded in
// scripts/restore-binaries.json. The icons match the output of
// `python scripts/gen-icons.py` (that script overwrites icons/; run it again
// after a restore if you want to redraw them). Run from anywhere:
//   node scripts/restore-binaries.mjs
// Files that already exist with a matching sha256 are left untouched.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '..');
const DATA = JSON.parse(fs.readFileSync(path.join(HERE, 'restore-binaries.json'), 'utf8'));

let ok = 0, wrote = 0, skipped = 0;
for (const f of DATA.files) {
  const target = path.join(ROOT, f.path);
  const buf = Buffer.from(f.base64, 'base64');
  if (buf.length !== f.bytes) {
    console.error(`FAIL ${f.path}: decoded ${buf.length} bytes, expected ${f.bytes}`);
    process.exitCode = 1;
    continue;
  }
  if (fs.existsSync(target)) {
    const cur = crypto.createHash('sha256').update(fs.readFileSync(target)).digest('hex');
    if (cur === f.sha256) {
      console.log(`skip ${f.path} (already correct)`);
      skipped++;
      ok++;
      continue;
    }
  }
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.writeFileSync(target, buf);
  const got = crypto.createHash('sha256').update(buf).digest('hex');
  if (got !== f.sha256) {
    console.error(`FAIL ${f.path}: sha256 mismatch after write`);
    process.exitCode = 1;
    continue;
  }
  console.log(`wrote ${f.path}`);
  wrote++;
  ok++;
}
console.log(`done: ${ok}/${DATA.files.length} verified (${wrote} written, ${skipped} already correct)`);
if (ok !== DATA.files.length) process.exitCode = 1;
