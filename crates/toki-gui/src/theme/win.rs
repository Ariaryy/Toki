//! `AppsUseLightTheme`, the per-user switch Settings > Personalization > Colors flips.

use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
};

const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
const VALUE: &str = "AppsUseLightTheme";

pub fn light() -> Option<bool> {
    let path = wide(KEY);
    let mut hkey = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            Some(0),
            KEY_READ,
            &mut hkey,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let _guard = Key(hkey);

    let name = wide(VALUE);
    let mut data = [0u8; 4];
    let mut len = data.len() as u32;
    let status = unsafe {
        RegQueryValueExW(
            hkey,
            PCWSTR(name.as_ptr()),
            None,
            None,
            Some(data.as_mut_ptr()),
            Some(&mut len),
        )
    };
    if status != ERROR_SUCCESS || len != 4 {
        return None;
    }
    Some(u32::from_ne_bytes(data) != 0)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Closes on drop, so an early return cannot leak a key handle.
struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        let _ = unsafe { RegCloseKey(self.0) };
    }
}
