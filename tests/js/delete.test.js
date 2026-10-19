import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deletePrompt, deleteResult } from '../../src/js/delete.js';

const s = (title) => ({ title });

test('one song is named in the question', () => {
  const p = deletePrompt([s('Baby Shark (remix)')]);
  assert.equal(p.title, 'Delete 1 song?');
  assert.match(p.message, /Baby Shark \(remix\)/);
  assert.match(p.message, /Recycle Bin/);
  assert.match(p.message, /restore/i);
});

test('several songs: first three named, the rest counted', () => {
  const p = deletePrompt([s('A'), s('B'), s('C'), s('D'), s('E')]);
  assert.equal(p.title, 'Delete 5 songs?');
  assert.match(p.message, /A, B, C and 2 more/);
});

test('result message reports what happened', () => {
  assert.equal(deleteResult({ deleted: [1, 2], failed: [] }), 'Moved 2 songs to the Recycle Bin.');
  assert.equal(deleteResult({ deleted: [1], failed: [[2, 'file is in use']] }),
    'Moved 1 song to the Recycle Bin. 1 could not be moved: file is in use');
});
