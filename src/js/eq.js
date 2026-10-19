// Equalizer panel helpers (F7.7).
import { t } from './i18n.js';

export function presetOptions(state) {
  const opts = [
    ...state.builtIn.map(p => ({ value: p.name, label: t(`eqPreset.${p.name}`), user: false })),
    ...state.user.map(p => ({ value: p.name, label: p.name, user: true })),
  ];
  return state.preset == null ? [{ value: '', label: t('eq.custom'), user: false }, ...opts] : opts;
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
  return t('eq.clip', { db: Math.round(over * 10) / 10 });
}
