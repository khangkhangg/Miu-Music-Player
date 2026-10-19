import { test } from 'node:test';
import assert from 'node:assert/strict';
import { presetOptions, bandLabel, dbLabel, withGain, clipText } from '../../src/js/eq.js';

const flat = { enabled: true, preampDb: 0, gainsDb: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0] };
const state = (over = {}) => ({
  settings: flat,
  builtIn: [{ name: 'Flat' }, { name: 'Bass boost' }],
  user: [{ name: 'Car' }],
  preset: 'Flat',
  clipRisk: false,
  ...over,
});

test('preset menu lists built-in then user presets', () => {
  assert.deepEqual(presetOptions(state()), [
    { value: 'Flat', label: 'Flat', user: false },
    { value: 'Bass boost', label: 'Bass boost', user: false },
    { value: 'Car', label: 'Car', user: true },
  ]);
});

test('Custom appears first when the settings match no preset', () => {
  const opts = presetOptions(state({ preset: null }));
  assert.deepEqual(opts[0], { value: '', label: 'Custom', user: false });
  assert.equal(opts.length, 4);
});

test('band and dB labels are short', () => {
  assert.deepEqual([31, 62, 500, 1000, 2000, 16000].map(bandLabel), ['31', '62', '500', '1k', '2k', '16k']);
  assert.deepEqual([6, 0, -4, 2.5].map(dbLabel), ['+6', '0', '-4', '+2.5']);
});

test('moving a slider changes only that band (or the preamp) and leaves the original alone', () => {
  const next = withGain(flat, 5, 4);
  assert.equal(next.gainsDb[5], 4);
  assert.equal(flat.gainsDb[5], 0, 'original untouched');
  assert.equal(withGain(flat, -1, -3).preampDb, -3, 'index -1 is the preamp');
});

test('clipping warning says how much to lower the preamp', () => {
  assert.equal(clipText(state()), null);
  const risky = { ...flat, preampDb: -1, gainsDb: [3, 0, 0, 0, 0, 0, 0, 0, 0, 0] };
  assert.equal(clipText(state({ settings: risky, clipRisk: true })), 'May distort: lower the preamp by 2 dB');
});
