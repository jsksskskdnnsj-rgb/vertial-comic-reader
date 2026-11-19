import assert from 'node:assert/strict';
import fs from 'node:fs';
import { cosersViewRect } from '../src/cosers-rect.js';

const host = { left: 100, top: 70, right: 900, bottom: 700, width: 800, height: 630 };
const rail = { left: 14, top: 70, right: 88, bottom: 700 };
const top = { left: 0, top: 0, right: 900, bottom: 58 };

const open = cosersViewRect(host, { rail, top, modal: false });
assert.deepEqual(open, { x: 100, y: 70, width: 800, height: 630 });
assert.ok(open.x >= rail.right);
assert.ok(open.y >= top.bottom);

assert.equal(cosersViewRect(host, { rail, top, modal: true }), null);

const covered = cosersViewRect(
  { left: 40, top: 20, right: 900, bottom: 700 },
  { rail, top, modal: false },
);
assert.equal(covered.x, rail.right);
assert.equal(covered.y, top.bottom);
assert.ok(covered.width >= 2 && covered.height >= 2);

const withChip = cosersViewRect(host, {
  rail,
  top,
  modal: false,
  scanFloat: { left: 90, top: 640, right: 240, bottom: 676, visible: true },
});
assert.equal(withChip.height, 640 - 6 - host.top);
assert.ok(withChip.y + withChip.height <= 640 - 6);

const rust = fs.readFileSync(new URL('../src-tauri/src/cosers.rs', import.meta.url), 'utf8');
assert.match(rust, /pub const COSERS_URL: &str = "https:\/\/coserslove\.com\/cosers"/);
assert.doesNotMatch(rust, /reqwest|scrape|adblock|initialization_script|on_download/i);

const js = fs.readFileSync(new URL('../src/cosers.js', import.meta.url), 'utf8');
assert.match(js, /cosers_show/);
assert.doesNotMatch(js, /fetch\(|DOMParser|innerHTML\s*=/);

const shell = fs.readFileSync(new URL('../src/shell-ui.js', import.meta.url), 'utf8');
assert.match(shell, /href: 'cosers\.html'/);
assert.match(shell, /cosers\.html/);

const pkg = JSON.parse(fs.readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
const conf = JSON.parse(fs.readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
const cargo = fs.readFileSync(new URL('../src-tauri/Cargo.toml', import.meta.url), 'utf8');
const cargoPkg = cargo.split(/^\[/m).find((section) => section.startsWith('package]'));
const cargoVersion = cargoPkg?.match(/^version = "([^"]+)"/m)?.[1];
assert.equal(typeof pkg.version, 'string');
assert.match(pkg.version, /^\d+\.\d+\.\d+$/);
assert.equal(conf.version, pkg.version);
assert.equal(cargoVersion, pkg.version);
