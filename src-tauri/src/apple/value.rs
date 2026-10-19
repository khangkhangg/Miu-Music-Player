//! Plain Rust form of CoreFoundation property-list values. No FFI here, so code that only
//! inspects values (device info, sync messages) stays platform-neutral and unit-testable.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum CfValue {
    String(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Data(Vec<u8>),
    Array(Vec<CfValue>),
    Dict(BTreeMap<String, CfValue>),
}

impl CfValue {
    /// JSON for logs and captures. Binary data becomes `{"$data": "<hex>", "len": n}`.
    pub fn to_json(&self) -> serde_json::Value {
        use serde_json::{json, Value};
        match self {
            CfValue::String(s) => Value::String(s.clone()),
            CfValue::Int(n) => json!(n),
            CfValue::Float(x) => json!(x),
            CfValue::Bool(b) => Value::Bool(*b),
            CfValue::Data(d) => {
                let hex: String = d.iter().map(|b| format!("{b:02x}")).collect();
                json!({ "$data": hex, "len": d.len() })
            }
            CfValue::Array(items) => Value::Array(items.iter().map(CfValue::to_json).collect()),
            CfValue::Dict(map) => Value::Object(map.iter().map(|(k, v)| (k.clone(), v.to_json())).collect()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scalars_become_plain_json() {
        assert_eq!(CfValue::String("SyncAllowed".into()).to_json(), json!("SyncAllowed"));
        assert_eq!(CfValue::Int(-3).to_json(), json!(-3));
        assert_eq!(CfValue::Float(1.5).to_json(), json!(1.5));
        assert_eq!(CfValue::Bool(true).to_json(), json!(true));
    }

    #[test]
    fn data_becomes_tagged_hex() {
        assert_eq!(CfValue::Data(vec![0x04, 0x00, 0xff]).to_json(), json!({"$data": "0400ff", "len": 3}));
    }

    #[test]
    fn containers_nest() {
        let v = CfValue::Dict(BTreeMap::from([
            ("Command".to_string(), CfValue::String("SyncAllowed".into())),
            ("Params".to_string(), CfValue::Dict(BTreeMap::from([(
                "Dataclasses".to_string(),
                CfValue::Array(vec![CfValue::String("Media".into())]),
            )]))),
        ]));
        assert_eq!(v.to_json(), json!({"Command": "SyncAllowed", "Params": {"Dataclasses": ["Media"]}}));
    }
}
