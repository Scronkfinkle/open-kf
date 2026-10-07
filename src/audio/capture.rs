//! A tap on the final sound output, for video recordings (record.rs).
//!
//! Everything we play (our sound mixer and the music) goes through one
//! rodio mixer whose output passes through `TapSource` on its way to the
//! speakers. While a recording runs, the tap copies the samples (after the
//! master volumes) into chunks and hands them to the recorder over a
//! bounded channel. `--mute` silences the speakers *after* the tap, so a
//! muted game still records its sound, at the volume it would have played.
//!
//! The tap runs on the audio thread, so it never waits: it checks atomics,
//! uses `try_lock` / `try_send`, and drops a chunk (counted, logged at the
//! end of the recording) rather than block. Its only cost while recording
//! is one small allocation per chunk.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Samples (left, right interleaved) per chunk: 1024 frames, about 21 ms
/// at 48 kHz.
pub const CHUNK: usize = 2048;

/// A run of recorded output samples.
pub struct AudioChunk {
    /// The recording it belongs to (see `Capture::begin`).
    pub generation: u64,
    /// Stereo frames rendered by the tap in this recording before this
    /// chunk, counted even for dropped chunks, so the writer can tell
    /// where a chunk goes and fill gaps with silence.
    pub start_frame: u64,
    /// When the chunk's first sample was rendered.
    pub at: Instant,
    pub samples: Vec<f32>,
}

/// Shared between the game, the audio thread and the recorder.
pub struct Capture {
    /// Output sample rate (stereo).
    pub rate: u32,
    /// `--mute`: the speakers get silence; the recording does not.
    muted: AtomicBool,
    /// 0: not recording; else the current recording's number.
    generation: AtomicU64,
    /// Recordings started so far (numbers them).
    started: AtomicU64,
    sink: Mutex<Option<SyncSender<AudioChunk>>>,
    /// Chunks the tap could not hand over (channel full or busy).
    dropped: AtomicU64,
}

impl Capture {
    pub fn new(rate: u32, muted: bool) -> Self {
        Capture { rate, muted: AtomicBool::new(muted), generation: AtomicU64::new(0), started: AtomicU64::new(0), sink: Mutex::new(None), dropped: AtomicU64::new(0) }
    }

    /// Starts sending chunks to `tx`; returns the recording's number.
    pub fn begin(&self, tx: SyncSender<AudioChunk>) -> u64 {
        if let Ok(mut s) = self.sink.lock() {
            *s = Some(tx);
        }
        self.dropped.store(0, Ordering::Relaxed);
        let generation = self.started.fetch_add(1, Ordering::Relaxed) + 1;
        self.generation.store(generation, Ordering::Release);
        generation
    }

    /// Stops sending; dropping the sender ends the writer's loop once it
    /// has the chunks already sent. Returns the chunks dropped.
    pub fn end(&self) -> u64 {
        self.generation.store(0, Ordering::Release);
        if let Ok(mut s) = self.sink.lock() {
            *s = None;
        }
        self.dropped.load(Ordering::Relaxed)
    }
}

/// The audio thread's end of the tap: wraps the mixed output.
pub struct TapSource<S> {
    inner: S,
    capture: Arc<Capture>,
    /// The recording being captured (0: none).
    generation: u64,
    frames: u64,
    chunk: Vec<f32>,
    chunk_at: Instant,
    chunk_start: u64,
    /// The next sample is a left one.
    left: bool,
}

impl<S> TapSource<S> {
    pub fn new(inner: S, capture: Arc<Capture>) -> Self {
        TapSource { inner, capture, generation: 0, frames: 0, chunk: Vec::new(), chunk_at: Instant::now(), chunk_start: 0, left: true }
    }

    fn send(&mut self) {
        let samples = std::mem::replace(&mut self.chunk, Vec::with_capacity(CHUNK));
        let chunk = AudioChunk { generation: self.generation, start_frame: self.chunk_start, at: self.chunk_at, samples };
        let sent = match self.capture.sink.try_lock() {
            Ok(guard) => match guard.as_ref() {
                Some(tx) => !matches!(tx.try_send(chunk), Err(TrySendError::Full(_))),
                // The recording ended mid-chunk: nothing to count.
                None => true,
            },
            Err(_) => false,
        };
        if !sent {
            self.capture.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl<S: Iterator<Item = f32>> Iterator for TapSource<S> {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let s = self.inner.next().unwrap_or(0.0);
        // Start (or stop) capturing only between chunks, on a left sample.
        if self.left && self.chunk.is_empty() {
            let generation = self.capture.generation.load(Ordering::Acquire);
            if generation != self.generation {
                self.generation = generation;
                self.frames = 0;
            }
            if self.generation != 0 {
                self.chunk_at = Instant::now();
                self.chunk_start = self.frames;
                self.chunk.reserve(CHUNK);
            }
        }
        if self.generation != 0 {
            self.chunk.push(s);
            if !self.left {
                self.frames += 1;
            }
            if self.chunk.len() >= CHUNK {
                self.send();
            }
        }
        self.left = !self.left;
        Some(if self.capture.muted.load(Ordering::Relaxed) { 0.0 } else { s })
    }
}

impl<S: rodio::Source> rodio::Source for TapSource<S> {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> rodio::ChannelCount {
        self.inner.channels()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

/// Without a sound device nothing pulls the output, so voices never end
/// and recordings would be silent. This thread pulls it at real-time pace
/// instead (and throws it away after the tap).
pub fn run_without_device<S: Iterator<Item = f32> + Send + 'static>(mut source: TapSource<S>, rate: u32) {
    let spawned = std::thread::Builder::new().name("audio-null-output".into()).spawn(move || {
        let start = Instant::now();
        let mut pulled = 0u64;
        loop {
            let due = (start.elapsed().as_secs_f64() * f64::from(rate)) as u64;
            while pulled < due {
                source.next();
                source.next();
                pulled += 1;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    if let Err(e) = spawned {
        crate::engine::runlog::kv("audio_null_output", &format!("ok=false reason=\"{e}\""));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_records_while_muted_speakers_get_silence() {
        let capture = Arc::new(Capture::new(48000, true));
        let mut tap = TapSource::new(std::iter::repeat(0.5f32), capture.clone());
        assert_eq!(tap.next(), Some(0.0));
        assert_eq!(tap.next(), Some(0.0));
        let (tx, rx) = std::sync::mpsc::sync_channel(4);
        let generation = capture.begin(tx);
        for _ in 0..CHUNK * 2 + 2 {
            assert_eq!(tap.next(), Some(0.0));
        }
        let a = rx.try_recv().expect("first chunk");
        let b = rx.try_recv().expect("second chunk");
        assert_eq!((a.generation, a.start_frame, a.samples.len()), (generation, 0, CHUNK));
        assert_eq!(b.start_frame, (CHUNK / 2) as u64);
        assert!(a.samples.iter().all(|&s| s == 0.5));
        assert_eq!(capture.end(), 0);
        // The partial third chunk is not sent after the end.
        for _ in 0..CHUNK * 2 {
            tap.next();
        }
        assert!(rx.try_recv().is_err());
        // The next recording has a new number and starts at frame 0.
        let (tx, rx) = std::sync::mpsc::sync_channel(4);
        let second = capture.begin(tx);
        assert_ne!(second, generation);
        for _ in 0..CHUNK {
            tap.next();
        }
        let c = rx.try_recv().expect("chunk");
        assert_eq!((c.generation, c.start_frame), (second, 0));
    }

    #[test]
    fn a_full_channel_drops_and_counts() {
        let capture = Arc::new(Capture::new(48000, false));
        let mut tap = TapSource::new(std::iter::repeat(0.25f32), capture.clone());
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        capture.begin(tx);
        for _ in 0..CHUNK * 3 {
            assert_eq!(tap.next(), Some(0.25));
        }
        assert_eq!(capture.end(), 2);
        // The kept chunk is the first; later ones count their frames anyway.
        assert_eq!(rx.try_recv().map(|c| c.start_frame).ok(), Some(0));
    }
}
