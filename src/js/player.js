// Now-playing bar (F7.1). `songsById` holds raw song rows (durationMs, artist may be null).
import { formatDuration } from './songs.js';

export function nowPlaying(event, songsById) {
  const song = event?.songId != null ? songsById.get(event.songId) : null;
  if (!song) return null;
  const total = song.durationMs;
  return {
    title: song.title,
    artist: song.artist || 'Unknown artist',
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
