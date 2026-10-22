import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildManifest, assetName } from '../../tools/latest-json.mjs';

test('the release asset has a name without spaces', () => {
  assert.equal(assetName('0.3.1'), 'MiuMusic_0.3.1_x64-setup.exe');
});

test('latest.json has what the Tauri updater reads, pointing at the GitHub release asset', () => {
  const m = buildManifest({ version: '0.3.1', notes: 'Updates inside the app.', pubDate: new Date('2026-10-22T08:00:00Z'), signature: 'c2lnbmF0dXJl\n' });
  assert.deepEqual(m, {
    version: '0.3.1',
    notes: 'Updates inside the app.',
    pub_date: '2026-10-22T08:00:00.000Z',
    platforms: {
      'windows-x86_64': {
        signature: 'c2lnbmF0dXJl',
        url: 'https://github.com/khangkhangg/Miu-Music-Player/releases/download/v0.3.1/MiuMusic_0.3.1_x64-setup.exe',
      },
    },
  });
});

test('a leading v in the version is dropped', () => {
  assert.equal(buildManifest({ version: 'v1.0.0', notes: '', pubDate: new Date(0), signature: 'x' }).version, '1.0.0');
});

test('a missing signature or a version that is not x.y.z is refused', () => {
  assert.throws(() => buildManifest({ version: '0.3.1', notes: '', pubDate: new Date(0), signature: '  ' }), /signature/);
  assert.throws(() => buildManifest({ version: 'latest', notes: '', pubDate: new Date(0), signature: 'x' }), /version/);
});
