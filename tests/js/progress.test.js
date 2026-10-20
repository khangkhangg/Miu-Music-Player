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
