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

/** The light show `sinceMs` after the engine reported `show`: a pulse (1 on the beat, fading) and whether this beat starts a bar. */
export function beatLight(show, sinceMs) {
  if (!show?.beatS) return null;
  const beat = show.beatPos + sinceMs / 1000 / show.beatS;
  const phase = beat - Math.floor(beat + 1e-9);
  return { pulse: Math.exp(-phase * 4), bar: ((Math.floor(beat + 1e-9) % 4) + 4) % 4 === 0 };
}

/** The countdown into a built-up drop: bars left (4 down to 1), then "drop" for a beat after it lands. */
export function dropCountdown(show, sinceMs) {
  if (!show?.beatS || show.dropInS == null) return null;
  const left = show.dropInS - sinceMs / 1000;
  const bar = 4 * show.beatS;
  if (left > 0 && left <= 4 * bar) return { bars: Math.ceil(left / bar - 1e-9) };
  if (left <= 0 && left > -2 * show.beatS) return { drop: true };
  return null;
}

/** A sing-along `sinceMs` after the engine reported `show`: the line to sing now and the next one, or "sing it" without timed lyrics. */
export function singLines(show, sinceMs, lyrics) {
  if (!show?.sing) return null;
  const lines = lyrics?.synced ?? [];
  if (!lines.length) return { singIt: true };
  const ms = (show.posS + (sinceMs / 1000) * (show.speed || 1)) * 1000;
  let i = -1;
  while (i + 1 < lines.length && lines[i + 1].ms <= ms) i += 1;
  if (i < 0) return { now: '', next: lines[0].text };
  return { now: lines[i].text, next: lines[i + 1]?.text ?? '' };
}

/** The guest whose song is playing, for a shout-out on the party screen (none when the DJ picked it). */
export function shoutOut(view) {
  const name = view?.now?.addedName?.trim();
  return name || null;
}
