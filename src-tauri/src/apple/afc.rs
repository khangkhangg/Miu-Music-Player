//! AFC: file access to the iPhone's media partition (F1.5). Every path goes through
//! `afc_path::device_path` first. Prototypes: "Published signatures" in AIRTRAFFIC_NOTES.md.
//! `AFCDirectoryCreate(conn, const char*)` and file open mode 3 = write/create/truncate follow
//! libimobiledevice's AFC constants (confidence medium, confirmed by the round-trip test).
//! On MobileDevice 1860.2.5.1 (x64), `AFCConnectionOpen` takes the **raw socket** from
//! `AMDServiceConnectionGetSocket`; passing the service-connection object opens a connection
//! whose first request fails with AFC error 1.

use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::Arc;

use crate::apple::afc_path::{device_path, PathError};
use crate::apple::cf::{Cf, CfRef, CfValue, Ref};
use crate::apple::device::MobileDevice;

const MODE_READ: u64 = 1;
const MODE_WRITE_CREATE_TRUNCATE: u64 = 3;
const CHUNK: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum AfcError {
    #[error(transparent)]
    Path(#[from] PathError),
    #[error("MobileDevice.dll is missing {0}")]
    Symbol(&'static str),
    #[error("{0} failed with {1:#x}")]
    Device(&'static str, i32),
    #[error("{op} {path} failed with AFC error {code}")]
    Afc { op: &'static str, path: String, code: i32 },
}

#[allow(non_snake_case)]
struct Fns {
    AMDeviceConnect: unsafe extern "C" fn(Ref) -> i32,
    AMDeviceDisconnect: unsafe extern "C" fn(Ref) -> i32,
    AMDeviceValidatePairing: unsafe extern "C" fn(Ref) -> i32,
    AMDeviceStartSession: unsafe extern "C" fn(Ref) -> i32,
    AMDeviceStopSession: unsafe extern "C" fn(Ref) -> i32,
    AMDeviceSecureStartService: unsafe extern "C" fn(Ref, Ref, Ref, *mut Ref) -> i32,
    AFCConnectionOpen: unsafe extern "C" fn(usize, u32, *mut *mut c_void) -> i32,
    AMDServiceConnectionGetSocket: unsafe extern "C" fn(Ref) -> usize,
    AFCConnectionClose: unsafe extern "C" fn(*mut c_void) -> i32,
    AFCDirectoryOpen: unsafe extern "C" fn(*mut c_void, *const c_char, *mut *mut c_void) -> i32,
    AFCDirectoryRead: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut *const c_char) -> i32,
    AFCDirectoryClose: unsafe extern "C" fn(*mut c_void, *mut c_void) -> i32,
    AFCDirectoryCreate: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32,
    AFCFileRefOpen: unsafe extern "C" fn(*mut c_void, *const c_char, u64, *mut u64) -> i32,
    AFCFileRefRead: unsafe extern "C" fn(*mut c_void, u64, *mut u8, *mut usize) -> i32,
    AFCFileRefWrite: unsafe extern "C" fn(*mut c_void, u64, *const u8, usize) -> i32,
    AFCFileRefClose: unsafe extern "C" fn(*mut c_void, u64) -> i32,
    AFCRemovePath: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32,
    AFCRenamePath: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> i32,
}

macro_rules! afc_sym {
    ($lib:expr, $name:ident) => {
        // SAFETY: the type matches the published prototype (AIRTRAFFIC_NOTES.md).
        *unsafe { $lib.get(stringify!($name).as_bytes()) }.map_err(|_| AfcError::Symbol(stringify!($name)))?
    };
}

impl Fns {
    fn load(md: &MobileDevice) -> Result<Self, AfcError> {
        let lib = md.library();
        Ok(Self {
            AMDeviceConnect: afc_sym!(lib, AMDeviceConnect),
            AMDeviceDisconnect: afc_sym!(lib, AMDeviceDisconnect),
            AMDeviceValidatePairing: afc_sym!(lib, AMDeviceValidatePairing),
            AMDeviceStartSession: afc_sym!(lib, AMDeviceStartSession),
            AMDeviceStopSession: afc_sym!(lib, AMDeviceStopSession),
            AMDeviceSecureStartService: afc_sym!(lib, AMDeviceSecureStartService),
            AFCConnectionOpen: afc_sym!(lib, AFCConnectionOpen),
            AMDServiceConnectionGetSocket: afc_sym!(lib, AMDServiceConnectionGetSocket),
            AFCConnectionClose: afc_sym!(lib, AFCConnectionClose),
            AFCDirectoryOpen: afc_sym!(lib, AFCDirectoryOpen),
            AFCDirectoryRead: afc_sym!(lib, AFCDirectoryRead),
            AFCDirectoryClose: afc_sym!(lib, AFCDirectoryClose),
            AFCDirectoryCreate: afc_sym!(lib, AFCDirectoryCreate),
            AFCFileRefOpen: afc_sym!(lib, AFCFileRefOpen),
            AFCFileRefRead: afc_sym!(lib, AFCFileRefRead),
            AFCFileRefWrite: afc_sym!(lib, AFCFileRefWrite),
            AFCFileRefClose: afc_sym!(lib, AFCFileRefClose),
            AFCRemovePath: afc_sym!(lib, AFCRemovePath),
            AFCRenamePath: afc_sym!(lib, AFCRenamePath),
        })
    }
}

/// An open AFC connection to one iPhone's media partition. Closed on drop.
pub struct Afc {
    f: Fns,
    conn: *mut c_void,
    _service: CfRef,
    _md: Arc<MobileDevice>,
    _cf: Arc<Cf>,
}

impl Afc {
    pub fn open(cf: Arc<Cf>, md: Arc<MobileDevice>, device: &CfRef) -> Result<Self, AfcError> {
        let f = Fns::load(&md)?;
        let dev = device.as_ptr();
        let name = cf.to_cf(&CfValue::String("com.apple.afc".into())).map_err(|_| AfcError::Device("CFString", -1))?;
        // SAFETY: `dev` is a retained am_device; connect/session are paired with stop/disconnect;
        // the service connection returned (+1) is adopted and outlives the AFC connection.
        unsafe {
            check("AMDeviceConnect", (f.AMDeviceConnect)(dev))?;
            let mut service: Ref = std::ptr::null();
            let rc = match (f.AMDeviceValidatePairing)(dev) {
                0 => match (f.AMDeviceStartSession)(dev) {
                    0 => {
                        let rc = (f.AMDeviceSecureStartService)(dev, name.as_ptr(), std::ptr::null(), &mut service);
                        (f.AMDeviceStopSession)(dev);
                        rc
                    }
                    e => e,
                },
                e => e,
            };
            (f.AMDeviceDisconnect)(dev);
            check("AMDeviceSecureStartService(com.apple.afc)", rc)?;
            let service = cf.adopt(service).map_err(|_| AfcError::Device("AMDeviceSecureStartService", -1))?;
            let mut conn: *mut c_void = std::ptr::null_mut();
            let socket = (f.AMDServiceConnectionGetSocket)(service.as_ptr());
            check("AFCConnectionOpen", (f.AFCConnectionOpen)(socket, 0, &mut conn))?;
            Ok(Self { f, conn, _service: service, _md: md, _cf: cf })
        }
    }

    /// Names in a directory (without `.` and `..`).
    pub fn list(&self, path: &str) -> Result<Vec<String>, AfcError> {
        let (path, c) = c_path(path)?;
        let mut names = Vec::new();
        // SAFETY: conn is open; dir handle is closed before returning; entries are copied out.
        unsafe {
            let mut dir: *mut c_void = std::ptr::null_mut();
            self.afc("AFCDirectoryOpen", &path, (self.f.AFCDirectoryOpen)(self.conn, c.as_ptr(), &mut dir))?;
            loop {
                let mut entry: *const c_char = std::ptr::null();
                if (self.f.AFCDirectoryRead)(self.conn, dir, &mut entry) != 0 || entry.is_null() {
                    break;
                }
                let name = CStr::from_ptr(entry).to_string_lossy().into_owned();
                if name != "." && name != ".." {
                    names.push(name);
                }
            }
            (self.f.AFCDirectoryClose)(self.conn, dir);
        }
        Ok(names)
    }

    pub fn read(&self, path: &str) -> Result<Vec<u8>, AfcError> {
        let (path, c) = c_path(path)?;
        let mut out = Vec::new();
        // SAFETY: conn is open; the file ref is closed on every path out of this block.
        unsafe {
            let mut file = 0u64;
            self.afc("AFCFileRefOpen", &path, (self.f.AFCFileRefOpen)(self.conn, c.as_ptr(), MODE_READ, &mut file))?;
            let mut buf = vec![0u8; CHUNK];
            let result = loop {
                let mut len = buf.len();
                let rc = (self.f.AFCFileRefRead)(self.conn, file, buf.as_mut_ptr(), &mut len);
                if rc != 0 {
                    break Err(AfcError::Afc { op: "AFCFileRefRead", path: path.clone(), code: rc });
                }
                if len == 0 {
                    break Ok(());
                }
                out.extend_from_slice(&buf[..len]);
            };
            (self.f.AFCFileRefClose)(self.conn, file);
            result?;
        }
        Ok(out)
    }

    /// Create or replace a file.
    pub fn write(&self, path: &str, data: &[u8]) -> Result<(), AfcError> {
        let (path, c) = c_path(path)?;
        // SAFETY: conn is open; the file ref is closed on every path out of this block.
        unsafe {
            let mut file = 0u64;
            self.afc(
                "AFCFileRefOpen",
                &path,
                (self.f.AFCFileRefOpen)(self.conn, c.as_ptr(), MODE_WRITE_CREATE_TRUNCATE, &mut file),
            )?;
            let result = data.chunks(CHUNK).try_for_each(|chunk| {
                self.afc("AFCFileRefWrite", &path, (self.f.AFCFileRefWrite)(self.conn, file, chunk.as_ptr(), chunk.len()))
            });
            (self.f.AFCFileRefClose)(self.conn, file);
            result
        }
    }

    pub fn mkdir(&self, path: &str) -> Result<(), AfcError> {
        let (path, c) = c_path(path)?;
        // SAFETY: conn is open; `c` outlives the call.
        self.afc("AFCDirectoryCreate", &path, unsafe { (self.f.AFCDirectoryCreate)(self.conn, c.as_ptr()) })
    }

    /// Remove a file or an empty directory.
    pub fn remove(&self, path: &str) -> Result<(), AfcError> {
        let (path, c) = c_path(path)?;
        // SAFETY: conn is open; `c` outlives the call.
        self.afc("AFCRemovePath", &path, unsafe { (self.f.AFCRemovePath)(self.conn, c.as_ptr()) })
    }

    pub fn rename(&self, from: &str, to: &str) -> Result<(), AfcError> {
        let (from, cf) = c_path(from)?;
        let (_, ct) = c_path(to)?;
        // SAFETY: conn is open; both C strings outlive the call.
        self.afc("AFCRenamePath", &from, unsafe { (self.f.AFCRenamePath)(self.conn, cf.as_ptr(), ct.as_ptr()) })
    }

    fn afc(&self, op: &'static str, path: &str, code: i32) -> Result<(), AfcError> {
        if code == 0 {
            Ok(())
        } else {
            Err(AfcError::Afc { op, path: path.to_string(), code })
        }
    }
}

impl Drop for Afc {
    fn drop(&mut self) {
        // SAFETY: conn was opened by AFCConnectionOpen and is closed exactly once.
        unsafe { (self.f.AFCConnectionClose)(self.conn) };
    }
}

fn c_path(path: &str) -> Result<(String, CString), AfcError> {
    let clean = device_path(path)?;
    // device_path rejects NUL, so this cannot fail.
    let c = CString::new(clean.clone()).map_err(|_| PathError::BadChar)?;
    Ok((clean, c))
}

fn check(what: &'static str, rc: i32) -> Result<(), AfcError> {
    if rc == 0 {
        Ok(())
    } else {
        Err(AfcError::Device(what, rc))
    }
}

// Machine tests: need Apple's DLLs and the trusted iPhone plugged in.
// Run with `cargo test afc -- --include-ignored --test-threads=1`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::apple::afc_path::PathError;
    use crate::apple::cf::Cf;
    use crate::apple::device::{watch, MobileDevice};
    use crate::device::registry::{Interface, RawEvent};
    use std::sync::Arc;
    use std::time::Duration;

    fn afc() -> Afc {
        let dlls = crate::apple::loader::load_from_system().expect("Apple DLLs");
        let cf = Arc::new(Cf::new(dlls.core_foundation).unwrap());
        let md = Arc::new(MobileDevice::new(dlls.mobile_device).unwrap());
        let (events, handles) = watch(cf.clone(), md.clone()).expect("subscribe");
        loop {
            match events.recv_timeout(Duration::from_secs(20)).expect("no trusted USB iPhone within 20 s") {
                RawEvent::Connected { udid, interface: Interface::Usb, trusted: true, .. } => {
                    let device = handles.get(&udid).expect("device handle");
                    return Afc::open(cf, md, &device).expect("open AFC");
                }
                _ => continue,
            }
        }
    }

    #[test]
    #[ignore]
    fn lists_the_media_root() {
        let names = afc().list("/").unwrap();
        assert!(names.iter().any(|n| n == "iTunes_Control"), "{names:?}");
        assert!(!names.iter().any(|n| n == "." || n == ".."));
    }

    #[test]
    #[ignore]
    fn finds_the_music_database() {
        let names = afc().list("/iTunes_Control/iTunes").unwrap();
        assert!(names.iter().any(|n| n == "MediaLibrary.sqlitedb"), "{names:?}");
    }

    #[test]
    #[ignore]
    fn write_read_and_remove_round_trip_in_a_scratch_folder() {
        let afc = afc();
        let dir = "/MiuMusicTest";
        let file = "/MiuMusicTest/Mùa thu.bin";
        let data: Vec<u8> = (0..300_000u32).map(|i| (i * 31 % 251) as u8).collect();

        afc.mkdir(dir).unwrap();
        afc.write(file, &data).unwrap();
        assert!(afc.list(dir).unwrap().iter().any(|n| n == "Mùa thu.bin"));
        assert_eq!(afc.read(file).unwrap(), data);

        afc.remove(file).unwrap();
        afc.remove(dir).unwrap();
        assert!(!afc.list("/").unwrap().iter().any(|n| n == "MiuMusicTest"));
    }

    #[test]
    #[ignore]
    fn paths_outside_the_media_root_are_refused_before_reaching_the_phone() {
        assert!(matches!(afc().list("/../private"), Err(AfcError::Path(PathError::ParentDir))));
    }
}
