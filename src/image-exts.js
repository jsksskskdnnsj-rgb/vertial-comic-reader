// Formats the reader will import. Keep this list identical to IMAGE_EXTS in
// src-tauri/src/main.rs. svg / tif / heic / jxl are intentionally absent:
// the pipeline cannot decode them.
//
// avif is included because extractor::sniff serves those bytes unchanged and
// the webview can paint them. The Rust `image` crate is built without an AVIF
// decoder, so covers and resized/sliced AVIF pages are not produced.
export const IMAGE_EXTS = ['jpg', 'jpeg', 'jfif', 'jpe', 'png', 'webp', 'gif', 'bmp', 'avif'];
export const ZIP_EXTS = ['zip', 'cbz'];

export function archiveAndImageFilters(t) {
  return [
    { name: t('imp.zipFilter'), extensions: ZIP_EXTS },
    { name: t('imp.imageFilter'), extensions: IMAGE_EXTS },
  ];
}

export function imageFilters(t) {
  return [{ name: t('settings.imageFilter'), extensions: IMAGE_EXTS }];
}
