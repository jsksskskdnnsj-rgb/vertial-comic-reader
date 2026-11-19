// Rectangle for the official-site child webview, in viewport pixels.
// Null means the webview should be hidden (a dialog is covering the page, or the area collapsed).
// The site document is never measured here.

export function cosersViewRect(host, { rail, top, scanFloat, modal } = {}) {
  if (!host || modal) return null;
  let left = host.left;
  let topY = host.top;
  let right = host.right;
  let bottom = host.bottom;
  if (rail && left < rail.right) left = rail.right;
  if (top && topY < top.bottom) topY = top.bottom;
  if (scanFloat && scanFloat.visible !== false) {
    const overlaps = scanFloat.top < bottom && scanFloat.bottom > topY && scanFloat.left < right && scanFloat.right > left;
    if (overlaps) bottom = Math.min(bottom, scanFloat.top - 6);
  }
  left = Math.max(0, left);
  topY = Math.max(0, topY);
  const width = right - left;
  const height = bottom - topY;
  if (!(width >= 2) || !(height >= 2)) return null;
  return { x: left, y: topY, width, height };
}
