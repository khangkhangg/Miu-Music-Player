import { test } from 'node:test';
import assert from 'node:assert/strict';
import { folderItems, librarySummary } from '../../src/js/folders.js';

test('folder items show the last path part, the full path as tooltip and the song count', () => {
  assert.deepEqual(folderItems([{ id: 1, path: 'E:\\Nhạc Trịnh', songCount: 36 }]), [
    { id: 1, name: 'Nhạc Trịnh', title: 'E:\\Nhạc Trịnh', count: 36 },
  ]);
});

test('a drive root keeps its full name', () => {
  assert.equal(folderItems([{ id: 2, path: 'D:\\', songCount: 0 }])[0].name, 'D:\\');
});

test('no folders means no summary (the empty state shows instead)', () => {
  assert.equal(librarySummary([]), null);
  assert.equal(librarySummary(undefined), null);
});

test('summary counts songs and folders with plurals', () => {
  assert.equal(librarySummary([{ songCount: 382 }, { songCount: 64 }]), '446 songs in 2 folders');
  assert.equal(librarySummary([{ songCount: 1 }]), '1 song in 1 folder');
});
