// Background work (scanning folders, fingerprinting songs, importing and downloading podcasts)
// shown as a thin progress bar.
import { t } from './i18n.js';
import { formatBytes } from './device.js';

const LABELS = { scan: 'work.scan', fingerprint: 'work.fingerprint', import: 'work.import' };

function download(p) {
  if (!(p.done > 0)) return { show: false, percent: 100, text: '' };
  const known = p.total > 0;
  return {
    show: true,
    percent: known ? Math.round((p.done / p.total) * 100) : 0,
    text: known
      ? t('work.download', { title: p.title ?? '', done: formatBytes(p.done), total: formatBytes(p.total) })
      : t('work.downloadUnknown', { title: p.title ?? '', done: formatBytes(p.done) }),
  };
}

/** Whether to show the bar, how full it is, and what it says. */
export function workState(p) {
  if (p?.phase === 'download') return download(p);
  if (!p || !(p.total > 0) || p.done >= p.total) return { show: false, percent: 100, text: '' };
  return {
    show: true,
    percent: Math.round((p.done / p.total) * 100),
    text: t(LABELS[p.phase] ?? 'work.scan', { done: p.done, total: p.total }),
  };
}
