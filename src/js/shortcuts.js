// Keyboard shortcuts in Settings (K.1): reading a key press as keys, and showing keys.

import { t } from './i18n.js';

const NAMED = {
  Space: 'Space', ArrowLeft: 'Left', ArrowRight: 'Right', ArrowUp: 'Up', ArrowDown: 'Down',  // i18n-ignore: key names
  Home: 'Home', End: 'End', PageUp: 'PageUp', PageDown: 'PageDown', Insert: 'Insert', Delete: 'Delete',  // i18n-ignore: key names
};

/** The key of a key code, as Miu spells it ("KeyM" → "M"), or null for keys Miu does not use. */
function keyOf(code) {
  if (NAMED[code]) return NAMED[code];
  let m = /^Key([A-Z])$/.exec(code) || /^Digit([0-9])$/.exec(code);
  if (m) return m[1];
  m = /^F([1-9]|1[0-9]|2[0-4])$/.exec(code);
  return m ? `F${m[1]}` : null;
}

/** A key press as keys ("Ctrl+Alt+Space"), in the order Miu saves them; null until a usable key comes. */
export function keysFromEvent(e) {
  const key = keyOf(e.code);
  if (!key) return null;
  const mods = [e.ctrlKey && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift', e.metaKey && 'Win'].filter(Boolean); // i18n-ignore: key names
  return [...mods, key].join('+');
}

const ARROWS = { Left: '←', Right: '→', Up: '↑', Down: '↓' }; // i18n-ignore: key names

/** Keys to read: "Ctrl + Alt + →"; "Off" when there are none. */
export function keysLabel(keys) {
  if (!keys) return t('shortcuts.off');
  return keys.split('+').map(k => ARROWS[k] ?? k).join(' + ');
}
