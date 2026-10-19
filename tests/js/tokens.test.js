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
