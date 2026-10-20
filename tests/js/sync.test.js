import { test } from 'node:test';
import assert from 'node:assert/strict';
import { planSummary, smoothRate, eta, etaLabel } from '../../src/js/sync.js';

const step = (kind) => ({ kind });

test('the plan reads like "+42 add · 3 remove · 1.2 GB"', () => {
  const plan = { steps: [...Array(42).fill(step('add')), ...Array(3).fill(step('remove'))], addBytes: 1_200_000_000 };
  assert.equal(planSummary(plan), '+42 add · 3 remove · 1.2 GB');
});

test('updates and replacements are counted; empty parts are left out', () => {
  const plan = { steps: [step('update'), step('update'), step('replace')], addBytes: 9_000_000 };
  assert.equal(planSummary(plan), '1 replace · 2 update · 9.0 MB');
  assert.equal(planSummary({ steps: [], addBytes: 0 }), 'Up to date');
});

test('the transfer rate is smoothed so one slow second does not swing the estimate', () => {
  assert.equal(smoothRate(null, 1_000_000), 1_000_000, 'first sample taken as is');
  assert.equal(smoothRate(1_000_000, 0), 800_000, 'a stall pulls it down gently');
});

test('ETA from remaining bytes and the smoothed rate, never negative', () => {
  assert.equal(eta(10_000_000, 2_000_000), 5);
  assert.equal(eta(-5, 2_000_000), 0);
  assert.equal(eta(10_000_000, 0), null, 'unknown until data moves');
});

test('ETA reads as minutes and seconds', () => {
  assert.equal(etaLabel(5), 'About 5 s left');
  assert.equal(etaLabel(125), 'About 2 min left');
  assert.equal(etaLabel(null), '');
});

import { stageText, syncResult, syncStopped } from '../../src/js/sync.js';

test('each sync stage says what is happening; sending shows how far along', () => {
  assert.deepEqual(stageText({ stage: 'Backup' }), { text: 'Backing up the iPhone’s music database…', percent: null });
  assert.equal(stageText({ stage: 'CopyOff', songs: 3 }).text, 'Keeping a copy of 3 songs that are only on the iPhone…');
  assert.deepEqual(stageText({ stage: 'Apply', done: 1, total: 4 }), { text: 'Handing songs to iTunes: 1 of 4', percent: 25 });
  assert.equal(stageText({ stage: 'UpdateDevice' }).text, 'iTunes is syncing the iPhone…');
});

test('a finished sync says what changed and anything to look at', () => {
  const ok = syncResult({ added: 2, removed: 1, adopted: 5, failed: [], unexpectedRemoved: [], warnings: [], confirmed: true });
  assert.equal(ok.tone, 'ok');
  assert.deepEqual(ok.lines, ['Sync finished: 2 added, 1 removed.', '5 liked songs were already on the iPhone.']);
  const warn = syncResult({
    added: 1, removed: 0, adopted: 0, confirmed: false, warnings: [],
    failed: [[7, 'needs converting for the iPhone first (ffmpeg is not set up yet)']],
    unexpectedRemoved: [{ title: 'Bông Cỏ Mây' }],
  });
  assert.equal(warn.tone, 'warn');
  assert.deepEqual(warn.lines, [
    'Sync finished: 1 added, 0 removed.',
    '1 song could not go: needs converting for the iPhone first (ffmpeg is not set up yet)',
    'The sync removed “Bông Cỏ Mây” without being asked. A copy is in Miu’s backups folder.',
    'The iPhone has not shown every change yet.',
  ]);
});

test('a stopped sync says nothing changed, and why', () => {
  assert.deepEqual(syncStopped({ kind: 'blocked', reasons: ['iTunes does not list “X”, so a sync would delete it from the iPhone.'] }), {
    title: 'Sync stopped before changing anything',
    lines: ['iTunes does not list “X”, so a sync would delete it from the iPhone.'],
  });
  assert.deepEqual(syncStopped({ kind: 'failed', reason: 'cannot start iTunes' }), { title: 'Sync failed', lines: ['cannot start iTunes'] });
  assert.deepEqual(syncStopped('boom'), { title: 'Sync failed', lines: ['boom'] });
});

import { showsGuide, lastSyncLabel } from '../../src/js/sync.js';

test('the manual-management guide shows until a phone has synced once', () => {
  assert.equal(showsGuide({ lastRun: null }), true);
  assert.equal(showsGuide({ lastRun: { id: 1, finishedAt: 100, done: 51, failed: 0 } }), false);
});

test('the last sync reads as when and how it went', () => {
  const now = 1_000_000;
  assert.equal(lastSyncLabel({ finishedAt: now - 30, done: 51, failed: 0 }, now), 'Last sync: just now · 51 songs');
  assert.equal(lastSyncLabel({ finishedAt: now - 600, done: 3, failed: 2 }, now), 'Last sync: 10 min ago · 3 songs · 2 failed');
  assert.equal(lastSyncLabel({ finishedAt: now - 7_200, done: 1, failed: 0 }, now), 'Last sync: 2 h ago · 1 song');
  assert.equal(lastSyncLabel(null, now), '');
});

test('a sync says when songs went again and when iTunes refused everything', () => {
  const r = syncResult({ added: 1, removed: 0, adopted: 0, failed: [], unexpectedRemoved: [], warnings: [], confirmed: true, goneFromPhone: 1, manualManagementHint: false });
  assert.ok(r.lines.includes('1 song deleted on the iPhone was sent again.'), r.lines);
  const refused = syncResult({ added: 0, removed: 0, adopted: 0, failed: [[1, 'iTunes AddTrack failed']], unexpectedRemoved: [], warnings: [], confirmed: true, goneFromPhone: 0, manualManagementHint: true });
  assert.equal(refused.tone, 'warn');
  assert.ok(refused.lines.some((l) => l.includes('Manually manage music')), refused.lines);
});
