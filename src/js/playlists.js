// Playlists (PL): the user's own song lists in their own order. Pure helpers here; the views use
// them in app.js.

/** The playlist's songs in order; songs no longer in the library are left out. */
export function playlistSongs(ids, songsById) {
  return ids.map((id) => songsById.get(id)).filter(Boolean);
}

/** The order with the selected songs moved one place up (-1) or down (+1), together. */
export function moveSelected(ids, selected, by) {
  const out = [...ids];
  if (!selected.size) return out;
  const idx = out.map((id, i) => (selected.has(id) ? i : -1)).filter((i) => i >= 0);
  if (by < 0 ? idx[0] === 0 : idx[idx.length - 1] === out.length - 1) return out;
  const walk = by < 0 ? idx : [...idx].reverse();
  for (const i of walk) [out[i], out[i + by]] = [out[i + by], out[i]];
  return out;
}

/** The order with `moving` placed just before `target` (null: at the end). */
export function moveBefore(ids, moving, target) {
  if (target != null && moving.includes(target)) return [...ids];
  const set = new Set(moving);
  const rest = ids.filter((id) => !set.has(id));
  const at = target == null ? rest.length : rest.indexOf(target);
  rest.splice(at < 0 ? rest.length : at, 0, ...moving);
  return rest;
}

/** What a drag carries: the whole selection when the grabbed song is in it, else just that song. */
export function draggedIds(listIds, selected, grabbed) {
  return selected.has(grabbed) ? listIds.filter((id) => selected.has(id)) : [grabbed];
}

/** How long the songs play together, rounded to the minute: { h, min }. */
export function playlistLength(songs) {
  const total = Math.round(songs.reduce((sum, s) => sum + (s.durationMs ?? 0), 0) / 60_000);
  return { h: Math.floor(total / 60), min: total % 60 };
}
