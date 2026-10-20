//! Crash-safe sync journal (F6.3). Every step is written down before and after it runs, one JSON
//! line at a time and flushed, so after a crash or power cut Miu knows what was done, which step
//! was in flight (to check and roll back on the phone) and what is left to resume.

use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Entry {
    Begin { udid: String, steps: Vec<i64> },
    Started(i64),
    Done { song_id: i64, device_pid: Option<u64> },
    Failed { song_id: i64, error: String },
    Finish,
}

pub struct Journal {
    file: File,
}

impl Journal {
    /// Start a journal for a run over `steps` (song ids, in plan order) on phone `udid`.
    pub fn begin(path: &Path, udid: &str, steps: &[i64]) -> io::Result<Self> {
        let mut j = Self { file: File::create(path)? };
        j.write(&Entry::Begin { udid: udid.into(), steps: steps.to_vec() })?;
        Ok(j)
    }

    fn write(&mut self, e: &Entry) -> io::Result<()> {
        let mut line = serde_json::to_string(e).map_err(io::Error::other)?;
        line.push('\n');
        self.file.write_all(line.as_bytes())?;
        self.file.sync_data() // on disk before the step runs
    }

    pub fn started(&mut self, song_id: i64) -> io::Result<()> {
        self.write(&Entry::Started(song_id))
    }

    pub fn done(&mut self, song_id: i64, device_pid: Option<u64>) -> io::Result<()> {
        self.write(&Entry::Done { song_id, device_pid })
    }

    pub fn failed(&mut self, song_id: i64, error: &str) -> io::Result<()> {
        self.write(&Entry::Failed { song_id, error: error.into() })
    }

    pub fn finish(mut self) -> io::Result<()> {
        self.write(&Entry::Finish)
    }
}

/// What an interrupted run left behind.
#[derive(Debug, Clone, PartialEq)]
pub struct Recovery {
    pub udid: String,
    /// Steps that completed (song id, track id on the phone): record them in the history.
    pub done: Vec<(i64, Option<u64>)>,
    /// A step that started and never ended: check the phone and undo any half-written track.
    pub in_flight: Option<i64>,
    /// Steps to run again, in plan order (the in-flight one included).
    pub resume: Vec<i64>,
}

/// Read a journal after a restart. `None` when there is none or its run finished.
pub fn replay(path: &Path) -> io::Result<Option<Recovery>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut udid = None;
    let mut steps = Vec::new();
    let mut done = Vec::new();
    let mut in_flight = None;
    for line in BufReader::new(file).lines() {
        // A line cut off by the crash does not parse: everything before it still counts.
        let Ok(entry) = serde_json::from_str::<Entry>(&line?) else { break };
        match entry {
            Entry::Begin { udid: u, steps: s } => {
                udid = Some(u);
                steps = s;
            }
            Entry::Started(id) => in_flight = Some(id),
            Entry::Done { song_id, device_pid } => {
                done.push((song_id, device_pid));
                in_flight = None;
            }
            Entry::Failed { .. } => in_flight = None,
            Entry::Finish => return Ok(None),
        }
    }
    let Some(udid) = udid else { return Ok(None) };
    let resume = steps.into_iter().filter(|id| !done.iter().any(|(d, _)| d == id)).collect();
    Ok(Some(Recovery { udid, done, in_flight, resume }))
}

/// Remove the journal once its run has been recovered.
pub fn clear(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        r => r,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn journal(d: &std::path::Path) -> std::path::PathBuf {
        d.join("sync.journal")
    }

    #[test]
    fn a_finished_run_needs_no_recovery() {
        let d = tempfile::tempdir().unwrap();
        let mut j = Journal::begin(&journal(d.path()), "PHONE", &[1, 2]).unwrap();
        for id in [1, 2] {
            j.started(id).unwrap();
            j.done(id, Some(100 + id as u64)).unwrap();
        }
        j.finish().unwrap();
        assert_eq!(replay(&journal(d.path())).unwrap(), None);
    }

    #[test]
    fn a_crash_mid_step_rolls_that_step_back_and_resumes_the_rest() {
        let d = tempfile::tempdir().unwrap();
        let mut j = Journal::begin(&journal(d.path()), "PHONE", &[1, 2, 3]).unwrap();
        j.started(1).unwrap();
        j.done(1, Some(101)).unwrap();
        j.started(2).unwrap();
        drop(j); // the app dies here
        let r = replay(&journal(d.path())).unwrap().expect("needs recovery");
        assert_eq!(r.udid, "PHONE");
        assert_eq!(r.done, vec![(1, Some(101))]);
        assert_eq!(r.in_flight, Some(2), "check this one on the phone and undo it");
        assert_eq!(r.resume, vec![2, 3], "then run it again, and the rest");
    }

    #[test]
    fn a_failed_step_is_resumed_not_rolled_back() {
        let d = tempfile::tempdir().unwrap();
        let mut j = Journal::begin(&journal(d.path()), "PHONE", &[1, 2]).unwrap();
        j.started(1).unwrap();
        j.failed(1, "unplugged").unwrap();
        drop(j);
        let r = replay(&journal(d.path())).unwrap().unwrap();
        assert_eq!((r.in_flight, r.resume), (None, vec![1, 2]));
    }

    #[test]
    fn a_line_cut_off_by_the_crash_is_ignored() {
        let d = tempfile::tempdir().unwrap();
        let mut j = Journal::begin(&journal(d.path()), "PHONE", &[1, 2]).unwrap();
        j.started(1).unwrap();
        j.done(1, Some(7)).unwrap();
        drop(j);
        use std::io::Write;
        std::fs::OpenOptions::new().append(true).open(journal(d.path())).unwrap().write_all(br#"{"started":"#).unwrap();
        let r = replay(&journal(d.path())).unwrap().unwrap();
        assert_eq!((r.done.len(), r.in_flight, r.resume), (1, None, vec![2]));
    }

    #[test]
    fn no_journal_means_nothing_to_recover() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(replay(&journal(d.path())).unwrap(), None);
    }

    #[test]
    fn clearing_after_recovery_removes_the_journal() {
        let d = tempfile::tempdir().unwrap();
        Journal::begin(&journal(d.path()), "PHONE", &[1]).unwrap();
        clear(&journal(d.path())).unwrap();
        assert!(!journal(d.path()).exists());
        clear(&journal(d.path())).unwrap(); // twice is fine
    }
}
