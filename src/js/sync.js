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

/** Progress text for one stage of a sync through iTunes (S.3); `percent` only while songs are handed over. */
export function stageText(s) {
  switch (s?.stage) {
    case 'CopyOff': return { text: t('sync.stage.copyOff', { n: s.songs }), percent: null }; // i18n-ignore: stage name from Rust
    case 'Apply': return { text: t('sync.stage.apply', { done: s.done, total: s.total }), percent: Math.round((s.done / s.total) * 100) }; // i18n-ignore: stage name from Rust
    case 'Check': return { text: t('sync.stage.check'), percent: null }; // i18n-ignore: stage name from Rust
    case 'UpdateDevice': return { text: t('sync.stage.updateDevice'), percent: null }; // i18n-ignore: stage name from Rust
    case 'Waiting': return { text: t('sync.stage.waiting'), percent: null }; // i18n-ignore: stage name from Rust
    case 'Verify': return { text: t('sync.stage.verify'), percent: null }; // i18n-ignore: stage name from Rust
    default: return { text: t('sync.stage.backup'), percent: null };
  }
}

/** What a finished sync changed, and anything worth a look (`tone` 'warn'). */
export function syncResult(r) {
  const lines = [t('sync.done', { added: r.added, removed: r.removed })];
  if (r.adopted > 0) lines.push(t('sync.adopted', { n: r.adopted }));
  if (r.failed.length) lines.push(t('sync.failedSongs', { n: r.failed.length, reason: r.failed[0][1] }));
  for (const s of r.unexpectedRemoved) lines.push(t('sync.lost', { title: s.title }));
  lines.push(...r.warnings);
  if (!r.confirmed) lines.push(t('sync.unconfirmed'));
  const warn = r.failed.length > 0 || r.unexpectedRemoved.length > 0 || r.warnings.length > 0 || !r.confirmed;
  return { tone: warn ? 'warn' : 'ok', lines };
}

/** A sync that stopped: before changing anything (`blocked`), or on an error. */
export function syncStopped(err) {
  if (err?.kind === 'blocked') return { title: t('sync.blocked'), lines: err.reasons };
  return { title: t('sync.failed'), lines: [err?.reason ?? String(err)] };
}
