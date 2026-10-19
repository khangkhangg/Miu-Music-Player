//! What we show about a connected iPhone (F1.4), built from lockdown values.

use serde::Serialize;

use crate::apple::value::CfValue;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub name: String,
    pub model_id: String,
    pub model: String,
    pub ios_version: String,
    pub capacity_bytes: Option<u64>,
    pub free_bytes: Option<u64>,
}

const MODELS: &[(&str, &str)] = &[
    ("iPhone5,1", "iPhone 5"),
    ("iPhone5,2", "iPhone 5"),
    ("iPhone5,3", "iPhone 5c"),
    ("iPhone5,4", "iPhone 5c"),
    ("iPhone6,1", "iPhone 5s"),
    ("iPhone6,2", "iPhone 5s"),
    ("iPhone7,1", "iPhone 6 Plus"),
    ("iPhone7,2", "iPhone 6"),
    ("iPhone8,1", "iPhone 6s"),
    ("iPhone8,2", "iPhone 6s Plus"),
    ("iPhone8,4", "iPhone SE"),
    ("iPhone9,1", "iPhone 7"),
    ("iPhone9,3", "iPhone 7"),
    ("iPhone9,2", "iPhone 7 Plus"),
    ("iPhone9,4", "iPhone 7 Plus"),
    ("iPhone10,1", "iPhone 8"),
    ("iPhone10,4", "iPhone 8"),
    ("iPhone10,2", "iPhone 8 Plus"),
    ("iPhone10,5", "iPhone 8 Plus"),
    ("iPhone10,3", "iPhone X"),
    ("iPhone10,6", "iPhone X"),
];

/// Marketing name for a `ProductType`, or the identifier itself if we don't know it.
pub fn model_name(product_type: &str) -> String {
    MODELS
        .iter()
        .find(|(id, _)| *id == product_type)
        .map(|(_, name)| name.to_string())
        .unwrap_or_else(|| product_type.to_string())
}

impl DeviceInfo {
    /// `general`: values from the default lockdown domain; `disk`: domain `com.apple.disk_usage`.
    pub fn from_lockdown(general: &CfValue, disk: &CfValue) -> Self {
        let model_id = text(general, "ProductType");
        let model = if model_id.is_empty() { "iPhone".to_string() } else { model_name(&model_id) };
        let name = Some(text(general, "DeviceName")).filter(|n| !n.is_empty()).unwrap_or_else(|| model.clone());
        Self {
            name,
            model,
            model_id,
            ios_version: text(general, "ProductVersion"),
            capacity_bytes: size(disk, "TotalDataCapacity"),
            free_bytes: size(disk, "TotalDataAvailable").or_else(|| size(disk, "AmountDataAvailable")),
        }
    }
}

fn field<'a>(dict: &'a CfValue, key: &str) -> Option<&'a CfValue> {
    match dict {
        CfValue::Dict(map) => map.get(key),
        _ => None,
    }
}

fn text(dict: &CfValue, key: &str) -> String {
    match field(dict, key) {
        Some(CfValue::String(s)) => s.clone(),
        _ => String::new(),
    }
}

fn size(dict: &CfValue, key: &str) -> Option<u64> {
    match field(dict, key) {
        Some(CfValue::Int(n)) if *n >= 0 => Some(*n as u64),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn dict(pairs: &[(&str, CfValue)]) -> CfValue {
        CfValue::Dict(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect::<BTreeMap<_, _>>())
    }
    fn s(v: &str) -> CfValue {
        CfValue::String(v.into())
    }

    #[test]
    fn known_models_get_marketing_names() {
        assert_eq!(model_name("iPhone6,1"), "iPhone 5s");
        assert_eq!(model_name("iPhone6,2"), "iPhone 5s");
        assert_eq!(model_name("iPhone5,3"), "iPhone 5c");
        assert_eq!(model_name("iPhone7,2"), "iPhone 6");
        assert_eq!(model_name("iPhone8,4"), "iPhone SE");
    }

    #[test]
    fn unknown_models_keep_their_identifier() {
        assert_eq!(model_name("iPhone99,9"), "iPhone99,9");
    }

    #[test]
    fn builds_info_from_lockdown_values() {
        let general = dict(&[("DeviceName", s("Khang's iPhone")), ("ProductType", s("iPhone6,1")), ("ProductVersion", s("12.5.8"))]);
        let disk = dict(&[("TotalDataCapacity", CfValue::Int(12_000_000_000)), ("TotalDataAvailable", CfValue::Int(5_100_000_000))]);
        assert_eq!(
            DeviceInfo::from_lockdown(&general, &disk),
            DeviceInfo {
                name: "Khang's iPhone".into(),
                model_id: "iPhone6,1".into(),
                model: "iPhone 5s".into(),
                ios_version: "12.5.8".into(),
                capacity_bytes: Some(12_000_000_000),
                free_bytes: Some(5_100_000_000),
            }
        );
    }

    #[test]
    fn falls_back_to_amount_data_available_for_free_space() {
        let disk = dict(&[("TotalDataCapacity", CfValue::Int(100)), ("AmountDataAvailable", CfValue::Int(40))]);
        let info = DeviceInfo::from_lockdown(&dict(&[]), &disk);
        assert_eq!(info.free_bytes, Some(40));
    }

    #[test]
    fn missing_values_give_safe_defaults() {
        let info = DeviceInfo::from_lockdown(&CfValue::Bool(false), &dict(&[]));
        assert_eq!(info.name, "iPhone");
        assert_eq!(info.model, "iPhone");
        assert_eq!(info.ios_version, "");
        assert_eq!(info.capacity_bytes, None);
        assert_eq!(info.free_bytes, None);
    }

    #[test]
    fn negative_sizes_are_ignored() {
        let disk = dict(&[("TotalDataCapacity", CfValue::Int(-1))]);
        assert_eq!(DeviceInfo::from_lockdown(&dict(&[]), &disk).capacity_bytes, None);
    }

    #[test]
    fn serializes_camel_case_for_the_ui() {
        let info = DeviceInfo::from_lockdown(&dict(&[("ProductVersion", s("12.5.8"))]), &dict(&[]));
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["iosVersion"], "12.5.8");
        assert!(json.get("freeBytes").is_some());
    }
}
