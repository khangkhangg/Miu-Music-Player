import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { useDictionary } from '../../src/js/i18n.js';
import { BOOK_LANGUAGES, bookMeta, bookAction, bookProgress, markAdded } from '../../src/js/audiobooks.js';

const root = path.resolve(import.meta.dirname, '../..');
const en = JSON.parse(fs.readFileSync(path.join(root, 'src/i18n/en.json'), 'utf8'));
const vi = JSON.parse(fs.readFileSync(path.join(root, 'src/i18n/vi.json'), 'utf8'));

test('a catalogue book reads as author, length and chapters', () => {
  useDictionary('en', en);
  const b = { author: 'Robert Louis Stevenson', durationMs: 27_008_000, chapters: 26, language: 'English' };
  assert.equal(bookMeta(b), 'Robert Louis Stevenson · 7 h 30 min · 26 chapters');
  assert.equal(bookMeta({ author: '', durationMs: null, chapters: 1, language: 'Spanish' }), '1 chapter · Spanish', 'the language when it is not English');
  useDictionary('vi', vi);
  assert.equal(bookMeta(b), 'Robert Louis Stevenson · 7 giờ 30 phút · 26 chương');
  useDictionary('en', en);
});

test('a book in the library resumes, starts, or starts over', () => {
  assert.equal(bookAction({ chapters: 3, finished: 0, resumeChapter: 7, resumeMs: 0, downloaded: 3 }), 'start');
  assert.equal(bookAction({ chapters: 3, finished: 1, resumeChapter: 8, resumeMs: 0, downloaded: 3 }), 'resume', 'next chapter after a finished one');
  assert.equal(bookAction({ chapters: 3, finished: 0, resumeChapter: 7, resumeMs: 60_000, downloaded: 3 }), 'resume');
  assert.equal(bookAction({ chapters: 3, finished: 3, resumeChapter: null, resumeMs: 0, downloaded: 3 }), 'again');
  assert.equal(bookAction({ chapters: 3, finished: 0, resumeChapter: 7, resumeMs: 0, downloaded: 0 }), 'download', 'nothing on this PC yet');
});

test('download progress per book', () => {
  useDictionary('en', en);
  assert.equal(bookProgress({ chapters: 26, downloaded: 26, finished: 2 }), '2 of 26 chapters listened');
  assert.equal(bookProgress({ chapters: 26, downloaded: 12, finished: 0 }), '12 of 26 chapters on this PC');
  useDictionary('vi', vi);
  assert.equal(bookProgress({ chapters: 26, downloaded: 12, finished: 0 }), '12/26 chương đã tải về máy');
  useDictionary('en', en);
});

test('books already in the library are marked by their chapter feed', () => {
  const found = [{ id: 449, rssUrl: 'https://librivox.org/rss/449' }, { id: 59, rssUrl: 'https://librivox.org/rss/59' }];
  assert.deepEqual(markAdded(found, [{ feedUrl: 'https://librivox.org/rss/449' }]).map((b) => b.added), [true, false]);
});

test('browse by language offers English and Vietnamese first, each translated', () => {
  assert.deepEqual(BOOK_LANGUAGES.slice(0, 3), ['', 'English', 'Vietnamese']);
  for (const l of BOOK_LANGUAGES) {
    const key = `book.lang.${l || 'any'}`;
    assert.ok(en[key] && vi[key], `${key} in en and vi`);
  }
});
