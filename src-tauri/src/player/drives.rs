//! Which drives are music players: removable drives with a `Music` folder at the root (cheap MP3
//! players, Sony Walkman and SanDisk in USB mode, SD cards), and any drive the user set up as one.
//! A player is known by its volume serial number, so it keeps its settings and sync state whatever
//! drive letter Windows gives it next time.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;

use crate::player::profile::Profile;

/// A mounted volume, as Windows reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Volume {
    /// `E:\`
    pub root: PathBuf,
    pub label: String,
    pub serial: u32,
    /// Windows calls it removable (USB sticks, SD cards, most players). USB hard disks are "fixed".
    pub removable: bool,
    pub total_bytes: u64,
    pub free_bytes: u64,
}

/// The id a player's settings and sync state are kept under.
pub fn device_id(serial: u32) -> String {
    format!("vol:{serial:08X}")
}

/// How Miu reaches a player.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PlayerKind {
    /// A drive letter (USB stick, SD card, MP3 player in mass-storage mode).
    #[default]
    Drive,
    /// Windows Portable Devices (Android phone, Sony Walkman, newer SanDisk).
    Mtp,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Player {
    pub id: String,
    pub kind: PlayerKind,
    /// MTP only: the WPD device path and storage object id (both change between connections).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wpd_device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wpd_storage: Option<String>,
    pub root: PathBuf,
    pub label: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    /// Set up by the user; otherwise a suggestion from its `Music` folder.
    pub configured: bool,
    pub profile: Profile,
}

impl Player {
    /// The folder Miu fills.
    pub fn music_dir(&self) -> PathBuf {
        if self.profile.folder.is_empty() { self.root.clone() } else { self.root.join(&self.profile.folder) }
    }
}

/// The music folder at a drive's root, by its real name (`MUSIC`, `Music`).
fn music_folder(root_dirs: &[String]) -> Option<&String> {
    root_dirs.iter().find(|d| d.eq_ignore_ascii_case("music"))
}

/// The players among `volumes`: those set up in `known`, then removable drives that have a music
/// folder. `root_dirs` lists the folders at a drive's root.
pub fn players(volumes: &[Volume], known: &BTreeMap<String, Profile>, root_dirs: &dyn Fn(&std::path::Path) -> Vec<String>) -> Vec<Player> {
    let mut out = Vec::new();
    for v in volumes {
        let id = device_id(v.serial);
        let (configured, profile) = match known.get(&id) {
            Some(p) => (true, p.clone()),
            None if v.removable => match music_folder(&root_dirs(&v.root)) {
                Some(folder) => (false, Profile { folder: folder.clone(), ..Profile::default() }),
                None => continue,
            },
            None => continue,
        };
        out.push(Player { id, kind: PlayerKind::Drive, wpd_device: None, wpd_storage: None, root: v.root.clone(), label: v.label.clone(), total_bytes: v.total_bytes, free_bytes: v.free_bytes, configured, profile });
    }
    out
}

/// The volume a folder the user picked is on, and the folder relative to that volume's root.
pub fn volume_of<'a>(volumes: &'a [Volume], folder: &std::path::Path) -> Option<(&'a Volume, String)> {
    let text = folder.to_string_lossy();
    volumes.iter().find_map(|v| {
        let root = v.root.to_string_lossy();
        let rest = text.get(..root.len()).filter(|head| head.eq_ignore_ascii_case(&root)).map(|_| &text[root.len()..])?;
        Some((v, rest.trim_matches(['\\', '/']).to_string()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn vol(letter: char, serial: u32, removable: bool) -> Volume {
        Volume { root: PathBuf::from(format!("{letter}:\\")), label: format!("DISK {letter}"), serial, removable, total_bytes: 8_000_000_000, free_bytes: 2_000_000_000 }
    }

    fn dirs(map: &'static [(&'static str, &'static [&'static str])]) -> impl Fn(&Path) -> Vec<String> {
        move |root: &Path| map.iter().find(|(r, _)| Path::new(r) == root).map(|(_, d)| d.iter().map(|s| s.to_string()).collect()).unwrap_or_default()
    }

    #[test]
    fn the_id_is_the_volume_serial_so_drive_letters_do_not_matter() {
        assert_eq!(device_id(0x1A2B_3C4D), "vol:1A2B3C4D");
        assert_eq!(device_id(7), "vol:00000007");
    }

    #[test]
    fn a_removable_drive_with_a_music_folder_is_offered_as_a_player() {
        let vols = [vol('E', 1, true), vol('F', 2, true), vol('G', 3, false)];
        let found = players(&vols, &BTreeMap::new(), &dirs(&[("E:\\", &["MUSIC", "RECORD"]), ("F:\\", &["DCIM"]), ("G:\\", &["Music"])]));
        assert_eq!(found.len(), 1, "no music folder, or a fixed disk: not offered");
        let p = &found[0];
        assert_eq!((p.id.as_str(), p.configured, p.profile.folder.as_str()), ("vol:00000001", false, "MUSIC"), "the folder keeps its real name");
        assert_eq!(p.music_dir(), PathBuf::from("E:\\MUSIC"));
    }

    #[test]
    fn drives_the_user_set_up_are_players_whatever_they_hold() {
        let known = BTreeMap::from([(device_id(3), Profile { folder: "Songs".into(), ..Profile::default() })]);
        let found = players(&[vol('G', 3, false), vol('H', 4, true)], &known, &dirs(&[]));
        assert_eq!(found.len(), 1);
        assert!(found[0].configured);
        assert_eq!(found[0].music_dir(), PathBuf::from("G:\\Songs"));
        let root = BTreeMap::from([(device_id(4), Profile { folder: "".into(), ..Profile::default() })]);
        assert_eq!(players(&[vol('H', 4, true)], &root, &dirs(&[]))[0].music_dir(), PathBuf::from("H:\\"));
    }

    #[test]
    fn a_picked_folder_is_split_into_its_drive_and_the_folder_on_it() {
        let vols = [vol('E', 1, true), vol('F', 2, true)];
        let (v, folder) = volume_of(&vols, Path::new("f:\\Nhạc\\Miu")).unwrap();
        assert_eq!((v.serial, folder.as_str()), (2, "Nhạc\\Miu"));
        assert_eq!(volume_of(&vols, Path::new("E:\\")).unwrap().1, "");
        assert!(volume_of(&vols, Path::new("C:\\Users")).is_none());
    }
}
