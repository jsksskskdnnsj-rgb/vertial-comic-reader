// Page-scoped listeners for the shell. Navigating between shell pages re-runs the page
// module; without this, document/window listeners from the previous visit stay attached
// and every switch gets slower.

let current = null;

/** Abort the previous page's listeners and start a fresh scope. */
export function beginPage() {
  current?.abort();
  current = new AbortController();
  return current.signal;
}

export function pageSignal() {
  return current?.signal ?? null;
}

/** addEventListener that dies on the next beginPage(). */
export function onPage(target, type, fn, options) {
  if (!target?.addEventListener) return;
  const opts = { ...(options || {}) };
  const signal = current?.signal;
  if (signal) opts.signal = signal;
  target.addEventListener(type, fn, opts);
}
