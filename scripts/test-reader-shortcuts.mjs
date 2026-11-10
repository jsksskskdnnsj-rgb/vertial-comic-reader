#!/usr/bin/env node
import assert from 'node:assert/strict';
import { shortcutHelpRows } from '../src/reader-shortcuts.js';

const scroll = shortcutHelpRows('scroll');
const flip = shortcutHelpRows('flip');
assert.ok(scroll.some((r) => r.i18n === 'help.space'));
assert.ok(!flip.some((r) => r.i18n === 'help.space'));
assert.ok(flip.some((r) => r.i18n === 'help.flipTap'));
console.log('test-reader-shortcuts: ok');
