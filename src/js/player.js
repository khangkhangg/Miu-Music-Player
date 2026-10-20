// Now-playing bar (F7.1). `songsById` holds raw song rows (durationMs, artist may be null).
import { formatDuration } from './songs.js';
import { t } from './i18n.js';

export function nowPlaying(event, songsById) {
  const song = event?.songId != null ? songsById.get(event.songId) : null;
  if (!song) return null;
  const total = song.durationMs;
  return {
    title: song.title,
    artist: song.artist || t('song.unknownArtist'),
    elapsed: formatDuration(event.positionMs),
    duration: formatDuration(total),
    progress: total ? Math.min(1, Math.max(0, event.positionMs / total)) : 0,
    playing: Boolean(event.playing),
  };
}

/** Position (ms) for a click at `fraction` of the progress bar. */
export function seekTarget(fraction, durationMs) {
  if (!durationMs) return null;
  return Math.round(Math.min(1, Math.max(0, fraction)) * durationMs);
}

/** Mini player pin (F7.5): state and the label for what a click will do. */
export function pinButton(onTop) {
  return { pressed: Boolean(onTop), label: t(onTop ? 'mini.pinOn' : 'mini.pinOff') };
}

const REPEATS = ['off', 'all', 'one'];

/** Repeat mode after a click: off → all → one → off. */
export function nextRepeat(mode) {
  const at = Math.max(0, REPEATS.indexOf(mode)); // unknown counts as off
  return REPEATS[(at + 1) % REPEATS.length];
}

/** State and label of the repeat button for `mode`. */
export function repeatButton(mode) {
  const m = REPEATS.includes(mode) ? mode : 'off';
  return { pressed: m !== 'off', one: m === 'one', label: t({ off: 'player.repeatOff', all: 'player.repeatAll', one: 'player.repeatOne' }[m]) };
}
