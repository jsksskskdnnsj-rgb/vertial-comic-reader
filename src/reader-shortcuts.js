// reader-shortcuts.js — reader keyboard help rows (pure data + filter by view mode).
// Structure inspired by Yomikiru src/common/keybindings/catalog.ts command groups (MIT,
// Copyright (c) 2023 mienaiyami); labels are MH i18n keys, not copied strings.

/** @typedef {{ keys: string, i18n: string, modes?: ('scroll'|'flip'|'both')[] }} ShortcutRow */

/** @type {ShortcutRow[]} */
const ROWS = [
  { keys: 'Space  ·  PgDn', i18n: 'help.space', modes: ['scroll'] },
  { keys: 'Shift+Space  ·  PgUp', i18n: 'help.pageUp', modes: ['scroll'] },
  { keys: '↑  ↓', i18n: 'help.nudge', modes: ['scroll'] },
  { keys: '←  →', i18n: 'help.page', modes: ['both'] },
  { keys: 'Space  ·  PgDn / PgUp', i18n: 'help.flipPage', modes: ['flip'] },
  { keys: 'L / R edge click', i18n: 'help.flipTap', modes: ['flip'] },
  { keys: 'Home  ·  End', i18n: 'help.ends', modes: ['scroll'] },
  { keys: 'Ctrl  +  −  0', i18n: 'help.zoom', modes: ['scroll'] },
  { keys: 'B', i18n: 'help.bookmark', modes: ['both'] },
  { keys: 'M', i18n: 'help.mode', modes: ['both'] },
  { keys: 'A', i18n: 'help.auto', modes: ['both'] },
  { keys: 'D', i18n: 'help.dir', modes: ['scroll'] },
  { keys: 'C', i18n: 'help.history', modes: ['both'] },
  { keys: 'T', i18n: 'help.chrome', modes: ['both'] },
  { keys: 'G', i18n: 'help.jump', modes: ['both'] },
  { keys: 'F  ·  F11', i18n: 'help.full', modes: ['both'] },
  { keys: '[  ·  ]', i18n: 'help.episode', modes: ['both'] },
  { keys: 'Esc', i18n: 'help.leave', modes: ['both'] },
  { keys: '?  ·  H', i18n: 'help.toggle', modes: ['both'] },
];

/**
 * @param {'scroll'|'flip'} mode
 * @returns {{ keys: string, i18n: string }[]}
 */
export function shortcutHelpRows(mode = 'scroll') {
  const m = mode === 'flip' ? 'flip' : 'scroll';
  return ROWS.filter((r) => {
    const modes = r.modes || ['both'];
    return modes.includes('both') || modes.includes(m);
  }).map(({ keys, i18n }) => ({ keys, i18n }));
}
