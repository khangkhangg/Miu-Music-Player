//! Record the plugged-in iPhone's log for a while (read-only syslog relay), e.g. during an iTunes
//! sync. Usage: `cargo run --bin phone-log -- <seconds>` →
//! `../research/captures/<UTC time>-phone.txt` (gitignored); sync-related lines are printed.

use std::sync::Arc;
use std::time::Duration;

use miu_music_lib::apple::cf::Cf;
use miu_music_lib::apple::device::{watch, MobileDevice};
use miu_music_lib::apple::syslog_relay::SyslogRelay;
use miu_music_lib::device::registry::{Interface, RawEvent};
use miu_music_lib::device::syslog::is_sync_related;
use miu_music_lib::util::time::now_unix;

fn main() {
    let seconds: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(120);
    let dlls = miu_music_lib::apple::loader::load_from_system().unwrap_or_else(|e| fail(&e.to_string()));
    let cf = Arc::new(Cf::new(dlls.core_foundation).unwrap_or_else(|e| fail(&e.to_string())));
    let md = Arc::new(MobileDevice::new(dlls.mobile_device).unwrap_or_else(|e| fail(&e.to_string())));
    let (events, handles) = watch(cf.clone(), md.clone()).unwrap_or_else(|e| fail(&e.to_string()));
    let udid = loop {
        match events.recv_timeout(Duration::from_secs(20)) {
            Ok(RawEvent::Connected { udid, interface: Interface::Usb, trusted: true, .. }) => break udid,
            Ok(_) => continue,
            Err(_) => fail("no trusted iPhone on USB"),
        }
    };
    let device = handles.get(&udid).unwrap_or_else(|| fail("iPhone unplugged"));
    let relay = SyslogRelay::start(&cf, &md, &device).unwrap_or_else(|e| fail(&e));
    println!("recording the phone's log for {seconds} s…");
    let mut lines = Vec::new();
    relay.read_for(Duration::from_secs(seconds), |l| lines.push(l));
    std::fs::create_dir_all("../research/captures").unwrap_or_else(|e| fail(&e.to_string()));
    let path = format!("../research/captures/{}-phone.txt", now_unix());
    std::fs::write(&path, lines.join("\n")).unwrap_or_else(|e| fail(&e.to_string()));
    let related: Vec<&String> = lines.iter().filter(|l| is_sync_related(l)).collect();
    println!("{} lines, {} sync-related -> {path}", lines.len(), related.len());
    for l in related {
        println!("  {l}");
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1)
}
