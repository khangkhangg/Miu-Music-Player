import { test } from 'node:test';
import assert from 'node:assert/strict';
import { workState } from '../../src/js/progress.js';

test('background work shows a bar with what is happening and how far along it is', () => {
  assert.deepEqual(workState({ phase: 'fingerprint', done: 120, total: 480 }), { show: true, percent: 25, text: 'Fingerprinting songs: 120 of 480' });
  assert.deepEqual(workState({ phase: 'scan', done: 1, total: 3 }), { show: true, percent: 33, text: 'Checking folders: 1 of 3' });
});

test('finished or empty work hides the bar', () => {
  assert.equal(workState({ phase: 'scan', done: 3, total: 3 }).show, false);
  assert.equal(workState({ phase: 'fingerprint', done: 0, total: 0 }).show, false);
  assert.equal(workState(null).show, false);
});

test('importing podcasts has its own label', () => {
  assert.equal(workState({ phase: 'import', done: 2, total: 5 }).text, 'Adding podcasts: 2 of 5');
});

test('a download shows the episode and the amount in megabytes', () => {
  const w = workState({ phase: 'download', done: 12_000_000, total: 45_000_000, title: 'Tập 12' });
  assert.deepEqual(w, { show: true, percent: 27, text: 'Downloading “Tập 12”: 12.0 MB of 45.0 MB' });
  assert.equal(workState({ phase: 'download', done: 5_000_000, total: 0, title: 'x' }).text, 'Downloading “x”: 5.0 MB', 'size unknown');
  assert.equal(workState({ phase: 'download', done: 0, total: 0 }).show, false, 'finished');
});
