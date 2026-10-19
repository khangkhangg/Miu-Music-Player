//! Which iPhones are plugged in (F1.3). Pure state, fed by `apple::device` notifications.

use std::collections::BTreeMap;

use serde::Serialize;

/// `am_device_notification_callback_info.msg`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyKind {
    Connected,
    Disconnected,
    Unsubscribed,
}

impl NotifyKind {
    pub fn from_msg(msg: u32) -> Option<Self> {
        match msg {
            1 => Some(Self::Connected),
            2 => Some(Self::Disconnected),
            3 => Some(Self::Unsubscribed),
            _ => None,
        }
    }
}

/// `AMDeviceGetInterfaceType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interface {
    Usb,
    Wifi,
    Unknown,
}

impl Interface {
    pub fn from_raw(raw: i32) -> Self {
        match raw {
            1 => Self::Usb,
            2 => Self::Wifi,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RawEvent {
    Connected { udid: String, interface: Interface, trusted: bool },
    Disconnected { udid: String, interface: Interface },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Device {
    pub udid: String,
    pub trusted: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Added(Device),
    Updated(Device),
    Removed(String),
}

/// USB-connected devices only: syncing needs the cable, and a phone on the same Wi-Fi shows up
/// as a second appearance of the same UDID.
#[derive(Debug, Default)]
pub struct Registry {
    devices: BTreeMap<String, Device>,
}

impl Registry {
    pub fn apply(&mut self, event: RawEvent) -> Option<Change> {
        match event {
            RawEvent::Connected { udid, interface: Interface::Usb, trusted } => {
                let device = Device { udid: udid.clone(), trusted };
                match self.devices.insert(udid, device.clone()) {
                    None => Some(Change::Added(device)),
                    Some(old) if old != device => Some(Change::Updated(device)),
                    Some(_) => None,
                }
            }
            RawEvent::Disconnected { udid, interface: Interface::Usb } => {
                self.devices.remove(&udid).map(|_| Change::Removed(udid))
            }
            _ => None,
        }
    }

    pub fn devices(&self) -> Vec<&Device> {
        self.devices.values().collect()
    }
}

/// First 8 characters of a UDID, for logs (the full UDID identifies the phone).
pub fn short_udid(udid: &str) -> String {
    match udid.char_indices().nth(8) {
        Some((i, _)) => format!("{}…", &udid[..i]),
        None => udid.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connected(udid: &str, interface: Interface, trusted: bool) -> RawEvent {
        RawEvent::Connected { udid: udid.into(), interface, trusted }
    }

    #[test]
    fn maps_notification_codes() {
        assert_eq!(NotifyKind::from_msg(1), Some(NotifyKind::Connected));
        assert_eq!(NotifyKind::from_msg(2), Some(NotifyKind::Disconnected));
        assert_eq!(NotifyKind::from_msg(3), Some(NotifyKind::Unsubscribed));
        assert_eq!(NotifyKind::from_msg(99), None);
    }

    #[test]
    fn maps_interface_types() {
        assert_eq!(Interface::from_raw(1), Interface::Usb);
        assert_eq!(Interface::from_raw(2), Interface::Wifi);
        assert_eq!(Interface::from_raw(7), Interface::Unknown);
    }

    #[test]
    fn a_usb_connect_adds_the_device() {
        let mut r = Registry::default();
        let change = r.apply(connected("abc", Interface::Usb, false));
        assert_eq!(change, Some(Change::Added(Device { udid: "abc".into(), trusted: false })));
        assert_eq!(r.devices().len(), 1);
    }

    #[test]
    fn wifi_appearances_are_ignored() {
        let mut r = Registry::default();
        assert_eq!(r.apply(connected("abc", Interface::Wifi, true)), None);
        assert!(r.devices().is_empty());
    }

    #[test]
    fn repeating_the_same_connect_changes_nothing() {
        let mut r = Registry::default();
        r.apply(connected("abc", Interface::Usb, true));
        assert_eq!(r.apply(connected("abc", Interface::Usb, true)), None);
    }

    #[test]
    fn trusting_the_computer_updates_the_device() {
        let mut r = Registry::default();
        r.apply(connected("abc", Interface::Usb, false));
        let change = r.apply(connected("abc", Interface::Usb, true));
        assert_eq!(change, Some(Change::Updated(Device { udid: "abc".into(), trusted: true })));
    }

    #[test]
    fn usb_disconnect_removes_the_device() {
        let mut r = Registry::default();
        r.apply(connected("abc", Interface::Usb, true));
        assert_eq!(r.apply(RawEvent::Disconnected { udid: "abc".into(), interface: Interface::Usb }), Some(Change::Removed("abc".into())));
        assert!(r.devices().is_empty());
    }

    #[test]
    fn disconnecting_an_unknown_or_wifi_device_changes_nothing() {
        let mut r = Registry::default();
        r.apply(connected("abc", Interface::Usb, true));
        assert_eq!(r.apply(RawEvent::Disconnected { udid: "zzz".into(), interface: Interface::Usb }), None);
        assert_eq!(r.apply(RawEvent::Disconnected { udid: "abc".into(), interface: Interface::Wifi }), None);
        assert_eq!(r.devices().len(), 1);
    }

    #[test]
    fn devices_are_listed_in_udid_order() {
        let mut r = Registry::default();
        r.apply(connected("b", Interface::Usb, true));
        r.apply(connected("a", Interface::Usb, true));
        let udids: Vec<_> = r.devices().iter().map(|d| d.udid.as_str()).collect();
        assert_eq!(udids, ["a", "b"]);
    }

    #[test]
    fn udids_are_shortened_for_logs() {
        assert_eq!(short_udid("00008030001A2B3C4D5E6F70"), "00008030…");
        assert_eq!(short_udid("abc"), "abc");
    }
}
