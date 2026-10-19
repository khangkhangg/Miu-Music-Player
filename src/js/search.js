// Search (F8.4): accent- and case-insensitive, any script. "mua thu" finds "Mùa thu",
// "DIEM XUA" finds "Diễm xưa"; CJK matches by substring.

/** Lowercase, compatibility forms unified (full-width → ASCII), Latin accents removed, đ → d. */
export function fold(text) {
  return String(text ?? '')
    .normalize('NFKD')
    .replace(/[̀-ͯ]/g, '') // Latin combining marks only: kana sound marks stay
    .normalize('NFC')
    .toLowerCase()
    .replace(/đ/g, 'd');
}

// Folded title/artist/album per song object, built once (the list is re-filtered on every key).
const keys = new WeakMap();

function key(song) {
  let k = keys.get(song);
  if (k === undefined) {
    k = fold([song.title, song.artist, song.album].filter(Boolean).join('\n'));
    keys.set(song, k);
  }
  return k;
}

/** Songs whose title, artist or album contain every word of `query`, in their original order. */
export function searchSongs(list, query) {
  const words = fold(query).split(/\s+/).filter(Boolean);
  if (words.length === 0) return list;
  return list.filter((s) => {
    const k = key(s);
    return words.every((w) => k.includes(w));
  });
}
