//! F2.3 live run: backup → Media sync handshake (HostInfo + SyncRequest only) → backup → diff.
//! SENDS TO THE PHONE. Refuses to run unless MIU_ALLOW_SYNC_REQUEST=1.
//! Plan and exact payloads: docs/plans/F2.3-sync-handshake.md.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use miu_music_lib::apple::afc::Afc;
use miu_music_lib::apple::airtraffic::{AirTraffic, HandshakeEvent};
use miu_music_lib::apple::cf::Cf;
use miu_music_lib::apple::device::{watch, MobileDevice};
use miu_music_lib::apple::syslog_relay::SyslogRelay;
use miu_music_lib::device::syslog::is_sync_related;
use miu_music_lib::device::registry::{Interface, RawEvent};
use miu_music_lib::library::db::Db;
use miu_music_lib::sync::atc_log::CaptureLog;
use miu_music_lib::sync::backup::{backup, song_count, DB_NAME};
use miu_music_lib::sync::dbdiff::diff;
use miu_music_lib::sync::host_info::{host_info, library_id, sync_anchors, sync_dataclasses};
use miu_music_lib::util::{paths, time::now_unix};

fn main() {
    if std::env::var("MIU_ALLOW_SYNC_REQUEST").as_deref() != Ok("1") {
        fail("refusing: this sends HostInfo and a sync request to the phone. Set MIU_ALLOW_SYNC_REQUEST=1.");
    }
    // Kept alive for the whole run when --private-corefp is used.
    let _corefp = if std::env::args().any(|a| a == "--private-corefp") { Some(private_corefp()) } else { None };
    let dlls = miu_music_lib::apple::loader::load_from_system().unwrap_or_else(|e| fail(&e.to_string()));
    let cf = Arc::new(Cf::new(dlls.core_foundation).unwrap_or_else(|e| fail(&e.to_string())));
    let md = Arc::new(MobileDevice::new(dlls.mobile_device).unwrap_or_else(|e| fail(&e.to_string())));
    let (events, handles) = watch(cf.clone(), md.clone()).unwrap_or_else(|e| fail(&e.to_string()));
    println!("waiting for a trusted iPhone…");
    let udid = loop {
        match events.recv_timeout(Duration::from_secs(30)) {
            Ok(RawEvent::Connected { udid, interface: Interface::Usb, trusted: true, .. }) => break udid,
            Ok(_) => continue,
            Err(_) => fail("no trusted iPhone over USB within 30 s"),
        }
    };
    let root = paths::backups_dir_in(&paths::appdata_root());
    let take_backup = |label: &str| {
        let device = handles.get(&udid).unwrap_or_else(|| fail("iPhone unplugged"));
        let afc = Afc::open(cf.clone(), md.clone(), &device).unwrap_or_else(|e| fail(&e.to_string()));
        let b = backup(&afc, &root, &udid, now_unix()).unwrap_or_else(|e| fail(&e.to_string()));
        let songs = song_count(&b.dir).unwrap_or_else(|e| fail(&e.to_string()));
        println!("backup {label}: {} ({songs} songs)", b.dir.display());
        (b, songs)
    };

    let (before, songs_before) = take_backup("before");
    std::thread::sleep(Duration::from_secs(1)); // distinct backup folder names

    let db = Db::open(&paths::db_path_in(&paths::appdata_root())).unwrap_or_else(|e| fail(&e.to_string()));
    let id = library_id(&db).unwrap_or_else(|e| fail(&e.to_string()));
    let info = host_info(&id);
    println!("HostInfo: {}", info.to_json());

    // Phone log, captured during the handshake (read-only service).
    let device = handles.get(&udid).unwrap_or_else(|| fail("iPhone unplugged"));
    let relay = SyslogRelay::start(&cf, &md, &device).unwrap_or_else(|e| fail(&e));
    let syslog = std::thread::spawn(move || {
        let mut lines = Vec::new();
        relay.read_for(Duration::from_secs(14), |l| lines.push(l));
        lines
    });
    std::thread::sleep(Duration::from_millis(500));

    let ath = AirTraffic::new(dlls.air_traffic_host).unwrap_or_else(|e| fail(&e.to_string()));
    let mut log = CaptureLog::create(Path::new("../research/captures"), now_unix()).unwrap_or_else(|e| fail(&e.to_string()));
    let outcome = ath
        .handshake(&cf, &udid, &info, &sync_dataclasses(), &sync_anchors(), |e| match e {
            HandshakeEvent::Received { ms, name, message } => {
                println!("{ms:>6} ms  <- {}", name.as_deref().unwrap_or("<no name>"));
                log.record(ms, name.as_deref(), &message).expect("write capture");
            }
            HandshakeEvent::Sent { ms, what } => println!("{ms:>6} ms  -> {what}"),
        })
        .unwrap_or_else(|e| fail(&e.to_string()));
    println!("outcome: {outcome:?}  (capture: {})", log.path().display());

    let lines = syslog.join().unwrap_or_default();
    let syslog_path = log.path().with_file_name(
        log.path().file_name().unwrap().to_string_lossy().replace("-atc.jsonl", "-syslog.txt"),
    );
    std::fs::write(&syslog_path, lines.join("\n")).unwrap_or_else(|e| fail(&e.to_string()));
    let related: Vec<&String> = lines.iter().filter(|l| is_sync_related(l)).collect();
    println!("phone log: {} lines, {} sync-related -> {}", lines.len(), related.len(), syslog_path.display());
    for l in related {
        println!("  {l}");
    }

    std::thread::sleep(Duration::from_secs(3)); // let the phone settle after the aborted session
    let (after, songs_after) = take_backup("after");
    match diff(&before.dir.join(DB_NAME), &after.dir.join(DB_NAME)) {
        Ok(d) if d.is_empty() => println!("db-diff: no differences"),
        Ok(d) => d.tables.iter().for_each(|t| println!("db-diff: {}", t.summary())),
        Err(e) => println!("db-diff failed: {e}"),
    }
    println!("songs: {songs_before} -> {songs_after}");
}

/// Copy the Store package's CoreFP.dll (+ imports) into Miu's private DLL folder and make it findable
/// for this process only: folder first on this process's PATH, and the DLL pre-loaded so a load by
/// name returns it. No registry or system change.
fn private_corefp() -> libloading::os::windows::Library {
    use miu_music_lib::apple::{loader, locate, stage};
    let found = locate::locate_on_system();
    let cache = paths::apple_dlls_dir_in(&paths::local_appdata_root());
    let corefp = stage::stage_extra(&found, &cache, "CoreFP.dll").unwrap_or_else(|e| fail(&format!("stage CoreFP: {e}")));
    let dir = corefp.parent().unwrap().to_path_buf();
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs = vec![dir.clone()];
    dirs.extend(std::env::split_paths(&path));
    std::env::set_var("PATH", std::env::join_paths(dirs).unwrap());
    let lib = loader::load_dll(&corefp, &[dir]).unwrap_or_else(|e| fail(&format!("load CoreFP: {e}")));
    println!("private CoreFP loaded for this process: {}", corefp.display());
    lib
}

fn fail(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1)
}
