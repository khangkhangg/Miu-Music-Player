import { test } from 'node:test';
import assert from 'node:assert/strict';
import { prereqMessage } from '../../src/js/prereq.js';

test('no message when Apple drivers are ready', () => {
  assert.equal(prereqMessage({ ready: true, missing: [] }), null);
});

test('no message when the check has not run (browser preview, no Tauri)', () => {
  assert.equal(prereqMessage(undefined), null);
});

test('missing drivers explain what to install', () => {
  const m = prereqMessage({ ready: false, missing: ['AirTrafficHost.dll', 'MobileDevice.dll'] });
  assert.equal(m.title, 'Apple drivers needed');
  assert.match(m.body, /Apple Devices/);
  assert.match(m.body, /Microsoft Store/);
  assert.match(m.body, /restart Miu Music/);
});

test('missing drivers list the missing files for troubleshooting', () => {
  const m = prereqMessage({ ready: false, missing: ['AirTrafficHost.dll', 'MobileDevice.dll'] });
  assert.equal(m.detail, 'Missing: AirTrafficHost.dll, MobileDevice.dll');
});
