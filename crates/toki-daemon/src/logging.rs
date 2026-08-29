//! Diagnostics to `%LOCALAPPDATA%\Toki\logs`, rotated daily and kept for a week.
//!
//! The failure this exists for is the daemon having quietly stopped, or quietly stopped
//! recording, days before anyone looked at the numbers. `toki-daemon doctor` answers that
//! for right now; the log is what answers it for last Tuesday.
//!
//! Nothing that identifies what you were doing goes in here. Window titles and domains are
//! for the database, which lives one directory up and is not what you attach to a bug
//! report. Counts, timings and failures only.

use std::io::{self, Write};

use tracing::Level;
use tracing_appender::rolling::{self, RollingFileAppender};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::fmt::MakeWriter;

/// Seven days, which is one weekly digest's worth of history.
const KEEP_DAYS: usize = 7;

/// Start logging. `console` is whether anything would see stderr — false once the daemon has
/// let go of the console it was started with.
pub fn init(console: bool) {
    let dir = crate::paths::log_dir();
    let _ = std::fs::create_dir_all(&dir);

    let file = rolling::Builder::new()
        .rotation(rolling::Rotation::DAILY)
        .filename_prefix("toki")
        .filename_suffix("log")
        .max_log_files(KEEP_DAYS)
        .build(&dir);

    let builder = tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_timer(TimeOfDay)
        .with_target(false)
        .with_ansi(false);

    match file {
        Ok(file) => builder.with_writer(Both { file, console }).init(),
        // A log that cannot be opened is not worth failing to start over. The console, if
        // there is one, still gets everything.
        Err(_) => builder.with_writer(io::stderr).init(),
    }
}

/// Time of day only. The file rotates daily and its name carries the date, so repeating it
/// on every line buys nothing. Local, because every other time the user sees is local.
struct TimeOfDay;

impl FormatTime for TimeOfDay {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", chrono::Local::now().format("%H:%M:%S%.3f"))
    }
}

/// Writes to the log and, when there is one, the console.
///
/// Written by hand rather than reached for with a layered subscriber: two destinations is
/// the whole requirement, and this keeps `tracing-subscriber` down to `fmt`.
struct Both {
    file: RollingFileAppender,
    console: bool,
}

impl<'a> MakeWriter<'a> for Both {
    type Writer = BothWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        BothWriter {
            file: self.file.make_writer(),
            console: self.console,
        }
    }
}

struct BothWriter<'a> {
    file: rolling::RollingWriter<'a>,
    console: bool,
}

impl Write for BothWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // The console is best-effort: a closed pipe there must not stop the file from being
        // written, which is the copy that outlives the session.
        if self.console {
            let _ = io::stderr().write_all(buf);
        }
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.console {
            let _ = io::stderr().flush();
        }
        self.file.flush()
    }
}
