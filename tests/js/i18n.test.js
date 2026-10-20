import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { t, useDictionary, resolveLanguage } from '../../src/js/i18n.js';

const root = path.resolve(import.meta.dirname, '../..');
const read = (p) => fs.readFileSync(path.join(root, p), 'utf8');
const en = JSON.parse(read('src/i18n/en.json'));
const vi = JSON.parse(read('src/i18n/vi.json'));
const vars = (s) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();

test('every English key exists in Vietnamese, with the same placeholders', () => {
  const missing = Object.keys(en).filter((k) => !(k in vi));
  assert.deepEqual(missing, [], 'keys missing from vi.json');
  const extra = Object.keys(vi).filter((k) => !(k in en));
  assert.deepEqual(extra, [], 'keys in vi.json that en.json does not have');
  for (const k of Object.keys(en)) {
    assert.ok(vi[k].trim(), `vi.json ${k} is empty`);
    assert.deepEqual(vars(vi[k]), vars(en[k]), `placeholders differ for ${k}`);
  }
});

test('t() fills placeholders, picks plural forms and falls back to English', () => {
  useDictionary('en', en);
  assert.equal(t('library.summary', { songs: t('count.songs', { n: 1 }), folders: t('count.folders', { n: 2 }) }), '1 song in 2 folders');
  useDictionary('vi', { ...vi, 'count.songs.other': undefined });
  assert.equal(t('count.songs', { n: 3 }), '3 songs', 'missing Vietnamese text falls back to English');
  useDictionary('vi', vi);
  assert.equal(t('count.songs', { n: 3 }), '3 bài hát');
  assert.equal(t('no.such.key'), 'no.such.key');
  useDictionary('en', en);
});

test('the language follows the setting, else the system', () => {
  assert.equal(resolveLanguage('auto', ['vi-VN', 'en']), 'vi');
  assert.equal(resolveLanguage('auto', ['en-US']), 'en');
  assert.equal(resolveLanguage('auto', ['fr-FR']), 'en', 'English for languages Miu does not speak yet');
  assert.equal(resolveLanguage('vi', ['en-US']), 'vi');
  assert.equal(resolveLanguage('en', ['vi-VN']), 'en');
});

// ---- Lint: no hard-coded UI strings ----

const LETTERS = /\p{L}{2,}/u;

/** Visible text and labelled attributes in an HTML file that are not marked for translation. */
function untranslatedHtml(html) {
  const problems = [];
  const body = html.replace(/<!--[\s\S]*?-->/g, '').replace(/<(script|style|svg)[\s\S]*?<\/\1>/g, '');
  for (const m of body.matchAll(/<(\w+)((?:[^>"']|"[^"]*"|'[^']*')*)>([^<]*)/g)) {
    const [, tag, attrs, text] = m;
    const marked = /\sdata-i18n="[^"]+"/.test(attrs);
    if (LETTERS.test(text) && !marked) problems.push(`<${tag}> text "${text.trim()}"`);
    const i18nAttrs = attrs.match(/\sdata-i18n-attr="([^"]*)"/)?.[1] ?? '';
    for (const a of ['aria-label', 'title', 'placeholder', 'alt']) {
      const v = attrs.match(new RegExp(`\\s${a}="([^"]*)"`))?.[1];
      if (v && LETTERS.test(v) && !new RegExp(`(^|;)${a}:`).test(i18nAttrs)) problems.push(`<${tag}> ${a}="${v}"`);
    }
  }
  return problems;
}

// Strings that are code, not words for people.
const CODE_WORDS = new Set(['Escape', 'Enter', 'Space', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End']);

/** English-looking string literals in a JS file. Lines marked `i18n-ignore` hold names, not words. */
function untranslatedJs(src) {
  const code = src
    .split('\n').filter((line) => !line.includes('i18n-ignore')).join('\n')
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/(^|[^:\\])\/\/.*$/gm, '$1');
  const problems = [];
  for (const m of code.matchAll(/'((?:[^'\\\n]|\\.)*)'|"((?:[^"\\\n]|\\.)*)"|`((?:[^`\\]|\\.)*)`/g)) {
    const s = (m[1] ?? m[2] ?? m[3]).replace(/\$\{[^}]*\}/g, ' ');
    if (CODE_WORDS.has(s) || s.includes('<') || /^[a-z0-9-]+( [a-z0-9-]+)*$/.test(s)) continue; // markup, class lists
    if (/^[A-Z][a-z]{2,}/.test(s) || /[A-Za-z]{2,} [A-Za-z]{2,}/.test(s)) problems.push(s);
  }
  return problems;
}

test('the lint catches what it should', () => {
  assert.deepEqual(untranslatedHtml('<p>Hello</p><p data-i18n="a">Hi</p><button aria-label="Play"></button>'), ['<p> text "Hello"', '<button> aria-label="Play"']);
  assert.deepEqual(untranslatedHtml('<button aria-label="Play" data-i18n-attr="aria-label:x"></button>'), []);
  assert.deepEqual(untranslatedJs("el.textContent = 'Nothing playing'; on('player'); // Some comment\nkey === 'Escape'"), ['Nothing playing']);
  assert.deepEqual(untranslatedJs("b.className = 'chip active'; x = '<svg viewBox=\"0 0 1 1\"></svg>'; y = 'Ogg Vorbis'; // i18n-ignore"), []);
});

for (const file of ['src/index.html', 'src/mini.html']) {
  test(`no untranslated text in ${file}`, () => {
    assert.deepEqual(untranslatedHtml(read(file)), []);
  });
}

for (const file of fs.readdirSync(path.join(root, 'src/js')).filter((f) => f.endsWith('.js') && f !== 'i18n.js')) {
  test(`no hard-coded UI strings in src/js/${file}`, () => {
    assert.deepEqual(untranslatedJs(read(`src/js/${file}`)), []);
  });
}

test('every key the code and pages use exists in en.json (plain or as plural forms)', () => {
  const has = (k) => k in en || (`${k}.one` in en && `${k}.other` in en);
  const used = new Set();
  for (const file of fs.readdirSync(path.join(root, 'src/js')).filter((f) => f.endsWith('.js'))) {
    for (const m of read(`src/js/${file}`).matchAll(/\bt\('([\w.-]+)'/g)) used.add(m[1]);
  }
  for (const file of ['src/index.html', 'src/mini.html']) {
    const html = read(file);
    for (const m of html.matchAll(/data-i18n="([\w.-]+)"/g)) used.add(m[1]);
    for (const m of html.matchAll(/data-i18n-attr="([^"]+)"/g)) for (const pair of m[1].split(';')) used.add(pair.split(':')[1]);
  }
  assert.deepEqual([...used].filter((k) => !has(k)).sort(), []);
});
