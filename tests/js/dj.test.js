import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { useDictionary } from '../../src/js/i18n.js';
import {
  bpmLabel, tempoLabel, timeLeft, knobDrag, knobKey, meterHeight, waveformPath, keysCompatible, fits,
  songMeta, nextText, crossfaderFor, holdProgress, filterSongsForDj, analysisText, jogTurns,
} from '../../src/js/dj.js';

const en = JSON.parse(fs.readFileSync(path.resolve(import.meta.dirname, '../../src/i18n/en.json'), 'utf8'));
useDictionary('en', en);

test('tempo readouts', () => {
  assert.equal(bpmLabel(124), '124.0');
  assert.equal(bpmLabel(127.456), '127.5');
  assert.equal(bpmLabel(null), '--');
  assert.equal(tempoLabel(0), '0.0 %');
  assert.equal(tempoLabel(0.5), '+4.0 %');
  assert.equal(tempoLabel(-1), '-8.0 %');
});

test('time left counts down', () => {
  assert.equal(timeLeft({ positionS: 10, durationS: 200 }), '-3:10');
  assert.equal(timeLeft({ positionS: 199.5, durationS: 200 }), '-0:00');
  assert.equal(timeLeft({ positionS: 0, durationS: 0 }), '');
});

test('knobs turn by dragging up and down and by keys', () => {
  assert.equal(knobDrag(0, 75), 1, 'half the travel up from the middle reaches the top');
  assert.equal(knobDrag(0, -300), -1, 'stops at the bottom');
  assert.ok(Math.abs(knobDrag(0, 15) - 0.2) < 1e-9);
  assert.equal(knobDrag(0.5, 30, { min: 0, max: 1 }), 0.7);
  assert.ok(Math.abs(knobKey(0, 'ArrowUp') - 0.05) < 1e-9);
  assert.ok(Math.abs(knobKey(0, 'ArrowDown') + 0.05) < 1e-9);
  assert.equal(knobKey(0.7, 'Home', { reset: 0 }), 0, 'Home sets it back to the middle');
  assert.equal(knobKey(0.3, 'x'), null, 'other keys do nothing');
});

test('meters read in decibels', () => {
  assert.equal(meterHeight(1), 1);
  assert.equal(meterHeight(0), 0);
  assert.ok(Math.abs(meterHeight(0.5) - (1 - 6.02 / 48)) < 0.01, '-6 dB');
  assert.equal(meterHeight(2), 1, 'over full scale stays at the top');
});

test('the waveform is a mirrored outline across the width', () => {
  assert.equal(waveformPath([], 100, 20), '');
  assert.equal(waveformPath([0, 255], 100, 20), 'M0,10L100,0L100,20L0,10Z');
  assert.equal(waveformPath([0, 255, 0], 100, 20), 'M0,10L50,0L100,10L100,10L50,20L0,10Z');
});

test('Camelot keys that mix well', () => {
  assert.ok(keysCompatible('8A', '8A'));
  assert.ok(keysCompatible('8A', '9A'));
  assert.ok(keysCompatible('12B', '1B'), 'the wheel wraps');
  assert.ok(keysCompatible('8A', '8B'), 'relative major and minor');
  assert.ok(!keysCompatible('8A', '10A'));
  assert.ok(!keysCompatible('8A', '9B'));
  assert.ok(!keysCompatible(null, '8A'));
});

test('a song fits the one on air by key and tempo (half and double time too)', () => {
  const air = { bpm: 124, key: '8A' };
  assert.ok(fits(air, { bpm: 128, key: '9A' }));
  assert.ok(fits(air, { bpm: 63, key: '8B' }));
  assert.ok(!fits(air, { bpm: 140, key: '8A' }), 'too far in tempo');
  assert.ok(!fits(air, { bpm: 124, key: '3B' }), 'key clash');
  assert.ok(!fits(null, { bpm: 124, key: '8A' }));
});

test('song details for the list and decks', () => {
  assert.equal(songMeta({ bpm: 123.7, key: '8A', energy: 7 }), '124 BPM · 8A · Energy 7');
  assert.equal(songMeta({ bpm: 90, key: null, energy: 3 }), '90 BPM · Energy 3');
  assert.equal(songMeta(null), '');
});

test('the coming transition is described', () => {
  assert.equal(nextText({ transition: { type: 'beatBlend', bars: 16 }, startsInS: 42.4 }), 'Beat blend (16 bars) in 0:42');
  assert.equal(nextText({ transition: { type: 'crossfade', secs: 8 }, startsInS: 0 }), 'Crossfade now');
  assert.equal(nextText({ transition: { type: 'echoOut' }, startsInS: 75 }), 'Echo out in 1:15');
  assert.equal(nextText({ transition: { type: 'cut' }, startsInS: 3 }), 'Cut in 0:03');
  assert.equal(nextText(null), '');
});

test('the crossfader button sends a deck fully on air', () => {
  assert.equal(crossfaderFor('A'), -1);
  assert.equal(crossfaderFor('B'), 1);
});

test('the guest lock opens after holding for two seconds', () => {
  assert.equal(holdProgress(0), 0);
  assert.equal(holdProgress(1000), 0.5);
  assert.equal(holdProgress(2500), 1);
});

test('the DJ song list: search, fits, played last', () => {
  const songs = [
    { id: 1, title: 'Sugar', artist: 'Maroon 5' },
    { id: 2, title: 'Faded', artist: 'Alan Walker' },
    { id: 3, title: 'Đi Về Nhà', artist: 'Đen' },
    { id: 4, title: 'No analysis', artist: 'X' },
  ];
  const analyses = new Map([
    [1, { bpm: 120, key: '3B', energy: 7 }],
    [2, { bpm: 90, key: '8A', energy: 5 }],
    [3, { bpm: 122, key: '3B', energy: 6 }],
  ]);
  const all = filterSongsForDj(songs, analyses, { query: '', fitsWith: null, played: new Set() });
  assert.deepEqual(all.map((s) => s.id), [1, 2, 3, 4]);
  assert.deepEqual(filterSongsForDj(songs, analyses, { query: 'di ve', fitsWith: null, played: new Set() }).map((s) => s.id), [3], 'accent-insensitive');
  assert.deepEqual(filterSongsForDj(songs, analyses, { query: '', fitsWith: { bpm: 121, key: '3B' }, played: new Set() }).map((s) => s.id), [1, 3]);
  assert.deepEqual(filterSongsForDj(songs, analyses, { query: '', fitsWith: null, played: new Set([1]) }).map((s) => s.id), [2, 3, 4, 1]);
});

test('analysis progress line', () => {
  assert.equal(analysisText({ done: 3, total: 10 }), 'Analysing songs: 3 of 10');
  assert.equal(analysisText({ done: 10, total: 10 }), '');
});

test('dragging round the platter turns it (clockwise is forward)', () => {
  const c = { x: 0, y: 0 };
  assert.ok(Math.abs(jogTurns({ x: 0, y: -10 }, { x: 10, y: 0 }, c) - 0.25) < 1e-9, 'a quarter turn clockwise');
  assert.ok(Math.abs(jogTurns({ x: 10, y: 0 }, { x: 0, y: -10 }, c) + 0.25) < 1e-9);
  assert.ok(Math.abs(jogTurns({ x: -10, y: 1 }, { x: -10, y: -1 }, c) - 0.0318) < 0.001, 'across the 180 degree line');
});
