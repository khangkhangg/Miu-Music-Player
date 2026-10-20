import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { ERROR_CODES, errorState } from '../../src/js/errors.js';

const root = path.resolve(import.meta.dirname, '../..');
const dict = (lang) => JSON.parse(fs.readFileSync(path.join(root, `src/i18n/${lang}.json`), 'utf8'));

test('every error code has a title, an explanation and an action', () => {
  for (const code of ERROR_CODES) {
    const s = errorState(code);
    assert.ok(s.title && !s.title.includes('.'), `${code} title`);
    assert.ok(s.body.length > 20, `${code} body`);
    assert.ok(s.action && s.action.label && s.action.kind, `${code} action`);
  }
});

test('the words exist in English and Vietnamese', () => {
  for (const lang of ['en', 'vi']) {
    const d = dict(lang);
    for (const code of ERROR_CODES) {
      for (const part of ['title', 'body', 'action']) assert.ok(d[`error.${code}.${part}`], `${lang} error.${code}.${part}`);
    }
  }
});

test('actions say what they do', () => {
  assert.deepEqual(errorState('drivers-missing').action, { kind: 'open-url', label: 'Open Microsoft Store', url: 'ms-windows-store://pdp/?productid=9NP83LWLPZ9K' });
  assert.equal(errorState('device-untrusted').action.kind, 'retry');
  assert.equal(errorState('folder-missing', { path: 'E:\Nhạc' }).body.includes('E:\Nhạc'), true);
});

test('an unknown error still says something useful and offers the details', () => {
  const s = errorState('boom', { detail: 'AFC error 7' });
  assert.equal(s.code, 'unknown');
  assert.equal(s.action.kind, 'copy');
  assert.equal(s.action.text, 'AFC error 7');
});
