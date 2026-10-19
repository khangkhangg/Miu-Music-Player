// Watched folders in the sidebar and the library summary (F3.1). Strings move to src/i18n in F8.6.

export function folderItems(folders) {
  return (folders ?? []).map(f => {
    const parts = f.path.split('\\').filter(Boolean);
    const name = /^[A-Za-z]:\\?$/.test(f.path) ? f.path : parts[parts.length - 1] ?? f.path;
    return { id: f.id, name, title: f.path, count: f.songCount };
  });
}

const plural = (n, word) => `${n} ${word}${n === 1 ? '' : 's'}`;

export function librarySummary(folders) {
  if (!folders?.length) return null;
  const songs = folders.reduce((sum, f) => sum + f.songCount, 0);
  return `${plural(songs, 'song')} in ${plural(folders.length, 'folder')}`;
}
