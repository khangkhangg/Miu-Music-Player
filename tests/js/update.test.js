import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { useDictionary } from '../../src/js/i18n.js';
import { isNewer, bannerState, downloadState, checkResultText } from '../../src/js/update.js';

const root = path.resolve(import.meta.dirname, '../..');
useDictionary('en', JSON.parse(fs.readFileSync(path.join(root, 'src/i18n/en.json'), 'utf8')));

test('versions compare part by part, not as text', () => {
  assert.equal(isNewer('0.3.1', '0.3.0'), true);
  assert.equal(isNewer('0.10.0', '0.9.9'), true);
  assert.equal(isNewer('1.0.0', '1.0.0'), false);
  assert.equal(isNewer('0.2.9', '0.3.0'), false);
  assert.equal(isNewer('v0.4.0', '0.3.1'), true, 'a leading v is ignored');
  assert.equal(isNewer('0.4.0-beta', '0.3.1'), true, 'a suffix does not hide the numbers');
  assert.equal(isNewer('', '0.3.1'), false);
});

test('the banner names the new version and shows the release notes', () => {
  assert.deepEqual(bannerState({ version: '0.3.2', current: '0.3.1', notes: 'Faster sync.' }), {
    show: true,
    title: 'Miu Music 0.3.2 is available',
    notes: 'Faster sync.',
  });
});

test('no banner without an update, or for one that is not newer', () => {
  assert.equal(bannerState(null).show, false);
  assert.equal(bannerState({ version: '0.3.1', current: '0.3.1', notes: '' }).show, false);
});

test('the download shows how far along it is', () => {
  assert.deepEqual(downloadState({ done: 12e6, total: 48e6 }), { percent: 25, text: 'Downloading the update: 12.0 MB of 48.0 MB' });
  assert.deepEqual(downloadState({ done: 3e6, total: null }), { percent: 0, text: 'Downloading the update: 3.0 MB' });
});

test('Check now says what it found', () => {
  assert.equal(checkResultText({ version: '0.3.2', current: '0.3.1' }), 'Miu Music 0.3.2 is available');
  assert.equal(checkResultText(null), 'Miu Music is up to date.');
  assert.equal(checkResultText(new Error('offline')), 'Could not check for updates: offline');
  assert.equal(checkResultText('dns error'), 'Could not check for updates: dns error');
});
