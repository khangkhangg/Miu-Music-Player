//! LRC lyrics (L.1): `[mm:ss.xx] words` lines, as found in `.lrc` files, tags and LRCLIB's
//! synced lyrics. A line may carry several time stamps (a repeated chorus); `[offset:±ms]`
//! shifts every line; other `[tag:value]` lines (title, artist, …) are not lyrics.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    /// When the line starts, ms into the song.
    pub ms: u64,
    pub text: String,
}

/// The timed lines of `text`, in time order. Empty when it has no time stamps.
pub fn parse(text: &str) -> Vec<Line> {
    let mut offset: i64 = 0;
    let mut lines = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while let Some(inner) = rest.strip_prefix('[') {
            let Some(end) = inner.find(']') else { break };
            let tag = &inner[..end];
            match stamp(tag) {
                Some(ms) => stamps.push(ms),
                None => {
                    if let Some(v) = tag.strip_prefix("offset:") {
                        offset = v.trim().trim_start_matches('+').parse().unwrap_or(0);
                    }
                    break;
                }
            }
            rest = inner[end + 1..].trim_start();
        }
        for ms in stamps {
            lines.push(Line { ms: (ms as i64 - offset).max(0) as u64, text: rest.trim().to_string() });
        }
    }
    lines.sort_by_key(|l| l.ms);
    lines
}

/// `mm:ss`, `mm:ss.f..` or `mm:ss:ff` as ms; `None` for anything else (a `[tag:value]`).
fn stamp(tag: &str) -> Option<u64> {
    let (min, rest) = tag.split_once(':')?;
    let min: u64 = min.trim().parse().ok()?;
    let (sec, frac) = match rest.split_once(['.', ':']) {
        Some((s, f)) => (s, Some(f)),
        None => (rest, None),
    };
    let sec: u64 = sec.trim().parse().ok()?;
    if sec >= 60 {
        return None;
    }
    let frac_ms = match frac {
        None => 0,
        Some(f) if !f.is_empty() && f.len() <= 3 && f.bytes().all(|b| b.is_ascii_digit()) => {
            let n: u64 = f.parse().ok()?;
            n * 10u64.pow(3 - f.len() as u32)
        }
        Some(_) => return None,
    };
    Some(min * 60_000 + sec * 1000 + frac_ms)
}

/// True when `text` is LRC (has time-stamped lines) rather than plain lyrics.
pub fn is_synced(text: &str) -> bool {
    !parse(text).is_empty()
}

/// The line being sung at `ms`: the last one that has started. `None` before the first.
pub fn line_at(lines: &[Line], ms: u64) -> Option<usize> {
    lines.partition_point(|l| l.ms <= ms).checked_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(ms: u64, text: &str) -> Line {
        Line { ms, text: text.into() }
    }

    #[test]
    fn reads_time_stamped_lines_in_order() {
        let text = "[ti:Diễm Xưa]\n[ar:Khánh Ly]\n[00:12.50] Mưa vẫn mưa bay\n[00:18.00]trên tầng tháp cổ\n[01:02.3] third\n";
        assert_eq!(parse(text), vec![l(12_500, "Mưa vẫn mưa bay"), l(18_000, "trên tầng tháp cổ"), l(62_300, "third")]);
    }

    #[test]
    fn understands_the_usual_stamp_styles() {
        assert_eq!(parse("[01:02]a")[0].ms, 62_000);
        assert_eq!(parse("[01:02.5]a")[0].ms, 62_500);
        assert_eq!(parse("[01:02.05]a")[0].ms, 62_050);
        assert_eq!(parse("[01:02.123]a")[0].ms, 62_123);
        assert_eq!(parse("[01:02:05]a")[0].ms, 62_050, "colon before hundredths");
    }

    #[test]
    fn a_line_with_several_stamps_appears_at_each_time() {
        assert_eq!(parse("[00:10.00][01:10.00]Chorus\n[00:20.00]Verse"), vec![l(10_000, "Chorus"), l(20_000, "Verse"), l(70_000, "Chorus")]);
    }

    #[test]
    fn the_offset_tag_shifts_every_line() {
        // A positive offset means the lyrics come earlier.
        assert_eq!(parse("[offset:+500]\n[00:10.00]a\n[00:00.20]b"), vec![l(0, "b"), l(9_500, "a")]);
        assert_eq!(parse("[offset:-250]\n[00:10.00]a")[0].ms, 10_250);
    }

    #[test]
    fn empty_stamped_lines_are_kept_as_pauses_and_plain_text_is_not_lrc() {
        assert_eq!(parse("[00:01.00]a\n[00:05.00]\n"), vec![l(1_000, "a"), l(5_000, "")]);
        assert!(!is_synced("Just words\nno stamps"));
        assert!(is_synced("[00:01.00]a"));
        assert!(parse("[ar:Someone]\n[by:me]").is_empty());
    }

    #[test]
    fn windows_line_endings_and_extra_spaces_are_fine() {
        assert_eq!(parse("  [00:01.00]  a  \r\n[00:02.00]b\r\n"), vec![l(1_000, "a"), l(2_000, "b")]);
    }

    #[test]
    fn the_current_line_is_the_last_one_started() {
        let lines = vec![l(1_000, "a"), l(5_000, "b"), l(9_000, "c")];
        assert_eq!(line_at(&lines, 0), None);
        assert_eq!(line_at(&lines, 1_000), Some(0));
        assert_eq!(line_at(&lines, 8_999), Some(1));
        assert_eq!(line_at(&lines, 60_000), Some(2));
        assert_eq!(line_at(&[], 5), None);
    }
}
