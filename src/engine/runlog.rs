//! Plain-text run log at `logs/latest.log` (or the file given by
//! `--log FILE`; network games default to `logs/latest-host.log` /
//! `logs/latest-client.log`, so two games on one machine do not write
//! into the same file).
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
    /// The file's name without folder and `.log` when it is not the
    /// default one (e.g. "latest-host"): screenshots add it to their names.
    tag: Option<String>,
}

static LOG: OnceLock<RunLog> = OnceLock::new();

/// The log file this run should write: `--log FILE` if given, else
/// `logs/latest-host.log` with `--host`, `logs/latest-client.log` with
/// `--join`, else `logs/latest.log`. Read from the raw arguments because
/// the log starts before the options are parsed.
pub fn path_from_args(args: &[String]) -> String {
    if let Some(i) = args.iter().position(|a| a == "--log")
        && let Some(p) = args.get(i + 1)
    {
        return p.clone();
    }
    if args.iter().any(|a| a == "--host") {
        "logs/latest-host.log".into()
    } else if args.iter().any(|a| a == "--join") {
        "logs/latest-client.log".into()
    } else {
        LOG_PATH.into()
    }
}

/// Creates (or truncates) the log file. Call once at startup.
pub fn init(path: &str) -> std::io::Result<()> {
    let path = std::path::Path::new(path);
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let file = File::create(path)?;
    let tag = (path != std::path::Path::new(LOG_PATH)).then(|| path.file_stem().map(|s| s.to_string_lossy().into_owned())).flatten();
    let _ = LOG.set(RunLog {
        start: Instant::now(),
        file: Mutex::new(LineWriter::new(file)),
        tag,
    });
    Ok(())
}

/// The log's tag (see `RunLog::tag`), if any.
pub fn tag() -> Option<&'static str> {
    LOG.get().and_then(|l| l.tag.as_deref())
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
