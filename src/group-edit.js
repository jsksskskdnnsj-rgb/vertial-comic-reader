// Pure helpers for the group editor. The dialog in group-ui.js calls these, then the Rust commands.

export function moveIndex(list, from, to) {
  const next = Array.isArray(list) ? list.slice() : [];
  if (from < 0 || to < 0 || from >= next.length || to >= next.length || from === to) return next;
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item);
  return next;
}

export function partitionParts(parts, keys) {
  const drop = new Set(keys || []);
  const stay = [];
  const gone = [];
  for (const part of parts || []) {
    const hit = drop.has(part?.key) || (part?.paths || []).some((p) => drop.has(p));
    (hit ? gone : stay).push(part);
  }
  return { stay, gone };
}

export function removePlan(parts, keys) {
  const { stay, gone } = partitionParts(parts, keys);
  if (!gone.length) return { ok: false, reason: 'none' };
  if (!stay.length) return { ok: false, reason: 'needPart' };
  return { ok: true, stay, gone };
}

export function splitPlan(parts, keys) {
  const { stay, gone } = partitionParts(parts, keys);
  if (!gone.length || !stay.length) return { ok: false, reason: 'needPart' };
  return {
    ok: true,
    groups: [gone.map((p) => p.key), stay.map((p) => p.key)],
  };
}

/** Members whose checkbox was cleared. Those paths are split out of the review group. */
export function pathsToSplit(members) {
  return (members || []).filter((m) => m && m.keep === false && m.path).map((m) => m.path);
}

export function eachAlone(parts) {
  if (!parts || parts.length < 2) return { ok: false, reason: 'needPart' };
  return { ok: true, groups: parts.map((p) => [p.key]) };
}
