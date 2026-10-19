use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use log::{Level, LevelFilter, Log, Metadata, Record};

use crate::util::time::civil_from_days;

const BASE_NAME: &str = "miumusic";
pub const MAX_BYTES: u64 = 1024 * 1024;
pub const MAX_FILES: usize = 5;

/// One log line: `2026-10-19T12:34:56.789Z INFO  target: message\n` (UTC).
pub fn format_line(unix_ms: u64, level: Level, target: &str, msg: &str) -> String {
    let secs = unix_ms / 1000;
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    let rem = secs % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z {level:<5} {target}: {msg}\n",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        unix_ms % 1000
    )
}


/// Size-rotated log file: `miumusic.log`, `miumusic.1.log` … up to `max_files` files in total.
pub struct RotatingFile {
    dir: PathBuf,
    file: Option<File>,
    size: u64,
    max_bytes: u64,
    max_files: usize,
}

impl RotatingFile {
    pub fn open(dir: &Path, max_bytes: u64, max_files: usize) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let file = open_append(&current_path(dir))?;
        let size = file.metadata()?.len();
        Ok(Self { dir: dir.to_path_buf(), file: Some(file), size, max_bytes, max_files: max_files.max(1) })
    }

    fn rotate(&mut self) -> io::Result<()> {
        // Windows can't rename an open file: close it before shifting the names.
        if let Some(mut f) = self.file.take() {
            f.flush()?;
        }
        let oldest = rotated_path(&self.dir, self.max_files - 1);
        if oldest.exists() {
            fs::remove_file(&oldest)?;
        }
        for i in (1..self.max_files - 1).rev() {
            let from = rotated_path(&self.dir, i);
            if from.exists() {
                fs::rename(&from, rotated_path(&self.dir, i + 1))?;
            }
        }
        let current = current_path(&self.dir);
        if self.max_files > 1 {
            fs::rename(&current, rotated_path(&self.dir, 1))?;
        } else {
            fs::remove_file(&current)?;
        }
        self.file = Some(open_append(&current)?);
        self.size = 0;
        Ok(())
    }

    fn file(&mut self) -> io::Result<&mut File> {
        if self.file.is_none() {
            self.file = Some(open_append(&current_path(&self.dir))?);
        }
        Ok(self.file.as_mut().expect("opened above"))
    }
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.size > 0 && self.size + buf.len() as u64 > self.max_bytes {
            self.rotate()?;
        }
        let n = self.file()?.write(buf)?;
        self.size += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file()?.flush()
    }
}

fn current_path(dir: &Path) -> PathBuf {
    dir.join(format!("{BASE_NAME}.log"))
}

fn rotated_path(dir: &Path, i: usize) -> PathBuf {
    dir.join(format!("{BASE_NAME}.{i}.log"))
}

fn open_append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

struct FileLogger {
    out: Mutex<RotatingFile>,
    level: LevelFilter,
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let line = format_line(ms, record.level(), record.target(), &record.args().to_string());
        if let Ok(mut out) = self.out.lock() {
            let _ = out.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        if let Ok(mut out) = self.out.lock() {
            let _ = out.flush();
        }
    }
}

/// Install the global file logger writing into `dir`.
pub fn init(dir: &Path, level: LevelFilter) -> io::Result<()> {
    let out = RotatingFile::open(dir, MAX_BYTES, MAX_FILES)?;
    log::set_boxed_logger(Box::new(FileLogger { out: Mutex::new(out), level }))
        .map_err(|e| io::Error::new(io::ErrorKind::AlreadyExists, e))?;
    log::set_max_level(level);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    #[test]
    fn line_has_utc_timestamp_level_target_and_message() {
        // 2026-10-19T12:34:56.789Z
        let ms = 1_792_413_296_789;
        assert_eq!(
            format_line(ms, ::log::Level::Info, "miu::sync", "plan ready"),
            "2026-10-19T12:34:56.789Z INFO  miu::sync: plan ready\n"
        );
    }

    #[test]
    fn line_keeps_unicode_messages_intact() {
        let line = format_line(0, ::log::Level::Warn, "scan", "Mùa thu cho em");
        assert_eq!(line, "1970-01-01T00:00:00.000Z WARN  scan: Mùa thu cho em\n");
    }

    #[test]
    fn writes_to_current_log_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = RotatingFile::open(dir.path(), 1024, 3).unwrap();
        w.write_all(b"hello\n").unwrap();
        w.flush().unwrap();
        assert_eq!(fs::read_to_string(dir.path().join("miumusic.log")).unwrap(), "hello\n");
    }

    #[test]
    fn rotates_when_next_write_would_exceed_max_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = RotatingFile::open(dir.path(), 10, 3).unwrap();
        w.write_all(b"first-8\n").unwrap();
        w.write_all(b"second\n").unwrap();
        w.flush().unwrap();
        assert_eq!(fs::read_to_string(dir.path().join("miumusic.1.log")).unwrap(), "first-8\n");
        assert_eq!(fs::read_to_string(dir.path().join("miumusic.log")).unwrap(), "second\n");
    }

    #[test]
    fn keeps_at_most_max_files_and_drops_the_oldest() {
        let dir = tempfile::tempdir().unwrap();
        let mut w = RotatingFile::open(dir.path(), 4, 3).unwrap();
        for line in ["a1\n", "b2\n", "c3\n", "d4\n", "e5\n"] {
            w.write_all(line.as_bytes()).unwrap();
        }
        w.flush().unwrap();
        let mut names: Vec<_> = fs::read_dir(dir.path()).unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        assert_eq!(names, ["miumusic.1.log", "miumusic.2.log", "miumusic.log"]);
        assert_eq!(fs::read_to_string(dir.path().join("miumusic.log")).unwrap(), "e5\n");
        assert_eq!(fs::read_to_string(dir.path().join("miumusic.2.log")).unwrap(), "c3\n");
    }

    #[test]
    fn appends_to_existing_log_after_restart() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut w = RotatingFile::open(dir.path(), 1024, 3).unwrap();
            w.write_all(b"run1\n").unwrap();
        }
        let mut w = RotatingFile::open(dir.path(), 1024, 3).unwrap();
        w.write_all(b"run2\n").unwrap();
        w.flush().unwrap();
        assert_eq!(fs::read_to_string(dir.path().join("miumusic.log")).unwrap(), "run1\nrun2\n");
    }

    #[test]
    fn creates_the_log_directory_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("MiuMusic").join("logs");
        RotatingFile::open(&logs, 1024, 3).unwrap();
        assert!(logs.join("miumusic.log").exists());
    }
}
