//! Audio fingerprint for duplicate detection (F3.6): a hash of the decoded opening of a song, so
//! tags, file names and container (WAV vs AIFF vs FLAC) do not matter — only the sound does.

use std::path::Path;

use crate::audio::decode::Decoder;

/// How much of each song is hashed. Duration is compared separately (see `duplicates`).
const OPENING_SECONDS: u64 = 30;

/// FNV-1a, 64-bit: tiny and stable across Rust versions (stored hashes must stay comparable).
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 ^= *b as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
}

/// Hash of the first 30 s of decoded audio at 16-bit precision, with its rate and channel
/// count, as 16 hex digits.
pub fn audio_hash(path: &Path) -> Result<String, String> {
    let mut dec = Decoder::open(path).map_err(|e| e.to_string())?;
    let mut h = Fnv::new();
    h.write(&dec.sample_rate.to_le_bytes());
    h.write(&(dec.channels as u32).to_le_bytes());
    let mut left = OPENING_SECONDS * dec.sample_rate as u64 * dec.channels as u64;
    while left > 0 {
        let Some(chunk) = dec.next_chunk().map_err(|e| e.to_string())? else { break };
        let take = (chunk.len() as u64).min(left) as usize;
        for s in &chunk[..take] {
            h.write(&((s * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16).to_le_bytes());
        }
        left -= take as u64;
    }
    Ok(format!("{:016x}", h.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::fixtures::{aiff, wav, wav_with};
    use crate::library::tags::fixtures::{write_tags, Fields};
    use lofty::tag::TagType;

    fn fields(title: &str) -> Fields<'_> {
        Fields { title, artist: "Khánh Ly", album: "Trịnh Công Sơn", album_artist: "", genre: "", year: "1970",
                 track: 1, track_total: 1, disc: 1, cover: false }
    }

    #[test]
    fn same_audio_under_another_name_and_tags_has_the_same_hash() {
        let d = tempfile::tempdir().unwrap();
        let a = wav(d.path(), "Diễm xưa.wav", 44_100, 2, 16, 44_100);
        let b = d.path().join("Diễm xưa (copy).wav");
        std::fs::copy(&a, &b).unwrap();
        write_tags(&b, TagType::RiffInfo, &fields("Something else"));
        assert_ne!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap(), "the files themselves differ");
        assert_eq!(audio_hash(&a).unwrap(), audio_hash(&b).unwrap());
    }

    #[test]
    fn the_same_audio_in_another_container_has_the_same_hash() {
        let d = tempfile::tempdir().unwrap();
        let a = wav(d.path(), "a.wav", 44_100, 2, 16, 44_100);
        let b = aiff(d.path(), "a.aiff", 44_100, 2, 44_100);
        assert_eq!(audio_hash(&a).unwrap(), audio_hash(&b).unwrap());
    }

    #[test]
    fn different_audio_with_the_same_tags_has_another_hash() {
        let d = tempfile::tempdir().unwrap();
        let a = wav(d.path(), "a.wav", 44_100, 2, 16, 44_100);
        let b = wav_with(d.path(), "b.wav", 44_100, 2, 16, 44_100, |i, _| ((i as f32) * 0.05).sin() * 0.3);
        write_tags(&a, TagType::RiffInfo, &fields("Diễm xưa"));
        write_tags(&b, TagType::RiffInfo, &fields("Diễm xưa"));
        assert_ne!(audio_hash(&a).unwrap(), audio_hash(&b).unwrap());
    }

    #[test]
    fn the_hash_is_stable_text() {
        let d = tempfile::tempdir().unwrap();
        let h = audio_hash(&wav(d.path(), "a.wav", 8_000, 1, 16, 800)).unwrap();
        assert_eq!(h.len(), 16);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn not_audio_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.wav");
        std::fs::write(&p, b"nope").unwrap();
        assert!(audio_hash(&p).is_err());
    }
}
