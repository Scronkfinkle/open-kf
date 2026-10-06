//! Sound (milestone 10, S2): our own mixer, fed to the sound card through
//! rodio (the library Bevy's audio uses). See DESIGN.md, "Sound and music".
//!
//! Any system plays a sound by writing a [`PlaySound`] message, KF's
//! `Actor.PlaySound(Sound, Slot, Volume, bNoOverride, Radius, Pitch)`.
//! Once a frame, [`play_sounds`] looks the sound up in the game's packages
//! and starts a voice; [`update_voices`] sets every voice's left/right
//! volume and pitch from where the listener (the player's view) is. The
//! mixer runs on the audio thread and adds the voices together.
//!
//! Without a sound device the game runs silent (logged `audio_device`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use ue_assets::package::ObjectRef;
use ue_assets::package_set::{ObjectHandle, PackageSet};

use crate::camera::FlyCamera;
use crate::runlog;

/// KillingFloor.ini [ALAudio.ALAudioSubsystem] Channels: voices at once.
const MAX_VOICES: usize = 32;
/// KillingFloor.ini SoundVolume: the master volume of sound effects.
const SOUND_VOLUME: f32 = 0.3;
/// Actor defaults TransientSoundVolume and TransientSoundRadius: what
/// PlaySound uses when the call leaves them out.
pub const DEFAULT_VOLUME: f32 = 0.3;
pub const DEFAULT_RADIUS: f32 = 300.0;
const TEST_VOLUME: f32 = 1.8;
/// Frames the mixer renders per lock of the shared state (about 6 ms at
/// 44.1 kHz). Volume changes are ramped over one block to avoid clicks.
const BLOCK: usize = 256;

/// Actor.ESoundSlot. A new sound in an actor's slot stops the one playing
/// there (unless the new one says bNoOverride); SLOT_None never does.
// The slots, entity sounds and options are used from S3 on.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Slot {
    #[default]
    None,
    Misc,
    Pain,
    Interact,
    Ambient,
    Talk,
    Interface,
}

/// Where a sound plays from.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Emitter {
    /// Follows an entity (its slots belong to that entity).
    Entity(Entity),
    /// A fixed point, Bevy coordinates (metres).
    Point(Vec3),
    /// At the listener: no falloff, no panning (the HUD, the player's own
    /// weapon).
    Listener,
}

/// Actor.PlaySound. Build with [`PlaySound::new`], which fills in KF's
/// defaults, then change what the script call passes.
#[derive(Message, Clone, Debug)]
pub struct PlaySound {
    /// Full object path, e.g. `KF_9MMSnd.9mm_Fire` (case does not matter).
    pub sound: String,
    pub emitter: Emitter,
    pub slot: Slot,
    pub volume: f32,
    /// Unreal units.
    pub radius: f32,
    pub pitch: f32,
    pub no_override: bool,
    /// Which actor's slots, when several share one emitter: the player's
    /// weapons all play at the listener, but each KF weapon is its own
    /// actor with its own slots. 0 = the emitter itself.
    pub actor: u64,
}

#[allow(dead_code)]
impl PlaySound {
    pub fn new(sound: impl Into<String>, emitter: Emitter) -> Self {
        PlaySound { sound: sound.into(), emitter, slot: Slot::None, volume: DEFAULT_VOLUME, radius: DEFAULT_RADIUS, pitch: 1.0, no_override: false, actor: 0 }
    }
    pub fn slot(mut self, slot: Slot) -> Self {
        self.slot = slot;
        self
    }
    pub fn volume(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }
    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }
    pub fn pitch(mut self, pitch: f32) -> Self {
        self.pitch = pitch;
        self
    }
    pub fn actor(mut self, actor: u64) -> Self {
        self.actor = actor;
        self
    }
    pub fn no_override(mut self) -> Self {
        self.no_override = true;
        self
    }
}

/// Loads and decodes sounds ahead of use (KF's PreloadAssets), so the
/// first shot does not wait for a package; names that cannot be found are
/// logged (`sound_missing`).
#[derive(Message, Clone, Debug)]
pub struct PreloadSounds {
    /// For the log, e.g. the weapon class.
    pub what: String,
    pub sounds: Vec<String>,
}

/// Actor.AmbientSound: a sound that loops for as long as the entity has
/// this component, with SoundVolume (0-255), SoundRadius and SoundPitch
/// (64 = normal pitch). Changing the sound restarts it; changing the
/// volume, radius or pitch does not.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct AmbientSound {
    pub sound: String,
    pub volume: u8,
    pub radius: f32,
    pub pitch: u8,
    /// At the listener (the player's own weapon), like `Emitter::Listener`.
    pub at_listener: bool,
}

impl AmbientSound {
    /// The voice volume. **A guess**: 128 (Actor's default SoundVolume)
    /// counts as 1.0, matching KF's weapons, which end a 255 loop with a
    /// tail played at AmbientFireVolume/127 = 2.0 (KFHighROFFire).
    fn voice_volume(&self) -> f32 {
        self.volume as f32 / 128.0
    }
}

/// Decoded samples of one sound, -1..1, interleaved if stereo.
pub struct Clip {
    rate: u32,
    channels: usize,
    samples: Vec<f32>,
    /// Loop start and end frames (the wav's `smpl` chunk), else the whole
    /// clip. Only used by looping voices.
    loop_points: Option<(u32, u32)>,
}

impl Clip {
    fn frames(&self) -> usize {
        self.samples.len() / self.channels
    }

    fn duration(&self) -> f32 {
        self.frames() as f32 / self.rate as f32
    }

    /// The frames a looping voice repeats: [start, end).
    fn loop_range(&self) -> (f64, f64) {
        let n = self.frames() as f64;
        match self.loop_points {
            Some((a, b)) if (a as f64) < (b as f64).min(n) => (a as f64, (b as f64).min(n)),
            _ => (0.0, n),
        }
    }

    /// Linear interpolation between frames; `pos` in frames.
    fn sample(&self, pos: f64) -> (f32, f32) {
        let i = pos as usize;
        let t = (pos - i as f64) as f32;
        let at = |f: usize, c: usize| self.samples.get(f * self.channels + c).copied().unwrap_or(0.0);
        let lerp = |c: usize| at(i, c) + (at(i + 1, c) - at(i, c)) * t;
        if self.channels == 1 {
            let s = lerp(0);
            (s, s)
        } else {
            (lerp(0), lerp(1))
        }
    }
}

/// A sound or a SoundGroup: one or more clips with their Likelihood
/// (weight in the random pick).
type Entry = Arc<Vec<(Arc<Clip>, f32)>>;

/// Finds and decodes sounds by name. Not `Send` (the package reader uses
/// `Rc`), so it lives on the main thread.
pub struct SoundBank {
    set: PackageSet,
    cache: HashMap<String, Option<Entry>>,
    rng: u32,
}

impl SoundBank {
    fn new(root: PathBuf) -> Self {
        // A fixed seed, as elsewhere in the project (runs repeat exactly).
        SoundBank { set: PackageSet::new(&root), cache: HashMap::new(), rng: 0x2545_f491 }
    }

    fn lookup(&mut self, name: &str) -> Option<Entry> {
        let key = name.to_ascii_lowercase();
        if let Some(e) = self.cache.get(&key) {
            return e.clone();
        }
        let entry = self.set.find_object(name, None).and_then(|h| {
            let mut clips = Vec::new();
            self.collect(&h, &mut clips, 0);
            (!clips.is_empty()).then(|| Arc::new(clips))
        });
        if entry.is_none() {
            runlog::kv("sound_missing", &format!("sound={name}"));
        }
        self.cache.insert(key, entry.clone());
        entry
    }

    /// Adds a Sound's clip, or every member of a SoundGroup (groups may
    /// hold groups; depth-limited).
    fn collect(&self, h: &ObjectHandle, out: &mut Vec<(Arc<Clip>, f32)>, depth: u32) {
        let pkg = &h.package.pkg;
        match h.class_name() {
            "Sound" => match ue_assets::sound::read_sound(pkg, h.export) {
                Ok(s) => match ue_assets::sound::decode_wav(&s.data) {
                    Ok(w) => {
                        let channels = w.channels.min(2) as usize;
                        let samples = if w.channels as usize == channels {
                            w.samples.iter().map(|&s| s as f32 / 32768.0).collect()
                        } else {
                            // More than two channels (none in KF): keep the first two.
                            w.samples.chunks(w.channels as usize).flat_map(|f| [f[0], f[1]]).map(|s| s as f32 / 32768.0).collect()
                        };
                        out.push((Arc::new(Clip { rate: w.sample_rate, channels, samples, loop_points: w.loop_points }), s.likelihood.max(0.0)));
                    }
                    Err(e) => runlog::kv("sound_error", &format!("sound={} reason=\"{e}\"", h.path())),
                },
                Err(e) => runlog::kv("sound_error", &format!("sound={} reason=\"{e}\"", h.path())),
            },
            "SoundGroup" if depth < 4 => match ue_assets::sound::read_sound_group(pkg, h.export) {
                Ok(members) => {
                    for rf in members {
                        if let Some(m) = self.set.resolve(&h.package, rf).filter(|_| rf != ObjectRef::Null) {
                            self.collect(&m, out, depth + 1);
                        }
                    }
                }
                Err(e) => runlog::kv("sound_error", &format!("sound={} reason=\"{e}\"", h.path())),
            },
            other => runlog::kv("sound_error", &format!("sound={} reason=\"a {other}, not a sound\"", h.path())),
        }
    }

    /// SoundGroup: a random member, weighted by Likelihood.
    fn pick(&mut self, entry: &Entry) -> Arc<Clip> {
        if entry.len() == 1 {
            return entry[0].0.clone();
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        let total: f32 = entry.iter().map(|e| e.1).sum();
        let mut r = (self.rng as f32 / u32::MAX as f32) * total;
        for (clip, w) in entry.iter() {
            if r < *w {
                return clip.clone();
            }
            r -= w;
        }
        entry[entry.len() - 1].0.clone()
    }
}

/// Volume over distance. **A guess**: Epic's Unreal audio document says
/// sounds "diminish linearly according to distance and are culled" at
/// their radius; UT2004's own OpenAL code (which has Rolloff=0.5 in the
/// ini) is not public. To be compared by ear with the real game.
fn distance_gain(distance: f32, radius: f32) -> f32 {
    if radius <= 0.0 {
        return 0.0;
    }
    (1.0 - distance / radius).clamp(0.0, 1.0)
}

/// A voice's final volume: the PlaySound volume x the distance fade x the
/// master volume, capped at 1. **A guess**: OpenAL (which KF's audio used)
/// caps each source's gain at 1 by default (AL_MAX_GAIN), and KF's scripts
/// pass volumes far above 1 (guns 1.8, reload notifies 2.5, KFWeapon's
/// TransientSoundVolume 100 for the select sound, which is recorded quiet:
/// peak 0.18 against 0.99 for the 9mm shot).
fn voice_gain(loudness: f32, master: f32) -> f32 {
    (loudness * master).min(1.0)
}

/// Left/right volumes from where the sound is relative to the listener:
/// -1 fully left, 1 fully right. KF's ini has Use3DSound=False, so plain
/// stereo balance. The near ear stays at full volume, the far one fades.
fn pan_gains(pan: f32) -> [f32; 2] {
    let p = pan.clamp(-1.0, 1.0);
    [(1.0 - p).min(1.0), (1.0 + p).min(1.0)]
}

struct MixVoice {
    id: u64,
    clip: Arc<Clip>,
    /// Position in the clip, frames.
    pos: f64,
    /// Clip frames per output frame (sample rate ratio x pitch).
    step: f64,
    gain: [f32; 2],
    target: [f32; 2],
    stopping: bool,
    /// AmbientSound: repeats until stopped.
    looping: bool,
}

/// State shared between the game and the audio thread.
#[derive(Default)]
struct MixState {
    voices: Vec<MixVoice>,
    /// Ids of voices that ended, for the game to drop its records.
    finished: Vec<u64>,
}

impl MixState {
    /// Renders `out.len() / 2` stereo frames.
    fn render(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        let frames = out.len() / 2;
        let mut ended = Vec::new();
        for v in &mut self.voices {
            if v.stopping {
                v.target = [0.0; 2];
            }
            let last = v.clip.frames().saturating_sub(1) as f64;
            let (loop_start, loop_end) = v.clip.loop_range();
            for f in 0..frames {
                if v.looping && v.pos >= loop_end {
                    v.pos = loop_start + (v.pos - loop_end) % (loop_end - loop_start).max(1.0);
                }
                if !v.looping && v.pos > last {
                    break;
                }
                let t = f as f32 / frames as f32;
                let (l, r) = v.clip.sample(v.pos);
                out[2 * f] += l * (v.gain[0] + (v.target[0] - v.gain[0]) * t);
                out[2 * f + 1] += r * (v.gain[1] + (v.target[1] - v.gain[1]) * t);
                v.pos += v.step;
            }
            v.gain = v.target;
            if (!v.looping && v.pos > last) || v.stopping {
                ended.push(v.id);
            }
        }
        self.voices.retain(|v| !ended.contains(&v.id));
        self.finished.extend(ended);
        for s in out.iter_mut() {
            *s = s.clamp(-1.0, 1.0);
        }
    }
}

/// The audio thread's end: a never-ending stereo source.
struct MixSource {
    shared: Arc<Mutex<MixState>>,
    buf: Vec<f32>,
    at: usize,
    rate: rodio::SampleRate,
}

impl Iterator for MixSource {
    type Item = rodio::Sample;

    fn next(&mut self) -> Option<rodio::Sample> {
        if self.at >= self.buf.len() {
            if let Ok(mut s) = self.shared.lock() {
                s.render(&mut self.buf);
            } else {
                self.buf.fill(0.0);
            }
            self.at = 0;
        }
        self.at += 1;
        Some(self.buf[self.at - 1])
    }
}

impl rodio::Source for MixSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> rodio::ChannelCount {
        rodio::ChannelCount::new(2).unwrap()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        self.rate
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

/// The game's record of a playing voice.
struct VoiceInfo {
    id: u64,
    sound: String,
    slot: Slot,
    emitter: Emitter,
    volume: f32,
    radius: f32,
    pitch: f32,
    clip_rate: u32,
    /// Last computed loudness (0-1), to pick which voice to drop when all
    /// 32 are busy.
    loudness: f32,
    actor: u64,
    /// An AmbientSound's voice: the entity carrying it. These loop and are
    /// not counted in the 32 (a simplification).
    ambient: Option<Entity>,
}

#[derive(Resource)]
pub struct Audio {
    _sink: Option<rodio::MixerDeviceSink>,
    shared: Arc<Mutex<MixState>>,
    voices: Vec<VoiceInfo>,
    next_id: u64,
    out_rate: u32,
    /// `--mute`: voices are still tracked and logged, at zero volume.
    muted: bool,
}

/// `--mute` from the command line.
#[derive(Resource, Clone, Copy, Default)]
pub struct AudioSettings {
    pub muted: bool,
}

impl Audio {
    fn open(settings: AudioSettings) -> Self {
        let sink = match rodio::DeviceSinkBuilder::open_default_sink() {
            Ok(mut s) => {
                s.log_on_drop(false);
                Some(s)
            }
            Err(e) => {
                runlog::kv("audio_device", &format!("ok=false reason=\"{e}\""));
                None
            }
        };
        let out_rate = sink.as_ref().map_or(44100, |s| s.config().sample_rate().get());
        let shared = Arc::new(Mutex::new(MixState::default()));
        if let Some(s) = &sink {
            let rate = s.config().sample_rate();
            runlog::kv("audio_device", &format!("ok=true rate={} channels={} muted={}", rate.get(), s.config().channel_count().get(), settings.muted));
            s.mixer().add(MixSource { shared: shared.clone(), buf: vec![0.0; BLOCK * 2], at: BLOCK * 2, rate });
        }
        Audio { _sink: sink, shared, voices: Vec::new(), next_id: 1, out_rate, muted: settings.muted }
    }
}

pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        let root = app.world().resource::<crate::map::MapRequest>().install_root.clone();
        let settings = app.world().get_resource::<AudioSettings>().copied().unwrap_or_default();
        app.insert_non_send(SoundBank::new(root))
            .insert_resource(Audio::open(settings))
            .add_message::<PlaySound>()
            .add_message::<PreloadSounds>()
            .add_systems(Update, test_sounds)
            .add_systems(PostUpdate, (preload_sounds, play_sounds, sync_ambient_sounds, update_voices).chain().after(bevy::transform::TransformSystems::Propagate));
    }
}

/// Test actions: `sound:Package.Name` plays at the listener;
/// `sound_at:Package.Name@DIST` plays DIST Unreal units to the listener's
/// right (to hear the falloff and the panning). Both at volume 1.8, KF's
/// gunshot volume (SingleFire TransientSoundVolume), so they are easy to
/// hear; the default 0.3 is quiet.
fn test_sounds(
    script: Res<crate::weapon::ScriptedInput>,
    frames: Res<bevy::diagnostic::FrameCount>,
    listener: Query<&GlobalTransform, With<FlyCamera>>,
    mut out: MessageWriter<PlaySound>,
) {
    for (_, action) in script.0.iter().filter(|(f, _)| *f == frames.0) {
        if let Some(name) = action.strip_prefix("sound:") {
            out.write(PlaySound::new(name, Emitter::Listener).volume(TEST_VOLUME));
        } else if let Some(rest) = action.strip_prefix("sound_at:") {
            let (name, dist) = rest.split_once('@').unwrap_or((rest, "0"));
            let dist: f32 = dist.parse().unwrap_or(0.0);
            let Ok(cam) = listener.single() else {
                continue;
            };
            let at = cam.translation() + cam.right() * dist * crate::coords::SCALE;
            out.write(PlaySound::new(name, Emitter::Point(at)).radius(DEFAULT_RADIUS.max(dist * 2.0)).volume(TEST_VOLUME));
        }
    }
}

fn preload_sounds(mut requests: MessageReader<PreloadSounds>, mut bank: NonSendMut<SoundBank>) {
    for req in requests.read() {
        let started = std::time::Instant::now();
        let found = req.sounds.iter().filter(|s| bank.lookup(s).is_some()).count();
        runlog::kv(
            "sound_preload",
            &format!("what={} sounds={} missing={} ms={:.1}", req.what, req.sounds.len(), req.sounds.len() - found, started.elapsed().as_secs_f64() * 1000.0),
        );
    }
}

/// Starts the requested sounds: slot rules, then the 32-voice limit.
fn play_sounds(
    mut requests: MessageReader<PlaySound>,
    mut bank: NonSendMut<SoundBank>,
    mut audio: ResMut<Audio>,
    listener: Query<&GlobalTransform, With<FlyCamera>>,
    positions: Query<&GlobalTransform>,
) {
    let ear = listener.single().ok().map(|t| t.translation());
    for req in requests.read() {
        let Some(entry) = bank.lookup(&req.sound) else {
            continue;
        };
        // Slots belong to an entity, or to the listener (the player's own
        // sounds); sounds at a bare point have none.
        let same_slot = |v: &VoiceInfo| req.slot != Slot::None && v.slot == req.slot && v.emitter == req.emitter && v.actor == req.actor && !matches!(req.emitter, Emitter::Point(_));
        if let Some(busy) = audio.voices.iter().find(|v| same_slot(v)) {
            if req.no_override {
                runlog::kv("sound_skip", &format!("sound={} reason=slot_busy slot={:?} playing={}", req.sound, req.slot, busy.sound));
                continue;
            }
            let ids: Vec<u64> = audio.voices.iter().filter(|v| same_slot(v)).map(|v| v.id).collect();
            for id in ids {
                stop_voice(&mut audio, id, "slot_override");
            }
        }
        // Too far to hear: not started (KF culls them too).
        let at = emitter_position(req.emitter, ear, &positions);
        let distance = match (at, ear) {
            (Some(p), Some(e)) => p.distance(e) / crate::coords::SCALE,
            _ => 0.0,
        };
        if req.emitter != Emitter::Listener && distance_gain(distance, req.radius) <= 0.0 {
            runlog::kv("sound_skip", &format!("sound={} reason=out_of_range distance={distance:.0} radius={:.0}", req.sound, req.radius));
            continue;
        }
        // All voices busy: drop the quietest if it is quieter than the new
        // one (a guess at what ALAudio does), else skip the new one.
        let loudness = req.volume * if req.emitter == Emitter::Listener { 1.0 } else { distance_gain(distance, req.radius) };
        if audio.voices.iter().filter(|v| v.ambient.is_none()).count() >= MAX_VOICES {
            let quietest = audio.voices.iter().filter(|v| v.ambient.is_none()).min_by(|a, b| a.loudness.total_cmp(&b.loudness)).map(|v| (v.id, v.loudness));
            match quietest {
                Some((id, l)) if l < loudness => stop_voice(&mut audio, id, "voice_limit"),
                _ => {
                    runlog::kv("sound_skip", &format!("sound={} reason=voice_limit", req.sound));
                    continue;
                }
            }
        }
        let clip = bank.pick(&entry);
        let info = VoiceInfo { id: 0, sound: req.sound.clone(), slot: req.slot, emitter: req.emitter, actor: req.actor, volume: req.volume, radius: req.radius, pitch: req.pitch, clip_rate: clip.rate, loudness, ambient: None };
        let id = start_voice(&mut audio, clip.clone(), info, false);
        runlog::kv(
            "sound_play",
            &format!(
                "id={id} sound={} slot={:?} volume={:.2} radius={:.0} pitch={:.2} distance={distance:.0} length={:.2} voices={}",
                req.sound,
                req.slot,
                req.volume,
                req.radius,
                req.pitch,
                clip.duration(),
                audio.voices.len()
            ),
        );
    }
}

/// Registers a voice and hands it to the mixer, silent; `update_voices`
/// sets its volume the same frame.
fn start_voice(audio: &mut Audio, clip: Arc<Clip>, mut info: VoiceInfo, looping: bool) -> u64 {
    let id = audio.next_id;
    audio.next_id += 1;
    info.id = id;
    let step = clip.rate as f64 / audio.out_rate as f64 * info.pitch as f64;
    audio.voices.push(info);
    if let Ok(mut s) = audio.shared.lock() {
        s.voices.push(MixVoice { id, step, clip, pos: 0.0, gain: [0.0; 2], target: [0.0; 2], stopping: false, looping });
    }
    id
}

/// Starts, changes and stops the AmbientSound voices.
fn sync_ambient_sounds(
    mut bank: NonSendMut<SoundBank>,
    mut audio: ResMut<Audio>,
    changed: Query<(Entity, &AmbientSound), Changed<AmbientSound>>,
    mut removed: RemovedComponents<AmbientSound>,
) {
    for e in removed.read() {
        if let Some(id) = audio.voices.iter().find(|v| v.ambient == Some(e)).map(|v| v.id) {
            stop_voice(&mut audio, id, "ambient_removed");
        }
    }
    for (e, amb) in &changed {
        let emitter = if amb.at_listener { Emitter::Listener } else { Emitter::Entity(e) };
        let pitch = amb.pitch as f32 / 64.0;
        if let Some(v) = audio.voices.iter_mut().find(|v| v.ambient == Some(e)) {
            if v.sound.eq_ignore_ascii_case(&amb.sound) {
                (v.volume, v.radius, v.pitch, v.emitter) = (amb.voice_volume(), amb.radius, pitch, emitter);
                continue;
            }
            let id = v.id;
            stop_voice(&mut audio, id, "ambient_changed");
        }
        let Some(entry) = bank.lookup(&amb.sound) else {
            continue;
        };
        let clip = bank.pick(&entry);
        let info = VoiceInfo { id: 0, sound: amb.sound.clone(), slot: Slot::Ambient, emitter, actor: 0, volume: amb.voice_volume(), radius: amb.radius, pitch, clip_rate: clip.rate, loudness: 0.0, ambient: Some(e) };
        let id = start_voice(&mut audio, clip.clone(), info, true);
        runlog::kv(
            "sound_ambient",
            &format!("id={id} sound={} volume={} radius={:.0} pitch={} length={:.2} loop={:?}", amb.sound, amb.volume, amb.radius, amb.pitch, clip.duration(), clip.loop_points),
        );
    }
}

fn stop_voice(audio: &mut Audio, id: u64, reason: &str) {
    if let Some(i) = audio.voices.iter().position(|v| v.id == id) {
        let v = audio.voices.remove(i);
        runlog::kv("sound_stop", &format!("id={id} sound={} reason={reason}", v.sound));
    }
    if let Ok(mut s) = audio.shared.lock()
        && let Some(v) = s.voices.iter_mut().find(|v| v.id == id)
    {
        // Faded out over one block, then removed.
        v.stopping = true;
    }
}

fn emitter_position(emitter: Emitter, ear: Option<Vec3>, positions: &Query<&GlobalTransform>) -> Option<Vec3> {
    match emitter {
        Emitter::Entity(e) => positions.get(e).ok().map(|t| t.translation()),
        Emitter::Point(p) => Some(p),
        Emitter::Listener => ear,
    }
}

/// Every frame: each voice's volume, balance and pitch from the listener.
/// Pitch follows the game speed (zed time slows sounds down; assumed from
/// how KF sounds, to be checked).
fn update_voices(
    mut audio: ResMut<Audio>,
    listener: Query<&GlobalTransform, With<FlyCamera>>,
    positions: Query<&GlobalTransform>,
    time: Res<Time<Virtual>>,
) {
    let cam = listener.single().ok();
    let ear = cam.map(|t| t.translation());
    let speed = time.relative_speed() as f64;
    let master = if audio.muted { 0.0 } else { SOUND_VOLUME };
    let out_rate = audio.out_rate as f64;
    // Gains first, then one short lock to hand them over.
    let mut updates = Vec::with_capacity(audio.voices.len());
    for v in &mut audio.voices {
        let (gain, pan) = match (v.emitter, cam) {
            (Emitter::Listener, _) | (_, None) => (1.0, 0.0),
            (e, Some(cam)) => match emitter_position(e, ear, &positions) {
                Some(p) => {
                    let to = p - cam.translation();
                    let d = to.length();
                    let pan = if d > 1e-3 { to.dot(*cam.right()) / d } else { 0.0 };
                    (distance_gain(d / crate::coords::SCALE, v.radius), pan)
                }
                // The entity is gone: KF keeps playing at its last place;
                // ours fades out (not tracked yet).
                None => (0.0, 0.0),
            },
        };
        v.loudness = v.volume * gain;
        let g = pan_gains(pan).map(|x| x * voice_gain(v.loudness, master));
        updates.push((v.id, g, v.clip_rate as f64 / out_rate * v.pitch as f64 * speed));
    }
    let mut finished = Vec::new();
    if let Ok(mut s) = audio.shared.lock() {
        for (id, g, step) in updates {
            if let Some(m) = s.voices.iter_mut().find(|m| m.id == id) {
                m.target = g;
                m.step = step;
            }
        }
        finished = std::mem::take(&mut s.finished);
    }
    if !finished.is_empty() {
        audio.voices.retain(|v| !finished.contains(&v.id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falloff_is_linear_to_the_radius() {
        assert_eq!(distance_gain(0.0, 300.0), 1.0);
        assert_eq!(distance_gain(150.0, 300.0), 0.5);
        assert_eq!(distance_gain(300.0, 300.0), 0.0);
        assert_eq!(distance_gain(900.0, 300.0), 0.0);
    }

    #[test]
    fn panning_keeps_the_near_ear_full() {
        assert_eq!(pan_gains(0.0), [1.0, 1.0]);
        assert_eq!(pan_gains(1.0), [0.0, 1.0]);
        assert_eq!(pan_gains(-0.5), [1.0, 0.5]);
    }

    #[test]
    fn mixer_plays_a_clip_to_its_end_and_reports_it() {
        let clip = Arc::new(Clip { rate: 100, channels: 1, samples: vec![0.5; 10], loop_points: None });
        let mut s = MixState::default();
        s.voices.push(MixVoice { id: 7, clip, pos: 0.0, step: 1.0, gain: [1.0, 0.5], target: [1.0, 0.5], stopping: false, looping: false });
        let mut out = vec![0.0; 32];
        s.render(&mut out);
        assert_eq!(&out[..4], &[0.5, 0.25, 0.5, 0.25]);
        // 10 frames played, then silence.
        assert_eq!(out[2 * 10], 0.0);
        assert!(s.voices.is_empty());
        assert_eq!(s.finished, vec![7]);
    }

    #[test]
    fn looping_voices_wrap_at_the_loop_points() {
        // Frames 0..4 are 0.1 .. 0.4; the loop repeats frames 2..4.
        let clip = Arc::new(Clip { rate: 100, channels: 1, samples: vec![0.1, 0.2, 0.3, 0.4], loop_points: Some((2, 4)) });
        let mut s = MixState::default();
        s.voices.push(MixVoice { id: 1, clip, pos: 0.0, step: 1.0, gain: [1.0; 2], target: [1.0; 2], stopping: false, looping: true });
        let mut out = vec![0.0; 16];
        s.render(&mut out);
        // Half steps through the end of the loop must not stall it.
        s.voices[0].step = 0.5;
        let mut more = vec![0.0; 64];
        s.render(&mut more);
        assert!(more.chunks(2).skip(24).all(|f| f[0] > 0.0));
        let left: Vec<f32> = out.chunks(2).map(|f| (f[0] * 10.0).round() / 10.0).collect();
        assert_eq!(left, vec![0.1, 0.2, 0.3, 0.4, 0.3, 0.4, 0.3, 0.4]);
        assert!(s.finished.is_empty());
    }

    #[test]
    fn gain_is_capped_at_one() {
        assert!((voice_gain(1.8, 0.3) - 0.54).abs() < 1e-6);
        assert_eq!(voice_gain(100.0, 0.3), 1.0);
    }

    #[test]
    fn half_speed_doubles_the_length() {
        let clip = Arc::new(Clip { rate: 100, channels: 1, samples: vec![1.0; 10], loop_points: None });
        let mut s = MixState::default();
        s.voices.push(MixVoice { id: 1, clip, pos: 0.0, step: 0.5, gain: [1.0; 2], target: [1.0; 2], stopping: false, looping: false });
        let mut out = vec![0.0; 64];
        s.render(&mut out);
        let played = out.chunks(2).filter(|f| f[0] != 0.0).count();
        assert_eq!(played, 19);
    }
}
