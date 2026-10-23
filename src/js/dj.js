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

// The DJ keyboard, by physical key (any layout): deck A under the left hand, deck B under the right.
const DECK_KEYS = {
  A: { play: 'KeyZ', cue: 'KeyX', sync: 'KeyC', hot: ['Digit1', 'Digit2', 'Digit3', 'Digit4'], loop4: 'KeyQ', loop8: 'KeyW', loopOff: 'KeyE', echo: 'KeyA', sweep: 'KeyS', spin: 'KeyD' },  // i18n-ignore
  B: { play: 'KeyM', cue: 'KeyN', sync: 'KeyB', hot: ['Digit7', 'Digit8', 'Digit9', 'Digit0'], loop4: 'KeyU', loop8: 'KeyI', loopOff: 'KeyO', echo: 'KeyJ', sweep: 'KeyK', spin: 'KeyL' },  // i18n-ignore
};
const PAD_KEYS = ['F1', 'F2', 'F3', 'F4'];

/**
 * The DJ action for a key (`code`, `shiftKey`), as { down, up? } (up: for keys held like Cue),
 * or null. `state` is the engine state, for toggles and the crossfader.
 */
export function keyAction({ code, shiftKey }, state) {
  for (const [i, deck] of ['A', 'B'].entries()) {
    const k = DECK_KEYS[deck];
    const d = state?.decks?.[i] ?? {};
    const hot = k.hot.indexOf(code);
    if (hot >= 0) return { down: { type: shiftKey ? 'clearHotCue' : 'hotCue', deck, slot: hot } };
    switch (code) {
      case k.play: return { down: { type: 'playPause', deck } };
      case k.cue: return { down: { type: 'cue', deck, pressed: true }, up: { type: 'cue', deck, pressed: false } };
      case k.sync: return { down: { type: 'sync', deck } };
      case k.loop4: return { down: { type: 'loop', deck, beats: 4 } };
      case k.loop8: return { down: { type: 'loop', deck, beats: 8 } };
      case k.loopOff: return { down: { type: 'loop', deck, beats: null } };
      case k.echo: return { down: { type: 'effect', deck, effect: 'echo', on: !d.echo } };
      case k.sweep: return { down: { type: 'effect', deck, effect: 'filterSweep', on: !d.sweep } };
      case k.spin: return { down: { type: 'effect', deck, effect: 'spinBack', on: true } };
      default:
    }
  }
  const pad = PAD_KEYS.indexOf(code);
  if (pad >= 0) return { down: { type: 'pad', slot: pad } };
  if (code === 'ArrowLeft' || code === 'ArrowRight') {
    const x = (state?.crossfader ?? 0) + (code === 'ArrowLeft' ? -0.1 : 0.1);
    return { down: { type: 'crossfader', value: Math.round(Math.min(1, Math.max(-1, x)) * 100) / 100 } };
  }
  return null;
}

/** "Analysing songs: 3 of 10" while the library is being analysed. */
export function analysisText(p) {
  if (!p || p.done >= p.total) return '';
  return t('dj.analysing', { done: p.done, total: p.total });
}

/** Length of the deck's loop in beats of the song (the tempo control does not change it). */
export function loopBeats(deck) {
  if (!deck?.loopS || !deck.bpm) return null;
  const songBpm = deck.bpm / (1 + deck.tempo * (TEMPO_RANGE / 100));
  return Math.round(((deck.loopS[1] - deck.loopS[0]) * songBpm) / 60);
}

const LEARN = {
  play: 'playPause', cue: 'cue', sync: 'sync', tempo: 'tempo', jog: 'jog', filter: 'filter', gain: 'gain',
  volume: 'volume', phones: 'headphoneCue', xfader: 'crossfader', master: 'masterVolume', cueMix: 'cueMix',
  phonesVolume: 'headphoneVolume',
};
const BANDS = { high: 'eqHigh', mid: 'eqMid', low: 'eqLow' };

/** MIDI learn: the controller control a screen element stands for (from its data attributes), or null. */
export function learnControl(data, deck) {
  let control = null;
  if (data.ctl === 'eq') control = BANDS[data.band];
  else if (data.ctl) control = LEARN[data.ctl];
  else if (data.slot != null) control = `hotCue${Number(data.slot) + 1}`;
  else if (data.loop === '4') control = 'loop4';
  else if (data.loop === 'off') control = 'loopExit';
  else if (data.fx === 'echo') control = 'fxOn';
  return control ? { control, deck: deck ?? null } : null;
}

/** The song list selection after a browse knob turned `steps`, kept inside `count` rows (-1: none). */
export function moveSelection(index, steps, count) {
  if (count <= 0) return -1;
  const from = index < 0 ? (steps > 0 ? -1 : 0) : index;
  return Math.min(count - 1, Math.max(0, from + steps));
}
