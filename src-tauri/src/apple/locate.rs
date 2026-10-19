//! Find Apple's DLLs on disk (F0.3). Pure filesystem work, no FFI: loading happens in F1.1.

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Serialize;

const AIR_TRAFFIC_HOST: &[&str] = &["AirTrafficHost.dll"];
const MOBILE_DEVICE: &[&str] = &["MobileDevice.dll", "iTunesMobileDevice.dll"];
const CORE_FOUNDATION: &[&str] = &["CoreFoundation.dll"];
const MAX_DEPTH: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86,
    X64,
    Arm64,
    Other(u16),
}

impl Arch {
    pub fn current() -> Arch {
        if cfg!(target_arch = "x86_64") {
            Arch::X64
        } else if cfg!(target_arch = "aarch64") {
            Arch::Arm64
        } else {
            Arch::X86
        }
    }
}

/// Machine type from a PE file header, or `None` if the file isn't a PE image.
pub fn pe_arch(path: &Path) -> io::Result<Option<Arch>> {
    let mut f = File::open(path)?;
    let mut mz = [0u8; 0x40];
    if f.read_exact(&mut mz).is_err() || &mz[0..2] != b"MZ" {
        return Ok(None);
    }
    let pe_offset = u32::from_le_bytes(mz[0x3c..0x40].try_into().unwrap()) as u64;
    let mut hdr = [0u8; 6];
    f.seek(SeekFrom::Start(pe_offset))?;
    if f.read_exact(&mut hdr).is_err() || &hdr[0..4] != b"PE\0\0" {
        return Ok(None);
    }
    Ok(Some(match u16::from_le_bytes([hdr[4], hdr[5]]) {
        0x014c => Arch::X86,
        0x8664 => Arch::X64,
        0xaa64 => Arch::Arm64,
        other => Arch::Other(other),
    }))
}

#[derive(Debug, Default, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prereqs {
    pub air_traffic_host: Option<PathBuf>,
    pub mobile_device: Option<PathBuf>,
    pub core_foundation: Option<PathBuf>,
}

impl Prereqs {
    pub fn is_ready(&self) -> bool {
        self.missing().is_empty()
    }

    pub fn missing(&self) -> Vec<&'static str> {
        [
            (&self.air_traffic_host, AIR_TRAFFIC_HOST[0]),
            (&self.mobile_device, MOBILE_DEVICE[0]),
            (&self.core_foundation, CORE_FOUNDATION[0]),
        ]
        .into_iter()
        .filter(|(found, _)| found.is_none())
        .map(|(_, name)| name)
        .collect()
    }
}

/// What `check_prereqs` sends to the UI.
#[derive(Debug, Serialize)]
pub struct PrereqReport {
    pub ready: bool,
    pub missing: Vec<&'static str>,
    pub paths: Prereqs,
}

impl From<Prereqs> for PrereqReport {
    fn from(paths: Prereqs) -> Self {
        Self { ready: paths.is_ready(), missing: paths.missing(), paths }
    }
}

/// Search `roots` in order; within a root the shallowest match of the right architecture wins.
pub fn locate_in(roots: &[PathBuf], arch: Arch) -> Prereqs {
    let mut p = Prereqs::default();
    for root in roots {
        if p.air_traffic_host.is_none() {
            p.air_traffic_host = find(root, AIR_TRAFFIC_HOST, arch);
        }
        if p.mobile_device.is_none() {
            p.mobile_device = find(root, MOBILE_DEVICE, arch);
        }
        if p.core_foundation.is_none() {
            p.core_foundation = find(root, CORE_FOUNDATION, arch);
        }
    }
    p
}

fn find(root: &Path, names: &[&str], arch: Arch) -> Option<PathBuf> {
    let mut queue = VecDeque::from([(root.to_path_buf(), 0usize)]);
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut subdirs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                subdirs.push(path);
            } else if is_named(&path, names) && pe_arch(&path).ok().flatten() == Some(arch) {
                return Some(path);
            }
        }
        if depth < MAX_DEPTH {
            subdirs.sort();
            queue.extend(subdirs.into_iter().map(|d| (d, depth + 1)));
        }
    }
    None
}

fn is_named(path: &Path, names: &[&str]) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| names.iter().any(|want| n.eq_ignore_ascii_case(want)))
}

/// Search order: registry install dirs, classic Common Files install, Microsoft Store packages.
pub fn candidate_roots(registry_dirs: Vec<PathBuf>, common_files: &Path, packages: Vec<PathBuf>) -> Vec<PathBuf> {
    let apple = common_files.join("Apple");
    let mut roots = registry_dirs;
    roots.push(apple.join("Mobile Device Support"));
    roots.push(apple.join("Apple Application Support"));
    roots.extend(packages);
    roots
}

/// Keep Apple's Store packages (`AppleInc.*`), Apple Devices before iTunes before anything else.
pub fn apple_package_roots(packages: Vec<(String, PathBuf)>) -> Vec<PathBuf> {
    let rank = |name: &str| {
        if name.starts_with("AppleInc.AppleDevices_") {
            0
        } else if name.starts_with("AppleInc.iTunes_") {
            1
        } else {
            2
        }
    };
    let mut apple: Vec<_> = packages.into_iter().filter(|(n, _)| n.starts_with("AppleInc.")).collect();
    apple.sort_by_key(|(n, _)| rank(n));
    apple.into_iter().map(|(_, root)| root).collect()
}

/// Real lookup on this machine.
#[cfg(windows)]
pub fn locate_on_system() -> Prereqs {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let registry_dirs = ["Apple Mobile Device Support", "Apple Application Support"]
        .iter()
        .filter_map(|k| hklm.open_subkey(format!(r"SOFTWARE\Apple Inc.\{k}")).ok())
        .filter_map(|k| k.get_value::<String, _>("InstallDir").ok())
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .collect();

    let common_files = std::env::var_os("CommonProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files\Common Files"));

    let repo = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";
    let packages = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(repo)
        .map(|k| {
            k.enum_keys()
                .flatten()
                .filter_map(|name| {
                    let root: String = k.open_subkey(&name).ok()?.get_value("PackageRootFolder").ok()?;
                    Some((name, PathBuf::from(root)))
                })
                .collect()
        })
        .unwrap_or_default();

    let roots = candidate_roots(registry_dirs, &common_files, apple_package_roots(packages));
    locate_in(&roots, Arch::current())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    const X86: u16 = 0x014c;
    const X64: u16 = 0x8664;

    fn fake_pe(path: &Path, machine: u16) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut b = vec![0u8; 0x100];
        b[0..2].copy_from_slice(b"MZ");
        b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        b[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        fs::write(path, b).unwrap();
    }

    /// Layout of the Microsoft Store iTunes package seen on the dev PC (2026-10-19).
    fn store_itunes(root: &Path) {
        fake_pe(&root.join(r"AMDS32\AirTrafficHost.dll"), X86);
        fake_pe(&root.join(r"AMDS32\MobileDevice.dll"), X86);
        fake_pe(&root.join(r"AMDS64\AirTrafficHost.dll"), X64);
        fake_pe(&root.join(r"AMDS64\MobileDevice.dll"), X64);
        fake_pe(&root.join(r"VFS\ProgramFilesCommonX86\Apple\Apple Application Support\CoreFoundation.dll"), X86);
        fake_pe(&root.join("CoreFoundation.dll"), X64);
    }

    #[test]
    fn reads_x64_and_x86_machine_types() {
        let d = tempfile::tempdir().unwrap();
        fake_pe(&d.path().join("a.dll"), X64);
        fake_pe(&d.path().join("b.dll"), X86);
        assert_eq!(pe_arch(&d.path().join("a.dll")).unwrap(), Some(Arch::X64));
        assert_eq!(pe_arch(&d.path().join("b.dll")).unwrap(), Some(Arch::X86));
    }

    #[test]
    fn non_pe_file_has_no_arch() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("x.dll"), b"not a dll").unwrap();
        assert_eq!(pe_arch(&d.path().join("x.dll")).unwrap(), None);
    }

    #[test]
    fn finds_64_bit_dlls_in_store_itunes_layout() {
        let d = tempfile::tempdir().unwrap();
        store_itunes(d.path());
        let p = locate_in(&[d.path().to_path_buf()], Arch::X64);
        assert_eq!(p.air_traffic_host, Some(d.path().join(r"AMDS64\AirTrafficHost.dll")));
        assert_eq!(p.mobile_device, Some(d.path().join(r"AMDS64\MobileDevice.dll")));
        assert_eq!(p.core_foundation, Some(d.path().join("CoreFoundation.dll")));
        assert!(p.is_ready());
    }

    #[test]
    fn finds_classic_install_with_itunes_mobile_device_name() {
        let d = tempfile::tempdir().unwrap();
        let amds = d.path().join(r"Apple\Mobile Device Support");
        let aas = d.path().join(r"Apple\Apple Application Support");
        fake_pe(&amds.join("iTunesMobileDevice.dll"), X64);
        fake_pe(&amds.join("AirTrafficHost.dll"), X64);
        fake_pe(&aas.join("CoreFoundation.dll"), X64);
        let p = locate_in(&[amds.clone(), aas.clone()], Arch::X64);
        assert_eq!(p.mobile_device, Some(amds.join("iTunesMobileDevice.dll")));
        assert_eq!(p.core_foundation, Some(aas.join("CoreFoundation.dll")));
        assert!(p.is_ready());
    }

    #[test]
    fn earlier_root_wins() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        store_itunes(a.path());
        store_itunes(b.path());
        let p = locate_in(&[b.path().to_path_buf(), a.path().to_path_buf()], Arch::X64);
        assert_eq!(p.core_foundation, Some(b.path().join("CoreFoundation.dll")));
    }

    #[test]
    fn reports_missing_dlls_by_name() {
        let d = tempfile::tempdir().unwrap();
        fake_pe(&d.path().join("CoreFoundation.dll"), X64);
        fake_pe(&d.path().join("AirTrafficHost.dll"), X86); // wrong architecture doesn't count
        let p = locate_in(&[d.path().to_path_buf(), d.path().join("does-not-exist")], Arch::X64);
        assert!(!p.is_ready());
        assert_eq!(p.missing(), vec!["AirTrafficHost.dll", "MobileDevice.dll"]);
    }

    #[test]
    fn candidate_roots_put_registry_first_then_common_files_then_packages() {
        let roots = candidate_roots(
            vec![PathBuf::from(r"D:\AMDS")],
            Path::new(r"C:\Program Files\Common Files"),
            vec![PathBuf::from(r"C:\Program Files\WindowsApps\AppleInc.iTunes_1_x64__n")],
        );
        assert_eq!(roots, vec![
            PathBuf::from(r"D:\AMDS"),
            PathBuf::from(r"C:\Program Files\Common Files\Apple\Mobile Device Support"),
            PathBuf::from(r"C:\Program Files\Common Files\Apple\Apple Application Support"),
            PathBuf::from(r"C:\Program Files\WindowsApps\AppleInc.iTunes_1_x64__n"),
        ]);
    }

    #[test]
    fn keeps_only_apple_store_packages_preferring_apple_devices() {
        let pkgs = vec![
            ("Microsoft.Paint_11.0_x64__8w".to_string(), PathBuf::from(r"C:\p\paint")),
            ("AppleInc.iTunes_12139.0_x64__nzyj".to_string(), PathBuf::from(r"C:\p\itunes")),
            ("AppleInc.AppleDevices_1.0_x64__nzyj".to_string(), PathBuf::from(r"C:\p\devices")),
        ];
        assert_eq!(apple_package_roots(pkgs), vec![PathBuf::from(r"C:\p\devices"), PathBuf::from(r"C:\p\itunes")]);
    }
}

#[cfg(test)]
mod report_tests {
    use super::*;

    #[test]
    fn report_for_nothing_found_is_not_ready_and_lists_all_three() {
        let r = PrereqReport::from(Prereqs::default());
        assert!(!r.ready);
        assert_eq!(r.missing, vec!["AirTrafficHost.dll", "MobileDevice.dll", "CoreFoundation.dll"]);
    }

    #[test]
    fn report_serializes_with_camel_case_keys_for_the_ui() {
        let r = PrereqReport::from(Prereqs { core_foundation: Some(PathBuf::from(r"C:\x\CoreFoundation.dll")), ..Default::default() });
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["ready"], false);
        assert_eq!(json["paths"]["coreFoundation"], r"C:\x\CoreFoundation.dll");
        assert!(json["paths"]["airTrafficHost"].is_null());
    }

    /// Machine check: run with `cargo test -- --ignored` on a PC with iTunes / Apple Devices installed.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn finds_apple_dlls_on_this_pc() {
        let p = locate_on_system();
        println!("{p:#?}");
        assert!(p.is_ready(), "missing: {:?}", p.missing());
    }
}
