// Error and empty states (F5.6): every problem Miu can name gets plain words and one thing to do.
import { t } from './i18n.js';

const ACTIONS = {
  'drivers-missing': { kind: 'open-url', url: 'ms-windows-store://pdp/?productid=9NP83LWLPZ9K' }, // Apple Devices app
  'device-untrusted': { kind: 'retry' },
  'device-locked': { kind: 'retry' },
  'device-disconnected': { kind: 'retry' },
  'device-busy': { kind: 'retry' },
  'no-space': { kind: 'show-plan' },
  'folder-missing': { kind: 'remove-folder' },
  'sync-unavailable': { kind: 'dismiss' },
  'output-busy': { kind: 'open-output' },
  unknown: { kind: 'copy' },
};

export const ERROR_CODES = Object.keys(ACTIONS);

/** Title, explanation and action for `code`; `vars` fill the text (path, need, available, detail). */
export function errorState(code, vars = {}) {
  const known = code in ACTIONS ? code : 'unknown';
  const action = { ...ACTIONS[known], label: t(`error.${known}.action`) };
  if (known === 'unknown') action.text = vars.detail ?? String(code);
  return {
    code: known,
    title: t(`error.${known}.title`),
    body: t(`error.${known}.body`, vars),
    action,
  };
}
