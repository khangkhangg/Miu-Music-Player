// Song list helpers (F5.2): row text and virtual-scroll math. DOM code lives in app.js.

import { rateLabel } from './output.js';
import { t } from './i18n.js';

const LOSSLESS = new Set(['flac', 'alac', 'wav', 'aiff', 'ape', 'wavpack']);
const DISPLAY = { vorbis: 'Ogg Vorbis', opus: 'Opus', wavpack: 'WavPack' }; // i18n-ignore: format names

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
    artist: song.artist || t('song.unknownArtist'),
    album: song.album || '',
    duration: formatDuration(song.durationMs),
    quality: qualityLabel(song),
  };
}

/** "FLAC · 24-bit / 96 kHz" for lossless files, "MP3 · 320 kbps" for lossy ones; '' when unknown. */
export function qualityLabel({ format, sampleRate, bitDepth, bitrateKbps }) {
  if (!format || format === 'unknown') return '';
  const name = DISPLAY[format] ?? format.toUpperCase();
  let detail = '';
  if (LOSSLESS.has(format)) {
    const rate = sampleRate ? rateLabel(sampleRate) : '';
    detail = bitDepth && rate ? `${bitDepth}-bit / ${rate}` : rate;
  } else if (bitrateKbps) {
    detail = `${bitrateKbps} kbps`;
  }
  return detail ? `${name} · ${detail}` : name;
}

/** Rows to render for a fixed-height list: the visible ones plus `overscan` on each side. */
export function visibleRange({ scrollTop, height, rowHeight, total, overscan }) {
  const first = Math.floor(scrollTop / rowHeight);
  const count = Math.ceil(height / rowHeight);
  const start = Math.max(0, Math.min(first - overscan, total));
  const end = Math.max(start, Math.min(total, first + count + overscan));
  return { start, end };
}

/** New scrollTop that brings row `index` into view (centred), or null when it is fully visible already. */
export function scrollToShow(index, { scrollTop, height, rowHeight }) {
  if (index < 0) return null;
  const top = index * rowHeight;
  if (top >= scrollTop && top + rowHeight <= scrollTop + height) return null;
  return Math.max(0, top - height / 2 + rowHeight / 2);
}
