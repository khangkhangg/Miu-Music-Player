// Watched folders in the sidebar and the library summary (F3.1).
import { t } from './i18n.js';

export function folderItems(folders) {
  return (folders ?? []).map(f => {
    const parts = f.path.split('\\').filter(Boolean);
    const name = /^[A-Za-z]:\\?$/.test(f.path) ? f.path : parts[parts.length - 1] ?? f.path;
    return { id: f.id, name, title: f.path, count: f.songCount };
  });
}

export function librarySummary(folders) {
  if (!folders?.length) return null;
  const songs = folders.reduce((sum, f) => sum + f.songCount, 0);
  return t('library.summary', { songs: t('count.songs', { n: songs }), folders: t('count.folders', { n: folders.length }) });
}
