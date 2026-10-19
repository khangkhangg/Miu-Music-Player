//! The phone's live system log via `com.apple.syslog_relay` (read-only). Used to see why the
//! phone refuses a sync (e.g. atc's Grappa errors). Parsing lives in `device::syslog`.
//! `AMDServiceConnectionReceive(svc, buf, len) -> bytes read` (≤ 0 = closed); confidence medium.

use std::ffi::c_void;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::apple::cf::{Cf, CfRef, Ref};
use crate::apple::device::MobileDevice;
use crate::device::syslog::LineSplitter;

type Receive = unsafe extern "C" fn(Ref, *mut c_void, usize) -> isize;
type Invalidate = unsafe extern "C" fn(Ref);

pub struct SyslogRelay {
    service: CfRef,
    receive: Receive,
    invalidate: Invalidate,
}

/// The service connection pointer, for the reader thread.
#[derive(Clone, Copy)]
struct Svc(Ref);
// SAFETY: read by one thread; the owner only invalidates it (which is how a blocked receive ends).
unsafe impl Send for Svc {}

impl SyslogRelay {
    pub fn start(cf: &Cf, md: &MobileDevice, device: &CfRef) -> Result<Self, String> {
        let lib = md.library();
        // SAFETY: types follow the prototypes noted above.
        let receive: Receive = *unsafe { lib.get(b"AMDServiceConnectionReceive") }.map_err(|e| e.to_string())?;
        let invalidate: Invalidate = *unsafe { lib.get(b"AMDServiceConnectionInvalidate") }.map_err(|e| e.to_string())?;
        let service = md
            .start_service(cf, device.as_ptr(), "com.apple.syslog_relay")
            .map_err(|(call, rc)| format!("{call} failed with {rc:#x}"))?;
        Ok(Self { service, receive, invalidate })
    }

    /// Hand every log line received within `duration` to `on_line`, then stop.
    pub fn read_for(self, duration: Duration, mut on_line: impl FnMut(String)) {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let (svc, receive) = (Svc(self.service.as_ptr()), self.receive);
        let reader = std::thread::spawn(move || {
            let svc = svc;
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                // SAFETY: svc stays alive (owned by `self`) until after this thread is joined or abandoned.
                let n = unsafe { receive(svc.0, buf.as_mut_ptr().cast(), buf.len()) };
                if n <= 0 || tx.send(buf[..n as usize].to_vec()).is_err() {
                    break;
                }
            }
        });
        let started = Instant::now();
        let mut splitter = LineSplitter::default();
        while let Some(left) = duration.checked_sub(started.elapsed()) {
            match rx.recv_timeout(left) {
                Ok(chunk) => splitter.push(&chunk).into_iter().for_each(&mut on_line),
                Err(_) => break,
            }
        }
        // SAFETY: invalidating unblocks the reader's pending receive.
        unsafe { (self.invalidate)(self.service.as_ptr()) };
        let deadline = Instant::now() + Duration::from_secs(3);
        while !reader.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        if !reader.is_finished() {
            // Keep the connection alive rather than free it under a blocked reader.
            std::mem::forget(self.service);
        }
    }
}

// Machine test (read-only): listens to the phone's log for a few seconds.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::apple::device::watch;
    use crate::device::registry::{Interface, RawEvent};

    #[test]
    #[ignore]
    fn streams_live_log_lines_from_the_phone() {
        let dlls = crate::apple::loader::load_from_system().expect("Apple DLLs");
        let cf = Arc::new(Cf::new(dlls.core_foundation).unwrap());
        let md = Arc::new(MobileDevice::new(dlls.mobile_device).unwrap());
        let (events, handles) = watch(cf.clone(), md.clone()).unwrap();
        let udid = loop {
            if let RawEvent::Connected { udid, interface: Interface::Usb, trusted: true, .. } =
                events.recv_timeout(Duration::from_secs(20)).expect("trusted iPhone")
            {
                break udid;
            }
        };
        let device = handles.get(&udid).unwrap();
        let mut lines = Vec::new();
        let relay = SyslogRelay::start(&cf, &md, &device).expect("start syslog_relay");
        relay.read_for(Duration::from_secs(5), |line| lines.push(line));
        println!("{} lines; first: {:?}", lines.len(), lines.first());
        assert!(!lines.is_empty(), "no log lines in 5 s");
    }
}
