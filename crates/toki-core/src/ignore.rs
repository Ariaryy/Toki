//! Windows shell surfaces that take the foreground without being something you used.
//!
//! Pressing the Start key, opening the emoji picker, or invoking search all raise a real
//! window owned by a real process. Recorded literally, a day's log fills with them.
//!
//! An ignored app produces no event at all, so the session it interrupted stays open and
//! keeps accruing. That is the intended reading: typing into the search box for four
//! seconds is part of whatever you were doing, not four seconds of `SearchHost`.

/// Shell components ignored unless the user says otherwise.
///
/// Everything here ships with Windows and has no window a person would call an app. Third
/// party overlays are not guessed at — those go in the user's own list, because there is no
/// way to tell an overlay from an application from the outside.
const DEFAULT: &[&str] = &[
    "textinputhost.exe", // emoji picker, IME candidate window
    "searchhost.exe",    // Start menu search, Win11
    "searchapp.exe",     // Start menu search, Win10
    "startmenuexperiencehost.exe",
    "shellexperiencehost.exe", // action centre, notification flyouts
    "lockapp.exe",             // the lock screen itself
    "peopleexperiencehost.exe",
    "windows.internal.composableshell.experiences.textinput.inputapp.exe",
];

/// `explorer.exe` window classes that are shell chrome, not something a person would call
/// using File Explorer. Unlike everything in [`DEFAULT`], the executable itself is a real,
/// trackable application — a genuine File Explorer window is also `explorer.exe` — so the
/// whole process can't go on that list without losing real usage. The window class is what
/// tells the two apart: it is a fixed identifier from the window's creation, not a caption
/// that happens to be empty this time.
///
/// Confirmed empirically against a live Windows 11 desktop (2026-08-31), not guessed from
/// documentation — window classes are undocumented shell internals that shift between
/// Windows versions.
const EXPLORER_CHROME_CLASSES: &[&str] = &[
    "ApplicationManager_DesktopShellWindow", // the desktop itself (Win+D)
    "Shell_TrayWnd",                         // the taskbar
    "XamlExplorerHostIslandWindow",          // Alt-Tab's task switcher
    "ForegroundStaging",                     // transient window during that same transition
];

/// The file name of an executable, lowercased.
pub fn exe_name(exe_path: &str) -> String {
    exe_path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(exe_path)
        .to_ascii_lowercase()
}

/// Whether this executable is shell furniture rather than an application.
pub fn is_default_ignored(exe_path: &str) -> bool {
    DEFAULT.contains(&exe_name(exe_path).as_str())
}

/// Whether this window is `explorer.exe` shell chrome — the desktop, taskbar, or Alt-Tab
/// switcher — rather than a real File Explorer window. Checked separately from
/// [`is_default_ignored`] because the executable alone can't tell them apart.
pub fn is_shell_chrome(exe_path: &str, class: &str) -> bool {
    exe_name(exe_path) == "explorer.exe" && EXPLORER_CHROME_CLASSES.contains(&class)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_surfaces_are_ignored_by_default() {
        assert!(is_default_ignored(
            r"C:\Windows\SystemApps\MicrosoftWindows.Client.CBS_cw5n1h2txyewy\TextInputHost.exe"
        ));
        assert!(is_default_ignored(
            r"C:\Windows\SystemApps\x\SearchHost.exe"
        ));
        assert!(is_default_ignored(r"C:\Windows\SystemApps\x\LockApp.exe"));
    }

    #[test]
    fn real_applications_are_not() {
        assert!(!is_default_ignored(r"C:\Windows\explorer.exe"));
        assert!(!is_default_ignored(
            r"C:\Program Files\Google\Chrome\Application\chrome.exe"
        ));
        // Not a prefix or substring match.
        assert!(!is_default_ignored(r"C:\a\SearchHostReplacement.exe"));
    }

    #[test]
    fn explorer_shell_chrome_is_ignored_by_class_not_by_executable() {
        assert!(is_shell_chrome(
            r"C:\Windows\explorer.exe",
            "XamlExplorerHostIslandWindow"
        ));
        assert!(is_shell_chrome(r"C:\Windows\explorer.exe", "Shell_TrayWnd"));
        // explorer.exe is a real application too — a File Explorer window is the same exe
        // under a different class, and must stay trackable.
        assert!(!is_shell_chrome(
            r"C:\Windows\explorer.exe",
            "CabinetWClass"
        ));
        // The same class name on a different executable means nothing; only explorer.exe's
        // own shell chrome is being named here.
        assert!(!is_shell_chrome(
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            "Shell_TrayWnd"
        ));
    }

    #[test]
    fn the_name_is_taken_from_either_separator() {
        assert_eq!(exe_name(r"C:\a\b\Code.exe"), "code.exe");
        assert_eq!(exe_name("C:/a/b/Code.exe"), "code.exe");
        assert_eq!(exe_name("Code.exe"), "code.exe");
    }
}
