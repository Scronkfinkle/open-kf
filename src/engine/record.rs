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
//!
//! Sound: the final mixed output (audio/capture.rs) is copied by the audio
//! thread into chunks; an audio-writer thread places them on the same
//! real-time clock as the frames (silence before the first chunk and in
//! gaps) and writes raw samples to a temporary file. When both are done,
//! the raw sound is cut or padded to the video's length and ffmpeg muxes
//! it into the final file (video copied, sound to AAC).

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::audio::capture::{AudioChunk, Capture};

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::window::PrimaryWindow;

use crate::world::map::MapRequest;
use crate::engine::runlog;

const FPS: f64 = 30.0;
/// Videos are scaled down to at most this width (even sizes for H.264).
const MAX_WIDTH: u32 = 1280;
/// The writer skips a missing frame once this many later ones are waiting.
const MAX_WAITING: usize = 30;
/// Sound chunks waiting for the audio writer (about 5 s at 48 kHz); the
/// audio thread drops chunks (and the log counts them) past this.
const AUDIO_QUEUE: usize = 256;

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
    /// The sound tap, while it feeds this recording.
    capture: Option<Arc<Capture>>,
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
            .add_systems(Update, record.after(crate::engine::screenshot::take_screenshots));
    }
}

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn record(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    frames: Res<FrameCount>,
    script: Res<crate::weapons::weapon::ScriptedInput>,
    time: Res<Time<Real>>,
    request: Res<MapRequest>,
    audio: Option<Res<crate::audio::mixer::Audio>>,
    mut recorder: ResMut<Recorder>,
    shot_frame: Res<crate::engine::screenshot::ScreenshotFrame>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    let toggle = keys.just_pressed(KeyCode::F9) || script.0.iter().any(|(f, a)| *f == frames.0 && a == "record");
    let now = time.elapsed_secs_f64();
    if toggle {
        if let Some(r) = recorder.active.take() {
            let t = stop(r, "toggle");
            recorder.finishing.push(t);
        } else {
            // Slot 0 of the video, for the sound: now, while the game's
            // systems run (where every frame's sounds start), rather than
            // `time.last_update()`, taken before them at the frame's start.
            let at = Instant::now();
            match start(&request.map, now, at, audio.as_ref().map(|a| a.capture.clone())) {
                Ok(r) => recorder.active = Some(r),
                Err(e) => {
                    eprintln!("recording: {e}");
                    runlog::kv("record_error", &format!("error=\"{e}\""));
                }
            }
        }
        if let Ok(mut w) = windows.single_mut() {
            w.title = if recorder.active.is_some() { "Open KF [REC]".into() } else { "Open KF".into() };
        }
    }
    let Some(r) = recorder.active.as_mut() else {
        return;
    };
    // A screenshot (F12) has this frame's capture; the next one covers the slot.
    if shot_frame.0 == Some(frames.0) {
        return;
    }
    // The slots due by now; one capture covers all the new ones. Rounded:
    // the capture lands on the slot nearest its time (a half slot early or
    // late at most), not always on the slot it falls in (up to a whole slot
    // early against its sound).
    let due = ((now - r.started) * FPS + 0.5) as u64 + 1;
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

fn start(map: &str, now: f64, at: Instant, capture: Option<Arc<Capture>>) -> Result<Recording, String> {
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
    // The sound: its own writer thread, fed by the tap.
    let sound = match &capture {
        Some(c) => {
            let (atx, arx) = mpsc::sync_channel(AUDIO_QUEUE);
            let raw = path.with_extension("audio.raw");
            let rate = c.rate;
            let generation = c.begin(atx);
            let thread = std::thread::Builder::new()
                .name("audio-writer".into())
                .spawn(move || write_audio(arx, raw, rate, generation, at))
                .map_err(|e| {
                    c.end();
                    format!("cannot start the audio writer thread: {e}")
                })?;
            Some((thread, generation, rate))
        }
        None => None,
    };
    let (tx, rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let audio_info = sound.as_ref().map_or_else(|| "audio=none".to_string(), |(_, g, r)| format!("audio_rate={r} audio_generation={g}"));
    let thread = {
        let (path, stop) = (path.clone(), stop.clone());
        let audio_thread = sound.map(|(t, _, _)| t);
        std::thread::Builder::new()
            .name("video-writer".into())
            .spawn(move || write_video(rx, path, stop, audio_thread))
            .map_err(|e| {
                if let Some(c) = &capture {
                    c.end();
                }
                format!("cannot start the writer thread: {e}")
            })?
    };
    // Starting (the ffmpeg check above) takes tens of milliseconds; slot 0
    // is `at`, this long before this line's log time.
    let anchor_ms = at.elapsed().as_secs_f64() * 1000.0;
    runlog::kv("record_start", &format!("file={} fps={FPS} {audio_info} slot0_before_log_ms={anchor_ms:.1}", path.display()));
    Ok(Recording {
        tx,
        stop,
        thread,
        path,
        started: now,
        capture,
        slots: 0,
        captures: 0,
    })
}

/// Ends a recording; the writer finishes the file on its own thread.
fn stop(r: Recording, reason: &str) -> JoinHandle<()> {
    // Stops the sound tap; the audio writer ends after the chunks sent.
    let dropped = r.capture.as_ref().map_or(0, |c| c.end());
    runlog::kv(
        "record_stop",
        &format!(
            "file={} reason={reason} seconds={:.2} captures={} slots={} audio_chunks_dropped={dropped}",
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

fn spawn_ffmpeg(path: &PathBuf, log: &PathBuf, f: &Frame) -> Result<Encoder, String> {
    let log = std::fs::File::create(log).map_err(|e| e.to_string())?;
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

/// The writer thread: puts captures back in order and pipes them to
/// ffmpeg, then adds the sound (if any) to the file.
///
/// A capture is taken at the frame's real time, so it belongs to the slot
/// holding that time; the slots skipped since the previous capture (a slow
/// frame) repeat the *previous* image, so a picture never shows up before
/// the moment it shows (and so lines up with its sound).
fn write_video(rx: mpsc::Receiver<Frame>, path: PathBuf, stop: Arc<AtomicBool>, audio: Option<JoinHandle<Option<AudioTrack>>>) {
    let video_path = path.with_extension("video.mp4");
    let log_path = path.with_extension("ffmpeg.log");
    let mut encoder: Option<Encoder> = None;
    let mut waiting: BTreeMap<u64, Frame> = BTreeMap::new();
    let mut next = 0u64;
    let (mut written, mut repeated, mut skipped, mut wrong_size, mut slots_written) = (0u64, 0u64, 0u64, 0u64, 0u64);
    let mut error: Option<String> = None;
    // The last image written (for repeats).
    let mut last: Option<Vec<u8>> = None;
    let mut write = |f: Frame, encoder: &mut Option<Encoder>, error: &mut Option<String>| {
        if error.is_some() {
            return;
        }
        if encoder.is_none() {
            match spawn_ffmpeg(&video_path, &log_path, &f) {
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
        let before = last.as_deref().unwrap_or(&f.data);
        for i in 0..f.repeat {
            let data = if i + 1 == f.repeat { &f.data } else { before };
            if let Err(e) = enc.stdin.write_all(data) {
                *error = Some(format!("writing to ffmpeg: {e}"));
                return;
            }
            slots_written += 1;
        }
        written += 1;
        repeated += u64::from(f.repeat.saturating_sub(1));
        last = Some(f.data);
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
    let video_ok = error.is_none() && status.contains("exit status: 0") && video_path.exists();
    // The sound writer ends once the tap stops (stop() already did that).
    let track = audio.and_then(|t| t.join().ok().flatten());
    let video_seconds = slots_written as f64 / FPS;
    let audio_info = match (&track, video_ok) {
        (Some(track), true) => mux(&video_path, track, video_seconds, &path, &log_path),
        (Some(track), false) => {
            let _ = std::fs::remove_file(&track.path);
            "audio=\"not added: no video\"".to_string()
        }
        (None, _) => "audio=none".to_string(),
    };
    // No sound added: the video alone is the result.
    if video_ok && !path.exists() {
        let _ = std::fs::rename(&video_path, &path);
    }
    let bytes = std::fs::metadata(&path).map_or(0, |m| m.len());
    runlog::kv(
        "record_saved",
        &format!(
            "file={} captures_written={written} repeated_slots={repeated} skipped={skipped} wrong_size={wrong_size} video_frames={slots_written} video_seconds={video_seconds:.3} {audio_info} bytes={bytes} ffmpeg_status=\"{status}\" error=\"{}\"",
            path.display(),
            error.unwrap_or_default()
        ),
    );
}

/// The raw sound of a recording (stereo f32, little-endian).
struct AudioTrack {
    path: PathBuf,
    rate: u32,
    /// Stereo frames in the file.
    frames: u64,
}

/// The audio writer thread: puts the tap's chunks on the video's clock
/// (slot 0 = `video_start`) and writes them raw to `path`.
fn write_audio(rx: mpsc::Receiver<AudioChunk>, path: PathBuf, rate: u32, generation: u64, video_start: Instant) -> Option<AudioTrack> {
    let file = match std::fs::File::create(&path) {
        Ok(f) => f,
        Err(e) => {
            runlog::kv("record_audio", &format!("file={} error=\"{e}\"", path.display()));
            return None;
        }
    };
    let mut out = std::io::BufWriter::new(file);
    let rate_f = f64::from(rate);
    // Stereo frames written; the recording frame of the tap's frame 0.
    let mut frames = 0u64;
    let mut offset: Option<i64> = None;
    let (mut chunks, mut stale, mut gap_frames, mut cut_frames) = (0u64, 0u64, 0u64, 0u64);
    let mut first_at: Option<Instant> = None;
    // How far the chunk times stray from the sample count (audio clock vs
    // real clock, and bursts of the audio thread), milliseconds.
    let (mut drift_min, mut drift_max, mut drift_last) = (f64::MAX, f64::MIN, 0.0f64);
    let mut error: Option<String> = None;
    let zeros = [0u8; 8 * 1024];
    let write_silence = |out: &mut std::io::BufWriter<std::fs::File>, n: u64| -> std::io::Result<()> {
        let mut left = n * 8;
        while left > 0 {
            let k = left.min(zeros.len() as u64) as usize;
            out.write_all(&zeros[..k])?;
            left -= k as u64;
        }
        Ok(())
    };
    for chunk in rx {
        if chunk.generation != generation {
            stale += 1;
            continue;
        }
        chunks += 1;
        let first = *first_at.get_or_insert(chunk.at);
        // Where the tap's first frame lands on the video's clock.
        let base = *offset.get_or_insert_with(|| (chunk.at.saturating_duration_since(video_start).as_secs_f64() * rate_f).round() as i64);
        let expected = first + Duration::from_secs_f64(chunk.start_frame as f64 / rate_f);
        let drift = if chunk.at >= expected { (chunk.at - expected).as_secs_f64() } else { -(expected - chunk.at).as_secs_f64() } * 1000.0;
        (drift_min, drift_max, drift_last) = (drift_min.min(drift), drift_max.max(drift), drift);
        if error.is_some() {
            continue;
        }
        let at = base + chunk.start_frame as i64;
        let mut samples = &chunk.samples[..];
        if at > frames as i64 {
            // Before the first chunk, or chunks dropped: silence.
            let n = at as u64 - frames;
            if frames > 0 {
                gap_frames += n;
            }
            if let Err(e) = write_silence(&mut out, n) {
                error = Some(e.to_string());
                continue;
            }
            frames += n;
        } else if at < frames as i64 {
            let skip = ((frames as i64 - at) as usize * 2).min(samples.len());
            cut_frames += skip as u64 / 2;
            samples = &samples[skip..];
        }
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        if let Err(e) = out.write_all(&bytes) {
            error = Some(e.to_string());
            continue;
        }
        frames += samples.len() as u64 / 2;
    }
    if let Err(e) = out.flush() {
        error.get_or_insert(e.to_string());
    }
    let drift = if chunks > 0 { format!("drift_ms_min={drift_min:.1} drift_ms_max={drift_max:.1} drift_ms_end={drift_last:.1}") } else { String::new() };
    runlog::kv(
        "record_audio",
        &format!(
            "file={} rate={rate} chunks={chunks} stale_chunks={stale} start_offset_ms={:.1} frames={frames} seconds={:.3} gap_frames={gap_frames} overlap_frames={cut_frames} {drift} error=\"{}\"",
            path.display(),
            offset.unwrap_or(0) as f64 * 1000.0 / rate_f,
            frames as f64 / rate_f,
            error.clone().unwrap_or_default()
        ),
    );
    if error.is_some() || chunks == 0 {
        let _ = std::fs::remove_file(&path);
        return None;
    }
    Some(AudioTrack { path, rate, frames })
}

/// Cuts or pads the raw sound to the video's length and muxes both into
/// `out` (video copied, sound to AAC). Returns the log fields.
fn mux(video: &PathBuf, track: &AudioTrack, video_seconds: f64, out: &PathBuf, log: &PathBuf) -> String {
    let want = (video_seconds * f64::from(track.rate)).round() as u64;
    let fitted = std::fs::OpenOptions::new().write(true).open(&track.path).and_then(|f| f.set_len(want * 8));
    if let Err(e) = fitted {
        return format!("audio=\"not added: {e}\"");
    }
    let log_file = std::fs::OpenOptions::new().append(true).create(true).open(log).map(Stdio::from).unwrap_or_else(|_| Stdio::null());
    let started = Instant::now();
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(video)
        .args(["-f", "f32le", "-ar"])
        .arg(track.rate.to_string())
        .args(["-ac", "2", "-i"])
        .arg(&track.path)
        .args(["-map", "0:v", "-map", "1:a", "-c:v", "copy", "-c:a", "aac", "-b:a", "192k"])
        .arg(out)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log_file)
        .status();
    let ok = status.as_ref().is_ok_and(|s| s.success());
    let fields = format!(
        "audio_frames={} audio_frames_fitted={want} audio_padded_frames={} audio_cut_frames={} mux_ms={:.0} mux_status=\"{}\"",
        track.frames,
        want.saturating_sub(track.frames),
        track.frames.saturating_sub(want),
        started.elapsed().as_secs_f64() * 1000.0,
        status.map_or_else(|e| e.to_string(), |s| s.to_string())
    );
    if ok {
        let _ = std::fs::remove_file(video);
        let _ = std::fs::remove_file(&track.path);
    } else {
        // Keep the raw sound to look at; the video alone becomes the file.
        let _ = std::fs::remove_file(out);
    }
    fields
}
