import { test } from 'node:test';
import assert from 'node:assert/strict';
import { lineAt, livePosition, lyricsNote } from '../../src/js/lyrics.js';

const lines = [{ ms: 1000, text: 'a' }, { ms: 5000, text: 'b' }, { ms: 9000, text: 'c' }];

test('the current line is the last one that has started', () => {
  assert.equal(lineAt(lines, 0), -1);
  assert.equal(lineAt(lines, 1000), 0);
  assert.equal(lineAt(lines, 8999), 1);
  assert.equal(lineAt(lines, 120000), 2);
  assert.equal(lineAt([], 5), -1);
});

test('the position moves on between player events while playing', () => {
  assert.equal(livePosition({ positionMs: 4000, playing: true }, 10_000, 10_600), 4600);
  assert.equal(livePosition({ positionMs: 4000, playing: false }, 10_000, 10_600), 4000, 'paused');
  assert.equal(livePosition({ positionMs: 4000, playing: true }, 10_000, 9_000), 4000, 'clock earlier than the event');
  assert.equal(livePosition(null, 0, 1), 0);
});

test('a note says where the lyrics came from or why there are none', () => {
  assert.equal(lyricsNote(null), 'Looking for lyrics…');
  assert.equal(lyricsNote({ synced: [], plain: null, instrumental: false, source: 'none' }), 'No lyrics found for this song.');
  assert.equal(lyricsNote({ synced: [], plain: null, instrumental: true, source: 'online' }), 'Instrumental');
  assert.equal(lyricsNote({ synced: lines, plain: 'a', instrumental: false, source: 'online' }), 'Lyrics from LRCLIB');
  assert.equal(lyricsNote({ synced: [], plain: 'a', instrumental: false, source: 'tags' }), 'Lyrics from the song file · not timed');
  assert.equal(lyricsNote({ synced: lines, plain: 'a', instrumental: false, source: 'file' }), 'Lyrics from the .lrc file');
});

test('an unreachable lyrics service is not reported as no lyrics', () => {
  assert.equal(lyricsNote({ synced: [], plain: null, instrumental: false, source: 'unreachable' }), 'Could not reach the lyrics service. Try again later.');
});
