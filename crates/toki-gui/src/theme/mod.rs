//! Whether the system is in light or dark mode, so the window can start in the right theme
//! instead of flashing from one to the other.
//!
//! Read once, on launch. DESIGN.md §9 asks to follow the system setting for the initial
//! theme; there is no subscription to a live theme change here.

#[cfg(windows)]
mod win;
#[cfg(windows)]
use win::light;

#[cfg(not(windows))]
mod linux;
#[cfg(not(windows))]
use linux::light;

/// `"light"` or `"dark"`. Defaults to light on any failure — an unreadable setting is not
/// worth failing the window over.
pub fn system_theme() -> &'static str {
    if light().unwrap_or(true) {
        "light"
    } else {
        "dark"
    }
}
