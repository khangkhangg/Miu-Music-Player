import { test } from 'node:test';
import assert from 'node:assert/strict';
import { bandColour, bandHeight, sectionSpans, phraseTimes, beatGrid, mixZone, cueMarks, levelsAt, deckSpeed, SECTION_COLOURS } from '../../src/js/djwave.js';

const rgb = (s) => s.match(/\d+/g).map(Number);

test('colour says what is sounding: bass blue, mids amber, highs white', () => {
  const [r, g, b] = rgb(bandColour([200, 10, 5]));
  assert.ok(b > r && b > 150, `bass ${r},${g},${b}`);
  const [r2, g2, b2] = rgb(bandColour([5, 200, 10]));
  assert.ok(r2 > b2 + 80 && g2 > b2, `mids ${r2},${g2},${b2}`);
  const [r3, g3, b3] = rgb(bandColour([0, 5, 220]));
  assert.ok(r3 > 200 && g3 > 200 && b3 > 200, `highs ${r3},${g3},${b3}`);
  assert.deepEqual(rgb(bandColour([0, 0, 0])), rgb(bandColour([0, 0, 0])), 'silence has a colour too');
});

test('height follows how loud it is, 0 to 1', () => {
  assert.equal(bandHeight([0, 0, 0]), 0);
  assert.equal(bandHeight([255, 255, 255]), 1);
  assert.ok(bandHeight([120, 40, 20]) > bandHeight([60, 20, 10]));
});

const analysis = {
  bpm: 120, downbeatS: 0.5, firstBeatS: 0.5,
  structure: {
    phraseStartS: 0.5,
    sections: [
      { kind: 'intro', startS: 0, endS: 16.5 },
      { kind: 'drop', startS: 16.5, endS: 80.5 },
      { kind: 'outro', startS: 80.5, endS: 100 },
    ],
    dropS: 16.5, vocals: [], endS: 98,
  },
};

test('sections as parts of the song, each with its colour', () => {
  const s = sectionSpans(analysis, 100);
  assert.deepEqual(s.map((x) => x.kind), ['intro', 'drop', 'outro']);
  assert.equal(s[1].from, 0.165);
  assert.equal(s[2].to, 1);
  for (const k of ['intro', 'main', 'breakdown', 'build', 'drop', 'outro']) assert.ok(SECTION_COLOURS[k], k);
  assert.deepEqual(sectionSpans({ bpm: 0 }, 100), [], 'older analyses have none');
});

test('a line every phrase (8 bars) from the phrase grid', () => {
  // 120 BPM: a bar is 2 s, a phrase 16 s.
  assert.deepEqual(phraseTimes(analysis, 50), [0.5, 16.5, 32.5, 48.5]);
  assert.deepEqual(phraseTimes({ bpm: 120, structure: { phraseStartS: 20.5 } }, 40), [4.5, 20.5, 36.5], 'also before the first one found');
  assert.deepEqual(phraseTimes({ bpm: 0, structure: {} }, 40), []);
});

test('beats in a window, bars marked', () => {
  const g = beatGrid(analysis, 0.25, 3);
  assert.deepEqual(g.map((b) => b.t), [0.5, 1, 1.5, 2, 2.5]);
  assert.deepEqual(g.map((b) => b.bar), [true, false, false, false, true]);
  assert.deepEqual(beatGrid({ bpm: 0 }, 0, 3), []);
});

test('the mix zone on the song on air', () => {
  assert.deepEqual(mixZone({ atS: 60, lengthS: 16 }, 100), { from: 0.6, to: 0.76 });
  assert.deepEqual(mixZone({ atS: 95, lengthS: 16 }, 100), { from: 0.95, to: 1 });
  assert.equal(mixZone(null, 100), null);
  assert.equal(mixZone({ atS: 60, lengthS: 16 }, 0), null);
});

test('hot cues and the loop as places in the song', () => {
  assert.deepEqual(cueMarks({ hotCues: [10, null, 50, null], loopS: [20, 22], durationS: 100 }), {
    cues: [{ slot: 0, at: 0.1 }, { slot: 2, at: 0.5 }],
    loop: { from: 0.2, to: 0.22 },
  });
  assert.deepEqual(cueMarks({ durationS: 0 }), { cues: [], loop: null });
});

test('the band levels at a time in the zoomed view', () => {
  const detail = [1, 2, 3, 4, 5, 6, 7, 8, 9];
  assert.deepEqual(levelsAt(detail, 100, 0.015), [4, 5, 6]);
  assert.equal(levelsAt(detail, 100, -0.01), null);
  assert.equal(levelsAt(detail, 100, 0.05), null);
});

test('how fast a deck plays its song, to scroll the zoomed view', () => {
  assert.equal(deckSpeed({ bpm: 126, tempo: 0.5 }, { bpm: 120 }), 1.05);
  assert.equal(deckSpeed({ tempo: 0.5 }, null), 1.04);
  assert.equal(deckSpeed({ tempo: 0 }, { bpm: 0 }), 1);
});
