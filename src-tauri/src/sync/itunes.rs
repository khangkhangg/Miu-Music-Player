//! Sync through iTunes with "Manually manage music" on (Phase 4b, S.2). iTunes does the
//! authentication and signing; Miu tells it which files to put on, or take off, the phone.
//! The real iTunes is `apple::itunes_com::ITunesCom`; tests use a fake.
//!
//! Songs already on the phone (put there by 3uTools or iTunes) are *adopted*: recorded as present so
//! they are not sent twice, and never removed by Miu, even when they are no longer liked.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::library::convert::{action, Action, Policy};
use crate::library::format::Format;
use crate::sync::guard::same_recording;
use crate::library::db::Db;
use crate::sync::plan::{Step, StepResult};

/// A track in iTunes' view of the phone. `id` is iTunes' persistent ID (= the phone's sync_id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceTrack {
    pub id: i64,
    pub name: String,
    pub size: u64,
    pub duration_ms: Option<i64>,
}

/// What Miu needs from iTunes.
pub trait ITunes {
    fn device_tracks(&self) -> Result<Vec<DeviceTrack>, String>;
    /// Put a file on the phone; returns the new device track's persistent ID.
    fn add(&self, path: &Path) -> Result<i64, String>;
    /// Take a track off the phone. `Ok(false)` when it was not there.
    fn remove(&self, id: i64) -> Result<bool, String>;
    /// Run the device sync that applies the changes (UpdateIPod).
    fn sync_device(&self) -> Result<(), String>;
}

/// Record liked songs that are already on the phone as adopted. Returns how many were adopted.
pub fn adopt(db: &Db, udid: &str, device: &[DeviceTrack], now_unix: u64) -> Result<usize, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let hashes = crate::sync::plan::liked_hashes(db).map_err(e)?;
    let (tracked_songs, claimed): (HashSet<i64>, HashSet<i64>) = {
        let mut stmt = db.conn().prepare("SELECT song_id, device_pid FROM synced WHERE udid = ?1").map_err(e)?;
        let rows: Vec<(i64, Option<i64>)> = stmt.query_map([udid], |r| Ok((r.get(0)?, r.get(1)?))).map_err(e)?.collect::<rusqlite::Result<_>>().map_err(e)?;
        (rows.iter().map(|r| r.0).collect(), rows.iter().filter_map(|r| r.1).collect())
    };
    let mut free: Vec<&DeviceTrack> = device.iter().filter(|t| !claimed.contains(&t.id)).collect();
    let mut stmt = db.conn().prepare("SELECT id, COALESCE(title, ''), size, duration_ms FROM songs WHERE rating = 1 ORDER BY id").map_err(e)?;
    let liked: Vec<(i64, String, i64, Option<i64>)> =
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(e)?.collect::<rusqlite::Result<_>>().map_err(e)?;
    let mut adopted = 0;
    for (id, title, size, ms) in liked {
        if tracked_songs.contains(&id) {
            continue;
        }
        let Some(i) = free.iter().position(|t| same_recording((&title, size as u64, ms), (&t.name, t.size, t.duration_ms))) else { continue };
        let t = free.remove(i);
        let (meta, audio) = hashes.get(&id).cloned().unwrap_or_default();
        db.conn()
            .execute(
                "INSERT INTO synced (udid, song_id, device_pid, meta_hash, audio_hash, bytes, synced_at, adopted) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1)",
                rusqlite::params![udid, id, t.id, meta, audio, t.size as i64, now_unix as i64],
            )
            .map_err(e)?;
        adopted += 1;
    }
    Ok(adopted)
}

/// A song that needs converting, as the converter sees it.
#[derive(Debug, Clone)]
pub struct SongSource {
    pub path: PathBuf,
    /// Same audio, same key (the song's audio hash, or its path, size and time when not hashed yet).
    pub audio_key: String,
    /// Changes when the song's tags change.
    pub tags_key: String,
    pub action: Action,
}

/// Turns a song into the file the iPhone can play (`library::transcode::Cache` in the app).
pub type Convert<'a> = &'a dyn Fn(&SongSource) -> Result<PathBuf, String>;

/// The file to hand to iTunes for a song: the original, or its converted copy.
pub fn sendable_path(db: &Db, song_id: i64, policy: &Policy, convert: Option<Convert>) -> Result<PathBuf, String> {
    let (path, format, rate, bits, audio_hash, size, mtime): (String, String, Option<u32>, Option<u32>, Option<String>, i64, i64) = db
        .conn()
        .query_row("SELECT path, format, sample_rate, bit_depth, audio_hash, size, mtime FROM songs WHERE id = ?1", [song_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?))
        })
        .map_err(|e| e.to_string())?;
    let act = action(Format::from_name(&format), rate, bits, policy);
    match act {
        Action::Keep => Ok(PathBuf::from(path)),
        Action::Skip => Err("not a format Miu can read".into()),
        Action::Aac { .. } | Action::Alac { .. } => {
            let convert = convert.ok_or("needs converting for the iPhone, and the ffmpeg sidecar is missing")?;
            let audio_key = audio_hash.filter(|h| !h.is_empty()).unwrap_or_else(|| format!("{path}|{size}|{mtime}"));
            let tags_key = crate::sync::plan::tags_hash(db, song_id).map_err(|e| e.to_string())?;
            convert(&SongSource { path: PathBuf::from(path), audio_key, tags_key, action: act })
        }
    }
}

/// Carry out plan steps through iTunes, one result per step.
/// Removals take the track off; adds send the file; replacements and tag updates take the old
/// copy off and send the file again (iTunes re-reads the tags from the file).
pub fn apply(itunes: &dyn ITunes, steps: &[Step], source: &dyn Fn(i64) -> Result<PathBuf, String>) -> Vec<StepResult> {
    steps
        .iter()
        .map(|step| {
            let outcome = match step {
                Step::Remove { device_pid, .. } => device_pid.map_or(Ok(()), |pid| itunes.remove(pid as i64).map(|_| ())).map(|_| None),
                Step::Add { song_id, .. } => source(*song_id).and_then(|p| itunes.add(&p)).map(|id| Some(id as u64)),
                Step::Replace { song_id, device_pid, .. } | Step::Update { song_id, device_pid, .. } => source(*song_id).and_then(|p| {
                    if let Some(pid) = device_pid {
                        itunes.remove(*pid as i64)?;
                    }
                    itunes.add(&p).map(|id| Some(id as u64))
                }),
            };
            StepResult { song_id: step.song_id(), outcome }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::folders::add_folder;
    use crate::sync::plan::{plan, record_run};
    use std::cell::RefCell;

    const PHONE: &str = "e51e30a5aaaa";

    fn song(db: &Db, path: &str, title: &str, format: &str, size: u64, ms: i64, rating: i64) -> i64 {
        let f = match add_folder(db, r"E:\Nhạc", 1) {
            Ok(f) => f.id,
            Err(_) => db.conn().query_row("SELECT id FROM watched_folders", [], |r| r.get(0)).unwrap(),
        };
        db.conn()
            .execute(
                "INSERT INTO songs (folder_id, path, size, mtime, added_at, format, title, duration_ms, rating, tags_read, sample_rate, bit_depth)
                 VALUES (?1, ?2, ?3, 1, 1, ?4, ?5, ?6, ?7, 1, 44100, 16)",
                rusqlite::params![f, path, size as i64, format, title, ms, rating],
            )
            .unwrap();
        db.conn().last_insert_rowid()
    }

    fn track(id: i64, name: &str, size: u64, ms: i64) -> DeviceTrack {
        DeviceTrack { id, name: name.into(), size, duration_ms: Some(ms) }
    }

    fn kinds(steps: &[Step]) -> Vec<(&'static str, i64)> {
        steps.iter().map(|s| (s.kind(), s.song_id())).collect()
    }

    #[test]
    fn liked_songs_already_on_the_phone_are_adopted_not_sent_again() {
        let db = Db::open_in_memory().unwrap();
        let by_size = song(&db, r"E:\Nhạc\a.mp3", "Áo Mới Cà Mau", "mp3", 9_296_034, 228_884, 1);
        let by_title = song(&db, r"E:\Nhạc\b.mp3", "Diễm Xưa", "mp3", 5, 303_000, 1);
        let new = song(&db, r"E:\Nhạc\c.mp3", "Hạ Trắng", "mp3", 7, 200_000, 1);
        let not_liked = song(&db, r"E:\Nhạc\d.mp3", "Bông Cỏ Mây", "mp3", 4_000_000, 1, 0);
        let device = [
            track(11, "something else", 9_296_034, 1),
            track(12, "diễm xưa", 6_000_000, 304_000),
            track(13, "Bông Cỏ Mây", 4_000_000, 1),
        ];
        assert_eq!(adopt(&db, PHONE, &device, 100).unwrap(), 2);
        let p = plan(&db, PHONE, &Policy::default()).unwrap();
        assert_eq!(kinds(&p.steps), vec![("add", new)], "adopted songs are not added; {by_size} {by_title} {not_liked}");
        let pid: i64 = db.conn().query_row("SELECT device_pid FROM synced WHERE song_id = ?1", [by_size], |r| r.get(0)).unwrap();
        assert_eq!(pid, 11);
        assert_eq!(adopt(&db, PHONE, &device, 200).unwrap(), 0, "adopting twice changes nothing");
    }

    #[test]
    fn an_adopted_song_is_never_removed_when_unliked() {
        let db = Db::open_in_memory().unwrap();
        let s = song(&db, r"E:\Nhạc\a.mp3", "Áo Mới Cà Mau", "mp3", 9_296_034, 228_884, 1);
        assert_eq!(adopt(&db, PHONE, &[track(11, "Áo Mới Cà Mau", 9_296_034, 228_884)], 100).unwrap(), 1);
        crate::library::songs::set_rating(&db, s, crate::library::songs::Rating::NotRated).unwrap();
        assert!(plan(&db, PHONE, &Policy::default()).unwrap().steps.is_empty());
    }

    #[test]
    fn a_device_track_miu_already_tracks_is_not_adopted_twice() {
        let db = Db::open_in_memory().unwrap();
        let sent = song(&db, r"E:\Nhạc\a.mp3", "Hạ Trắng", "mp3", 7, 200_000, 1);
        let p = plan(&db, PHONE, &Policy::default()).unwrap();
        record_run(&db, PHONE, &p, &[StepResult { song_id: sent, outcome: Ok(Some(21)) }], 50).unwrap();
        let twin = song(&db, r"E:\Nhạc\copy\a.mp3", "Hạ Trắng", "mp3", 7, 200_000, 1);
        assert_eq!(adopt(&db, PHONE, &[track(21, "Hạ Trắng", 7, 200_000)], 100).unwrap(), 0, "track 21 is already {sent}'s, not {twin}'s");
    }

    #[test]
    fn songs_that_need_converting_cannot_be_sent_without_a_converter() {
        let db = Db::open_in_memory().unwrap();
        let mp3 = song(&db, r"E:\Nhạc\a.mp3", "a", "mp3", 1, 1, 1);
        let flac = song(&db, r"E:\Nhạc\b.flac", "b", "flac", 1, 1, 1);
        assert_eq!(sendable_path(&db, mp3, &Policy::default(), None).unwrap(), PathBuf::from(r"E:\Nhạc\a.mp3"));
        let err = sendable_path(&db, flac, &Policy::default(), None).unwrap_err();
        assert!(err.contains("convert"), "{err}");
    }

    #[test]
    fn a_flac_goes_as_its_converted_copy_keyed_by_audio_and_tags() {
        let db = Db::open_in_memory().unwrap();
        let flac = song(&db, r"E:\Nhạc\b.flac", "Diễm xưa", "flac", 1, 1, 1);
        db.conn().execute("UPDATE songs SET audio_hash = 'abc' WHERE id = ?1", [flac]).unwrap();
        let seen = RefCell::new(Vec::new());
        let convert = |s: &SongSource| {
            seen.borrow_mut().push((s.path.clone(), s.audio_key.clone(), s.tags_key.clone(), s.action));
            Ok(PathBuf::from(r"C:\cache\x.m4a"))
        };
        assert_eq!(sendable_path(&db, flac, &Policy::default(), Some(&convert)).unwrap(), PathBuf::from(r"C:\cache\x.m4a"));
        db.conn().execute("UPDATE songs SET title = 'Diễm Xưa (live)' WHERE id = ?1", [flac]).unwrap();
        sendable_path(&db, flac, &Policy::default(), Some(&convert)).unwrap();
        let seen = seen.borrow();
        assert_eq!(seen[0].0, PathBuf::from(r"E:\Nhạc\b.flac"));
        assert_eq!(seen[0].1, "abc");
        assert_eq!(seen[0].3, Action::Alac { bits: 16, rate: 44_100 });
        assert_eq!(seen[0].1, seen[1].1, "same audio");
        assert_ne!(seen[0].2, seen[1].2, "a title edit changes the tags key");
    }

    #[derive(Default)]
    struct FakeITunes {
        on_phone: RefCell<Vec<i64>>,
        calls: RefCell<Vec<String>>,
        next_id: RefCell<i64>,
    }

    impl ITunes for FakeITunes {
        fn device_tracks(&self) -> Result<Vec<DeviceTrack>, String> {
            Ok(self.on_phone.borrow().iter().map(|&id| track(id, "x", 1, 1)).collect())
        }
        fn add(&self, path: &Path) -> Result<i64, String> {
            self.calls.borrow_mut().push(format!("add {}", path.display()));
            *self.next_id.borrow_mut() += 1;
            let id = 900 + *self.next_id.borrow();
            self.on_phone.borrow_mut().push(id);
            Ok(id)
        }
        fn remove(&self, id: i64) -> Result<bool, String> {
            self.calls.borrow_mut().push(format!("remove {id}"));
            let mut on = self.on_phone.borrow_mut();
            let before = on.len();
            on.retain(|&x| x != id);
            Ok(on.len() < before)
        }
        fn sync_device(&self) -> Result<(), String> {
            self.calls.borrow_mut().push("sync".into());
            Ok(())
        }
    }

    #[test]
    fn apply_adds_removes_and_replaces_through_itunes() {
        let it = FakeITunes { on_phone: RefCell::new(vec![5, 6]), ..Default::default() };
        let steps = vec![
            Step::Remove { song_id: 1, device_pid: Some(5), bytes: 1 },
            Step::Remove { song_id: 2, device_pid: Some(77), bytes: 1 },
            Step::Add { song_id: 3, bytes: 1, meta_hash: "m".into(), audio_hash: None },
            Step::Replace { song_id: 4, device_pid: Some(6), bytes: 1, old_bytes: 1, meta_hash: "m".into(), audio_hash: None },
            Step::Add { song_id: 9, bytes: 1, meta_hash: "m".into(), audio_hash: None },
        ];
        let source = |id: i64| if id == 9 { Err("needs converting".to_string()) } else { Ok(PathBuf::from(format!("song{id}.mp3"))) };
        let results = apply(&it, &steps, &source);
        let outcomes: Vec<(i64, Result<Option<u64>, String>)> = results.into_iter().map(|r| (r.song_id, r.outcome)).collect();
        assert_eq!(
            outcomes,
            vec![
                (1, Ok(None)),
                (2, Ok(None)),
                (3, Ok(Some(901))),
                (4, Ok(Some(902))),
                (9, Err("needs converting".into())),
            ],
            "a track already gone counts as removed"
        );
        assert_eq!(*it.calls.borrow(), vec!["remove 5", "remove 77", "add song3.mp3", "remove 6", "add song4.mp3"]);
        assert!(!it.calls.borrow().contains(&"sync".to_string()), "apply never starts the device sync itself");
    }

    #[test]
    fn a_tag_update_is_sent_as_a_fresh_copy() {
        let it = FakeITunes { on_phone: RefCell::new(vec![6]), ..Default::default() };
        let steps = vec![Step::Update { song_id: 4, device_pid: Some(6), meta_hash: "m".into() }];
        let results = apply(&it, &steps, &|id| Ok(PathBuf::from(format!("song{id}.mp3"))));
        assert_eq!(results[0].outcome, Ok(Some(901)));
        assert_eq!(*it.calls.borrow(), vec!["remove 6", "add song4.mp3"]);
    }
}
