//! Toki's decision logic, with no dependency on Windows.
//!
//! Everything that decides *what gets recorded* lives here: session assembly, the noise
//! floor, idle and audio presence rules, day bucketing, and title-to-domain matching.
//! `toki-daemon` supplies events from the platform and writes the resulting actions to
//! SQLite; it makes no decisions of its own.
//!
//! The split exists so these rules can be tested by replaying an event stream instead of
//! re-enacting a day at the keyboard.

pub mod event;
pub mod ignore;
pub mod resolve;
pub mod time;
pub mod tracker;

pub use event::{AppKey, Event, Millis, Presence};
pub use tracker::{
    Action, Config, IdleBucket, IdleHistogram, Session, Tracker, IDLE_BUCKET_BOUNDS_MS,
    IDLE_THRESHOLD_BOUNDS_MS,
};
