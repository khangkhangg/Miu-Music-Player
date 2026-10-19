// Confirmation and result text for deleting disliked songs (F7.3).
import { t } from './i18n.js';

export function deletePrompt(songs) {
  const names = songs.slice(0, 3).map(s => s.title).join(', ');
  const listed = songs.length > 3 ? t('delete.andMore', { names, n: songs.length - 3 }) : names;
  return {
    title: t('delete.title', { n: songs.length }),
    message: t('delete.message', { names: listed }),
  };
}

export function deleteResult(report) {
  const moved = t('delete.moved', { n: report.deleted.length });
  if (!report.failed.length) return moved;
  const reasons = [...new Set(report.failed.map(([, why]) => why))].join('; ');
  return `${moved} ${t('delete.failed', { n: report.failed.length, reasons })}`;
}
