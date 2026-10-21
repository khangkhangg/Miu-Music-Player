//! Read-only probe of MTP devices (Android phones, Sony Walkman, newer SanDisk players) through the
//! Windows Portable Devices (WPD) COM API: lists each device and its storages with their space.
//! Nothing is written. See `docs/MTP.md` for what syncing over MTP would take.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MtpStorage {
    pub object_id: String,
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

#[cfg(windows)]
pub use imp::probe;

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

    fn storages(device: &IPortableDevice) -> windows::core::Result<(String, Vec<MtpStorage>)> {
        // SAFETY: plain COM calls on live interfaces; returned strings are freed by `take`.
        unsafe {
            let content = device.Content()?;
            let props = content.Properties()?;
            let root = HSTRING::from("DEVICE");
            let protocol = properties(&props, &root, &[WPD_DEVICE_PROTOCOL]).map(|v| values_text(&v, &WPD_DEVICE_PROTOCOL)).unwrap_or_default();
            let ids = content.EnumObjects(0, WPD_DEVICE_OBJECT_ID, None)?;
            let mut out = Vec::new();
            loop {
                let mut batch = [PWSTR::null(); 16];
                let mut fetched = 0u32;
                let hr = ids.Next(&mut batch, &mut fetched);
                for p in &batch[..fetched as usize] {
                    let object_id = take(*p);
                    let id = HSTRING::from(object_id.as_str());
                    let Ok(v) = properties(&props, &id, &[WPD_FUNCTIONAL_OBJECT_CATEGORY, WPD_OBJECT_NAME, WPD_STORAGE_DESCRIPTION, WPD_STORAGE_CAPACITY, WPD_STORAGE_FREE_SPACE_IN_BYTES]) else { continue };
                    if v.GetGuidValue(&WPD_FUNCTIONAL_OBJECT_CATEGORY).ok() != Some(WPD_FUNCTIONAL_CATEGORY_STORAGE) {
                        continue;
                    }
                    out.push(MtpStorage {
                        object_id,
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
            Ok((protocol, out))
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
                let (protocol, storages) = match device.Open(&id, &client) {
                    Ok(()) => {
                        let found = storages(&device).unwrap_or_else(|e| (format!("? ({e})"), Vec::new()));
                        let _ = device.Close();
                        found
                    }
                    Err(e) => (format!("? (could not open: {e})"), Vec::new()),
                };
                out.push(MtpDevice { id: id.to_string(), name, manufacturer, protocol, storages });
            }
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
