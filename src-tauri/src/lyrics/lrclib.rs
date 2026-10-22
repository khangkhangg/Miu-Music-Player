//! LRCLIB (lrclib.net): a free, open lyrics database, no key or account. `/api/get` finds one
//! track by artist, title, album and length; `/api/search` is the looser fallback. Records carry
//! plain lyrics and often synced (LRC) lyrics.

use serde::Deserialize;

const BASE: &str = "https://lrclib.net/api";

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    #[serde(default)]
    pub track_name: String,
    #[serde(default)]
    pub artist_name: String,
    /// Seconds.
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub instrumental: bool,
    pub plain_lyrics: Option<String>,
    pub synced_lyrics: Option<String>,
}

impl Record {
    fn has_synced(&self) -> bool {
        self.synced_lyrics.as_deref().is_some_and(|s| !s.trim().is_empty())
    }

    fn has_words(&self) -> bool {
        self.has_synced() || self.plain_lyrics.as_deref().is_some_and(|s| !s.trim().is_empty())
    }
}

/// Exact lookup; needs the album and the length (LRCLIB matches on all four).
pub fn get_url(artist: &str, title: &str, album: &str, duration_s: u64) -> String {
    format!("{BASE}/get?artist_name={}&track_name={}&album_name={}&duration={duration_s}", encode(artist), encode(title), encode(album))
}

pub fn search_url(artist: &str, title: &str) -> String {
    format!("{BASE}/search?artist_name={}&track_name={}", encode(artist), encode(title))
}

/// Percent-encode a query value (RFC 3986 unreserved characters stay).
fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn parse_record(json: &str) -> Option<Record> {
    serde_json::from_str(json).ok()
}

pub fn parse_search(json: &str) -> Vec<Record> {
    serde_json::from_str(json).unwrap_or_default()
}

/// From search results: one within 3 s of the song's length (when known) that has words,
/// synced lyrics first, then the closest length. An instrumental match counts too.
pub fn best(records: Vec<Record>, duration_s: Option<u64>) -> Option<Record> {
    let off = |r: &Record| duration_s.map_or(0.0, |d| (r.duration - d as f64).abs());
    records
        .into_iter()
        .filter(|r| (r.has_words() || r.instrumental) && off(r) <= 3.0)
        .min_by(|a, b| b.has_synced().cmp(&a.has_synced()).then(off(a).total_cmp(&off(b))))
}

/// Find lyrics for a song. `http(url)` returns the body, `None` for "not found" (404).
pub fn lookup(
    artist: &str,
    title: &str,
    album: Option<&str>,
    duration_s: Option<u64>,
    http: &dyn Fn(&str) -> Result<Option<String>, String>,
) -> Result<Option<Record>, String> {
    if let (Some(album), Some(d)) = (album.filter(|a| !a.trim().is_empty()), duration_s) {
        if let Some(r) = http(&get_url(artist, title, album, d))?.as_deref().and_then(parse_record) {
            if r.has_words() || r.instrumental {
                return Ok(Some(r));
            }
        }
    }
    // Then searches, looser each time: as tagged, with the main artist and a clean title, then
    // free text. The length check in `best` keeps the loose ones from finding another song.
    let (main, clean) = (main_artist(artist), clean_title(title, artist));
    let mut urls = vec![search_url(artist, title)];
    if (main, clean.as_str()) != (artist.trim(), title.trim()) {
        urls.push(search_url(main, &clean));
    }
    urls.push(query_url(&format!("{main} {clean}")));
    for url in urls {
        let found = http(&url)?.as_deref().map(parse_search).unwrap_or_default();
        if let Some(r) = best(found, duration_s) {
            return Ok(Some(r));
        }
    }
    Ok(None)
}

pub fn query_url(q: &str) -> String {
    format!("{BASE}/search?q={}", encode(q))
}

/// The first of several artists ("A, B", "A feat. B", "A ft. B", "A & B", "A; B").
pub fn main_artist(artist: &str) -> &str {
    let lower = artist.to_lowercase();
    let cut = [",", ";", " & ", " feat.", " feat ", " ft.", " ft ", " x ", " / "]
        .iter()
        .filter_map(|sep| lower.find(sep))
        .min()
        .unwrap_or(artist.len());
    // `to_lowercase` can change byte lengths; fall back to the whole name if the cut lands oddly.
    artist.get(..cut).unwrap_or(artist).trim()
}

/// The song's own title: without "Artist - " / " - Artist", video labels like "(Official MV)" or
/// "[Lyric Video]", or a "feat." credit.
pub fn clean_title(title: &str, artist: &str) -> String {
    const LABELS: [&str; 9] = ["official", "mv", "m/v", "lyric", "video", "audio", "visualizer", "feat", "ft."];
    let mut out = String::new();
    let mut depth = 0;
    let mut group = String::new();
    for ch in title.chars() {
        match ch {
            '(' | '[' => {
                depth += 1;
                group.push(ch);
            }
            ')' | ']' if depth > 0 => {
                depth -= 1;
                group.push(ch);
                if depth == 0 {
                    let g = group.to_lowercase();
                    if !LABELS.iter().any(|l| g.contains(l)) {
                        out.push_str(&group);
                    }
                    group.clear();
                }
            }
            _ if depth > 0 => group.push(ch),
            _ => out.push(ch),
        }
    }
    out.push_str(&group);
    let mut t = out.split_whitespace().collect::<Vec<_>>().join(" ");
    for credit in [" feat. ", " ft. ", " feat ", " ft "] {
        if let Some(i) = t.to_lowercase().find(credit) {
            t.truncate(i);
        }
    }
    for name in [artist.trim(), main_artist(artist)] {
        if name.is_empty() {
            continue;
        }
        let lower = t.to_lowercase();
        let n = name.to_lowercase();
        if lower.ends_with(&format!(" - {n}")) {
            t.truncate(t.len() - name.len() - 3);
        } else if lower.starts_with(&format!("{n} - ")) {
            t = t[name.len() + 3..].to_string();
        }
    }
    t.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const FOUND: &str = r#"{"id":3396226,"trackName":"I Want to Live","artistName":"Borislav Slavov","albumName":"Baldur's Gate 3","duration":233.0,"instrumental":false,"plainLyrics":"I feel your breath\nupon my neck","syncedLyrics":"[00:17.12] I feel your breath\n[00:22.20] upon my neck"}"#;

    fn rec(duration: f64, plain: Option<&str>, synced: Option<&str>) -> Record {
        Record { track_name: "t".into(), artist_name: "a".into(), duration, instrumental: false, plain_lyrics: plain.map(Into::into), synced_lyrics: synced.map(Into::into) }
    }

    #[test]
    fn urls_carry_the_song_encoded() {
        assert_eq!(
            get_url("Khánh Ly", "Diễm Xưa", "Hát cho quê hương", 233),
            "https://lrclib.net/api/get?artist_name=Kh%C3%A1nh%20Ly&track_name=Di%E1%BB%85m%20X%C6%B0a&album_name=H%C3%A1t%20cho%20qu%C3%AA%20h%C6%B0%C6%A1ng&duration=233"
        );
        assert_eq!(search_url("AC/DC", "T.N.T."), "https://lrclib.net/api/search?artist_name=AC%2FDC&track_name=T.N.T.");
    }

    #[test]
    fn reads_a_record_and_a_search_answer() {
        let r = parse_record(FOUND).unwrap();
        assert_eq!(r.track_name, "I Want to Live");
        assert_eq!(r.duration, 233.0);
        assert!(r.synced_lyrics.unwrap().starts_with("[00:17.12]"));
        assert_eq!(parse_search(&format!("[{FOUND},{FOUND}]")).len(), 2);
        assert!(parse_record("not json").is_none());
        assert!(parse_search("{}").is_empty());
    }

    #[test]
    fn the_best_result_matches_the_length_and_prefers_synced() {
        let picked = best(vec![rec(233.0, Some("p"), None), rec(234.0, Some("p"), Some("[00:01.00]s")), rec(300.0, None, Some("[00:01.00]x"))], Some(233)).unwrap();
        assert!(picked.has_synced() && picked.duration == 234.0);
        assert!(best(vec![rec(300.0, Some("p"), None)], Some(233)).is_none(), "a different recording");
        assert!(best(vec![rec(233.0, None, None)], Some(233)).is_none(), "no words");
        assert_eq!(best(vec![rec(100.0, Some("p"), None)], None).unwrap().duration, 100.0, "length unknown");
    }

    #[test]
    fn the_exact_lookup_comes_first_then_search() {
        let asked = RefCell::new(Vec::new());
        let http = |url: &str| -> Result<Option<String>, String> {
            asked.borrow_mut().push(url.to_string());
            Ok(if url.contains("/get?") { None } else { Some(format!("[{FOUND}]")) })
        };
        let r = lookup("Borislav Slavov", "I Want to Live", Some("Baldur's Gate 3"), Some(233), &http).unwrap().unwrap();
        assert!(r.has_synced());
        let asked = asked.into_inner();
        assert!(asked[0].contains("/get?") && asked[1].contains("/search?"), "{asked:?}");
    }

    #[test]
    fn without_an_album_it_searches_straight_away_and_nothing_found_is_none() {
        let asked = RefCell::new(0);
        let http = |url: &str| -> Result<Option<String>, String> {
            *asked.borrow_mut() += 1;
            assert!(url.contains("/search?"));
            Ok(Some("[]".into()))
        };
        assert_eq!(lookup("a", "t", None, Some(100), &http).unwrap(), None);
        assert_eq!(asked.into_inner(), 2, "the search, then free text");
    }

    #[test]
    fn a_network_error_is_reported() {
        let http = |_: &str| -> Result<Option<String>, String> { Err("offline".into()) };
        assert!(lookup("a", "t", Some("b"), Some(1), &http).is_err());
    }
}

#[cfg(test)]
mod matching_tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn the_main_artist_is_the_first_of_several() {
        assert_eq!(main_artist("Duongg, Nâu, W/n"), "Duongg");
        assert_eq!(main_artist("Edward Dương Nguyễn, Tùng Acoustic"), "Edward Dương Nguyễn");
        assert_eq!(main_artist("TINH HÀ \"SAY HI\" feat. CoolKid & Ánh Sáng AZA"), "TINH HÀ \"SAY HI\"");
        assert_eq!(main_artist("Sơn Tùng M-TP ft. Snoop Dogg"), "Sơn Tùng M-TP");
        assert_eq!(main_artist("A; B"), "A");
        assert_eq!(main_artist("Simon & Garfunkel"), "Simon", "a duo loses its second name; the full name is tried first anyway");
        assert_eq!(main_artist("Khánh Ly"), "Khánh Ly");
    }

    #[test]
    fn titles_lose_the_artist_and_video_labels() {
        assert_eq!(clean_title("1 2 3 4 - Chi Dân", "Chi Dân"), "1 2 3 4");
        assert_eq!(clean_title("Chi Dân - 1 2 3 4", "Chi Dân"), "1 2 3 4");
        assert_eq!(clean_title("Hãy Trao Cho Anh (Official MV)", "Sơn Tùng M-TP"), "Hãy Trao Cho Anh");
        assert_eq!(clean_title("Nơi Này Có Anh [Lyric Video]", "x"), "Nơi Này Có Anh");
        assert_eq!(clean_title("See You Again (feat. Charlie Puth)", "Wiz Khalifa"), "See You Again");
        assert_eq!(clean_title("Lạc Trôi ft. Triple D", "x"), "Lạc Trôi");
        assert_eq!(clean_title("Hotel California (Live)", "Eagles"), "Hotel California (Live)", "a real version label stays");
    }

    #[test]
    fn when_the_tags_find_nothing_it_tries_the_main_artist_and_a_clean_title_then_free_text() {
        let asked = RefCell::new(Vec::new());
        let http = |url: &str| -> Result<Option<String>, String> {
            asked.borrow_mut().push(url.to_string());
            Ok(Some("[]".into()))
        };
        assert_eq!(lookup("Chi Dân, X", "1 2 3 4 - Chi Dân", None, Some(200), &http).unwrap(), None);
        let asked = asked.into_inner();
        assert_eq!(asked.len(), 3, "{asked:?}");
        assert!(asked[0].contains("artist_name=Chi%20D%C3%A2n%2C%20X"));
        assert!(asked[1].contains("artist_name=Chi%20D%C3%A2n&track_name=1%202%203%204"), "{}", asked[1]);
        assert!(asked[2].contains("q=Chi%20D%C3%A2n%201%202%203%204"), "{}", asked[2]);
    }

    #[test]
    fn a_tidy_song_is_not_asked_twice() {
        let asked = RefCell::new(0);
        let http = |_: &str| -> Result<Option<String>, String> {
            *asked.borrow_mut() += 1;
            Ok(Some("[]".into()))
        };
        lookup("Khánh Ly", "Diễm Xưa", None, Some(200), &http).unwrap();
        assert_eq!(asked.into_inner(), 2, "search, then free text; no repeat of the same search");
    }
}
