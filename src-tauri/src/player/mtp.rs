//! Read-only probe of MTP devices (Android phones, Sony Walkman, newer SanDisk players) through the
//! Windows Portable Devices (WPD) COM API: lists each device and its storages with their space.
//! Nothing is written. See `docs/MTP.md` for what syncing over MTP would take.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::player::drives::{Player, PlayerKind};
use crate::player::profile::Profile;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MtpStorage {
    pub object_id: String,
    /// Stays the same across connections (WPD_OBJECT_PERSISTENT_UNIQUE_ID); may be empty.
    #[serde(default)]
    pub persistent_id: String,
    pub name: String,
    pub description: String,
    pub capacity_bytes: Option<u64>,
    pub free_bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MtpDevice {
    /// The PnP device path WPD opens it by.
    pub id: String,
    pub name: String,
    pub manufacturer: String,
    /// WPD_DEVICE_SERIAL_NUMBER; may be empty.
    #[serde(default)]
    pub serial: String,
    /// "MTP: 1.00", "PTP: 1.00"…
    pub protocol: String,
    pub storages: Vec<MtpStorage>,
}

/// What a WPD device is, for Miu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Speaks MTP: a phone or player Miu could copy music to.
    Player,
    /// An iPhone shows up as a PTP camera (photos only); Miu reaches it through Apple's DLLs instead.
    Iphone,
    /// Cameras and anything else.
    Other,
}

pub fn kind(protocol: &str, manufacturer: &str) -> Kind {
    if manufacturer.trim().eq_ignore_ascii_case("apple inc.") || manufacturer.trim().eq_ignore_ascii_case("apple") {
        Kind::Iphone
    } else if protocol.trim_start().to_ascii_uppercase().starts_with("MTP") {
        Kind::Player
    } else {
        Kind::Other
    }
}

/// The players among WPD devices: one per storage of each MTP device (a phone with an SD card is two).
/// Known by device serial and storage id, which survive replugging (the PnP path does not).
pub fn players(devices: &[MtpDevice], known: &BTreeMap<String, Profile>) -> Vec<Player> {
    let mut out = Vec::new();
    for d in devices.iter().filter(|d| kind(&d.protocol, &d.manufacturer) == Kind::Player) {
        let device_key = if d.serial.trim().is_empty() { d.name.trim() } else { d.serial.trim() };
        for s in &d.storages {
            let storage_key = if s.persistent_id.trim().is_empty() { s.name.trim() } else { s.persistent_id.trim() };
            let id = format!("mtp:{device_key}:{storage_key}");
            let (configured, profile) = match known.get(&id) {
                Some(p) => (true, p.clone()),
                None => (false, Profile::default()),
            };
            out.push(Player {
                id,
                kind: PlayerKind::Mtp,
                wpd_device: Some(d.id.clone()),
                wpd_storage: Some(s.object_id.clone()),
                root: std::path::PathBuf::new(),
                label: format!("{} — {}", d.name.trim(), s.name.trim()),
                total_bytes: s.capacity_bytes.unwrap_or(0),
                free_bytes: s.free_bytes.unwrap_or(0),
                configured,
                profile,
            });
        }
    }
    out
}

/// The WPD object format for a file Miu writes, by extension.
pub fn format_for(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "mp3" => "mp3",
        "m4a" | "mp4" => "m4a",
        "aac" => "aac",
        "flac" => "flac",
        "wav" => "wav",
        _ => "unspecified",
    }
}

#[cfg(windows)]
pub use imp::{device_ids, probe};

#[cfg(windows)]
mod imp {
    use windows::core::{HSTRING, PWSTR};
    use windows::Win32::Devices::PortableDevices::*;
    use windows::Win32::Foundation::PROPERTYKEY;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};

    use super::{MtpDevice, MtpStorage};

    fn take(p: PWSTR) -> String {
        if p.is_null() {
            return String::new();
        }
        // SAFETY: WPD hands back a NUL-terminated string allocated with CoTaskMemAlloc.
        let s = unsafe { p.to_string() }.unwrap_or_default();
        unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
        s
    }

    /// A manager string property (friendly name, manufacturer) with WPD's two-call pattern.
    fn manager_text(get: impl Fn(PWSTR, *mut u32) -> windows::core::Result<()>) -> String {
        let mut len = 0u32;
        if get(PWSTR::null(), &mut len).is_err() || len == 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize];
        if get(PWSTR(buf.as_mut_ptr()), &mut len).is_err() {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..buf.iter().position(|c| *c == 0).unwrap_or(buf.len())])
    }

    fn values_text(v: &IPortableDeviceValues, key: &PROPERTYKEY) -> String {
        // SAFETY: valid interface and key.
        unsafe { v.GetStringValue(key) }.map(take).unwrap_or_default()
    }

    fn values_u64(v: &IPortableDeviceValues, key: &PROPERTYKEY) -> Option<u64> {
        // SAFETY: valid interface and key.
        unsafe { v.GetUnsignedLargeIntegerValue(key) }.ok()
    }

    fn properties(props: &IPortableDeviceProperties, object: &HSTRING, keys: &[PROPERTYKEY]) -> windows::core::Result<IPortableDeviceValues> {
        // SAFETY: plain COM calls on live interfaces.
        unsafe {
            let coll: IPortableDeviceKeyCollection = CoCreateInstance(&PortableDeviceKeyCollection, None, CLSCTX_INPROC_SERVER)?;
            for k in keys {
                coll.Add(k)?;
            }
            props.GetValues(object, &coll)
        }
    }

    fn storages(device: &IPortableDevice) -> windows::core::Result<(String, String, Vec<MtpStorage>)> {
        // SAFETY: plain COM calls on live interfaces; returned strings are freed by `take`.
        unsafe {
            let content = device.Content()?;
            let props = content.Properties()?;
            let root = HSTRING::from("DEVICE");
let about = properties(&props, &root, &[WPD_DEVICE_PROTOCOL, WPD_DEVICE_SERIAL_NUMBER]).ok();
            let protocol = about.as_ref().map(|v| values_text(v, &WPD_DEVICE_PROTOCOL)).unwrap_or_default();
            let serial = about.as_ref().map(|v| values_text(v, &WPD_DEVICE_SERIAL_NUMBER)).unwrap_or_default();
            let ids = content.EnumObjects(0, WPD_DEVICE_OBJECT_ID, None)?;
            let mut out = Vec::new();
            loop {
                let mut batch = [PWSTR::null(); 16];
                let mut fetched = 0u32;
                let hr = ids.Next(&mut batch, &mut fetched);
                for p in &batch[..fetched as usize] {
                    let object_id = take(*p);
                    let id = HSTRING::from(object_id.as_str());
                    let Ok(v) = properties(&props, &id, &[WPD_FUNCTIONAL_OBJECT_CATEGORY, WPD_OBJECT_NAME, WPD_OBJECT_PERSISTENT_UNIQUE_ID, WPD_STORAGE_DESCRIPTION, WPD_STORAGE_CAPACITY, WPD_STORAGE_FREE_SPACE_IN_BYTES]) else { continue };
                    if v.GetGuidValue(&WPD_FUNCTIONAL_OBJECT_CATEGORY).ok() != Some(WPD_FUNCTIONAL_CATEGORY_STORAGE) {
                        continue;
                    }
                    out.push(MtpStorage {
                        object_id,
                        persistent_id: values_text(&v, &WPD_OBJECT_PERSISTENT_UNIQUE_ID),
                        name: values_text(&v, &WPD_OBJECT_NAME),
                        description: values_text(&v, &WPD_STORAGE_DESCRIPTION),
                        capacity_bytes: values_u64(&v, &WPD_STORAGE_CAPACITY),
                        free_bytes: values_u64(&v, &WPD_STORAGE_FREE_SPACE_IN_BYTES),
                    });
                }
                if hr.is_err() || fetched == 0 || hr.0 == 1 {
                    break; // S_FALSE: no more objects
                }
            }
            Ok((protocol, serial, out))
        }
    }

    /// The WPD device ids plugged in now (no device is opened).
    pub fn device_ids() -> Vec<String> {
        // SAFETY: COM initialised for this thread; strings freed by `take`.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let Ok(manager) = CoCreateInstance::<_, IPortableDeviceManager>(&PortableDeviceManager, None, CLSCTX_INPROC_SERVER) else { return Vec::new() };
            let _ = manager.RefreshDeviceList();
            let mut count = 0u32;
            if manager.GetDevices(std::ptr::null_mut(), &mut count).is_err() || count == 0 {
                return Vec::new();
            }
            let mut ids = vec![PWSTR::null(); count as usize];
            if manager.GetDevices(ids.as_mut_ptr(), &mut count).is_err() {
                return Vec::new();
            }
            ids.into_iter().take(count as usize).map(take).collect()
        }
    }

    /// Every WPD device Windows knows now, with its storages. Read-only.
    pub fn probe() -> Result<Vec<MtpDevice>, String> {
        let err = |e: windows::core::Error| format!("WPD: {e}");
        // SAFETY: COM is initialised for this thread (a second call is harmless); every call below
        // is on interfaces WPD returned, and every returned string is freed once.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let manager: IPortableDeviceManager = CoCreateInstance(&PortableDeviceManager, None, CLSCTX_INPROC_SERVER).map_err(err)?;
            let mut count = 0u32;
            manager.GetDevices(std::ptr::null_mut(), &mut count).map_err(err)?;
            let mut ids = vec![PWSTR::null(); count as usize];
            if count > 0 {
                manager.GetDevices(ids.as_mut_ptr(), &mut count).map_err(err)?;
            }
            let client: IPortableDeviceValues = CoCreateInstance(&PortableDeviceValues, None, CLSCTX_INPROC_SERVER).map_err(err)?;
            client.SetStringValue(&WPD_CLIENT_NAME, &HSTRING::from("Miu Music")).map_err(err)?;
            client.SetUnsignedIntegerValue(&WPD_CLIENT_MAJOR_VERSION, 0).map_err(err)?;
            let mut out = Vec::new();
            for p in ids.into_iter().take(count as usize) {
                let id = HSTRING::from(take(p));
                let name = manager_text(|buf, len| manager.GetDeviceFriendlyName(&id, buf, len));
                let manufacturer = manager_text(|buf, len| manager.GetDeviceManufacturer(&id, buf, len));
                let device: IPortableDevice = CoCreateInstance(&PortableDeviceFTM, None, CLSCTX_INPROC_SERVER).map_err(err)?;
                let (protocol, serial, storages) = match device.Open(&id, &client) {
                    Ok(()) => {
                        let found = storages(&device).unwrap_or_else(|e| (format!("? ({e})"), String::new(), Vec::new()));
                        let _ = device.Close();
                        found
                    }
                    Err(e) => (format!("? (could not open: {e})"), String::new(), Vec::new()),
                };
                out.push(MtpDevice { id: id.to_string(), name, manufacturer, serial, protocol, storages });
            }
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn storage(id: &str, puid: &str, name: &str) -> MtpStorage {
        MtpStorage { object_id: id.into(), persistent_id: puid.into(), name: name.into(), description: String::new(), capacity_bytes: Some(64_000_000_000), free_bytes: Some(20_000_000_000) }
    }

    fn device(name: &str, maker: &str, protocol: &str, serial: &str, storages: Vec<MtpStorage>) -> MtpDevice {
        MtpDevice { id: format!("wpd-{name}"), name: name.into(), manufacturer: maker.into(), serial: serial.into(), protocol: protocol.into(), storages }
    }

    #[test]
    fn each_storage_of_an_mtp_device_is_a_player_known_by_serial_and_storage() {
        let devices = vec![
            device("Pixel 7", "Google", "MTP: 1.00", "28141FDH", vec![storage("s10001", "{AAA}", "Internal shared storage"), storage("s20001", "{BBB}", "SD card")]),
            device("Apple iPhone", "Apple Inc.", "PTP: 1.00", "e51e30a5", vec![storage("s1", "{C}", "Internal Storage")]),
            device("2tb", "Seagate", "MSC:", "", vec![storage("D:", "", "D:")]),
        ];
        let mut known = BTreeMap::new();
        known.insert("mtp:28141FDH:{BBB}".to_string(), Profile { folder: "Audio".into(), ..Profile::default() });
        let players = players(&devices, &known);
        assert_eq!(players.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["mtp:28141FDH:{AAA}", "mtp:28141FDH:{BBB}"]);
        let (internal, card) = (&players[0], &players[1]);
        assert_eq!(internal.kind, PlayerKind::Mtp);
        assert_eq!(internal.label, "Pixel 7 — Internal shared storage");
        assert_eq!((internal.total_bytes, internal.free_bytes), (64_000_000_000, 20_000_000_000));
        assert!(!internal.configured, "a suggestion until it is set up");
        assert_eq!(internal.profile.folder, "Music");
        assert_eq!(internal.wpd_device.as_deref(), Some("wpd-Pixel 7"));
        assert_eq!(internal.wpd_storage.as_deref(), Some("s10001"));
        assert!(card.configured);
        assert_eq!(card.profile.folder, "Audio");
    }

    #[test]
    fn without_a_serial_or_storage_id_the_names_identify_the_player() {
        let devices = vec![device("NW-A306", "Sony", "MTP: 1.00", "", vec![storage("s1", "", "Internal Storage")])];
        assert_eq!(players(&devices, &BTreeMap::new())[0].id, "mtp:NW-A306:Internal Storage");
    }

    #[test]
    fn files_are_announced_in_their_format() {
        assert_eq!(format_for("mp3"), "mp3");
        assert_eq!(format_for("M4A"), "m4a");
        assert_eq!(format_for("flac"), "flac");
        assert_eq!(format_for("wav"), "wav");
        assert_eq!(format_for("xyz"), "unspecified");
    }

    #[test]
    fn mtp_devices_are_players_and_an_iphone_is_not() {
        assert_eq!(kind("MTP: 1.00", "Sony"), Kind::Player);
        assert_eq!(kind("MTP: 1.00", "samsung"), Kind::Player);
        assert_eq!(kind("PTP: 1.00", "Apple Inc."), Kind::Iphone, "an iPhone is a PTP camera to WPD");
        assert_eq!(kind("MTP: 1.00", "Apple Inc."), Kind::Iphone);
        assert_eq!(kind("PTP: 1.00", "Canon Inc."), Kind::Other);
        assert_eq!(kind("", ""), Kind::Other);
        assert_eq!(kind("MSC:", "Seagate "), Kind::Other, "USB drives also show up in WPD; Miu copies to them as drives");
    }

    /// Lists the MTP / PTP devices plugged in now. `cargo test --lib player::mtp -- --ignored --nocapture`
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn lists_the_portable_devices_plugged_in() {
        let devices = probe().unwrap();
        if devices.is_empty() {
            println!("no portable devices plugged in");
        }
        for d in &devices {
            println!("{:?} by {:?}, {:?} → {:?}", d.name, d.manufacturer, d.protocol, kind(&d.protocol, &d.manufacturer));
            for s in &d.storages {
                println!("  {} {:?} {:?}: {:?} of {:?} bytes free", s.object_id, s.name, s.description, s.free_bytes, s.capacity_bytes);
            }
        }
    }
}
