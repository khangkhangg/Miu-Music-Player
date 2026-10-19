import { test } from 'node:test';
import assert from 'node:assert/strict';
import { formatDuration, songRow, visibleRange } from '../../src/js/songs.js';

test('durations read like a player shows them', () => {
  assert.equal(formatDuration(0), '0:00');
  assert.equal(formatDuration(252_000), '4:12');
  assert.equal(formatDuration(59_999), '0:59');
  assert.equal(formatDuration(3_725_000), '1:02:05');
  assert.equal(formatDuration(null), '');
  assert.equal(formatDuration(undefined), '');
});

test('a row keeps Vietnamese text and names missing artists', () => {
  assert.deepEqual(
    songRow({ id: 1, title: 'Diễm xưa', artist: 'Khánh Ly', album: 'Trịnh Công Sơn', durationMs: 303_000 }),
    { id: 1, title: 'Diễm xưa', artist: 'Khánh Ly', album: 'Trịnh Công Sơn', duration: '5:03' },
  );
  const r = songRow({ id: 2, title: 'x', artist: null, album: null, durationMs: null });
  assert.equal(r.artist, 'Unknown artist');
  assert.equal(r.album, '');
});

test('visible range at the top includes overscan below only', () => {
  assert.deepEqual(visibleRange({ scrollTop: 0, height: 400, rowHeight: 36, total: 1000, overscan: 5 }), { start: 0, end: 17 });
});

test('visible range in the middle has overscan on both sides', () => {
  assert.deepEqual(visibleRange({ scrollTop: 3600, height: 400, rowHeight: 36, total: 1000, overscan: 5 }), { start: 95, end: 117 });
});

test('visible range never goes past the end or below zero', () => {
  assert.deepEqual(visibleRange({ scrollTop: 36_000, height: 400, rowHeight: 36, total: 1000, overscan: 5 }), { start: 995, end: 1000 });
  assert.deepEqual(visibleRange({ scrollTop: 0, height: 400, rowHeight: 36, total: 3, overscan: 5 }), { start: 0, end: 3 });
  assert.deepEqual(visibleRange({ scrollTop: 0, height: 400, rowHeight: 36, total: 0, overscan: 5 }), { start: 0, end: 0 });
});
