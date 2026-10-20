import { test } from 'node:test';
import assert from 'node:assert/strict';
import { select } from '../../src/js/selection.js';

const ids = [10, 11, 12, 13, 14];
const ids_ = (s) => [...s.selected].sort((x, y) => x - y);

test('a click selects one song and sets the anchor', () => {
  const s = select({ selected: new Set([12]), anchor: 2 }, ids, 0, {});
  assert.deepEqual(ids_(s), [10]);
  assert.equal(s.anchor, 0);
});

test('ctrl-click adds or removes one song', () => {
  let s = select({ selected: new Set([10]), anchor: 0 }, ids, 3, { ctrl: true });
  assert.deepEqual(ids_(s), [10, 13]);
  s = select(s, ids, 0, { ctrl: true });
  assert.deepEqual(ids_(s), [13]);
});

test('shift-click selects the range from the anchor, either direction', () => {
  assert.deepEqual(ids_(select({ selected: new Set([11]), anchor: 1 }, ids, 3, { shift: true })), [11, 12, 13]);
  assert.deepEqual(ids_(select({ selected: new Set([13]), anchor: 3 }, ids, 0, { shift: true })), [10, 11, 12, 13]);
});

test('ctrl+shift adds the range to what is selected', () => {
  const s = select({ selected: new Set([14]), anchor: 0 }, ids, 1, { shift: true, ctrl: true });
  assert.deepEqual(ids_(s), [10, 11, 14]);
});

test('the selection starts empty', () => {
  assert.deepEqual(ids_(select(undefined, ids, 4, {})), [14]);
});
