// Music players (USB drives, memory cards) in the sidebar: card text, per-player options, sync progress.
import { t } from './i18n.js';
import { formatBytes } from './device.js';

const TARGETS = ['mp3V0', 'mp3320', 'aac256'];

/** Formats a player may play as they are, besides MP3 (by the library's format names). */
export const PLAYS = [
  { value: 'flac', label: 'FLAC' }, // i18n-ignore: format name
  { value: 'wav', label: 'WAV' }, // i18n-ignore: format name
  { value: 'aac', label: 'AAC / M4A' }, // i18n-ignore: format name
  { value: 'alac', label: 'ALAC' }, // i18n-ignore: format name
];

function drive(root) {
  return String(root ?? '').replace(/\\$/, '');
}

export function playerCard(p) {
  const letter = drive(p.root);
  const name = p.label?.trim() || t('player.usbDrive');
  const folder = p.profile?.folder ? `${letter}\\${p.profile.folder}` : `${letter}\\`;
  const total = p.totalBytes;
  return {
    title: `${name} (${letter})`,
    detail: t('player.folder', { path: folder }),
    storage: total ? { usedPercent: Math.round(((total - p.freeBytes) / total) * 100), label: t('device.storage', { free: formatBytes(p.freeBytes), total: formatBytes(total) }) } : null,
    suggested: !p.configured,
  };
}

export function targetOptions(selected) {
  return TARGETS.map(value => ({ value, label: t(`player.target.${value}`), selected: value === selected }));
}

/** A copy of `profile` with `format` ticked or not. */
export function withPlays(profile, format, on) {
  const plays = new Set(profile.plays ?? []);
  if (on) plays.add(format); else plays.delete(format);
  return { ...profile, plays: [...plays].sort() };
}

export function playerStageText(s) {
  if (s?.stage === 'copy') return { text: t('player.stage.copy', { done: s.done, total: s.total }), percent: s.total ? Math.round((s.done / s.total) * 100) : null };
  return { text: t('player.stage.check'), percent: null };
}

export function playerResult(r) {
  const lines = [t('player.done', { added: r.added, updated: r.updated + r.replaced, removed: r.removed })];
  if (r.goneFromPlayer > 0) lines.push(t('player.gone', { n: r.goneFromPlayer }));
  if (r.leftAlone > 0) lines.push(t('player.leftAlone', { n: r.leftAlone }));
  if (r.failed.length) lines.push(t('sync.failedSongs', { n: r.failed.length, reason: r.failed[0][1] }));
  return { tone: lines.length > 1 ? 'warn' : 'ok', lines };
}

export function playerStopped(err) {
  if (err?.kind === 'noSpace') return { title: t('sync.blocked'), lines: [t('player.noSpace', { need: formatBytes(err.need), available: formatBytes(err.available) })] };
  return { title: t('sync.failed'), lines: [err?.reason ?? String(err)] };
}
