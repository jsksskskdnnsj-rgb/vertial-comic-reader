// folder-scan.js - recursive folder inspection for the import UI. Pure (no imports, no DOM, no Tauri) so it can
// be unit-tested in Node. It summarises what the EXISTING `list_images(path)` command returns: that command
// (main.rs walk_dir) already walks every nested subfolder and reports each file with its path relative to the
// picked folder ('/'-separated), so no backend change is needed for the preview.
// The image extensions are exactly main.rs IMAGE_EXTS; archives are counted separately.
export const IMAGE_RE = /\.(jpe?g|jfif|jpe|png|webp|gif|bmp|avif)$/i;
export const ARCHIVE_RE = /\.(zip|cbz)$/i;
/** A picked path that is a single file (image or archive) by extension; everything else is treated as a folder. */
export const looksLikeFile = (p) => IMAGE_RE.test(String(p)) || ARCHIVE_RE.test(String(p));

/** files: [{ name: 'sub/dir/001.jpg', path, size }] -> counts. `subfolders` = folders that directly hold images. */
export function summarizeFolder(files) {
  let images = 0, archives = 0, top = 0, bytes = 0, depth = 0;
  const dirs = new Map();
  for (const f of files || []) {
    const rel = String(f?.name ?? '').replace(/\\/g, '/');
    if (ARCHIVE_RE.test(rel)) { archives++; continue; }
    if (!IMAGE_RE.test(rel)) continue;
    images++; bytes += Number(f.size) || 0;
    const i = rel.lastIndexOf('/');
    if (i < 0) top++;
    else { const d = rel.slice(0, i); dirs.set(d, (dirs.get(d) || 0) + 1); depth = Math.max(depth, d.split('/').length); }
  }
  return { images, archives, top, nested: images - top, bytes, depth, subfolders: dirs.size, subfolderNames: [...dirs.keys()], empty: images === 0 };
}

/** listImages(path) -> Promise<files>. Never throws: { ok:false } when the path cannot be read as a folder. */
export async function inspectFolder(path, listImages) {
  try { return { ok: true, ...summarizeFolder(await listImages(path)) }; }
  catch (e) { return { ok: false, error: String(e?.detail ?? e?.message ?? e) }; }
}
