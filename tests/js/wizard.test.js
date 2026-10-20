import { test } from 'node:test';
import assert from 'node:assert/strict';
import { wizard } from '../../src/js/wizard.js';

const base = { driversReady: false, phone: 'none', folders: 0, skippedPhone: false, dismissed: false };

test('drivers come first', () => {
  const w = wizard(base);
  assert.equal(w.current, 'drivers');
  assert.deepEqual(w.steps.map((s) => [s.id, s.state]), [['drivers', 'current'], ['phone', 'todo'], ['folders', 'todo']]);
});

test('then the phone, waiting for Trust when it is plugged in but untrusted', () => {
  assert.equal(wizard({ ...base, driversReady: true }).current, 'phone');
  assert.equal(wizard({ ...base, driversReady: true, phone: 'untrusted' }).hint, 'trust');
});

test('the phone step can be skipped (play music without an iPhone)', () => {
  const w = wizard({ ...base, driversReady: true, skippedPhone: true });
  assert.equal(w.current, 'folders');
  assert.equal(w.steps[1].state, 'skipped');
});

test('folders last; with all done the wizard closes', () => {
  assert.equal(wizard({ ...base, driversReady: true, phone: 'ready' }).current, 'folders');
  const done = wizard({ ...base, driversReady: true, phone: 'ready', folders: 2 });
  assert.equal(done.current, null);
  assert.equal(done.show, false);
});

test('a dismissed wizard stays closed', () => {
  assert.equal(wizard({ ...base, dismissed: true }).show, false);
});

test('without drivers the phone cannot be skipped past them, but folders still can be added', () => {
  const w = wizard({ ...base, folders: 1 });
  assert.equal(w.current, 'drivers');
  assert.equal(w.steps[2].state, 'done');
});
