//! What Miu tells the phone about itself in an AirTraffic sync (F2.3).
//! Values and reasons: docs/plans/F2.3-sync-handshake.md.

use std::collections::BTreeMap;
use std::hash::{BuildHasher, Hasher};

use crate::apple::value::CfValue;
use crate::library::db::{Db, Result};

const LIBRARY_ID_KEY: &str = "atc_library_id";

fn s(v: &str) -> CfValue {
    CfValue::String(v.into())
}

pub fn host_info(library_id: &str) -> CfValue {
    CfValue::Dict(BTreeMap::from([
        ("Type".into(), s("iTunes")),
        ("Version".into(), s("12.13.9.1")),
        ("MacOSVersion".into(), s("Windows NT 10.0")),
        ("SyncHostName".into(), s("Miu Music")),
        ("LibraryID".into(), s(library_id)),
        ("SyncedDataclasses".into(), CfValue::Array(vec![s("Media")])),
        ("SyncedAssetTypes".into(), CfValue::Array(vec![s("Music")])),
        ("Wakeable".into(), CfValue::Bool(false)),
    ]))
}

pub fn sync_dataclasses() -> CfValue {
    CfValue::Array(vec![s("Media")])
}

/// Empty for now: the phone's ReadyForSync reply should show the anchor format it expects.
pub fn sync_anchors() -> CfValue {
    CfValue::Dict(BTreeMap::new())
}

/// This install's library ID: 16 upper-case hex chars, created on first use and kept in settings,
/// so the phone sees the same "computer" every time.
pub fn library_id(db: &Db) -> Result<String> {
    if let Some(id) = db.setting(LIBRARY_ID_KEY)? {
        return Ok(id);
    }
    let id = format!("{:016X}", random_u64());
    db.set_setting(LIBRARY_ID_KEY, &id)?;
    Ok(id)
}

/// Not cryptographic; only needs to differ between installs. `RandomState` is seeded per process.
fn random_u64() -> u64 {
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    h.write_u32(std::process::id());
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> CfValue {
        CfValue::String(v.into())
    }

    #[test]
    fn host_info_has_exactly_the_planned_keys_and_values() {
        let CfValue::Dict(d) = host_info("0123456789ABCDEF") else { panic!("not a dict") };
        let keys: Vec<&str> = d.keys().map(String::as_str).collect();
        assert_eq!(keys, ["LibraryID", "MacOSVersion", "SyncHostName", "SyncedAssetTypes", "SyncedDataclasses", "Type", "Version", "Wakeable"]);
        assert_eq!(d["Type"], s("iTunes"));
        assert_eq!(d["Version"], s("12.13.9.1"));
        assert_eq!(d["MacOSVersion"], s("Windows NT 10.0"));
        assert_eq!(d["SyncHostName"], s("Miu Music"));
        assert_eq!(d["LibraryID"], s("0123456789ABCDEF"));
        assert_eq!(d["SyncedDataclasses"], CfValue::Array(vec![s("Media")]));
        assert_eq!(d["SyncedAssetTypes"], CfValue::Array(vec![s("Music")]));
        assert_eq!(d["Wakeable"], CfValue::Bool(false));
    }

    #[test]
    fn sync_request_asks_for_media_with_empty_anchors() {
        assert_eq!(sync_dataclasses(), CfValue::Array(vec![s("Media")]));
        assert_eq!(sync_anchors(), CfValue::Dict(Default::default()));
    }

    #[test]
    fn library_id_is_16_upper_hex_chars() {
        let id = library_id(&Db::open_in_memory().unwrap()).unwrap();
        assert_eq!(id.len(), 16);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase()), "{id}");
    }

    #[test]
    fn library_id_is_generated_once_and_kept() {
        let db = Db::open_in_memory().unwrap();
        let first = library_id(&db).unwrap();
        assert_eq!(library_id(&db).unwrap(), first);
        assert_eq!(db.setting("atc_library_id").unwrap().as_deref(), Some(first.as_str()));
    }

    #[test]
    fn different_installs_get_different_ids() {
        let a = library_id(&Db::open_in_memory().unwrap()).unwrap();
        let b = library_id(&Db::open_in_memory().unwrap()).unwrap();
        assert_ne!(a, b);
    }
}
