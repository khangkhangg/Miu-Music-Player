//! Windows side of `taskbar`: the ITaskbarList3 thumbnail toolbar on the main window. Everything
//! here runs on the window's UI thread (Tauri's main thread), so its state is thread-local.

use std::cell::RefCell;
use std::sync::OnceLock;

use ::windows::core::w;
use ::windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use ::windows::Win32::Graphics::Gdi::{CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ};
use ::windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use ::windows::Win32::UI::Shell::{
    DefSubclassProc, ITaskbarList3, SetWindowSubclass, TaskbarList, THBF_ENABLED, THBN_CLICKED, THB_FLAGS, THB_ICON, THB_TOOLTIP, THUMBBUTTON,
};
use ::windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, GetSystemMetrics, RegisterWindowMessageW, HICON, ICONINFO, SM_CXSMICON, WM_COMMAND};
use tauri::{AppHandle, Emitter};

use crate::taskbar::{button_name, glyph, Glyph, BUTTONS};

/// Tooltips, in the UI language.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Labels {
    pub prev: String,
    pub play: String,
    pub pause: String,
    pub next: String,
}

struct State {
    hwnd: HWND,
    list: ITaskbarList3,
    labels: Labels,
    icons: [HICON; 4],
    playing: bool,
    added: bool,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}
static APP: OnceLock<AppHandle> = OnceLock::new();
static CREATED_MSG: OnceLock<u32> = OnceLock::new();
const SUBCLASS_ID: usize = 0x4d49_55;

/// Set up the buttons on the window `hwnd` (raw handle), or just update their tooltips.
/// Call on the UI thread.
pub fn init(app: &AppHandle, hwnd: isize, labels: Labels) -> Result<(), String> {
    let _ = APP.set(app.clone());
    CREATED_MSG.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarButtonCreated")) });
    if STATE.with(|s| s.borrow().is_some()) {
        STATE.with(|s| {
            if let Some(st) = s.borrow_mut().as_mut() {
                st.labels = labels;
            }
        });
        return update();
    }
    let hwnd = HWND(hwnd as *mut _);
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let list: ITaskbarList3 = CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER).map_err(|e| e.to_string())?;
        list.HrInit().map_err(|e| e.to_string())?;
        let size = GetSystemMetrics(SM_CXSMICON).max(16) as u32;
        let rgb = if light_theme() { [32, 32, 32] } else { [255, 255, 255] };
        let icons = [
            icon(&glyph(Glyph::Prev, size, rgb), size)?,
            icon(&glyph(Glyph::Play, size, rgb), size)?,
            icon(&glyph(Glyph::Pause, size, rgb), size)?,
            icon(&glyph(Glyph::Next, size, rgb), size)?,
        ];
        STATE.with(|s| *s.borrow_mut() = Some(State { hwnd, list, labels, icons, playing: false, added: false }));
        if !SetWindowSubclass(hwnd, Some(subclass), SUBCLASS_ID, 0).as_bool() {
            return Err("could not watch the window for taskbar clicks".into());
        }
    }
    add()
}

/// Show pause while playing, play otherwise.
pub fn set_playing(playing: bool) -> Result<(), String> {
    let changed = STATE.with(|s| match s.borrow_mut().as_mut() {
        Some(st) if st.playing != playing => {
            st.playing = playing;
            true
        }
        _ => false,
    });
    if changed { update() } else { Ok(()) }
}

fn buttons(st: &State) -> [THUMBBUTTON; 3] {
    BUTTONS.map(|(id, name)| {
        let (icon, tip) = match name {
            "prev" => (st.icons[0], &st.labels.prev),
            "next" => (st.icons[3], &st.labels.next),
            _ if st.playing => (st.icons[2], &st.labels.pause),
            _ => (st.icons[1], &st.labels.play),
        };
        let mut sz = [0u16; 260];
        for (i, c) in tip.encode_utf16().take(259).enumerate() {
            sz[i] = c;
        }
        THUMBBUTTON { dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS, iId: id, iBitmap: 0, hIcon: icon, szTip: sz, dwFlags: THBF_ENABLED }
    })
}

fn add() -> Result<(), String> {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        let Some(st) = s.as_mut() else { return Ok(()) };
        if st.added {
            return Ok(());
        }
        unsafe { st.list.ThumbBarAddButtons(st.hwnd, &buttons(st)) }.map_err(|e| e.to_string())?;
        st.added = true;
        log::info!("taskbar buttons ready");
        Ok(())
    })
}

fn update() -> Result<(), String> {
    STATE.with(|s| {
        let s = s.borrow();
        let Some(st) = s.as_ref() else { return Ok(()) };
        if !st.added {
            return Ok(());
        }
        unsafe { st.list.ThumbBarUpdateButtons(st.hwnd, &buttons(st)) }.map_err(|e| e.to_string())
    })
}

unsafe extern "system" fn subclass(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    if Some(&msg) == CREATED_MSG.get() {
        // Explorer (re)created our taskbar button: the buttons have to be added again.
        STATE.with(|s| {
            if let Some(st) = s.borrow_mut().as_mut() {
                st.added = false;
            }
        });
        if let Err(e) = add() {
            log::warn!("taskbar buttons: {e}");
        }
    } else if msg == WM_COMMAND && ((wparam.0 >> 16) & 0xffff) as u32 == THBN_CLICKED {
        if let (Some(app), Some(name)) = (APP.get(), button_name((wparam.0 & 0xffff) as u32)) {
            let _ = app.emit("taskbar-button", name);
        }
        return LRESULT(0);
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// A 32-bit icon from premultiplied RGBA pixels.
unsafe fn icon(rgba: &[u8], size: u32) -> Result<HICON, String> {
    let header = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: size as i32,
        biHeight: -(size as i32), // top-down rows
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB.0,
        ..Default::default()
    };
    let info = BITMAPINFO { bmiHeader: header, ..Default::default() };
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let color = unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }.map_err(|e| e.to_string())?;
    let dst = unsafe { std::slice::from_raw_parts_mut(bits as *mut u8, rgba.len()) };
    for (d, s) in dst.chunks_mut(4).zip(rgba.chunks(4)) {
        d.copy_from_slice(&[s[2], s[1], s[0], s[3]]);
    }
    // Rows of a monochrome bitmap are padded to 16 bits; all zero = use the colour's alpha.
    let mask_bits = vec![0u8; (size.div_ceil(16) * 2 * size) as usize];
    let mask = unsafe { CreateBitmap(size as i32, size as i32, 1, 1, Some(mask_bits.as_ptr() as *const _)) };
    let info = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
    let made = unsafe { CreateIconIndirect(&info) }.map_err(|e| e.to_string());
    unsafe {
        let _ = DeleteObject(HGDIOBJ(color.0));
        let _ = DeleteObject(HGDIOBJ(mask.0));
    }
    made
}

/// Windows' taskbar theme (HKCU, read only): dark glyphs on a light taskbar.
fn light_theme() -> bool {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|k| k.get_value::<u32, _>("SystemUsesLightTheme"))
        .map(|v| v == 1)
        .unwrap_or(false)
}
