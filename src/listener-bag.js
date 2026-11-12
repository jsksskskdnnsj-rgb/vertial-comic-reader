// One bag of DOM listeners. clear() removes every one it added, so a page
// change or a second open cannot leave the previous handlers stacked.

export function createListenerBag() {
  const items = [];
  return {
    add(target, type, fn, options) {
      if (!target?.addEventListener) return;
      target.addEventListener(type, fn, options);
      const capture = typeof options === 'boolean' ? options : !!options?.capture;
      items.push({ target, type, fn, capture });
    },
    clear() {
      while (items.length) {
        const it = items.pop();
        it.target.removeEventListener?.(it.type, it.fn, it.capture);
      }
    },
    get size() {
      return items.length;
    },
  };
}
