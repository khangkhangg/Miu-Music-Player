import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  FIELDS, editorFields, changesFor, parseFilename, autoNumber, findReplace, changeCase, previewRows,
} from '../../src/js/editor.js';

const a = { id: 1, path: 'D:\\Nhạc\\01 - Khánh Ly - Diễm xưa.mp3', tags: { title: 'Diễm xưa', artist: 'Khánh Ly', album: 'Trịnh', year: 1970, track: 1 } };
const b = { id: 2, path: 'D:\\Nhạc\\02 - Khánh Ly - Hạ trắng.mp3', tags: { title: 'Hạ trắng', artist: 'Khánh Ly', album: 'Trịnh', year: null, track: 2 } };

test('one song shows its values; several show shared values and "mixed" for the rest', () => {
  assert.deepEqual(editorFields([a]).title, { value: 'Diễm xưa', mixed: false });
  const f = editorFields([a, b]);
  assert.deepEqual(f.artist, { value: 'Khánh Ly', mixed: false });
  assert.deepEqual(f.title, { value: '', mixed: true });
  assert.deepEqual(f.year, { value: '', mixed: true });
  assert.deepEqual(f.genre, { value: '', mixed: false });
  assert.deepEqual(Object.keys(f), FIELDS.map((x) => x.key));
});

test('only touched fields change; emptied ones clear; numbers are numbers', () => {
  const edited = { title: 'Mùa thu', year: '', track: '7' };
  assert.deepEqual(changesFor(edited), { title: { set: 'Mùa thu' }, year: 'clear', track: { set: 7 } });
  assert.deepEqual(changesFor({}), {});
  assert.deepEqual(changesFor({ track: 'abc' }), {}, 'not a number: left alone');
});

test('file names parse with a pattern', () => {
  assert.deepEqual(parseFilename('%track% - %artist% - %title%', a.path), { track: '01', artist: 'Khánh Ly', title: 'Diễm xưa' });
  assert.deepEqual(parseFilename('%artist% - %title%', 'C:\\x\\Ngọc Lan - Mùa thu cho em.flac'), { artist: 'Ngọc Lan', title: 'Mùa thu cho em' });
  assert.equal(parseFilename('%track%. %title%', a.path), null, 'no match');
});

test('auto-number gives tracks in order and the total', () => {
  assert.deepEqual(autoNumber([b, a]), [{ track: '1', trackTotal: '2' }, { track: '2', trackTotal: '2' }]);
  assert.deepEqual(autoNumber([a, b], 5), [{ track: '5', trackTotal: '6' }, { track: '6', trackTotal: '6' }]);
});

test('find and replace, with or without matching case', () => {
  assert.equal(findReplace('Khánh Ly - Live', ' - Live', ''), 'Khánh Ly');
  assert.equal(findReplace('DIỄM xưa diễm', 'diễm', 'Hạ', false), 'Hạ xưa Hạ');
  assert.equal(findReplace('DIỄM xưa diễm', 'diễm', 'Hạ', true), 'DIỄM xưa Hạ');
  assert.equal(findReplace('a.b', '.', '-'), 'a-b', 'plain text, not a pattern');
});

test('case changes handle Vietnamese letters', () => {
  assert.equal(changeCase('diễm xưa của trịnh', 'title'), 'Diễm Xưa Của Trịnh');
  assert.equal(changeCase('đêm đông', 'upper'), 'ĐÊM ĐÔNG');
  assert.equal(changeCase('ĐÊM ĐÔNG', 'lower'), 'đêm đông');
  assert.equal(changeCase('ĐÊM ĐÔNG LẠNH', 'sentence'), 'Đêm đông lạnh');
});

test('the preview lists only songs whose shown values change', () => {
  const rows = previewRows([a, b], [{ title: 'Diễm xưa' }, { title: 'Hạ Trắng', track: '9' }]);
  assert.deepEqual(rows, [{ id: 2, changes: [{ field: 'title', from: 'Hạ trắng', to: 'Hạ Trắng' }, { field: 'track', from: '2', to: '9' }] }]);
});
