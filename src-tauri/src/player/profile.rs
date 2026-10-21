//! Per-player settings: which folder on the drive Miu fills, which formats the player plays as they
//! are, and what everything else is converted to. Stored in Miu's settings under `players`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::library::convert::{Action, Mp3Mode};
use crate::library::db::Db;
use crate::library::format::Format;

/// What songs the player cannot play become.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Target {
    /// LAME V0: transparent for almost everyone, smaller than 320.
    #[default]
    Mp3V0,
    Mp3320,
    Aac256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    /// The folder Miu fills, relative to the drive's root (`Music`). Empty = the root itself.
    pub folder: String,
    pub target: Target,
    /// Formats copied as they are besides MP3 (which every player plays), by `Format::name()`.
    #[serde(default)]
    pub plays: Vec<String>,
}

impl Default for Profile {
    fn default() -> Self {
        Profile { folder: "Music".into(), target: Target::default(), plays: Vec::new() }
    }
}

/// Highest sample rate MP3 and AAC take, staying in the source's family (44.1 or 48 kHz).
fn lossy_rate(rate: u32) -> u32 {
    match rate {
        r if r <= 48_000 => r,
        r if r % 44_100 == 0 => 44_100,
        _ => 48_000,
    }
}

impl Profile {
    fn plays_format(&self, f: Format) -> bool {
        f == Format::Mp3 || (f == Format::Aac && self.target == Target::Aac256) || self.plays.iter().any(|p| p == f.name())
    }

    /// What becomes of a song of this format on the player.
    pub fn action(&self, format: Format, rate: Option<u32>, _bits: Option<u32>) -> Action {
        if format == Format::Unknown {
            return Action::Skip;
        }
        if self.plays_format(format) {
            return Action::Keep;
        }
        let rate = lossy_rate(rate.unwrap_or(44_100));
        match self.target {
            Target::Mp3V0 => Action::Mp3 { mode: Mp3Mode::V0, rate },
            Target::Mp3320 => Action::Mp3 { mode: Mp3Mode::Cbr320, rate },
            Target::Aac256 => Action::Aac { kbps: 256, rate },
        }
    }

    /// Refuses folders that would leave the drive (`..`, absolute paths); trims slashes.
    pub fn checked(mut self) -> Result<Profile, String> {
        let folder = self.folder.trim().trim_matches(['\\', '/']).replace('/', "\\");
        if folder.contains(':') || folder.split('\\').any(|part| part.trim() == "..") {
            return Err(format!("not a folder on the player: {:?}", self.folder));
        }
        self.folder = folder;
        self.plays.retain(|p| Format::from_name(p) != Format::Unknown);
        self.plays.sort();
        self.plays.dedup();
        Ok(self)
    }
}

const PLAYERS: &str = "players";

/// Every player set up in Miu, by device id (`vol:XXXXXXXX`).
pub fn players(db: &Db) -> BTreeMap<String, Profile> {
    db.setting_json(PLAYERS).ok().flatten().unwrap_or_default()
}

pub fn set_player(db: &Db, id: &str, profile: Profile) -> Result<Profile, String> {
    let profile = profile.checked()?;
    let mut all = players(db);
    all.insert(id.to_string(), profile.clone());
    db.set_setting_json(PLAYERS, &all).map_err(|e| e.to_string())?;
    Ok(profile)
}

/// Stop treating a drive as a player. What Miu copied there stays on the drive, and Miu still
/// knows those files are its own if the drive is set up again.
pub fn forget_player(db: &Db, id: &str) -> Result<(), String> {
    let mut all = players(db);
    all.remove(id);
    db.set_setting_json(PLAYERS, &all).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use Format::*;

    fn with(target: Target, plays: &[&str]) -> Profile {
        Profile { folder: "Music".into(), target, plays: plays.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn mp3_always_goes_as_it_is_and_unknown_files_never_go() {
        let p = Profile::default();
        assert_eq!(p.action(Mp3, Some(44_100), None), Action::Keep);
        assert_eq!(p.action(Unknown, None, None), Action::Skip);
    }

    #[test]
    fn everything_else_becomes_the_target_format_by_default_mp3_v0() {
        let p = Profile::default();
        assert_eq!(p.action(Flac, Some(44_100), Some(16)), Action::Mp3 { mode: Mp3Mode::V0, rate: 44_100 });
        assert_eq!(p.action(Aac, Some(48_000), None), Action::Mp3 { mode: Mp3Mode::V0, rate: 48_000 }, "not every player reads M4A");
        assert_eq!(with(Target::Mp3320, &[]).action(Vorbis, None, None), Action::Mp3 { mode: Mp3Mode::Cbr320, rate: 44_100 });
        assert_eq!(with(Target::Aac256, &[]).action(Flac, Some(44_100), Some(16)), Action::Aac { kbps: 256, rate: 44_100 });
    }

    #[test]
    fn an_aac_player_takes_aac_files_as_they_are() {
        assert_eq!(with(Target::Aac256, &[]).action(Aac, Some(44_100), None), Action::Keep);
    }

    #[test]
    fn formats_the_player_plays_are_copied_as_they_are_even_hi_res() {
        let p = with(Target::Mp3V0, &["flac", "wav"]);
        assert_eq!(p.action(Flac, Some(96_000), Some(24)), Action::Keep);
        assert_eq!(p.action(Wav, Some(44_100), Some(16)), Action::Keep);
        assert_eq!(p.action(Alac, Some(44_100), Some(16)), Action::Mp3 { mode: Mp3Mode::V0, rate: 44_100 });
    }

    #[test]
    fn hi_res_sources_are_brought_down_to_a_rate_mp3_takes() {
        let p = Profile::default();
        assert_eq!(p.action(Flac, Some(96_000), Some(24)), Action::Mp3 { mode: Mp3Mode::V0, rate: 48_000 });
        assert_eq!(p.action(Flac, Some(88_200), Some(24)), Action::Mp3 { mode: Mp3Mode::V0, rate: 44_100 });
    }

    #[test]
    fn folders_outside_the_drive_are_refused_and_slashes_tidied() {
        assert_eq!(with(Target::Mp3V0, &[]).checked().unwrap().folder, "Music");
        let p = Profile { folder: "/MUSIC/Miu/".into(), ..Default::default() }.checked().unwrap();
        assert_eq!(p.folder, "MUSIC\\Miu");
        assert_eq!(Profile { folder: "\\".into(), ..Default::default() }.checked().unwrap().folder, "", "the root is allowed");
        assert!(Profile { folder: "..\\Windows".into(), ..Default::default() }.checked().is_err());
        assert!(Profile { folder: "C:\\Music".into(), ..Default::default() }.checked().is_err());
        let p = Profile { plays: vec!["flac".into(), "nonsense".into(), "flac".into()], ..Default::default() }.checked().unwrap();
        assert_eq!(p.plays, vec!["flac".to_string()]);
    }

    #[test]
    fn players_are_remembered_by_id_and_can_be_forgotten() {
        let db = Db::open_in_memory().unwrap();
        assert!(players(&db).is_empty());
        set_player(&db, "vol:1A2B3C4D", with(Target::Aac256, &["flac"])).unwrap();
        set_player(&db, "vol:00000001", Profile::default()).unwrap();
        assert_eq!(players(&db)["vol:1A2B3C4D"].target, Target::Aac256);
        forget_player(&db, "vol:1A2B3C4D").unwrap();
        assert_eq!(players(&db).keys().collect::<Vec<_>>(), vec!["vol:00000001"]);
        assert!(set_player(&db, "vol:2", Profile { folder: "..".into(), ..Default::default() }).is_err());
    }
}
