//! Background device discovery for the app: Apple notifications → `Registry` → `devices` event.

use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter};

use crate::apple::cf::Cf;
use crate::apple::device::{watch, MobileDevice};
use crate::device::registry::{short_udid, Change, Device, Registry};

pub type SharedRegistry = Arc<Mutex<Registry>>;

pub fn snapshot(registry: &SharedRegistry) -> Vec<Device> {
    registry.lock().map(|r| r.devices().into_iter().cloned().collect()).unwrap_or_default()
}

pub fn start(app: AppHandle, registry: SharedRegistry) {
    let spawned = std::thread::Builder::new().name("device-service".into()).spawn(move || {
        let dlls = match crate::apple::loader::load_from_system() {
            Ok(d) => d,
            Err(e) => return log::warn!("device discovery off: {e}"),
        };
        let cf = match Cf::new(dlls.core_foundation) {
            Ok(c) => Arc::new(c),
            Err(e) => return log::error!("CoreFoundation: {e}"),
        };
        let md = match MobileDevice::new(dlls.mobile_device) {
            Ok(m) => Arc::new(m),
            Err(e) => return log::error!("MobileDevice: {e}"),
        };
        let (events, _handles) = match watch(cf, md) {
            Ok(rx) => rx,
            Err(e) => return log::error!("device notifications: {e}"),
        };
        log::info!("watching for iPhones");
        for event in events {
            let change = registry.lock().ok().and_then(|mut r| r.apply(event));
            let Some(change) = change else { continue };
            match &change {
                Change::Added(d) => log::info!("iPhone {} connected (trusted: {})", short_udid(&d.udid), d.trusted),
                Change::Updated(d) => log::info!("iPhone {} trusted: {}", short_udid(&d.udid), d.trusted),
                Change::Removed(u) => log::info!("iPhone {} disconnected", short_udid(u)),
            }
            let _ = app.emit("devices", snapshot(&registry));
        }
    });
    if let Err(e) = spawned {
        log::error!("could not start device service: {e}");
    }
}
