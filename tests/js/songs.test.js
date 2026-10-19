import { test } from 'node:test';
import assert from 'node:assert/strict';
import { formatDuration, songRow, visibleRange, qualityLabel } from '../../src/js/songs.js';

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
    { id: 1, title: 'Diễm xưa', artist: 'Khánh Ly', album: 'Trịnh Công Sơn', duration: '5:03', quality: '' },
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

test('lossless songs show bit depth and sample rate', () => {
  assert.equal(qualityLabel({ format: 'flac', sampleRate: 96_000, bitDepth: 24, bitrateKbps: 2_800 }), 'FLAC · 24-bit / 96 kHz');
  assert.equal(qualityLabel({ format: 'alac', sampleRate: 44_100, bitDepth: 16, bitrateKbps: 900 }), 'ALAC · 16-bit / 44.1 kHz');
  assert.equal(qualityLabel({ format: 'wavpack', sampleRate: 48_000, bitDepth: 24 }), 'WavPack · 24-bit / 48 kHz');
});

test('lossy songs show their bitrate', () => {
  assert.equal(qualityLabel({ format: 'mp3', sampleRate: 44_100, bitDepth: null, bitrateKbps: 320 }), 'MP3 · 320 kbps');
  assert.equal(qualityLabel({ format: 'vorbis', sampleRate: 44_100, bitrateKbps: 192 }), 'Ogg Vorbis · 192 kbps');
});

test('missing details shorten the label instead of showing blanks', () => {
  assert.equal(qualityLabel({ format: 'aac', sampleRate: null, bitDepth: null, bitrateKbps: null }), 'AAC');
  assert.equal(qualityLabel({ format: 'flac', sampleRate: 44_100, bitDepth: null }), 'FLAC · 44.1 kHz');
  assert.equal(qualityLabel({ format: 'unknown' }), '');
});

test('song rows carry the quality label', () => {
  assert.equal(songRow({ id: 1, title: 't', format: 'mp3', bitrateKbps: 128 }).quality, 'MP3 · 128 kbps');
});
