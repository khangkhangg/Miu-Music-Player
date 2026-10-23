import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { useDictionary } from '../../src/js/i18n.js';
import { videoUrl, videoEnded, listenMessage, entryLines, partyLimitOptions, relayNote, partyQr, nowAndNext, beatLight, dropCountdown, singLines, shoutOut } from '../../src/js/party.js';

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

test('the relay link, or why it is not working', () => {
  assert.equal(relayNote({ relayOn: false }), '');
  assert.equal(relayNote({ relayOn: true }), 'Opening the party link…');
  assert.equal(relayNote({ relayOn: true, relayUrl: 'https://musicplayer.vietnamcos.com/party/?r=AB12' }), 'https://musicplayer.vietnamcos.com/party/?r=AB12');
  assert.equal(relayNote({ relayOn: true, relayError: 'relayOff' }), 'The site has party links turned off right now.');
  assert.equal(relayNote({ relayOn: true, relayError: 'offline' }), 'Cannot reach musicplayer.vietnamcos.com. Trying again…');
  assert.equal(relayNote({ relayOn: true, relayError: 'http502' }), 'The party link is not working (http502). Trying again…');
});

test('the party screen shows the link that works for everyone', () => {
  assert.equal(partyQr({ localOn: false, relayOn: false }), null);
  assert.equal(partyQr({ localOn: true, localQr: '<svg>wifi</svg>', relayOn: false }), '<svg>wifi</svg>');
  assert.equal(partyQr({ localOn: true, localQr: '<svg>wifi</svg>', relayOn: true, relayQr: '<svg>relay</svg>' }), '<svg>relay</svg>');
  assert.equal(partyQr({ localOn: true, localQr: '<svg>wifi</svg>', relayOn: true, relayError: 'offline' }), '<svg>wifi</svg>', 'the Wi-Fi one while the relay is not working');
});

test('the party screen shows the song on air and the one coming', () => {
  const songs = new Map([[1, { id: 1, title: 'Faded' }], [2, { id: 2, title: 'Love Yourself' }]]);
  const byId = (id) => songs.get(id);
  const deck = (songId, extra = {}) => ({ songId, playing: false, positionS: 30, durationS: 120, level: 0.5, ...extra });
  let r = nowAndNext({ onAir: 'B', decks: [deck(2), deck(1, { playing: true })] }, byId);
  assert.equal(r.now.title, 'Faded');
  assert.equal(r.next.title, 'Love Yourself');
  assert.equal(r.progress, 0.25);
  assert.equal(r.level, 0.5);
  r = nowAndNext({ onAir: null, decks: [deck(1, { playing: true }), deck(null)] }, byId);
  assert.equal(r.now.title, 'Faded', 'no deck on air: the one playing');
  assert.equal(r.next, null);
  r = nowAndNext({ onAir: null, decks: [deck(null), deck(null)] }, byId);
  assert.deepEqual(r, { now: null, next: null, progress: 0, level: 0 });
  assert.deepEqual(nowAndNext(null, byId), { now: null, next: null, progress: 0, level: 0 });
});

test('the lights pulse on every beat, harder on the first beat of a bar', () => {
  const show = { beatS: 0.5, beatPos: 8, dropInS: null };
  const onBar = beatLight(show, 0);
  assert.ok(onBar.pulse > 0.95 && onBar.bar, 'a bar starts right now');
  const later = beatLight(show, 200); // 0.4 of the next beat
  assert.ok(later.pulse < 0.4 && later.bar, 'fading, still the first beat of the bar');
  const nextBeat = beatLight(show, 500);
  assert.ok(nextBeat.pulse > 0.95 && !nextBeat.bar, 'beat 2 of the bar');
  assert.equal(beatLight(null, 0), null, 'no beat grid: no lights');
});

test('the countdown shows the bars left before the drop, then the drop', () => {
  const show = { beatS: 0.5, beatPos: 0, dropInS: 7.5 }; // bars of 2 s
  assert.deepEqual(dropCountdown(show, 0), { bars: 4 });
  assert.deepEqual(dropCountdown(show, 2000), { bars: 3 });
  assert.deepEqual(dropCountdown(show, 7000), { bars: 1 });
  assert.deepEqual(dropCountdown(show, 7600), { drop: true }, 'just landed');
  assert.equal(dropCountdown(show, 9000), null, 'over');
  assert.equal(dropCountdown({ beatS: 0.5, beatPos: 0, dropInS: 30 }, 0), null, 'too far away to count');
  assert.equal(dropCountdown({ beatS: 0.5, beatPos: 0, dropInS: null }, 0), null);
});

test('the sing-along shows the line to sing now and the next, big', () => {
  const lyrics = { synced: [{ ms: 10000, text: 'first line' }, { ms: 14000, text: 'second line' }, { ms: 18000, text: 'third line' }] };
  const show = { sing: true, posS: 13, speed: 1 };
  assert.deepEqual(singLines(show, 0, lyrics), { now: 'first line', next: 'second line' });
  assert.deepEqual(singLines(show, 1500, lyrics), { now: 'second line', next: 'third line' }, 'the song moved on since the report');
  assert.deepEqual(singLines({ ...show, speed: 2 }, 2600, lyrics), { now: 'third line', next: '' }, 'played faster');
  assert.deepEqual(singLines(show, 0, { synced: [] }), { singIt: true }, 'no timed lyrics: just "sing it"');
  assert.deepEqual(singLines(show, 0, null), { singIt: true });
  assert.equal(singLines({ ...show, sing: false }, 0, lyrics), null, 'not a sing-along');
  assert.equal(singLines(null, 0, lyrics), null);
});

test('a guest gets a shout-out when their song plays', () => {
  assert.equal(shoutOut({ now: { id: 4, addedName: 'Linh' } }), 'Linh');
  assert.equal(shoutOut({ now: { id: 4, addedName: '  ' } }), null, 'no name given');
  assert.equal(shoutOut({ now: { id: 4, addedName: '' } }), null, 'the DJ or Auto DJ picked it');
  assert.equal(shoutOut({ now: null }), null);
  assert.equal(shoutOut(null), null);
});
