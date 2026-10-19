// Ratings (F7.2): 1 liked, 0 not rated, -1 disliked. Strings move to src/i18n in F8.6.

export function nextRating(current, action) {
  const target = action === 'like' ? 1 : -1;
  return current === target ? 0 : target;
}

const FILTERS = [
  { id: 'all', name: 'All', test: () => true },
  { id: 'liked', name: 'Liked', test: s => s.rating === 1 },
  { id: 'not-rated', name: 'Not rated', test: s => s.rating === 0 },
  { id: 'disliked', name: 'Disliked', test: s => s.rating === -1 },
];

/** Songs with the same audio as another (F3.6), each group together, in group order. */
function duplicates(list) {
  return list.filter(s => s.duplicateGroup != null).sort((a, b) => a.duplicateGroup - b.duplicateGroup);
}

export function filterSongs(list, filter) {
  if (filter === 'duplicates') return duplicates(list);
  const f = FILTERS.find(x => x.id === filter) ?? FILTERS[0];
  return list.filter(f.test);
}

export function ratingCounts(list) {
  return Object.fromEntries(FILTERS.map(f => [f.id, list.filter(f.test).length]));
}

export function filterChips(list, active) {
  const counts = ratingCounts(list);
  const chips = FILTERS.map(f => ({ id: f.id, label: `${f.name} ${counts[f.id]}`, active: f.id === active }));
  const copies = duplicates(list).length;
  if (copies > 0 || active === 'duplicates') {
    chips.push({ id: 'duplicates', label: `Duplicates ${copies}`, active: active === 'duplicates' });
  }
  return chips;
}

/** Apply a `song-rated` event to this window's songs; returns whether anything changed. */
export function applyRated(songsById, { id, rating }) {
  const song = songsById.get(id);
  if (!song || song.rating === rating) return false;
  song.rating = rating;
  return true;
}
