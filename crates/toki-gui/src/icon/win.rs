//! The shell's large icon for a file, as straight-alpha RGBA.

use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO};

pub fn rgba(exe_path: &str) -> Option<(u32, u32, Vec<u8>)> {
    let wide: Vec<u16> = exe_path.encode_utf16().chain(Some(0)).collect();
    let mut info = SHFILEINFOW::default();
    // SAFETY: `wide` is NUL-terminated and outlives the call; `info` is the size we pass.
    let ok = unsafe {
        SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        )
    };
    if ok == 0 || info.hIcon.is_invalid() {
        return None;
    }

    let mut icon = ICONINFO::default();
    // SAFETY: `hIcon` came from the call above and is destroyed exactly once, below.
    let got = unsafe { GetIconInfo(info.hIcon, &mut icon) };
    let out = got.ok().and_then(|()| {
        let px = read(icon.hbmColor, icon.hbmMask);
        // SAFETY: GetIconInfo hands over ownership of both bitmaps.
        unsafe {
            let _ = DeleteObject(HGDIOBJ(icon.hbmColor.0));
            let _ = DeleteObject(HGDIOBJ(icon.hbmMask.0));
        }
        px
    });
    // SAFETY: see above.
    unsafe {
        let _ = DestroyIcon(info.hIcon);
    }
    out
}

fn read(color: HBITMAP, mask: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    if color.is_invalid() {
        return None;
    }
    let mut bm = BITMAP::default();
    // SAFETY: `bm` is the size we pass.
    let n = unsafe {
        GetObjectW(
            HGDIOBJ(color.0),
            size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut _),
        )
    };
    if n == 0 || bm.bmWidth <= 0 || bm.bmHeight <= 0 {
        return None;
    }
    let (w, h) = (bm.bmWidth as u32, bm.bmHeight as u32);

    let mut px = dib(color, w, h)?;
    // Icons predating alpha carry it in a separate 1-bit mask instead: black is opaque.
    if px.chunks_exact(4).all(|p| p[3] == 0) {
        let m = if mask.is_invalid() {
            None
        } else {
            dib(mask, w, h)
        };
        for (i, p) in px.chunks_exact_mut(4).enumerate() {
            p[3] = match &m {
                Some(m) if m[i * 4] != 0 => 0,
                _ => 255,
            };
        }
    }
    // BGRA -> RGBA.
    for p in px.chunks_exact_mut(4) {
        p.swap(0, 2);
    }
    Some((w, h, px))
}

/// Top-down 32-bit BGRA pixels of `bitmap`.
fn dib(bitmap: HBITMAP, w: u32, h: u32) -> Option<Vec<u8>> {
    let mut header = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut buf = vec![0u8; (w * h * 4) as usize];
    // SAFETY: `buf` holds exactly `h` rows of `w` 32-bit pixels, which is what is asked for.
    let lines = unsafe {
        let dc = GetDC(None);
        let lines = GetDIBits(
            dc,
            bitmap,
            0,
            h,
            Some(buf.as_mut_ptr().cast()),
            &mut header,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, dc);
        lines
    };
    (lines != 0).then_some(buf)
}
