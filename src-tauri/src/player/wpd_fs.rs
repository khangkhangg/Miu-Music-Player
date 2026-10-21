//! `PlayerFs` over one storage of an MTP device (Android phone, Sony Walkman, newer SanDisk) through
//! Windows Portable Devices. Paths are relative to the storage (`Music\Artist\Album\03 Title.mp3`)
//! and are resolved to WPD object ids folder by folder, with the listings cached for the run.
//! MTP has no dependable rename, so `can_rename` is false and the sync writes straight to the name.
//! Findings from real devices belong in docs/MTP.md.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Component, Path};

use windows::core::{HSTRING, PWSTR};
use windows::Win32::Devices::PortableDevices::*;
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, IStream, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, STGC_DEFAULT};
use windows::core::Interface;

use crate::player::sync::PlayerFs;

/// A child of a folder on the device.
#[derive(Clone)]
struct Entry {
    id: String,
    folder: bool,
}

pub struct WpdFs {
    device: IPortableDevice,
    content: IPortableDeviceContent,
    props: IPortableDeviceProperties,
    storage: String,
    /// Folder object id → its children by lower-cased name.
    listings: RefCell<HashMap<String, HashMap<String, Entry>>>,
}

fn take(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: WPD returns NUL-terminated strings allocated with CoTaskMemAlloc; freed once here.
    let s = unsafe { p.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
    s
}

fn err(what: &str) -> impl Fn(windows::core::Error) -> String + '_ {
    move |e| format!("{what}: {e}")
}

fn format_guid(ext: &str) -> windows::core::GUID {
    match crate::player::mtp::format_for(ext) {
        "mp3" => WPD_OBJECT_FORMAT_MP3,
        "m4a" => WPD_OBJECT_FORMAT_MP4,
        "aac" => WPD_OBJECT_FORMAT_AAC,
        "flac" => WPD_OBJECT_FORMAT_FLAC,
        "wav" => WPD_OBJECT_FORMAT_WAVE,
        _ => WPD_OBJECT_FORMAT_UNSPECIFIED,
    }
}

fn names(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

impl WpdFs {
    /// Open the device at `device_path` (from `mtp::probe`) and work in the storage `storage_object`.
    pub fn open(device_path: &str, storage_object: &str) -> Result<Self, String> {
        // SAFETY: COM calls on interfaces WPD hands back; COM is initialised for this thread first.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let client: IPortableDeviceValues = CoCreateInstance(&PortableDeviceValues, None, CLSCTX_INPROC_SERVER).map_err(err("WPD"))?;
            client.SetStringValue(&WPD_CLIENT_NAME, &HSTRING::from("Miu Music")).map_err(err("WPD"))?;
            client.SetUnsignedIntegerValue(&WPD_CLIENT_MAJOR_VERSION, 0).map_err(err("WPD"))?;
            let device: IPortableDevice = CoCreateInstance(&PortableDeviceFTM, None, CLSCTX_INPROC_SERVER).map_err(err("WPD"))?;
            device
                .Open(&HSTRING::from(device_path), &client)
                .map_err(|e| format!("could not open the device (is it unlocked and set to File transfer?): {e}"))?;
            let content = device.Content().map_err(err("WPD content"))?;
            let props = content.Properties().map_err(err("WPD properties"))?;
            Ok(Self { device, content, props, storage: storage_object.to_string(), listings: RefCell::new(HashMap::new()) })
        }
    }

    fn values(&self, object: &str, keys: &[PROPERTYKEY]) -> Option<IPortableDeviceValues> {
        // SAFETY: plain COM calls on live interfaces.
        unsafe {
            let coll: IPortableDeviceKeyCollection = CoCreateInstance(&PortableDeviceKeyCollection, None, CLSCTX_INPROC_SERVER).ok()?;
            for k in keys {
                coll.Add(k).ok()?;
            }
            self.props.GetValues(&HSTRING::from(object), &coll).ok()
        }
    }

    /// The children of a folder object, listed once per run.
    fn children(&self, parent: &str) -> HashMap<String, Entry> {
        if let Some(found) = self.listings.borrow().get(parent) {
            return found.clone();
        }
        let mut out = HashMap::new();
        // SAFETY: plain COM calls on live interfaces; every returned string is freed by `take`.
        unsafe {
            if let Ok(ids) = self.content.EnumObjects(0, &HSTRING::from(parent), None) {
                loop {
                    let mut batch = [PWSTR::null(); 32];
                    let mut fetched = 0u32;
                    let hr = ids.Next(&mut batch, &mut fetched);
                    for p in &batch[..fetched as usize] {
                        let id = take(*p);
                        let Some(v) = self.values(&id, &[WPD_OBJECT_ORIGINAL_FILE_NAME, WPD_OBJECT_NAME, WPD_OBJECT_CONTENT_TYPE]) else { continue };
                        let mut name = v.GetStringValue(&WPD_OBJECT_ORIGINAL_FILE_NAME).map(take).unwrap_or_default();
                        if name.is_empty() {
                            name = v.GetStringValue(&WPD_OBJECT_NAME).map(take).unwrap_or_default();
                        }
                        let folder = matches!(v.GetGuidValue(&WPD_OBJECT_CONTENT_TYPE), Ok(g) if g == WPD_CONTENT_TYPE_FOLDER || g == WPD_CONTENT_TYPE_FUNCTIONAL_OBJECT);
                        out.insert(name.to_lowercase(), Entry { id, folder });
                    }
                    if hr.is_err() || fetched == 0 || hr.0 == 1 {
                        break; // S_FALSE: no more
                    }
                }
            }
        }
        self.listings.borrow_mut().insert(parent.to_string(), out.clone());
        out
    }

    fn resolve(&self, path: &Path) -> Option<Entry> {
        let mut at = Entry { id: self.storage.clone(), folder: true };
        for name in names(path) {
            if !at.folder {
                return None;
            }
            at = self.children(&at.id).get(&name.to_lowercase())?.clone();
        }
        Some(at)
    }

    fn remember(&self, parent: &str, name: &str, entry: Option<Entry>) {
        if let Some(list) = self.listings.borrow_mut().get_mut(parent) {
            match entry {
                Some(e) => list.insert(name.to_lowercase(), e),
                None => list.remove(&name.to_lowercase()),
            };
        }
    }

    fn delete(&self, id: &str) -> Result<(), String> {
        // SAFETY: plain COM calls; the id is wrapped in a PROPVARIANT by WPD itself (via a values bag).
        unsafe {
            let bag: IPortableDeviceValues = CoCreateInstance(&PortableDeviceValues, None, CLSCTX_INPROC_SERVER).map_err(err("WPD"))?;
            bag.SetStringValue(&WPD_OBJECT_ID, &HSTRING::from(id)).map_err(err("WPD"))?;
            let pv = bag.GetValue(&WPD_OBJECT_ID).map_err(err("WPD"))?;
            let ids: IPortableDevicePropVariantCollection = CoCreateInstance(&PortableDevicePropVariantCollection, None, CLSCTX_INPROC_SERVER).map_err(err("WPD"))?;
            ids.Add(&pv).map_err(err("WPD"))?;
            self.content.Delete(PORTABLE_DEVICE_DELETE_NO_RECURSION.0 as u32, &ids, std::ptr::null_mut()).map_err(err("deleting on the device"))
        }
    }

    fn parent_of(&self, path: &Path) -> Result<(Entry, String), String> {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or("no file name")?;
        let parent = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => self.resolve(p).filter(|e| e.folder).ok_or("the folder is not on the device")?,
            _ => Entry { id: self.storage.clone(), folder: true },
        };
        Ok((parent, name))
    }
}

impl Drop for WpdFs {
    fn drop(&mut self) {
        // SAFETY: closing the session this value opened.
        let _ = unsafe { self.device.Close() };
    }
}

impl PlayerFs for WpdFs {
    fn len(&self, path: &Path) -> Option<u64> {
        let e = self.resolve(path).filter(|e| !e.folder)?;
        // SAFETY: plain COM call on a live interface.
        self.values(&e.id, &[WPD_OBJECT_SIZE]).and_then(|v| unsafe { v.GetUnsignedLargeIntegerValue(&WPD_OBJECT_SIZE) }.ok())
    }

    fn copy_in(&self, src: &Path, dst: &Path) -> Result<u64, String> {
        let (parent, name) = self.parent_of(dst)?;
        let size = std::fs::metadata(src).map_err(|e| format!("cannot read the song: {e}"))?.len();
        let ext = dst.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
        let stem = dst.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| name.clone());
        // SAFETY: COM calls on live interfaces; the stream is written then committed, once.
        unsafe {
            let v: IPortableDeviceValues = CoCreateInstance(&PortableDeviceValues, None, CLSCTX_INPROC_SERVER).map_err(err("WPD"))?;
            v.SetStringValue(&WPD_OBJECT_PARENT_ID, &HSTRING::from(parent.id.as_str())).map_err(err("WPD"))?;
            v.SetStringValue(&WPD_OBJECT_ORIGINAL_FILE_NAME, &HSTRING::from(name.as_str())).map_err(err("WPD"))?;
            v.SetStringValue(&WPD_OBJECT_NAME, &HSTRING::from(stem.as_str())).map_err(err("WPD"))?;
            v.SetUnsignedLargeIntegerValue(&WPD_OBJECT_SIZE, size).map_err(err("WPD"))?;
            v.SetGuidValue(&WPD_OBJECT_CONTENT_TYPE, &WPD_CONTENT_TYPE_AUDIO).map_err(err("WPD"))?;
            v.SetGuidValue(&WPD_OBJECT_FORMAT, &format_guid(&ext)).map_err(err("WPD"))?;
            let mut stream: Option<IStream> = None;
            let mut chunk = 0u32;
            self.content
                .CreateObjectWithPropertiesAndData(&v, &mut stream, &mut chunk, std::ptr::null_mut())
                .map_err(err("the device refused the file"))?;
            let stream = stream.ok_or("the device gave no stream to write to")?;
            let mut file = std::fs::File::open(src).map_err(|e| format!("cannot read the song: {e}"))?;
            let mut buf = vec![0u8; (chunk as usize).clamp(64 * 1024, 4 * 1024 * 1024)];
            let mut sent = 0u64;
            loop {
                let n = file.read(&mut buf).map_err(|e| format!("cannot read the song: {e}"))?;
                if n == 0 {
                    break;
                }
                let mut written = 0u32;
                stream.Write(buf.as_ptr() as *const _, n as u32, Some(&mut written)).ok().map_err(err("writing to the device"))?;
                if written as usize != n {
                    return Err("the device stopped taking data".into());
                }
                sent += n as u64;
            }
            stream.Commit(STGC_DEFAULT).map_err(err("finishing the file on the device"))?;
            let id = stream.cast::<IPortableDeviceDataStream>().and_then(|d| d.GetObjectID()).map(take).unwrap_or_default();
            // A fresh listing next time if the id did not come back.
            if id.is_empty() {
                self.listings.borrow_mut().remove(&parent.id);
            } else {
                self.remember(&parent.id, &name, Some(Entry { id, folder: false }));
            }
            Ok(sent)
        }
    }

    fn rename(&self, _from: &Path, _to: &Path) -> Result<(), String> {
        Err("files cannot be renamed on this device".into())
    }

    fn remove(&self, path: &Path) -> Result<(), String> {
        let (parent, name) = self.parent_of(path)?;
        let e = self.resolve(path).ok_or("no such file on the device")?;
        self.delete(&e.id)?;
        self.remember(&parent.id, &name, None);
        Ok(())
    }

    fn make_dirs(&self, dir: &Path) -> Result<(), String> {
        let mut at = Entry { id: self.storage.clone(), folder: true };
        for name in names(dir) {
            if let Some(found) = self.children(&at.id).get(&name.to_lowercase()) {
                if !found.folder {
                    return Err(format!("{name} is a file on the device, not a folder"));
                }
                at = found.clone();
                continue;
            }
            // SAFETY: COM calls on live interfaces; the new id is freed by `take`.
            let id = unsafe {
                let v: IPortableDeviceValues = CoCreateInstance(&PortableDeviceValues, None, CLSCTX_INPROC_SERVER).map_err(err("WPD"))?;
                v.SetStringValue(&WPD_OBJECT_PARENT_ID, &HSTRING::from(at.id.as_str())).map_err(err("WPD"))?;
                v.SetStringValue(&WPD_OBJECT_NAME, &HSTRING::from(name.as_str())).map_err(err("WPD"))?;
                v.SetStringValue(&WPD_OBJECT_ORIGINAL_FILE_NAME, &HSTRING::from(name.as_str())).map_err(err("WPD"))?;
                v.SetGuidValue(&WPD_OBJECT_CONTENT_TYPE, &WPD_CONTENT_TYPE_FOLDER).map_err(err("WPD"))?;
                v.SetGuidValue(&WPD_OBJECT_FORMAT, &WPD_OBJECT_FORMAT_PROPERTIES_ONLY).map_err(err("WPD"))?;
                let mut out = PWSTR::null();
                self.content.CreateObjectWithPropertiesOnly(&v, &mut out).map_err(err("the device refused the folder"))?;
                take(out)
            };
            let entry = Entry { id, folder: true };
            self.remember(&at.id, &name, Some(entry.clone()));
            self.listings.borrow_mut().insert(entry.id.clone(), HashMap::new());
            at = entry;
        }
        Ok(())
    }

    fn remove_empty_dir(&self, dir: &Path) -> bool {
        let Ok((parent, name)) = self.parent_of(dir) else { return false };
        let Some(e) = self.resolve(dir).filter(|e| e.folder) else { return false };
        // Ask the device again: something else may have put files there.
        self.listings.borrow_mut().remove(&e.id);
        if !self.children(&e.id).is_empty() || self.delete(&e.id).is_err() {
            return false;
        }
        self.remember(&parent.id, &name, None);
        self.listings.borrow_mut().remove(&e.id);
        true
    }

    fn free_bytes(&self) -> Option<u64> {
        // SAFETY: plain COM call on a live interface.
        self.values(&self.storage, &[WPD_STORAGE_FREE_SPACE_IN_BYTES]).and_then(|v| unsafe { v.GetUnsignedLargeIntegerValue(&WPD_STORAGE_FREE_SPACE_IN_BYTES) }.ok())
    }

    fn can_rename(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_split_into_folder_names() {
        assert_eq!(names(Path::new(r"Music\Khánh Ly\Tình ca\03 Diễm xưa.mp3")), vec!["Music", "Khánh Ly", "Tình ca", "03 Diễm xưa.mp3"]);
        assert_eq!(names(Path::new("Music/a.mp3")), vec!["Music", "a.mp3"]);
        assert!(names(Path::new("")).is_empty());
    }

    /// Needs an MTP device (phone unlocked, File transfer). Writes a few KB into a `MiuTest-…`
    /// folder on its first storage, checks the size, and deletes it all again.
    /// `cargo test --lib wpd_fs -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn writes_checks_and_cleans_up_on_a_real_mtp_device() {
        let devices = crate::player::mtp::probe().unwrap();
        let Some((device, storage)) = devices
            .iter()
            .filter(|d| crate::player::mtp::kind(&d.protocol, &d.manufacturer) == crate::player::mtp::Kind::Player)
            .find_map(|d| d.storages.first().map(|s| (d, s)))
        else {
            println!("no MTP device with storage plugged in; skipped");
            return;
        };
        println!("testing on {} — {}", device.name, storage.name);
        let fs = WpdFs::open(&device.id, &storage.object_id).unwrap();
        println!("free: {:?}", fs.free_bytes());
        let dir = tempfile::tempdir().unwrap();
        let wav = crate::audio::fixtures::wav(dir.path(), "t.wav", 44_100, 2, 16, 4_410);
        let size = std::fs::metadata(&wav).unwrap().len();
        let folder = std::path::PathBuf::from(format!("MiuTest-{}", crate::util::time::now_unix())).join("Khánh Ly");
        fs.make_dirs(&folder).unwrap();
        let file = folder.join("01 Diễm xưa.wav");
        assert_eq!(fs.copy_in(&wav, &file).unwrap(), size);
        let fresh = WpdFs::open(&device.id, &storage.object_id).unwrap();
        println!("size reported after commit: {:?} (sent {size})", fresh.len(&file));
        assert_eq!(fresh.len(&file), Some(size), "the device reports the size straight after Commit");
        fresh.remove(&file).unwrap();
        assert_eq!(fresh.len(&file), None);
        assert!(fresh.remove_empty_dir(&folder));
        assert!(fresh.remove_empty_dir(folder.parent().unwrap()));
    }
}
