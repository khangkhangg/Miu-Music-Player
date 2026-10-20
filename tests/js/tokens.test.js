import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

const css = readFileSync(new URL('../../src/styles/tokens.css', import.meta.url), 'utf8');

function varsIn(selectorRegex) {
  const m = css.match(selectorRegex);
  assert.ok(m, `block not found: ${selectorRegex}`);
  return new Set([...m[1].matchAll(/(--[\w-]+)\s*:/g)].map(x => x[1]).sort());
}

const light = () => varsIn(/^:root\s*\{([^}]*)\}/m);
const dark = () => varsIn(/:root\[data-theme="dark"\]\s*\{([^}]*)\}/);
const systemDark = () => varsIn(/@media \(prefers-color-scheme: dark\)\s*\{\s*:root:not\(\[data-theme="light"\]\)\s*\{([^}]*)\}/);

test('light theme defines the core color tokens', () => {
  for (const t of ['--bg', '--surface', '--text', '--text-muted', '--border', '--accent', '--on-accent', '--success', '--danger']) {
    assert.ok(light().has(t), `missing ${t}`);
  }
});

test('dark theme defines every color token the light theme does', () => {
  const colors = [...light()].filter(t => !t.startsWith('--font') && !t.startsWith('--radius') && !t.startsWith('--space'));
  for (const t of colors) assert.ok(dark().has(t), `dark missing ${t}`);
});

test('system dark mode block matches the explicit dark theme', () => {
  assert.deepEqual([...systemDark()], [...dark()]);
});

test('tokens use only the system font stack, no web fonts', () => {
  assert.ok(!/@import|@font-face|url\(/.test(css));
});

// ---- Redesign: charcoal / warm light with a yellow accent ----

function valuesIn(selectorRegex) {
  const m = css.match(selectorRegex);
  return Object.fromEntries([...m[1].matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)].map((x) => [x[1], x[2].trim()]));
}

function luminance(hex) {
  const [r, g, b] = hex.replace('#', '').match(/../g).map((h) => parseInt(h, 16) / 255)
    .map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

const contrast = (a, b) => {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};

for (const [name, block] of [['light', /^:root\s*\{([^}]*)\}/m], ['dark', /:root\[data-theme="dark"\]\s*\{([^}]*)\}/]]) {
  test(`${name} theme has the redesign surfaces and the yellow accent`, () => {
    const v = valuesIn(block);
    for (const t of ['--sidebar', '--surface-raised', '--accent-ink']) assert.ok(v[t], `${name} missing ${t}`);
    assert.equal(v['--accent'].toLowerCase(), '#f2c12e');
  });

  test(`${name} theme text is readable (WCAG AA 4.5:1)`, () => {
    const v = valuesIn(block);
    for (const surface of ['--surface', '--surface-raised', '--sidebar']) {
      assert.ok(contrast(v['--text'], v[surface]) >= 4.5, `${name} text on ${surface}`);
      assert.ok(contrast(v['--text-muted'], v[surface]) >= 4.5, `${name} muted text on ${surface}`);
      assert.ok(contrast(v['--accent-ink'], v[surface]) >= 4.5, `${name} accent text on ${surface}`);
    }
    assert.ok(contrast(v['--on-accent'], v['--accent']) >= 4.5, `${name} text on the accent`);
  });
}
