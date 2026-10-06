//! The map's own sounds (milestone 10, S5a): AmbientSound actors (loops
//! and SoundEmitters' random one-shots), other actors with an AmbientSound
//! (ZoneInfo, ...), and ScriptedTrigger sounds that wait for an event the
//! player start fires (the helicopter taking off on KF-WestLondon and
//! KF-Farm). See DESIGN.md, "Sound and music".

use bevy::prelude::*;
use ue_assets::package::{ObjectRef, Package};
use ue_assets::package_set::PackageSet;
use ue_assets::properties::{PropertyList, Value};

use crate::audio::{AmbientSound, Emitter, Falloff, PlaySound, Slot};
use crate::runlog;

/// Engine.AmbientSound defaults (Actor: 128 and 64 for other classes).
const AMBIENT_SOUND_VOLUME: u8 = 100;
const AMBIENT_SOUND_RADIUS: f32 = 100.0;
/// KillingFloor.ini [Engine.AmbientSound] AmbientVolume as shipped.
const AMBIENT_VOLUME: f32 = 0.25;

/// AmbientSound.SoundEmitters: one sound every EmitInterval +- a random
/// part of EmitVariance.
#[derive(Component)]
struct RandomSounds {
    emitters: Vec<(f32, f32, String)>,
    /// Game seconds when each plays next.
    next: Vec<f32>,
    volume: f32,
    radius: f32,
    pitch: f32,
    rng: u32,
}

/// A sound waiting for the player start's event (ScriptedTrigger:
/// WAITFOREVENT, then PLAYSOUND).
#[derive(Resource, Default)]
struct SpawnSounds(Vec<(String, f32, bool, Vec3)>);

pub struct MapSoundPlugin;

impl Plugin for MapSoundPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpawnSounds>().add_systems(Startup, load_map_sounds).add_systems(Update, (random_sounds, spawn_sounds));
    }
}

fn object_path(pkg: &Package, rf: ObjectRef) -> Option<String> {
    match rf {
        ObjectRef::Import(_) => Some(pkg.object_path(rf)),
        _ => None,
    }
}

fn load_map_sounds(mut commands: Commands, request: Res<crate::map::MapRequest>, mut spawn: ResMut<SpawnSounds>) {
    let path = request.install_root.join("Maps").join(format!("{}.rom", request.map));
    let set = PackageSet::new(&request.install_root);
    let Ok(lp) = set.load_path(&path) else {
        return;
    };
    let pkg = &lp.pkg;
    let scale = ambient_volume(&request.install_root);
    let (mut loops, mut randoms, mut silent) = (0, 0, 0);
    let mut sounds: Vec<String> = Vec::new();
    let mut player_events: Vec<String> = Vec::new();
    let mut triggers: Vec<PropertyList> = Vec::new();
    for i in 0..pkg.exports.len() {
        let class = pkg.export_class_name(i);
        // Movers carry their own sounds (S5b); pawns are not placed in maps.
        if class.contains("Mover") || class == "KFTraderDoor" {
            continue;
        }
        let is_ambient = class == "AmbientSound";
        let struct_arrays: &[&str] = if is_ambient { &["SoundEmitters"] } else { &[] };
        let Ok(props) = ue_assets::properties::read_export_properties_ext(pkg, i, struct_arrays) else {
            continue;
        };
        match class {
            "PlayerStart" => {
                if let Some(Value::Name(n)) = props.get(pkg, "Event") {
                    player_events.push(pkg.name(*n).to_ascii_lowercase());
                }
                continue;
            }
            "ScriptedTrigger" => {
                triggers.push(props);
                continue;
            }
            _ => {}
        }
        let ambient = match props.get(pkg, "AmbientSound") {
            Some(Value::Object(rf)) => object_path(pkg, *rf),
            _ => None,
        };
        let emitters: Vec<(f32, f32, String)> = match props.get(pkg, "SoundEmitters") {
            Some(Value::StructArray(items)) => items
                .iter()
                .filter_map(|e| {
                    let f = |n: &str| match e.get(pkg, n) {
                        Some(Value::Float(x)) => *x,
                        _ => 0.0,
                    };
                    let sound = match e.get(pkg, "EmitSound") {
                        Some(Value::Object(rf)) => object_path(pkg, *rf),
                        _ => None,
                    }?;
                    Some((f("EmitInterval"), f("EmitVariance"), sound))
                })
                .collect(),
            _ => Vec::new(),
        };
        if ambient.is_none() && emitters.is_empty() {
            if is_ambient {
                silent += 1;
            }
            continue;
        }
        let volume = match props.get(pkg, "SoundVolume") {
            Some(Value::Byte(b)) => *b,
            _ if is_ambient => AMBIENT_SOUND_VOLUME,
            _ => 128,
        };
        let radius = match props.get(pkg, "SoundRadius") {
            Some(Value::Float(r)) => *r,
            _ if is_ambient => AMBIENT_SOUND_RADIUS,
            _ => 64.0,
        };
        // SoundPitch 0 (18 actors in KF's maps) would stop the sound; taken
        // as normal pitch (what the engine does is not known).
        let pitch = match props.get(pkg, "SoundPitch") {
            Some(Value::Byte(0)) | None => 64,
            Some(Value::Byte(b)) => *b,
            _ => 64,
        };
        let full_volume = matches!(props.get(pkg, "bFullVolume"), Some(Value::Bool(true)));
        let location = match props.get(pkg, "Location") {
            Some(Value::Vector(v)) => *v,
            _ => [0.0; 3],
        };
        let mut e = commands.spawn((Transform::from_translation(crate::coords::pos(location)), Name::new(format!("map sound {}", pkg.object_name(ObjectRef::Export(i))))));
        if let Some(sound) = ambient {
            loops += 1;
            sounds.push(sound.clone());
            e.insert(AmbientSound { sound, volume, radius, pitch, at_listener: false, falloff: Falloff::Radius { full_volume }, scale });
        }
        if !emitters.is_empty() {
            randoms += 1;
            sounds.extend(emitters.iter().map(|x| x.2.clone()));
            let n = emitters.len();
            e.insert(RandomSounds {
                emitters,
                next: vec![f32::NAN; n],
                volume: volume as f32 / 128.0 * scale,
                radius,
                pitch: pitch as f32 / 64.0,
                rng: 0x9e37_79b9 ^ i as u32,
            });
        }
    }
    // ScriptedTrigger: Actions = [WAITFOREVENT (ExternalEvent), PLAYSOUND
    // (Sound, Volume, Pitch, bAttenuate)], played when a player start
    // fires that event (GameInfo.RestartPlayer: TriggerEvent(StartSpot.Event)).
    for t in &triggers {
        let Some(Value::Array { count, raw }) = t.get(pkg, "Actions") else {
            continue;
        };
        let mut r = ue_assets::reader::Reader::new(raw);
        let actions: Vec<ObjectRef> = (0..*count).filter_map(|_| r.compact_index().ok().map(ObjectRef::from_raw)).collect();
        let mut event = None;
        let at = match t.get(pkg, "Location") {
            Some(Value::Vector(v)) => crate::coords::pos(*v),
            _ => Vec3::ZERO,
        };
        for a in actions {
            let ObjectRef::Export(ai) = a else {
                continue;
            };
            let Ok(p) = ue_assets::properties::read_export_properties(pkg, ai) else {
                continue;
            };
            match pkg.export_class_name(ai).to_ascii_lowercase().as_str() {
                "action_waitforevent" => {
                    if let Some(Value::Name(n)) = p.get(pkg, "ExternalEvent") {
                        event = Some(pkg.name(*n).to_ascii_lowercase());
                    }
                }
                "action_playsound" => {
                    let sound = match p.get(pkg, "Sound") {
                        Some(Value::Object(rf)) => object_path(pkg, *rf),
                        _ => None,
                    };
                    let volume = match p.get(pkg, "Volume") {
                        Some(Value::Float(v)) => *v,
                        _ => 1.0,
                    };
                    let attenuate = !matches!(p.get(pkg, "bAttenuate"), Some(Value::Bool(false)));
                    if let (Some(sound), Some(ev)) = (sound, &event)
                        && player_events.contains(ev)
                    {
                        runlog::kv("map_sound_on_spawn", &format!("event={ev} sound={sound} volume={volume} attenuate={attenuate}"));
                        sounds.push(sound.clone());
                        spawn.0.push((sound, volume, attenuate, at));
                    }
                }
                _ => {}
            }
        }
    }
    sounds.sort();
    sounds.dedup();
    runlog::kv("map_sounds", &format!("loops={loops} random={randoms} silent_ambient_actors={silent} ambient_volume={scale} distinct_sounds={}", sounds.len()));
    commands.write_message(crate::audio::PreloadSounds { what: format!("map {}", request.map), sounds });
}

/// [Engine.AmbientSound] AmbientVolume from the install's KillingFloor.ini.
fn ambient_volume(root: &std::path::Path) -> f32 {
    let text = std::fs::read_to_string(root.join("System").join("KillingFloor.ini")).unwrap_or_default();
    let mut in_section = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_section = line.eq_ignore_ascii_case("[Engine.AmbientSound]");
        } else if in_section
            && let Some((k, v)) = line.split_once('=')
            && k.trim().eq_ignore_ascii_case("AmbientVolume")
            && let Ok(x) = v.trim().parse()
        {
            return x;
        }
    }
    AMBIENT_VOLUME
}

/// AmbientSound's SoundEmitters (native): when the time comes, the sound
/// plays at the actor and the next time is EmitInterval +- a random part
/// of EmitVariance away (the first within one interval). Volume and
/// radius: the actor's ambient values (a guess).
fn random_sounds(time: Res<Time>, mut actors: Query<(&Transform, &mut RandomSounds)>, mut out: MessageWriter<PlaySound>) {
    let now = time.elapsed_secs();
    for (t, mut r) in &mut actors {
        for k in 0..r.emitters.len() {
            r.rng ^= r.rng << 13;
            r.rng ^= r.rng >> 17;
            r.rng ^= r.rng << 5;
            let roll = (r.rng % 10000) as f32 / 10000.0;
            let (interval, variance) = (r.emitters[k].0, r.emitters[k].1);
            if r.next[k].is_nan() {
                r.next[k] = now + interval * roll;
                continue;
            }
            if now >= r.next[k] {
                r.next[k] = now + (interval + variance * (2.0 * roll - 1.0)).max(0.1);
                out.write(PlaySound::new(r.emitters[k].2.clone(), Emitter::Point(t.translation)).volume(r.volume).radius(r.radius).pitch(r.pitch));
            }
        }
    }
}

/// ACTION_PlaySound when the player start's event fires (the player
/// spawns: once, at the start): SLOT_Interact, its Volume; bAttenuate
/// false = not faded by distance (played at the listener; a guess at the
/// native flag).
fn spawn_sounds(frames: Res<bevy::diagnostic::FrameCount>, mut spawn: ResMut<SpawnSounds>, mut out: MessageWriter<PlaySound>) {
    if frames.0 < 30 || spawn.0.is_empty() {
        return;
    }
    for (sound, volume, attenuate, at) in std::mem::take(&mut spawn.0) {
        // Played by the ScriptedTrigger (GetSoundSource), radius its
        // TransientSoundRadius (300).
        let emitter = if attenuate { Emitter::Point(at) } else { Emitter::Listener };
        out.write(PlaySound::new(sound, emitter).slot(Slot::Interact).volume(volume).actor(3));
    }
}
