//! The phone's live system log (`com.apple.syslog_relay`): a byte stream of NUL-separated lines,
//! with non-ASCII bytes vis(3)-escaped (`\M-C\M-9` = 0xC3 0xB9 = "ù").

/// Turns arbitrary chunks of the stream into complete lines.
#[derive(Debug, Default)]
pub struct LineSplitter {
    pending: Vec<u8>,
}

impl LineSplitter {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut lines = Vec::new();
        for &b in bytes {
            if b == 0 {
                let raw = std::mem::take(&mut self.pending);
                let text = String::from_utf8_lossy(&raw);
                let line = text.trim_end_matches(['\n', '\r']);
                if !line.is_empty() {
                    lines.push(decode_vis(line));
                }
            } else {
                self.pending.push(b);
            }
        }
        lines
    }
}

/// Undo vis(3) escapes: `\\`, `\M-c` (c | 0x80), `\M^c` (ctrl(c) | 0x80), `\^c` (ctrl(c)).
pub fn decode_vis(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let ctrl = |c: u8| if c == b'?' { 0x7f } else { c & 0x1f };
    let mut i = 0;
    while i < b.len() {
        match (b[i], b.get(i + 1), b.get(i + 2), b.get(i + 3)) {
            (b'\\', Some(b'\\'), _, _) => {
                out.push(b'\\');
                i += 2;
            }
            (b'\\', Some(b'M'), Some(b'-'), Some(&c)) => {
                out.push(c | 0x80);
                i += 4;
            }
            (b'\\', Some(b'M'), Some(b'^'), Some(&c)) => {
                out.push(ctrl(c) | 0x80);
                i += 4;
            }
            (b'\\', Some(b'^'), Some(&c), _) => {
                out.push(ctrl(c));
                i += 3;
            }
            (c, _, _, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Lines from the processes involved in music sync.
pub fn is_sync_related(line: &str) -> bool {
    const MARKERS: [&str; 6] = [" atc[", "AirTraffic", "Grappa", "ATHost", "medialibraryd[", "SyncFailed"];
    MARKERS.iter().any(|m| line.contains(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_nul_separated_lines_and_drops_trailing_newlines() {
        let mut s = LineSplitter::default();
        assert_eq!(s.push(b"Oct 20 atc[42] <Notice>: hello\n\0Oct 20 kernel: x\n\0"), vec![
            "Oct 20 atc[42] <Notice>: hello".to_string(),
            "Oct 20 kernel: x".to_string(),
        ]);
    }

    #[test]
    fn keeps_partial_lines_until_they_complete() {
        let mut s = LineSplitter::default();
        assert!(s.push(b"Oct 20 atc[42] <Err").is_empty());
        assert_eq!(s.push(b">: Grappa failed\n\0Oct"), vec!["Oct 20 atc[42] <Err>: Grappa failed".to_string()]);
        assert_eq!(s.push(b" 21 x\0"), vec!["Oct 21 x".to_string()]);
    }

    #[test]
    fn skips_empty_records() {
        let mut s = LineSplitter::default();
        assert!(s.push(b"\0\n\0").is_empty());
    }

    #[test]
    fn decodes_vis_escaped_utf8() {
        assert_eq!(decode_vis(r"M\M-C\M-9a thu"), "Mùa thu");
        assert_eq!(decode_vis(r"back\\slash"), r"back\slash");
        assert_eq!(decode_vis(r"tab\^Iend"), "tab\tend");
    }

    #[test]
    fn leaves_unknown_escapes_alone() {
        assert_eq!(decode_vis(r"odd \q end"), r"odd \q end");
    }

    #[test]
    fn picks_sync_related_lines() {
        assert!(is_sync_related("Oct 20 18:27:29 iPhone atc[123] <Error>: Grappa session could not be established"));
        assert!(is_sync_related("Oct 20 iPhone AirTrafficHost[9] <Notice>: x"));
        assert!(is_sync_related("Oct 20 iPhone medialibraryd[77] <Notice>: SyncFailed"));
        assert!(!is_sync_related("Oct 20 iPhone kernel[0] <Notice>: wifi"));
        assert!(!is_sync_related("Oct 20 iPhone batteryd[5] <Notice>: watch level"));
    }
}
