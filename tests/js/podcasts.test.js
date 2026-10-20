import { test } from 'node:test';
import assert from 'node:assert/strict';
import { episodeMeta, lengthLabel } from '../../src/js/podcasts.js';

test('lengths read as hours and minutes', () => {
  assert.equal(lengthLabel(3_723_000), '1 h 2 min');
  assert.equal(lengthLabel(2_730_000), '46 min');
  assert.equal(lengthLabel(40_000), '1 min', 'never 0');
  assert.equal(lengthLabel(null), '');
});

test('an episode line shows its date and length', () => {
  const e = { published: 1_792_371_600, durationMs: 3_723_000 };
  assert.equal(episodeMeta(e, 'en', 'UTC'), 'Oct 19, 2026 · 1 h 2 min');
  assert.equal(episodeMeta({ published: null, durationMs: 2_730_000 }, 'en', 'UTC'), '46 min');
  assert.equal(episodeMeta(e, 'vi', 'UTC'), '19 thg 10, 2026 · 1 h 2 min');
});

import { episodeState } from '../../src/js/podcasts.js';

test('an episode is downloadable, downloading, or on this PC', () => {
  assert.equal(episodeState({ id: 1, downloadPath: null }, new Set()), 'download');
  assert.equal(episodeState({ id: 1, downloadPath: null }, new Set([1])), 'downloading');
  assert.equal(episodeState({ id: 1, downloadPath: 'C:/p/1.mp3' }, new Set([1])), 'downloaded');
});

import { markSubscribed } from '../../src/js/podcasts.js';

test('discover results you already follow are marked, by feed or by name', () => {
  const found = [
    { appleId: 1, title: 'The Daily', feedUrl: null },
    { appleId: 2, title: 'Tám Sài Gòn', feedUrl: 'https://anchor.fm/s/1/podcast/rss' },
    { appleId: 3, title: 'Something new', feedUrl: null },
  ];
  const shows = [{ title: 'the daily', feedUrl: 'https://feeds.simplecast.com/x' }, { title: 'Other', feedUrl: 'https://anchor.fm/s/1/podcast/rss' }];
  assert.deepEqual(markSubscribed(found, shows).map((f) => f.subscribed), [true, true, false]);
});
