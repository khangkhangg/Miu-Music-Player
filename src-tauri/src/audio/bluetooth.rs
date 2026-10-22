//! Battery level of Bluetooth speakers (F7.9). Windows does not tell apps how strong a connected
//! speaker's signal is, but it does keep the battery level some speakers report (through their
//! hands-free service), as a device property. Output names look like "Speakers (Bose Flex
//! SoundLink)"; the Bluetooth devices are named after the speaker.

/// The speaker's own name inside an output name: "Speakers (Bose Flex SoundLink)" → "Bose Flex SoundLink".
pub fn speaker_name(output: &str) -> Option<&str> {
    let inner = output.strip_suffix(')')?;
    let open = inner.find('(')?;
    Some(inner[open + 1..].trim()).filter(|s| !s.is_empty())
}

/// What Windows appends to a speaker's name for its Bluetooth services.
const SERVICE_SUFFIXES: [&str; 3] = [" Hands-Free AG", " Hands-Free", " Avrcp Transport"];

fn device_speaker(device: &str) -> &str {
    SERVICE_SUFFIXES.iter().find_map(|s| device.strip_suffix(s)).unwrap_or(device)
}

/// Battery (percent) for `output`, from present devices as (name, battery if it reports one).
pub fn battery_for(output: &str, devices: &[(String, Option<u8>)]) -> Option<u8> {
    let name = speaker_name(output)?;
    devices.iter().filter(|(d, _)| device_speaker(d) == name).find_map(|(_, b)| *b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_speaker_name_is_the_part_in_brackets() {
        assert_eq!(speaker_name("Speakers (Bose Flex SoundLink)"), Some("Bose Flex SoundLink"));
        assert_eq!(speaker_name("Headphones (Klipsch The One)"), Some("Klipsch The One"));
        assert_eq!(speaker_name("Speakers (2- JBL Flip 6 (Stereo))"), Some("2- JBL Flip 6 (Stereo)"), "brackets inside the name");
        assert_eq!(speaker_name("Speakers"), None);
    }

    fn devices() -> Vec<(String, Option<u8>)> {
        vec![
            ("Bose Flex SoundLink".into(), None),
            ("Bose Flex SoundLink Avrcp Transport".into(), None),
            ("Bose Flex SoundLink Hands-Free AG".into(), Some(40)),
            ("Klipsch The One".into(), None),
        ]
    }

    #[test]
    fn the_battery_comes_from_whichever_device_of_that_speaker_reports_it() {
        assert_eq!(battery_for("Speakers (Bose Flex SoundLink)", &devices()), Some(40));
        assert_eq!(battery_for("Headphones (Klipsch The One)", &devices()), None, "mains-powered: no battery");
        assert_eq!(battery_for("LG HDR 4K (HD Audio Driver for Display Audio)", &devices()), None);
    }

    #[test]
    fn a_speaker_whose_name_starts_another_speakers_name_does_not_borrow_its_battery() {
        let d = vec![("JBL Flip 6 Pro Hands-Free AG".into(), Some(80)), ("JBL Flip 6 Hands-Free AG".into(), Some(55))];
        assert_eq!(battery_for("Speakers (JBL Flip 6)", &d), Some(55));
    }
}

/// Present Bluetooth devices: (name, battery percent if the device reports one).
#[cfg(windows)]
pub fn present_devices() -> Vec<(String, Option<u8>)> {
    use windows::core::{GUID, PCWSTR};
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        CM_Get_DevNode_PropertyW, CM_Get_Device_ID_ListW, CM_Get_Device_ID_List_SizeW, CM_Locate_DevNodeW, CM_GETIDLIST_FILTER_PRESENT, CM_LOCATE_DEVNODE_NORMAL, CR_SUCCESS,
    };
    use windows::Win32::Devices::Properties::{DEVPKEY_Device_FriendlyName, DEVPROPTYPE};
    use windows::Win32::Foundation::DEVPROPKEY;

    // DEVPKEY_Bluetooth_Battery: one byte, percent.
    const BATTERY: DEVPROPKEY = DEVPROPKEY { fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5), pid: 2 };

    let property = |node: u32, key: &DEVPROPKEY| -> Option<Vec<u8>> {
        let mut kind = DEVPROPTYPE::default();
        let mut size = 0u32;
        // SAFETY: a size query (no buffer), then a read into a buffer of exactly that size.
        unsafe {
            let _ = CM_Get_DevNode_PropertyW(node, key, &mut kind, None, &mut size, 0);
            if size == 0 {
                return None;
            }
            let mut buf = vec![0u8; size as usize];
            (CM_Get_DevNode_PropertyW(node, key, &mut kind, Some(buf.as_mut_ptr()), &mut size, 0) == CR_SUCCESS).then_some(buf)
        }
    };
    let mut len = 0u32;
    // SAFETY: plain size query and list fill with a buffer of the size Windows asked for.
    let list = unsafe {
        if CM_Get_Device_ID_List_SizeW(&mut len, PCWSTR::null(), CM_GETIDLIST_FILTER_PRESENT) != CR_SUCCESS {
            return Vec::new();
        }
        let mut buf = vec![0u16; len as usize];
        if CM_Get_Device_ID_ListW(PCWSTR::null(), &mut buf, CM_GETIDLIST_FILTER_PRESENT) != CR_SUCCESS {
            return Vec::new();
        }
        buf
    };
    let mut out = Vec::new();
    for id in list.split(|&c| c == 0).filter(|s| !s.is_empty()) {
        if !String::from_utf16_lossy(id).starts_with("BTH") {
            continue;
        }
        let mut wide = id.to_vec();
        wide.push(0);
        let mut node = 0u32;
        // SAFETY: `wide` is a NUL-terminated device instance id.
        if unsafe { CM_Locate_DevNodeW(&mut node, PCWSTR(wide.as_ptr()), CM_LOCATE_DEVNODE_NORMAL) } != CR_SUCCESS {
            continue;
        }
        let Some(name) = property(node, &DEVPKEY_Device_FriendlyName) else { continue };
        let units: Vec<u16> = name.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).take_while(|&c| c != 0).collect();
        let battery = property(node, &BATTERY).and_then(|b| b.first().copied()).filter(|&b| b <= 100);
        out.push((String::from_utf16_lossy(&units), battery));
    }
    out
}

#[cfg(not(windows))]
pub fn present_devices() -> Vec<(String, Option<u8>)> {
    Vec::new()
}

#[cfg(test)]
mod machine_tests {
    // Machine test: lists the Bluetooth devices Windows knows, with any battery level.
    #[test]
    #[ignore]
    fn lists_bluetooth_devices_with_battery() {
        let d = super::present_devices();
        println!("{d:#?}");
        assert!(!d.is_empty(), "no Bluetooth devices present");
    }
}

/// Microphones that can listen during calibration: not one belonging to a speaker in `playing`
/// (opening a Bluetooth speaker's microphone switches it into call mode, which ruins its sound).
pub fn usable_mics(inputs: &[String], outputs: &[String]) -> Vec<String> {
    let speakers: Vec<&str> = outputs.iter().filter_map(|o| speaker_name(o)).collect();
    inputs
        .iter()
        .filter(|i| !i.contains("Hands-Free"))
        .filter(|i| speaker_name(i).is_none_or(|s| !speakers.contains(&s)))
        .cloned()
        .collect()
}

#[cfg(test)]
mod mic_tests {
    use super::*;

    #[test]
    fn a_speakers_own_microphone_is_never_offered() {
        let inputs = vec![
            "Microphone (USB Audio and HID)".to_string(),
            "Headset Microphone (Bose Flex SoundLink)".to_string(),
            "Microphone (Microsoft LifeCam NX-6000)".to_string(),
        ];
        let outputs = vec!["Headphones (Klipsch The One)".to_string(), "Speakers (Bose Flex SoundLink)".to_string()];
        assert_eq!(usable_mics(&inputs, &outputs), vec!["Microphone (USB Audio and HID)".to_string(), "Microphone (Microsoft LifeCam NX-6000)".to_string()]);
    }

    #[test]
    fn only_the_playing_speakers_mics_are_left_out() {
        // A USB headset has both; its speaker side is not playing, so its microphone is fine.
        let inputs = vec!["Microphone (USB Audio and HID)".to_string()];
        let playing = vec!["Headphones (Klipsch The One)".to_string(), "Speakers (Bose Flex SoundLink)".to_string()];
        assert_eq!(usable_mics(&inputs, &playing), inputs);
    }

    #[test]
    fn hands_free_inputs_are_left_out_too() {
        let inputs = vec!["Headset (JBL Flip 6 Hands-Free)".to_string()];
        assert!(usable_mics(&inputs, &[]).is_empty());
    }
}
