// Equalizer panel helpers (F7.7). Strings move to src/i18n in F8.6.

export function presetOptions(state) {
  const opts = [
    ...state.builtIn.map(p => ({ value: p.name, label: p.name, user: false })),
    ...state.user.map(p => ({ value: p.name, label: p.name, user: true })),
  ];
  return state.preset == null ? [{ value: '', label: 'Custom', user: false }, ...opts] : opts;
}

export function bandLabel(hz) {
  return hz >= 1000 ? `${hz / 1000}k` : String(hz);
}

export function dbLabel(db) {
  return db > 0 ? `+${db}` : String(db);
}

/** Settings with band `index` (or the preamp when index is -1) set to `db`. */
export function withGain(settings, index, db) {
  if (index < 0) return { ...settings, preampDb: db };
  const gainsDb = [...settings.gainsDb];
  gainsDb[index] = db;
  return { ...settings, gainsDb };
}

export function clipText(state) {
  if (!state.clipRisk) return null;
  const s = state.settings;
  const over = s.preampDb + Math.max(0, ...s.gainsDb);
  return `May distort: lower the preamp by ${Math.round(over * 10) / 10} dB`;
}
