import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fold, searchSongs } from '../../src/js/search.js';

const songs = [
  { id: 1, title: 'Mùa thu cho em', artist: 'Ngọc Lan', album: 'Mùa thu' },
  { id: 2, title: 'Diễm xưa', artist: 'Khánh Ly', album: 'Trịnh Công Sơn' },
  { id: 3, title: 'Đà Lạt hoàng hôn', artist: 'Đức Huy', album: null },
  { id: 4, title: '夜に駆ける', artist: 'YOASOBI', album: 'THE BOOK' },
  { id: 5, title: 'Café del Mar', artist: 'Energy 52', album: '' },
];

const ids = (q) => searchSongs(songs, q).map((s) => s.id);

test('folding drops accents and case, and turns đ into d', () => {
  assert.equal(fold('Mùa Thu'), 'mua thu');
  assert.equal(fold('Diễm xưa'), 'diem xua');
  assert.equal(fold('Đà Lạt'), 'da lat');
  assert.equal(fold('ＹＯＡＳＯＢＩ'), 'yoasobi', 'full-width letters fold too');
});

test('Vietnamese without accents finds accented titles', () => {
  assert.deepEqual(ids('mua thu'), [1]);
  assert.deepEqual(ids('DIEM XUA'), [2]);
  assert.deepEqual(ids('da lat'), [3]);
  assert.deepEqual(ids('Mùa Thu'), [1], 'typing the accents works too');
});

test('CJK matches by substring', () => {
  assert.deepEqual(ids('駆け'), [4]);
  assert.deepEqual(ids('夜'), [4]);
});

test('kana sound marks are kept (ガ is not カ)', () => {
  assert.notEqual(fold('ガ'), fold('カ'));
});

test('every word must match, in any field', () => {
  assert.deepEqual(ids('khanh ly diem'), [2], 'artist and title words together');
  assert.deepEqual(ids('khanh ly mua'), []);
  assert.deepEqual(ids('cafe'), [5]);
});

test('an empty query shows everything in order', () => {
  assert.deepEqual(ids(''), [1, 2, 3, 4, 5]);
  assert.deepEqual(ids('   '), [1, 2, 3, 4, 5]);
});
