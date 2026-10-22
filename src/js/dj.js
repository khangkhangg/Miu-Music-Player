// DJ mode screen (D.4): readouts, knob maths, meters, waveform, what fits the song on air.

import { t } from './i18n.js';
import { fold } from './search.js';

const TEMPO_RANGE = 8; // percent either way, as in the engine

export function bpmLabel(bpm) {
  return bpm == null ? '--' : bpm.toFixed(1);
}

/** Tempo control -1..1 as a percentage: "+4.0 %". */
export function tempoLabel(tempo) {
  const p = tempo * TEMPO_RANGE;
  return `${p > 0 ? '+' : ''}${p.toFixed(1)} %`;
}

function clock(s) {
  const n = Math.max(0, Math.floor(s));
  return `${Math.floor(n / 60)}:${String(n % 60).padStart(2, '0')}`;
}

/** "-3:10" until the end of the song on a deck. */
export function timeLeft(deck) {
  if (!deck?.durationS) return '';
  return `-${clock(deck.durationS - deck.positionS)}`;
}

/** A knob dragged `dy` pixels up (negative: down); its whole travel is `pixels`. */
export function knobDrag(value, dy, { min = -1, max = 1, pixels = 150 } = {}) {
  const v = value + (dy / pixels) * (max - min);
  return Math.min(max, Math.max(min, Math.round(v * 1000) / 1000));
}

/** Arrow keys turn a knob a step; Home puts it back. Null for other keys. */
export function knobKey(value, key, { min = -1, max = 1, step = 0.05, reset = 0 } = {}) {
  const clamp = (v) => Math.min(max, Math.max(min, v));
  if (key === 'ArrowUp' || key === 'ArrowRight') return clamp(value + step);
  if (key === 'ArrowDown' || key === 'ArrowLeft') return clamp(value - step);
  if (key === 'Home') return reset;
  return null;
}

/** Turns of the platter between two pointer positions round `center` (clockwise positive). */
export function jogTurns(from, to, center) {
  const angle = (p) => Math.atan2(p.y - center.y, p.x - center.x);
  let d = angle(to) - angle(from);
  if (d > Math.PI) d -= 2 * Math.PI;
  if (d < -Math.PI) d += 2 * Math.PI;
  return d / (2 * Math.PI);
}

/** Meter level (peak, 1 = full scale) as a height 0..1 over the top 48 dB. */
export function meterHeight(level) {
  if (!(level > 0)) return 0;
  const db = 20 * Math.log10(level);
  return Math.min(1, Math.max(0, (db + 48) / 48));
}

/** SVG path of the whole song: the peaks above the middle line and mirrored below. */
export function waveformPath(peaks, width, height) {
  if (!peaks?.length) return '';
  const mid = height / 2;
  const n = peaks.length;
  const x = (i) => (n === 1 ? 0 : Math.round((i * width) / (n - 1) * 10) / 10);
  const h = (v) => Math.round((v / 255) * mid * 10) / 10;
  const top = peaks.map((v, i) => `${x(i)},${mid - h(v)}`);
  const bottom = peaks.map((v, i) => `${x(i)},${mid + h(v)}`).reverse();
  return `M${top.join('L')}L${bottom.join('L')}Z`;
}

function parseKey(k) {
  const m = /^(\d{1,2})([AB])$/.exec(k ?? '');
  return m ? { n: Number(m[1]), letter: m[2] } : null;
}

/** Camelot keys that mix well: the same, one step round the wheel, or the relative major/minor. */
export function keysCompatible(a, b) {
  const x = parseKey(a);
  const y = parseKey(b);
  if (!x || !y) return false;
  if (x.letter !== y.letter) return x.n === y.n;
  const d = Math.abs(x.n - y.n);
  return d === 0 || d === 1 || d === 11;
}

function tempoReachable(from, to) {
  if (!(from > 0) || !(to > 0)) return false;
  return [1, 2, 0.5].some((per) => Math.abs(from / (to * per) - 1) <= TEMPO_RANGE / 100);
}

/** True when `song` can follow `air` with a beat blend: tempo in reach and a compatible key. */
export function fits(air, song) {
  if (!air || !song) return false;
  return tempoReachable(air.bpm, song.bpm) && keysCompatible(air.key, song.key);
}

/** "124 BPM · 8A · Energy 7". */
export function songMeta(a) {
  if (!a) return '';
  return [`${Math.round(a.bpm)} BPM`, a.key, t('dj.energy', { n: a.energy })].filter(Boolean).join(' · '); // i18n-ignore
}

const TRANSITIONS = { beatBlend: 'dj.tr.beatBlend', echoOut: 'dj.tr.echoOut', crossfade: 'dj.tr.crossfade', cut: 'dj.tr.cut' };

/** "Beat blend (16 bars) in 0:42". */
export function nextText(next) {
  if (!next) return '';
  const what = t(TRANSITIONS[next.transition.type], { bars: next.transition.bars ?? 0 });
  return next.startsInS < 0.5 ? t('dj.tr.now', { what }) : t('dj.tr.in', { what, time: clock(next.startsInS) });
}

/** Crossfader position that puts only this deck on air. */
export function crossfaderFor(deck) {
  return deck === 'A' ? -1 : 1;
}

const HOLD_MS = 2000;

/** How far a hold on the guest lock has got (1 = unlocked). */
export function holdProgress(ms) {
  return Math.min(1, Math.max(0, ms / HOLD_MS));
}

/**
 * Songs for the DJ list: matching the search, only those that fit the song on air when asked,
 * songs already played this session at the end.
 */
export function filterSongsForDj(songs, analyses, { query, fitsWith, played }) {
  const words = fold(query).split(/\s+/).filter(Boolean);
  const list = songs.filter((s) => {
    if (words.length) {
      const k = fold(`${s.title ?? ''}\n${s.artist ?? ''}`);
      if (!words.every((w) => k.includes(w))) return false;
    }
    return !fitsWith || fits(fitsWith, analyses.get(s.id));
  });
  return [...list.filter((s) => !played.has(s.id)), ...list.filter((s) => played.has(s.id))];
}

/** "Analysing songs: 3 of 10" while the library is being analysed. */
export function analysisText(p) {
  if (!p || p.done >= p.total) return '';
  return t('dj.analysing', { done: p.done, total: p.total });
}
