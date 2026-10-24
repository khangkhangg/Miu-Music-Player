// Ratings (F7.2): 1 liked, 0 not rated, -1 disliked.
import { t } from './i18n.js';

export function nextRating(current, action) {
  const target = action === 'like' ? 1 : -1;
  return current === target ? 0 : target;
}

const FILTERS = [
  { id: 'all', name: 'filters.all', test: () => true },
  { id: 'liked', name: 'filters.liked', test: s => s.rating === 1 },
  { id: 'not-rated', name: 'filters.notRated', test: s => s.rating === 0 },
  { id: 'disliked', name: 'filters.disliked', test: s => s.rating === -1 },
];

/** Songs with the same audio as another (F3.6), each group together, in group order. */
function duplicates(list) {
  return list.filter(s => s.duplicateGroup != null).sort((a, b) => a.duplicateGroup - b.duplicateGroup);
}

const LOSSLESS = new Set(['flac', 'alac', 'wav', 'aiff', 'ape', 'wavpack']);

/** Which copy is better to keep: liked over not rated over disliked, then lossless, then the best sound, then artwork. */
function better(x, y) {
  const score = (s) => [s.rating ?? 0, LOSSLESS.has(s.format) ? 1 : 0, s.bitDepth ?? 0, s.sampleRate ?? 0, s.bitrateKbps ?? 0, s.hasArtwork ? 1 : 0];
  const a = score(x);
  const b = score(y);
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return a[i] > b[i];
  return false; // a tie keeps the one listed first
}

/** The ids of every copy but the best one of each song with duplicates, to select and delete. */
export function extraCopies(list) {
  const keep = new Map();
  for (const s of list) {
    if (s.duplicateGroup == null) continue;
    const k = keep.get(s.duplicateGroup);
    if (!k || better(s, k)) keep.set(s.duplicateGroup, s);
  }
  return list.filter((s) => s.duplicateGroup != null && keep.get(s.duplicateGroup) !== s).map((s) => s.id);
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
  const chips = FILTERS.map(f => ({ id: f.id, label: `${t(f.name)} ${counts[f.id]}`, active: f.id === active }));
  const copies = duplicates(list).length;
  if (copies > 0 || active === 'duplicates') {
    chips.push({ id: 'duplicates', label: `${t('filters.duplicates')} ${copies}`, active: active === 'duplicates' });
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
