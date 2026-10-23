// Watched folders in the sidebar and the library summary (F3.1).
import { t } from './i18n.js';

export function folderItems(folders) {
  return (folders ?? []).map(f => {
    const parts = f.path.split('\\').filter(Boolean);
    const name = /^[A-Za-z]:\\?$/.test(f.path) ? f.path : parts[parts.length - 1] ?? f.path;
    return { id: f.id, name, title: f.path, count: f.songCount, encoding: f.tagEncoding ?? '', offline: f.online === false };
  });
}

export function librarySummary(folders) {
  if (!folders?.length) return null;
  const songs = folders.reduce((sum, f) => sum + f.songCount, 0);
  return t('library.summary', { songs: t('count.songs', { n: songs }), folders: t('count.folders', { n: folders.length }) });
}

// Legacy tag encodings a folder can be forced to (F8.1), most common for this app's users first.
const ENCODINGS = ['windows-1258', 'gbk', 'big5', 'shift_jis', 'euc-kr', 'windows-1251', 'windows-1252'];

/** Choices for a folder's tag encoding; '' = detect per song. */
export function encodingOptions(current) {
  return [{ value: '', key: 'encoding.auto' }, ...ENCODINGS.map((value) => ({ value, key: `encoding.${value}` }))]
    .map(({ value, key }) => ({ value, label: t(key), selected: value === (current ?? '') }));
}
