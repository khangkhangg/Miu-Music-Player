// The landing page (site/): every language says everything English says, with the same markup.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';

const site = path.resolve(import.meta.dirname, '../../site');
// lang.js is a plain browser script that sets window.MiuLang: run it the same way.
const browser = {};
new Function(fs.readFileSync(path.join(site, 'lang.js'), 'utf8')).call(browser);
const { LANGUAGES, pickLanguage, shotLanguage, tickerWords } = browser.MiuLang;
const read = (code) => JSON.parse(fs.readFileSync(path.join(site, 'i18n', `${code}.json`), 'utf8'));
const en = read('en');
const tags = (s) => (s.match(/<\/?[a-z]+>/g) ?? []).join('');

test('the ten most spoken languages and Vietnamese', () => {
  assert.deepEqual(LANGUAGES.map((l) => l.code), ['en', 'zh', 'hi', 'es', 'ar', 'fr', 'bn', 'pt', 'ru', 'ur', 'vi']);
  assert.deepEqual(LANGUAGES.filter((l) => l.rtl).map((l) => l.code), ['ar', 'ur']);
});

test('every key in the page is in English, and English has nothing the page lacks', () => {
  const html = fs.readFileSync(path.join(site, 'index.html'), 'utf8');
  const used = new Set([...html.matchAll(/data-(?:i18n|i18n-html|i18n-alt|ticker)="([\w.-]+)"/g)].map((m) => m[1]));
  for (const k of ['meta.title', 'meta.description']) used.add(k);
  assert.deepEqual([...used].sort(), Object.keys(en).sort());
});

for (const { code } of LANGUAGES.filter((l) => l.code !== 'en')) {
  test(`${code}: every key, translated, same markup`, () => {
    const dict = read(code);
    assert.deepEqual(Object.keys(dict).sort(), Object.keys(en).sort(), 'same keys as English');
    for (const [k, v] of Object.entries(dict)) {
      assert.ok(typeof v === 'string' && v.trim(), `${k} is empty`);
      assert.equal(tags(v), tags(en[k]), `${k} keeps the same <em>/<b> markup`);
      if (k === 'hero.ticker') assert.equal(tickerWords(v).length, tickerWords(en[k]).length, 'as many occasions');
    }
  });
}

test('the language shown: saved choice, then the browser, then English', () => {
  assert.equal(pickLanguage('fr', ['vi-VN']), 'fr');
  assert.equal(pickLanguage(null, ['de-DE', 'pt-BR', 'en']), 'pt');
  assert.equal(pickLanguage('xx', ['zh-CN']), 'zh');
  assert.equal(pickLanguage(null, ['de']), 'en');
  assert.equal(pickLanguage(null, undefined), 'en');
});

test('screenshots in the app language: Vietnamese, else English', () => {
  assert.equal(shotLanguage('vi'), 'vi');
  assert.equal(shotLanguage('ar'), 'en');
});
