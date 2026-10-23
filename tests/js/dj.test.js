import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { useDictionary } from '../../src/js/i18n.js';
import {
  bpmLabel, tempoLabel, timeLeft, knobDrag, knobKey, meterHeight, waveformPath, keysCompatible, fits,
  songMeta, nextText, crossfaderFor, filterSongsForDj, analysisText, jogTurns, keyAction, loopBeats, learnControl, moveSelection, transitionChoices, transitionName, keyShiftLabel, kindChoices, transitionsSummary,
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

test('the DJ keyboard: deck A on the left hand, deck B on the right', () => {
  const state = { crossfader: 0.95, decks: [{ echo: false, sweep: false }, { echo: true, sweep: false }] };
  const k = (code, shiftKey = false) => keyAction({ code, shiftKey }, state);
  assert.deepEqual(k('KeyZ').down, { type: 'playPause', deck: 'A' });
  assert.deepEqual(k('KeyM').down, { type: 'playPause', deck: 'B' });
  assert.deepEqual(k('KeyX'), { down: { type: 'cue', deck: 'A', pressed: true }, up: { type: 'cue', deck: 'A', pressed: false } });
  assert.deepEqual(k('KeyC').down, { type: 'sync', deck: 'A' });
  assert.deepEqual(k('Digit2').down, { type: 'hotCue', deck: 'A', slot: 1 });
  assert.deepEqual(k('Digit2', true).down, { type: 'clearHotCue', deck: 'A', slot: 1 });
  assert.deepEqual(k('Digit9').down, { type: 'hotCue', deck: 'B', slot: 2 });
  assert.deepEqual(k('KeyQ').down, { type: 'loop', deck: 'A', beats: 4 });
  assert.deepEqual(k('KeyE').down, { type: 'loop', deck: 'A', beats: null });
  assert.deepEqual(k('KeyA').down, { type: 'effect', deck: 'A', effect: 'echo', on: true });
  assert.deepEqual(k('KeyJ').down, { type: 'effect', deck: 'B', effect: 'echo', on: false }, 'toggles off');
  assert.deepEqual(k('KeyL').down, { type: 'effect', deck: 'B', effect: 'spinBack', on: true });
  assert.deepEqual(k('F3').down, { type: 'pad', slot: 2 });
  assert.deepEqual(k('ArrowRight').down, { type: 'crossfader', value: 1 });
  assert.deepEqual(k('ArrowLeft').down, { type: 'crossfader', value: 0.85 });
  assert.equal(k('KeyY'), null);
});

test('a loop is shown by its length in beats (of the song, whatever the tempo)', () => {
  assert.equal(loopBeats({ loopS: [10, 12], bpm: 120, tempo: 0 }), 4);
  assert.equal(loopBeats({ loopS: [10, 14], bpm: 124.8, tempo: 0.5 }), 8, 'the song is at 120, played 4 % faster');
  assert.equal(loopBeats({ loopS: null, bpm: 120, tempo: 0 }), null);
});

test('MIDI learn: each control on the screen stands for a controller control', () => {
  assert.deepEqual(learnControl({ ctl: 'play' }, 'A'), { control: 'playPause', deck: 'A' });
  assert.deepEqual(learnControl({ ctl: 'eq', band: 'mid' }, 'B'), { control: 'eqMid', deck: 'B' });
  assert.deepEqual(learnControl({ ctl: 'phones' }, 'A'), { control: 'headphoneCue', deck: 'A' });
  assert.deepEqual(learnControl({ slot: '2' }, 'B'), { control: 'hotCue3', deck: 'B' });
  assert.deepEqual(learnControl({ loop: '4' }, 'A'), { control: 'loop4', deck: 'A' });
  assert.deepEqual(learnControl({ loop: 'off' }, 'A'), { control: 'loopExit', deck: 'A' });
  assert.deepEqual(learnControl({ fx: 'echo' }, 'A'), { control: 'fxOn', deck: 'A' });
  assert.deepEqual(learnControl({ ctl: 'xfader' }, null), { control: 'crossfader', deck: null });
  assert.deepEqual(learnControl({ ctl: 'master' }, null), { control: 'masterVolume', deck: null });
  assert.deepEqual(learnControl({ ctl: 'jog' }, 'B'), { control: 'jog', deck: 'B' });
  assert.equal(learnControl({ loop: '16' }, 'A'), null, 'only the 4-beat loop has a controller button');
  assert.equal(learnControl({ pad: '1' }, null), null);
});

test('a controller browse knob moves the list selection, within the list', () => {
  assert.equal(moveSelection(-1, 1, 10), 0, 'nothing selected: the first');
  assert.equal(moveSelection(3, 2, 10), 5);
  assert.equal(moveSelection(8, 5, 10), 9);
  assert.equal(moveSelection(1, -3, 10), 0);
  assert.equal(moveSelection(0, 1, 0), -1, 'empty list');
});

test('every transition Auto DJ can play has a name', () => {
  const kinds = [
    { type: 'beatBlend', bars: 16 }, { type: 'filterBlend', bars: 8 }, { type: 'breakdownBlend', bars: 8 },
    { type: 'loopOut', bars: 8 }, { type: 'dropSwap', bars: 8 }, { type: 'echoOut' }, { type: 'reverbWash' },
    { type: 'spinBack' }, { type: 'brake' }, { type: 'crossfade', secs: 8 }, { type: 'balladGap' }, { type: 'cut' },
  ];
  for (const transition of kinds) {
    const text = nextText({ transition, startsInS: 10 });
    assert.ok(text && !text.includes('dj.tr') && !text.startsWith('undefined'), `${transition.type}: ${text}`);
  }
  assert.equal(nextText({ transition: { type: 'dropSwap', bars: 8 }, startsInS: 10 }), 'Drop swap in 0:10');
});

test('transitions to pick by hand, each with a name', () => {
  const choices = transitionChoices();
  assert.ok(choices.length >= 14);
  assert.equal(choices[0].value, 'auto');
  for (const c of choices.slice(1)) {
    assert.ok(c.label && !c.label.includes('dj.tr'), `${c.value}: ${c.label}`);
    assert.deepEqual(JSON.parse(c.value).type.length > 0, true);
  }
  assert.ok(choices.some((c) => c.value !== 'auto' && JSON.parse(c.value).type === 'vocalOut'));
  assert.equal(transitionName({ type: 'tempoRamp', bars: 32 }), 'Tempo ramp');
  assert.equal(transitionName({ type: 'keyShiftBlend', bars: 16, semitones: -2 }), 'Key shift blend (-2)');
});

test('key shift reads as semitones', () => {
  assert.equal(keyShiftLabel(0), '');
  assert.equal(keyShiftLabel(2), '+2');
  assert.equal(keyShiftLabel(-1), '-1');
});

test('the transition checklist: every kind with a name, and a short summary', () => {
  const kinds = kindChoices();
  assert.equal(kinds.length, 15);
  for (const k of kinds) assert.ok(k.label && !k.label.includes('dj.'), `${k.kind}: ${k.label}`);
  assert.equal(transitionsSummary({ allowed: [], mixItUp: false }), 'Transitions: best fit');
  assert.equal(transitionsSummary({ allowed: ['echoOut', 'cut', 'brake', 'beatBlend'], mixItUp: false }), 'Transitions: 4 chosen');
  assert.equal(transitionsSummary({ allowed: ['echoOut', 'cut'], mixItUp: true }), 'Transitions: 2 chosen, mixed up');
});
