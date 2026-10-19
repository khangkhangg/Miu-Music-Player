//! Device discovery (F1.3): subscribe to MobileDevice notifications and report USB iPhones.
//!
//! Signatures: see "Published signatures" in docs/AIRTRAFFIC_NOTES.md (S1, S5).
//! `AMDeviceGetInterfaceType` → `int (am_device*)`, 1 = USB, 2 = Wi-Fi (ios-deploy usage; medium).

use std::ffi::c_void;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use libloading::os::windows::Library;

use crate::apple::cf::{Cf, CfRef, CfValue, Ref};
use crate::device::registry::{Interface, NotifyKind, RawEvent};

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
    _lib: Library,
}

macro_rules! md_sym {
    ($lib:expr, $name:ident) => {
        // SAFETY: the type matches the published prototype (AIRTRAFFIC_NOTES.md).
        *unsafe { $lib.get(stringify!($name).as_bytes()) }.map_err(|_| DeviceError::Symbol(stringify!($name)))?
    };
}

impl MobileDevice {
    pub fn new(lib: Library) -> Result<Self, DeviceError> {
        Ok(Self {
            AMDeviceNotificationSubscribe: md_sym!(lib, AMDeviceNotificationSubscribe),
            AMDeviceCopyDeviceIdentifier: md_sym!(lib, AMDeviceCopyDeviceIdentifier),
            AMDeviceGetInterfaceType: md_sym!(lib, AMDeviceGetInterfaceType),
            AMDeviceConnect: md_sym!(lib, AMDeviceConnect),
            AMDeviceDisconnect: md_sym!(lib, AMDeviceDisconnect),
            AMDeviceIsPaired: md_sym!(lib, AMDeviceIsPaired),
            AMDeviceValidatePairing: md_sym!(lib, AMDeviceValidatePairing),
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

    /// "Trust this computer" accepted: paired and the pairing validates.
    fn is_trusted(&self, device: Ref) -> bool {
        // SAFETY: `device` is a retained am_device; every successful connect is matched by disconnect.
        unsafe {
            if (self.AMDeviceConnect)(device) != 0 {
                return false;
            }
            let trusted = (self.AMDeviceIsPaired)(device) != 0 && (self.AMDeviceValidatePairing)(device) == 0;
            (self.AMDeviceDisconnect)(device);
            trusted
        }
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
pub fn watch(cf: Arc<Cf>, md: Arc<MobileDevice>) -> Result<Receiver<RawEvent>, DeviceError> {
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
                let Some(udid) = md.udid(&cf, device.as_ptr()) else { continue };
                let interface = md.interface(device.as_ptr());
                let event = match kind {
                    NotifyKind::Connected => {
                        RawEvent::Connected { trusted: md.is_trusted(device.as_ptr()), udid, interface }
                    }
                    NotifyKind::Disconnected => RawEvent::Disconnected { udid, interface },
                    NotifyKind::Unsubscribed => continue,
                };
                if out_tx.send(event).is_err() {
                    break;
                }
            }
        })
        .map_err(|_| DeviceError::ThreadGone)?;

    Ok(out_rx)
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
        let events = watch(cf, md).expect("subscribe");
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match events.recv_timeout(left) {
                Ok(RawEvent::Connected { udid, interface: Interface::Usb, trusted }) => {
                    println!("iPhone {} over USB, trusted = {trusted}", crate::device::registry::short_udid(&udid));
                    assert!(udid.len() >= 24, "udid looks wrong: {udid:?}");
                    break;
                }
                Ok(other) => println!("other event: {other:?}"),
                Err(e) => panic!("no USB iPhone within 15 s ({e}) — is it plugged in and unlocked?"),
            }
        }
    }
}
