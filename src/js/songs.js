// Song list helpers (F5.2): row text and virtual-scroll math. DOM code lives in app.js.
// Strings move to src/i18n in F8.6.

export function formatDuration(ms) {
  if (ms == null) return '';
  const total = Math.floor(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, '0');
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${s}` : `${m}:${s}`;
}

export function songRow(song) {
  return {
    id: song.id,
    title: song.title,
    artist: song.artist || 'Unknown artist',
    album: song.album || '',
    duration: formatDuration(song.durationMs),
  };
}

/** Rows to render for a fixed-height list: the visible ones plus `overscan` on each side. */
export function visibleRange({ scrollTop, height, rowHeight, total, overscan }) {
  const first = Math.floor(scrollTop / rowHeight);
  const count = Math.ceil(height / rowHeight);
  const start = Math.max(0, Math.min(first - overscan, total));
  const end = Math.max(start, Math.min(total, first + count + overscan));
  return { start, end };
}
