// reader-fit.js — page fit modes (OpenComic / Yomikiru scaling ideas, reimplemented; pure).
export const FIT_MODES = ['width', 'height', 'original', 'custom'];

/**
 * @param {'width'|'height'|'original'|'custom'} mode
 * @param {{ setWidthPct?: Function, setZoom?: Function, setFit?: Function, params?: object }} view
 */
export function applyReaderFit(mode, view) {
  if (!view) return;
  const m = FIT_MODES.includes(mode) ? mode : 'width';
  if (m === 'original') {
    view.setZoom?.(1);
    view.setWidthPct?.(100);
    return;
  }
  if (m === 'width') {
    view.setZoom?.(1);
    view.setWidthPct?.(100);
    view.setFit?.();
    return;
  }
  if (m === 'height') {
    view.setZoom?.(1);
    view.setWidthPct?.(88);
    return;
  }
  // custom: keep current zoom/width
}

export function fitModeLabelKey(mode) {
  if (mode === 'height') return 'reader.fitHeight';
  if (mode === 'original') return 'reader.fitOriginal';
  if (mode === 'custom') return 'reader.fitCustom';
  return 'reader.fitWidth';
}
