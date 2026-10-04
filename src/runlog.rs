//! Plain-text run log at `logs/latest.log`.
//!
//! One `key=value` line per event, prefixed with seconds since start, so a
//! run can be checked by reading numbers instead of looking at the screen.
//! Each line is flushed immediately so a crash loses nothing.

use std::fs::File;
use std::io::{LineWriter, Write};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

pub const LOG_PATH: &str = "logs/latest.log";

struct RunLog {
    start: Instant,
    file: Mutex<LineWriter<File>>,
}

static LOG: OnceLock<RunLog> = OnceLock::new();

/// Creates (or truncates) the log file. Call once at startup.
pub fn init() -> std::io::Result<()> {
    std::fs::create_dir_all("logs")?;
    let file = File::create(LOG_PATH)?;
    let _ = LOG.set(RunLog {
        start: Instant::now(),
        file: Mutex::new(LineWriter::new(file)),
    });
    Ok(())
}

/// Writes one line, e.g. `kv("startup", "install=/x build=y")`.
/// Does nothing if `init` was not called or failed.
pub fn kv(event: &str, fields: &str) {
    if let Some(log) = LOG.get() {
        let t = log.start.elapsed().as_secs_f64();
        if let Ok(mut f) = log.file.lock() {
            let _ = writeln!(f, "t={t:.3} event={event} {fields}");
        }
    }
}
