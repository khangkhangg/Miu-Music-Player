//! What a file really is, from its bytes rather than its extension (F3.2). Signatures only —
//! no decoding. M4A files are told apart (AAC vs ALAC) by walking the MP4 box tree to `stsd`.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Mp3,
    Aac,
    Alac,
    Flac,
    Wav,
    Aiff,
    Opus,
    Vorbis,
    Wma,
    Ape,
    WavPack,
    Unknown,
}

const NAMES: [(Format, &str); 12] = [
    (Format::Mp3, "mp3"),
    (Format::Aac, "aac"),
    (Format::Alac, "alac"),
    (Format::Flac, "flac"),
    (Format::Wav, "wav"),
    (Format::Aiff, "aiff"),
    (Format::Opus, "opus"),
    (Format::Vorbis, "vorbis"),
    (Format::Wma, "wma"),
    (Format::Ape, "ape"),
    (Format::WavPack, "wavpack"),
    (Format::Unknown, "unknown"),
];

impl Format {
    /// Stable name for storage.
    pub fn name(self) -> &'static str {
        NAMES.iter().find(|(f, _)| *f == self).map(|(_, n)| *n).unwrap_or("unknown")
    }

    pub fn from_name(name: &str) -> Self {
        NAMES.iter().find(|(_, n)| *n == name).map(|(f, _)| *f).unwrap_or(Format::Unknown)
    }

    /// Plays in the iPhone's Music app without conversion (CLAUDE.md rule 5).
    pub fn iphone_playable(self) -> bool {
        matches!(self, Format::Mp3 | Format::Aac | Format::Alac | Format::Wav | Format::Aiff)
    }
}

const ASF_GUID: [u8; 16] = [0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE, 0x6C];
const HEAD: usize = 64 * 1024;

enum Sniffed {
    Is(Format),
    /// An ID3v2 tag of this many bytes comes first; look again after it.
    SkipId3(usize),
    Mp4,
}

fn sniff_head(b: &[u8]) -> Sniffed {
    if b.len() >= 10 && &b[..3] == b"ID3" {
        let size = b[6..10].iter().fold(0usize, |acc, &x| (acc << 7) | (x & 0x7F) as usize);
        let footer = if b[5] & 0x10 != 0 { 10 } else { 0 };
        return Sniffed::SkipId3(10 + size + footer);
    }
    let at = |i: usize, sig: &[u8]| b.len() >= i + sig.len() && &b[i..i + sig.len()] == sig;
    let fmt = if at(0, b"fLaC") {
        Format::Flac
    } else if at(0, b"RIFF") && at(8, b"WAVE") {
        Format::Wav
    } else if at(0, b"FORM") && (at(8, b"AIFF") || at(8, b"AIFC")) {
        Format::Aiff
    } else if at(0, b"OggS") {
        let window = &b[..b.len().min(128)];
        let has = |sig: &[u8]| window.windows(sig.len()).any(|w| w == sig);
        if has(b"OpusHead") {
            Format::Opus
        } else if has(b"\x01vorbis") {
            Format::Vorbis
        } else if has(b"\x7fFLAC") {
            Format::Flac
        } else {
            Format::Unknown
        }
    } else if at(0, &ASF_GUID) {
        Format::Wma
    } else if at(0, b"MAC ") {
        Format::Ape
    } else if at(0, b"wvpk") {
        Format::WavPack
    } else if at(4, b"ftyp") {
        return Sniffed::Mp4;
    } else if b.len() >= 2 && b[0] == 0xFF && b[1] & 0xE0 == 0xE0 {
        match (b[1] >> 1) & 0b11 {
            0b01 => Format::Mp3, // Layer III
            0b00 => Format::Aac, // ADTS
            _ => Format::Unknown,
        }
    } else {
        Format::Unknown
    };
    Sniffed::Is(fmt)
}

/// Format of an in-memory file (or its beginning).
pub fn sniff_bytes(bytes: &[u8]) -> Format {
    match sniff_head(bytes) {
        Sniffed::Is(f) => f,
        Sniffed::SkipId3(n) if n < bytes.len() => sniff_bytes(&bytes[n..]),
        Sniffed::SkipId3(_) => Format::Unknown,
        Sniffed::Mp4 => {
            let mut c = io::Cursor::new(bytes);
            mp4_format(&mut c, 0, bytes.len() as u64).unwrap_or(Format::Unknown)
        }
    }
}

/// Format of a file on disk. Reads the beginning, seeks past ID3 tags, walks MP4 boxes.
pub fn detect_file(path: &Path) -> io::Result<Format> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let mut offset = 0u64;
    for _ in 0..4 {
        f.seek(SeekFrom::Start(offset))?;
        let mut head = Vec::with_capacity(HEAD);
        (&mut f).take(HEAD as u64).read_to_end(&mut head)?;
        match sniff_head(&head) {
            Sniffed::Is(fmt) => return Ok(fmt),
            Sniffed::SkipId3(n) => offset += n as u64,
            Sniffed::Mp4 => return Ok(mp4_format(&mut f, offset, len).unwrap_or(Format::Unknown)),
        }
    }
    Ok(Format::Unknown)
}

/// Walk boxes in `[start, end)`, descending moov/trak/mdia/minf/stbl, until `stsd`'s first entry.
fn mp4_format<R: Read + Seek>(r: &mut R, start: u64, end: u64) -> Option<Format> {
    let mut pos = start;
    while pos + 8 <= end {
        r.seek(SeekFrom::Start(pos)).ok()?;
        let mut hdr = [0u8; 8];
        r.read_exact(&mut hdr).ok()?;
        let mut size = u32::from_be_bytes(hdr[..4].try_into().ok()?) as u64;
        let kind: [u8; 4] = hdr[4..8].try_into().ok()?;
        let mut header = 8;
        if size == 1 {
            let mut big = [0u8; 8];
            r.read_exact(&mut big).ok()?;
            size = u64::from_be_bytes(big);
            header = 16;
        } else if size == 0 {
            size = end - pos;
        }
        if size < header {
            return None;
        }
        match &kind {
            b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" => {
                if let Some(f) = mp4_format(r, pos + header, (pos + size).min(end)) {
                    return Some(f);
                }
            }
            b"stsd" => {
                r.seek(SeekFrom::Start(pos + header + 8)).ok()?; // version/flags + entry count
                let mut entry = [0u8; 8];
                r.read_exact(&mut entry).ok()?;
                return Some(match &entry[4..8] {
                    b"mp4a" => Format::Aac,
                    b"alac" => Format::Alac,
                    _ => Format::Unknown,
                });
            }
            _ => {}
        }
        pos += size;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const MP3_FRAME: [u8; 4] = [0xFF, 0xFB, 0x90, 0x00]; // MPEG-1 Layer III
    const ADTS: [u8; 4] = [0xFF, 0xF1, 0x50, 0x80]; // AAC ADTS

    fn id3(payload: &[u8]) -> Vec<u8> {
        // ID3v2.3 header with a 20-byte (syncsafe) tag body.
        let mut v = b"ID3\x03\x00\x00\x00\x00\x00\x14".to_vec();
        v.extend([0u8; 20]);
        v.extend(payload);
        v
    }

    fn mp4_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend(kind);
        v.extend(payload);
        v
    }

    fn m4a(codec: &[u8; 4], moov_last: bool) -> Vec<u8> {
        let mut stsd = vec![0u8; 8]; // version/flags + entry count
        stsd[7] = 1;
        stsd.extend(mp4_box(codec, &[0u8; 28]));
        let moov = mp4_box(b"moov", &mp4_box(b"trak", &mp4_box(b"mdia", &mp4_box(b"minf", &mp4_box(b"stbl", &mp4_box(b"stsd", &stsd))))));
        let ftyp = mp4_box(b"ftyp", b"M4A \x00\x00\x00\x00M4A mp42isom");
        let mdat = mp4_box(b"mdat", &[0u8; 5000]);
        if moov_last { [ftyp, mdat, moov].concat() } else { [ftyp, moov, mdat].concat() }
    }

    fn file_with(bytes: &[u8], name: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join(name);
        std::fs::write(&p, bytes).unwrap();
        (d, p)
    }

    #[test]
    fn a_flac_renamed_to_mp3_is_detected_as_flac() {
        let (_d, p) = file_with(b"fLaC\x00\x00\x00\x22rest", "song.mp3");
        assert_eq!(detect_file(&p).unwrap(), Format::Flac);
    }

    #[test]
    fn mp3_behind_an_id3_tag() {
        assert_eq!(sniff_bytes(&id3(&MP3_FRAME)), Format::Mp3);
    }

    #[test]
    fn flac_behind_an_id3_tag() {
        assert_eq!(sniff_bytes(&id3(b"fLaC\x00\x00\x00\x22")), Format::Flac);
    }

    #[test]
    fn bare_mpeg_and_adts_frames() {
        assert_eq!(sniff_bytes(&MP3_FRAME), Format::Mp3);
        assert_eq!(sniff_bytes(&ADTS), Format::Aac);
    }

    #[test]
    fn riff_and_iff_containers() {
        assert_eq!(sniff_bytes(b"RIFF\x24\x00\x00\x00WAVEfmt "), Format::Wav);
        assert_eq!(sniff_bytes(b"FORM\x00\x00\x00\x24AIFFCOMM"), Format::Aiff);
        assert_eq!(sniff_bytes(b"FORM\x00\x00\x00\x24AIFCFVER"), Format::Aiff);
    }

    #[test]
    fn ogg_codecs() {
        assert_eq!(sniff_bytes(b"OggS\x00\x02\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x01\x13OpusHead"), Format::Opus);
        assert_eq!(sniff_bytes(b"OggS\x00\x02\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x01\x1e\x01vorbis"), Format::Vorbis);
    }

    #[test]
    fn wma_ape_and_wavpack() {
        let asf = [0x30, 0x26, 0xB2, 0x75, 0x8E, 0x66, 0xCF, 0x11, 0xA6, 0xD9, 0x00, 0xAA, 0x00, 0x62, 0xCE, 0x6C];
        assert_eq!(sniff_bytes(&asf), Format::Wma);
        assert_eq!(sniff_bytes(b"MAC \x96\x0f\x00\x00"), Format::Ape);
        assert_eq!(sniff_bytes(b"wvpk\x00\x10\x00\x00"), Format::WavPack);
    }

    #[test]
    fn m4a_codec_comes_from_the_sample_description() {
        let (_a, aac) = file_with(&m4a(b"mp4a", false), "a.m4a");
        let (_b, alac) = file_with(&m4a(b"alac", false), "b.m4a");
        assert_eq!(detect_file(&aac).unwrap(), Format::Aac);
        assert_eq!(detect_file(&alac).unwrap(), Format::Alac);
    }

    #[test]
    fn m4a_with_metadata_at_the_end_is_still_read() {
        let (_d, p) = file_with(&m4a(b"alac", true), "c.m4a");
        assert_eq!(detect_file(&p).unwrap(), Format::Alac);
    }

    #[test]
    fn unknown_or_empty_content() {
        assert_eq!(sniff_bytes(b"hello, not audio"), Format::Unknown);
        assert_eq!(sniff_bytes(b""), Format::Unknown);
        let (_d, p) = file_with(b"", "empty.mp3");
        assert_eq!(detect_file(&p).unwrap(), Format::Unknown);
    }

    #[test]
    fn iphone_plays_mp3_aac_alac_wav_aiff_only() {
        for f in [Format::Mp3, Format::Aac, Format::Alac, Format::Wav, Format::Aiff] {
            assert!(f.iphone_playable(), "{f:?}");
        }
        for f in [Format::Flac, Format::Opus, Format::Vorbis, Format::Wma, Format::Ape, Format::WavPack, Format::Unknown] {
            assert!(!f.iphone_playable(), "{f:?}");
        }
    }

    #[test]
    fn names_round_trip_for_storage() {
        for f in [Format::Mp3, Format::Aac, Format::Alac, Format::Flac, Format::Wav, Format::Aiff, Format::Opus,
                  Format::Vorbis, Format::Wma, Format::Ape, Format::WavPack, Format::Unknown] {
            assert_eq!(Format::from_name(f.name()), f);
        }
    }
}
