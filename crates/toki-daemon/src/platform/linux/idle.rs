//! When the keyboard and mouse were last touched.
//!
//! Wayland has no "last input time" to ask for — a client may not watch input it does not
//! own. What it offers instead is `ext-idle-notify-v1`: name a timeout and the compositor says
//! when the seat has been idle that long, and again when input resumes. With a short timeout
//! that is enough to reconstruct the answer: while idle, input stopped exactly one timeout
//! before the `idled` event; otherwise it is happening now.
//!
//! Version 2 of the protocol adds a notification that ignores idle inhibitors. That one is
//! preferred, because a playing video inhibits idle and would otherwise read as someone at
//! the keyboard. Toki has its own, bounded, audio exemption for that case.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Once;

use chrono::Utc;
use tracing::warn;
use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{
    self, ExtIdleNotificationV1,
};
use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::ExtIdleNotifierV1;

use toki_core::event::Millis;

/// How long without input before the compositor reports it. Short, because it is the
/// resolution of the answer: anything under this reads as active.
const TIMEOUT_MS: u32 = 5_000;

/// When input stopped, or zero while it has not.
static IDLE_SINCE: AtomicI64 = AtomicI64::new(0);

/// Wall-clock time of the last keyboard or mouse input.
///
/// The first call starts the watcher. Until it is connected — or forever, when there is no
/// Wayland compositor to ask — this returns `now`, which reads as "input just happened".
/// Failing the other way would let a broken connection silently close every session at
/// the idle threshold.
pub fn last_input_at(now: Millis) -> Millis {
    static START: Once = Once::new();
    START.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("toki-idle".into())
            .spawn(|| {
                if let Err(e) = watch() {
                    warn!(%e, "idle detection unavailable; every session will read as active");
                }
            });
    });
    match IDLE_SINCE.load(Ordering::Relaxed) {
        0 => now,
        since => since.min(now),
    }
}

fn watch() -> Result<(), String> {
    let conn = Connection::connect_to_env().map_err(|e| e.to_string())?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(|e| e.to_string())?;
    let qh = queue.handle();
    let seat: wl_seat::WlSeat = globals
        .bind(&qh, 1..=1, ())
        .map_err(|e| format!("no seat: {e}"))?;
    let notifier: ExtIdleNotifierV1 = globals
        .bind(&qh, 1..=2, ())
        .map_err(|e| format!("compositor lacks ext-idle-notify-v1: {e}"))?;
    let _notification = if notifier.version() >= 2 {
        notifier.get_input_idle_notification(TIMEOUT_MS, &seat, &qh, ())
    } else {
        notifier.get_idle_notification(TIMEOUT_MS, &seat, &qh, ())
    };
    loop {
        queue
            .blocking_dispatch(&mut State)
            .map_err(|e| e.to_string())?;
    }
}

struct State;

impl Dispatch<ExtIdleNotificationV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ExtIdleNotificationV1,
        event: ext_idle_notification_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => {
                let now = Utc::now().timestamp_millis();
                IDLE_SINCE.store(now - Millis::from(TIMEOUT_MS), Ordering::Relaxed);
            }
            ext_idle_notification_v1::Event::Resumed => IDLE_SINCE.store(0, Ordering::Relaxed),
            _ => {}
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtIdleNotifierV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ExtIdleNotifierV1,
        _: <ExtIdleNotifierV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
