//! Load Apple's DLLs (F1.1). The only place that calls LoadLibrary on them (CLAUDE.md rule 2).
//!
//! The Store packages keep CoreFoundation.dll apart from the AMDS64 DLLs, so every DLL folder is
//! registered with `AddDllDirectory` and each load searches those folders plus the DLL's own folder.
//! The process-wide default search order is left alone.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use libloading::os::windows::{
    Library, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_USER_DIRS,
};

use crate::apple::exports::{REQUIRED_AIR_TRAFFIC, REQUIRED_MOBILE_DEVICE};
use crate::apple::locate::Prereqs;

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("{0} not found")]
    NotFound(&'static str),
    #[error("could not load {}: {message}", path.display())]
    LoadFailed { path: PathBuf, message: String },
    #[error("could not copy Apple's DLLs: {0}")]
    Stage(String),
    #[error("{dll} is missing exports: {names:?}")]
    MissingExports { dll: &'static str, names: Vec<&'static str> },
}

/// Loaded in dependency order; dropping this unloads them.
#[derive(Debug)]
pub struct AppleDlls {
    pub air_traffic_host: Library,
    pub mobile_device: Library,
    pub core_foundation: Library,
}

/// Folders to add to the DLL search path: CoreFoundation's first, no duplicates.
pub fn search_dirs(p: &Prereqs) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for dll in [&p.core_foundation, &p.mobile_device, &p.air_traffic_host].into_iter().flatten() {
        if let Some(dir) = dll.parent() {
            if !dirs.iter().any(|d| d == dir) {
                dirs.push(dir.to_path_buf());
            }
        }
    }
    dirs
}

pub fn load_dll(path: &Path, extra_dirs: &[PathBuf]) -> Result<Library, LoadError> {
    for dir in extra_dirs {
        add_dll_directory(dir);
    }
    let flags = LOAD_LIBRARY_SEARCH_DEFAULT_DIRS | LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_USER_DIRS;
    // SAFETY: loading runs the DLL's initialisers; these are Apple's signed DLLs or system DLLs.
    unsafe { Library::load_with_flags(path, flags) }
        .map_err(|e| LoadError::LoadFailed { path: path.to_path_buf(), message: e.to_string() })
}

pub fn missing_symbols<'a>(lib: &Library, names: &[&'a str]) -> Vec<&'a str> {
    names
        .iter()
        .copied()
        // SAFETY: only looks the symbol up; it is never called through this pointer type.
        .filter(|n| unsafe { lib.get::<unsafe extern "system" fn()>(n.as_bytes()) }.is_err())
        .collect()
}

pub fn load_apple(p: &Prereqs) -> Result<AppleDlls, LoadError> {
    let dirs = search_dirs(p);
    let core_foundation = load_dll(p.core_foundation.as_deref().ok_or(LoadError::NotFound("CoreFoundation.dll"))?, &dirs)?;
    let mobile_device = load_required(p.mobile_device.as_deref(), "MobileDevice.dll", REQUIRED_MOBILE_DEVICE, &dirs)?;
    let air_traffic_host = load_required(p.air_traffic_host.as_deref(), "AirTrafficHost.dll", REQUIRED_AIR_TRAFFIC, &dirs)?;
    Ok(AppleDlls { air_traffic_host, mobile_device, core_foundation })
}

/// Find the DLLs on this PC, make a private copy if they're in a Store package, and load them.
pub fn load_from_system() -> Result<AppleDlls, LoadError> {
    let found = crate::apple::locate::locate_on_system();
    let cache = crate::util::paths::apple_dlls_dir_in(&crate::util::paths::local_appdata_root());
    let staged = crate::apple::stage::stage(&found, &cache).map_err(|e| LoadError::Stage(e.to_string()))?;
    load_apple(&staged)
}

fn load_required(path: Option<&Path>, dll: &'static str, required: &[&'static str], dirs: &[PathBuf]) -> Result<Library, LoadError> {
    let lib = load_dll(path.ok_or(LoadError::NotFound(dll))?, dirs)?;
    let names = missing_symbols(&lib, required);
    if names.is_empty() {
        Ok(lib)
    } else {
        Err(LoadError::MissingExports { dll, names })
    }
}

fn add_dll_directory(dir: &Path) {
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call. Adding the same
    // folder twice is harmless; the cookie is kept for the life of the process.
    unsafe { windows_sys::Win32::System::LibraryLoader::AddDllDirectory(wide.as_ptr()) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apple::locate::Prereqs;
    use std::path::{Path, PathBuf};

    fn system32(name: &str) -> PathBuf {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        Path::new(&root).join("System32").join(name)
    }

    #[test]
    fn search_dirs_put_core_foundation_first_and_drop_duplicates() {
        let pkg = PathBuf::from(r"C:\Program Files\WindowsApps\AppleInc.iTunes_1_x64__n");
        let p = Prereqs {
            air_traffic_host: Some(pkg.join(r"AMDS64\AirTrafficHost.dll")),
            mobile_device: Some(pkg.join(r"AMDS64\MobileDevice.dll")),
            core_foundation: Some(pkg.join("CoreFoundation.dll")),
        };
        assert_eq!(search_dirs(&p), vec![pkg.clone(), pkg.join("AMDS64")]);
    }

    #[test]
    fn loads_a_dll_by_full_path() {
        assert!(load_dll(&system32("kernel32.dll"), &[]).is_ok());
    }

    #[test]
    fn load_failure_names_the_path() {
        let missing = PathBuf::from(r"C:\nowhere\NoSuch.dll");
        match load_dll(&missing, &[]) {
            Err(LoadError::LoadFailed { path, .. }) => assert_eq!(path, missing),
            other => panic!("expected LoadFailed, got {other:?}"),
        }
    }

    #[test]
    fn reports_missing_symbols() {
        let lib = load_dll(&system32("kernel32.dll"), &[]).unwrap();
        assert_eq!(missing_symbols(&lib, &["CreateFileW", "NoSuchFunction"]), vec!["NoSuchFunction"]);
    }

    #[test]
    fn nothing_found_fails_on_core_foundation_first() {
        match load_apple(&Prereqs::default()) {
            Err(LoadError::NotFound(name)) => assert_eq!(name, "CoreFoundation.dll"),
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn a_dll_without_the_required_exports_is_rejected() {
        let k = system32("kernel32.dll");
        let p = Prereqs { air_traffic_host: Some(k.clone()), mobile_device: Some(k.clone()), core_foundation: Some(k) };
        match load_apple(&p) {
            Err(LoadError::MissingExports { dll, names }) => {
                assert_eq!(dll, "MobileDevice.dll");
                assert!(names.contains(&"AMDeviceNotificationSubscribe"));
            }
            other => panic!("expected MissingExports, got {other:?}"),
        }
    }

    /// Run with `cargo test -- --ignored` on a PC with iTunes / Apple Devices installed.
    #[test]
    #[ignore]
    fn loads_the_real_apple_dlls_on_this_pc() {
        let dlls = load_from_system().expect("load Apple DLLs");
        assert!(missing_symbols(&dlls.air_traffic_host, crate::apple::exports::REQUIRED_AIR_TRAFFIC).is_empty());
        assert!(missing_symbols(&dlls.mobile_device, crate::apple::exports::REQUIRED_MOBILE_DEVICE).is_empty());
    }
}
