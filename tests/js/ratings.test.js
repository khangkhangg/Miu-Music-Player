import { test } from 'node:test';
import assert from 'node:assert/strict';
import { nextRating, filterSongs, ratingCounts, filterChips } from '../../src/js/ratings.js';

test('like toggles between liked and not rated', () => {
  assert.equal(nextRating(0, 'like'), 1);
  assert.equal(nextRating(1, 'like'), 0);
  assert.equal(nextRating(-1, 'like'), 1, 'like replaces a dislike');
});

test('dislike toggles between disliked and not rated', () => {
  assert.equal(nextRating(0, 'dislike'), -1);
  assert.equal(nextRating(-1, 'dislike'), 0);
  assert.equal(nextRating(1, 'dislike'), -1, 'dislike replaces a like');
});

const list = [
  { id: 1, rating: 1 }, { id: 2, rating: 0 }, { id: 3, rating: -1 }, { id: 4, rating: 1 }, { id: 5, rating: 0 },
];

test('filters pick the right songs', () => {
  assert.deepEqual(filterSongs(list, 'all').map(s => s.id), [1, 2, 3, 4, 5]);
  assert.deepEqual(filterSongs(list, 'liked').map(s => s.id), [1, 4]);
  assert.deepEqual(filterSongs(list, 'not-rated').map(s => s.id), [2, 5]);
  assert.deepEqual(filterSongs(list, 'disliked').map(s => s.id), [3]);
});

test('counts per rating', () => {
  assert.deepEqual(ratingCounts(list), { all: 5, liked: 2, 'not-rated': 2, disliked: 1 });
  assert.deepEqual(ratingCounts([]), { all: 0, liked: 0, 'not-rated': 0, disliked: 0 });
});

test('chips show labels with counts and mark the active one', () => {
  const chips = filterChips(list, 'liked');
  assert.deepEqual(chips.map(c => c.label), ['All 5', 'Liked 2', 'Not rated 2', 'Disliked 1']);
  assert.deepEqual(chips.map(c => c.active), [false, true, false, false]);
  assert.deepEqual(chips.map(c => c.id), ['all', 'liked', 'not-rated', 'disliked']);
});

import { applyRated } from '../../src/js/ratings.js';

test('a rating announced by the other window updates this window\x27s copy', () => {
  const byId = new Map([[1, { id: 1, rating: 0 }]]);
  assert.equal(applyRated(byId, { id: 1, rating: 1 }), true);
  assert.equal(byId.get(1).rating, 1);
});

test('an unchanged or unknown song is ignored', () => {
  const byId = new Map([[1, { id: 1, rating: 1 }]]);
  assert.equal(applyRated(byId, { id: 1, rating: 1 }), false);
  assert.equal(applyRated(byId, { id: 9, rating: -1 }), false);
});

const withCopies = [
  { id: 1, rating: 0, duplicateGroup: 4 }, { id: 2, rating: 1, duplicateGroup: null },
  { id: 3, rating: 0, duplicateGroup: 3 }, { id: 4, rating: 1, duplicateGroup: 4 }, { id: 5, rating: 0, duplicateGroup: 3 },
];

test('the duplicates filter keeps copies next to each other', () => {
  assert.deepEqual(filterSongs(withCopies, 'duplicates').map(s => s.id), [3, 5, 1, 4]);
});

test('a duplicates chip appears only when there are copies, or while it is open', () => {
  assert.equal(filterChips(withCopies, 'all').at(-1).label, 'Duplicates 4');
  assert.equal(filterChips(list, 'all').some(c => c.id === 'duplicates'), false);
  assert.deepEqual(filterChips(list, 'duplicates').at(-1), { id: 'duplicates', label: 'Duplicates 0', active: true });
});
