//! Device discovery (F1.3): subscribe to MobileDevice notifications and report USB iPhones.
//!
//! Signatures: see "Published signatures" in docs/AIRTRAFFIC_NOTES.md (S1, S5).
//! `AMDeviceGetInterfaceType` → `int (am_device*)`, 1 = USB, 2 = Wi-Fi (ios-deploy usage; medium).

use std::ffi::c_void;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use libloading::os::windows::Library;

use crate::apple::cf::{Cf, CfRef, CfValue, Ref};
use crate::device::info::DeviceInfo;
use crate::device::registry::{short_udid, Interface, NotifyKind, RawEvent};

#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    #[error("MobileDevice.dll is missing {0}")]
    Symbol(&'static str),
    #[error("AMDeviceNotificationSubscribe failed: {0:#x}")]
    Subscribe(i32),
    #[error("device notification thread stopped")]
    ThreadGone,
}

/// `struct am_device_notification_callback_info` (packed).
#[repr(C, packed)]
struct NotificationInfo {
    device: Ref,
    msg: u32,
}

type NotifyCallback = unsafe extern "C" fn(*const NotificationInfo, *mut c_void);

#[allow(non_snake_case)]
pub struct MobileDevice {
    AMDeviceNotificationSubscribe: unsafe extern "C" fn(NotifyCallback, u32, u32, *mut c_void, *mut *mut c_void) -> i32,
    AMDeviceCopyDeviceIdentifier: unsafe extern "C" fn(Ref) -> Ref,
    AMDeviceGetInterfaceType: unsafe extern "C" fn(Ref) -> i32,
    AMDeviceConnect: unsafe extern "C" fn(Ref) -> i32,
    AMDeviceDisconnect: unsafe extern "C" fn(Ref) -> i32,
    AMDeviceIsPaired: unsafe extern "C" fn(Ref) -> i32,
    AMDeviceValidatePairing: unsafe extern "C" fn(Ref) -> i32,
    AMDeviceStartSession: unsafe extern "C" fn(Ref) -> i32,
    AMDeviceStopSession: unsafe extern "C" fn(Ref) -> i32,
    AMDeviceCopyValue: unsafe extern "C" fn(Ref, Ref, Ref) -> Ref,
    AMDeviceSecureStartService: unsafe extern "C" fn(Ref, Ref, Ref, *mut Ref) -> i32,
    _lib: Library,
}

macro_rules! md_sym {
    ($lib:expr, $name:ident) => {
        // SAFETY: the type matches the published prototype (AIRTRAFFIC_NOTES.md).
        *unsafe { $lib.get(stringify!($name).as_bytes()) }.map_err(|_| DeviceError::Symbol(stringify!($name)))?
    };
}

impl MobileDevice {
    /// The loaded MobileDevice.dll, for other parts of `apple/` that resolve more of its symbols.
    pub(crate) fn library(&self) -> &Library {
        &self._lib
    }

    pub fn new(lib: Library) -> Result<Self, DeviceError> {
        Ok(Self {
            AMDeviceNotificationSubscribe: md_sym!(lib, AMDeviceNotificationSubscribe),
            AMDeviceCopyDeviceIdentifier: md_sym!(lib, AMDeviceCopyDeviceIdentifier),
            AMDeviceGetInterfaceType: md_sym!(lib, AMDeviceGetInterfaceType),
            AMDeviceConnect: md_sym!(lib, AMDeviceConnect),
            AMDeviceDisconnect: md_sym!(lib, AMDeviceDisconnect),
            AMDeviceIsPaired: md_sym!(lib, AMDeviceIsPaired),
            AMDeviceValidatePairing: md_sym!(lib, AMDeviceValidatePairing),
            AMDeviceStartSession: md_sym!(lib, AMDeviceStartSession),
            AMDeviceStopSession: md_sym!(lib, AMDeviceStopSession),
            AMDeviceCopyValue: md_sym!(lib, AMDeviceCopyValue),
            AMDeviceSecureStartService: md_sym!(lib, AMDeviceSecureStartService),
            _lib: lib,
        })
    }

    fn udid(&self, cf: &Cf, device: Ref) -> Option<String> {
        // SAFETY: `device` is a retained am_device; the returned CFString is +1 and adopted.
        let s = cf.adopt(unsafe { (self.AMDeviceCopyDeviceIdentifier)(device) }).ok()?;
        match cf.from_cf(s.as_ptr()) {
            Ok(CfValue::String(u)) => Some(u),
            _ => None,
        }
    }

    fn interface(&self, device: Ref) -> Interface {
        // SAFETY: `device` is a retained am_device.
        Interface::from_raw(unsafe { (self.AMDeviceGetInterfaceType)(device) })
    }

    /// Trust check, and for a trusted phone its lockdown info (name, model, iOS, storage).
    fn inspect(&self, cf: &Cf, device: Ref) -> (bool, Option<DeviceInfo>) {
        // SAFETY: `device` is a retained am_device; connect/session calls are paired with
        // stop/disconnect, and values returned by AMDeviceCopyValue (+1) are adopted.
        unsafe {
            if (self.AMDeviceConnect)(device) != 0 {
                return (false, None);
            }
            let trusted = (self.AMDeviceIsPaired)(device) != 0 && (self.AMDeviceValidatePairing)(device) == 0;
            let mut info = None;
            if trusted && (self.AMDeviceStartSession)(device) == 0 {
                let general = self.copy_domain(cf, device, None);
                let disk = self.copy_domain(cf, device, Some("com.apple.disk_usage"));
                (self.AMDeviceStopSession)(device);
                info = Some(DeviceInfo::from_lockdown(&general, &disk));
            }
            (self.AMDeviceDisconnect)(device);
            (trusted, info)
        }
    }

    /// All values of a lockdown domain (`None` = default domain) as a dictionary.
    /// Start a lockdown service (e.g. `com.apple.afc`, `com.apple.syslog_relay`) on a trusted phone.
    /// Returns the owned AMDServiceConnection, or the failing call and its error code.
    pub fn start_service(&self, cf: &Cf, device: Ref, name: &str) -> Result<CfRef, (&'static str, i32)> {
        let name_ref = cf.to_cf(&CfValue::String(name.into())).map_err(|_| ("CFString", -1))?;
        // SAFETY: `device` is a retained am_device; connect/session are paired with stop/disconnect;
        // the service connection returned (+1) is adopted.
        unsafe {
            match (self.AMDeviceConnect)(device) {
                0 => {}
                e => return Err(("AMDeviceConnect", e)),
            }
            let mut service: Ref = std::ptr::null();
            let rc = match (self.AMDeviceValidatePairing)(device) {
                0 => match (self.AMDeviceStartSession)(device) {
                    0 => {
                        let rc = (self.AMDeviceSecureStartService)(device, name_ref.as_ptr(), std::ptr::null(), &mut service);
                        (self.AMDeviceStopSession)(device);
                        rc
                    }
                    e => e,
                },
                e => e,
            };
            (self.AMDeviceDisconnect)(device);
            if rc != 0 {
                return Err(("AMDeviceSecureStartService", rc));
            }
            cf.adopt(service).map_err(|_| ("AMDeviceSecureStartService", -1))
        }
    }

    unsafe fn copy_domain(&self, cf: &Cf, device: Ref, domain: Option<&str>) -> CfValue {
        let domain_ref = domain.and_then(|d| cf.to_cf(&CfValue::String(d.into())).ok());
        let domain_ptr = domain_ref.as_ref().map_or(std::ptr::null(), CfRef::as_ptr);
        let value = cf.adopt((self.AMDeviceCopyValue)(device, domain_ptr, std::ptr::null()));
        value.ok().and_then(|v| cf.from_cf(v.as_ptr()).ok()).unwrap_or(CfValue::Dict(Default::default()))
    }
}

struct CallbackContext {
    cf: Arc<Cf>,
    tx: Sender<(NotifyKind, CfRef)>,
}

unsafe extern "C" fn on_notification(info: *const NotificationInfo, ctx: *mut c_void) {
    let _ = std::panic::catch_unwind(|| {
        if info.is_null() || ctx.is_null() {
            return;
        }
        // SAFETY: MobileDevice passes a valid info struct and the context pointer we registered.
        let (device, msg) = unsafe {
            let info = std::ptr::read_unaligned(info);
            (info.device, info.msg)
        };
        let ctx = unsafe { &*(ctx as *const CallbackContext) };
        if let (Some(kind), false) = (NotifyKind::from_msg(msg), device.is_null()) {
            let _ = ctx.tx.send((kind, ctx.cf.retain(device)));
        }
    });
}

/// Start watching for iPhones. Existing devices are reported first, then plug/unplug events.
/// Runs two background threads for the life of the process.
pub fn watch(cf: Arc<Cf>, md: Arc<MobileDevice>) -> Result<(Receiver<RawEvent>, DeviceHandles), DeviceError> {
    let handles = DeviceHandles::new(cf.clone());
    let worker_handles = handles.clone();
    let (note_tx, note_rx) = mpsc::channel::<(NotifyKind, CfRef)>();
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), DeviceError>>();
    let (out_tx, out_rx) = mpsc::channel::<RawEvent>();

    let (cf_loop, md_loop) = (cf.clone(), md.clone());
    std::thread::Builder::new()
        .name("apple-notify".into())
        .spawn(move || {
            // Leaked on purpose: MobileDevice may call back for the rest of the process.
            let ctx = Box::into_raw(Box::new(CallbackContext { cf: cf_loop.clone(), tx: note_tx }));
            let mut handle: *mut c_void = std::ptr::null_mut();
            // SAFETY: callback/context stay valid forever; out-pointer is a local.
            let rc = unsafe { (md_loop.AMDeviceNotificationSubscribe)(on_notification, 0, 0, ctx.cast(), &mut handle) };
            let ok = rc == 0;
            let _ = ready_tx.send(if ok { Ok(()) } else { Err(DeviceError::Subscribe(rc)) });
            if ok {
                cf_loop.run_loop();
            }
        })
        .map_err(|_| DeviceError::ThreadGone)?;

    ready_rx.recv().map_err(|_| DeviceError::ThreadGone)??;

    std::thread::Builder::new()
        .name("apple-devices".into())
        .spawn(move || {
            for (kind, device) in note_rx {
                let Some(udid) = md.udid(&cf, device.as_ptr()) else {
                    log::warn!("device notification {kind:?} without a UDID");
                    continue;
                };
                let interface = md.interface(device.as_ptr());
                log::info!("device notification {kind:?} {} via {interface:?}", short_udid(&udid));
                let event = match kind {
                    NotifyKind::Connected => {
                        let started = std::time::Instant::now();
                        let (trusted, info) = md.inspect(&cf, device.as_ptr());
                        log::info!("trust check + info for {} took {} ms", short_udid(&udid), started.elapsed().as_millis());
                        if interface == Interface::Usb {
                            worker_handles.insert(udid.clone(), device);
                        }
                        RawEvent::Connected { trusted, udid, interface, info }
                    }
                    NotifyKind::Disconnected => {
                        if interface == Interface::Usb {
                            worker_handles.remove(&udid);
                        }
                        RawEvent::Disconnected { udid, interface }
                    }
                    NotifyKind::Unsubscribed => continue,
                };
                if out_tx.send(event).is_err() {
                    break;
                }
            }
        })
        .map_err(|_| DeviceError::ThreadGone)?;

    Ok((out_rx, handles))
}

/// Retained `am_device` handles of USB iPhones currently plugged in, by UDID.
#[derive(Clone)]
pub struct DeviceHandles {
    cf: Arc<Cf>,
    map: Arc<std::sync::Mutex<std::collections::HashMap<String, CfRef>>>,
}

impl DeviceHandles {
    fn new(cf: Arc<Cf>) -> Self {
        Self { cf, map: Default::default() }
    }

    fn insert(&self, udid: String, device: CfRef) {
        if let Ok(mut m) = self.map.lock() {
            m.insert(udid, device);
        }
    }

    fn remove(&self, udid: &str) {
        if let Ok(mut m) = self.map.lock() {
            m.remove(udid);
        }
    }

    /// An extra retain on the device, so it stays valid even if it is unplugged meanwhile.
    pub fn get(&self, udid: &str) -> Option<CfRef> {
        self.map.lock().ok()?.get(udid).map(|d| self.cf.retain(d.as_ptr()))
    }
}

// Machine tests: need Apple's DLLs and the iPhone plugged in. Run with `cargo test -- --include-ignored`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::registry::{Interface, RawEvent};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    #[ignore]
    fn sees_the_plugged_in_iphone_over_usb() {
        let dlls = crate::apple::loader::load_from_system().expect("Apple DLLs");
        let cf = Arc::new(crate::apple::cf::Cf::new(dlls.core_foundation).unwrap());
        let md = Arc::new(MobileDevice::new(dlls.mobile_device).unwrap());
        let (events, _handles) = watch(cf, md).expect("subscribe");
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match events.recv_timeout(left) {
                Ok(RawEvent::Connected { udid, interface: Interface::Usb, trusted, info }) => {
                    println!("iPhone {} over USB, trusted = {trusted}", crate::device::registry::short_udid(&udid));
                    assert!(udid.len() >= 24, "udid looks wrong: {udid:?}");
                    // The test phone (iPhone 5s, iOS 12.5.8) trusts this PC, so its info must be readable.
                    let info = info.expect("device info for a trusted iPhone");
                    println!("{} · {} ({}) · iOS {} · {:?} free of {:?}", info.name, info.model, info.model_id,
                        info.ios_version, info.free_bytes, info.capacity_bytes);
                    assert_eq!(info.model, "iPhone 5s");
                    assert_eq!(info.ios_version, "12.5.8");
                    assert!(info.free_bytes.unwrap() < info.capacity_bytes.unwrap());
                    break;
                }
                Ok(other) => println!("other event: {other:?}"),
                Err(e) => panic!("no USB iPhone within 15 s ({e}) — is it plugged in and unlocked?"),
            }
        }
    }
}
