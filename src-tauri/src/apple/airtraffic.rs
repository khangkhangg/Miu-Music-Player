//! AirTraffic (ATC) connection (F2.2). For now only **reads** messages: no host info, no sync
//! request. Prototypes (S1–S4, high confidence) in AIRTRAFFIC_NOTES.md "Published signatures".

use std::ffi::c_void;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use libloading::os::windows::Library;

use crate::apple::cf::{Cf, CfValue, Ref};

#[derive(Debug, thiserror::Error)]
pub enum AtcError {
    #[error("AirTrafficHost.dll is missing {0}")]
    Symbol(&'static str),
    #[error("ATHostConnectionCreate returned null")]
    CreateFailed,
    #[error("could not start the reader thread")]
    Thread,
}

#[allow(non_snake_case)]
#[derive(Clone, Copy)]
struct Fns {
    ATHostConnectionCreate: unsafe extern "C" fn(Ref) -> *mut c_void,
    ATHostConnectionReadMessage: unsafe extern "C" fn(*mut c_void) -> Ref,
    ATHostConnectionInvalidate: unsafe extern "C" fn(*mut c_void),
    ATHostConnectionRelease: unsafe extern "C" fn(*mut c_void),
    ATCFMessageGetName: unsafe extern "C" fn(Ref) -> Ref,
}

pub struct AirTraffic {
    f: Fns,
    _lib: Library,
}

macro_rules! ath_sym {
    ($lib:expr, $name:ident) => {
        // SAFETY: the type matches the published prototype (AIRTRAFFIC_NOTES.md).
        *unsafe { $lib.get(stringify!($name).as_bytes()) }.map_err(|_| AtcError::Symbol(stringify!($name)))?
    };
}

/// Raw connection pointer that may cross to the reader thread.
#[derive(Clone, Copy)]
struct Conn(*mut c_void);
// SAFETY: the connection is used by one reader thread plus Invalidate from the owner, which is how
// AirTrafficHost is meant to be stopped (cf. S1–S4, which invalidate/release from the main thread).
unsafe impl Send for Conn {}

impl AirTraffic {
    pub fn new(lib: Library) -> Result<Self, AtcError> {
        Ok(Self {
            f: Fns {
                ATHostConnectionCreate: ath_sym!(lib, ATHostConnectionCreate),
                ATHostConnectionReadMessage: ath_sym!(lib, ATHostConnectionReadMessage),
                ATHostConnectionInvalidate: ath_sym!(lib, ATHostConnectionInvalidate),
                ATHostConnectionRelease: ath_sym!(lib, ATHostConnectionRelease),
                ATCFMessageGetName: ath_sym!(lib, ATCFMessageGetName),
            },
            _lib: lib,
        })
    }

    /// Open an ATC connection to `udid` and hand every message the phone sends to `on_message`
    /// (ms since start, message name, full message). Stops after `quiet` without a message or
    /// `max` in total. Returns the number of messages. Sends nothing to the phone.
    pub fn listen(
        &self,
        cf: &Arc<Cf>,
        udid: &str,
        quiet: Duration,
        max: Duration,
        mut on_message: impl FnMut(u64, Option<String>, CfValue),
    ) -> Result<usize, AtcError> {
        let f = self.f;
        let udid_ref = cf.to_cf(&CfValue::String(udid.into())).map_err(|_| AtcError::CreateFailed)?;
        // SAFETY: udid_ref is a live CFString; the connection is released below.
        let conn = Conn(unsafe { (f.ATHostConnectionCreate)(udid_ref.as_ptr()) });
        drop(udid_ref);
        if conn.0.is_null() {
            return Err(AtcError::CreateFailed);
        }
        log::info!("ATC connection created");

        let started = Instant::now();
        let (tx, rx) = mpsc::channel::<(u64, Option<String>, CfValue)>();
        let reader_cf = cf.clone();
        let reader = std::thread::Builder::new()
            .name("atc-reader".into())
            .spawn(move || {
                let conn = conn;
                loop {
                    // SAFETY: conn stays valid until the owner releases it after this thread ends
                    // (or leaks it if the read never returns). The message is +1 and adopted.
                    let raw = unsafe { (f.ATHostConnectionReadMessage)(conn.0) };
                    let Ok(msg) = reader_cf.adopt(raw) else { break };
                    // SAFETY: GetName follows the CF "Get" rule: borrowed, not released.
                    let name = match reader_cf.from_cf(unsafe { (f.ATCFMessageGetName)(msg.as_ptr()) }) {
                        Ok(CfValue::String(s)) => Some(s),
                        _ => None,
                    };
                    let value = reader_cf.from_cf(msg.as_ptr()).unwrap_or(CfValue::Dict(Default::default()));
                    if tx.send((started.elapsed().as_millis() as u64, name, value)).is_err() {
                        break;
                    }
                }
            })
            .map_err(|_| AtcError::Thread)?;

        let mut count = 0;
        loop {
            let left = max.saturating_sub(started.elapsed());
            match rx.recv_timeout(quiet.min(left)) {
                Ok((ms, name, value)) => {
                    log::info!("ATC message {} at {ms} ms", name.as_deref().unwrap_or("<no name>"));
                    on_message(ms, name, value);
                    count += 1;
                }
                Err(_) => break,
            }
        }

        // SAFETY: Invalidate is meant to unblock a pending read; Release only once the reader is gone.
        unsafe { (f.ATHostConnectionInvalidate)(conn.0) };
        let deadline = Instant::now() + Duration::from_secs(5);
        while !reader.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        if reader.is_finished() {
            unsafe { (f.ATHostConnectionRelease)(conn.0) };
            log::info!("ATC connection closed after {count} messages");
        } else {
            log::warn!("ATC reader still blocked after Invalidate; leaking the connection");
        }
        Ok(count)
    }
}

// Machine test: needs the trusted iPhone. Sends nothing — only reads what the phone says.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::apple::device::{watch, MobileDevice};
    use crate::device::registry::{Interface, RawEvent};

    #[test]
    #[ignore]
    fn reads_messages_from_the_phone_without_sending_anything() {
        let dlls = crate::apple::loader::load_from_system().expect("Apple DLLs");
        let cf = Arc::new(Cf::new(dlls.core_foundation).unwrap());
        let md = Arc::new(MobileDevice::new(dlls.mobile_device).unwrap());
        let (events, _handles) = watch(cf.clone(), md).unwrap();
        let udid = loop {
            if let RawEvent::Connected { udid, interface: Interface::Usb, trusted: true, .. } =
                events.recv_timeout(Duration::from_secs(20)).expect("trusted iPhone")
            {
                break udid;
            }
        };
        let ath = AirTraffic::new(dlls.air_traffic_host).unwrap();
        let mut names = Vec::new();
        let n = ath
            .listen(&cf, &udid, Duration::from_secs(5), Duration::from_secs(20), |ms, name, _msg| {
                println!("{ms:>6} ms  {}", name.as_deref().unwrap_or("<no name>"));
                names.push(name);
            })
            .unwrap();
        assert!(n > 0, "the phone sent no AirTraffic messages");
        assert_eq!(n, names.len());
    }
}
