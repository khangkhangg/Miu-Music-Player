import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deviceCard, formatBytes } from '../../src/js/device.js';

test('no device asks for the cable', () => {
  assert.deepEqual(deviceCard([]), {
    state: 'empty', title: 'No device connected', detail: 'Plug in your iPhone with a USB cable', storage: null,
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

const info = {
  name: 'khang’s iPhone', model: 'iPhone 5s', modelId: 'iPhone6,1', iosVersion: '12.5.8',
  capacityBytes: 27687563264, freeBytes: 25046335488,
};

test('a trusted iPhone with info shows its name, model and iOS', () => {
  const c = deviceCard([{ udid: 'e51e30a5aaaa', trusted: true, info }]);
  assert.equal(c.title, 'khang’s iPhone');
  assert.equal(c.detail, 'iPhone 5s · iOS 12.5.8');
});

test('storage shows free space and how full the phone is', () => {
  const c = deviceCard([{ udid: 'e51e30a5aaaa', trusted: true, info }]);
  assert.deepEqual(c.storage, { usedPercent: 10, label: '25.0 GB free of 27.7 GB' });
});

test('no storage bar when sizes are unknown', () => {
  const c = deviceCard([{ udid: 'a', trusted: true, info: { ...info, capacityBytes: null, freeBytes: null } }]);
  assert.equal(c.storage, null);
});

test('formats sizes in decimal units like iOS does', () => {
  assert.equal(formatBytes(999), '999 B');
  assert.equal(formatBytes(1_500_000), '1.5 MB');
  assert.equal(formatBytes(64_000_000_000), '64.0 GB');
});
