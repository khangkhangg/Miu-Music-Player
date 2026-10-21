//! The mounted volumes, from Windows (drive letters, labels, serial numbers, space).

use std::path::PathBuf;

use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW};
use windows_sys::Win32::System::Diagnostics::Debug::{SetThreadErrorMode, SEM_FAILCRITICALERRORS};

use crate::player::drives::Volume;

const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Free bytes on the drive that holds `root` (`E:\`).
pub fn free_bytes(root: &std::path::Path) -> Option<u64> {
    let root = wide(&root.to_string_lossy());
    let mut free = 0u64;
    // SAFETY: `root` is a NUL-terminated wide string; the out pointer is valid for the call.
    let ok = unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
    (ok != 0).then_some(free)
}

fn volume(letter: char) -> Option<Volume> {
    let root = format!("{letter}:\\");
    let w = wide(&root);
    // SAFETY: NUL-terminated wide string.
    let kind = unsafe { GetDriveTypeW(w.as_ptr()) };
    if kind != DRIVE_REMOVABLE && kind != DRIVE_FIXED {
        return None;
    }
    let mut label = [0u16; 261];
    let mut serial = 0u32;
    // SAFETY: buffers are valid for their stated lengths; unused outputs are null.
    let ok = unsafe {
        GetVolumeInformationW(w.as_ptr(), label.as_mut_ptr(), label.len() as u32, &mut serial, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), 0)
    };
    if ok == 0 {
        return None; // a card reader with no card in it
    }
    let (mut free, mut total) = (0u64, 0u64);
    // SAFETY: as above.
    unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut free, &mut total, std::ptr::null_mut()) };
    let len = label.iter().position(|c| *c == 0).unwrap_or(0);
    Some(Volume { root: PathBuf::from(root), label: String::from_utf16_lossy(&label[..len]), serial, removable: kind == DRIVE_REMOVABLE, total_bytes: total, free_bytes: free })
}

/// Removable and fixed volumes with media, except the one Windows runs from.
pub fn list() -> Vec<Volume> {
    let system = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()).to_ascii_uppercase();
    let mut old = 0;
    // SAFETY: plain calls; empty card readers then fail quietly instead of showing "Insert a disk".
    unsafe { SetThreadErrorMode(SEM_FAILCRITICALERRORS, &mut old) };
    // SAFETY: no arguments.
    let mask = unsafe { GetLogicalDrives() };
    let out = (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| (b'A' + i) as char)
        .filter(|l| !system.starts_with(*l))
        .filter_map(volume)
        .collect();
    // SAFETY: restores the mode saved above.
    unsafe { SetThreadErrorMode(old, std::ptr::null_mut()) };
    out
}

/// The drive letters present now: cheap enough to poll every couple of seconds.
pub fn letters() -> u32 {
    // SAFETY: no arguments.
    unsafe { GetLogicalDrives() }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lists this PC's drives. Plug in a USB stick to see it marked removable.
    /// `cargo test --lib player::volumes -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn lists_the_drives_of_this_pc() {
        let vols = list();
        for v in &vols {
            println!("{} {:?} serial {:08X} removable {} {} / {} MB free", v.root.display(), v.label, v.serial, v.removable, v.free_bytes / 1_000_000, v.total_bytes / 1_000_000);
            assert!(v.total_bytes >= v.free_bytes);
            assert_eq!(free_bytes(&v.root).map(|f| f / 1_000_000_000), Some(v.free_bytes / 1_000_000_000));
        }
        assert!(vols.iter().all(|v| !v.root.to_string_lossy().eq_ignore_ascii_case("C:\\")), "the system drive is never offered");
    }
}
