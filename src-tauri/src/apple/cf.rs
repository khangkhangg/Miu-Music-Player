//! CoreFoundation bridge (F1.2): Rust values ⇄ CF objects, with ownership handled by `CfRef`.
//! Function pointers come from Apple's CoreFoundation.dll, loaded by `apple::loader`.

use std::collections::BTreeMap;
use std::ffi::c_void;

use libloading::os::windows::Library;

pub type Ref = *const c_void;

const UTF8: u32 = 0x0800_0100; // kCFStringEncodingUTF8
const SINT64: isize = 4; // kCFNumberSInt64Type
const FLOAT64: isize = 6; // kCFNumberFloat64Type

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

#[derive(Debug, thiserror::Error)]
pub enum CfError {
    #[error("CoreFoundation.dll is missing {0}")]
    Symbol(String),
    #[error("null CF object")]
    Null,
    #[error("unsupported CF type id {0}")]
    Unsupported(usize),
    #[error("dictionary key is not a string")]
    NonStringKey,
    #[error("{0} returned null")]
    CreateFailed(&'static str),
}

type Result<T> = std::result::Result<T, CfError>;

/// An owned CF object (+1 retain); released on drop.
pub struct CfRef {
    ptr: Ref,
    release: unsafe extern "C" fn(Ref),
}

impl CfRef {
    pub fn as_ptr(&self) -> Ref {
        self.ptr
    }
}

impl Drop for CfRef {
    fn drop(&mut self) {
        // SAFETY: we own exactly one retain on `ptr` (created or retained by us).
        unsafe { (self.release)(self.ptr) }
    }
}

impl std::fmt::Debug for CfRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CfRef({:p})", self.ptr)
    }
}

#[allow(non_snake_case)]
struct Fns {
    CFRelease: unsafe extern "C" fn(Ref),
    CFRetain: unsafe extern "C" fn(Ref) -> Ref,
    CFGetRetainCount: unsafe extern "C" fn(Ref) -> isize,
    CFGetTypeID: unsafe extern "C" fn(Ref) -> usize,
    CFStringCreateWithBytes: unsafe extern "C" fn(Ref, *const u8, isize, u32, u8) -> Ref,
    CFStringGetLength: unsafe extern "C" fn(Ref) -> isize,
    CFStringGetMaximumSizeForEncoding: unsafe extern "C" fn(isize, u32) -> isize,
    CFStringGetCString: unsafe extern "C" fn(Ref, *mut u8, isize, u32) -> u8,
    CFNumberCreate: unsafe extern "C" fn(Ref, isize, *const c_void) -> Ref,
    CFNumberGetValue: unsafe extern "C" fn(Ref, isize, *mut c_void) -> u8,
    CFNumberIsFloatType: unsafe extern "C" fn(Ref) -> u8,
    CFBooleanGetValue: unsafe extern "C" fn(Ref) -> u8,
    CFDataCreate: unsafe extern "C" fn(Ref, *const u8, isize) -> Ref,
    CFDataGetLength: unsafe extern "C" fn(Ref) -> isize,
    CFDataGetBytePtr: unsafe extern "C" fn(Ref) -> *const u8,
    CFArrayCreate: unsafe extern "C" fn(Ref, *const Ref, isize, *const c_void) -> Ref,
    CFArrayGetCount: unsafe extern "C" fn(Ref) -> isize,
    CFArrayGetValueAtIndex: unsafe extern "C" fn(Ref, isize) -> Ref,
    CFDictionaryCreate: unsafe extern "C" fn(Ref, *const Ref, *const Ref, isize, *const c_void, *const c_void) -> Ref,
    CFDictionaryGetCount: unsafe extern "C" fn(Ref) -> isize,
    CFDictionaryGetKeysAndValues: unsafe extern "C" fn(Ref, *mut Ref, *mut Ref),
}

struct TypeIds {
    string: usize,
    number: usize,
    boolean: usize,
    data: usize,
    array: usize,
    dict: usize,
}

pub struct Cf {
    f: Fns,
    ids: TypeIds,
    bool_true: Ref,
    bool_false: Ref,
    array_callbacks: *const c_void,
    dict_key_callbacks: *const c_void,
    dict_value_callbacks: *const c_void,
    _lib: Library,
}

// SAFETY: the pointers are immutable CF constants and callback tables inside the loaded DLL,
// which `_lib` keeps alive; CF's create/read functions used here are thread-safe.
unsafe impl Send for Cf {}
unsafe impl Sync for Cf {}

macro_rules! sym {
    ($lib:expr, $name:ident) => {
        // SAFETY: the type matches CoreFoundation's documented prototype.
        *unsafe { $lib.get(stringify!($name).as_bytes()) }.map_err(|_| CfError::Symbol(stringify!($name).into()))?
    };
}

fn data_symbol(lib: &Library, name: &str) -> Result<*const c_void> {
    // SAFETY: we only take the address of an exported variable.
    unsafe { lib.get::<*const c_void>(name.as_bytes()) }.map(|s| *s).map_err(|_| CfError::Symbol(name.into()))
}

impl Cf {
    pub fn new(lib: Library) -> Result<Self> {
        let f = Fns {
            CFRelease: sym!(lib, CFRelease),
            CFRetain: sym!(lib, CFRetain),
            CFGetRetainCount: sym!(lib, CFGetRetainCount),
            CFGetTypeID: sym!(lib, CFGetTypeID),
            CFStringCreateWithBytes: sym!(lib, CFStringCreateWithBytes),
            CFStringGetLength: sym!(lib, CFStringGetLength),
            CFStringGetMaximumSizeForEncoding: sym!(lib, CFStringGetMaximumSizeForEncoding),
            CFStringGetCString: sym!(lib, CFStringGetCString),
            CFNumberCreate: sym!(lib, CFNumberCreate),
            CFNumberGetValue: sym!(lib, CFNumberGetValue),
            CFNumberIsFloatType: sym!(lib, CFNumberIsFloatType),
            CFBooleanGetValue: sym!(lib, CFBooleanGetValue),
            CFDataCreate: sym!(lib, CFDataCreate),
            CFDataGetLength: sym!(lib, CFDataGetLength),
            CFDataGetBytePtr: sym!(lib, CFDataGetBytePtr),
            CFArrayCreate: sym!(lib, CFArrayCreate),
            CFArrayGetCount: sym!(lib, CFArrayGetCount),
            CFArrayGetValueAtIndex: sym!(lib, CFArrayGetValueAtIndex),
            CFDictionaryCreate: sym!(lib, CFDictionaryCreate),
            CFDictionaryGetCount: sym!(lib, CFDictionaryGetCount),
            CFDictionaryGetKeysAndValues: sym!(lib, CFDictionaryGetKeysAndValues),
        };
        let type_id = |name: &str| -> Result<usize> {
            // SAFETY: `CF*GetTypeID` take no arguments and return the class id.
            let get: unsafe extern "C" fn() -> usize =
                *unsafe { lib.get(name.as_bytes()) }.map_err(|_| CfError::Symbol(name.into()))?;
            Ok(unsafe { get() })
        };
        let ids = TypeIds {
            string: type_id("CFStringGetTypeID")?,
            number: type_id("CFNumberGetTypeID")?,
            boolean: type_id("CFBooleanGetTypeID")?,
            data: type_id("CFDataGetTypeID")?,
            array: type_id("CFArrayGetTypeID")?,
            dict: type_id("CFDictionaryGetTypeID")?,
        };
        // SAFETY: kCFBooleanTrue/False are exported `const CFBooleanRef` variables.
        let bool_true = unsafe { *(data_symbol(&lib, "kCFBooleanTrue")? as *const Ref) };
        let bool_false = unsafe { *(data_symbol(&lib, "kCFBooleanFalse")? as *const Ref) };
        Ok(Self {
            array_callbacks: data_symbol(&lib, "kCFTypeArrayCallBacks")?,
            dict_key_callbacks: data_symbol(&lib, "kCFTypeDictionaryKeyCallBacks")?,
            dict_value_callbacks: data_symbol(&lib, "kCFTypeDictionaryValueCallBacks")?,
            f,
            ids,
            bool_true,
            bool_false,
            _lib: lib,
        })
    }

    fn owned(&self, ptr: Ref, what: &'static str) -> Result<CfRef> {
        if ptr.is_null() {
            Err(CfError::CreateFailed(what))
        } else {
            Ok(CfRef { ptr, release: self.f.CFRelease })
        }
    }

    /// Wrap a +1 pointer returned by an Apple "Create"/"Copy" function so it gets released.
    pub fn adopt(&self, ptr: Ref) -> Result<CfRef> {
        self.owned(ptr, "adopted object")
    }

    pub fn to_cf(&self, v: &CfValue) -> Result<CfRef> {
        // SAFETY (whole match): arguments follow the CF prototypes; buffers outlive the calls.
        unsafe {
            match v {
                CfValue::String(s) => self.owned(
                    (self.f.CFStringCreateWithBytes)(std::ptr::null(), s.as_ptr(), s.len() as isize, UTF8, 0),
                    "CFStringCreateWithBytes",
                ),
                CfValue::Int(n) => self.owned(
                    (self.f.CFNumberCreate)(std::ptr::null(), SINT64, n as *const i64 as *const c_void),
                    "CFNumberCreate",
                ),
                CfValue::Float(x) => self.owned(
                    (self.f.CFNumberCreate)(std::ptr::null(), FLOAT64, x as *const f64 as *const c_void),
                    "CFNumberCreate",
                ),
                CfValue::Bool(b) => {
                    let constant = if *b { self.bool_true } else { self.bool_false };
                    self.owned((self.f.CFRetain)(constant), "CFRetain")
                }
                CfValue::Data(d) => {
                    self.owned((self.f.CFDataCreate)(std::ptr::null(), d.as_ptr(), d.len() as isize), "CFDataCreate")
                }
                CfValue::Array(items) => {
                    let children = items.iter().map(|i| self.to_cf(i)).collect::<Result<Vec<_>>>()?;
                    let ptrs: Vec<Ref> = children.iter().map(CfRef::as_ptr).collect();
                    self.owned(
                        (self.f.CFArrayCreate)(std::ptr::null(), ptrs.as_ptr(), ptrs.len() as isize, self.array_callbacks),
                        "CFArrayCreate",
                    )
                }
                CfValue::Dict(map) => {
                    let keys = map.keys().map(|k| self.to_cf(&CfValue::String(k.clone()))).collect::<Result<Vec<_>>>()?;
                    let vals = map.values().map(|v| self.to_cf(v)).collect::<Result<Vec<_>>>()?;
                    let kp: Vec<Ref> = keys.iter().map(CfRef::as_ptr).collect();
                    let vp: Vec<Ref> = vals.iter().map(CfRef::as_ptr).collect();
                    self.owned(
                        (self.f.CFDictionaryCreate)(
                            std::ptr::null(),
                            kp.as_ptr(),
                            vp.as_ptr(),
                            kp.len() as isize,
                            self.dict_key_callbacks,
                            self.dict_value_callbacks,
                        ),
                        "CFDictionaryCreate",
                    )
                }
            }
        }
    }

    /// Read a CF object we don't own (no retain/release).
    pub fn from_cf(&self, ptr: Ref) -> Result<CfValue> {
        if ptr.is_null() {
            return Err(CfError::Null);
        }
        // SAFETY (whole block): `ptr` is a live CF object of the type we check before each call.
        unsafe {
            let id = (self.f.CFGetTypeID)(ptr);
            if id == self.ids.string {
                self.read_string(ptr).map(CfValue::String)
            } else if id == self.ids.boolean {
                Ok(CfValue::Bool((self.f.CFBooleanGetValue)(ptr) != 0))
            } else if id == self.ids.number {
                if (self.f.CFNumberIsFloatType)(ptr) != 0 {
                    let mut x = 0f64;
                    (self.f.CFNumberGetValue)(ptr, FLOAT64, &mut x as *mut f64 as *mut c_void);
                    Ok(CfValue::Float(x))
                } else {
                    let mut n = 0i64;
                    (self.f.CFNumberGetValue)(ptr, SINT64, &mut n as *mut i64 as *mut c_void);
                    Ok(CfValue::Int(n))
                }
            } else if id == self.ids.data {
                let len = (self.f.CFDataGetLength)(ptr) as usize;
                let bytes = (self.f.CFDataGetBytePtr)(ptr);
                Ok(CfValue::Data(if len == 0 { Vec::new() } else { std::slice::from_raw_parts(bytes, len).to_vec() }))
            } else if id == self.ids.array {
                let n = (self.f.CFArrayGetCount)(ptr);
                (0..n).map(|i| self.from_cf((self.f.CFArrayGetValueAtIndex)(ptr, i))).collect::<Result<_>>().map(CfValue::Array)
            } else if id == self.ids.dict {
                let n = (self.f.CFDictionaryGetCount)(ptr) as usize;
                let mut keys = vec![std::ptr::null(); n];
                let mut vals = vec![std::ptr::null(); n];
                (self.f.CFDictionaryGetKeysAndValues)(ptr, keys.as_mut_ptr(), vals.as_mut_ptr());
                let mut map = BTreeMap::new();
                for (k, v) in keys.into_iter().zip(vals) {
                    let CfValue::String(key) = self.from_cf(k)? else { return Err(CfError::NonStringKey) };
                    map.insert(key, self.from_cf(v)?);
                }
                Ok(CfValue::Dict(map))
            } else {
                Err(CfError::Unsupported(id))
            }
        }
    }

    unsafe fn read_string(&self, ptr: Ref) -> Result<String> {
        let len = (self.f.CFStringGetLength)(ptr);
        let cap = (self.f.CFStringGetMaximumSizeForEncoding)(len, UTF8) + 1;
        let mut buf = vec![0u8; cap as usize];
        if (self.f.CFStringGetCString)(ptr, buf.as_mut_ptr(), cap, UTF8) == 0 {
            return Err(CfError::CreateFailed("CFStringGetCString"));
        }
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        buf.truncate(end);
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }

    pub fn retain_count(&self, ptr: Ref) -> isize {
        // SAFETY: `ptr` is a live CF object.
        unsafe { (self.f.CFGetRetainCount)(ptr) }
    }

    /// Borrowed element `i` of a CFArray.
    pub fn array_item(&self, arr: Ref, i: isize) -> Result<Ref> {
        // SAFETY: caller passes a live CFArray; index is bounds-checked first.
        unsafe {
            if (self.f.CFGetTypeID)(arr) != self.ids.array || i < 0 || i >= (self.f.CFArrayGetCount)(arr) {
                return Err(CfError::Null);
            }
            Ok((self.f.CFArrayGetValueAtIndex)(arr, i))
        }
    }
}

// Machine tests: they need Apple's CoreFoundation.dll, so they're #[ignore] and run with
// `cargo test -- --include-ignored` on a PC with iTunes / Apple Devices installed.
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::OnceLock;

    fn cf() -> &'static Cf {
        static CF: OnceLock<Cf> = OnceLock::new();
        CF.get_or_init(|| {
            let dlls = crate::apple::loader::load_from_system().expect("Apple DLLs");
            Cf::new(dlls.core_foundation).expect("CoreFoundation symbols")
        })
    }

    fn round_trip(v: CfValue) -> CfValue {
        let obj = cf().to_cf(&v).unwrap();
        cf().from_cf(obj.as_ptr()).unwrap()
    }

    fn dict(pairs: &[(&str, CfValue)]) -> CfValue {
        CfValue::Dict(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect::<BTreeMap<_, _>>())
    }

    #[test]
    #[ignore]
    fn strings_round_trip_including_vietnamese_and_cjk() {
        for s in ["", "SyncAllowed", "Mùa thu cho em", "Nhạc Trịnh", "日本語のタイトル", "emoji 🎵"] {
            assert_eq!(round_trip(CfValue::String(s.into())), CfValue::String(s.into()));
        }
    }

    #[test]
    #[ignore]
    fn integers_round_trip_at_the_extremes() {
        for n in [0, 1, -1, 69, i64::MAX, i64::MIN] {
            assert_eq!(round_trip(CfValue::Int(n)), CfValue::Int(n));
        }
    }

    #[test]
    #[ignore]
    fn floats_and_booleans_round_trip() {
        assert_eq!(round_trip(CfValue::Float(252.5)), CfValue::Float(252.5));
        assert_eq!(round_trip(CfValue::Bool(true)), CfValue::Bool(true));
        assert_eq!(round_trip(CfValue::Bool(false)), CfValue::Bool(false));
    }

    #[test]
    #[ignore]
    fn data_round_trips_with_zero_bytes() {
        let bytes = vec![0x04, 0x00, 0xff, 0x00, 0x10];
        assert_eq!(round_trip(CfValue::Data(bytes.clone())), CfValue::Data(bytes));
    }

    #[test]
    #[ignore]
    fn nested_arrays_and_dictionaries_round_trip() {
        let v = dict(&[
            ("Dataclasses", CfValue::Array(vec![CfValue::String("Media".into()), CfValue::String("Keybag".into())])),
            ("Anchors", dict(&[("Media", CfValue::Int(0))])),
            ("Tên bài", CfValue::String("Diễm xưa".into())),
            ("Nested", CfValue::Array(vec![dict(&[("ok", CfValue::Bool(true))]), CfValue::Array(vec![])])),
        ]);
        assert_eq!(round_trip(v.clone()), v);
    }

    #[test]
    #[ignore]
    fn created_objects_are_owned_once_and_children_are_held_by_their_container() {
        let arr = cf().to_cf(&CfValue::Array(vec![CfValue::String("x".into())])).unwrap();
        assert_eq!(cf().retain_count(arr.as_ptr()), 1);
        let child = cf().array_item(arr.as_ptr(), 0).unwrap();
        assert_eq!(cf().retain_count(child), 1, "child leaked an extra retain");
    }

    #[test]
    #[ignore]
    fn reading_does_not_change_the_retain_count() {
        let s = cf().to_cf(&CfValue::String("Mùa thu".into())).unwrap();
        cf().from_cf(s.as_ptr()).unwrap();
        assert_eq!(cf().retain_count(s.as_ptr()), 1);
    }

    #[test]
    #[ignore]
    fn null_pointer_is_an_error_not_a_crash() {
        assert!(cf().from_cf(std::ptr::null()).is_err());
    }
}
