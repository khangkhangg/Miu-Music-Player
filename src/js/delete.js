// Confirmation and result text for deleting disliked songs (F7.3). Strings move to src/i18n in F8.6.

const plural = (n) => `${n} song${n === 1 ? '' : 's'}`;

export function deletePrompt(songs) {
  const names = songs.slice(0, 3).map(s => s.title).join(', ');
  const more = songs.length > 3 ? ` and ${songs.length - 3} more` : '';
  return {
    title: `Delete ${plural(songs.length)}?`,
    message: `${names}${more} will be moved to the Recycle Bin and removed from Miu Music. `
      + 'You can restore files from the Recycle Bin.',
  };
}

export function deleteResult(report) {
  const moved = `Moved ${plural(report.deleted.length)} to the Recycle Bin.`;
  if (!report.failed.length) return moved;
  const reasons = [...new Set(report.failed.map(([, why]) => why))].join('; ');
  return `${moved} ${report.failed.length} could not be moved: ${reasons}`;
}
