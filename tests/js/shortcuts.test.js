import { test } from 'node:test';
import assert from 'node:assert/strict';
import { keysFromEvent, keysLabel } from '../../src/js/shortcuts.js';

const ev = (code, mods = {}) => ({ code, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...mods });

test('a key press becomes keys in the same spelling Miu saves', () => {
  assert.equal(keysFromEvent(ev('Space', { ctrlKey: true, altKey: true })), 'Ctrl+Alt+Space');
  assert.equal(keysFromEvent(ev('KeyM', { altKey: true, ctrlKey: true })), 'Ctrl+Alt+M');
  assert.equal(keysFromEvent(ev('ArrowRight', { shiftKey: true, ctrlKey: true })), 'Ctrl+Shift+Right');
  assert.equal(keysFromEvent(ev('Digit7', { metaKey: true })), 'Win+7');
  assert.equal(keysFromEvent(ev('F5', { altKey: true })), 'Alt+F5');
  assert.equal(keysFromEvent(ev('PageDown', { ctrlKey: true })), 'Ctrl+PageDown');
});

test('only modifiers so far, or a key Miu cannot use, gives nothing yet', () => {
  assert.equal(keysFromEvent(ev('ControlLeft', { ctrlKey: true })), null);
  assert.equal(keysFromEvent(ev('AltRight', { altKey: true, ctrlKey: true })), null);
  assert.equal(keysFromEvent(ev('Semicolon', { ctrlKey: true })), null);
});

test('keys read naturally, arrows as arrows', () => {
  assert.equal(keysLabel('Ctrl+Alt+Right'), 'Ctrl + Alt + →');
  assert.equal(keysLabel('Ctrl+Alt+Up'), 'Ctrl + Alt + ↑');
  assert.equal(keysLabel('Ctrl+Alt+Space'), 'Ctrl + Alt + Space');
  assert.equal(keysLabel(null), 'Off');
});
