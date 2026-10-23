// The landing page's languages: English is in the page itself, the others are i18n/<code>.json
// (missing keys fall back to English). The ten most spoken languages in the world, plus Vietnamese.
(function (global) {
  var LANGUAGES = [
    { code: 'en', name: 'English' },
    { code: 'zh', name: '中文', html: 'zh-Hans' },
    { code: 'hi', name: 'हिन्दी' },
    { code: 'es', name: 'Español' },
    { code: 'ar', name: 'العربية', rtl: true },
    { code: 'fr', name: 'Français' },
    { code: 'bn', name: 'বাংলা' },
    { code: 'pt', name: 'Português' },
    { code: 'ru', name: 'Русский' },
    { code: 'ur', name: 'اردو', rtl: true },
    { code: 'vi', name: 'Tiếng Việt' },
  ];

  /** The language to show: the one picked before, else the first of the browser's we have, else English. */
  function pickLanguage(saved, browserLanguages) {
    var known = function (c) { return LANGUAGES.some(function (l) { return l.code === c; }); };
    if (saved && known(saved)) return saved;
    var list = browserLanguages || [];
    for (var i = 0; i < list.length; i++) {
      var c = String(list[i]).toLowerCase().split('-')[0];
      if (known(c)) return c;
    }
    return 'en';
  }

  /** Which screenshots to show: the app itself speaks English and Vietnamese. */
  function shotLanguage(code) {
    return code === 'vi' ? 'vi' : 'en';
  }

  /** The words of the occasions strip, from its " | " separated list. */
  function tickerWords(text) {
    return String(text || '').split('|').map(function (w) { return w.trim(); }).filter(Boolean);
  }

  var api = { LANGUAGES: LANGUAGES, pickLanguage: pickLanguage, shotLanguage: shotLanguage, tickerWords: tickerWords };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  else global.MiuLang = api;
})(this);
