//! Which executable paths are the same app.
//!
//! An app is keyed by its executable path, which is right until an updater moves it. Two
//! installers do that on every update: Squirrel (Discord, Slack, GitHub Desktop) installs
//! each version into its own `app-1.0.9258` folder, and the Microsoft Store installs into a
//! folder named after the full package version. Left alone, every update starts a new row
//! and splits one app's history across them.
//!
//! The rule is deliberately narrow: drop the version from those two folder shapes and
//! nothing else. Anything subtler (two browsers sharing `chrome.exe`, Minecraft running in
//! `javaw.exe`) needs more than a path to decide, and guessing would merge apps that are
//! not the same.

/// The path an app is recorded under: `exe_path` with any version-numbered install folder
/// reduced to the part that survives an update. Unchanged for every other path.
///
/// The result need not exist on disk. It is a key, not a file to open.
pub fn canonical_exe_path(exe_path: &str) -> String {
    let sep = if exe_path.contains('\\') { '\\' } else { '/' };
    let parts: Vec<&str> = exe_path.split(sep).collect();
    let mut out: Vec<String> = Vec::with_capacity(parts.len());
    for (i, part) in parts.iter().enumerate() {
        let is_last = i + 1 == parts.len();
        if !is_last && is_squirrel_version(part) {
            continue;
        }
        let in_store = i > 0 && parts[i - 1].eq_ignore_ascii_case("WindowsApps");
        match package_family(part) {
            Some(family) if in_store && !is_last => out.push(family),
            _ => out.push((*part).to_string()),
        }
    }
    out.join(&sep.to_string())
}

/// `app-1.0.9258`: Squirrel's folder for one installed version.
fn is_squirrel_version(part: &str) -> bool {
    part.strip_prefix("app-").is_some_and(is_version)
}

/// The package family name (`Microsoft.WindowsTerminal_8wekyb3d8bbwe`) from a full package
/// name (`Microsoft.WindowsTerminal_1.24.11911.0_x64__8wekyb3d8bbwe`). The full name is
/// `Name_Version_Arch_ResourceId_PublisherId`; only the version and architecture change
/// between updates, and the resource id is usually empty.
fn package_family(part: &str) -> Option<String> {
    let fields: Vec<&str> = part.split('_').collect();
    let [name, version, _arch, _resource, publisher] = fields[..] else {
        return None;
    };
    (is_version(version) && !name.is_empty() && !publisher.is_empty())
        .then(|| format!("{name}_{publisher}"))
}

/// Dot-separated numbers, at least two of them: `1.0.9258`, `1.24.11911.0`.
fn is_version(s: &str) -> bool {
    let mut n = 0;
    for piece in s.split('.') {
        if piece.is_empty() || !piece.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        n += 1;
    }
    n >= 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squirrel_versions_fold_into_one_path() {
        let a = r"C:\Users\a\AppData\Local\Discord\app-1.0.9258\Discord.exe";
        let b = r"C:\Users\a\AppData\Local\Discord\app-1.0.9301\Discord.exe";
        assert_eq!(
            canonical_exe_path(a),
            r"C:\Users\a\AppData\Local\Discord\Discord.exe"
        );
        assert_eq!(canonical_exe_path(a), canonical_exe_path(b));
    }

    #[test]
    fn store_packages_keep_only_the_family_name() {
        let a = r"C:\Program Files\WindowsApps\Microsoft.WindowsTerminal_1.24.11911.0_x64__8wekyb3d8bbwe\WindowsTerminal.exe";
        let b = r"C:\Program Files\WindowsApps\Microsoft.WindowsTerminal_1.25.200.0_arm64__8wekyb3d8bbwe\WindowsTerminal.exe";
        assert_eq!(
            canonical_exe_path(a),
            r"C:\Program Files\WindowsApps\Microsoft.WindowsTerminal_8wekyb3d8bbwe\WindowsTerminal.exe"
        );
        assert_eq!(canonical_exe_path(a), canonical_exe_path(b));
    }

    #[test]
    fn ordinary_paths_are_left_alone() {
        for p in [
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files\imput\Helium\Application\chrome.exe",
            r"C:\Users\a\AppData\Roaming\.sklauncher\runtime\java-runtime-epsilon\windows-x64\java-runtime-epsilon\bin\javaw.exe",
            r"C:\tools\app-store\app-1.0.exe",
            "/usr/bin/zen-bin",
            "",
        ] {
            assert_eq!(canonical_exe_path(p), p);
        }
    }

    #[test]
    fn a_package_shaped_folder_outside_windowsapps_is_not_touched() {
        let p = r"C:\games\Foo_1.2_x64__abc\foo.exe";
        assert_eq!(canonical_exe_path(p), p);
    }

    #[test]
    fn folding_twice_changes_nothing() {
        let p = r"C:\Users\a\AppData\Local\Discord\app-1.0.9258\Discord.exe";
        let once = canonical_exe_path(p);
        assert_eq!(canonical_exe_path(&once), once);
    }
}
