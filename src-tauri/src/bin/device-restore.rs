//! Put a backup of the music database back on the plugged-in iPhone. WRITES TO THE PHONE.
//! Backs up the phone's current state first. Refuses to run without `--yes-write-to-phone`.
//! Usage: `cargo run --bin device-restore -- <backup folder> --yes-write-to-phone`

use std::path::PathBuf;
use std::time::Duration;

use miu_music_lib::apple::afc::open_first_trusted;
use miu_music_lib::sync::backup::restore;
use miu_music_lib::util::{paths, time};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let confirmed = args.iter().any(|a| a == "--yes-write-to-phone");
    let Some(from) = args.iter().find(|a| !a.starts_with("--")).map(PathBuf::from) else {
        eprintln!("usage: device-restore <backup folder> --yes-write-to-phone");
        std::process::exit(2);
    };
    if !confirmed {
        eprintln!("refusing: this overwrites the phone's music database. Add --yes-write-to-phone to proceed.");
        std::process::exit(2);
    }
    let (afc, udid) = open_first_trusted(Duration::from_secs(20)).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1)
    });
    let root = paths::backups_dir_in(&paths::appdata_root());
    match restore(&afc, &from, &root, &udid, time::now_unix()) {
        Ok(safety) => println!("restored {}; previous phone state saved in {}", from.display(), safety.dir.display()),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
