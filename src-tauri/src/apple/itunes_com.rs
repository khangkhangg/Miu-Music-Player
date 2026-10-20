//! iTunes for Windows through its COM automation interface (`iTunes.Application`, late-bound
//! IDispatch). Used with "Manually manage music" on: files are added to the iTunes library, then
//! the library track is added to the phone's library playlist; `UpdateIPod` applies the changes.
//! iTunes is a packaged app whose server is not in HKCR, so it is created by CLSID.
//! Findings and observed behaviour: docs/AIRTRAFFIC_NOTES.md (2026-10-20 entries).

use std::mem::ManuallyDrop;
use std::path::Path;
use std::time::{Duration, Instant};

use windows::core::{BSTR, GUID, PCWSTR};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, IDispatch, CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, DISPATCH_FLAGS, DISPATCH_METHOD,
    DISPATCH_PROPERTYGET, DISPPARAMS,
};
use windows::Win32::System::Variant::{
    VariantChangeType, VariantClear, VARENUM, VARIANT, VAR_CHANGE_FLAGS, VT_BOOL, VT_BSTR, VT_DISPATCH, VT_EMPTY, VT_I4, VT_I8,
};

use crate::sync::itunes::{DeviceTrack, ITunes};

const CLSID_ITUNES_APP: GUID = GUID::from_u128(0xDC0C2640_1415_4644_875C_6F4D769839BA);
/// `ITSourceKind` of an iPod / iPhone.
const SOURCE_KIND_IPOD: i64 = 2;
const ADD_TIMEOUT: Duration = Duration::from_secs(120);

/// One argument to a COM call.
pub enum Arg<'a> {
    I4(i32),
    Str(&'a str),
    Disp(&'a IDispatch),
}

/// A COM result value; cleared on drop.
pub struct Var(VARIANT);

impl Drop for Var {
    fn drop(&mut self) {
        unsafe {
            let _ = VariantClear(&mut self.0);
        }
    }
}

impl Var {
    fn vt(&self) -> VARENUM {
        unsafe { self.0.Anonymous.Anonymous.vt }
    }

    pub fn is_empty(&self) -> bool {
        self.vt() == VT_EMPTY
    }

    pub fn i64(&self) -> Result<i64, String> {
        unsafe {
            let mut out = Var(std::mem::zeroed());
            VariantChangeType(&mut out.0, &self.0, VAR_CHANGE_FLAGS(0), VT_I8).map_err(|e| format!("not a number: {e}"))?;
            Ok(out.0.Anonymous.Anonymous.Anonymous.llVal)
        }
    }

    pub fn bool(&self) -> Result<bool, String> {
        if self.vt() == VT_BOOL {
            Ok(unsafe { self.0.Anonymous.Anonymous.Anonymous.boolVal.0 } != 0)
        } else {
            self.i64().map(|n| n != 0)
        }
    }

    pub fn string(&self) -> Result<String, String> {
        if self.vt() != VT_BSTR {
            return Err(format!("not a string (vt {})", self.vt().0));
        }
        Ok(unsafe { self.0.Anonymous.Anonymous.Anonymous.bstrVal.to_string() })
    }

    pub fn object(&self) -> Result<IDispatch, String> {
        if self.vt() != VT_DISPATCH {
            return Err(format!("not an object (vt {})", self.vt().0));
        }
        unsafe { (*self.0.Anonymous.Anonymous.Anonymous.pdispVal).clone() }.ok_or_else(|| "empty object".into())
    }
}

fn to_variant(a: &Arg) -> VARIANT {
    unsafe {
        let mut v: VARIANT = std::mem::zeroed();
        let inner = &mut *v.Anonymous.Anonymous;
        match a {
            Arg::I4(n) => {
                inner.vt = VT_I4;
                inner.Anonymous.lVal = *n;
            }
            Arg::Str(s) => {
                inner.vt = VT_BSTR;
                inner.Anonymous.bstrVal = ManuallyDrop::new(BSTR::from(*s));
            }
            Arg::Disp(d) => {
                inner.vt = VT_DISPATCH;
                inner.Anonymous.pdispVal = ManuallyDrop::new(Some((*d).clone()));
            }
        }
        v
    }
}

fn invoke(obj: &IDispatch, name: &str, flags: DISPATCH_FLAGS, args: &[Arg]) -> Result<Var, String> {
    unsafe {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut id = 0i32;
        obj.GetIDsOfNames(&GUID::zeroed(), &PCWSTR(wide.as_ptr()), 1, 0, &mut id).map_err(|e| format!("iTunes has no {name}: {e}"))?;
        // COM takes arguments last to first.
        let mut vars: Vec<VARIANT> = args.iter().rev().map(to_variant).collect();
        let params = DISPPARAMS { rgvarg: vars.as_mut_ptr(), rgdispidNamedArgs: std::ptr::null_mut(), cArgs: vars.len() as u32, cNamedArgs: 0 };
        let mut out = Var(std::mem::zeroed());
        let result = obj.Invoke(id, &GUID::zeroed(), 0, flags, &params, Some(&mut out.0), None, None);
        for v in &mut vars {
            let _ = VariantClear(v);
        }
        result.map_err(|e| format!("iTunes {name} failed: {e}"))?;
        Ok(out)
    }
}

pub fn get(obj: &IDispatch, name: &str) -> Result<Var, String> {
    invoke(obj, name, DISPATCH_PROPERTYGET, &[])
}

pub fn get_with(obj: &IDispatch, name: &str, args: &[Arg]) -> Result<Var, String> {
    invoke(obj, name, DISPATCH_PROPERTYGET | DISPATCH_METHOD, args)
}

pub fn call(obj: &IDispatch, name: &str, args: &[Arg]) -> Result<Var, String> {
    invoke(obj, name, DISPATCH_METHOD, args)
}

/// Items of a 1-based COM collection.
fn items(collection: &IDispatch) -> Result<Vec<IDispatch>, String> {
    let n = get(collection, "Count")?.i64()?;
    (1..=n).map(|i| get_with(collection, "Item", &[Arg::I4(i as i32)])?.object()).collect()
}

/// The running (or started) iTunes, through COM. Stays on the thread that created it.
pub struct ITunesCom {
    app: IDispatch,
}

impl ITunesCom {
    pub fn connect() -> Result<Self, String> {
        unsafe {
            // S_FALSE (already initialised) and a different apartment model both still work here.
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let app: IDispatch = CoCreateInstance(&CLSID_ITUNES_APP, None, CLSCTX_LOCAL_SERVER).map_err(|e| format!("cannot start iTunes: {e}"))?;
            Ok(Self { app })
        }
    }

    /// iTunes' persistent ID of an object, as one signed 64-bit number (the phone's sync_id).
    fn persistent_id(&self, obj: &IDispatch) -> Result<i64, String> {
        let hi = get_with(&self.app, "ITObjectPersistentIDHigh", &[Arg::Disp(obj)])?.i64()? as u32;
        let lo = get_with(&self.app, "ITObjectPersistentIDLow", &[Arg::Disp(obj)])?.i64()? as u32;
        Ok((((hi as u64) << 32) | lo as u64) as i64)
    }

    /// The phone source (kind 2). iTunes sees one iPhone at a time here.
    pub fn phone(&self) -> Result<IDispatch, String> {
        let sources = get(&self.app, "Sources")?.object()?;
        for s in items(&sources)? {
            if get(&s, "Kind")?.i64()? == SOURCE_KIND_IPOD {
                return Ok(s);
            }
        }
        Err("iTunes does not see the iPhone (is it plugged in and trusted?)".into())
    }

    pub fn phone_name(&self) -> Result<String, String> {
        get(&self.phone()?, "Name")?.string()
    }

    /// The phone's library playlist (first playlist of the source).
    fn device_library(&self) -> Result<IDispatch, String> {
        let playlists = get(&self.phone()?, "Playlists")?.object()?;
        get_with(&playlists, "Item", &[Arg::I4(1)])?.object()
    }

    fn device_track_objects(&self) -> Result<Vec<IDispatch>, String> {
        items(&get(&self.device_library()?, "Tracks")?.object()?)
    }

    fn describe(&self, t: &IDispatch) -> Result<DeviceTrack, String> {
        Ok(DeviceTrack {
            id: self.persistent_id(t)?,
            name: get(t, "Name")?.string().unwrap_or_default(),
            size: get(t, "Size")?.i64().unwrap_or(0).max(0) as u64,
            duration_ms: get(t, "Duration").ok().and_then(|v| v.i64().ok()).map(|s| s * 1000),
        })
    }
}

impl ITunes for ITunesCom {
    fn device_tracks(&self) -> Result<Vec<DeviceTrack>, String> {
        self.device_track_objects()?.iter().map(|t| self.describe(t)).collect()
    }

    fn add(&self, path: &Path) -> Result<i64, String> {
        let before: std::collections::HashSet<i64> = self.device_tracks()?.into_iter().map(|t| t.id).collect();
        // 1. Into the iTunes library (iTunes' own drag-and-drop path; AddFile on the device playlist
        //    returns nothing with manual management, see the notes).
        let library = get(&self.app, "LibraryPlaylist")?.object()?;
        let status = call(&library, "AddFile", &[Arg::Str(&path.to_string_lossy())])?;
        if status.is_empty() {
            return Err(format!("iTunes did not accept {}", path.display()));
        }
        let status = status.object()?;
        let started = Instant::now();
        while get(&status, "InProgress")?.bool()? {
            if started.elapsed() > ADD_TIMEOUT {
                return Err(format!("iTunes took too long adding {}", path.display()));
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let tracks = get(&status, "Tracks")?.object()?;
        let track = items(&tracks)?.into_iter().next().ok_or_else(|| format!("iTunes added nothing for {}", path.display()))?;
        // 2. Onto the phone.
        call(&self.device_library()?, "AddTrack", &[Arg::Disp(&track)])?;
        let added: Vec<i64> = self.device_tracks()?.into_iter().map(|t| t.id).filter(|id| !before.contains(id)).collect();
        match added.as_slice() {
            [id] => Ok(*id),
            [] => Err(format!("iTunes did not put {} on the phone", path.display())),
            _ => Err(format!("iTunes added {} tracks for {}", added.len(), path.display())),
        }
    }

    fn remove(&self, id: i64) -> Result<bool, String> {
        for t in self.device_track_objects()? {
            if self.persistent_id(&t)? == id {
                call(&t, "Delete", &[])?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn sync_device(&self) -> Result<(), String> {
        call(&self.phone()?, "UpdateIPod", &[]).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read-only: needs iTunes installed and the iPhone plugged in.
    /// `cargo test --lib itunes_com -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn lists_the_phones_tracks_with_persistent_ids() {
        let it = ITunesCom::connect().unwrap();
        println!("phone: {}", it.phone_name().unwrap());
        let tracks = it.device_tracks().unwrap();
        println!("{} tracks; first: {:?}", tracks.len(), tracks.first());
        assert!(!tracks.is_empty());
        assert!(tracks.iter().all(|t| t.id != 0 && t.size > 0));
        let ids: std::collections::HashSet<i64> = tracks.iter().map(|t| t.id).collect();
        assert_eq!(ids.len(), tracks.len(), "persistent IDs are unique");
    }
}
