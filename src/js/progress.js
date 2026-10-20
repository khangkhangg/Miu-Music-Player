// Background work (scanning folders, fingerprinting songs) shown as a thin progress bar.
import { t } from './i18n.js';

const LABELS = { scan: 'work.scan', fingerprint: 'work.fingerprint', import: 'work.import' };

/** Whether to show the bar, how full it is, and what it says. */
export function workState(p) {
  if (!p || !(p.total > 0) || p.done >= p.total) return { show: false, percent: 100, text: '' };
  return {
    show: true,
    percent: Math.round((p.done / p.total) * 100),
    text: t(LABELS[p.phase] ?? 'work.scan', { done: p.done, total: p.total }),
  };
}
