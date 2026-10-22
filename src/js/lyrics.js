// Lyrics view helpers (L): which line is being sung, and a note on where the lyrics came from.

import { t } from './i18n.js';

/** Index of the line being sung at `ms` (the last one started), -1 before the first. */
export function lineAt(lines, ms) {
  let lo = 0;
  let hi = lines.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (lines[mid].ms <= ms) lo = mid + 1;
    else hi = mid;
  }
  return lo - 1;
}

/** Playback position now, from the last player event (they come every 250 ms or so). */
export function livePosition(ev, eventAt, now) {
  if (!ev) return 0;
  return ev.positionMs + (ev.playing ? Math.max(0, now - eventAt) : 0);
}

/** One line under the lyrics: where they came from, or why there are none. */
export function lyricsNote(lyrics) {
  if (!lyrics) return t('lyrics.loading');
  if (lyrics.instrumental && !lyrics.synced.length && !lyrics.plain) return t('lyrics.instrumental');
  if (lyrics.source === 'unreachable') return t('lyrics.unreachable');
  if (lyrics.source === 'none') return t('lyrics.none');
  const from = t(`lyrics.from.${lyrics.source}`);
  return lyrics.synced.length ? from : `${from} · ${t('lyrics.notTimed')}`;
}
