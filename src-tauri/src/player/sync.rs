//! Sync to a USB mass-storage music player: mirror the liked songs into the player's music folder.
//! The plan is the iPhone's (`sync::plan`, keyed by the player's id); Miu records every file it
//! copies in `player_files`, and only ever removes those, and only while they are unchanged.
//! Files are written under a temporary name, their size checked, then renamed into place.

use std::path::{Path, PathBuf};

use rusqlite::OptionalExtension;
use serde::Serialize;

use crate::library::db::Db;
use crate::player::layout::{numbered, song_path, SongName};
use crate::player::profile::Profile;
use crate::sync::itunes::{sendable_path_with, Convert};
use crate::sync::plan::{check_space, plan_with, record_run, Plan, Step, StepResult};

/// Room left on the player after a sync.
pub const MARGIN_BYTES: u64 = 16 * 1024 * 1024;

/// The player's drive. Paths are relative to the drive's root.
pub trait PlayerFs {
    /// Size of the file at `path`; `None` when there is none.
    fn len(&self, path: &Path) -> Option<u64>;
    /// Copy `src` (a file on the PC) to `dst`, flushed to the drive. Returns the bytes written.
    fn copy_in(&self, src: &Path, dst: &Path) -> Result<u64, String>;
    fn rename(&self, from: &Path, to: &Path) -> Result<(), String>;
    fn remove(&self, path: &Path) -> Result<(), String>;
    fn make_dirs(&self, dir: &Path) -> Result<(), String>;
    /// Remove `dir` only if it is empty.
    fn remove_empty_dir(&self, dir: &Path) -> bool;
    fn free_bytes(&self) -> Option<u64>;
}

/// Progress, sent as `player-sync-progress`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "stage", rename_all = "camelCase")]
pub enum Stage {
    Check,
    Copy { done: usize, total: usize },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub added: usize,
    pub replaced: usize,
    pub updated: usize,
    pub removed: usize,
    /// (title, reason) of songs that did not go.
    pub failed: Vec<(String, String)>,
    /// Files Miu put there that someone changed since: kept, no longer Miu's.
    pub left_alone: usize,
    /// Songs Miu copied that were deleted on the player: copied again.
    pub gone_from_player: usize,
}

/// Why a sync did not start.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Stop {
    NoSpace { need: u64, available: u64 },
    Failed { reason: String },
}

fn failed(e: impl ToString) -> Stop {
    Stop::Failed { reason: e.to_string() }
}

/// What a sync would do on the player `id`.
pub fn plan_for(db: &Db, id: &str, profile: &Profile) -> rusqlite::Result<Plan> {
    plan_with(db, id, &|format, rate, bits| profile.action(format, rate, bits))
}

fn key(p: &Path) -> String {
    p.to_string_lossy().replace('/', "\\").to_lowercase()
}

fn inside(path: &Path, folder: &Path) -> bool {
    let (p, f) = (key(path), key(folder));
    f.is_empty() || p.starts_with(&format!("{}\\", f.trim_end_matches('\\')))
}

fn player_file(db: &Db, id: &str, song_id: i64) -> Result<Option<(PathBuf, u64)>, String> {
    db.conn()
        .query_row("SELECT path, bytes FROM player_files WHERE device = ?1 AND song_id = ?2", rusqlite::params![id, song_id], |r| {
            Ok((PathBuf::from(r.get::<_, String>(0)?), r.get::<_, i64>(1)? as u64))
        })
        .optional()
        .map_err(|e| e.to_string())
}

fn forget_file(db: &Db, id: &str, song_id: i64) -> Result<(), String> {
    db.conn().execute("DELETE FROM player_files WHERE device = ?1 AND song_id = ?2", rusqlite::params![id, song_id]).map(|_| ()).map_err(|e| e.to_string())
}

fn song_name(db: &Db, song_id: i64) -> Result<(String, SongName), String> {
    db.conn()
        .query_row("SELECT title, artist, album_artist, album, track_no, disc_no, path FROM songs WHERE id = ?1", [song_id], |r| {
            let path: String = r.get(6)?;
            let file_stem = Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let name = SongName { title: r.get(0)?, artist: r.get(1)?, album_artist: r.get(2)?, album: r.get(3)?, track_no: r.get(4)?, disc_no: r.get(5)?, file_stem };
            Ok((name.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| name.file_stem.clone()), name))
        })
        .map_err(|e| e.to_string())
}

/// Before planning: forget songs deleted on the player (so they go again), and mark songs left in
/// an old music folder as changed (so they move to the current one).
fn reconcile(db: &Db, id: &str, folder: &Path, fs: &dyn PlayerFs) -> Result<usize, String> {
    let rows: Vec<(i64, PathBuf)> = {
        let mut stmt = db.conn().prepare("SELECT song_id, path FROM player_files WHERE device = ?1").map_err(|e| e.to_string())?;
        let rows = stmt.query_map([id], |r| Ok((r.get(0)?, PathBuf::from(r.get::<_, String>(1)?)))).map_err(|e| e.to_string())?;
        rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
    };
    let mut gone = 0;
    for (song_id, path) in rows {
        if fs.len(&path).is_none() {
            forget_file(db, id, song_id)?;
            db.conn().execute("DELETE FROM synced WHERE udid = ?1 AND song_id = ?2", rusqlite::params![id, song_id]).map_err(|e| e.to_string())?;
            gone += 1;
        } else if !inside(&path, folder) {
            db.conn().execute("UPDATE synced SET meta_hash = '' WHERE udid = ?1 AND song_id = ?2", rusqlite::params![id, song_id]).map_err(|e| e.to_string())?;
        }
    }
    Ok(gone)
}

/// Remove empty folders from `dir` up to, not including, the music folder.
fn prune(fs: &dyn PlayerFs, mut dir: Option<&Path>, folder: &Path) {
    while let Some(d) = dir {
        if !inside(d, folder) || !fs.remove_empty_dir(d) {
            break;
        }
        dir = d.parent();
    }
}

/// Delete a file Miu copied, if it is still the one Miu wrote. Returns false when it was left alone.
fn remove_ours(db: &Db, id: &str, song_id: i64, folder: &Path, fs: &dyn PlayerFs) -> Result<bool, String> {
    let Some((path, bytes)) = player_file(db, id, song_id)? else { return Ok(true) };
    let ours = match fs.len(&path) {
        Some(n) if n == bytes => {
            fs.remove(&path)?;
            prune(fs, path.parent(), folder);
            true
        }
        Some(_) => false,
        None => true,
    };
    forget_file(db, id, song_id)?;
    Ok(ours)
}

/// Copy one song into place; returns where it went.
fn put(db: &Db, id: &str, song_id: i64, src: &Path, folder: &Path, fs: &dyn PlayerFs) -> Result<(), String> {
    let (_, name) = song_name(db, song_id)?;
    let ext = src.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let wanted = folder.join(song_path(&name, &ext));
    let src_len = std::fs::metadata(src).map_err(|e| format!("cannot read the song: {e}"))?.len();
    let old = player_file(db, id, song_id)?;
    let ours = |p: &Path| old.as_ref().is_some_and(|(o, bytes)| key(o) == key(p) && fs.len(p) == Some(*bytes));
    let mut dst = wanted.clone();
    let mut n = 2;
    while fs.len(&dst).is_some() && !ours(&dst) {
        dst = numbered(&wanted, n);
        n += 1;
    }
    let part = dst.with_file_name(format!("{}.miu-part", dst.file_name().unwrap_or_default().to_string_lossy()));
    if let Some(parent) = dst.parent() {
        fs.make_dirs(parent)?;
    }
    let written = fs.copy_in(src, &part).and_then(|_| match fs.len(&part) {
        Some(n) if n == src_len => Ok(()),
        Some(n) => Err(format!("the copy on the player has {n} bytes, not {src_len}")),
        None => Err("the copy did not arrive on the player".into()),
    });
    if let Err(e) = written {
        let _ = fs.remove(&part);
        return Err(e);
    }
    if fs.len(&dst).is_some() {
        fs.remove(&dst)?;
    }
    fs.rename(&part, &dst)?;
    if let Some((old_path, _)) = &old {
        if key(old_path) != key(&dst) {
            remove_ours(db, id, song_id, folder, fs)?;
        }
    }
    db.conn()
        .execute(
            "INSERT INTO player_files (device, song_id, path, bytes) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(device, song_id) DO UPDATE SET path = excluded.path, bytes = excluded.bytes",
            rusqlite::params![id, song_id, dst.to_string_lossy(), src_len as i64],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Sync the liked songs to the player `id`, whose music folder is `profile.folder` on `fs`.
pub fn run(db: &Db, id: &str, profile: &Profile, fs: &dyn PlayerFs, convert: Option<Convert>, progress: &mut dyn FnMut(&Stage), now_unix: u64) -> Result<Report, Stop> {
    progress(&Stage::Check);
    let folder = PathBuf::from(&profile.folder);
    let mut report = Report { gone_from_player: reconcile(db, id, &folder, fs).map_err(failed)?, ..Report::default() };
    let plan = plan_for(db, id, profile).map_err(failed)?;
    if let Some(free) = fs.free_bytes() {
        check_space(&plan, free, MARGIN_BYTES).map_err(|s| Stop::NoSpace { need: s.need, available: s.available })?;
    }
    let decide = |format, rate, bits| profile.action(format, rate, bits);
    let total = plan.steps.len();
    let mut results = Vec::with_capacity(total);
    for (i, step) in plan.steps.iter().enumerate() {
        progress(&Stage::Copy { done: i, total });
        let song_id = step.song_id();
        let outcome = match step {
            Step::Remove { .. } => remove_ours(db, id, song_id, &folder, fs).map(|ours| {
                report.removed += 1;
                if !ours {
                    report.left_alone += 1;
                }
            }),
            _ => sendable_path_with(db, song_id, &decide, convert).and_then(|src| put(db, id, song_id, &src, &folder, fs)).map(|_| match step {
                Step::Add { .. } => report.added += 1,
                Step::Replace { .. } => report.replaced += 1,
                _ => report.updated += 1,
            }),
        };
        if let Err(e) = &outcome {
            let title = song_name(db, song_id).map(|(t, _)| t).unwrap_or_else(|_| format!("#{song_id}"));
            report.failed.push((title, e.clone()));
        }
        results.push(StepResult { song_id, outcome: outcome.map(|_| None) });
    }
    record_run(db, id, &plan, &results, now_unix).map_err(failed)?;
    progress(&Stage::Copy { done: total, total });
    Ok(report)
}

/// The real drive.
pub struct DriveFs {
    pub root: PathBuf,
    pub free: Box<dyn Fn() -> Option<u64> + Send + Sync>,
}

impl PlayerFs for DriveFs {
    fn len(&self, path: &Path) -> Option<u64> {
        std::fs::metadata(self.root.join(path)).ok().filter(|m| m.is_file()).map(|m| m.len())
    }
    fn copy_in(&self, src: &Path, dst: &Path) -> Result<u64, String> {
        use std::io::Write;
        let mut from = std::fs::File::open(src).map_err(|e| e.to_string())?;
        let to = std::fs::File::create(self.root.join(dst)).map_err(|e| e.to_string())?;
        let mut w = std::io::BufWriter::with_capacity(1 << 20, to);
        let n = std::io::copy(&mut from, &mut w).map_err(|e| e.to_string())?;
        w.flush().map_err(|e| e.to_string())?;
        // Flushed to the stick before the size is checked, so pulling it out later loses nothing.
        w.get_ref().sync_all().map_err(|e| e.to_string())?;
        Ok(n)
    }
    fn rename(&self, from: &Path, to: &Path) -> Result<(), String> {
        std::fs::rename(self.root.join(from), self.root.join(to)).map_err(|e| e.to_string())
    }
    fn remove(&self, path: &Path) -> Result<(), String> {
        std::fs::remove_file(self.root.join(path)).map_err(|e| e.to_string())
    }
    fn make_dirs(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(self.root.join(dir)).map_err(|e| e.to_string())
    }
    fn remove_empty_dir(&self, dir: &Path) -> bool {
        // remove_dir refuses folders that still hold anything.
        std::fs::remove_dir(self.root.join(dir)).is_ok()
    }
    fn free_bytes(&self) -> Option<u64> {
        (self.free)()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use crate::audio::fixtures::wav;
    use crate::library::folders::add_folder;
    use crate::library::scan::rescan_folder;
    use crate::library::songs::{list_songs, set_rating, Rating};
    use crate::player::profile::Target;

    /// A drive in memory: files (by lower-cased path) with their size, and folders.
    #[derive(Default)]
    pub(crate) struct FakeDrive {
        files: RefCell<BTreeMap<String, (PathBuf, u64)>>,
        dirs: RefCell<BTreeMap<String, PathBuf>>,
        pub free: Option<u64>,
        /// Copies whose name contains this arrive one byte short.
        pub short: Option<String>,
    }

    impl FakeDrive {
        pub fn put(&self, path: &str, bytes: u64) {
            let p = PathBuf::from(path);
            let mut parent = p.parent();
            while let Some(d) = parent.filter(|d| !d.as_os_str().is_empty()) {
                self.dirs.borrow_mut().insert(key(d), d.to_path_buf());
                parent = d.parent();
            }
            self.files.borrow_mut().insert(key(&p), (p, bytes));
        }
        pub fn paths(&self) -> Vec<String> {
            self.files.borrow().values().map(|(p, _)| p.to_string_lossy().replace('\\', "/")).collect()
        }
        pub fn has_dir(&self, path: &str) -> bool {
            self.dirs.borrow().contains_key(&key(Path::new(path)))
        }
    }

    impl PlayerFs for FakeDrive {
        fn len(&self, path: &Path) -> Option<u64> {
            self.files.borrow().get(&key(path)).map(|f| f.1)
        }
        fn copy_in(&self, src: &Path, dst: &Path) -> Result<u64, String> {
            let mut n = std::fs::metadata(src).map_err(|e| e.to_string())?.len();
            if self.short.as_ref().is_some_and(|s| dst.to_string_lossy().contains(s.as_str())) {
                n -= 1;
            }
            if !self.dirs.borrow().contains_key(&key(dst.parent().unwrap())) {
                return Err("no such folder".into());
            }
            self.files.borrow_mut().insert(key(dst), (dst.to_path_buf(), n));
            Ok(n)
        }
        fn rename(&self, from: &Path, to: &Path) -> Result<(), String> {
            let (_, n) = self.files.borrow_mut().remove(&key(from)).ok_or("no such file")?;
            self.files.borrow_mut().insert(key(to), (to.to_path_buf(), n));
            Ok(())
        }
        fn remove(&self, path: &Path) -> Result<(), String> {
            self.files.borrow_mut().remove(&key(path)).map(|_| ()).ok_or_else(|| "no such file".into())
        }
        fn make_dirs(&self, dir: &Path) -> Result<(), String> {
            self.put(&dir.join("x").to_string_lossy(), 0);
            self.files.borrow_mut().remove(&key(&dir.join("x")));
            Ok(())
        }
        fn remove_empty_dir(&self, dir: &Path) -> bool {
            let prefix = format!("{}\\", key(dir));
            let busy = self.files.borrow().keys().any(|k| k.starts_with(&prefix)) || self.dirs.borrow().keys().any(|k| k.starts_with(&prefix));
            !busy && self.dirs.borrow_mut().remove(&key(dir)).is_some()
        }
        fn free_bytes(&self) -> Option<u64> {
            self.free
        }
    }

    const ID: &str = "vol:1A2B3C4D";

    struct Lib {
        dir: tempfile::TempDir,
        db: Db,
        ids: Vec<i64>,
    }

    /// Songs a.wav, b.wav, c.wav (in id order), tagged by `tags` (title, artist, album, track).
    fn library(tags: &[(&str, &str, &str, u32)]) -> Lib {
        let dir = tempfile::tempdir().unwrap();
        for (i, name) in ["a.wav", "b.wav", "c.wav"].iter().enumerate() {
            wav(dir.path(), name, 44_100, 2, 16, 4_410 + i * 100);
        }
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &dir.path().to_string_lossy(), 1).unwrap();
        rescan_folder(&db, f.id, &f.path, 1).unwrap();
        crate::library::duplicates::fill_audio_hashes(&db, 10).unwrap();
        let mut ids: Vec<i64> = list_songs(&db).unwrap().iter().map(|s| s.id).collect();
        ids.sort();
        for (id, (title, artist, album, track)) in ids.iter().zip(tags) {
            db.conn().execute("UPDATE songs SET title = ?1, artist = ?2, album = ?3, track_no = ?4 WHERE id = ?5", rusqlite::params![title, artist, album, track, id]).unwrap();
        }
        Lib { dir, db, ids }
    }

    fn tagged() -> Lib {
        library(&[("Diễm xưa", "Khánh Ly", "Tình ca", 3), ("Hạ trắng", "Khánh Ly", "Tình ca", 4), ("Song", "Other", "Album", 1)])
    }

    fn like(lib: &Lib, i: usize) {
        set_rating(&lib.db, lib.ids[i], Rating::Liked).unwrap();
    }

    /// Plays WAV, so nothing needs converting.
    fn wav_player() -> Profile {
        Profile { folder: "MUSIC".into(), target: Target::Mp3V0, plays: vec!["wav".into()] }
    }

    fn sync(lib: &Lib, drive: &FakeDrive, profile: &Profile) -> Result<Report, Stop> {
        run(&lib.db, ID, profile, drive, None, &mut |_| {}, 100)
    }

    fn size(lib: &Lib, name: &str) -> u64 {
        std::fs::metadata(lib.dir.path().join(name)).unwrap().len()
    }

    #[test]
    fn liked_songs_are_copied_as_artist_album_track_title() {
        let lib = tagged();
        like(&lib, 0);
        like(&lib, 2);
        let drive = FakeDrive::default();
        let r = sync(&lib, &drive, &wav_player()).unwrap();
        assert_eq!(r.added, 2);
        assert_eq!(drive.paths(), vec!["MUSIC/Khánh Ly/Tình ca/03 Diễm xưa.wav", "MUSIC/Other/Album/01 Song.wav"]);
        assert_eq!(drive.len(Path::new("MUSIC/Khánh Ly/Tình ca/03 Diễm xưa.wav")), Some(size(&lib, "a.wav")), "sizes checked after copying");
        assert!(plan_for(&lib.db, ID, &wav_player()).unwrap().steps.is_empty(), "a second sync has nothing to do");
        assert_eq!(sync(&lib, &drive, &wav_player()).unwrap(), Report::default());
    }

    #[test]
    fn unliking_removes_only_miu_s_file_and_its_emptied_folders() {
        let lib = tagged();
        like(&lib, 0);
        like(&lib, 2);
        let drive = FakeDrive::default();
        drive.put("MUSIC/Khánh Ly/Tình ca/cover.jpg", 10);
        drive.put("MUSIC/mine.mp3", 5);
        sync(&lib, &drive, &wav_player()).unwrap();
        set_rating(&lib.db, lib.ids[0], Rating::Disliked).unwrap();
        set_rating(&lib.db, lib.ids[2], Rating::NotRated).unwrap();
        let r = sync(&lib, &drive, &wav_player()).unwrap();
        assert_eq!((r.removed, r.left_alone), (2, 0));
        assert_eq!(drive.paths(), vec!["MUSIC/Khánh Ly/Tình ca/cover.jpg", "MUSIC/mine.mp3"], "other files are never touched");
        assert!(drive.has_dir("MUSIC/Khánh Ly/Tình ca"), "a folder that still holds something stays");
        assert!(!drive.has_dir("MUSIC/Other"), "emptied folders go");
        assert!(drive.has_dir("MUSIC"), "the music folder itself stays");
    }

    #[test]
    fn a_file_changed_on_the_player_is_left_alone() {
        let lib = tagged();
        like(&lib, 0);
        let drive = FakeDrive::default();
        sync(&lib, &drive, &wav_player()).unwrap();
        drive.put("MUSIC/Khánh Ly/Tình ca/03 Diễm xưa.wav", 7);
        set_rating(&lib.db, lib.ids[0], Rating::NotRated).unwrap();
        let r = sync(&lib, &drive, &wav_player()).unwrap();
        assert_eq!((r.removed, r.left_alone), (1, 1));
        assert_eq!(drive.paths(), vec!["MUSIC/Khánh Ly/Tình ca/03 Diễm xưa.wav"]);
    }

    #[test]
    fn a_file_that_is_not_miu_s_is_never_overwritten() {
        let lib = tagged();
        like(&lib, 0);
        let drive = FakeDrive::default();
        drive.put("MUSIC/Khánh Ly/Tình ca/03 Diễm xưa.wav", 7);
        sync(&lib, &drive, &wav_player()).unwrap();
        assert_eq!(drive.len(Path::new("MUSIC/Khánh Ly/Tình ca/03 Diễm xưa.wav")), Some(7));
        assert_eq!(drive.len(Path::new("MUSIC/Khánh Ly/Tình ca/03 Diễm xưa (2).wav")), Some(size(&lib, "a.wav")));
    }

    #[test]
    fn a_song_deleted_on_the_player_is_copied_again() {
        let lib = tagged();
        like(&lib, 0);
        let drive = FakeDrive::default();
        sync(&lib, &drive, &wav_player()).unwrap();
        drive.remove(Path::new("MUSIC/Khánh Ly/Tình ca/03 Diễm xưa.wav")).unwrap();
        let r = sync(&lib, &drive, &wav_player()).unwrap();
        assert_eq!((r.gone_from_player, r.added), (1, 1));
        assert_eq!(drive.paths(), vec!["MUSIC/Khánh Ly/Tình ca/03 Diễm xưa.wav"]);
    }

    #[test]
    fn edited_tags_move_the_file_to_its_new_name() {
        let lib = tagged();
        like(&lib, 0);
        let drive = FakeDrive::default();
        sync(&lib, &drive, &wav_player()).unwrap();
        lib.db.conn().execute("UPDATE songs SET title = 'Diễm xưa (live)' WHERE id = ?1", [lib.ids[0]]).unwrap();
        let r = sync(&lib, &drive, &wav_player()).unwrap();
        assert_eq!(r.updated, 1);
        assert_eq!(drive.paths(), vec!["MUSIC/Khánh Ly/Tình ca/03 Diễm xưa (live).wav"]);
    }

    #[test]
    fn a_new_music_folder_moves_the_songs_there() {
        let lib = tagged();
        like(&lib, 2);
        let drive = FakeDrive::default();
        sync(&lib, &drive, &wav_player()).unwrap();
        let moved = Profile { folder: "Songs".into(), ..wav_player() };
        sync(&lib, &drive, &moved).unwrap();
        assert_eq!(drive.paths(), vec!["Songs/Other/Album/01 Song.wav"]);
        assert!(drive.has_dir("MUSIC"), "the old music folder itself is not deleted");
    }

    #[test]
    fn formats_the_player_cannot_play_go_converted_and_fail_without_ffmpeg() {
        let lib = tagged();
        like(&lib, 0);
        like(&lib, 2);
        let drive = FakeDrive::default();
        let mp3_only = Profile { plays: vec![], ..wav_player() };
        let r = run(&lib.db, ID, &mp3_only, &drive, None, &mut |_| {}, 100).unwrap();
        assert_eq!(r.added, 0);
        assert_eq!(r.failed.len(), 2);
        assert!(r.failed[0].1.contains("ffmpeg"), "{:?}", r.failed);
        let converted = lib.dir.path().join("converted.mp3");
        std::fs::write(&converted, b"mp3 bytes").unwrap();
        let seen = RefCell::new(Vec::new());
        let convert = |s: &crate::sync::itunes::SongSource| {
            seen.borrow_mut().push(s.action);
            Ok(converted.clone())
        };
        let r = run(&lib.db, ID, &mp3_only, &drive, Some(&convert), &mut |_| {}, 101).unwrap();
        assert_eq!(r.added, 2);
        assert!(seen.borrow().iter().all(|a| matches!(a, crate::library::convert::Action::Mp3 { .. })));
        assert_eq!(drive.paths(), vec!["MUSIC/Khánh Ly/Tình ca/03 Diễm xưa.mp3", "MUSIC/Other/Album/01 Song.mp3"]);
    }

    #[test]
    fn a_copy_of_the_wrong_size_is_deleted_and_tried_again_next_time() {
        let lib = tagged();
        like(&lib, 0);
        like(&lib, 2);
        let drive = FakeDrive { short: Some("Diễm".into()), ..Default::default() };
        let r = sync(&lib, &drive, &wav_player()).unwrap();
        assert_eq!(r.added, 1);
        assert_eq!(r.failed.len(), 1);
        assert_eq!(r.failed[0].0, "Diễm xưa");
        assert_eq!(drive.paths(), vec!["MUSIC/Other/Album/01 Song.wav"], "no partial file left");
        let next = plan_for(&lib.db, ID, &wav_player()).unwrap();
        assert_eq!(next.steps.iter().map(|s| s.song_id()).collect::<Vec<_>>(), vec![lib.ids[0]]);
    }

    #[test]
    fn a_sync_that_does_not_fit_stops_before_copying_anything() {
        let lib = tagged();
        like(&lib, 0);
        let drive = FakeDrive { free: Some(MARGIN_BYTES), ..Default::default() };
        assert_eq!(sync(&lib, &drive, &wav_player()), Err(Stop::NoSpace { need: size(&lib, "a.wav"), available: 0 }));
        assert!(drive.paths().is_empty());
    }

    #[test]
    fn progress_counts_the_steps() {
        let lib = tagged();
        like(&lib, 0);
        like(&lib, 1);
        let drive = FakeDrive::default();
        let mut stages = Vec::new();
        run(&lib.db, ID, &wav_player(), &drive, None, &mut |s| stages.push(s.clone()), 100).unwrap();
        assert_eq!(stages, vec![Stage::Check, Stage::Copy { done: 0, total: 2 }, Stage::Copy { done: 1, total: 2 }, Stage::Copy { done: 2, total: 2 }]);
    }

    /// Syncs two songs to the first removable drive (a USB stick will do) into a `MiuTest-…`
    /// folder, then unlikes them and checks the drive is left as it was. Passes, saying so, when
    /// no removable drive is plugged in. `cargo test --lib player::sync -- --ignored --nocapture`
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn syncs_to_a_real_removable_drive_and_cleans_up() {
        let Some(v) = crate::player::volumes::list().into_iter().find(|v| v.removable) else {
            println!("no removable drive plugged in: nothing to test");
            return;
        };
        println!("testing on {} {:?}", v.root.display(), v.label);
        let folder = format!("MiuTest-{}", std::process::id());
        let profile = Profile { folder: folder.clone(), ..wav_player() };
        let root = v.root.clone();
        let drive = DriveFs { root: v.root.clone(), free: Box::new(move || crate::player::volumes::free_bytes(&root)) };
        let lib = tagged();
        like(&lib, 0);
        like(&lib, 2);
        let r = run(&lib.db, ID, &profile, &drive, None, &mut |_| {}, 100).unwrap();
        assert_eq!((r.added, r.failed.len()), (2, 0), "{r:?}");
        let song = v.root.join(&folder).join("Khánh Ly").join("Tình ca").join("03 Diễm xưa.wav");
        assert_eq!(std::fs::read(&song).unwrap(), std::fs::read(lib.dir.path().join("a.wav")).unwrap(), "byte for byte");
        set_rating(&lib.db, lib.ids[0], Rating::NotRated).unwrap();
        set_rating(&lib.db, lib.ids[2], Rating::NotRated).unwrap();
        assert_eq!(run(&lib.db, ID, &profile, &drive, None, &mut |_| {}, 101).unwrap().removed, 2);
        let left: Vec<_> = std::fs::read_dir(v.root.join(&folder)).unwrap().collect();
        assert!(left.is_empty(), "artist and album folders removed: {left:?}");
        std::fs::remove_dir(v.root.join(&folder)).unwrap();
    }

    #[test]
    fn the_real_drive_copies_flushes_and_cleans_up() {
        let root = tempfile::tempdir().unwrap();
        let src = root.path().join("src.bin");
        std::fs::write(&src, vec![7u8; 3_000]).unwrap();
        let drive = DriveFs { root: root.path().join("E"), free: Box::new(|| Some(1)) };
        drive.make_dirs(Path::new("Music\\A\\B")).unwrap();
        assert_eq!(drive.copy_in(&src, Path::new("Music\\A\\B\\x.part")).unwrap(), 3_000);
        drive.rename(Path::new("Music\\A\\B\\x.part"), Path::new("Music\\A\\B\\x.mp3")).unwrap();
        assert_eq!(drive.len(Path::new("Music\\A\\B\\x.mp3")), Some(3_000));
        assert!(!drive.remove_empty_dir(Path::new("Music\\A\\B")), "not empty");
        drive.remove(Path::new("Music\\A\\B\\x.mp3")).unwrap();
        assert!(drive.remove_empty_dir(Path::new("Music\\A\\B")));
        assert_eq!(drive.len(Path::new("Music\\A")), None, "folders are not files");
        assert_eq!(drive.free_bytes(), Some(1));
    }
}
