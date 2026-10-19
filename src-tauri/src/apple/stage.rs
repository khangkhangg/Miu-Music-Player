//! Private copy of Apple's DLLs (F1.1). Windows won't load DLLs from a Store package
//! (`WindowsApps`) into our process, so we copy the ones we need — plus everything they import
//! from the package — into `%LOCALAPPDATA%\MiuMusic\apple\<package>\` and load that copy.
//! Classic installs (Common Files\Apple) are loaded in place. The DLLs are never shipped with Miu.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use object::Object;

use crate::apple::locate::Prereqs;

/// The package folder name when `path` lives under `…\WindowsApps\<package>\`.
pub fn package_name(path: &Path) -> Option<String> {
    let mut parts = path.components();
    while let Some(c) = parts.next() {
        if let Component::Normal(name) = c {
            if name.eq_ignore_ascii_case("WindowsApps") {
                return match parts.next() {
                    Some(Component::Normal(pkg)) => Some(pkg.to_string_lossy().into_owned()),
                    _ => None,
                };
            }
        }
    }
    None
}

/// Names of the DLLs a PE file imports.
pub fn import_names(path: &Path) -> Result<Vec<String>, String> {
    let data = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file = object::File::parse(&*data).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut names: Vec<String> = file
        .imports()
        .map_err(|e| format!("{}: {e}", path.display()))?
        .iter()
        .map(|i| String::from_utf8_lossy(i.library()).into_owned())
        .collect();
    names.sort_by_key(|n| n.to_ascii_lowercase());
    names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    Ok(names)
}

/// `start` plus every DLL reachable through imports that `find` can locate (i.e. inside the package).
pub fn dependency_closure(
    start: &[PathBuf],
    find: impl Fn(&str) -> Option<PathBuf>,
    imports: impl Fn(&Path) -> Vec<String>,
) -> Vec<PathBuf> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    let mut queue: Vec<PathBuf> = start.to_vec();
    while let Some(path) = queue.pop() {
        let key = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if !seen.insert(key) {
            continue;
        }
        for dep in imports(&path) {
            if let Some(found) = find(&dep) {
                queue.push(found);
            }
        }
        out.push(path);
    }
    out
}

/// Make the DLLs loadable: copy a Store package's DLLs into `dest_root\<package>\`, or return
/// classic-install paths unchanged.
pub fn stage(p: &Prereqs, dest_root: &Path) -> io::Result<Prereqs> {
    stage_with(p, dest_root, |f| import_names(f).unwrap_or_default())
}

pub fn stage_with(p: &Prereqs, dest_root: &Path, imports: impl Fn(&Path) -> Vec<String>) -> io::Result<Prereqs> {
    let dlls: Vec<&PathBuf> = [&p.mobile_device, &p.air_traffic_host, &p.core_foundation].into_iter().flatten().collect();
    let Some(pkg) = dlls.iter().find_map(|d| package_name(d)) else {
        return Ok(p.clone());
    };

    // Look for imports next to MobileDevice/AirTrafficHost first, then next to CoreFoundation.
    let mut dirs: Vec<PathBuf> = Vec::new();
    for d in &dlls {
        if let Some(dir) = d.parent() {
            if !dirs.iter().any(|x| x == dir) {
                dirs.push(dir.to_path_buf());
            }
        }
    }
    let find = |name: &str| dirs.iter().map(|dir| dir.join(name)).find(|f| f.is_file());
    let files = dependency_closure(&dlls.iter().map(|d| (*d).clone()).collect::<Vec<_>>(), find, imports);

    let dest = dest_root.join(&pkg);
    fs::create_dir_all(&dest)?;
    for src in &files {
        copy_if_changed(src, &dest.join(src.file_name().expect("file name")))?;
    }

    let moved = |o: &Option<PathBuf>| o.as_ref().and_then(|f| f.file_name()).map(|n| dest.join(n));
    Ok(Prereqs {
        air_traffic_host: moved(&p.air_traffic_host),
        mobile_device: moved(&p.mobile_device),
        core_foundation: moved(&p.core_foundation),
    })
}

/// Copy via a temp file + rename so a crash never leaves a half-written DLL behind.
fn copy_if_changed(src: &Path, dst: &Path) -> io::Result<()> {
    if let (Ok(a), Ok(b)) = (fs::metadata(src), fs::metadata(dst)) {
        if a.len() == b.len() {
            return Ok(());
        }
    }
    let tmp = dst.with_extension("partial");
    fs::copy(src, &tmp)?;
    if dst.exists() {
        fs::remove_file(dst)?;
    }
    fs::rename(&tmp, dst)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    const PKG: &str = "AppleInc.iTunes_12139.0_x64__nzyj";

    /// Fake Store package: AMDS64\{MobileDevice,AirTrafficHost}.dll + root CoreFoundation and deps.
    fn fake_package(base: &Path) -> (PathBuf, Prereqs) {
        let root = base.join("WindowsApps").join(PKG);
        for f in [r"AMDS64\MobileDevice.dll", r"AMDS64\AirTrafficHost.dll", "CoreFoundation.dll", "objc.dll",
                  "CFNetwork.dll", "zlib1.dll", "Unused.dll"] {
            let p = root.join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, f.as_bytes()).unwrap();
        }
        let p = Prereqs {
            air_traffic_host: Some(root.join(r"AMDS64\AirTrafficHost.dll")),
            mobile_device: Some(root.join(r"AMDS64\MobileDevice.dll")),
            core_foundation: Some(root.join("CoreFoundation.dll")),
        };
        (root, p)
    }

    fn fake_imports(path: &Path) -> Vec<String> {
        let map: HashMap<&str, Vec<&str>> = HashMap::from([
            ("MobileDevice.dll", vec!["CoreFoundation.dll", "CFNetwork.dll", "KERNEL32.dll"]),
            ("AirTrafficHost.dll", vec!["MobileDevice.dll", "CoreFoundation.dll"]),
            ("CoreFoundation.dll", vec!["objc.dll", "KERNEL32.dll"]),
            ("CFNetwork.dll", vec!["zlib1.dll", "CoreFoundation.dll"]),
        ]);
        let name = path.file_name().unwrap().to_str().unwrap();
        map.get(name).map(|v| v.iter().map(|s| s.to_string()).collect()).unwrap_or_default()
    }

    #[test]
    fn package_name_is_the_folder_under_windowsapps() {
        let p = PathBuf::from(r"C:\Program Files\WindowsApps\AppleInc.iTunes_1_x64__n\AMDS64\MobileDevice.dll");
        assert_eq!(package_name(&p).as_deref(), Some("AppleInc.iTunes_1_x64__n"));
        assert_eq!(package_name(Path::new(r"C:\Program Files\Common Files\Apple\Mobile Device Support\MobileDevice.dll")), None);
    }

    #[test]
    fn closure_follows_imports_found_in_the_package_and_skips_system_dlls() {
        let d = tempfile::tempdir().unwrap();
        let (root, p) = fake_package(d.path());
        let dirs = [root.join("AMDS64"), root.clone()];
        let find = |name: &str| dirs.iter().map(|dir| dir.join(name)).find(|f| f.exists());
        let mut names: Vec<String> = dependency_closure(&[p.mobile_device.unwrap(), p.air_traffic_host.unwrap()], find, fake_imports)
            .iter().map(|f| f.file_name().unwrap().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["AirTrafficHost.dll", "CFNetwork.dll", "CoreFoundation.dll", "MobileDevice.dll", "objc.dll", "zlib1.dll"]);
    }

    #[test]
    fn stages_the_closure_into_a_folder_named_after_the_package() {
        let d = tempfile::tempdir().unwrap();
        let (_, p) = fake_package(d.path());
        let dest = d.path().join("MiuMusic").join("apple");
        let staged = stage_with(&p, &dest, fake_imports).unwrap();
        let dir = dest.join(PKG);
        assert_eq!(staged.mobile_device, Some(dir.join("MobileDevice.dll")));
        assert_eq!(staged.air_traffic_host, Some(dir.join("AirTrafficHost.dll")));
        assert_eq!(staged.core_foundation, Some(dir.join("CoreFoundation.dll")));
        for f in ["CFNetwork.dll", "objc.dll", "zlib1.dll"] {
            assert!(dir.join(f).exists(), "{f} not copied");
        }
        assert!(!dir.join("Unused.dll").exists());
        assert_eq!(fs::read(dir.join("MobileDevice.dll")).unwrap(), br"AMDS64\MobileDevice.dll");
    }

    #[test]
    fn staging_again_does_not_recopy_unchanged_files() {
        let d = tempfile::tempdir().unwrap();
        let (_, p) = fake_package(d.path());
        let dest = d.path().join("apple");
        stage_with(&p, &dest, fake_imports).unwrap();
        let copied = dest.join(PKG).join("CoreFoundation.dll");
        let before = fs::metadata(&copied).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        stage_with(&p, &dest, fake_imports).unwrap();
        assert_eq!(fs::metadata(&copied).unwrap().modified().unwrap(), before);
    }

    #[test]
    fn classic_installs_are_used_in_place() {
        let d = tempfile::tempdir().unwrap();
        let amds = d.path().join(r"Common Files\Apple\Mobile Device Support");
        let p = Prereqs {
            air_traffic_host: Some(amds.join("AirTrafficHost.dll")),
            mobile_device: Some(amds.join("iTunesMobileDevice.dll")),
            core_foundation: Some(d.path().join(r"Common Files\Apple\Apple Application Support\CoreFoundation.dll")),
        };
        let dest = d.path().join("apple");
        assert_eq!(stage_with(&p, &dest, fake_imports).unwrap(), p);
        assert!(!dest.exists());
    }

    #[cfg(windows)]
    #[test]
    fn reads_import_names_of_a_system_dll() {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        let names = import_names(&Path::new(&root).join(r"System32\kernel32.dll")).unwrap();
        assert!(names.iter().any(|n| n.eq_ignore_ascii_case("ntdll.dll")), "{names:?}");
    }
}
