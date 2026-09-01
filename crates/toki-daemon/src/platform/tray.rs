//! The tray icon: everything Toki can be told to do before the GUI exists.
//!
//! It lives on the pump's window because `Shell_NotifyIcon` needs a window to send its
//! callback to, and that window is already being pumped. Nothing here decides anything
//! either — a menu click becomes a [`Command`] and the run loop works out what it means.
//!
//! The icon is drawn rather than shipped. An `.ico` asset would need a resource compiler in
//! the build for a single sixteen-pixel glyph, and a drawn one costs about forty lines and
//! renders at whatever size the shell asks for.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NIN_BALLOONUSERCLICK, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos,
    GetSystemMetrics, PostMessageW, RegisterWindowMessageW, SetForegroundWindow, TrackPopupMenu,
    HICON, ICONINFO, MF_GRAYED, MF_SEPARATOR, MF_STRING, SM_CXSMICON, TPM_BOTTOMALIGN,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_CONTEXTMENU, WM_LBUTTONDBLCLK, WM_LBUTTONUP,
    WM_NULL, WM_RBUTTONUP,
};

use toki_core::event::Millis;

/// What the user asked for. Durations are how long to pause, not when it ends, because the
/// clock the pump reads and the clock the run loop stamps with are not the same call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Open,
    /// `None` pauses until it is turned back on by hand.
    Pause(Option<Millis>),
    Resume,
    Quit,
}

/// The shell's callback for our icon.
const WM_TRAY: u32 = WM_APP + 2;
/// Posted by [`set_paused`] from the run loop, because `Shell_NotifyIcon` should be called
/// from the thread that owns the window.
const WM_REFRESH: u32 = WM_APP + 3;
/// Posted by [`show_digest`], same reasoning as [`WM_REFRESH`].
const WM_DIGEST: u32 = WM_APP + 4;

const ICON_ID: u32 = 1;

const ID_OPEN: usize = 1;
const ID_PAUSE_15: usize = 2;
const ID_PAUSE_60: usize = 3;
const ID_PAUSE_UNTIL: usize = 4;
const ID_RESUME: usize = 5;
const ID_QUIT: usize = 6;

const MINUTE_MS: Millis = 60 * 1_000;

/// Mirrors the run loop's pause state so the menu and the tooltip can be built here.
static PAUSED: AtomicBool = AtomicBool::new(false);
/// The pump's window, so the run loop can ask for a refresh without holding an `HWND`.
static WINDOW: AtomicIsize = AtomicIsize::new(0);
/// Broadcast when Explorer restarts. Without re-adding the icon it is gone for the rest of
/// the session, and a tracker with no visible sign of life is one you stop trusting.
static TASKBAR_CREATED: OnceLock<u32> = OnceLock::new();
/// Set by [`show_digest`] before posting [`WM_DIGEST`], and drained on the pump's thread —
/// the only way to hand the balloon's text across, since `Shell_NotifyIconW` has to run on
/// the thread that owns the window.
static PENDING_DIGEST: Mutex<Option<(String, String)>> = Mutex::new(None);

/// Put the icon in the notification area.
pub fn add(hwnd: HWND) {
    WINDOW.store(hwnd.0 as isize, Ordering::Relaxed);
    let _ = TASKBAR_CREATED.set(unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) });
    let _ = unsafe { Shell_NotifyIconW(NIM_ADD, &data(hwnd)) };
}

pub fn remove(hwnd: HWND) {
    let nid = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        ..Default::default()
    };
    let _ = unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
    WINDOW.store(0, Ordering::Relaxed);
}

/// Tell the tray what the run loop is doing, from any thread.
pub fn set_paused(paused: bool) {
    if PAUSED.swap(paused, Ordering::Relaxed) == paused {
        return;
    }
    let hwnd = WINDOW.load(Ordering::Relaxed);
    if hwnd != 0 {
        let _ =
            unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), WM_REFRESH, WPARAM(0), LPARAM(0)) };
    }
}

/// Pop up the weekly digest as a balloon on the tray icon, from any thread. Clicking it is
/// reported back through [`handle`] as [`Command::Open`], the same as clicking the icon.
///
/// Returns whether the icon actually existed to show it on — `false` in the narrow window
/// where the very first focus event of a run reaches the collector before the pump thread
/// has created its window. The caller uses this to decide whether the digest may fire again
/// on the next focus event rather than being silently spent for the week.
pub fn show_digest(title: &str, body: &str) -> bool {
    *PENDING_DIGEST.lock().unwrap() = Some((title.to_string(), body.to_string()));
    let hwnd = WINDOW.load(Ordering::Relaxed);
    if hwnd == 0 {
        return false;
    }
    unsafe { PostMessageW(Some(HWND(hwnd as *mut _)), WM_DIGEST, WPARAM(0), LPARAM(0)) }.is_ok()
}

/// Handle a message if it is ours, and report what the user asked for.
///
/// `None` covers both "not a tray message" and "handled, nothing to do"; the caller falls
/// through to `DefWindowProc` either way, which is correct for all three of these.
pub fn handle(hwnd: HWND, msg: u32, _wparam: WPARAM, lparam: LPARAM) -> Option<Command> {
    if Some(&msg) == TASKBAR_CREATED.get() {
        let _ = unsafe { Shell_NotifyIconW(NIM_ADD, &data(hwnd)) };
        return None;
    }
    match msg {
        WM_REFRESH => {
            let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data(hwnd)) };
            None
        }
        WM_DIGEST => {
            if let Some((title, body)) = PENDING_DIGEST.lock().unwrap().take() {
                let _ = unsafe { Shell_NotifyIconW(NIM_MODIFY, &balloon(hwnd, &title, &body)) };
            }
            None
        }
        WM_TRAY => match lparam.0 as u32 {
            WM_LBUTTONUP | WM_LBUTTONDBLCLK => Some(Command::Open),
            WM_RBUTTONUP | WM_CONTEXTMENU => menu(hwnd),
            NIN_BALLOONUSERCLICK => Some(Command::Open),
            _ => None,
        },
        _ => None,
    }
}

/// Show the context menu and wait for a choice.
///
/// This blocks the pump for as long as the menu is open. Win events are posted to the
/// thread's queue rather than delivered inline, so they wait rather than being lost, and
/// nobody switches apps while holding a menu open with the mouse.
fn menu(hwnd: HWND) -> Option<Command> {
    let menu = unsafe { CreatePopupMenu() }.ok()?;
    let paused = PAUSED.load(Ordering::Relaxed);

    // Greyed rather than hidden while the GUI is not installed beside us: the entry is the
    // only hint that there is one.
    let open_flags = if crate::paths::gui_path().is_some() {
        MF_STRING
    } else {
        MF_STRING | MF_GRAYED
    };
    unsafe {
        let _ = AppendMenuW(menu, open_flags, ID_OPEN, w!("Open Toki"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        if paused {
            let _ = AppendMenuW(menu, MF_STRING, ID_RESUME, w!("Resume tracking"));
        } else {
            let _ = AppendMenuW(menu, MF_STRING, ID_PAUSE_15, w!("Pause for 15 minutes"));
            let _ = AppendMenuW(menu, MF_STRING, ID_PAUSE_60, w!("Pause for 1 hour"));
            let _ = AppendMenuW(menu, MF_STRING, ID_PAUSE_UNTIL, w!("Pause until resumed"));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, w!("Quit Toki"));
    }

    let mut at = POINT::default();
    let _ = unsafe { GetCursorPos(&mut at) };
    // Documented workaround: without taking the foreground the menu stays up after a click
    // elsewhere, and the post afterwards is what lets it close on the next click.
    let _ = unsafe { SetForegroundWindow(hwnd) };
    let chosen = unsafe {
        TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_RETURNCMD,
            at.x,
            at.y,
            None,
            hwnd,
            None,
        )
    };
    let _ = unsafe { PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0)) };
    let _ = unsafe { DestroyMenu(menu) };

    match chosen.0 as usize {
        ID_OPEN => Some(Command::Open),
        ID_PAUSE_15 => Some(Command::Pause(Some(15 * MINUTE_MS))),
        ID_PAUSE_60 => Some(Command::Pause(Some(60 * MINUTE_MS))),
        ID_PAUSE_UNTIL => Some(Command::Pause(None)),
        ID_RESUME => Some(Command::Resume),
        ID_QUIT => Some(Command::Quit),
        _ => None,
    }
}

/// The icon's current description. Rebuilt for every add and modify, so the tooltip and the
/// state can never drift apart.
fn data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: icon(),
        ..Default::default()
    };
    let tip = if PAUSED.load(Ordering::Relaxed) {
        "Toki — paused"
    } else {
        "Toki — tracking"
    };
    for (slot, ch) in nid.szTip.iter_mut().zip(tip.encode_utf16()) {
        *slot = ch;
    }
    nid
}

/// The base icon description plus the balloon fields. `NIF_INFO` on an otherwise-unchanged
/// icon is what turns a plain `NIM_MODIFY` into a popped-up notification.
fn balloon(hwnd: HWND, title: &str, body: &str) -> NOTIFYICONDATAW {
    let mut nid = data(hwnd);
    nid.uFlags |= NIF_INFO;
    nid.dwInfoFlags = NIIF_INFO;
    // `zip` stops at the shorter side, so text past the buffer's width is silently dropped
    // rather than overflowing — same trick `data()` uses for the tooltip.
    for (slot, ch) in nid.szInfoTitle.iter_mut().zip(title.encode_utf16()) {
        *slot = ch;
    }
    for (slot, ch) in nid.szInfo.iter_mut().zip(body.encode_utf16()) {
        *slot = ch;
    }
    nid
}

/// Built once. The shell keeps its own copy, but destroying ours while the icon is live is
/// the kind of thing that works until it does not.
fn icon() -> HICON {
    static CACHED: OnceLock<isize> = OnceLock::new();
    HICON(*CACHED.get_or_init(|| make_icon().map(|h| h.0 as isize).unwrap_or(0)) as *mut _)
}

fn make_icon() -> Option<HICON> {
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as usize;
    let pixels = paint(size);

    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size as i32,
            // Negative for top-down rows, so the drawing code can think in screen order.
            biHeight: -(size as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let colour =
        unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }.ok()?;
    if bits.is_null() {
        let _ = unsafe { DeleteObject(colour.into()) };
        return None;
    }
    unsafe { std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len()) };

    // An all-zero AND mask means opaque everywhere; the alpha channel is what actually
    // shapes the icon.
    let stride = size.div_ceil(32) * 4;
    let empty = vec![0u8; stride * size];
    let mask = unsafe {
        CreateBitmap(
            size as i32,
            size as i32,
            1,
            1,
            Some(empty.as_ptr() as *const _),
        )
    };

    let icon = unsafe {
        CreateIconIndirect(&ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: colour,
        })
    }
    .ok();
    unsafe {
        let _ = DeleteObject(colour.into());
        let _ = DeleteObject(mask.into());
    }
    icon
}

/// An ensō: an open ring with the clock hands crossing its hollow centre and a small accent
/// dot marking the gap where the ring breaks, in premultiplied BGRA.
///
/// The reference artwork is white strokes on a near-black square, but a tray icon has no
/// square behind it — just whatever the taskbar looks like — so the strokes are blue and
/// orange instead of white and red. White would vanish on a light taskbar; a saturated colour
/// reads on both. Sixteen pixels is small enough that the ring and its gap are the whole
/// impression, which is what the four-by-four supersampling is for.
fn paint(size: usize) -> Vec<u8> {
    const R: f32 = 37.0;
    const G: f32 = 99.0;
    const B: f32 = 235.0;
    const ACCENT_R: f32 = 242.0;
    const ACCENT_G: f32 = 84.0;
    const ACCENT_B: f32 = 45.0;

    let mut out = vec![0u8; size * size * 4];
    let centre = size as f32 / 2.0;
    let radius = centre - 0.5;
    let thickness = (size as f32 * 0.14).max(1.4);
    let inner_radius = radius - thickness;
    let hand = (size as f32 * 0.075).max(0.7);
    // Half-width, in radians, of the break at the top of the ring where the accent dot sits.
    let gap_half_angle = 12.0f32.to_radians();
    let dot_radius = (thickness * 0.6).max(1.0);
    let dot_centre_y = -(radius - thickness / 2.0);

    for y in 0..size {
        for x in 0..size {
            let mut cover_main = 0.0f32;
            let mut cover_accent = 0.0f32;
            for sub in 0..16 {
                let px = x as f32 + ((sub % 4) as f32 + 0.5) / 4.0 - centre;
                let py = y as f32 + ((sub / 4) as f32 + 0.5) / 4.0 - centre;

                let dot_dy = py - dot_centre_y;
                if px * px + dot_dy * dot_dy <= dot_radius * dot_radius {
                    cover_accent += 1.0 / 16.0;
                    continue;
                }

                let dist_sq = px * px + py * py;
                let in_ring = dist_sq <= radius * radius && dist_sq >= inner_radius * inner_radius;
                // Angle measured from twelve o'clock, wrapped to (-pi, pi] so it stays small
                // near the top regardless of which side it's approached from.
                let from_top = {
                    let raw = py.atan2(px) + std::f32::consts::FRAC_PI_2 + std::f32::consts::PI;
                    raw.rem_euclid(2.0 * std::f32::consts::PI) - std::f32::consts::PI
                };
                let in_gap = in_ring && from_top.abs() <= gap_half_angle;

                let to_twelve = px.abs() <= hand && (-radius * 0.62..=0.0).contains(&py);
                let to_three = py.abs() <= hand && (0.0..=radius * 0.46).contains(&px);

                if (in_ring && !in_gap) || to_twelve || to_three {
                    cover_main += 1.0 / 16.0;
                }
            }
            let total = cover_main + cover_accent;
            if total == 0.0 {
                continue;
            }
            let r = (R * cover_main + ACCENT_R * cover_accent) / total;
            let g = (G * cover_main + ACCENT_G * cover_accent) / total;
            let b = (B * cover_main + ACCENT_B * cover_accent) / total;
            let i = (y * size + x) * 4;
            out[i] = b.round() as u8;
            out[i + 1] = g.round() as u8;
            out[i + 2] = r.round() as u8;
            out[i + 3] = (255.0 * total).round() as u8;
        }
    }
    out
}

/// Only called if the icon is dropped without the process ending, which today it is not.
#[allow(dead_code)]
pub fn destroy_icon() {
    let h = icon();
    if !h.is_invalid() {
        let _ = unsafe { DestroyIcon(h) };
    }
}
