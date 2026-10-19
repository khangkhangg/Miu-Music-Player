//! What each song becomes on the iPhone (F3.4). The Music app plays MP3, AAC, ALAC, WAV and AIFF
//! (CLAUDE.md rule 5) and an iPhone 5s outputs at most 48 kHz, so anything else is converted.
//! PC playback always uses the original file.

use serde::{Deserialize, Serialize};

use crate::library::format::Format;

/// What to do with a song before it goes to the phone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Copy the file as it is.
    Keep,
    Alac { bits: u32, rate: u32 },
    Aac { kbps: u32, rate: u32 },
    /// Not a format Miu can read: never synced.
    Skip,
}

/// Hi-res sources (above 48 kHz or 24-bit) on the phone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HiRes {
    /// Best the iPhone can play: 24-bit, 48 kHz (44.1 kHz for 88.2 / 176.4 kHz sources).
    #[default]
    Alac24_48,
    /// CD quality, to save space.
    Alac16_44,
}

/// Lossless sources the Music app cannot play as they are (FLAC, APE, WavPack, hi-res).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Lossless {
    #[default]
    Alac,
    /// Much smaller files, audibly the same for most people.
    Aac256,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    pub hi_res: HiRes,
    pub lossless: Lossless,
}

const AAC_KBPS: u32 = 256;
const PHONE_MAX_RATE: u32 = 48_000;

/// The highest rate the phone plays, staying in the source's family (44.1 kHz or 48 kHz).
fn phone_rate(rate: u32) -> u32 {
    match rate {
        r if r <= PHONE_MAX_RATE => r,
        r if r % 44_100 == 0 => 44_100,
        _ => PHONE_MAX_RATE,
    }
}

/// The action for a song, from its real format and its stream details (`None` = unknown,
/// treated as CD quality).
pub fn action(format: Format, rate: Option<u32>, bits: Option<u32>, policy: &Policy) -> Action {
    use Format::*;
    let rate = rate.unwrap_or(44_100);
    let bits = bits.unwrap_or(16);
    match format {
        Unknown => Action::Skip,
        Alac | Flac | Wav | Aiff | Ape | WavPack => {
            let hi_res = rate > PHONE_MAX_RATE || bits > 24;
            if format.iphone_playable() && !hi_res {
                return Action::Keep;
            }
            let (bits, rate) = match (hi_res, policy.hi_res) {
                (false, _) => (bits, rate),
                (true, HiRes::Alac24_48) => (bits.min(24), phone_rate(rate)),
                (true, HiRes::Alac16_44) => (16, 44_100),
            };
            match policy.lossless {
                Lossless::Alac => Action::Alac { bits, rate },
                Lossless::Aac256 => Action::Aac { kbps: AAC_KBPS, rate },
            }
        }
        Mp3 | Aac => Action::Keep,
        Opus | Vorbis | Wma => Action::Aac { kbps: AAC_KBPS, rate: phone_rate(rate) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::format::Format::{self, *};

    fn plan(format: Format, rate: Option<u32>, bits: Option<u32>) -> Action {
        action(format, rate, bits, &Policy::default())
    }

    #[test]
    fn formats_the_music_app_plays_are_kept() {
        assert_eq!(plan(Mp3, Some(44_100), None), Action::Keep);
        assert_eq!(plan(Aac, Some(44_100), None), Action::Keep);
        assert_eq!(plan(Alac, Some(44_100), Some(16)), Action::Keep);
        assert_eq!(plan(Wav, Some(48_000), Some(24)), Action::Keep, "24/48 is not hi-res for the phone");
        assert_eq!(plan(Aiff, Some(44_100), Some(16)), Action::Keep);
    }

    #[test]
    fn other_lossless_formats_become_alac_at_the_same_quality() {
        assert_eq!(plan(Flac, Some(44_100), Some(16)), Action::Alac { bits: 16, rate: 44_100 });
        assert_eq!(plan(Ape, Some(48_000), Some(24)), Action::Alac { bits: 24, rate: 48_000 });
        assert_eq!(plan(WavPack, Some(44_100), Some(16)), Action::Alac { bits: 16, rate: 44_100 });
    }

    #[test]
    fn other_lossy_formats_become_aac_256k() {
        assert_eq!(plan(Vorbis, Some(44_100), None), Action::Aac { kbps: 256, rate: 44_100 });
        assert_eq!(plan(Opus, Some(48_000), None), Action::Aac { kbps: 256, rate: 48_000 });
        assert_eq!(plan(Wma, Some(44_100), None), Action::Aac { kbps: 256, rate: 44_100 });
    }

    #[test]
    fn hi_res_becomes_alac_24_48_keeping_the_rate_family() {
        assert_eq!(plan(Flac, Some(96_000), Some(24)), Action::Alac { bits: 24, rate: 48_000 });
        assert_eq!(plan(Flac, Some(192_000), Some(24)), Action::Alac { bits: 24, rate: 48_000 });
        assert_eq!(plan(Flac, Some(88_200), Some(24)), Action::Alac { bits: 24, rate: 44_100 }, "88.2 halves cleanly to 44.1");
        assert_eq!(plan(Wav, Some(96_000), Some(24)), Action::Alac { bits: 24, rate: 48_000 }, "playable, but hi-res");
        assert_eq!(plan(Alac, Some(176_400), Some(24)), Action::Alac { bits: 24, rate: 44_100 });
        assert_eq!(plan(Wav, Some(48_000), Some(32)), Action::Alac { bits: 24, rate: 48_000 }, "32-bit is hi-res too");
    }

    #[test]
    fn the_space_saving_hi_res_setting_gives_16_44() {
        let p = Policy { hi_res: HiRes::Alac16_44, ..Policy::default() };
        assert_eq!(action(Flac, Some(96_000), Some(24), &p), Action::Alac { bits: 16, rate: 44_100 });
        assert_eq!(action(Flac, Some(44_100), Some(16), &p), Action::Alac { bits: 16, rate: 44_100 }, "CD quality untouched");
        assert_eq!(action(Wav, Some(48_000), Some(24), &p), Action::Keep, "only hi-res sources are affected");
    }

    #[test]
    fn the_aac_setting_turns_converted_lossless_into_aac_256k() {
        let p = Policy { lossless: Lossless::Aac256, ..Policy::default() };
        assert_eq!(action(Flac, Some(44_100), Some(16), &p), Action::Aac { kbps: 256, rate: 44_100 });
        assert_eq!(action(Flac, Some(96_000), Some(24), &p), Action::Aac { kbps: 256, rate: 48_000 });
        assert_eq!(action(Alac, Some(44_100), Some(16), &p), Action::Keep, "already playable lossless stays");
    }

    #[test]
    fn unknown_details_are_treated_as_cd_quality() {
        assert_eq!(plan(Flac, None, None), Action::Alac { bits: 16, rate: 44_100 });
        assert_eq!(plan(Vorbis, None, None), Action::Aac { kbps: 256, rate: 44_100 });
    }

    #[test]
    fn unrecognised_files_cannot_go_to_the_phone() {
        assert_eq!(plan(Unknown, Some(44_100), None), Action::Skip);
    }
}
