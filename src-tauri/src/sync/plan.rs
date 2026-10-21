//! The sync plan (F4.1, F4.2, F4.4, F4.6): what to add to, update on, replace on and remove from
//! one iPhone so that it holds exactly the liked songs. Miu remembers what it sent in `synced`, so
//! it only ever removes tracks it added itself — songs put there by iTunes or 3uTools are left alone.

use std::collections::HashMap;

use serde::Serialize;

use crate::library::convert::{action, Action, Mp3Mode, Policy};
use crate::library::db::Db;
use crate::library::fingerprint::fnv64;
use crate::library::format::Format;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Step {
    /// Liked, not on the phone yet.
    Add { song_id: i64, bytes: u64, meta_hash: String, audio_hash: Option<String> },
    /// On the phone, but the audio changed: send the file again.
    Replace { song_id: i64, device_pid: Option<u64>, bytes: u64, old_bytes: u64, meta_hash: String, audio_hash: Option<String> },
    /// On the phone, tags changed (an edit, F9.5): metadata only.
    Update { song_id: i64, device_pid: Option<u64>, meta_hash: String },
    /// Miu sent it, but it is no longer liked or no longer in the library.
    Remove { song_id: i64, device_pid: Option<u64>, bytes: u64 },
}

impl Step {
    pub fn song_id(&self) -> i64 {
        match self {
            Step::Add { song_id, .. } | Step::Replace { song_id, .. } | Step::Update { song_id, .. } | Step::Remove { song_id, .. } => *song_id,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Step::Add { .. } => "add",
            Step::Replace { .. } => "replace",
            Step::Update { .. } => "update",
            Step::Remove { .. } => "remove",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    /// Removals first (they free space), then the rest by song id.
    pub steps: Vec<Step>,
    /// Bytes the phone needs for new and replaced files.
    pub add_bytes: u64,
    /// Bytes freed by removals and by the old copies of replaced files.
    pub free_after_remove_bytes: u64,
}

/// Size of a song on the phone after the conversion `action` (estimates for converted files).
pub fn estimated_bytes(act: Action, source_bytes: u64, duration_ms: u64, channels: u64) -> u64 {
    match act {
        Action::Keep | Action::Skip => source_bytes,
        Action::Aac { kbps, .. } => kbps as u64 * 125 * duration_ms / 1000,
        // LAME V0 averages about 245 kbit/s.
        Action::Mp3 { mode: Mp3Mode::V0, .. } => 245 * 125 * duration_ms / 1000,
        Action::Mp3 { mode: Mp3Mode::Cbr320, .. } => 320 * 125 * duration_ms / 1000,
        // ALAC typically lands near 60 % of the PCM size.
        Action::Alac { bits, rate } => duration_ms * rate as u64 * channels * bits.div_ceil(8) as u64 * 6 / 10 / 1000,
    }
}

struct Liked {
    id: i64,
    bytes: u64,
    meta_hash: String,
    audio_hash: Option<String>,
}

struct Sent {
    device_pid: Option<u64>,
    meta_hash: String,
    audio_hash: Option<String>,
    bytes: u64,
    /// Found on the phone rather than sent by Miu: never removed.
    adopted: bool,
}

/// The tags the phone shows, hashed: a change means the phone needs a metadata update.
fn meta_hash(r: &rusqlite::Row<'_>) -> rusqlite::Result<String> {
    let mut s = String::new();
    for i in 1..=9 {
        let v: rusqlite::types::Value = r.get(i)?;
        s.push_str(&match v {
            rusqlite::types::Value::Text(t) => t,
            rusqlite::types::Value::Integer(n) => n.to_string(),
            _ => String::new(),
        });
        s.push('\u{1f}');
    }
    Ok(fnv64(s.as_bytes()))
}

/// The tag hash of one song, as a sync records it.
pub(crate) fn tags_hash(db: &Db, song_id: i64) -> rusqlite::Result<String> {
    db.conn().query_row(
        "SELECT id, title, artist, album, album_artist, genre, year, track_no, track_total, disc_no FROM songs WHERE id = ?1",
        [song_id],
        meta_hash,
    )
}

/// Tag and audio hashes of the liked songs, as a sync records them.
pub(crate) fn liked_hashes(db: &Db) -> rusqlite::Result<HashMap<i64, (String, Option<String>)>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, title, artist, album, album_artist, genre, year, track_no, track_total, disc_no, audio_hash
         FROM songs WHERE rating = 1",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, (meta_hash(r)?, r.get(10)?))))?;
    rows.collect()
}

/// What a device does with a song of this format, sample rate and bit depth.
pub type Decide<'a> = &'a dyn Fn(Format, Option<u32>, Option<u32>) -> Action;

fn liked_songs(db: &Db, decide: Decide) -> rusqlite::Result<Vec<Liked>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, title, artist, album, album_artist, genre, year, track_no, track_total, disc_no,
                format, sample_rate, bit_depth, size, duration_ms, audio_hash
         FROM songs WHERE rating = 1 ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| {
        let act = decide(Format::from_name(&r.get::<_, String>(10)?), r.get(11)?, r.get(12)?);
        let bytes = estimated_bytes(act, r.get::<_, i64>(13)? as u64, r.get::<_, Option<i64>>(14)?.unwrap_or(0) as u64, 2);
        Ok((act, Liked { id: r.get(0)?, bytes, meta_hash: meta_hash(r)?, audio_hash: r.get(15)? }))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (act, liked) = row?;
        if act != Action::Skip {
            out.push(liked);
        }
    }
    Ok(out)
}

fn sent(db: &Db, udid: &str) -> rusqlite::Result<HashMap<i64, Sent>> {
    let mut stmt = db.conn().prepare("SELECT song_id, device_pid, meta_hash, audio_hash, bytes, adopted FROM synced WHERE udid = ?1")?;
    let rows = stmt.query_map([udid], |r| {
        Ok((r.get(0)?, Sent { device_pid: r.get::<_, Option<i64>>(1)?.map(|p| p as u64), meta_hash: r.get(2)?, audio_hash: r.get(3)?, bytes: r.get::<_, i64>(4)? as u64, adopted: r.get(5)? }))
    })?;
    rows.collect()
}

/// What to do on the phone `udid` so it holds exactly the liked songs Miu can sync.
pub fn plan(db: &Db, udid: &str, policy: &Policy) -> rusqlite::Result<Plan> {
    plan_with(db, udid, &|format, rate, bits| action(format, rate, bits, policy))
}

/// The plan for any device (an iPhone, or a music player keyed by its volume): `decide` says what
/// becomes of each liked song there.
pub fn plan_with(db: &Db, udid: &str, decide: Decide) -> rusqlite::Result<Plan> {
    let mut on_phone = sent(db, udid)?;
    let mut p = Plan::default();
    let mut rest = Vec::new();
    for s in liked_songs(db, decide)? {
        match on_phone.remove(&s.id) {
            None => {
                p.add_bytes += s.bytes;
                rest.push(Step::Add { song_id: s.id, bytes: s.bytes, meta_hash: s.meta_hash, audio_hash: s.audio_hash });
            }
            Some(old) if old.audio_hash.is_some() && s.audio_hash.is_some() && old.audio_hash != s.audio_hash => {
                p.add_bytes += s.bytes;
                p.free_after_remove_bytes += old.bytes;
                rest.push(Step::Replace { song_id: s.id, device_pid: old.device_pid, bytes: s.bytes, old_bytes: old.bytes, meta_hash: s.meta_hash, audio_hash: s.audio_hash });
            }
            Some(old) if old.meta_hash != s.meta_hash => {
                rest.push(Step::Update { song_id: s.id, device_pid: old.device_pid, meta_hash: s.meta_hash });
            }
            Some(_) => {}
        }
    }
    let mut removals: Vec<(i64, Sent)> = on_phone.into_iter().collect();
    removals.sort_by_key(|(id, _)| *id);
    for (song_id, old) in removals {
        if old.adopted {
            continue;
        }
        p.free_after_remove_bytes += old.bytes;
        p.steps.push(Step::Remove { song_id, device_pid: old.device_pid, bytes: old.bytes });
    }
    p.steps.extend(rest);
    Ok(p)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortfall {
    pub need: u64,
    pub available: u64,
}

/// Whether the plan fits: free space plus what removals free, minus a safety `margin`.
pub fn check_space(p: &Plan, free_bytes: u64, margin_bytes: u64) -> Result<(), Shortfall> {
    let available = (free_bytes + p.free_after_remove_bytes).saturating_sub(margin_bytes);
    if p.add_bytes <= available { Ok(()) } else { Err(Shortfall { need: p.add_bytes, available }) }
}

/// How one step went: the track's id on the phone when it worked.
#[derive(Debug, Clone, PartialEq)]
pub struct StepResult {
    pub song_id: i64,
    pub outcome: Result<Option<u64>, String>,
}

/// Store a finished run: update what the phone holds and keep the history. Returns the run id.
pub fn record_run(db: &Db, udid: &str, p: &Plan, results: &[StepResult], now_unix: u64) -> rusqlite::Result<i64> {
    let tx = db.conn().unchecked_transaction()?;
    let by_song: HashMap<i64, &StepResult> = results.iter().map(|r| (r.song_id, r)).collect();
    let (mut done, mut failed) = (0, 0);
    tx.execute("INSERT INTO sync_runs (udid, finished_at, done, failed) VALUES (?1, ?2, 0, 0)", rusqlite::params![udid, now_unix as i64])?;
    let run = tx.last_insert_rowid();
    for step in &p.steps {
        let Some(result) = by_song.get(&step.song_id()) else { continue };
        let error = result.outcome.as_ref().err().cloned();
        tx.execute("INSERT INTO sync_items (run_id, song_id, kind, error) VALUES (?1, ?2, ?3, ?4)", rusqlite::params![run, step.song_id(), step.kind(), error])?;
        let Ok(new_pid) = &result.outcome else {
            failed += 1;
            continue;
        };
        done += 1;
        let upsert = |pid: Option<u64>, meta: &str, audio: Option<&str>, bytes: u64| {
            tx.execute(
                "INSERT INTO synced (udid, song_id, device_pid, meta_hash, audio_hash, bytes, synced_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(udid, song_id) DO UPDATE SET device_pid = excluded.device_pid, meta_hash = excluded.meta_hash,
                     audio_hash = excluded.audio_hash, bytes = excluded.bytes, synced_at = excluded.synced_at",
                rusqlite::params![udid, step.song_id(), pid.map(|p| p as i64), meta, audio, bytes as i64, now_unix as i64],
            )
        };
        match step {
            Step::Add { bytes, meta_hash, audio_hash, .. } => upsert(*new_pid, meta_hash, audio_hash.as_deref(), *bytes)?,
            Step::Replace { device_pid, bytes, meta_hash, audio_hash, .. } => upsert(new_pid.or(*device_pid), meta_hash, audio_hash.as_deref(), *bytes)?,
            Step::Update { meta_hash, .. } => {
                tx.execute("UPDATE synced SET meta_hash = ?1, synced_at = ?2 WHERE udid = ?3 AND song_id = ?4", rusqlite::params![meta_hash, now_unix as i64, udid, step.song_id()])?
            }
            Step::Remove { .. } => tx.execute("DELETE FROM synced WHERE udid = ?1 AND song_id = ?2", rusqlite::params![udid, step.song_id()])?,
        };
    }
    tx.execute("UPDATE sync_runs SET done = ?1, failed = ?2 WHERE id = ?3", rusqlite::params![done, failed, run])?;
    tx.commit()?;
    Ok(run)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub id: i64,
    pub finished_at: i64,
    pub done: i64,
    pub failed: i64,
}

/// Past runs for a phone, newest first.
pub fn history(db: &Db, udid: &str) -> rusqlite::Result<Vec<RunSummary>> {
    let mut stmt = db.conn().prepare("SELECT id, finished_at, done, failed FROM sync_runs WHERE udid = ?1 ORDER BY id DESC")?;
    let rows = stmt.query_map([udid], |r| Ok(RunSummary { id: r.get(0)?, finished_at: r.get(1)?, done: r.get(2)?, failed: r.get(3)? }))?;
    rows.collect()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunItem {
    pub song_id: i64,
    pub kind: String,
    pub error: Option<String>,
}

/// Every track of one run, in plan order.
pub fn run_items(db: &Db, run: i64) -> rusqlite::Result<Vec<RunItem>> {
    let mut stmt = db.conn().prepare("SELECT song_id, kind, error FROM sync_items WHERE run_id = ?1 ORDER BY rowid")?;
    let rows = stmt.query_map([run], |r| Ok(RunItem { song_id: r.get(0)?, kind: r.get(1)?, error: r.get(2)? }))?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::fixtures::wav;
    use crate::library::db::Db;
    use crate::library::edit::{edit_songs, Change, Edit, TagChanges};
    use crate::library::folders::add_folder;
    use crate::library::scan::rescan_folder;
    use crate::library::songs::{list_songs, set_rating, Rating};

    const PHONE: &str = "00008030-PHONE";

    struct Lib {
        _dir: tempfile::TempDir,
        db: Db,
        ids: Vec<i64>,
    }

    /// Three songs: a.wav, b.wav, c.wav (in id order).
    fn library() -> Lib {
        let dir = tempfile::tempdir().unwrap();
        for (i, name) in ["a.wav", "b.wav", "c.wav"].iter().enumerate() {
            wav(dir.path(), name, 44_100, 2, 16, 44_100 + i * 100);
        }
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &dir.path().to_string_lossy(), 1).unwrap();
        rescan_folder(&db, f.id, &f.path, 1).unwrap();
        crate::library::duplicates::fill_audio_hashes(&db, 10).unwrap();
        let mut ids: Vec<i64> = list_songs(&db).unwrap().iter().map(|s| s.id).collect();
        ids.sort();
        Lib { _dir: dir, db, ids }
    }

    fn like(lib: &Lib, i: usize) {
        set_rating(&lib.db, lib.ids[i], Rating::Liked).unwrap();
    }

    fn kinds(p: &Plan) -> Vec<(&'static str, i64)> {
        p.steps.iter().map(|s| (s.kind(), s.song_id())).collect()
    }

    /// Pretend every step went through.
    fn sync_all(lib: &Lib, p: &Plan, now: u64) {
        let results: Vec<StepResult> = p.steps.iter().enumerate().map(|(i, s)| StepResult { song_id: s.song_id(), outcome: Ok(Some(1_000 + i as u64)) }).collect();
        record_run(&lib.db, PHONE, p, &results, now).unwrap();
    }

    #[test]
    fn only_liked_songs_are_added() {
        let lib = library();
        like(&lib, 0);
        set_rating(&lib.db, lib.ids[1], Rating::Disliked).unwrap();
        let p = plan(&lib.db, PHONE, &Default::default()).unwrap();
        assert_eq!(kinds(&p), vec![("add", lib.ids[0])], "not rated and disliked never go");
    }

    #[test]
    fn a_synced_unchanged_library_plans_nothing() {
        let lib = library();
        like(&lib, 0);
        like(&lib, 2);
        let p = plan(&lib.db, PHONE, &Default::default()).unwrap();
        sync_all(&lib, &p, 10);
        assert!(plan(&lib.db, PHONE, &Default::default()).unwrap().steps.is_empty());
    }

    #[test]
    fn unliking_or_deleting_a_synced_song_removes_it_from_the_phone() {
        let lib = library();
        like(&lib, 0);
        like(&lib, 1);
        sync_all(&lib, &plan(&lib.db, PHONE, &Default::default()).unwrap(), 10);
        set_rating(&lib.db, lib.ids[0], Rating::Disliked).unwrap();
        lib.db.conn().execute("DELETE FROM songs WHERE id = ?1", [lib.ids[1]]).unwrap();
        let p = plan(&lib.db, PHONE, &Default::default()).unwrap();
        assert_eq!(kinds(&p), vec![("remove", lib.ids[0]), ("remove", lib.ids[1])]);
        assert!(p.steps.iter().all(|s| matches!(s, Step::Remove { device_pid: Some(_), .. })), "removal knows the track on the phone");
    }

    #[test]
    fn edited_tags_of_a_liked_song_are_an_update() {
        let lib = library();
        like(&lib, 0);
        sync_all(&lib, &plan(&lib.db, PHONE, &Default::default()).unwrap(), 10);
        let edit = Edit { id: lib.ids[0], changes: TagChanges { title: Change::Set("Diễm xưa".into()), ..Default::default() } };
        edit_songs(&lib.db, &[edit]).unwrap();
        assert_eq!(kinds(&plan(&lib.db, PHONE, &Default::default()).unwrap()), vec![("update", lib.ids[0])]);
    }

    #[test]
    fn changed_audio_is_replaced() {
        let lib = library();
        like(&lib, 0);
        sync_all(&lib, &plan(&lib.db, PHONE, &Default::default()).unwrap(), 10);
        lib.db.conn().execute("UPDATE songs SET audio_hash = 'ffffffffffffffff' WHERE id = ?1", [lib.ids[0]]).unwrap();
        assert_eq!(kinds(&plan(&lib.db, PHONE, &Default::default()).unwrap()), vec![("replace", lib.ids[0])]);
    }

    #[test]
    fn each_phone_has_its_own_state() {
        let lib = library();
        like(&lib, 0);
        sync_all(&lib, &plan(&lib.db, PHONE, &Default::default()).unwrap(), 10);
        assert_eq!(kinds(&plan(&lib.db, "OTHER-PHONE", &Default::default()).unwrap()), vec![("add", lib.ids[0])]);
    }

    #[test]
    fn failed_steps_come_back_in_the_next_plan_and_history_keeps_every_track() {
        let lib = library();
        like(&lib, 0);
        like(&lib, 1);
        let p = plan(&lib.db, PHONE, &Default::default()).unwrap();
        let results = vec![
            StepResult { song_id: lib.ids[0], outcome: Ok(Some(77)) },
            StepResult { song_id: lib.ids[1], outcome: Err("cable unplugged".into()) },
        ];
        let run = record_run(&lib.db, PHONE, &p, &results, 10).unwrap();
        assert_eq!(kinds(&plan(&lib.db, PHONE, &Default::default()).unwrap()), vec![("add", lib.ids[1])]);
        let h = history(&lib.db, PHONE).unwrap();
        assert_eq!(h.len(), 1);
        assert_eq!((h[0].id, h[0].done, h[0].failed), (run, 1, 1));
        let items = run_items(&lib.db, run).unwrap();
        assert_eq!(items.iter().map(|i| (i.song_id, i.kind.as_str(), i.error.as_deref())).collect::<Vec<_>>(),
            vec![(lib.ids[0], "add", None), (lib.ids[1], "add", Some("cable unplugged"))]);
    }

    #[test]
    fn sizes_follow_the_conversion_policy() {
        assert_eq!(estimated_bytes(crate::library::convert::Action::Keep, 5_000_000, 60_000, 2), 5_000_000);
        assert_eq!(estimated_bytes(crate::library::convert::Action::Aac { kbps: 256, rate: 44_100 }, 30_000_000, 60_000, 2), 1_920_000);
        // ALAC ≈ 60 % of the PCM size at the target format: 60 s × 48 kHz × 2 ch × 3 bytes × 0.6.
        assert_eq!(estimated_bytes(crate::library::convert::Action::Alac { bits: 24, rate: 48_000 }, 99, 60_000, 2), 10_368_000);
    }

    #[test]
    fn a_plan_that_does_not_fit_is_blocked_with_the_sizes() {
        let p = Plan { steps: vec![], add_bytes: 900, free_after_remove_bytes: 100 };
        assert_eq!(check_space(&p, 1_000, 200), Ok(()), "free 1000 + 100 freed − 200 margin = 900: fits exactly");
        let p = Plan { steps: vec![], add_bytes: 901, free_after_remove_bytes: 100 };
        assert_eq!(check_space(&p, 1_000, 200), Err(Shortfall { need: 901, available: 900 }));
        let p = Plan { steps: vec![], add_bytes: 5_000, free_after_remove_bytes: 0 };
        assert_eq!(check_space(&p, 1_000, 200), Err(Shortfall { need: 5_000, available: 800 }));
    }
}
