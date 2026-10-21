// Output panel (F7.8): what the file is, where the sound goes, and what happens on the way.

import { t } from './i18n.js';

const khz = (hz) => String(Math.round(hz / 100) / 10);

export function rateLabel(hz) {
  return `${khz(hz)} kHz`;
}

/** "FLAC · 24-bit / 96 kHz" from the codec and the decoder's source format. */
export function sourceLabel(codec, source) {
  if (!source) return '';
  const depth = source.bits ? `${source.bits}-bit / ` : '';
  const parts = [String(codec || '').toUpperCase(), `${depth}${rateLabel(source.rate)}`];
  if (source.channels === 1) parts.push(t('output.mono'));
  return parts.filter(Boolean).join(' · ');
}

/** True when the EQ actually changes the sound (on and not flat). */
export function eqActive(eq) {
  if (!eq?.enabled) return false;
  return eq.preampDb !== 0 || eq.gainsDb.some((g) => g !== 0);
}

/**
 * "Bit-perfect" only with exclusive mode, EQ off, no resampling, full volume and enough bits;
 * otherwise what happens to the sound on the way.
 */
export function signalPath({ sourceRate, deviceRate, eqActive: eq, exclusive, volume = 1, sourceBits = null, deviceBits = 0 }) {
  const resampled = sourceRate && deviceRate && sourceRate !== deviceRate;
  const quieter = exclusive && volume < 1;
  const truncated = exclusive && sourceBits && deviceBits && deviceBits < sourceBits;
  if (exclusive && !eq && !resampled && !quieter && !truncated) return t('output.bitPerfect');
  const parts = [t(exclusive ? 'output.exclusiveMode' : 'output.shared')];
  if (resampled) parts.push(t('output.resampled', { from: khz(sourceRate), to: rateLabel(deviceRate) }));
  if (eq) parts.push(t('output.eqOn'));
  if (quieter) parts.push(t('output.volume', { percent: Math.round(volume * 100) }));
  if (truncated) parts.push(t('output.reduced', { bits: deviceBits }));
  return parts.join(' · ');
}

/** Options for the device menu: "System default (…)" first, then every device. */
export function deviceOptions({ devices, chosen }) {
  const def = devices.find((d) => d.isDefault);
  const first = { value: '', label: def ? t('output.systemDefaultNamed', { name: def.name }) : t('output.systemDefault'), selected: !chosen };
  return [first, ...devices.map((d) => ({ value: d.name, label: d.name, selected: d.name === chosen }))];
}

/** The first output's device: the chosen one, else the system default. */
function firstOutput({ devices, chosen }) {
  return chosen ?? devices.find((d) => d.isDefault)?.name ?? null;
}

/**
 * Options for the second output menu (F7.9): "None" first, then every device but the first output
 * (the chosen one, and `playing`: the device the first output really uses, if it fell back).
 */
export function secondOptions({ devices, chosen, second }, playing = null) {
  const first = firstOutput({ devices, chosen });
  const opts = [{ value: '', label: t('output.secondNone'), selected: !second }];
  for (const d of devices) {
    if (d.name !== first && d.name !== playing) opts.push({ value: d.name, label: d.name, selected: d.name === second });
  }
  if (second && !devices.some((d) => d.name === second)) {
    opts.push({ value: second, label: t('output.notConnected', { name: second }), selected: true });
  }
  return opts;
}

const signed = (n) => `${n < 0 ? '−' : '+'}${Math.abs(Math.round(n))}`;

/** One line on how the two outputs are doing, from the player event. */
export function dualStatus({ second, secondLost }) {
  if (!second) return secondLost ? t('output.secondLost') : '';
  const clock = signed(second.driftPpm);
  if (Math.abs(second.offsetMs) < 2) return t('output.inStep', { ppm: clock });
  const ms = Math.round(Math.abs(second.offsetMs));
  return t(second.offsetMs < 0 ? 'output.behind' : 'output.ahead', { ms, ppm: clock });
}
