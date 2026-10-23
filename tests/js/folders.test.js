import { test } from 'node:test';
import assert from 'node:assert/strict';
import { folderItems, librarySummary, encodingOptions } from '../../src/js/folders.js';

test('folder items show the last path part, the full path as tooltip and the song count', () => {
  assert.deepEqual(folderItems([{ id: 1, path: 'E:\\Nhạc Trịnh', songCount: 36 }]), [
    { id: 1, name: 'Nhạc Trịnh', title: 'E:\\Nhạc Trịnh', count: 36, encoding: '', offline: false },
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

test('a folder remembers its forced tag encoding', () => {
  assert.equal(folderItems([{ id: 1, path: 'E:\Old', songCount: 1, tagEncoding: 'windows-1258' }])[0].encoding, 'windows-1258');
});

test('encoding choices start with automatic detection and mark the current one', () => {
  const opts = encodingOptions('gbk');
  assert.deepEqual(opts[0], { value: '', label: 'Detect automatically', selected: false });
  assert.deepEqual(opts.map((o) => o.value), ['', 'windows-1258', 'gbk', 'big5', 'shift_jis', 'euc-kr', 'windows-1251', 'windows-1252']);
  assert.equal(opts.find((o) => o.selected).label, 'Chinese, simplified (GBK)');
  assert.equal(encodingOptions('').find((o) => o.selected).value, '');
});

test('a folder whose drive is not connected is marked offline, and only then', () => {
  assert.equal(folderItems([{ id: 1, path: 'D:\Music', songCount: 940, online: false }])[0].offline, true);
  assert.equal(folderItems([{ id: 1, path: 'D:\Music', songCount: 940, online: true }])[0].offline, false);
});
