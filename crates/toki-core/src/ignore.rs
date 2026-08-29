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
    fn the_name_is_taken_from_either_separator() {
        assert_eq!(exe_name(r"C:\a\b\Code.exe"), "code.exe");
        assert_eq!(exe_name("C:/a/b/Code.exe"), "code.exe");
        assert_eq!(exe_name("Code.exe"), "code.exe");
    }
}
