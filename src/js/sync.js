// Sync plan preview and progress text (F5.3, F5.4).
import { t } from './i18n.js';
import { formatBytes } from './device.js';

const ORDER = ['add', 'remove', 'replace', 'update'];

/** "+42 add · 3 remove · 1.2 GB", or "Up to date" for an empty plan. */
export function planSummary(plan) {
  const counts = Object.fromEntries(ORDER.map((k) => [k, 0]));
  for (const s of plan?.steps ?? []) counts[s.kind] += 1;
  const parts = ORDER.filter((k) => counts[k] > 0).map((k) => t(`sync.${k}`, { n: counts[k] }));
  if (!parts.length) return t('sync.upToDate');
  if (plan.addBytes > 0) parts.push(formatBytes(plan.addBytes));
  return parts.join(' · ');
}

/** Exponentially smoothed bytes per second (one slow or fast second moves it 20 %). */
export function smoothRate(previous, sample) {
  return previous == null ? sample : previous * 0.8 + sample * 0.2;
}

/** Seconds left, never negative; null while nothing has moved yet. */
export function eta(remainingBytes, bytesPerSecond) {
  if (!(bytesPerSecond > 0)) return null;
  return Math.max(0, Math.ceil(remainingBytes / bytesPerSecond));
}

export function etaLabel(seconds) {
  if (seconds == null) return '';
  return seconds < 60 ? t('sync.etaSeconds', { n: seconds }) : t('sync.etaMinutes', { n: Math.round(seconds / 60) });
}
