//! Back up the plugged-in iPhone's music database (read-only on the phone).
//! Usage: `cargo run --bin device-backup` → `%APPDATA%\MiuMusic\backups\<udid8>\<UTC time>\`.

use std::time::Duration;

use miu_music_lib::apple::afc::open_first_trusted;
use miu_music_lib::sync::backup::{backup, song_count};
use miu_music_lib::util::{paths, time};

fn main() {
    let (afc, udid) = open_first_trusted(Duration::from_secs(20)).unwrap_or_else(|e| fail(&e));
    let root = paths::backups_dir_in(&paths::appdata_root());
    let b = backup(&afc, &root, &udid, time::now_unix()).unwrap_or_else(|e| fail(&e.to_string()));
    println!("backup: {}", b.dir.display());
    for (name, size) in &b.manifest.files {
        println!("  {name}: {size} bytes");
    }
    match song_count(&b.dir) {
        Ok(n) => println!("songs in backup: {n}"),
        Err(e) => fail(&format!("backup does not open: {e}")),
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1)
}
