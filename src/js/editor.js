// Tag editor helpers (F9.1–F9.4): what the editor shows for one or many songs, which changes it
// sends, and the bulk tools. Values in the editor are strings; changes follow the Rust
// `TagChanges` shape: a field is absent (keep), "clear", or { set: value }.

export const FIELDS = [
  { key: 'title', label: 'editor.title' },
  { key: 'artist', label: 'editor.artist' },
  { key: 'album', label: 'editor.album' },
  { key: 'albumArtist', label: 'editor.albumArtist' },
  { key: 'genre', label: 'editor.genre' },
  { key: 'year', label: 'editor.year', number: true },
  { key: 'track', label: 'editor.track', number: true },
  { key: 'trackTotal', label: 'editor.trackTotal', number: true },
  { key: 'disc', label: 'editor.disc', number: true },
];

const NUMBER = new Set(FIELDS.filter((f) => f.number).map((f) => f.key));
const shown = (v) => (v == null ? '' : String(v));

/** Per field: the shared value, or `mixed` when the songs differ (bulk editing leaves those alone). */
export function editorFields(songs) {
  return Object.fromEntries(FIELDS.map(({ key }) => {
    const values = new Set(songs.map((s) => shown(s.tags?.[key])));
    const mixed = values.size > 1;
    return [key, { value: mixed ? '' : [...values][0] ?? '', mixed }];
  }));
}

/** Changes for the fields the user touched (`edited`: key → typed string). */
export function changesFor(edited) {
  const out = {};
  for (const [key, raw] of Object.entries(edited)) {
    const v = String(raw ?? '').trim();
    if (v === '') out[key] = 'clear';
    else if (!NUMBER.has(key)) out[key] = { set: v };
    else if (/^\d+$/.test(v)) out[key] = { set: Number(v) };
  }
  return out;
}

const fileBase = (path) => path.split(/[\\/]/).pop().replace(/\.[^.]+$/, '');

/** Fields from a file name with a pattern like "%track% - %artist% - %title%"; null if it doesn't fit. */
export function parseFilename(pattern, path) {
  const keys = [];
  const source = pattern.split(/(%\w+%)/).map((part) => {
    const m = part.match(/^%(\w+)%$/);
    if (!m) return part.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    keys.push(m[1]);
    return '(.+?)';
  }).join('');
  const m = fileBase(path).match(new RegExp(`^${source}$`, 'u'));
  if (!m) return null;
  return Object.fromEntries(keys.map((k, i) => [k, m[i + 1].trim()]));
}

/** Track numbers in list order from `start`, with the total. */
export function autoNumber(songs, start = 1) {
  const total = String(start + songs.length - 1);
  return songs.map((_, i) => ({ track: String(start + i), trackTotal: total }));
}

/** Replace every `find` (plain text) in `value`. */
export function findReplace(value, find, replacement, matchCase = false) {
  if (!find) return value;
  const re = new RegExp(find.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'), matchCase ? 'gu' : 'giu');
  return String(value ?? '').replace(re, () => replacement);
}

/** 'title' (Each Word), 'sentence' (First word), 'upper' or 'lower', Vietnamese-aware. */
export function changeCase(value, mode) {
  const s = String(value ?? '');
  const lower = s.toLocaleLowerCase('vi');
  switch (mode) {
    case 'upper': return s.toLocaleUpperCase('vi');
    case 'lower': return lower;
    case 'title': return lower.replace(/(^|[\s(\[-])(\p{L})/gu, (_, before, c) => before + c.toLocaleUpperCase('vi'));
    case 'sentence': return lower.replace(/\p{L}/u, (c) => c.toLocaleUpperCase('vi'));
    default: return s;
  }
}

/** What will change, song by song (songs whose shown values stay the same are left out). */
export function previewRows(songs, editedPerSong) {
  const rows = [];
  songs.forEach((song, i) => {
    const edited = editedPerSong[i] ?? {};
    const changes = FIELDS.filter(({ key }) => key in edited).flatMap(({ key }) => {
      const from = shown(song.tags?.[key]);
      let to = String(edited[key] ?? '').trim();
      if (NUMBER.has(key) && /^\d+$/.test(to)) to = String(Number(to));
      return from === to ? [] : [{ field: key, from, to }];
    });
    if (changes.length) rows.push({ id: song.id, changes });
  });
  return rows;
}
