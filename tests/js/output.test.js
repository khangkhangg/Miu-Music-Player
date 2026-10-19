import { test } from 'node:test';
import assert from 'node:assert/strict';
import { rateLabel, sourceLabel, signalPath, eqActive, deviceOptions } from '../../src/js/output.js';

test('sample rates read like a spec sheet', () => {
  assert.deepEqual([44_100, 48_000, 96_000, 192_000, 88_200].map(rateLabel), ['44.1 kHz', '48 kHz', '96 kHz', '192 kHz', '88.2 kHz']);
});

test('source shows codec, bit depth and rate', () => {
  assert.equal(sourceLabel('flac', { rate: 96_000, bits: 24, channels: 2 }), 'FLAC · 24-bit / 96 kHz');
  assert.equal(sourceLabel('mp3', { rate: 44_100, bits: null, channels: 2 }), 'MP3 · 44.1 kHz');
  assert.equal(sourceLabel('alac', { rate: 44_100, bits: 16, channels: 1 }), 'ALAC · 16-bit / 44.1 kHz · mono');
  assert.equal(sourceLabel('mp3', null), '');
});

test('bit-perfect only with exclusive mode, EQ off and matching rates', () => {
  assert.equal(signalPath({ sourceRate: 96_000, deviceRate: 96_000, eqActive: false, exclusive: true, volume: 1, sourceBits: 24, deviceBits: 32 }), 'Bit-perfect');
  assert.notEqual(signalPath({ sourceRate: 96_000, deviceRate: 96_000, eqActive: true, exclusive: true }), 'Bit-perfect');
  assert.notEqual(signalPath({ sourceRate: 44_100, deviceRate: 96_000, eqActive: false, exclusive: true }), 'Bit-perfect');
  assert.notEqual(signalPath({ sourceRate: 48_000, deviceRate: 48_000, eqActive: false, exclusive: false }), 'Bit-perfect');
});

test('otherwise the path says what happens to the sound', () => {
  assert.equal(signalPath({ sourceRate: 44_100, deviceRate: 48_000, eqActive: true, exclusive: false }),
    'Shared mode (Windows mixer) · resampled 44.1 → 48 kHz · EQ on (32-bit float)');
  assert.equal(signalPath({ sourceRate: 48_000, deviceRate: 48_000, eqActive: false, exclusive: false }),
    'Shared mode (Windows mixer)');
});

test('the EQ counts as active only when on and not flat', () => {
  const flat = { enabled: true, preampDb: 0, gainsDb: Array(10).fill(0) };
  assert.equal(eqActive(flat), false);
  assert.equal(eqActive({ ...flat, preampDb: -3 }), true);
  assert.equal(eqActive({ ...flat, enabled: false, preampDb: -3 }), false);
});

test('device menu starts with the system default', () => {
  const state = { devices: [{ name: 'LG HDR 4K', isDefault: false }, { name: 'Speakers (USB Audio and HID)', isDefault: true }], chosen: null };
  assert.deepEqual(deviceOptions(state), [
    { value: '', label: 'System default (Speakers (USB Audio and HID))', selected: true },
    { value: 'LG HDR 4K', label: 'LG HDR 4K', selected: false },
    { value: 'Speakers (USB Audio and HID)', label: 'Speakers (USB Audio and HID)', selected: false },
  ]);
  assert.equal(deviceOptions({ ...state, chosen: 'LG HDR 4K' })[1].selected, true);
});

test('exclusive mode below full volume or with too few bits says why it is not bit-perfect', () => {
  const base = { sourceRate: 44_100, deviceRate: 44_100, eqActive: false, exclusive: true, volume: 1, sourceBits: 24, deviceBits: 24 };
  assert.equal(signalPath({ ...base, volume: 0.8 }), 'Exclusive mode · volume 80%');
  assert.equal(signalPath({ ...base, deviceBits: 16 }), 'Exclusive mode · reduced to 16-bit');
  assert.equal(signalPath({ ...base, sourceBits: null }), 'Bit-perfect', 'lossy files have no bit depth to lose');
});
