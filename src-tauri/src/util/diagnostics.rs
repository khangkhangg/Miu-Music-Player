//! Diagnostics export (F6.4): one text file with the logs and device facts, for bug reports.
//! It never includes a device database, anything from `research/`, or account identifiers.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

static EMAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").unwrap());
static ACCOUNT_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(dsid|dspersonid|account_?id|apple_?id)(\s*[:=]\s*|\s+)\d+").unwrap());

/// Blank out e-mail addresses and Apple account identifiers.
pub fn redact(text: &str) -> String {
    let no_ids = ACCOUNT_ID.replace_all(text, "${1}${2}<redacted>");
    EMAIL.replace_all(&no_ids, "<email>").into_owned()
}

/// Log files only: never databases, never anything named after the research captures.
fn shareable(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains(".log") && !n.contains("research") && !n.contains(".sqlitedb") && !n.ends_with(".db")
}

/// The diagnostics text: app version, device facts, then every log file (oldest rotation last).
pub fn report(app_data: &Path, version: &str, device: &str) -> String {
    let mut out = format!("{version}\nDevice: {device}\n");
    let mut files: Vec<_> = std::fs::read_dir(app_data.join("logs"))
        .map(|it| it.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect())
        .unwrap_or_default();
    files.sort();
    for f in files {
        let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !shareable(&name) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&f) else { continue };
        out.push_str(&format!("\n===== {name} =====\n"));
        out.push_str(&redact(&String::from_utf8_lossy(&bytes)));
    }
    out
}

/// Write the report to `to` (folders created as needed).
pub fn export(app_data: &Path, version: &str, device: &str, to: &Path) -> std::io::Result<()> {
    if let Some(dir) = to.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(to, report(app_data, version, device))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn app_data() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let logs = d.path().join("logs");
        fs::create_dir_all(&logs).unwrap();
        fs::write(logs.join("miumusic.log"), "INFO audio output: 48000 Hz\nINFO signed in dsid=1234567890 as khang@icloud.com\n").unwrap();
        fs::write(logs.join("miumusic.log.1"), "WARN older line\n").unwrap();
        let backup = d.path().join("backups").join("UDID").join("20261020");
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("MediaLibrary.sqlitedb"), "SQLite format 3 secret songs").unwrap();
        fs::write(d.path().join("miumusic.db"), "SQLite format 3 library").unwrap();
        let research = d.path().join("research");
        fs::create_dir_all(&research).unwrap();
        fs::write(research.join("capture.txt"), "DSPersonID 987654321").unwrap();
        fs::write(logs.join("research-notes.log"), "should not be here").unwrap();
        d
    }

    #[test]
    fn the_report_has_the_logs_and_device_facts() {
        let d = app_data();
        let r = report(d.path(), "Miu Music 0.1.0", "iPhone 5s · iOS 12.5.8");
        assert!(r.contains("Miu Music 0.1.0"));
        assert!(r.contains("iPhone 5s · iOS 12.5.8"));
        assert!(r.contains("audio output: 48000 Hz"));
        assert!(r.contains("older line"), "rotated logs too");
    }

    #[test]
    fn databases_research_and_account_ids_never_go_out() {
        let d = app_data();
        let r = report(d.path(), "v", "device");
        assert!(!r.contains("SQLite format 3"), "no database content");
        assert!(!r.contains("secret songs") && !r.contains("MediaLibrary"));
        assert!(!r.contains("987654321") && !r.contains("should not be here"), "nothing from research");
        assert!(!r.contains("1234567890"), "account id redacted");
        assert!(!r.contains("khang@icloud.com"), "email redacted");
        assert!(r.contains("dsid=<redacted>") && r.contains("<email>"));
    }

    #[test]
    fn redaction_covers_the_usual_spellings() {
        assert_eq!(redact("DSPersonID: 42424242 x"), "DSPersonID: <redacted> x");
        assert_eq!(redact("accountId=99 a.b-c@example.co.uk"), "accountId=<redacted> <email>");
        assert_eq!(redact("rate 44100 Hz"), "rate 44100 Hz", "ordinary numbers stay");
    }

    #[test]
    fn export_writes_the_report_to_the_chosen_file() {
        let d = app_data();
        let out = d.path().join("out").join("MiuMusic-diagnostics.txt");
        export(d.path(), "v", "device", &out).unwrap();
        assert!(fs::read_to_string(&out).unwrap().contains("audio output"));
    }
}
