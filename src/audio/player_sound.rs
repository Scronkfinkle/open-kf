//! The player's own sounds (milestone 10, S4c): pain, death, footsteps,
//! jump and landing, low-health breathing. See DESIGN.md, "Sound and
//! music". Values from KFHumanPawn, KFPawn, xPawn and KFMaleSoundGroup.
//!
//! Every sound plays at the player (`Emitter::Listener`, the pawn's slots).
//! Footsteps, jumps and landings pick their sound by the surface under the
//! feet (KFPawn.FootStepping / GetSound; see DESIGN.md, "Surface types").

use avian3d::prelude::SpatialQuery;
use bevy::prelude::*;

use crate::audio::mixer::{Emitter, PlaySound, Slot};
use crate::engine::camera::FlyCamera;
use crate::engine::coords::SCALE;
use crate::engine::runlog;
use crate::player::walk::Walker;

/// KFMaleSoundGroup.PainSounds: six times the same group.
const PAIN_SOUND: &str = "Inf_Player.playerhurt.Wounding";
/// KFMaleSoundGroup.DeathSounds (GetDeathSound: one at random).
const DEATH_SOUNDS: [&str; 5] = [
    "Inf_Player.playerdeath.Generic",
    "Inf_Player.playerdeath.Headshot",
    "Inf_Player.playerdeath.UpperBodyShot",
    "Inf_Player.playerdeath.LowerBodyShot",
    "Inf_Player.playerdeath.LimbShot",
];
/// KFPawn.SoundFootsteps[20], KFMaleSoundGroup.JumpSounds[20] and
/// LandSounds[20], by ESurfaceTypes (class defaults).
const STEP_SOUNDS: [&str; 20] = [
    "Default", "Dirt", "Dirt", "Metal", "Wood", "Grass", "Dirt", "Default", "Default", "Water", "BrGlass", "Default", "Conc", "Wood", "Default", "Metal", "Default",
    "Default", "Default", "Default",
];
const JUMP_SOUNDS: [&str; 20] = [
    "Dirt", "Asphalt", "Dirt", "Metal", "Wood", "Grass", "Dirt", "SnowRough", "SnowHard", "WaterShallow", "Dirt", "Dirt", "Asphalt", "Wood", "Mud", "Metal", "Asphalt",
    "Dirt", "Dirt", "Dirt",
];
const LAND_SOUNDS: [&str; 20] = [
    "Default", "Conc", "Dirt", "Metal", "Wood", "Grass", "Default", "Dirt", "Dirt", "Water", "Dirt", "Dirt", "Conc", "Wood", "Dirt", "Metal", "Conc", "Default", "Default",
    "Default",
];
/// KFMaleSoundGroup.BreathingSound.
const BREATH_SOUND: &str = "KFPlayerSound.Malebreath";

/// The player pawn's TransientSoundVolume and Radius (Actor defaults, not
/// overridden in its class chain).
const PAWN_VOLUME: f32 = 0.3;
const PAWN_RADIUS: f32 = 300.0;
/// KFPawn FootstepVolume and FootStepSoundRadius.
const FOOTSTEP_VOLUME: f32 = 0.45;
const FOOTSTEP_RADIUS: f32 = 125.0;
/// xPawn GruntVolume; KFPawn JumpZ.
const GRUNT_VOLUME: f32 = 0.5;
const JUMP_Z: f32 = 325.0;
/// Ours, not KF's: our walker leaves the ground for a frame on steps and
/// edges (UE2 stays in PHYS_Walking there), and each of those "landings"
/// would play (and cut the last one off, SLOT_Interact). Landings slower
/// than this (Unreal units/s) are not played.
const MIN_LANDING_SPEED: f32 = 100.0;
/// xPawn MinTimeBetweenPainSounds.
const MIN_TIME_BETWEEN_PAIN_SOUNDS: f32 = 0.35;

/// Events from the damage code (combat.rs).
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub enum PlayerSoundEvent {
    /// Damage taken while alive (KFPawn.TakeDamage -> PlayTakeHit).
    Hurt,
    Died,
}

pub struct PlayerSoundPlugin;

impl Plugin for PlayerSoundPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<PlayerSoundEvent>().add_systems(Update, player_sounds);
    }
}

#[derive(Default)]
struct State {
    /// LastPainSound and LastPainTime (game seconds).
    last_pain_sound: f32,
    last_pain_time: f32,
    /// The pending death sound: game seconds when it plays.
    death_at: Option<f32>,
    /// CheckBob's last step count, int(0.5 Pi + 9 BobTime / Pi).
    last_step: i32,
    was_on_ground: bool,
    /// Vertical speed while in the air (Unreal units/s), for the landing.
    fall_speed: f32,
    /// KFHumanPawn.Timer (every 1.5 s).
    breath_timer: f32,
    rng: u32,
}

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn player_sounds(
    spatial: SpatialQuery,
    surfaces: Query<(&avian3d::prelude::Collider, &GlobalTransform, &crate::world::collision::SurfaceMap)>,
    time: Res<Time>,
    mut events: MessageReader<PlayerSoundEvent>,
    health: Res<crate::game::combat::PlayerHealth>,
    walker: Query<&Walker, With<FlyCamera>>,
    mut out: MessageWriter<PlaySound>,
    mut st: Local<State>,
) {
    let now = time.elapsed_secs();
    if st.rng == 0 {
        // A fixed seed, as elsewhere in the project.
        *st = State { last_pain_sound: f32::MIN, last_pain_time: f32::MIN, rng: 0x5bd1_e995, ..default() };
    }
    let at = Emitter::Listener;
    for e in events.read() {
        match e {
            // KFPawn.TakeDamage calls PlayTakeHit when alive and more than
            // 0.1 s after the last pain; xPawn.PlayTakeHit: at most every
            // MinTimeBetweenPainSounds, GetHitSound at SLOT_Pain,
            // 2 x TransientSoundVolume, radius 200.
            PlayerSoundEvent::Hurt => {
                let ok = now - st.last_pain_time > 0.1 && now - st.last_pain_sound >= MIN_TIME_BETWEEN_PAIN_SOUNDS;
                st.last_pain_time = now;
                if ok {
                    st.last_pain_sound = now;
                    out.write(PlaySound::new(PAIN_SOUND, at).slot(Slot::Pain).volume(2.0 * PAWN_VOLUME).radius(200.0));
                }
            }
            // Pawn state Dying: Sleep(0.2), then KFPawn.PlayDyingSound.
            PlayerSoundEvent::Died => st.death_at = Some(now + 0.2),
        }
    }
    if st.death_at.is_some_and(|t| now >= t) {
        st.death_at = None;
        st.rng = st.rng.wrapping_mul(1_103_515_245).wrapping_add(12345);
        let pick = DEATH_SOUNDS[(st.rng >> 16) as usize % DEATH_SOUNDS.len()];
        out.write(PlaySound::new(pick, at).slot(Slot::Pain).volume(2.5 * PAWN_VOLUME).radius(500.0).no_override());
    }
    // KFHumanPawn.Timer: under a quarter of HealthMax, Malebreath at
    // ((50 - Health) / 5) x TransientSoundVolume, an integer division.
    st.breath_timer += time.delta_secs();
    if st.breath_timer >= 1.5 {
        st.breath_timer -= 1.5;
        let h = health.health.floor() as i32;
        if h > 0 && h < 25 {
            let volume = ((50 - h) / 5) as f32 * PAWN_VOLUME;
            out.write(PlaySound::new(BREATH_SOUND, at).slot(Slot::Talk).volume(volume).radius(PAWN_RADIUS));
        }
    }
    let Ok(w) = walker.single() else {
        return;
    };
    // FootStepping: the Base actor's SurfaceType if not the level and not
    // 0, else the material 16 units below the feet (Trace from
    // Location - CollisionHeight). Water volumes (the deep-water step) are
    // not done.
    let surface = || {
        let feet = w.center - Vec3::Y * crate::player::walk::kf::HALF_HEIGHT * SCALE;
        let hit = spatial.cast_ray(feet, Dir3::NEG_Y, 16.0 * SCALE, true, &crate::world::collision::world_filter())?;
        let [material, actor] = crate::world::collision::surface_of_hit(&surfaces, hit.entity, feet, Vec3::NEG_Y, 16.0 * SCALE)?;
        Some(if actor != 0 { actor } else { material })
    };
    let surface = |w_on: bool| if w_on { surface().unwrap_or(0).min(19) as usize } else { 0 };
    // Jump (xPawn.DoJump): GetSound(EST_Jump), SLOT_Pain, GruntVolume,
    // radius 80. Seen here as leaving the ground going up fast (walking off
    // a ledge is not a jump).
    let vz = w.velocity.y / SCALE;
    if st.was_on_ground && !w.on_ground && vz > 0.5 * JUMP_Z {
        let st = surface(true);
        out.write(PlaySound::new(format!("Inf_Player.footsteps.Jump{}", JUMP_SOUNDS[st]), at).slot(Slot::Pain).volume(GRUNT_VOLUME).radius(80.0));
    }
    // Landed (xPawn): GetSound(EST_Land), SLOT_Interact, volume
    // min(1, -0.3 x Velocity.Z / JumpZ), using the speed of the last
    // frame in the air (ours zeroes it on touching down).
    if !w.on_ground {
        st.fall_speed = vz;
    } else if !st.was_on_ground && st.fall_speed < -MIN_LANDING_SPEED {
        let volume = (-0.3 * st.fall_speed / JUMP_Z).min(1.0);
        let sf = surface(true);
        runlog::kv("player_landed", &format!("fall_speed_unreal={:.0} volume={volume:.2} surface={}", -st.fall_speed, crate::world::collision::surface_name(sf as u8)));
        out.write(PlaySound::new(format!("KF_PlayerGlobalSnd.Player_Land{}", LAND_SOUNDS[sf]), at).slot(Slot::Interact).volume(volume).radius(PAWN_RADIUS));
    }
    st.was_on_ground = w.on_ground;
    // KFPawn.CheckBob: a footstep when int(0.5 Pi + 9 BobTime / Pi)
    // changes, walking on the ground faster than 10 (FootStepping:
    // SoundFootsteps at FootstepVolume, SLOT_Interact, FootStepSoundRadius;
    // x QuietFootStepVolume crouched or walking, which we do not have).
    let step = (0.5 * std::f32::consts::PI + 9.0 * w.bob_time / std::f32::consts::PI) as i32;
    let speed2d = w.velocity.with_y(0.0).length() / SCALE;
    if step != st.last_step && w.on_ground && speed2d >= 10.0 {
        let sf = surface(true);
        out.write(PlaySound::new(format!("KF_PlayerGlobalSnd.Player_Step{}", STEP_SOUNDS[sf]), at).slot(Slot::Interact).volume(FOOTSTEP_VOLUME).radius(FOOTSTEP_RADIUS));
    }
    st.last_step = step;
}
