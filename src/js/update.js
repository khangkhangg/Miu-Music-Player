// In-app updates: the banner when a new version is out, the download bar and "Check now".
import { t } from './i18n.js';
import { formatBytes } from './device.js';

const parts = v => (String(v ?? '').replace(/^v/, '').match(/^\d+(\.\d+)*/)?.[0] ?? '').split('.').filter(Boolean).map(Number);

/** Whether version `latest` is newer than `current` (x.y.z, compared part by part). */
export function isNewer(latest, current) {
  const a = parts(latest);
  const b = parts(current);
  if (!a.length) return false;
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const d = (a[i] ?? 0) - (b[i] ?? 0);
    if (d) return d > 0;
  }
  return false;
}

/** The "new version" banner for what the last check found. */
export function bannerState(info) {
  if (!info || !isNewer(info.version, info.current)) return { show: false, title: '', notes: '' };
  return { show: true, title: t('update.available', { version: info.version }), notes: info.notes ?? '' };
}

/** The download bar while an update comes in. */
export function downloadState(p) {
  const known = p.total > 0;
  return {
    percent: known ? Math.min(100, Math.round((p.done / p.total) * 100)) : 0,
    text: known
      ? t('update.downloading', { done: formatBytes(p.done), total: formatBytes(p.total) })
      : t('update.downloadingUnknown', { done: formatBytes(p.done) }),
  };
}

/** What "Check now" found: an update, nothing, or an error. */
export function checkResultText(result) {
  if (result instanceof Error || typeof result === 'string') return t('update.checkFailed', { error: result.message ?? result });
  return result ? t('update.available', { version: result.version }) : t('update.upToDate');
}
