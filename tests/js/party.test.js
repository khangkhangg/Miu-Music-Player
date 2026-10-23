import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { useDictionary } from '../../src/js/i18n.js';
import { videoUrl, videoEnded, listenMessage, entryLines, partyLimitOptions } from '../../src/js/party.js';

const en = JSON.parse(fs.readFileSync(path.resolve(import.meta.dirname, '../../src/i18n/en.json'), 'utf8'));
useDictionary('en', en);

test('YouTube plays in its own embedded player, told to report back', () => {
  const u = new URL(videoUrl('dQw4w9WgXcQ', 'http://tauri.localhost'));
  assert.equal(u.origin, 'https://www.youtube.com');
  assert.equal(u.pathname, '/embed/dQw4w9WgXcQ');
  assert.equal(u.searchParams.get('autoplay'), '1');
  assert.equal(u.searchParams.get('enablejsapi'), '1');
  assert.equal(u.searchParams.get('origin'), 'http://tauri.localhost');
  assert.equal(listenMessage(), JSON.stringify({ event: 'listening', id: 1, channel: 'widget' }));
});

test('the player says when the video has ended', () => {
  const msg = (data) => ({ origin: 'https://www.youtube.com', data: JSON.stringify(data) });
  assert.equal(videoEnded(msg({ event: 'onStateChange', info: 0 })), true);
  assert.equal(videoEnded(msg({ event: 'infoDelivery', info: { playerState: 0 } })), true);
  assert.equal(videoEnded(msg({ event: 'infoDelivery', info: { playerState: 1 } })), false);
  assert.equal(videoEnded({ origin: 'https://evil.example', data: JSON.stringify({ event: 'onStateChange', info: 0 }) }), false, 'only YouTube');
  assert.equal(videoEnded({ origin: 'https://www.youtube.com', data: 'not json' }), false);
});

test('queue entries read as title and who added them', () => {
  assert.deepEqual(entryLines({ kind: 'song', title: 'Faded', artist: 'Alan Walker', addedName: 'Ann', votes: 3 }), { title: 'Faded', sub: 'Alan Walker · added by Ann', votes: 3, video: false });
  assert.deepEqual(entryLines({ kind: 'video', title: 'Karaoke', addedName: '', votes: 1 }), { title: 'Karaoke', sub: '', votes: 1, video: true });
});

test('the per-guest limit choices', () => {
  assert.deepEqual(partyLimitOptions().map((o) => o.value), [1, 2, 3, 5, 10]);
});
