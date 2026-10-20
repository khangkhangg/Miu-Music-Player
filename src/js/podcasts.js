// Podcasts (Phase 7): labels for shows and episodes. DOM glue lives in app.js.
import { t } from './i18n.js';

/** "1 h 2 min" or "46 min" (at least 1 min); '' when unknown. */
export function lengthLabel(ms) {
  if (ms == null) return '';
  const minutes = Math.max(1, Math.round(ms / 60_000));
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return h ? t('podcast.hours', { h, m }) : t('podcast.minutes', { m });
}

/** "19 Oct 2026 · 1 h 2 min", in the UI language. */
export function episodeMeta(e, locale, timeZone) {
  const parts = [];
  if (e.published != null) {
    parts.push(new Intl.DateTimeFormat(locale, { day: 'numeric', month: 'short', year: 'numeric', timeZone }).format(new Date(e.published * 1000)));
  }
  const len = lengthLabel(e.durationMs);
  if (len) parts.push(len);
  return parts.join(' · ');
}

/** 'download', 'downloading' (asked for, not on disk yet) or 'downloaded'. */
export function episodeState(e, pending) {
  if (e.downloadPath) return 'downloaded';
  return pending.has(e.id) ? 'downloading' : 'download';
}

/** Mark Discover results already subscribed (same feed, or same name when a chart gave no feed). */
export function markSubscribed(found, shows) {
  const feeds = new Set(shows.map((s) => s.feedUrl));
  const names = new Set(shows.map((s) => s.title.trim().toLocaleLowerCase()));
  return found.map((f) => ({ ...f, subscribed: (f.feedUrl && feeds.has(f.feedUrl)) || names.has(f.title.trim().toLocaleLowerCase()) }));
}
