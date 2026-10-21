import { test } from 'node:test';
import assert from 'node:assert/strict';
import { playerCard, targetOptions, withPlays, PLAYS, playerStageText, playerResult, playerStopped } from '../../src/js/players.js';

const player = (over = {}) => ({
  id: 'vol:1A2B3C4D', root: 'E:\\', label: 'WALKMAN', totalBytes: 8_000_000_000, freeBytes: 2_000_000_000, configured: true,
  profile: { folder: 'MUSIC', target: 'mp3V0', plays: [] }, ...over,
});

test('a player shows its name, drive letter, folder and space', () => {
  const c = playerCard(player());
  assert.equal(c.title, 'WALKMAN (E:)');
  assert.equal(c.detail, 'Songs go to E:\\MUSIC');
  assert.deepEqual(c.storage, { usedPercent: 75, label: '2.0 GB free of 8.0 GB' });
  assert.equal(c.suggested, false);
});

test('an unnamed drive is called a USB drive; the root folder reads as the drive', () => {
  const c = playerCard(player({ label: '', profile: { folder: '', target: 'mp3V0', plays: [] } }));
  assert.equal(c.title, 'USB drive (E:)');
  assert.equal(c.detail, 'Songs go to E:\\');
});

test('a drive found by its Music folder is a suggestion until it is set up', () => {
  assert.equal(playerCard(player({ configured: false })).suggested, true);
});

test('the formats to convert to are offered, the chosen one marked', () => {
  const opts = targetOptions('aac256');
  assert.deepEqual(opts.map(o => o.value), ['mp3V0', 'mp3320', 'aac256']);
  assert.deepEqual(opts.map(o => o.label), ['MP3 V0 (about 245 kbit/s)', 'MP3 320 kbit/s', 'AAC 256 kbit/s']);
  assert.deepEqual(opts.map(o => o.selected), [false, false, true]);
});

test('ticking a format the player plays keeps the others and never duplicates', () => {
  const p = { folder: 'MUSIC', target: 'mp3V0', plays: ['wav'] };
  assert.deepEqual(withPlays(p, 'flac', true).plays, ['flac', 'wav']);
  assert.deepEqual(withPlays(withPlays(p, 'flac', true), 'flac', true).plays, ['flac', 'wav']);
  assert.deepEqual(withPlays(p, 'wav', false).plays, []);
  assert.deepEqual(p.plays, ['wav'], 'the old profile is left as it was');
  assert.ok(PLAYS.every(f => typeof f.value === 'string' && f.label));
});

test('progress reads as checking, then copying n of total', () => {
  assert.deepEqual(playerStageText({ stage: 'check' }), { text: 'Checking the player…', percent: null });
  assert.deepEqual(playerStageText({ stage: 'copy', done: 3, total: 12 }), { text: 'Copying songs: 3 of 12', percent: 25 });
});

test('a clean sync reads as one line; anything odd turns it into a warning', () => {
  const ok = { added: 3, replaced: 0, updated: 1, removed: 2, failed: [], leftAlone: 0, goneFromPlayer: 0 };
  assert.deepEqual(playerResult(ok), { tone: 'ok', lines: ['Sync finished: 3 added, 1 updated, 2 removed.'] });
  const odd = { ...ok, updated: 0, failed: [['Diễm xưa', 'disk full']], leftAlone: 1, goneFromPlayer: 2 };
  const r = playerResult(odd);
  assert.equal(r.tone, 'warn');
  assert.deepEqual(r.lines, [
    'Sync finished: 3 added, 0 updated, 2 removed.',
    '2 songs deleted on the player were copied again.',
    '1 file changed on the player was left there.',
    '1 song could not go: disk full',
  ]);
});

test('a sync that did not start says why', () => {
  assert.deepEqual(playerStopped({ kind: 'noSpace', need: 3_000_000_000, available: 1_000_000_000 }), {
    title: 'Sync stopped before changing anything', lines: ['Not enough space on the player: needs 3.0 GB, 1.0 GB available.'],
  });
  assert.deepEqual(playerStopped({ kind: 'failed', reason: 'The player is not connected.' }), { title: 'Sync failed', lines: ['The player is not connected.'] });
});

test('a phone or Walkman over MTP shows its name and storage, not a drive letter, and asks to stay unlocked', () => {
  const c = playerCard(player({ kind: 'mtp', root: '', label: 'Pixel 7 — Internal shared storage', profile: { folder: 'Music', target: 'aac256', plays: [] } }));
  assert.equal(c.title, 'Pixel 7 — Internal shared storage');
  assert.equal(c.detail, 'Songs go to Music');
  assert.equal(c.mtp, true);
  assert.equal(c.hint, 'Keep the phone unlocked and set to File transfer while it syncs.');
  assert.equal(playerCard(player()).mtp, false, 'a drive is not MTP');
});
