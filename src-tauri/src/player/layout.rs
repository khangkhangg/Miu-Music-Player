//! Where a song goes on a music player: `Artist\Album\NN Title.ext`, with names every FAT32 / exFAT
//! player accepts. Players browse by folder as often as by tag, so the layout must read well.

use std::path::PathBuf;

/// The tags a song's place on the player is made from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SongName {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album_artist: Option<String>,
    pub album: Option<String>,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    /// The original file name without its extension, for songs without a title.
    pub file_stem: String,
}

/// Longest folder or file name (characters); FAT32 allows 255, but full paths must stay short
/// enough for players and for Windows' 260-character limit.
const MAX_NAME: usize = 80;
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5",
    "LPT6", "LPT7", "LPT8", "LPT9",
];

/// One folder or file name: characters FAT forbids become `_`, no leading or trailing dots and
/// spaces, no reserved device names, at most `MAX_NAME` characters.
pub fn clean(name: &str, fallback: &str) -> String {
    let replaced: String = name.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let mut s: String = replaced.trim_matches(|c: char| c == '.' || c.is_whitespace()).chars().take(MAX_NAME).collect();
    s = s.trim_end_matches(|c: char| c == '.' || c.is_whitespace()).to_string();
    if s.is_empty() {
        return fallback.to_string();
    }
    let stem = s.split('.').next().unwrap_or("");
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        s.push('_');
    }
    s
}

fn tag(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// `Artist\Album\NN Title.ext` relative to the player's music folder; disc 2 and later get
/// `D-NN`. Folder names use the album artist so compilations stay in one folder.
pub fn song_path(s: &SongName, ext: &str) -> PathBuf {
    let artist = clean(tag(&s.album_artist).or(tag(&s.artist)).unwrap_or(""), "Unknown Artist");
    let album = clean(tag(&s.album).unwrap_or(""), "Unknown Album");
    let title = tag(&s.title).unwrap_or(&s.file_stem);
    let number = match (s.disc_no.filter(|d| *d > 1), s.track_no.filter(|t| *t > 0)) {
        (Some(d), Some(t)) => format!("{d}-{t:02} "),
        (None, Some(t)) => format!("{t:02} "),
        _ => String::new(),
    };
    let file = clean(&format!("{number}{title}"), "Untitled");
    PathBuf::from(artist).join(album).join(format!("{file}.{}", ext.to_ascii_lowercase()))
}

/// `name (2).ext`, `name (3).ext`… for when `path` is taken by a file Miu did not put there.
pub fn numbered(path: &std::path::Path, n: u32) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name = match path.extension() {
        Some(ext) => format!("{stem} ({n}).{}", ext.to_string_lossy()),
        None => format!("{stem} ({n})"),
    };
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn song(title: &str, artist: &str, album: &str, track: Option<u32>) -> SongName {
        SongName { title: Some(title.into()), artist: Some(artist.into()), album: Some(album.into()), track_no: track, file_stem: "file".into(), ..Default::default() }
    }

    #[test]
    fn songs_go_under_artist_and_album_with_a_track_number() {
        assert_eq!(song_path(&song("Diễm xưa", "Khánh Ly", "Tình ca", Some(3)), "MP3"), Path::new("Khánh Ly").join("Tình ca").join("03 Diễm xưa.mp3"));
        assert_eq!(song_path(&song("Song", "A", "B", None), "flac"), Path::new("A").join("B").join("Song.flac"), "no number without a track");
    }

    #[test]
    fn the_album_artist_keeps_compilations_together_and_later_discs_are_numbered() {
        let mut s = song("Song", "Guest", "Hits", Some(7));
        s.album_artist = Some("Various Artists".into());
        s.disc_no = Some(2);
        assert_eq!(song_path(&s, "mp3"), Path::new("Various Artists").join("Hits").join("2-07 Song.mp3"));
        s.disc_no = Some(1);
        assert_eq!(song_path(&s, "mp3").file_name().unwrap(), "07 Song.mp3", "disc 1 is not marked");
    }

    #[test]
    fn missing_tags_fall_back_to_readable_names() {
        let s = SongName { file_stem: "track01".into(), ..Default::default() };
        assert_eq!(song_path(&s, "mp3"), Path::new("Unknown Artist").join("Unknown Album").join("track01.mp3"));
        let blank = SongName { title: Some("  ".into()), artist: Some("".into()), file_stem: "x".into(), ..Default::default() };
        assert_eq!(song_path(&blank, "mp3"), Path::new("Unknown Artist").join("Unknown Album").join("x.mp3"));
    }

    #[test]
    fn names_fat_players_refuse_are_made_safe() {
        assert_eq!(clean("AC/DC", "x"), "AC_DC");
        assert_eq!(clean("What? <Live>: \"Best\" | Mix*", "x"), "What_ _Live__ _Best_ _ Mix_");
        assert_eq!(clean("  ...Hidden. . ", "x"), "Hidden");
        assert_eq!(clean("tab\there", "x"), "tab_here");
        assert_eq!(clean("CON", "x"), "CON_");
        assert_eq!(clean("aux.live", "x"), "aux.live_");
        assert_eq!(clean("...", "Unknown"), "Unknown");
        assert_eq!(clean("Ánh trăng", "x"), "Ánh trăng", "any script is fine on FAT32 and exFAT");
    }

    #[test]
    fn long_names_are_cut_on_a_character_boundary() {
        let long = "Đ".repeat(200);
        let c = clean(&long, "x");
        assert_eq!(c.chars().count(), MAX_NAME);
        let spaced = format!("{}  .tail", "a".repeat(MAX_NAME - 1));
        assert_eq!(clean(&spaced, "x"), "a".repeat(MAX_NAME - 1), "no trailing space left after the cut");
    }

    #[test]
    fn a_taken_name_gets_a_number() {
        let p = Path::new("A").join("B").join("03 Song.mp3");
        assert_eq!(numbered(&p, 2), Path::new("A").join("B").join("03 Song (2).mp3"));
    }
}
