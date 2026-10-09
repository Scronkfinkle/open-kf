//! What other players see and hear when a pawn that is not the local
//! player's fires (multiplayer; single player has no such pawns, so this
//! does nothing there).
//!
//! KF runs this on every machine that does not own the weapon:
//! - KFWeaponAttachment.ThirdPersonEffects (on each FlashCount change):
//!   DoFlashEmitter spawns the attachment's mMuzFlashClass once, attaches it
//!   to the `tip` bone, and calls SpawnParticle(1) per shot.
//!   WeaponLight: the attachment's own light for 0.15 s
//!   (weapons/muzzle_light.rs), here at the `tip` bone (KF: the
//!   attachment's origin, in the hand; **approximation**).
//! - WeaponFire / KFFire.PlayFiring on a non-owner: FireSound (the owner
//!   hears StereoFireSound instead), SLOT_Interact, TransientSoundVolume,
//!   TransientSoundRadius, pitch 1 +- RandomPitchAdjustAmt x FRand. At the
//!   pawn, so it fades with distance.
//! - KFHighROFFire state FireLoop (full auto): no per-shot sound; the
//!   attachment's AmbientSound is AmbientFireSound (SoundVolume
//!   AmbientFireVolume, SoundRadius AmbientFireSoundRadius) while firing,
//!   and EndState plays FireEndSound at AmbientFireVolume / 127.

use std::collections::HashMap;

use bevy::prelude::*;

use super::load::BodyModels;
use super::{PawnBody, PawnState};
use crate::audio::mixer::{AmbientSound, Emitter, PlaySound, Slot};
use crate::engine::runlog;
use crate::render::particles::{EffectLibrary, ParticleEffect, SpawnOptions};
use crate::weapons::muzzle_light::MuzzleLight;

/// Per remote pawn: the shots seen, the flash emitter, the fire loop.
#[derive(Default)]
pub(super) struct RemoteFire {
    seen_flash: Option<u32>,
    attachment: Option<usize>,
    flash: Option<Entity>,
    /// The fire mode whose AmbientFireSound loops on the pawn.
    looping: Option<usize>,
    rng: u32,
    shots: u32,
}

impl RemoteFire {
    /// FRand (xorshift).
    fn frand(&mut self) -> f32 {
        if self.rng == 0 {
            self.rng = 0x2545_f491;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }
}

fn kill(effects: &mut Query<&mut ParticleEffect>, e: Option<Entity>) {
    if let Some(e) = e
        && let Ok(mut fx) = effects.get_mut(e)
    {
        fx.kill();
    }
}

/// The sound "actor" of a weapon on a pawn (its own slots, as KF's weapon
/// actor; see weapons/weapon/sounds.rs `weapon_actor`).
fn actor_of(pawn: Entity, class: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (pawn, class).hash(&mut h);
    h.finish() | 1
}

#[allow(clippy::too_many_arguments)] // Bevy system parameters
pub(super) fn remote_fire_effects(
    mut commands: Commands,
    models: Res<BodyModels>,
    pawns: Query<(Entity, &PawnState, &PawnBody)>,
    library: Option<Res<EffectLibrary>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut effects: Query<&mut ParticleEffect>,
    mut sounds: MessageWriter<PlaySound>,
    mut state: Local<HashMap<Entity, RemoteFire>>,
    mut muzzles: Query<&mut MuzzleLight>,
) {
    let mut seen = Vec::new();
    for (e, s, body) in &pawns {
        if s.local {
            continue;
        }
        seen.push(e);
        let fx = state.entry(e).or_insert_with(|| RemoteFire { rng: e.index_u32() ^ 0x9e37_79b9, ..default() });
        let att = body.attachment.map(|i| &models.attachments[i]);
        // A new weapon in hand (or none, dead): the old flash and loop go.
        if fx.attachment != body.attachment || s.dead || !s.active {
            kill(&mut effects, fx.flash.take());
            if fx.attachment != body.attachment && fx.looping.take().is_some() {
                commands.entity(e).remove::<AmbientSound>();
            }
            fx.attachment = body.attachment;
        }
        let shots = fx.seen_flash.map_or(0, |seen| s.flash_count.wrapping_sub(seen));
        fx.seen_flash = Some(s.flash_count);
        let mode = usize::from(s.firing_mode.min(1));
        let Some(a) = att else { continue };
        let class = body.weapon_class.clone().unwrap_or_default();
        // WeaponLight (the attachment's light, seen in third person).
        match muzzles.get_mut(e) {
            Ok(mut m) => {
                m.set_weapon(&a.class, a.light, &body.who);
                m.pos = body.tip.filter(|_| !s.dead).map(|(p, _)| crate::engine::coords::pos(p.to_array()));
                if shots > 0 && shots < 100 && !s.dead && a.light_rule.allows(s.firing_mode) {
                    m.flash();
                }
            }
            Err(_) => {
                commands.entity(e).insert(MuzzleLight::default());
            }
        }
        let fs = &a.fire_sounds[mode];
        if shots > 0 && shots < 100 && !s.dead {
            fx.shots += shots;
            // PlayFiring (not in the full-auto loop): one FireSound; a
            // second in the same frame would cut the first (SLOT_Interact).
            let mut played = None;
            if fs.ambient.is_none()
                && let Some(sound) = &fs.sound
            {
                let mut pitch = 1.0;
                if fs.random_pitch > 0.0 {
                    let r = fx.frand() * fs.random_pitch;
                    pitch += if fx.frand() < 0.5 { -r } else { r };
                }
                sounds.write(PlaySound::new(sound.clone(), Emitter::Entity(e)).slot(Slot::Interact).volume(fs.volume).radius(fs.radius).pitch(pitch).actor(actor_of(e, &class)));
                played = Some(sound.as_str());
            }
            // DoFlashEmitter: spawned once, then SpawnParticle(1) per shot
            // (the first call spawns it and triggers it too).
            let mut flashed = false;
            if let (Some(flash), Some(tip), Some(lib)) = (&a.muzzle_flash, body.tip, library.as_deref()) {
                if fx.flash.is_none() {
                    let options = SpawnOptions { persistent: true, ..default() };
                    fx.flash = crate::render::particles::spawn_effect_with(&mut commands, lib, &mut meshes, flash, tip.0, tip.1, fx.rng, options);
                }
                if let Some(f) = fx.flash
                    && let Ok(mut p) = effects.get_mut(f)
                {
                    for _ in 0..shots.min(3) {
                        p.spawn_all(1);
                    }
                    flashed = true;
                }
            }
            runlog::kv(
                "remote_fire",
                &format!(
                    "who={} weapon={class} mode={mode} shots={shots} total={} sound={} flash={} flash_class={} tip_unreal={}",
                    body.who,
                    fx.shots,
                    played.unwrap_or(if fs.ambient.is_some() { "loop" } else { "none" }),
                    if flashed { "triggered" } else if fx.flash.is_some() { "spawned" } else { "none" },
                    a.muzzle_flash.as_deref().unwrap_or("none"),
                    body.tip.map_or("none".to_string(), |(p, _)| format!("({:.0},{:.0},{:.0})", p.x, p.y, p.z)),
                ),
            );
        }
        // Follow the tip (AttachToBone).
        if let (Some(f), Some(tip)) = (fx.flash, body.tip)
            && let Ok(mut p) = effects.get_mut(f)
        {
            p.frame = tip;
        }
        // The full-auto loop on the pawn (the attachment's AmbientSound).
        let want = (s.firing && !s.dead && s.active && fs.ambient.is_some()).then_some(mode);
        if want != fx.looping {
            match want.and_then(|m| a.fire_sounds[m].ambient.as_ref().map(|snd| (m, snd))) {
                Some((m, snd)) => {
                    let f = &a.fire_sounds[m];
                    commands.entity(e).insert(AmbientSound { sound: snd.clone(), volume: f.ambient_volume, radius: f.ambient_radius, pitch: 64, ..default() });
                }
                None => {
                    commands.entity(e).remove::<AmbientSound>();
                }
            }
            if let Some(old) = fx.looping
                && let Some(end) = &a.fire_sounds[old].end
            {
                let f = &a.fire_sounds[old];
                sounds.write(PlaySound::new(end.clone(), Emitter::Entity(e)).volume(f.ambient_volume as f32 / 127.0).radius(f.ambient_radius).actor(actor_of(e, &class)));
            }
            runlog::kv("remote_fire_loop", &format!("who={} weapon={class} looping={want:?} was={:?}", body.who, fx.looping));
            fx.looping = want;
        }
    }
    // Pawns gone (the player left): their flash goes.
    state.retain(|e, fx| {
        let keep = seen.contains(e);
        if !keep {
            kill(&mut effects, fx.flash.take());
        }
        keep
    });
}
