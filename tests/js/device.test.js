import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deviceCard } from '../../src/js/device.js';

test('no device asks for the cable', () => {
  assert.deepEqual(deviceCard([]), {
    state: 'empty', title: 'No device connected', detail: 'Plug in your iPhone with a USB cable',
  });
});

test('an untrusted iPhone asks the user to tap Trust', () => {
  const c = deviceCard([{ udid: 'e51e30a5aaaa', trusted: false }]);
  assert.equal(c.state, 'untrusted');
  assert.equal(c.title, 'iPhone connected');
  assert.equal(c.detail, 'Unlock your iPhone and tap Trust');
});

test('a trusted iPhone is ready', () => {
  const c = deviceCard([{ udid: 'e51e30a5aaaa', trusted: true }]);
  assert.equal(c.state, 'ready');
  assert.equal(c.detail, 'Connected over USB');
});

test('extra devices are counted', () => {
  const c = deviceCard([{ udid: 'a', trusted: true }, { udid: 'b', trusted: true }]);
  assert.equal(c.detail, 'Connected over USB · 1 more device');
});

test('missing list behaves like no device', () => {
  assert.equal(deviceCard(undefined).state, 'empty');
});
