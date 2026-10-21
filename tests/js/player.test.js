import { test } from 'node:test';
import assert from 'node:assert/strict';
import { nowPlaying, seekTarget } from '../../src/js/player.js';

const songs = new Map([[7, { id: 7, title: 'Mùa thu cho em', artist: 'Ngọc Lan', durationMs: 252_000 }]]);

test('nothing playing gives no bar content', () => {
  assert.equal(nowPlaying({ available: true, playing: false, songId: null, positionMs: 0 }, songs), null);
  assert.equal(nowPlaying(undefined, songs), null);
});

test('shows the current song with elapsed and total time', () => {
  const n = nowPlaying({ available: true, playing: true, songId: 7, positionMs: 84_000 }, songs);
  assert.deepEqual(n, {
    title: 'Mùa thu cho em', artist: 'Ngọc Lan', elapsed: '1:24', duration: '4:12', progress: 84 / 252, playing: true,
  });
});

test('progress is clamped and zero when the length is unknown', () => {
  const long = nowPlaying({ playing: true, songId: 7, positionMs: 999_000 }, songs);
  assert.equal(long.progress, 1);
  const unknown = nowPlaying({ playing: true, songId: 8, positionMs: 5_000 }, new Map([[8, { id: 8, title: 'x', artist: null, durationMs: null }]]));
  assert.equal(unknown.progress, 0);
  assert.equal(unknown.artist, 'Unknown artist');
  assert.equal(unknown.duration, '');
});

test('a click on the progress bar becomes a seek position', () => {
  assert.equal(seekTarget(0.5, 252_000), 126_000);
  assert.equal(seekTarget(-0.2, 252_000), 0);
  assert.equal(seekTarget(1.3, 252_000), 252_000);
  assert.equal(seekTarget(0.5, null), null);
});

import { nextRepeat, repeatButton } from '../../src/js/player.js';

test('the repeat button cycles off → all → one → off', () => {
  assert.deepEqual(['off', 'all', 'one'].map(nextRepeat), ['all', 'one', 'off']);
  assert.equal(nextRepeat(undefined), 'all');
});

test('the repeat button says what it does now', () => {
  assert.deepEqual(repeatButton('off'), { pressed: false, one: false, label: 'Repeat: off' });
  assert.deepEqual(repeatButton('all'), { pressed: true, one: false, label: 'Repeat: all songs' });
  assert.deepEqual(repeatButton('one'), { pressed: true, one: true, label: 'Repeat: this song' });
});

import { skipTarget } from '../../src/js/player.js';

test('skip back 15 s and forward 30 s stay inside the episode', () => {
  assert.equal(skipTarget(60_000, -15_000, 3_600_000), 45_000);
  assert.equal(skipTarget(5_000, -15_000, 3_600_000), 0);
  assert.equal(skipTarget(3_590_000, 30_000, 3_600_000), 3_599_000, 'not past the end');
  assert.equal(skipTarget(10_000, 30_000, null), 40_000, 'length unknown');
});

import { playButtonAction } from '../../src/js/player.js';

test('the play button pauses or resumes whatever is loaded, a podcast episode too', () => {
  assert.equal(playButtonAction({ songId: 4, playing: true }, 10), 'pause');
  assert.equal(playButtonAction({ songId: 4, playing: false }, 10), 'resume');
  assert.equal(playButtonAction({ songId: null, episodeId: 7, playing: true }, 10), 'pause');
  assert.equal(playButtonAction({ songId: null, episodeId: 7, playing: false }, 10), 'resume');
});

test('with nothing loaded, the play button plays the list on screen', () => {
  assert.equal(playButtonAction(null, 10), 'playList');
  assert.equal(playButtonAction({ songId: null, episodeId: null, playing: false }, 10), 'playList');
  assert.equal(playButtonAction(null, 0), 'none');
});

import { startIndex } from '../../src/js/player.js';

test('playing a whole list starts at the top, or at a random song when shuffle is on', () => {
  assert.equal(startIndex(10, false, () => 0.99), 0);
  assert.equal(startIndex(10, true, () => 0.55), 5);
  assert.equal(startIndex(10, true, () => 0.999), 9, 'never past the end');
  assert.equal(startIndex(0, true, () => 0.5), 0);
});
