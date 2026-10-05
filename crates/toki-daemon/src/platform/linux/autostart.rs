//! Starting at login, as a systemd user service bound to the graphical session.
//!
//! A service rather than an XDG autostart entry because it is tied to the session's
//! lifetime both ways: started once the compositor's environment is in place — Hyprland's
//! instance signature, the Wayland display — and stopped with it, rather than outliving a
//! logout. uwsm, which Omarchy uses, and the major desktops all reach
//! `graphical-session.target`.
//!
//! Per-user and in the user's own config, for the same reason the Windows side uses `HKCU`:
//! turning it off must never need root.

use std::io::{Error, ErrorKind, Result};
use std::path::PathBuf;
use std::process::Command;

const UNIT: &str = "toki-daemon.service";

/// What the service runs: this executable, told to collect. Quoted, which systemd accepts,
/// so a path with a space in it survives.
pub fn command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\" run", exe.display()))
}

/// The command the enabled service runs, or `None` when autostart is off.
pub fn current() -> Option<String> {
    if !wants_dir()?.join(UNIT).exists() {
        return None;
    }
    let unit = std::fs::read_to_string(unit_path()?).ok()?;
    unit.lines()
        .find_map(|l| l.strip_prefix("ExecStart="))
        .map(str::to_string)
}

pub fn enable() -> Result<()> {
    let path = unit_path().ok_or_else(no_home)?;
    let cmd = command().ok_or_else(|| Error::other("cannot locate this executable"))?;
    std::fs::create_dir_all(path.parent().ok_or_else(no_home)?)?;
    std::fs::write(&path, unit(&cmd))?;
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", UNIT])
}

pub fn disable() -> Result<()> {
    // Disabling a unit that was never enabled is not an error worth reporting.
    let _ = systemctl(&["disable", UNIT]);
    if let Some(path) = unit_path() {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    systemctl(&["daemon-reload"])
}

/// `Restart=on-abnormal` brings it back after a crash but not after "Quit Toki" or a second
/// copy exiting because one is already running — both are deliberate, both exit cleanly.
fn unit(cmd: &str) -> String {
    format!(
        "[Unit]\n\
         Description=Toki screen-time collector\n\
         PartOf=graphical-session.target\n\
         After=graphical-session.target\n\
         \n\
         [Service]\n\
         ExecStart={cmd}\n\
         Restart=on-abnormal\n\
         \n\
         [Install]\n\
         WantedBy=graphical-session.target\n"
    )
}

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|c| c.join("systemd/user"))
}

fn unit_path() -> Option<PathBuf> {
    Some(config_dir()?.join(UNIT))
}

fn wants_dir() -> Option<PathBuf> {
    Some(config_dir()?.join("graphical-session.target.wants"))
}

fn no_home() -> Error {
    Error::other("neither XDG_CONFIG_HOME nor HOME is set")
}

fn systemctl(args: &[&str]) -> Result<()> {
    let status = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::other(format!(
            "systemctl --user {} failed",
            args.join(" ")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unit_runs_the_command_and_follows_the_session() {
        let u = unit("\"/usr/bin/toki-daemon\" run");
        assert!(u.contains("ExecStart=\"/usr/bin/toki-daemon\" run\n"));
        assert!(u.contains("WantedBy=graphical-session.target"));
        assert!(u.contains("PartOf=graphical-session.target"));
    }
}
