// Albums, artists and the sidebar numbers (redesign), built from the song rows.

const collators = new Map();
const collator = (locale) => {
  if (!collators.has(locale)) collators.set(locale, new Intl.Collator(locale, { numeric: true }));
  return collators.get(locale);
};

const byTrack = (a, b) => (a.discNo ?? 0) - (b.discNo ?? 0) || (a.trackNo ?? 0) - (b.trackNo ?? 0) || a.id - b.id;

/** Albums: songs grouped by album and album artist (or artist), in disc and track order. */
export function groupAlbums(songs, locale) {
  const groups = new Map();
  for (const s of songs) {
    if (!s.album) continue;
    const artist = s.albumArtist || s.artist || '';
    const key = `${artist}\u0000${s.album}`;
    if (!groups.has(key)) groups.set(key, { key, title: s.album, artist, songs: [] });
    groups.get(key).songs.push(s);
  }
  const c = collator(locale);
  return [...groups.values()]
    .map((g) => {
      g.songs.sort(byTrack);
      return {
        ...g,
        year: g.songs.find((s) => s.year)?.year ?? null,
        count: g.songs.length,
        durationMs: g.songs.reduce((sum, s) => sum + (s.durationMs ?? 0), 0),
        coverId: g.songs.find((s) => s.hasArtwork)?.id ?? null,
      };
    })
    .sort((a, b) => c.compare(a.title, b.title) || c.compare(a.artist, b.artist));
}

/** Artists by song artist, with how many songs and albums each has. */
export function groupArtists(songs, locale) {
  const groups = new Map();
  for (const s of songs) {
    if (!s.artist) continue;
    if (!groups.has(s.artist)) groups.set(s.artist, { name: s.artist, songs: [], albumSet: new Set() });
    const g = groups.get(s.artist);
    g.songs.push(s);
    if (s.album) g.albumSet.add(s.album);
  }
  const c = collator(locale);
  return [...groups.values()]
    .map(({ name, songs: list, albumSet }) => ({
      name,
      songs: list,
      count: list.length,
      albums: albumSet.size,
      coverId: list.find((s) => s.hasArtwork)?.id ?? null,
    }))
    .sort((a, b) => c.compare(a.name, b.name));
}

export function libraryStats(songs) {
  return { songs: songs.length, liked: songs.filter((s) => s.rating === 1).length };
}

/** Two lines for the light / bold title style: first half of the words, then the rest. */
export function splitTitle(title) {
  const words = String(title ?? '').trim().split(/\s+/).filter(Boolean);
  if (words.length < 2) return ['', words[0] ?? ''];
  const cut = Math.ceil(words.length / 2);
  return [words.slice(0, cut).join(' '), words.slice(cut).join(' ')];
}

/** The "always sync" rule an artist or album page stands for, or null when it can't have one. */
export function syncRuleFor(page) {
  if (page?.kind === 'artist' && page.title) return { kind: 'artist', artist: page.title, album: null };
  if (page?.kind === 'album' && page.title && page.artist) return { kind: 'album', artist: page.artist, album: page.title };
  return null;
}

export function hasSyncRule(rules, rule) {
  return Boolean(rule) && rules.some((r) => r.kind === rule.kind && r.artist === rule.artist && (r.album ?? null) === (rule.album ?? null));
}

/** Whether an album page should offer "Find artwork": an album with an artist and no cover yet. */
export function needsArtwork(page) {
  return page?.kind === 'album' && Boolean(page.title) && Boolean(page.artist) && page.coverId == null;
}
