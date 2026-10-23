import { test } from 'node:test';
import assert from 'node:assert/strict';
import { playlistSongs, moveSelected, moveBefore, draggedIds, playlistLength } from '../../src/js/playlists.js';

test('a playlist shows its songs in its own order, skipping songs no longer in the library', () => {
  const byId = new Map([[1, { id: 1 }], [2, { id: 2 }], [3, { id: 3 }]]);
  assert.deepEqual(playlistSongs([3, 9, 1], byId).map((s) => s.id), [3, 1]);
  assert.deepEqual(playlistSongs([], byId), []);
});

test('selected songs move up or down together, stopping at the ends', () => {
  const ids = [1, 2, 3, 4, 5];
  assert.deepEqual(moveSelected(ids, new Set([3]), -1), [1, 3, 2, 4, 5]);
  assert.deepEqual(moveSelected(ids, new Set([2, 4]), 1), [1, 3, 2, 5, 4]);
  assert.deepEqual(moveSelected(ids, new Set([1, 2]), -1), ids, 'already at the top');
  assert.deepEqual(moveSelected(ids, new Set([5]), 1), ids, 'already at the bottom');
  assert.deepEqual(moveSelected(ids, new Set(), 1), ids);
});

test('dragged songs land before the song they are dropped on, or at the end', () => {
  const ids = [1, 2, 3, 4, 5];
  assert.deepEqual(moveBefore(ids, [4, 5], 2), [1, 4, 5, 2, 3]);
  assert.deepEqual(moveBefore(ids, [1], null), [2, 3, 4, 5, 1]);
  assert.deepEqual(moveBefore(ids, [2, 3], 3), ids, 'dropped on itself: nothing moves');
});

test('dragging a selected song drags the whole selection, in list order', () => {
  const list = [10, 20, 30, 40];
  assert.deepEqual(draggedIds(list, new Set([40, 20]), 20), [20, 40]);
  assert.deepEqual(draggedIds(list, new Set([40, 20]), 30), [30], 'an unselected song goes alone');
});

test('how long a playlist plays, in minutes or hours and minutes', () => {
  assert.deepEqual(playlistLength([{ durationMs: 200_000 }, { durationMs: 100_000 }, {}]), { h: 0, min: 5 });
  assert.deepEqual(playlistLength([{ durationMs: 4_380_000 }]), { h: 1, min: 13 });
  assert.deepEqual(playlistLength([]), { h: 0, min: 0 });
});
