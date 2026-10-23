// book-actions.js - per-book actions shared by the library menu and the book detail page.
// Each returns true when something changed (caller reloads), false when cancelled or failed
// (an error toast has already been shown).
import { T, t, toast, confirmDialog, promptDialog } from './shell.js';
import { lib } from './legacy-api.js';

const errText = (e) => String(e?.detail ?? e?.message ?? e);
async function run(fn, okMsg) {
  try { await fn(); if (okMsg) toast(okMsg); return true; }
  catch (e) { toast(errText(e)); return false; }
}

export async function renameBook(b) {
  const name = await promptDialog({ title: t('lib.renameTitle'), value: b.name, placeholder: b.name, ok: t('common.save'), cancel: t('common.cancel') });
  if (typeof name !== 'string' || !name.trim() || name.trim() === b.name) return false;
  return run(() => lib.rename(b.id, name.trim()), t('common.saved'));
}
export async function startOver(b) {
  if (!await confirmDialog({ title: t('lib.startOver'), message: t('lib.startOverMsg', { title: b.name }), ok: t('lib.startOver') })) return false;
  return run(() => lib.startOver(b.id), t('book.progressReset'));
}
export async function resetEdits(b) {
  if (!await confirmDialog({ title: t('lib.resetEdits'), message: t('lib.resetEditsMsg', { title: b.name }), ok: t('common.ok') })) return false;
  return run(() => lib.resetPageEdits(b.id), t('common.saved'));
}
export async function removeBook(b) {
  if (!await confirmDialog({ title: t('lib.remove'), message: t('lib.removeMsg', { title: b.name }), ok: t('common.delete'), danger: true })) return false;
  return run(() => lib.remove(b.id), t('common.saved'));
}
export async function relinkBook(b) {
  const p = await T.dialog?.open({ multiple: false, directory: b.kind === 'folder' }).catch(() => null);
  if (!p) return false;
  return run(() => lib.relinkBook(b.id, p), t('common.saved'));
}
export async function moveBook(b, folderId) {
  return run(() => lib.moveTo(b.id, folderId), t('common.saved'));
}
export const revealBook = (b) => lib.revealBook(b.id).catch(() => {});
