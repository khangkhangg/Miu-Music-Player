//! Dump the export tables of Apple's DLLs (F2.1).
//!
//! Usage: `cargo run --bin dll-exports -- [--out DIR] [DLL ...]`
//! With no DLLs given, uses the ones `check_prereqs` finds on this PC.
//! Writes one `<dll>.txt` per DLL into DIR (default `../research/dll-exports`, gitignored).

use std::path::PathBuf;

use miu_music_lib::apple::exports::{export_names, group_by_prefix};
use miu_music_lib::apple::locate::locate_on_system;

const PREFIXES: &[&str] = &["ATHost", "AMDevice", "AMDServiceConnection", "AFC", "AMRestorableDevice", "CF"];

fn main() {
    let mut args = std::env::args().skip(1);
    let mut out = PathBuf::from("../research/dll-exports");
    let mut dlls = Vec::new();
    while let Some(a) = args.next() {
        if a == "--out" {
            out = args.next().map(PathBuf::from).expect("--out needs a directory");
        } else {
            dlls.push(PathBuf::from(a));
        }
    }
    if dlls.is_empty() {
        let found = locate_on_system();
        dlls = [found.air_traffic_host, found.mobile_device, found.core_foundation].into_iter().flatten().collect();
    }
    std::fs::create_dir_all(&out).expect("create output dir");

    for dll in dlls {
        let names = match export_names(&dll) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("skip {e}");
                continue;
            }
        };
        let file = dll.file_name().unwrap().to_string_lossy().into_owned();
        let dump = out.join(format!("{file}.txt"));
        std::fs::write(&dump, names.join("\n") + "\n").expect("write dump");
        println!("{file}: {} exports -> {}", names.len(), dump.display());
        for (prefix, group) in group_by_prefix(&names, PREFIXES) {
            if !group.is_empty() {
                println!("  {prefix}*: {}", group.len());
            }
        }
    }
}
