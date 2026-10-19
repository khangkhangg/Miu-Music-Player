//! Capture of AirTraffic (ATC) messages as JSON lines (F2.2), for working out the sync protocol.
//! Captures may contain account data: they go to `research/captures/` (gitignored), never to the
//! app log, which only gets message names.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::apple::value::CfValue;
use crate::util::time::file_stamp;

pub struct CaptureLog {
    path: PathBuf,
    file: File,
    seq: u64,
}

impl CaptureLog {
    pub fn create(dir: &Path, now_unix: u64) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("{}-atc.jsonl", file_stamp(now_unix)));
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self { path, file, seq: 0 })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `ms`: milliseconds since the capture started; `name`: from `ATCFMessageGetName`.
    pub fn record(&mut self, ms: u64, name: Option<&str>, message: &CfValue) -> io::Result<()> {
        self.seq += 1;
        let line = serde_json::json!({ "seq": self.seq, "ms": ms, "name": name, "message": message.to_json() });
        writeln!(self.file, "{line}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn msg(name: &str) -> CfValue {
        CfValue::Dict(BTreeMap::from([
            ("Command".to_string(), CfValue::String(name.into())),
            ("Params".to_string(), CfValue::Dict(BTreeMap::new())),
        ]))
    }

    #[test]
    fn capture_file_is_named_by_utc_time_in_the_captures_dir() {
        let d = tempfile::tempdir().unwrap();
        let log = CaptureLog::create(d.path(), 1_792_465_445).unwrap();
        assert_eq!(log.path(), d.path().join("20261020-030405-atc.jsonl"));
    }

    #[test]
    fn each_message_is_one_json_line_with_sequence_time_and_name() {
        let d = tempfile::tempdir().unwrap();
        let mut log = CaptureLog::create(d.path(), 0).unwrap();
        log.record(1_250, Some("SyncAllowed"), &msg("SyncAllowed")).unwrap();
        log.record(1_900, None, &msg("Ping")).unwrap();
        let lines: Vec<serde_json::Value> = std::fs::read_to_string(log.path()).unwrap()
            .lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["seq"], 1);
        assert_eq!(lines[0]["ms"], 1_250);
        assert_eq!(lines[0]["name"], "SyncAllowed");
        assert_eq!(lines[0]["message"]["Command"], "SyncAllowed");
        assert_eq!(lines[1]["seq"], 2);
        assert!(lines[1]["name"].is_null());
    }

    #[test]
    fn creates_the_captures_dir() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("research").join("captures");
        CaptureLog::create(&dir, 0).unwrap();
        assert!(dir.is_dir());
    }
}
