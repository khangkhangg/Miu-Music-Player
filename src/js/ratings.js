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

export function filterSongs(list, filter) {
  const f = FILTERS.find(x => x.id === filter) ?? FILTERS[0];
  return list.filter(f.test);
}

export function ratingCounts(list) {
  return Object.fromEntries(FILTERS.map(f => [f.id, list.filter(f.test).length]));
}

export function filterChips(list, active) {
  const counts = ratingCounts(list);
  return FILTERS.map(f => ({ id: f.id, label: `${f.name} ${counts[f.id]}`, active: f.id === active }));
}
