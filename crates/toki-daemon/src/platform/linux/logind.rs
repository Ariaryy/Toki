//! Sleep and lock, from systemd-logind over the system bus.
//!
//! The Linux counterparts of `WM_POWERBROADCAST` and `WM_WTSSESSION_CHANGE`.
//! `PrepareForSleep` fires on the way down and again on the way up. `Lock` and `Unlock`
//! fire when something asks logind to lock — `loginctl lock-session`, which is how
//! hypridle locks before sleep. A locker started directly, without telling logind, is not
//! seen here; idle detection still closes that session at the last input.
//!
//! Each runs on its own thread because the blocking signal iterators each own one.

use chrono::Utc;
use tracing::warn;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedObjectPath;

use crate::platform::Raw;

const LOGIND: &str = "org.freedesktop.login1";

pub fn watch() {
    spawn("toki-sleep", sleep_events);
    spawn("toki-lock", lock_events);
}

fn spawn(name: &str, body: fn() -> zbus::Result<()>) {
    let _ = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            if let Err(e) = body() {
                warn!(%e, "logind unavailable; sleep and lock will be inferred from idle");
            }
        });
}

fn now() -> i64 {
    Utc::now().timestamp_millis()
}

fn sleep_events() -> zbus::Result<()> {
    let conn = Connection::system()?;
    let manager = manager(&conn)?;
    for msg in manager.receive_signal("PrepareForSleep")? {
        let going_down: bool = msg.body().deserialize()?;
        let at = now();
        super::pump::send(if going_down {
            Raw::Suspend { at }
        } else {
            Raw::Resume { at }
        });
    }
    Ok(())
}

fn lock_events() -> zbus::Result<()> {
    let conn = Connection::system()?;
    let session = Proxy::new(
        &conn,
        LOGIND,
        session_path(&conn)?,
        "org.freedesktop.login1.Session",
    )?;
    for msg in session.receive_all_signals()? {
        let at = now();
        match msg.header().member().map(|m| m.as_str()) {
            Some("Lock") => super::pump::send(Raw::Lock { at }),
            // Same as on Windows: an unlock is a resume, so the run loop asks what is in
            // front rather than waiting to be told.
            Some("Unlock") => super::pump::send(Raw::Resume { at }),
            _ => {}
        }
    }
    Ok(())
}

fn manager(conn: &Connection) -> zbus::Result<Proxy<'static>> {
    Proxy::new(
        conn,
        LOGIND,
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
}

/// This user's graphical session.
///
/// `XDG_SESSION_ID` when the environment has it. A systemd user service does not — it runs
/// outside any session — so failing that, the user's display session, which is the one
/// a desktop user means.
fn session_path(conn: &Connection) -> zbus::Result<OwnedObjectPath> {
    let manager = manager(conn)?;
    if let Ok(id) = std::env::var("XDG_SESSION_ID") {
        return manager.call("GetSession", &(id,));
    }
    let user: OwnedObjectPath = manager.call("GetUser", &(unsafe { libc::getuid() },))?;
    let user = Proxy::new(conn, LOGIND, user, "org.freedesktop.login1.User")?;
    let (_id, path): (String, OwnedObjectPath) = user.get_property("Display")?;
    Ok(path)
}
