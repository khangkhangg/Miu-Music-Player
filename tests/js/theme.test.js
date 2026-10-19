import { test } from 'node:test';
import assert from 'node:assert/strict';
import { resolveTheme } from '../../src/js/theme.js';

test('explicit light and dark preferences win over the system setting', () => {
  assert.equal(resolveTheme('light', true), 'light');
  assert.equal(resolveTheme('dark', false), 'dark');
});

test('system preference follows the OS dark mode flag', () => {
  assert.equal(resolveTheme('system', true), 'dark');
  assert.equal(resolveTheme('system', false), 'light');
});

test('unknown or missing preference falls back to system', () => {
  assert.equal(resolveTheme(undefined, true), 'dark');
  assert.equal(resolveTheme('purple', false), 'light');
});
