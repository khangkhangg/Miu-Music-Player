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
