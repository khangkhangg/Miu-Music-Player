//! What Windows' media controls (SMTC: the lock screen, the Quick Settings / volume media card,
//! keyboard media keys and headset buttons) show for Miu and do when pressed. This part is pure;
//! `media_controls_win` hands it to Windows.

use std::path::Path;

/// Miu's Windows app identity (AppUserModelID), as on the Start menu shortcut the installer makes.
/// The process declares it at start so Windows can tie the media session (lock screen) and the
/// taskbar button to the installed app.
pub const APP_ID: &str = "com.khangkhangg.miumusic";

/// What the controls show the player doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// No audio output: the controls are hidden.
    Closed,
    /// Nothing loaded.
    Stopped,
    Playing,
    Paused,
}

/// What is playing: a library song, or a podcast episode / audiobook chapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Track {
    Song(i64),
    Episode(i64),
}

pub fn track(song_id: Option<i64>, episode_id: Option<i64>) -> Option<Track> {
    episode_id.map(Track::Episode).or(song_id.map(Track::Song))
}

pub fn status(available: bool, playing: bool, track: Option<Track>) -> Status {
    match (available, track, playing) {
        (false, ..) => Status::Closed,
        (_, None, _) => Status::Stopped,
        (_, _, true) => Status::Playing,
        _ => Status::Paused,
    }
}

/// A button pressed on the controls (or a media key / headset button).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Play,
    Pause,
    Stop,
    Next,
    Previous,
}

/// The player command it stands for: the same ones the UI's buttons call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Resume,
    Pause,
    Skip(i64),
    /// Dragging the position on the controls (where Windows offers it).
    Seek(u64),
}

pub fn command(button: Button) -> Command {
    match button {
        Button::Play => Command::Resume,
        Button::Pause | Button::Stop => Command::Pause,
        Button::Next => Command::Skip(1),
        Button::Previous => Command::Skip(-1),
    }
}

/// The text, unless it is empty or blank.
fn filled(s: Option<&str>) -> Option<&str> {
    s.filter(|s| !s.trim().is_empty())
}

/// The three lines the controls show.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Display {
    pub title: String,
    pub artist: String,
    pub album: String,
}

/// A song: untagged songs show their file name; the artist falls back to the album artist.
pub fn song_display(title: Option<&str>, artist: Option<&str>, album_artist: Option<&str>, album: Option<&str>, path: &Path) -> Display {
    let file = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    Display {
        title: filled(title).map_or(file, str::to_string),
        artist: filled(artist).or(filled(album_artist)).unwrap_or_default().to_string(),
        album: filled(album).unwrap_or_default().to_string(),
    }
}

/// An episode or chapter: the show (or book) is the album, its author the artist.
pub fn episode_display(title: &str, show: &str, author: Option<&str>) -> Display {
    Display { title: title.to_string(), artist: filled(author).unwrap_or(show).to_string(), album: show.to_string() }
}

/// Decides when the position is worth sending again. Windows moves the position along by itself
/// while playing, so it only needs telling on a new track, play/pause, a seek, or now and then.
#[derive(Debug, Default)]
pub struct TimelineClock {
    last: Option<Sent>,
}

#[derive(Debug, Clone, Copy)]
struct Sent {
    track: Option<Track>,
    playing: bool,
    position_ms: u64,
    at_ms: u64,
}

/// Resend at least this often.
pub const TIMELINE_EVERY_MS: u64 = 5_000;
/// A position this far from where it should be by now is a seek.
const JUMP_MS: u64 = 1_500;

impl TimelineClock {
    /// Whether to send the timeline for this state at `now_ms` (any steady clock); when true, it counts as sent.
    pub fn due(&mut self, track: Option<Track>, playing: bool, position_ms: u64, now_ms: u64) -> bool {
        let due = match self.last {
            None => true,
            Some(l) => {
                let elapsed = now_ms.saturating_sub(l.at_ms);
                let expected = if l.playing { l.position_ms + elapsed } else { l.position_ms };
                l.track != track || l.playing != playing || expected.abs_diff(position_ms) > JUMP_MS || (playing && elapsed >= TIMELINE_EVERY_MS)
            }
        };
        if due {
            self.last = Some(Sent { track, playing, position_ms, at_ms: now_ms });
        }
        due
    }
}

/// What a track looks like on the controls, from the library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub display: Display,
    pub duration_ms: Option<u64>,
    /// The show's artwork online (episodes); songs' covers come from `library::service::cover_jpeg`.
    pub artwork_url: Option<String>,
}

pub fn describe(db: &crate::library::db::Db, track: Track) -> Option<Info> {
    let ms = |v: Option<i64>| v.map(|v| v.max(0) as u64);
    match track {
        Track::Song(id) => db
            .conn()
            .query_row("SELECT title, artist, album_artist, album, path, duration_ms FROM songs WHERE id = ?1", [id], |r| {
                let (title, artist, album_artist, album, path): (Option<String>, Option<String>, Option<String>, Option<String>, String) =
                    (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?);
                Ok(Info {
                    display: song_display(title.as_deref(), artist.as_deref(), album_artist.as_deref(), album.as_deref(), Path::new(&path)),
                    duration_ms: ms(r.get(5)?),
                    artwork_url: None,
                })
            })
            .ok(),
        Track::Episode(id) => db
            .conn()
            .query_row(
                "SELECT e.title, p.title, p.author, e.duration_ms, p.artwork_url FROM episodes e JOIN podcasts p ON p.id = e.podcast_id WHERE e.id = ?1",
                [id],
                |r| {
                    let (title, show, author): (String, String, Option<String>) = (r.get(0)?, r.get(1)?, r.get(2)?);
                    Ok(Info { display: episode_display(&title, &show, author.as_deref()), duration_ms: ms(r.get(3)?), artwork_url: r.get(4)? })
                },
            )
            .ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::db::Db;

    #[test]
    fn songs_and_episodes_are_described_from_the_library() {
        let db = Db::open_in_memory().unwrap();
        let f = crate::library::folders::add_folder(&db, r"E:\Nhạc", 1).unwrap().id;
        db.conn()
            .execute(
                "INSERT INTO songs (id, folder_id, path, size, mtime, added_at, title, artist, album, duration_ms) VALUES (5, ?1, 'E:\\Nhạc\\a.mp3', 1, 1, 1, 'A', 'B', 'C', 200000)",
                [f],
            )
            .unwrap();
        db.conn()
            .execute_batch(
                "INSERT INTO podcasts (id, feed_url, title, author, artwork_url, added_at) VALUES (2, 'https://x/feed', 'Show', NULL, 'https://x/a.jpg', 1);
                 INSERT INTO episodes (id, podcast_id, guid, title, duration_ms, audio_url) VALUES (8, 2, 'g', 'Ep', NULL, 'https://x/e.mp3');",
            )
            .unwrap();
        let song = describe(&db, Track::Song(5)).unwrap();
        assert_eq!(song.display, Display { title: "A".into(), artist: "B".into(), album: "C".into() });
        assert_eq!((song.duration_ms, song.artwork_url), (Some(200_000), None));
        let ep = describe(&db, Track::Episode(8)).unwrap();
        assert_eq!(ep.display, Display { title: "Ep".into(), artist: "Show".into(), album: "Show".into() });
        assert_eq!((ep.duration_ms, ep.artwork_url.as_deref()), (None, Some("https://x/a.jpg")));
        assert_eq!(describe(&db, Track::Song(99)), None);
    }

    #[test]
    fn an_episode_wins_over_a_song_id() {
        assert_eq!(track(Some(3), None), Some(Track::Song(3)));
        assert_eq!(track(None, Some(9)), Some(Track::Episode(9)));
        assert_eq!(track(Some(3), Some(9)), Some(Track::Episode(9)));
        assert_eq!(track(None, None), None);
    }

    #[test]
    fn status_follows_the_player() {
        let song = Some(Track::Song(1));
        assert_eq!(status(false, false, song), Status::Closed, "no output device");
        assert_eq!(status(true, false, None), Status::Stopped);
        assert_eq!(status(true, true, song), Status::Playing);
        assert_eq!(status(true, false, song), Status::Paused);
    }

    #[test]
    fn buttons_call_the_player_commands_of_the_ui() {
        assert_eq!(command(Button::Play), Command::Resume);
        assert_eq!(command(Button::Pause), Command::Pause);
        assert_eq!(command(Button::Stop), Command::Pause, "stop keeps the queue, like pause");
        assert_eq!(command(Button::Next), Command::Skip(1));
        assert_eq!(command(Button::Previous), Command::Skip(-1));
    }

    #[test]
    fn a_tagged_song_shows_its_tags() {
        let d = song_display(Some("Hạ Trắng"), Some("Khánh Ly"), Some("Various"), Some("Trịnh Công Sơn"), Path::new(r"C:\m\01.mp3"));
        assert_eq!(d, Display { title: "Hạ Trắng".into(), artist: "Khánh Ly".into(), album: "Trịnh Công Sơn".into() });
    }

    #[test]
    fn an_untagged_song_shows_its_file_name_and_the_album_artist() {
        let d = song_display(None, Some("  "), Some("Album Artist"), None, Path::new(r"C:\m\Some Song.flac"));
        assert_eq!(d, Display { title: "Some Song".into(), artist: "Album Artist".into(), album: String::new() });
        let d = song_display(Some(""), None, None, None, Path::new(r"C:\m\x.mp3"));
        assert_eq!(d.title, "x");
        assert_eq!(d.artist, "");
    }

    #[test]
    fn an_episode_shows_the_show_as_album_and_its_author() {
        assert_eq!(episode_display("Ep 1", "The Show", Some("Host")), Display { title: "Ep 1".into(), artist: "Host".into(), album: "The Show".into() });
        assert_eq!(episode_display("Chapter 2", "A Book", None).artist, "A Book", "no author: the show's name");
        assert_eq!(episode_display("Chapter 2", "A Book", Some(" ")).artist, "A Book");
    }

    #[test]
    fn the_timeline_is_sent_on_changes_and_now_and_then() {
        let mut c = TimelineClock::default();
        let s = Some(Track::Song(1));
        assert!(c.due(s, true, 0, 0), "first state");
        assert!(!c.due(s, true, 250, 250), "playing on as expected");
        assert!(!c.due(s, true, 4_000, 4_000));
        assert!(c.due(s, true, 5_000, 5_000), "every few seconds");
        assert!(c.due(s, false, 5_200, 5_200), "paused");
        assert!(!c.due(s, false, 5_200, 7_000), "paused, not moving");
        assert!(c.due(s, false, 60_000, 7_250), "seek while paused");
        assert!(c.due(s, true, 60_000, 7_500), "resumed");
        assert!(c.due(s, true, 10_000, 7_750), "seek back while playing");
        assert!(c.due(Some(Track::Song(2)), true, 0, 8_000), "next song");
        assert!(c.due(None, false, 0, 8_250), "stopped");
    }
}

#[cfg(test)]
mod app_id_tests {
    #[test]
    fn the_app_id_is_the_one_the_installer_gives_the_start_menu_shortcut() {
        // The NSIS installer stamps the shortcut with the bundle identifier; Windows ties the media
        // controls (and the taskbar button) to the installed app only when the process says the same.
        let conf: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(super::APP_ID, conf["identifier"].as_str().unwrap());
    }
}
