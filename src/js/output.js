// Output panel (F7.8): what the file is, where the sound goes, and what happens on the way.

const khz = (hz) => String(Math.round(hz / 100) / 10);

export function rateLabel(hz) {
  return `${khz(hz)} kHz`;
}

/** "FLAC · 24-bit / 96 kHz" from the codec and the decoder's source format. */
export function sourceLabel(codec, source) {
  if (!source) return '';
  const depth = source.bits ? `${source.bits}-bit / ` : '';
  const parts = [String(codec || '').toUpperCase(), `${depth}${rateLabel(source.rate)}`];
  if (source.channels === 1) parts.push('mono');
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
  if (exclusive && !eq && !resampled && !quieter && !truncated) return 'Bit-perfect';
  const parts = [exclusive ? 'Exclusive mode' : 'Shared mode (Windows mixer)'];
  if (resampled) parts.push(`resampled ${khz(sourceRate)} → ${rateLabel(deviceRate)}`);
  if (eq) parts.push('EQ on (32-bit float)');
  if (quieter) parts.push(`volume ${Math.round(volume * 100)}%`);
  if (truncated) parts.push(`reduced to ${deviceBits}-bit`);
  return parts.join(' · ');
}

/** Options for the device menu: "System default (…)" first, then every device. */
export function deviceOptions({ devices, chosen }) {
  const def = devices.find((d) => d.isDefault);
  const first = { value: '', label: def ? `System default (${def.name})` : 'System default', selected: !chosen };
  return [first, ...devices.map((d) => ({ value: d.name, label: d.name, selected: d.name === chosen }))];
}
