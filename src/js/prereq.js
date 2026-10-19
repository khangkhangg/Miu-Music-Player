// User-facing message for the Apple driver check (F0.3).
import { t } from './i18n.js';
export function prereqMessage(report) {
  if (!report || report.ready) return null;
  return {
    title: t('prereq.title'),
    body: t('prereq.body'),
    detail: t('prereq.missing', { list: report.missing.join(', ') }),
  };
}
