//! Song lyrics (L): from the file's tags, a `.lrc` file beside it, or LRCLIB (lrclib.net, a free
//! and open lyrics database), cached locally so each song is looked up online once.

pub mod lrc;
pub mod lrclib;
pub mod service;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use lrc::Line;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// Written in the song file's tags.
    Tags,
    /// A `.lrc` file next to the song.
    File,
    /// LRCLIB.
    Online,
    /// None found.
    None,
    /// The lyrics service could not be reached this time (not cached: asked again next time).
    Unreachable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lyrics {
    /// Time-stamped lines, empty when only plain lyrics are known.
    pub synced: Vec<Line>,
    pub plain: Option<String>,
    pub instrumental: bool,
    pub source: Source,
}

impl Lyrics {
    pub fn none() -> Self {
        Self { synced: Vec::new(), plain: None, instrumental: false, source: Source::None }
    }

    /// LRC text becomes synced lines (and plain text without the stamps); anything else is plain.
    pub fn from_text(text: &str, source: Source) -> Self {
        let text = text.trim_start_matches('\u{feff}').trim();
        if text.is_empty() {
            return Self::none();
        }
        let synced = lrc::parse(text);
        let plain = if synced.is_empty() {
            text.to_string()
        } else {
            synced.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n").trim().to_string()
        };
        Self { synced, plain: Some(plain).filter(|p| !p.is_empty()), instrumental: false, source }
    }
}

/// What a lookup needs to know about the song.
pub struct Song<'a> {
    pub path: &'a Path,
    pub artist: Option<&'a str>,
    pub title: Option<&'a str>,
    pub album: Option<&'a str>,
    pub duration_ms: Option<u64>,
}

/// How long "not found online" is remembered before asking again, seconds.
pub const RETRY_AFTER_S: u64 = 7 * 24 * 3600;

/// The song's lyrics: its tags (`embedded`), then a `.lrc` beside it, then the cache in
/// `cache_dir`, then LRCLIB when `online` (through `http`, see `lrclib::lookup`). Online answers,
/// "none" included, are cached; a failed request is not, so it is tried again next time.
pub fn find(
    song: &Song,
    embedded: Option<&str>,
    cache_dir: &Path,
    online: bool,
    now: u64,
    http: &dyn Fn(&str) -> Result<Option<String>, String>,
) -> Lyrics {
    if let Some(l) = embedded.map(|t| Lyrics::from_text(t, Source::Tags)).filter(|l| l.source != Source::None) {
        return l;
    }
    if let Ok(bytes) = std::fs::read(song.path.with_extension("lrc")) {
        let l = Lyrics::from_text(&String::from_utf8_lossy(&bytes), Source::File);
        if l.source != Source::None {
            return l;
        }
    }
    let cache = cache_file(cache_dir, song);
    if let Some(c) = std::fs::read(&cache).ok().and_then(|b| serde_json::from_slice::<Cached>(&b).ok()) {
        if c.lyrics.source != Source::None || now.saturating_sub(c.fetched_at) < RETRY_AFTER_S {
            return c.lyrics;
        }
    }
    let (Some(artist), Some(title), true) = (song.artist.filter(|a| !a.trim().is_empty()), song.title.filter(|t| !t.trim().is_empty()), online) else {
        return Lyrics::none();
    };
    let found = match lrclib::lookup(artist, title, song.album, song.duration_ms.map(|ms| (ms + 500) / 1000), http) {
        Ok(found) => found,
        Err(e) => {
            log::warn!("lyrics lookup failed: {e}");
            return Lyrics { source: Source::Unreachable, ..Lyrics::none() };
        }
    };
    let lyrics = match found {
        Some(r) => {
            let mut l = match (r.synced_lyrics.as_deref(), r.plain_lyrics.as_deref()) {
                (Some(s), _) if lrc::is_synced(s) => Lyrics::from_text(s, Source::Online),
                (_, Some(p)) => Lyrics::from_text(p, Source::Online),
                _ => Lyrics { source: Source::Online, ..Lyrics::none() },
            };
            if let Some(p) = r.plain_lyrics.filter(|p| !p.trim().is_empty()) {
                l.plain = Some(p.trim().to_string());
            }
            l.instrumental = r.instrumental;
            l.source = Source::Online;
            l
        }
        None => Lyrics::none(),
    };
    let _ = std::fs::create_dir_all(cache_dir);
    if let Ok(json) = serde_json::to_vec(&Cached { fetched_at: now, lyrics: lyrics.clone() }) {
        let _ = std::fs::write(&cache, json);
    }
    lyrics
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cached {
    fetched_at: u64,
    lyrics: Lyrics,
}

/// Cache file for a song, named after its artist, title, album and length (not its path, so
/// moving files keeps their lyrics).
pub fn cache_file(cache_dir: &Path, song: &Song) -> PathBuf {
    let key = format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        song.artist.unwrap_or("").trim().to_lowercase(),
        song.title.unwrap_or("").trim().to_lowercase(),
        song.album.unwrap_or("").trim().to_lowercase(),
        song.duration_ms.map_or(0, |ms| (ms + 500) / 1000),
    );
    // FNV-1a, 64 bit: a stable file name for the key.
    let hash = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3));
    cache_dir.join(format!("{hash:016x}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    const SYNCED: &str = "[00:01.00] first\n[00:04.50] second";

    fn song(path: &Path) -> Song<'_> {
        Song { path, artist: Some("Khánh Ly"), title: Some("Diễm Xưa"), album: Some("Hát cho quê hương"), duration_ms: Some(233_400) }
    }

    fn offline(_: &str) -> Result<Option<String>, String> {
        panic!("must not go online")
    }

    #[test]
    fn lrc_text_is_synced_and_also_readable_as_plain() {
        let l = Lyrics::from_text(SYNCED, Source::File);
        assert_eq!(l.synced.len(), 2);
        assert_eq!(l.plain.as_deref(), Some("first\nsecond"));
        let p = Lyrics::from_text("just\nwords", Source::Tags);
        assert!(p.synced.is_empty());
        assert_eq!(p.plain.as_deref(), Some("just\nwords"));
        assert_eq!(Lyrics::from_text("  ", Source::Tags), Lyrics::none());
    }

    #[test]
    fn lyrics_in_the_tags_come_first() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.mp3");
        std::fs::write(d.path().join("a.lrc"), SYNCED).unwrap();
        let l = find(&song(&p), Some("from tags"), d.path(), true, 0, &offline);
        assert_eq!(l.source, Source::Tags);
    }

    #[test]
    fn then_an_lrc_file_beside_the_song() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.flac");
        std::fs::write(d.path().join("a.lrc"), format!("\u{feff}{SYNCED}")).unwrap();
        let l = find(&song(&p), None, d.path(), true, 0, &offline);
        assert_eq!(l.source, Source::File);
        assert_eq!(l.synced[0].text, "first", "byte-order mark dropped");
    }

    #[test]
    fn then_lrclib_once_and_the_cache_after() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.mp3");
        let calls = Cell::new(0);
        let http = |url: &str| -> Result<Option<String>, String> {
            calls.set(calls.get() + 1);
            assert!(url.contains("/get?"));
            Ok(Some(format!(r#"{{"trackName":"Diễm Xưa","artistName":"Khánh Ly","duration":233.0,"instrumental":false,"plainLyrics":"first\nsecond","syncedLyrics":"{}"}}"#, SYNCED.replace('\n', "\\n"))))
        };
        let first = find(&song(&p), None, d.path(), true, 100, &http);
        assert_eq!(first.source, Source::Online);
        assert_eq!(first.synced.len(), 2);
        let again = find(&song(&p), None, d.path(), true, 200, &http);
        assert_eq!(again, first);
        assert_eq!(calls.get(), 1, "the second time comes from the cache");
    }

    #[test]
    fn not_found_is_remembered_for_a_while_but_a_failed_request_is_not() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.mp3");
        let calls = Cell::new(0);
        let none = |_: &str| -> Result<Option<String>, String> {
            calls.set(calls.get() + 1);
            Ok(Some("[]".into()))
        };
        assert_eq!(find(&song(&p), None, d.path(), true, 1_000, &none).source, Source::None);
        let n = calls.get();
        find(&song(&p), None, d.path(), true, 1_000 + 3600, &none);
        assert_eq!(calls.get(), n, "asked again too soon");
        find(&song(&p), None, d.path(), true, 1_000 + RETRY_AFTER_S + 1, &none);
        assert!(calls.get() > n, "asked again after a week");

        let d2 = tempfile::tempdir().unwrap();
        let failing = |_: &str| -> Result<Option<String>, String> { Err("offline".into()) };
        assert_eq!(find(&song(&p), None, d2.path(), true, 0, &failing).source, Source::Unreachable, "said apart from not found");
        assert!(std::fs::read_dir(d2.path()).unwrap().next().is_none(), "nothing cached");
    }

    #[test]
    fn online_lookup_can_be_turned_off_and_needs_artist_and_title() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.mp3");
        assert_eq!(find(&song(&p), None, d.path(), false, 0, &offline).source, Source::None);
        let unnamed = Song { path: &p, artist: None, title: Some("x"), album: None, duration_ms: None };
        assert_eq!(find(&unnamed, None, d.path(), true, 0, &offline).source, Source::None);
    }

    #[test]
    fn the_cache_follows_the_song_not_its_path() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("x/a.mp3"), d.path().join("y/b.mp3"));
        assert_eq!(cache_file(d.path(), &song(&a)), cache_file(d.path(), &song(&b)));
        let other = Song { title: Some("Hạ Trắng"), ..song(&a) };
        assert_ne!(cache_file(d.path(), &song(&a)), cache_file(d.path(), &other));
    }
}
