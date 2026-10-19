//! Read a DLL's export table without loading it (F2.1). Used by the `dll-exports` tool and,
//! from F1.1, to check a DLL has the functions we need before we load it.

use std::path::Path;

use object::Object;

pub fn export_names(path: &Path) -> Result<Vec<String>, String> {
    let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file = object::File::parse(&*data).map_err(|e| format!("{}: {e}", path.display()))?;
    let exports = file.exports().map_err(|e| format!("{}: {e}", path.display()))?;
    let mut names: Vec<String> = exports
        .iter()
        .map(|e| String::from_utf8_lossy(e.name()).into_owned())
        .filter(|n| !n.is_empty())
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// For each prefix (in order), the exports starting with it.
pub fn group_by_prefix<'a>(names: &'a [String], prefixes: &[&'static str]) -> Vec<(&'static str, Vec<&'a str>)> {
    prefixes
        .iter()
        .map(|p| (*p, names.iter().map(String::as_str).filter(|n| n.starts_with(p)).collect()))
        .collect()
}

pub fn missing_exports<'a>(names: &[String], required: &[&'a str]) -> Vec<&'a str> {
    required.iter().copied().filter(|r| names.binary_search_by(|n| n.as_str().cmp(r)).is_err()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn system_dll(name: &str) -> std::path::PathBuf {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
        Path::new(&root).join("System32").join(name)
    }

    #[test]
    fn lists_named_exports_of_a_system_dll() {
        let names = export_names(&system_dll("kernel32.dll")).unwrap();
        assert!(names.iter().any(|n| n == "CreateFileW"));
        assert!(names.len() > 500);
    }

    #[test]
    fn export_names_are_sorted_and_unique() {
        let names = export_names(&system_dll("kernel32.dll")).unwrap();
        assert!(names.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn non_pe_file_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("fake.dll");
        std::fs::write(&p, b"definitely not a dll").unwrap();
        assert!(export_names(&p).is_err());
    }

    #[test]
    fn groups_exports_by_api_prefix() {
        let names: Vec<String> = ["AFCConnectionOpen", "AMDeviceConnect", "ATHostConnectionCreate",
            "ATHostConnectionRelease", "AMDeviceNotificationSubscribe", "Unrelated"]
            .iter().map(|s| s.to_string()).collect();
        let groups = group_by_prefix(&names, &["ATHostConnection", "AMDevice", "AFC"]);
        assert_eq!(groups[0], ("ATHostConnection", vec!["ATHostConnectionCreate", "ATHostConnectionRelease"]));
        assert_eq!(groups[1], ("AMDevice", vec!["AMDeviceConnect", "AMDeviceNotificationSubscribe"]));
        assert_eq!(groups[2], ("AFC", vec!["AFCConnectionOpen"]));
    }

    #[test]
    fn reports_which_required_exports_are_missing() {
        let names: Vec<String> = vec!["A".into(), "C".into()];
        assert_eq!(missing_exports(&names, &["A", "B", "C", "D"]), vec!["B", "D"]);
    }
}

/// Exports the sync path depends on (confirmed present in AirTrafficHost 4026.110.1.6, 2026-10-20).
pub const REQUIRED_AIR_TRAFFIC: &[&str] = &[
    "ATHostConnectionCreateWithLibrary",
    "ATHostConnectionGetGrappaSessionId",
    "ATHostConnectionInvalidate",
    "ATHostConnectionReadMessage",
    "ATHostConnectionRelease",
    "ATHostConnectionSendAssetCompleted",
    "ATHostConnectionSendFileBegin",
    "ATHostConnectionSendHostInfo",
    "ATHostConnectionSendSyncRequest",
];

/// Device discovery, sessions and AFC (confirmed present in MobileDevice 1860.2.5.1, 2026-10-20).
pub const REQUIRED_MOBILE_DEVICE: &[&str] = &[
    "AFCConnectionOpen",
    "AFCDirectoryOpen",
    "AFCDirectoryRead",
    "AFCFileRefClose",
    "AFCFileRefOpen",
    "AFCFileRefRead",
    "AFCFileRefWrite",
    "AMDeviceConnect",
    "AMDeviceCopyDeviceIdentifier",
    "AMDeviceCopyValue",
    "AMDeviceDisconnect",
    "AMDeviceIsPaired",
    "AMDeviceNotificationSubscribe",
    "AMDeviceSecureStartService",
    "AMDeviceStartSession",
    "AMDeviceStopSession",
    "AMDeviceValidatePairing",
];

#[cfg(all(test, windows))]
mod machine_tests {
    use super::*;
    use crate::apple::locate::locate_on_system;

    /// Run with `cargo test -- --ignored` on a PC with iTunes / Apple Devices installed.
    #[test]
    #[ignore]
    fn apple_dlls_on_this_pc_have_the_exports_we_need() {
        let found = locate_on_system();
        let ath = export_names(found.air_traffic_host.as_deref().expect("AirTrafficHost.dll")).unwrap();
        let md = export_names(found.mobile_device.as_deref().expect("MobileDevice.dll")).unwrap();
        assert_eq!(missing_exports(&ath, REQUIRED_AIR_TRAFFIC), Vec::<&str>::new());
        assert_eq!(missing_exports(&md, REQUIRED_MOBILE_DEVICE), Vec::<&str>::new());
    }
}
