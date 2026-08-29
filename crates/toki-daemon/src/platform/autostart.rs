//! Starting with Windows, via `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
//!
//! Per-user rather than per-machine on purpose. `HKLM` would need elevation to change, and
//! a tracker you cannot turn off without an admin prompt is the wrong shape for something
//! whose whole claim is that the data is yours. It also means turning it off costs nothing
//! on a laptop running on battery, which is how this was asked for.
//!
//! The installer writes the same value. Keeping the command here as well means the GUI's
//! settings screen and `toki-daemon autostart` agree with it rather than inventing a second
//! spelling.

use windows::core::{Result, PCWSTR};
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE,
};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "Toki";

/// What the Run key should say: this executable, quoted, told to collect.
///
/// Quoted because `Program Files` has a space in it and an unquoted path there is read as a
/// command plus arguments.
pub fn command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" run", exe.display()))
}

/// The value currently in the Run key, if any.
pub fn current() -> Option<String> {
    let key = open(KEY_READ).ok()?;
    let name = wide(VALUE);
    let mut kind = REG_VALUE_TYPE::default();
    let mut len = 0u32;

    // Asked twice: once for the size, once for the data. The first call is how you find out
    // whether the value exists at all.
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            None,
            Some(&mut len),
        )
    };
    if status.is_err() || len == 0 {
        return None;
    }
    let mut buf = vec![0u16; len as usize / 2 + 1];
    let mut size = (buf.len() * 2) as u32;
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            Some(buf.as_mut_ptr() as *mut u8),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return None;
    }
    let chars = (size as usize / 2).min(buf.len());
    Some(
        String::from_utf16_lossy(&buf[..chars])
            .trim_end_matches('\0')
            .to_string(),
    )
}

/// Write the Run value, replacing whatever was there.
///
/// Rewriting an existing value is the point rather than a side effect: after a move or an
/// upgrade to a different directory, the stale path is a daemon that silently stops
/// starting.
pub fn enable() -> Result<()> {
    let cmd = command().ok_or_else(|| windows::core::Error::from(ERROR_FILE_NOT_FOUND))?;
    let key = open(KEY_WRITE)?;
    let name = wide(VALUE);
    let data = wide(&cmd);
    let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2) };
    unsafe { RegSetValueExW(key.0, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes)) }.ok()
}

/// Remove the Run value. Absent is success: the caller asked for it to be off.
pub fn disable() -> Result<()> {
    let key = open(KEY_WRITE)?;
    let name = wide(VALUE);
    let status = unsafe { RegDeleteValueW(key.0, PCWSTR(name.as_ptr())) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    status.ok()
}

/// Open the Run key, creating it if this profile has never had one.
fn open(access: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Result<Key> {
    let path = wide(RUN_KEY);
    let mut key = HKEY::default();
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            access,
            None,
            &mut key,
            None,
        )
    }
    .ok()?;
    Ok(Key(key))
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
