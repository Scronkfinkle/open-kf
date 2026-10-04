//! Video recording to `work/videos/` (gitignored), a debugging aid that is
//! not part of KF. F9, or the scripted input action `record`, starts and
//! stops it. See `docs/DESIGN.md`, "Video recording".
//!
//! Frames are window captures (as for F12) asked for at 30 per second of
//! real time; when the game runs slower, a capture counts for every 1/30 s
//! slot it covers, so the video plays back at real speed. Captures finish
//! on background tasks, possibly out of order, so each carries a sequence
//! number and the writer thread puts them back in order before piping the
//! raw pixels to `ffmpeg` (from the flake's dev shell).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::PrimaryWindow;

use crate::map::MapRequest;
use crate::runlog;

const FPS: f64 = 30.0;
/// Videos are scaled down to at most this width (even sizes for H.264).
const MAX_WIDTH: u32 = 1280;
/// The writer skips a missing frame once this many later ones are waiting.
const MAX_WAITING: usize = 30;

/// One capture for the writer: shown for `repeat` frame slots.
struct Frame {
    seq: u64,
    repeat: u32,
    width: u32,
    height: u32,
    /// ffmpeg's name for the pixel layout ("rgba" or "bgra").
    pix_fmt: &'static str,
    data: Vec<u8>,
}

struct Recording {
    tx: mpsc::Sender<Frame>,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
    path: PathBuf,
    /// Real time at the start, seconds.
    started: f64,
    /// Frame slots asked for so far (repeats included), and captures.
    slots: u64,
    captures: u64,
}

/// The current recording, and finished ones whose writer may still be
/// encoding (joined on exit so their files are complete).
#[derive(Resource, Default)]
pub struct Recorder {
    active: Option<Recording>,
    finishing: Vec<JoinHandle<()>>,
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if let Some(r) = self.active.take() {
            self.finishing.push(stop(r, "quit"));
        }
        for t in self.finishing.drain(..) {
            let _ = t.join();
        }
    }
}

pub struct RecordPlugin;

impl Plugin for RecordPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Recorder>()
            .add_systems(Update, record.after(crate::screenshot::take_screenshots));
    }
}

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn record(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    frames: Res<FrameCount>,
    script: Res<crate::weapon::ScriptedInput>,
    time: Res<Time<Real>>,
    request: Res<MapRequest>,
    mut recorder: ResMut<Recorder>,
    shot_frame: Res<crate::screenshot::ScreenshotFrame>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    let toggle = keys.just_pressed(KeyCode::F9) || script.0.iter().any(|(f, a)| *f == frames.0 && a == "record");
    let now = time.elapsed_secs_f64();
    if toggle {
        if let Some(r) = recorder.active.take() {
            let t = stop(r, "toggle");
            recorder.finishing.push(t);
        } else {
            match start(&request.map, now) {
                Ok(r) => recorder.active = Some(r),
                Err(e) => {
                    eprintln!("recording: {e}");
                    runlog::kv("record_error", &format!("error=\"{e}\""));
                }
            }
        }
        if let Ok(mut w) = windows.single_mut() {
            w.title = if recorder.active.is_some() { "kf-rs [REC]".into() } else { "kf-rs".into() };
        }
    }
    let Some(r) = recorder.active.as_mut() else {
        return;
    };
    // A screenshot (F12) has this frame's capture; the next one covers the slot.
    if shot_frame.0 == Some(frames.0) {
        return;
    }
    // The slots due by now; one capture covers all the new ones.
    let due = ((now - r.started) * FPS) as u64 + 1;
    if due <= r.slots {
        return;
    }
    let repeat = (due - r.slots) as u32;
    r.slots = due;
    let seq = r.captures;
    r.captures += 1;
    let tx = r.tx.clone();
    commands.spawn(Screenshot::primary_window()).observe(move |captured: On<ScreenshotCaptured>| {
        let image = &captured.image;
        let size = image.texture_descriptor.size;
        let (pix_fmt, data) = match image.texture_descriptor.format {
            TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb => ("bgra", image.data.clone()),
            TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb => ("rgba", image.data.clone()),
            // Anything else (e.g. HDR): convert, as save_to_disk does.
            _ => ("rgba", image.clone().try_into_dynamic().ok().map(|d| d.to_rgba8().into_raw())),
        };
        let Some(data) = data else {
            return;
        };
        let _ = tx.send(Frame {
            seq,
            repeat,
            width: size.width,
            height: size.height,
            pix_fmt,
            data,
        });
    });
}

fn start(map: &str, now: f64) -> Result<Recording, String> {
    // Fail at once, and say so, if ffmpeg cannot run (e.g. the game was
    // started outside `nix develop`, or a profile copy clashes with the
    // shell's libraries).
    let ok = Command::new("ffmpeg")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        return Err("ffmpeg cannot run; start the game inside `nix develop` (or direnv), which provides it".into());
    }
    let dir = PathBuf::from("work/videos");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let path = dir.join(format!("{map}-{stamp}.mp4"));
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let thread = {
        let (path, stop) = (path.clone(), stop.clone());
        std::thread::Builder::new()
            .name("video-writer".into())
            .spawn(move || write_video(rx, path, stop))
            .map_err(|e| format!("cannot start the writer thread: {e}"))?
    };
    runlog::kv("record_start", &format!("file={} fps={FPS}", path.display()));
    Ok(Recording {
        tx,
        stop,
        thread,
        path,
        started: now,
        slots: 0,
        captures: 0,
    })
}

/// Ends a recording; the writer finishes the file on its own thread.
fn stop(r: Recording, reason: &str) -> JoinHandle<()> {
    runlog::kv(
        "record_stop",
        &format!(
            "file={} reason={reason} seconds={:.2} captures={} slots={}",
            r.path.display(),
            r.slots as f64 / FPS,
            r.captures,
            r.slots
        ),
    );
    r.stop.store(true, Ordering::SeqCst);
    // Dropping our sender: the writer stops once the captures in flight
    // arrive (or after a short wait if some never do).
    drop(r.tx);
    r.thread
}

/// The ffmpeg process, started with the first frame's size.
struct Encoder {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    width: u32,
    height: u32,
}

fn spawn_ffmpeg(path: &PathBuf, f: &Frame) -> Result<Encoder, String> {
    let log = std::fs::File::create(path.with_extension("ffmpeg.log")).map_err(|e| e.to_string())?;
    let mut child = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", f.pix_fmt, "-s"])
        .arg(format!("{}x{}", f.width, f.height))
        .args(["-r", "30", "-i", "-", "-vf"])
        .arg(format!("scale='min({MAX_WIDTH},trunc(iw/2)*2)':-2"))
        .args(["-c:v", "libx264", "-preset", "veryfast", "-crf", "23", "-pix_fmt", "yuv420p"])
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log))
        .spawn()
        .map_err(|e| format!("cannot run ffmpeg (is it on PATH? it comes with `nix develop`): {e}"))?;
    let stdin = child.stdin.take().ok_or("no ffmpeg stdin")?;
    Ok(Encoder {
        child,
        stdin,
        width: f.width,
        height: f.height,
    })
}

/// The writer thread: puts captures back in order and pipes them to ffmpeg.
fn write_video(rx: mpsc::Receiver<Frame>, path: PathBuf, stop: Arc<AtomicBool>) {
    let mut encoder: Option<Encoder> = None;
    let mut waiting: BTreeMap<u64, Frame> = BTreeMap::new();
    let mut next = 0u64;
    let (mut written, mut repeated, mut skipped, mut wrong_size) = (0u64, 0u64, 0u64, 0u64);
    let mut error: Option<String> = None;
    let mut write = |f: Frame, encoder: &mut Option<Encoder>, error: &mut Option<String>| {
        if error.is_some() {
            return;
        }
        if encoder.is_none() {
            match spawn_ffmpeg(&path, &f) {
                Ok(e) => *encoder = Some(e),
                Err(e) => {
                    *error = Some(e);
                    return;
                }
            }
        }
        let Some(enc) = encoder.as_mut() else {
            return;
        };
        // The window was resized: ffmpeg's input size is fixed.
        if (f.width, f.height) != (enc.width, enc.height) {
            wrong_size += 1;
            return;
        }
        for _ in 0..f.repeat {
            if let Err(e) = enc.stdin.write_all(&f.data) {
                *error = Some(format!("writing to ffmpeg: {e}"));
                return;
            }
        }
        written += 1;
        repeated += u64::from(f.repeat.saturating_sub(1));
    };
    loop {
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(f) => {
                waiting.insert(f.seq, f);
            }
            Err(mpsc::RecvTimeoutError::Timeout) if stop.load(Ordering::SeqCst) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        loop {
            if let Some(f) = waiting.remove(&next) {
                next += 1;
                write(f, &mut encoder, &mut error);
            } else if waiting.len() > MAX_WAITING {
                // A capture never came: go on without it.
                let first = *waiting.keys().next().expect("not empty");
                skipped += first - next;
                next = first;
            } else {
                break;
            }
        }
    }
    // Whatever is left, in order (gaps skipped).
    for (seq, f) in std::mem::take(&mut waiting) {
        skipped += seq.saturating_sub(next);
        next = seq + 1;
        write(f, &mut encoder, &mut error);
    }
    let status = match encoder {
        Some(Encoder { mut child, stdin, .. }) => {
            drop(stdin); // end of input: ffmpeg finishes the file
            child.wait().map_or_else(|e| format!("wait failed: {e}"), |s| s.to_string())
        }
        None => "not started".into(),
    };
    let bytes = std::fs::metadata(&path).map_or(0, |m| m.len());
    runlog::kv(
        "record_saved",
        &format!(
            "file={} captures_written={written} repeated_slots={repeated} skipped={skipped} wrong_size={wrong_size} bytes={bytes} ffmpeg_status=\"{status}\" error=\"{}\"",
            path.display(),
            error.unwrap_or_default()
        ),
    );
}
