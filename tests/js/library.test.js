import { test } from 'node:test';
import assert from 'node:assert/strict';
import { groupAlbums, groupArtists, libraryStats, splitTitle } from '../../src/js/library.js';

const song = (id, title, artist, album, extra = {}) => ({
  id, title, artist, album, albumArtist: null, year: null, trackNo: null, discNo: null, hasArtwork: false, durationMs: 200_000, rating: 0, ...extra,
});

const songs = [
  song(1, 'Hạ trắng', 'Khánh Ly', 'Diễm xưa', { trackNo: 2, year: 1970 }),
  song(2, 'Diễm xưa', 'Khánh Ly', 'Diễm xưa', { trackNo: 1, hasArtwork: true, rating: 1 }),
  song(3, 'Mùa thu cho em', 'Ngọc Lan', 'Mùa thu', { hasArtwork: true, rating: 1 }),
  song(4, 'Greatest', 'A', 'Best of'),
  song(5, 'Greatest', 'B', 'Best of'),
  song(6, 'Song 1', 'X', 'Tình ca', { albumArtist: 'Nhiều ca sĩ', discNo: 2, trackNo: 1 }),
  song(7, 'Song 2', 'Y', 'Tình ca', { albumArtist: 'Nhiều ca sĩ', discNo: 1, trackNo: 5 }),
  song(8, 'Loose', 'Z', null, { rating: -1 }),
];

test('albums group by album and album artist, songs in disc and track order', () => {
  const albums = groupAlbums(songs, 'vi');
  const diem = albums.find((a) => a.title === 'Diễm xưa');
  assert.deepEqual(diem.songs.map((s) => s.id), [2, 1]);
  assert.equal(diem.artist, 'Khánh Ly');
  assert.equal(diem.year, 1970);
  assert.equal(diem.count, 2);
  assert.equal(diem.durationMs, 400_000);
  assert.equal(diem.coverId, 2, 'first song with art');
});

test('a compilation is one album; same name by different artists is two', () => {
  const albums = groupAlbums(songs, 'vi');
  assert.deepEqual(albums.find((a) => a.title === 'Tình ca').songs.map((s) => s.id), [7, 6]);
  assert.equal(albums.filter((a) => a.title === 'Best of').length, 2);
});

test('songs without an album are not an album; albums sort the Vietnamese way', () => {
  const titles = groupAlbums(songs, 'vi').map((a) => `${a.title}/${a.artist}`);
  assert.deepEqual(titles, ['Best of/A', 'Best of/B', 'Diễm xưa/Khánh Ly', 'Mùa thu/Ngọc Lan', 'Tình ca/Nhiều ca sĩ']);
  assert.equal(groupAlbums(songs, 'vi').find((a) => a.title === 'Mùa thu').coverId, 3);
  assert.equal(groupAlbums(songs, 'vi').find((a) => a.title === 'Best of').coverId, null);
});

test('artists list their songs and albums', () => {
  const artists = groupArtists(songs, 'vi');
  const kl = artists.find((a) => a.name === 'Khánh Ly');
  assert.deepEqual([kl.count, kl.albums, kl.coverId], [2, 1, 2]);
  assert.deepEqual(artists.map((a) => a.name), ['A', 'B', 'Khánh Ly', 'Ngọc Lan', 'X', 'Y', 'Z']);
});

test('sidebar stats', () => {
  assert.deepEqual(libraryStats(songs), { songs: 8, liked: 2 });
});

test('titles split in two for the light / bold display', () => {
  assert.deepEqual(splitTitle('Mùa thu cho em'), ['Mùa thu', 'cho em']);
  assert.deepEqual(splitTitle('Diễm xưa'), ['Diễm', 'xưa']);
  assert.deepEqual(splitTitle('Nối vòng tay'), ['Nối vòng', 'tay']);
  assert.deepEqual(splitTitle('Dạ'), ['', 'Dạ']);
  assert.deepEqual(splitTitle('夜に駆ける'), ['', '夜に駆ける'], 'no spaces: one line');
});

import { syncRuleFor, hasSyncRule } from '../../src/js/library.js';

test('an artist or album page names the sync rule it would turn on', () => {
  assert.deepEqual(syncRuleFor({ kind: 'artist', title: 'Hà Nhi', artist: '' }), { kind: 'artist', artist: 'Hà Nhi', album: null });
  assert.deepEqual(syncRuleFor({ kind: 'album', title: 'Tình ca', artist: 'Khánh Ly' }), { kind: 'album', artist: 'Khánh Ly', album: 'Tình ca' });
  assert.equal(syncRuleFor({ kind: 'album', title: 'Tình ca', artist: '' }), null, 'albums without an artist cannot be matched');
  assert.equal(syncRuleFor(null), null);
});

test('a page shows whether its rule is on', () => {
  const rules = [{ kind: 'album', artist: 'Khánh Ly', album: 'Tình ca' }, { kind: 'artist', artist: 'Hà Nhi', album: null }];
  assert.equal(hasSyncRule(rules, { kind: 'artist', artist: 'Hà Nhi', album: null }), true);
  assert.equal(hasSyncRule(rules, { kind: 'album', artist: 'Khánh Ly', album: 'Tình ca' }), true);
  assert.equal(hasSyncRule(rules, { kind: 'artist', artist: 'Khánh Ly', album: null }), false);
  assert.equal(hasSyncRule(rules, null), false);
});
