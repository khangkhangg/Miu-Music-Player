import { test } from 'node:test';
import assert from 'node:assert/strict';
import { episodeMeta, lengthLabel } from '../../src/js/podcasts.js';

test('lengths read as hours and minutes', () => {
  assert.equal(lengthLabel(3_723_000), '1 h 2 min');
  assert.equal(lengthLabel(2_730_000), '46 min');
  assert.equal(lengthLabel(40_000), '1 min', 'never 0');
  assert.equal(lengthLabel(null), '');
});

test('an episode line shows its date and length', () => {
  const e = { published: 1_792_371_600, durationMs: 3_723_000 };
  assert.equal(episodeMeta(e, 'en', 'UTC'), 'Oct 19, 2026 · 1 h 2 min');
  assert.equal(episodeMeta({ published: null, durationMs: 2_730_000 }, 'en', 'UTC'), '46 min');
  assert.equal(episodeMeta(e, 'vi', 'UTC'), '19 thg 10, 2026 · 1 h 2 min');
});
