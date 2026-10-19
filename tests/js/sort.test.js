import { test } from 'node:test';
import assert from 'node:assert/strict';
import { sortSongs, indexLetter, indexEntries, sortLocale } from '../../src/js/sort.js';

const titles = (list) => list.map((s) => s.title);
const make = (...ts) => ts.map((title, id) => ({ id, title, artist: '' }));

test('Vietnamese order: A Ă Â … D Đ E Ê … O Ô Ơ … U Ư', () => {
  const list = make('Uyên', 'Ở', 'Đà Lạt', 'Ăn', 'Em', 'Âm thầm', 'Ông', 'Anh', 'Dạ khúc', 'Ứ', 'Ê a', 'Ơn', 'Bé');
  assert.deepEqual(titles(sortSongs(list, 'vi')),
    ['Anh', 'Ăn', 'Âm thầm', 'Bé', 'Dạ khúc', 'Đà Lạt', 'Em', 'Ê a', 'Ông', 'Ở', 'Ơn', 'Uyên', 'Ứ']);
});

test('numbers sort by value and English titles keep their usual order', () => {
  assert.deepEqual(titles(sortSongs(make('10 năm', 'Zebra', '2 vợ', 'apple', 'Banana'), 'vi')), ['2 vợ', '10 năm', 'apple', 'Banana', 'Zebra']);
});

test('same titles fall back to the artist', () => {
  const list = [{ id: 1, title: 'Hello', artist: 'Lionel Richie' }, { id: 2, title: 'Hello', artist: 'Adele' }];
  assert.deepEqual(sortSongs(list, 'vi').map((s) => s.id), [2, 1]);
});

test('sorting leaves the input list alone', () => {
  const list = make('b', 'a');
  sortSongs(list, 'vi');
  assert.deepEqual(titles(list), ['b', 'a']);
});

test('index letters follow the Vietnamese alphabet', () => {
  const letters = ['Ấm áp', 'ăn', 'Đêm đông', 'Ở trọ', 'Ước mơ', 'Ồ'].map((t) => indexLetter(t, 'vi'));
  assert.deepEqual(letters, ['Â', 'Ă', 'Đ', 'Ơ', 'Ư', 'Ô']);
});

test('elsewhere accented letters index under the plain letter', () => {
  assert.deepEqual(['Ấm áp', 'Đêm', 'Éclair'].map((t) => indexLetter(t, 'en')), ['A', 'D', 'E']);
});

test('digits, other scripts and leading punctuation', () => {
  assert.equal(indexLetter('10 năm', 'vi'), '#');
  assert.equal(indexLetter('"(I Can\'t Get No) Satisfaction"', 'vi'), 'I');
  assert.equal(indexLetter('Привет', 'vi'), 'П');
  assert.equal(indexLetter('夜に駆ける', 'vi'), '…');
  assert.equal(indexLetter('', 'vi'), '…');
});

test('the index bar lists each letter once, at its first song', () => {
  const sorted = sortSongs(make('Anh', 'Ăn', 'Bé', 'Bên', 'Đêm', '2 vợ'), 'vi');
  assert.deepEqual(indexEntries(sorted, 'vi'), [
    { letter: '#', index: 0 }, { letter: 'A', index: 1 }, { letter: 'Ă', index: 2 }, { letter: 'B', index: 3 }, { letter: 'Đ', index: 5 },
  ]);
});

test('English and Vietnamese systems sort with Vietnamese rules; others keep their own', () => {
  assert.equal(sortLocale(['en-US', 'en']), 'vi');
  assert.equal(sortLocale(['vi-VN']), 'vi');
  assert.equal(sortLocale(['de-DE', 'en']), 'de-DE');
  assert.equal(sortLocale([]), 'vi');
});
