// Audiobooks (Phase 8): labels for LibriVox books and their place in the library. DOM glue lives in app.js.
import { t } from './i18n.js';
import { lengthLabel } from './podcasts.js';

/** Languages to browse by, as LibriVox names them ('' = any). */
export const BOOK_LANGUAGES = ['', 'English', 'Vietnamese', 'Spanish', 'French', 'German', 'Italian', 'Portuguese', 'Dutch', 'Russian', 'Chinese', 'Japanese']; // i18n-ignore: catalogue names, shown via book.lang.*

/** A LibriVox language in the UI language (the catalogue's own name when Miu has none). */
export function languageName(l) {
  const key = `book.lang.${l || 'any'}`;
  const s = t(key);
  return s === key ? l : s;
}

/** "Robert Louis Stevenson · 7 h 30 min · 26 chapters", plus the language when it is not English. */
export function bookMeta(b) {
  const parts = [];
  if (b.author) parts.push(b.author);
  const len = lengthLabel(b.durationMs);
  if (len) parts.push(len);
  if (b.chapters) parts.push(t('book.chapters', { n: b.chapters }));
  if (b.language && b.language !== 'English') parts.push(languageName(b.language)); // i18n-ignore: catalogue name
  return parts.join(' · ');
}

/** What the book's main button does: 'download', 'start', 'resume' or 'again' (all finished). */
export function bookAction(b) {
  if (!b.downloaded) return 'download';
  if (b.resumeChapter == null) return 'again';
  return b.finished > 0 || b.resumeMs > 5000 ? 'resume' : 'start';
}

/** "12 of 26 chapters on this PC" while downloading, then "2 of 26 chapters listened". */
export function bookProgress(b) {
  return b.downloaded < b.chapters
    ? t('book.downloadedOf', { done: b.downloaded, n: b.chapters })
    : t('book.listenedOf', { done: b.finished, n: b.chapters });
}

/** Mark catalogue results that are already in the library (same chapter feed). */
export function markAdded(found, books) {
  const feeds = new Set(books.map((b) => b.feedUrl));
  return found.map((f) => ({ ...f, added: feeds.has(f.rssUrl) }));
}
