// Sorting and the index bar (F8.5): language-aware order from the WebView's ICU collator, and the
// letter each song files under. Vietnamese has its own letters (Ă Â Đ Ê Ô Ơ Ư); tones don't count.

import { fold } from './search.js';

const collators = new Map();

function collator(locale) {
  if (!collators.has(locale)) collators.set(locale, new Intl.Collator(locale, { numeric: true }));
  return collators.get(locale);
}

/**
 * The collation locale for these system languages. English and Vietnamese systems use Vietnamese
 * rules: English titles keep the usual order and Vietnamese ones sort correctly. (A language
 * setting arrives with F8.6.)
 */
export function sortLocale(languages) {
  const first = languages?.[0];
  return !first || /^(en|vi)\b/i.test(first) ? 'vi' : first;
}

/** A sorted copy: by title, then artist. */
export function sortSongs(list, locale) {
  const c = collator(locale);
  return [...list].sort((a, b) => c.compare(a.title ?? '', b.title ?? '') || c.compare(a.artist ?? '', b.artist ?? ''));
}

const VI_ALPHABET = new Set('AĂÂBCDĐEÊFGHIJKLMNOÔƠPQRSTUƯVWXYZ');
const VI_TONES = /[̣̀́̃̉]/g; // huyền, sắc, ngã, hỏi, nặng

/** The index-bar letter for a title: '#' for numbers, '…' for scripts without letters to index. */
export function indexLetter(title, locale) {
  const ch = String(title ?? '').match(/[\p{L}\p{N}]/u)?.[0];
  if (!ch) return '…';
  if (/\p{N}/u.test(ch)) return '#';
  if (/\p{Script=Latin}/u.test(ch)) {
    if (locale.startsWith('vi')) {
      const letter = ch.normalize('NFD').replace(VI_TONES, '').normalize('NFC').toUpperCase();
      if (VI_ALPHABET.has(letter)) return letter;
    }
    return fold(ch).toUpperCase();
  }
  // Other alphabets (Cyrillic, Greek…) index by their own capital letters.
  if (ch.toUpperCase() !== ch.toLowerCase()) return ch.toUpperCase();
  return '…';
}

/** Each letter once, at the position of its first song in the sorted list. */
export function indexEntries(sorted, locale) {
  const seen = new Set();
  const out = [];
  sorted.forEach((s, index) => {
    const letter = indexLetter(s.title, locale);
    if (!seen.has(letter)) {
      seen.add(letter);
      out.push({ letter, index });
    }
  });
  return out;
}
