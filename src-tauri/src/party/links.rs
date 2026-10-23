//! Links guests paste (P.2): YouTube plays as a video in YouTube's own player; Spotify and
//! Apple Music cannot be played here, so their song is looked up in the DJ's library instead.

use unicode_normalization::UnicodeNormalization;

use crate::library::artfetch::fold;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    YouTube { video_id: String },
    /// A Spotify track: its title comes from Spotify's public oEmbed page.
    Spotify { url: String },
    /// An Apple Music song: the title is in the link itself.
    Apple { title: String },
}

fn is_video_id(s: &str) -> bool {
    s.len() == 11 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// What a pasted link points at, when Miu knows the service.
pub fn parse(url: &str) -> Option<Link> {
    let u = url.trim();
    let rest = u.strip_prefix("https://").or_else(|| u.strip_prefix("http://")).unwrap_or(u);
    let (host_path, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (host, path) = host_path.split_once('/').unwrap_or((host_path, ""));
    let host = host.trim_start_matches("www.").trim_start_matches("m.").to_lowercase();
    let param = |key: &str| query.split('&').find_map(|kv| kv.strip_prefix(&format!("{key}=")));
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match host.as_str() {
        "youtube.com" | "music.youtube.com" => {
            let id = match segments.as_slice() {
                ["watch"] => param("v")?,
                ["shorts" | "embed" | "live", id, ..] => id,
                _ => return None,
            };
            is_video_id(id).then(|| Link::YouTube { video_id: id.to_string() })
        }
        "youtu.be" => segments.first().filter(|id| is_video_id(id)).map(|id| Link::YouTube { video_id: id.to_string() }),
        "open.spotify.com" => {
            let i = segments.iter().position(|s| *s == "track")?;
            let id = segments.get(i + 1)?;
            Some(Link::Spotify { url: format!("https://open.spotify.com/track/{id}") })
        }
        "music.apple.com" => {
            // /{country}/song/{title}/{id} or /{country}/album/{album}/{id}?i={track}
            let slug = segments.get(2)?;
            matches!(segments.get(1), Some(&("song" | "album"))).then(|| Link::Apple { title: slug.replace('-', " ") })
        }
        _ => None,
    }
}

/// Lower case, no accents, words only (brackets kept, unlike `fold`).
fn plain(s: &str) -> String {
    let lower = s.to_lowercase().replace('đ', "d");
    let bare: String = lower.nfd().filter(|c| !('\u{300}'..='\u{36f}').contains(c)).collect();
    bare.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// A video or link title without the extras: "Artist - Title (Official Video) feat. X" → "Title".
pub fn clean_title(title: &str) -> String {
    let after_artist = title.split_once(" - ").map_or(title, |(_, t)| t);
    let mut kept = String::new();
    let mut depth = 0;
    for c in after_artist.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => kept.push(c),
            _ => {}
        }
    }
    let lower = kept.to_lowercase();
    let cut = [" feat.", " ft.", " feat ", " ft "].iter().filter_map(|m| lower.find(m)).min().unwrap_or(kept.len());
    kept[..cut].trim().to_string()
}

/// The library song (`(id, title, artist)`) with this title: an exact title before a remix of it,
/// the artist (when given) deciding between songs of the same name.
pub fn find_in_library(title: &str, artist: Option<&str>, songs: &[(i64, String, String)]) -> Option<i64> {
    let want_plain = plain(title);
    let want = fold(&clean_title(title));
    if want.is_empty() {
        return None;
    }
    let artist = artist.map(fold).filter(|a| !a.is_empty());
    songs
        .iter()
        .filter_map(|(id, t, a)| {
            let exact = plain(t) == want_plain || plain(t) == want;
            let close = fold(t) == want;
            if !exact && !close {
                return None;
            }
            let by = artist.as_ref().is_some_and(|w| fold(a).contains(w.as_str()));
            Some(((by as u8) * 2 + exact as u8, std::cmp::Reverse(*id)))
        })
        .max()
        .map(|(_, std::cmp::Reverse(id))| id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_links_in_every_shape() {
        for url in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com/watch?v=dQw4w9WgXcQ&list=RD123&t=42s",
            "https://m.youtube.com/watch?feature=share&v=dQw4w9WgXcQ",
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ&si=abc",
            "https://youtu.be/dQw4w9WgXcQ?si=xyz",
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
            "https://www.youtube.com/embed/dQw4w9WgXcQ",
            "youtube.com/live/dQw4w9WgXcQ",
            "  https://youtu.be/dQw4w9WgXcQ  ",
        ] {
            assert_eq!(parse(url), Some(Link::YouTube { video_id: "dQw4w9WgXcQ".into() }), "{url}");
        }
        assert_eq!(parse("https://www.youtube.com/watch?v=short"), None, "not a video id");
        assert_eq!(parse("https://www.youtube.com/playlist?list=PL123"), None);
    }

    #[test]
    fn spotify_and_apple_music_links() {
        assert_eq!(
            parse("https://open.spotify.com/intl-vi/track/4cOdK2wGLETKBW3PvgPWqT?si=abc"),
            Some(Link::Spotify { url: "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT".into() })
        );
        assert_eq!(parse("https://music.apple.com/us/song/love-yourself/1440833098"), Some(Link::Apple { title: "love yourself".into() }));
        assert_eq!(
            parse("https://music.apple.com/vn/album/purpose-deluxe/1440831203?i=1440833098"),
            Some(Link::Apple { title: "purpose deluxe".into() }),
            "an album link only names the album"
        );
        assert_eq!(parse("https://example.com/song"), None);
        assert_eq!(parse("not a link"), None);
    }

    #[test]
    fn a_song_is_found_in_the_library_by_its_title() {
        let lib = vec![
            (1, "Love Yourself".to_string(), "Justin Bieber".to_string()),
            (2, "Đi Về Nhà".to_string(), "Đen, JustaTee".to_string()),
            (3, "Faded (Remix)".to_string(), "Alan Walker".to_string()),
            (4, "Faded".to_string(), "Alan Walker".to_string()),
            (5, "Love Yourself".to_string(), "Someone Else".to_string()),
        ];
        assert_eq!(find_in_library("love yourself", None, &lib), Some(1));
        assert_eq!(find_in_library("Love Yourself", Some("Someone Else"), &lib), Some(5), "the artist decides");
        assert_eq!(find_in_library("di ve nha", None, &lib), Some(2), "accents and case do not matter");
        assert_eq!(find_in_library("Faded", None, &lib), Some(4), "the plain title before a remix");
        assert_eq!(find_in_library("Faded (Official Video)", None, &lib), Some(4), "video extras left out");
        assert_eq!(find_in_library("Something Else", None, &lib), None);
    }

    #[test]
    fn video_titles_lose_the_extras() {
        assert_eq!(clean_title("Alan Walker - Faded (Official Music Video)"), "Faded");
        assert_eq!(clean_title("Đi Về Nhà [Karaoke]"), "Đi Về Nhà");
        assert_eq!(clean_title("Love Yourself feat. Someone"), "Love Yourself");
    }
}
