// Short-lived invoke cache shared by every shell page module.
// `library.js?shell=3` is a fresh module; this file is not, so a tab switch
// can reuse the shelf list from a moment ago instead of asking SQLite again.

const slots = new Map();

export function markDirty(key) {
  if (key == null) slots.clear();
  else slots.delete(key);
}

export async function cachedInvoke(key, load, fresh = false, ttl = 8000) {
  const now = Date.now();
  const hit = slots.get(key);
  if (!fresh && hit && now - hit.at < ttl) return hit.value;
  const value = await load();
  slots.set(key, { at: Date.now(), value });
  return value;
}
