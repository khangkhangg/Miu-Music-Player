import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fromBytes, geometry } from '../../src/js/visualizer.js';

test('bytes from the engine become 0..1 levels', () => {
  assert.deepEqual(fromBytes([0, 255, 51]), [0, 1, 0.2]);
});

test('bars fill the width with gaps and stand on the bottom', () => {
  const g = geometry('bars', [1, 0.5], 102, 40, 2);
  assert.deepEqual(g.rects, [
    { x: 0, y: 0, w: 50, h: 40 },
    { x: 52, y: 20, w: 50, h: 20 },
  ]);
});

test('silent bands still show a hairline so the strip is visible', () => {
  const g = geometry('bars', [0], 10, 40, 2);
  assert.equal(g.rects[0].h, 1);
  assert.equal(g.rects[0].y, 39);
});

test('mirrored bars are centred vertically', () => {
  const g = geometry('mirrored', [0.5], 10, 40, 2);
  assert.deepEqual(g.rects, [{ x: 0, y: 10, w: 10, h: 20 }]);
});

test('line style gives one point per band, at the band centre', () => {
  const g = geometry('line', [1, 0, 0.5], 30, 40, 0);
  assert.deepEqual(g.points, [{ x: 5, y: 0 }, { x: 15, y: 40 }, { x: 25, y: 20 }]);
});

test('an unknown style draws bars', () => {
  assert.ok(geometry('lasers', [0.5], 10, 40, 2).rects);
});
