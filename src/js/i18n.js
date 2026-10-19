// UI translation (F8.6). Strings live in src/i18n/<lang>.json; English is the fallback for any
// key a translation lacks. HTML marks text with data-i18n="key" and attributes with
// data-i18n-attr="aria-label:key;title:key".

import en from '../i18n/en.json' with { type: 'json' };

export const LANGUAGES = ['en', 'vi'];

let lang = 'en';
let dict = en;
let plurals = new Intl.PluralRules('en');

/** Switch to an already loaded dictionary (tests; `setLanguage` loads from disk). */
export function useDictionary(code, d) {
  lang = code;
  dict = d;
  plurals = new Intl.PluralRules(code);
}

export function language() {
  return lang;
}

function pick(key) {
  return dict[key] ?? en[key];
}

/**
 * The text for `key` with `{name}` placeholders filled from `vars`. With `vars.n`, the plural form
 * `key.one` / `key.other` (CLDR categories of the current language) is used.
 */
export function t(key, vars = {}) {
  let s;
  if (vars.n != null) s = pick(`${key}.${plurals.select(vars.n)}`) ?? pick(`${key}.other`);
  s ??= pick(key);
  if (s == null) return key;
  return s.replace(/\{(\w+)\}/g, (m, k) => (k in vars ? String(vars[k]) : m));
}

/** 'auto' follows the system: Vietnamese for a Vietnamese system, English otherwise. */
export function resolveLanguage(setting, systemLanguages) {
  if (LANGUAGES.includes(setting)) return setting;
  return /^vi\b/i.test(systemLanguages?.[0] ?? '') ? 'vi' : 'en';
}

/** Load a language and translate the page's marked elements. */
export async function setLanguage(code, root = document) {
  const d = code === 'en' ? en : await fetch(new URL(`../i18n/${code}.json`, import.meta.url)).then((r) => r.json());
  useDictionary(code, d);
  translatePage(root);
}

export function translatePage(root = document) {
  root.documentElement?.setAttribute('lang', lang);
  for (const el of root.querySelectorAll('[data-i18n]')) el.textContent = t(el.dataset.i18n);
  for (const el of root.querySelectorAll('[data-i18n-attr]')) {
    for (const pair of el.dataset.i18nAttr.split(';')) {
      const [attr, key] = pair.split(':');
      if (attr && key) el.setAttribute(attr.trim(), t(key.trim()));
    }
  }
}

/**
 * Window startup: load the saved language (Tauri `invoke`) before anything renders, and reload
 * this window when the language is changed anywhere. Returns the setting ("auto", "en", "vi").
 */
export async function startInLanguage(invoke) {
  const setting = await invoke('ui_language').catch(() => 'auto');
  await setLanguage(resolveLanguage(setting, navigator.languages)).catch(() => useDictionary('en', en));
  window.__TAURI__.event.listen('language-changed', () => location.reload());
  return setting;
}
