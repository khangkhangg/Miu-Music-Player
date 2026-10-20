//! Safeguards around an iTunes sync (Phase 4b, see `sync::guard`). Only reads from the phone.
//!
//! `cargo run --bin sync-guard -- before`
//!   backs up the phone's music database, copies every phone song with no copy in Miu's library
//!   into `%APPDATA%\MiuMusic\backups\<udid8>\media\`, and checks iTunes' view of the phone (read
//!   over COM). Exit code 2 = do not sync.
//! `cargo run --bin sync-guard -- after`
//!   backs up again and lists what the sync added and removed. Exit code 2 = songs were removed.

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use miu_music_lib::apple::afc::open_first_trusted;
use miu_music_lib::apple::itunes_com::ITunesCom;
use miu_music_lib::sync::itunes::ITunes;
use miu_music_lib::sync::backup::{backup, latest_backup};
use miu_music_lib::sync::guard::{copy_name, copy_off, diff, phone_only, phone_tracks, precheck, Concern, PcSong, PhoneTrack};
use miu_music_lib::util::{paths, time};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("before") => before(),
        Some("after") => after(),
        _ => fail("usage: sync-guard before | after"),
    }
}

fn before() {
    let (afc, udid) = open_first_trusted(Duration::from_secs(20)).unwrap_or_else(|e| fail(&e));
    let root = paths::backups_dir_in(&paths::appdata_root());
    let b = backup(&afc, &root, &udid, time::now_unix()).unwrap_or_else(|e| fail(&e.to_string()));
    let tracks = phone_tracks(&b.dir).unwrap_or_else(|e| fail(&e));
    println!("backup: {} ({} songs on the phone)", b.dir.display(), tracks.len());

    let pc = pc_songs(&paths::db_path_in(&paths::appdata_root()));
    let only = phone_only(&tracks, &pc);
    let media = b.dir.parent().unwrap_or(&root).join("media");
    let report = copy_off(&afc, &only, &media).unwrap_or_else(|e| fail(&e));
    println!(
        "{} songs are only on the phone: {} copied, {} already copied -> {}",
        only.len(),
        report.copied,
        report.already_there,
        media.display()
    );
    for (title, err) in &report.failed {
        println!("  could not copy {title}: {err}");
    }

    let itunes = ITunesCom::connect().unwrap_or_else(|e| fail(&e));
    let ids: HashSet<i64> = itunes.device_tracks().unwrap_or_else(|e| fail(&e)).into_iter().map(|t| t.id).collect();
    println!("iTunes lists {} tracks on the phone", ids.len());
    let concerns = precheck(&tracks, &ids);
    for c in &concerns {
        match c {
            Concern::MissingFromItunes(t) => println!("  NOT in iTunes' view (a sync would delete it): {}", describe(t)),
            Concern::LeadingZeroId(t) => println!("  warning: ID with a leading zero byte (the kind iTunes dropped; a copy is kept): {}", describe(t)),
        }
    }
    let blocked = concerns.iter().any(|c| matches!(c, Concern::MissingFromItunes(_)));
    if !blocked && report.failed.is_empty() {
        println!("ok to sync");
    } else {
        println!("do not sync");
        std::process::exit(2);
    }
}

fn after() {
    let (afc, udid) = open_first_trusted(Duration::from_secs(20)).unwrap_or_else(|e| fail(&e));
    let root = paths::backups_dir_in(&paths::appdata_root());
    let previous = latest_backup(&root, &udid).unwrap_or_else(|| fail("no earlier backup: run `sync-guard before` first"));
    let was = phone_tracks(&previous).unwrap_or_else(|e| fail(&e));
    let b = backup(&afc, &root, &udid, time::now_unix()).unwrap_or_else(|e| fail(&e.to_string()));
    let is = phone_tracks(&b.dir).unwrap_or_else(|e| fail(&e));
    println!("{} songs before ({}), {} now ({})", was.len(), previous.display(), is.len(), b.dir.display());
    let d = diff(&was, &is);
    for t in &d.added {
        println!("  added:   {}", describe(t));
    }
    let media = b.dir.parent().unwrap_or(&root).join("media");
    for t in &d.removed {
        let copy = media.join(copy_name(t));
        let note = if copy.is_file() { format!("copy kept at {}", copy.display()) } else { "on this PC".into() };
        println!("  REMOVED: {} ({note})", describe(t));
    }
    if !d.removed.is_empty() {
        std::process::exit(2);
    }
}

fn describe(t: &PhoneTrack) -> String {
    format!("{} [{}, id {:016X}]", t.title, t.path, t.sync_id as u64)
}

/// Miu's library, read-only (the app may have it open).
fn pc_songs(db: &Path) -> Vec<PcSong> {
    let Ok(c) = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return Vec::new();
    };
    let Ok(mut stmt) = c.prepare("SELECT COALESCE(title, ''), size, duration_ms FROM songs") else { return Vec::new() };
    stmt.query_map([], |r| Ok(PcSong { title: r.get(0)?, size: r.get::<_, i64>(1)?.max(0) as u64, duration_ms: r.get(2)? }))
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
}

fn fail(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1)
}
