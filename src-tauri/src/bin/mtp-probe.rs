//! List the portable devices (MTP phones and players, PTP cameras, drives) Windows Portable Devices
//! sees, with their storages and free space. Read-only: nothing is written to any device.
//!
//! Usage: `cargo run --bin mtp-probe`

fn main() {
    #[cfg(windows)]
    match miu_music_lib::player::mtp::probe() {
        Ok(devices) if devices.is_empty() => println!("no portable devices plugged in"),
        Ok(devices) => {
            for d in devices {
                let kind = miu_music_lib::player::mtp::kind(&d.protocol, &d.manufacturer);
                println!("{} — {} [{}] {:?}", d.name, d.manufacturer.trim(), d.protocol, kind);
                for s in d.storages {
                    let gb = |b: Option<u64>| b.map_or("?".to_string(), |b| format!("{:.1} GB", b as f64 / 1e9));
                    println!("    {} ({}): {} free of {}", s.name, s.description, gb(s.free_bytes), gb(s.capacity_bytes));
                }
            }
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
    #[cfg(not(windows))]
    eprintln!("Windows only");
}
