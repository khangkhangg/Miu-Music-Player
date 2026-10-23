// Party mode on the DJ's screen (P.5): YouTube videos in YouTube's own embedded player, and how
// queue entries read.

import { t } from './i18n.js';

const YOUTUBE = 'https://www.youtube.com';

/** The embedded player for a video: plays at once and reports its state to `origin`. */
export function videoUrl(videoId, origin) {
  const u = new URL(`/embed/${encodeURIComponent(videoId)}`, YOUTUBE);
  u.searchParams.set('autoplay', '1');
  u.searchParams.set('enablejsapi', '1');
  u.searchParams.set('playsinline', '1');
  u.searchParams.set('rel', '0');
  u.searchParams.set('origin', origin);
  return u.toString();
}

/** What the page sends the player so it starts reporting (YouTube's iframe message protocol). */
export function listenMessage() {
  return JSON.stringify({ event: 'listening', id: 1, channel: 'widget' });
}

/** True when a message from the player says the video has ended (player state 0). */
export function videoEnded(message) {
  if (message?.origin !== YOUTUBE) return false;
  let data;
  try {
    data = typeof message.data === 'string' ? JSON.parse(message.data) : message.data;
  } catch {
    return false;
  }
  if (data?.event === 'onStateChange') return data.info === 0;
  if (data?.event === 'infoDelivery') return data.info?.playerState === 0;
  return false;
}

/** A queue entry as two lines and a vote count. */
export function entryLines(e) {
  const by = e.addedName ? t('party.addedBy', { name: e.addedName }) : '';
  return { title: e.title, sub: [e.artist, by].filter(Boolean).join(' · '), votes: e.votes, video: e.kind === 'video' };
}

/** How many songs one guest may have waiting. */
export function partyLimitOptions() {
  return [1, 2, 3, 5, 10].map((value) => ({ value, label: String(value) }));
}

/** The line under the relay switch: the guests' link, or why it is not working yet. */
export function relayNote(info) {
  if (!info.relayOn) return '';
  if (info.relayError === 'relayOff') return t('party.relayOff');
  if (info.relayError === 'offline') return t('party.relayOffline');
  if (info.relayError) return t('party.relayFailed', { code: info.relayError });
  return info.relayUrl ?? t('party.relayOpening');
}

/** The QR code on the party screen: the relay's works for everyone, the Wi-Fi one while it does not. */
export function partyQr(info) {
  if (info.relayOn && info.relayQr && !info.relayError) return info.relayQr;
  return info.localOn ? info.localQr ?? null : null;
}

/** What the party screen shows from the DJ's state: the song on air, the one on the other deck. */
export function nowAndNext(state, songById) {
  const decks = state?.decks ?? [];
  const i = state?.onAir ? ['A', 'B'].indexOf(state.onAir) : decks.findIndex((d) => d.playing);
  const d = decks[i];
  const now = d?.songId != null ? songById(d.songId) ?? null : null;
  if (!now) return { now: null, next: null, progress: 0, level: 0 };
  const other = decks[1 - i];
  return {
    now,
    next: other?.songId != null ? songById(other.songId) ?? null : null,
    progress: d.durationS ? Math.min(1, d.positionS / d.durationS) : 0,
    level: d.level ?? 0,
  };
}
