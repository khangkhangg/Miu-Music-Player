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
