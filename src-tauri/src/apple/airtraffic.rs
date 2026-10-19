//! AirTraffic (ATC) connection. `listen` (F2.2) only reads; `handshake` (F2.3) also sends HostInfo
//! and one sync request. Prototypes (S1–S4, high confidence) in AIRTRAFFIC_NOTES.md.

use std::ffi::c_void;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use libloading::os::windows::Library;

use crate::apple::cf::{Cf, CfValue, Ref};
use crate::sync::handshake::{Action, Handshake, Outcome, Phase};

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
    ATHostConnectionSendHostInfo: unsafe extern "C" fn(*mut c_void, Ref),
    ATHostConnectionSendSyncRequest: unsafe extern "C" fn(*mut c_void, Ref, Ref, Ref),
}

/// What happened during a handshake, in order, for logging and captures.
#[derive(Debug, Clone)]
pub enum HandshakeEvent {
    Received { ms: u64, name: Option<String>, message: CfValue },
    Sent { ms: u64, what: &'static str },
}

type Incoming = (u64, Option<String>, CfValue);

/// An open connection plus its reader thread. Closed (Invalidate, then Release) by `close`.
struct Session {
    f: Fns,
    conn: Conn,
    rx: mpsc::Receiver<Incoming>,
    reader: std::thread::JoinHandle<()>,
    started: Instant,
}

impl Session {
    fn ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    fn close(self, count: usize) {
        // SAFETY: Invalidate is meant to unblock a pending read; Release only once the reader is gone.
        unsafe { (self.f.ATHostConnectionInvalidate)(self.conn.0) };
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.reader.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        if self.reader.is_finished() {
            unsafe { (self.f.ATHostConnectionRelease)(self.conn.0) };
            log::info!("ATC connection closed after {count} messages");
        } else {
            log::warn!("ATC reader still blocked after Invalidate; leaking the connection");
        }
    }
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
                ATHostConnectionSendHostInfo: ath_sym!(lib, ATHostConnectionSendHostInfo),
                ATHostConnectionSendSyncRequest: ath_sym!(lib, ATHostConnectionSendSyncRequest),
            },
            _lib: lib,
        })
    }

    fn open(&self, cf: &Arc<Cf>, udid: &str) -> Result<Session, AtcError> {
        let f = self.f;
        let udid_ref = cf.to_cf(&CfValue::String(udid.into())).map_err(|_| AtcError::CreateFailed)?;
        // SAFETY: udid_ref is a live CFString; the connection is released in `Session::close`.
        let conn = Conn(unsafe { (f.ATHostConnectionCreate)(udid_ref.as_ptr()) });
        drop(udid_ref);
        if conn.0.is_null() {
            return Err(AtcError::CreateFailed);
        }
        log::info!("ATC connection created");

        let started = Instant::now();
        let (tx, rx) = mpsc::channel::<Incoming>();
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
        Ok(Session { f, conn, rx, reader, started })
    }

    /// F2.3: wait for SyncAllowed, send `host_info`, then the sync request, and stop at
    /// ReadyForSync / SyncFailed / timeout (see `sync::handshake`). Sends nothing else.
    pub fn handshake(
        &self,
        cf: &Arc<Cf>,
        udid: &str,
        host_info: &CfValue,
        dataclasses: &CfValue,
        anchors: &CfValue,
        mut on_event: impl FnMut(HandshakeEvent),
    ) -> Result<Outcome, AtcError> {
        let to_cf = |v: &CfValue| cf.to_cf(v).map_err(|_| AtcError::CreateFailed);
        let (info_ref, classes_ref, anchors_ref) = (to_cf(host_info)?, to_cf(dataclasses)?, to_cf(anchors)?);
        let session = self.open(cf, udid)?;
        let mut machine = Handshake::new(0);
        let mut count = 0;
        let outcome = loop {
            let action = match session.rx.recv_timeout(Duration::from_millis(200)) {
                Ok((ms, name, message)) => {
                    log::info!("ATC message {} at {ms} ms", name.as_deref().unwrap_or("<no name>"));
                    count += 1;
                    let action = machine.on_message(name.as_deref().unwrap_or(""), ms);
                    on_event(HandshakeEvent::Received { ms, name, message });
                    action
                }
                Err(mpsc::RecvTimeoutError::Timeout) => machine.on_tick(session.ms()),
                Err(mpsc::RecvTimeoutError::Disconnected) => break Outcome::Timeout(Phase::WaitingForReadyForSync),
            };
            match action {
                Action::Wait => {}
                Action::SendHostInfo => {
                    // SAFETY: connection open; dictionary is a live CF object for the call.
                    unsafe { (self.f.ATHostConnectionSendHostInfo)(session.conn.0, info_ref.as_ptr()) };
                    log::info!("ATC sent HostInfo");
                    on_event(HandshakeEvent::Sent { ms: session.ms(), what: "HostInfo" });
                }
                Action::SendSyncRequest => {
                    // SAFETY: connection open; all three arguments are live CF objects for the call.
                    unsafe {
                        (self.f.ATHostConnectionSendSyncRequest)(
                            session.conn.0,
                            classes_ref.as_ptr(),
                            anchors_ref.as_ptr(),
                            info_ref.as_ptr(),
                        )
                    };
                    log::info!("ATC sent SyncRequest");
                    on_event(HandshakeEvent::Sent { ms: session.ms(), what: "SyncRequest" });
                }
                Action::Stop(outcome) => break outcome,
            }
        };
        log::info!("ATC handshake ended: {outcome:?}");
        session.close(count);
        Ok(outcome)
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
        let session = self.open(cf, udid)?;
        let mut count = 0;
        loop {
            let left = max.saturating_sub(session.started.elapsed());
            match session.rx.recv_timeout(quiet.min(left)) {
                Ok((ms, name, value)) => {
                    log::info!("ATC message {} at {ms} ms", name.as_deref().unwrap_or("<no name>"));
                    on_message(ms, name, value);
                    count += 1;
                }
                Err(_) => break,
            }
        }
        session.close(count);
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

/// F2.3 live handshake. Doubly gated: `#[ignore]` and `MIU_ALLOW_SYNC_REQUEST=1`, because it sends
/// HostInfo and a sync request to the phone. Prefer `cargo run --bin atc-handshake` (backs up first).
#[cfg(test)]
mod handshake_machine_tests {
    use super::*;
    use crate::apple::device::{watch, MobileDevice};
    use crate::device::registry::{Interface, RawEvent};
    use crate::sync::host_info::{host_info, sync_anchors, sync_dataclasses};

    #[test]
    #[ignore]
    fn media_handshake_reaches_ready_for_sync() {
        if std::env::var("MIU_ALLOW_SYNC_REQUEST").as_deref() != Ok("1") {
            println!("skipped: set MIU_ALLOW_SYNC_REQUEST=1 to send a sync request to the phone");
            return;
        }
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
        let outcome = ath
            .handshake(&cf, &udid, &host_info("0123456789ABCDEF"), &sync_dataclasses(), &sync_anchors(), |e| println!("{e:?}"))
            .unwrap();
        assert_eq!(outcome, Outcome::ReadyForSync);
    }
}
