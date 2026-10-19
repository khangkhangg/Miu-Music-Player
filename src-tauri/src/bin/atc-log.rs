//! Capture what the iPhone says over AirTraffic, without sending anything (F2.2).
//! Usage: `cargo run --bin atc-log` → `../research/captures/<UTC time>-atc.jsonl` (gitignored).

use std::sync::Arc;
use std::time::Duration;

use miu_music_lib::apple::airtraffic::AirTraffic;
use miu_music_lib::apple::cf::Cf;
use miu_music_lib::apple::device::{watch, MobileDevice};
use miu_music_lib::device::registry::{Interface, RawEvent};
use miu_music_lib::sync::atc_log::CaptureLog;
use miu_music_lib::util::time::now_unix;

fn main() {
    let dlls = miu_music_lib::apple::loader::load_from_system().unwrap_or_else(|e| fail(&e.to_string()));
    let cf = Arc::new(Cf::new(dlls.core_foundation).unwrap_or_else(|e| fail(&e.to_string())));
    let md = Arc::new(MobileDevice::new(dlls.mobile_device).unwrap_or_else(|e| fail(&e.to_string())));
    let (events, _handles) = watch(cf.clone(), md).unwrap_or_else(|e| fail(&e.to_string()));
    println!("waiting for a trusted iPhone…");
    let udid = loop {
        match events.recv_timeout(Duration::from_secs(30)) {
            Ok(RawEvent::Connected { udid, interface: Interface::Usb, trusted: true, .. }) => break udid,
            Ok(_) => continue,
            Err(_) => fail("no trusted iPhone over USB within 30 s"),
        }
    };
    let ath = AirTraffic::new(dlls.air_traffic_host).unwrap_or_else(|e| fail(&e.to_string()));
    let mut log = CaptureLog::create(std::path::Path::new("../research/captures"), now_unix()).unwrap_or_else(|e| fail(&e.to_string()));
    let n = ath
        .listen(&cf, &udid, Duration::from_secs(10), Duration::from_secs(60), |ms, name, msg| {
            println!("{ms:>6} ms  {}", name.as_deref().unwrap_or("<no name>"));
            log.record(ms, name.as_deref(), &msg).expect("write capture");
        })
        .unwrap_or_else(|e| fail(&e.to_string()));
    println!("{n} messages -> {}", log.path().display());
}

fn fail(msg: &str) -> ! {
    eprintln!("error: {msg}");
    std::process::exit(1)
}
