//! Each frame: which animations the body plays (KFPawn / xPawn rules and
//! our guesses for UE2's native movement animation), posing and skinning
//! the body, the weapon attachment on the right hand, and owner no-see.

use bevy::prelude::*;

use crate::render::actor_light::{ActorLight, LitPart};
use ue_assets::properties::Rotator;

use super::load::{AnimNames, BodyModel, BodyModels};
use super::{BaseKind, FireState, PawnBody, PawnState, Play};
use crate::engine::coords::{self, SCALE};
use crate::engine::runlog;
use crate::engine::view_target::ViewTarget;

/// xPawn BlendChangeTime: the tween between movement animations.
const BLEND_CHANGE_TIME: f32 = 0.25;
/// Guess: below this horizontal speed (Unreal units/s) the pawn is idle.
const IDLE_SPEED: f32 = 10.0;
/// Guess: the movement animations play at rate 1 at KFHumanPawn's
/// GroundSpeed (200), scaled by the speed, clamped.
const RUN_REFERENCE_SPEED: f32 = 200.0;
const RUN_RATE_RANGE: (f32, f32) = (0.4, 1.5);
/// Guess: turning in place starts above this yaw speed (Unreal units/s,
/// about 22 degrees/s) and lasts this long after it drops.
const TURN_START_RATE: f32 = 4000.0;
const TURN_HOLD: f32 = 0.2;
/// Guess: TakeoffStillAnim / AirStillAnim below this horizontal speed.
const STILL_AIR_SPEED: f32 = 50.0;
/// Guess: a landing animation only after this long in the air (stairs
/// and small drops do not play one).
const LAND_AFTER_AIR: f32 = 0.3;
/// Guess: the aim pitch the spine follows, at most.
const MAX_AIM_PITCH_DEG: f32 = 60.0;
/// KFPawn.AnimEnd / AnimBlendTimer: AnimBlendToAlpha(1, 0.0, 0.12).
const UPPER_BLEND_OUT: f32 = 0.12;
/// PlayDirectionalHit: PlayAnim(HitAnims[i],, 0.1); PlayDirectionalDeath: 0.2.
const HIT_TWEEN: f32 = 0.1;
const DEATH_TWEEN: f32 = 0.2;
/// Guess: takeoff and landing tween like PlayDoubleJump (0.1).
const JUMP_TWEEN: f32 = 0.1;
/// The death ragdoll is off: two attempts left the British_Soldier1
/// ragdoll shaking and sliding for seconds (its joints start 15-23 degrees
/// past their limits, even with the collars at the reference pose; see
/// DESIGN.md). A dead body without a ragdoll or a death animation (the
/// soldier meshes have no DeathF/B/L/R) is hidden.
const PLAYER_RAGDOLL: bool = false;

/// The drawn root of a body.
#[derive(Component)]
pub(super) struct BodyRoot;

/// Which loaded character a pawn is drawn with: the local player's, or
/// the one its `PawnCharacter` names. None: not loaded yet (wait); a
/// character that failed to load is drawn as the local player's.
fn character_index(models: &BodyModels, s: &PawnState, wanted: Option<&super::PawnCharacter>) -> Option<usize> {
    match wanted {
        Some(c) if !s.local => models.by_name.get(&c.0.to_ascii_lowercase()).map(|i| i.unwrap_or(models.local)),
        _ => Some(models.local),
    }
}

/// A pawn whose `PawnCharacter` changed (another player changed
/// character) loses its body; `spawn_bodies` gives it the new one.
pub(super) fn follow_pawn_character(
    mut commands: Commands,
    models: Res<BodyModels>,
    pawns: Query<(Entity, &PawnState, &super::PawnCharacter, &PawnBody), Changed<super::PawnCharacter>>,
) {
    for (e, s, c, body) in &pawns {
        if character_index(&models, s, Some(c)) != Some(body.character) {
            runlog::kv("body_character_change", &format!("wanted={} local={}", c.0, s.local));
            body.despawn(&mut commands, e);
        }
    }
}

/// Pawns without a body: their state, wanted character and name.
type BodilessPawn<'a> = (Entity, &'a PawnState, Option<&'a super::PawnCharacter>, Option<&'a Name>);

/// Gives every pawn with a `PawnState` a body (once the model is loaded).
pub(super) fn spawn_bodies(
    mut commands: Commands,
    models: Res<BodyModels>,
    pawns: Query<BodilessPawn, Without<PawnBody>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    for (e, s, wanted, name) in &pawns {
        let Some(character) = character_index(&models, s, wanted) else { continue };
        let Some(b) = models.characters.get(character) else { continue };
        let handles = b.model.new_instance(&mut meshes);
        let root = commands
            .spawn((
                Transform::from_translation(s.location),
                Visibility::Hidden,
                BodyRoot,
                // Lit by the map (actor_light.rs): xPawn MaxLights 8, KFPawn
                // AmbientGlow 0.
                ActorLight::new(format!("body_{}", if s.local { "local".to_string() } else { format!("{e:?}") }), Vec3::ZERO, 8, 0),
            ))
            .id();
        for (part, h) in b.model.parts.iter().zip(&handles) {
            commands.spawn((Mesh3d(h.clone()), MeshMaterial3d(part.material.clone()), Transform::IDENTITY, ChildOf(root), LitPart { owner: root, animated: true, own: None }));
        }
        commands.entity(e).insert(PawnBody {
            character,
            root,
            meshes: handles,
            weapon_class: None,
            attachment: None,
            attachment_parts: Vec::new(),
            attachment_meshes: Vec::new(),
            base_kind: BaseKind::Idle,
            base: None,
            phase: 0.0,
            move_weights: [0.0; 4],
            tween_from: Vec::new(),
            tween_left: 0.0,
            tween_time: 0.0,
            last_base: Vec::new(),
            upper: None,
            upper_alpha: 0.0,
            upper_blend: None,
            upper_timer: None,
            fire_state: FireState::None,
            seen_flash: s.flash_count,
            seen_reloads: s.reloads,
            seen_hits: s.hits,
            was_on_ground: true,
            air_time: 0.0,
            upper_ending: false,
            last_yaw: s.yaw,
            turn_rate: 0.0,
            turn_hold: 0.0,
            rng: 0x9e37_79b9,
            last_pose: Vec::new(),
            last_locals: Vec::new(),
            ragdoll: None,
            frozen_root: None,
            ragdoll_location: None,
            log_second: -1,
            who: if s.local { "local".into() } else { name.map_or(format!("{e:?}"), |n| n.as_str().replace(' ', "_")) },
            tip: None,
        });
        runlog::kv("body_spawned", &format!("character={} local={} parts={} pawn={e:?}", b.name, s.local, b.model.parts.len()));
    }
}

/// Pawn.Get4WayDirection (native; assumed): the largest of the
/// velocity's forward / sideways parts in the pawn's axes. 0 forward,
/// 1 back, 2 left, 3 right.
pub(super) fn four_way(forward: f32, right: f32) -> usize {
    if forward.abs() >= right.abs() {
        if forward >= 0.0 { 0 } else { 1 }
    } else if right < 0.0 {
        2
    } else {
        3
    }
}

/// Guess for the native movement blend: forward / back / left / right
/// weights from the velocity's direction in the pawn's axes, normalised.
pub(super) fn move_weights(forward: f32, right: f32) -> [f32; 4] {
    let w = [forward.max(0.0), (-forward).max(0.0), (-right).max(0.0), right.max(0.0)];
    let sum: f32 = w.iter().sum();
    if sum <= 1e-6 { [1.0, 0.0, 0.0, 0.0] } else { w.map(|x| x / sum) }
}

/// KFPawn.PlayDirectionalHit: the direction to the hit location in the
/// pawn's axes (forward, right): dot X > 0.7 HitAnims[0], < -0.7
/// HitAnims[1], dot Y > 0 HitAnims[3], else HitAnims[2]. No location: [0].
pub(super) fn hit_index(dir: Option<(f32, f32)>) -> usize {
    let Some((x, y)) = dir else { return 0 };
    if x > 0.7 {
        0
    } else if x < -0.7 {
        1
    } else if y > 0.0 {
        3
    } else {
        2
    }
}

/// xPawn.PlayDirectionalDeath: Dir (velocity, else towards the hit) in
/// the pawn's axes: dot X > 0.7 DeathB, < -0.7 DeathF, dot Y > 0 DeathL,
/// else DeathR.
pub(super) fn death_anim(dir: Option<(f32, f32)>) -> &'static str {
    let Some((x, y)) = dir else { return "DeathB" };
    if x > 0.7 {
        "DeathB"
    } else if x < -0.7 {
        "DeathF"
    } else if y > 0.0 {
        "DeathL"
    } else {
        "DeathR"
    }
}

fn seq(b: &BodyModel, name: &str) -> Option<usize> {
    if name.is_empty() { None } else { b.model.sequence(name) }
}

impl PawnBody {
    fn random(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    /// Starts a new base animation, tweening from the current pose.
    fn set_base(&mut self, b: &BodyModel, kind: BaseKind, play: Option<Play>, tween: f32, why: &str) {
        let same_seq = self.base.map(|p| p.seq) == play.map(|p| p.seq);
        if self.base_kind == kind && (kind == BaseKind::Move || same_seq) {
            return;
        }
        if !self.last_base.is_empty() && tween > 0.0 {
            self.tween_from = self.last_base.clone();
            self.tween_left = tween;
            self.tween_time = tween;
        } else {
            self.tween_left = 0.0;
        }
        runlog::kv(
            "body_anim",
            &format!(
                "who={} channel=0 kind={kind:?} sequence={} tween={tween} reason={why}",
                self.who,
                play.and_then(|p| b.model.sequence_name(p.seq)).unwrap_or("-")
            ),
        );
        self.base_kind = kind;
        self.base = play;
    }

    /// Channel 1 from FireRootBone at alpha 1 (AnimBlendParams(1, 1.0, ...)).
    fn play_upper(&mut self, b: &BodyModel, seq: usize, looping: bool, why: &str) {
        self.upper = Some(Play { seq, frame: 0.0, looping, rate: 1.0 });
        self.upper_alpha = 1.0;
        self.upper_blend = None;
        self.upper_ending = false;
        runlog::kv(
            "body_anim",
            &format!("who={} channel=1 sequence={} looping={looping} fire_state={:?} reason={why}", self.who, b.model.sequence_name(seq).unwrap_or("?"), self.fire_state),
        );
    }

    /// AnimBlendToAlpha(1, 0.0, 0.12).
    fn blend_out_upper(&mut self) {
        self.upper_blend = Some((0.0, self.upper_alpha.max(1e-3) / UPPER_BLEND_OUT));
        self.upper_ending = true;
    }

    /// KFPawn.AnimEnd(1).
    fn upper_anim_end(&mut self, b: &BodyModel, names: &AnimNames) {
        match self.fire_state {
            FireState::Ready => {
                self.blend_out_upper();
                self.fire_state = FireState::None;
            }
            FireState::PlayOnce => {
                self.fire_state = FireState::Ready;
                match seq(b, &names.post_fire_blend_stand) {
                    Some(s) => self.play_upper(b, s, false, "post_fire_blend"),
                    None => self.blend_out_upper(),
                }
            }
            _ => self.blend_out_upper(),
        }
    }

    /// KFPawn.StartFiringX: FireAnims[Rand(4)] (FireAltAnims for mode 1);
    /// looped while firing with bRapidFire, else played once.
    fn start_firing(&mut self, b: &BodyModel, names: &AnimNames, mode: u8, rapid: bool) {
        let set = if mode == 1 { &names.fire_alt } else { &names.fire };
        let i = (self.random() % 4) as usize;
        let Some(s) = seq(b, &set[i]) else { return };
        if rapid {
            if self.fire_state != FireState::Looping {
                self.fire_state = FireState::Looping;
                self.play_upper(b, s, true, "fire_rapid");
            }
        } else {
            self.fire_state = FireState::PlayOnce;
            self.play_upper(b, s, false, "fire");
        }
    }
}

/// Advances a single-sequence play; true when a non-looping one ended
/// (on its last key, N - 1, which it then holds).
fn advance(b: &BodyModel, p: &mut Play, dt: f32) -> bool {
    let len = b.model.length(p.seq).max(1e-3);
    p.frame += dt * b.model.rate(p.seq) * p.rate;
    if p.looping {
        p.frame %= len;
        false
    } else if p.frame >= b.model.last_frame(p.seq) {
        p.frame = b.model.last_frame(p.seq);
        true
    } else {
        false
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)] // Bevy system parameters
pub(super) fn animate_bodies(
    mut commands: Commands,
    time: Res<Time>,
    models: Res<BodyModels>,
    view: Res<ViewTarget>,
    mut pawns: Query<(&PawnState, &mut PawnBody)>,
    mut roots: Query<(&mut Transform, &mut Visibility), (With<BodyRoot>, Without<PawnState>)>,
    rag_bodies: Query<(&Transform, &avian3d::prelude::LinearVelocity), (With<crate::zeds::ragdoll::RagdollBody>, Without<BodyRoot>)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let dt = time.delta_secs();
    for (s, mut body) in &mut pawns {
        let Some(b) = models.characters.get(body.character) else { continue };
        // Dead with nothing to show (no ragdoll, no death animation): hidden.
        let no_corpse = s.dead
            && body.ragdoll.is_none()
            && !(PLAYER_RAGDOLL && b.ragdoll.is_some())
            && ["DeathF", "DeathB", "DeathL", "DeathR"].iter().all(|n| seq(b, n).is_none());
        let visible = s.active && !no_corpse && !(s.local && view.first_person());
        if let Ok((mut t, mut vis)) = roots.get_mut(body.root) {
            match body.frozen_root {
                Some(f) => *t = f,
                None => {
                    t.translation = s.location;
                    t.rotation = Quat::from_rotation_y(s.yaw);
                }
            }
            let want = if visible { Visibility::Inherited } else { Visibility::Hidden };
            if *vis != want {
                *vis = want;
                runlog::kv("body_visible", &format!("who={} visible={visible} active={} local={} first_person={} dead={} no_corpse={no_corpse}", body.who, s.active, s.local, view.first_person(), s.dead));
            }
        }
        if !s.active {
            continue;
        }

        // The attachment in hand (dropped on death: Pawn.Died drops the weapon).
        let class = if s.dead { None } else { s.weapon_class.clone() };
        let att_index = class.as_ref().and_then(|c| models.by_weapon.get(&c.to_ascii_lowercase()).copied().flatten());
        if class != body.weapon_class || att_index != body.attachment {
            let switched = body.weapon_class.is_some() && class.is_some();
            for e in std::mem::take(&mut body.attachment_parts) {
                commands.entity(e).despawn();
            }
            body.attachment_meshes.clear();
            if let Some(m) = att_index.and_then(|i| models.attachments[i].model.as_ref()) {
                let handles = m.new_instance(&mut meshes);
                let root = body.root;
                body.attachment_parts = m
                    .parts
                    .iter()
                    .zip(&handles)
                    // bUseLightingFromBase (InventoryAttachment): the body's light.
                    .map(|(p, h)| commands.spawn((Mesh3d(h.clone()), MeshMaterial3d(p.material.clone()), Transform::IDENTITY, ChildOf(root), LitPart { owner: root, animated: true, own: None })).id())
                    .collect();
                body.attachment_meshes = handles;
            }
            runlog::kv(
                "body_attachment_held",
                &format!("who={} weapon={} attachment={}", body.who, class.as_deref().unwrap_or("none"), att_index.map_or("none".to_string(), |i| models.attachments[i].class.clone())),
            );
            body.weapon_class = class;
            body.attachment = att_index;
            // Pawn.ChangedWeapon -> PlayWeaponSwitch -> SetAnimAction('Weapon_Switch').
            if switched && let Some(ws) = seq(b, "Weapon_Switch") {
                body.play_upper(b, ws, false, "weapon_switch");
                // KFPawn.SetAnimAction: AnimBlendTime = GetAnimDuration + 0.1.
                body.upper_timer = Some(b.model.length(ws) / b.model.rate(ws).max(1e-3) + 0.1);
            }
        }
        let att = body.attachment.map(|i| &models.attachments[i]);
        let names = att.and_then(|a| a.names.as_ref()).unwrap_or(&b.names);

        // The velocity in the pawn's axes (Unreal units/s).
        let local_v = Quat::from_rotation_y(-s.yaw) * s.velocity;
        let (fwd, right) = (-local_v.z / SCALE, local_v.x / SCALE);
        let speed = fwd.hypot(right);
        let dir4 = four_way(fwd, right);
        let flat_dir = (speed > 1.0).then(|| (fwd / speed, right / speed));
        // Turning speed (Unreal units/s; positive = to the left).
        let mut dyaw = s.yaw - body.last_yaw;
        dyaw = (dyaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        body.last_yaw = s.yaw;
        if dt > 0.0 {
            body.turn_rate = dyaw / dt * 65536.0 / std::f32::consts::TAU;
        }
        // The direction to the last hit, in the pawn's axes.
        let hit_dir = s.hit_from.and_then(|from| {
            let d = Quat::from_rotation_y(-s.yaw) * (from - s.location);
            let (x, y) = (-d.z, d.x);
            let l = x.hypot(y);
            (l > 1e-4).then(|| (x / l, y / l))
        });

        if s.dead {
            if body.base_kind != BaseKind::Death {
                // PlayDying: AnimBlendParams(1, 0.0), FireState = FS_None.
                body.upper = None;
                body.upper_alpha = 0.0;
                body.fire_state = FireState::None;
                match (b.ragdoll.as_ref().filter(|_| PLAYER_RAGDOLL), body.last_pose.is_empty()) {
                    (Some(def), false) => {
                        // KFPawn.PlayDyingAnimation: KStartLinVel = 0.6 x the
                        // horizontal velocity + the vertical one + RagDeathVel
                        // along the killing shot (TearOffMomentum: from the
                        // attacker to us, assumed), + RagDeathUpKick unless
                        // falling. No start spin (simplified).
                        let v = s.velocity;
                        let mut velocity = Vec3::new(0.6 * v.x, v.y, 0.6 * v.z);
                        if let Some(from) = s.hit_from {
                            velocity += (s.location - from).normalize_or_zero() * b.rag_death_vel * SCALE;
                        }
                        if v.y > -10.0 * SCALE {
                            velocity.y += b.rag_up_kick * SCALE;
                        }
                        let actor = Transform::from_translation(s.location).with_rotation(Quat::from_rotation_y(s.yaw));
                        let r = b.model.mesh.rot_origin;
                        let frame = crate::zeds::ragdoll::MeshFrame::new(
                            coords::ue_rotation_matrix(Rotator { pitch: r[0], yaw: r[1], roll: r[2] }),
                            b.model.mesh.scale[0],
                            Vec3::from_array(b.model.mesh.origin),
                            b.draw_scale,
                            b.pre_pivot,
                            &actor,
                        );
                        let launch = crate::zeds::ragdoll::Launch { velocity, angular_velocity: Vec3::ZERO, pivot: None };
                        // The pose at death, without the aim pitch (dying:
                        // SetTwistLook(0, 0)) and with the arm collars at
                        // their reference pose: our fix, not KF's. The held-
                        // weapon poses swing the collars 19-23 degrees past
                        // the ragdoll's 6 degree limit, and those shapeless,
                        // nearly massless parts then shook the body for
                        // seconds (body_ragdoll_limits_at_death, 2026-10-06).
                        let pose = if body.last_locals.len() == b.model.mesh.bones.len() {
                            let mut locals = body.last_locals.clone();
                            let bind = b.model.sample_locals(None, 0.0);
                            for (i, bone) in b.model.mesh.bones.iter().enumerate() {
                                if bone.name.to_ascii_lowercase().contains("collar") {
                                    locals[i] = bind[i];
                                }
                            }
                            b.model.pose_from_locals(&locals, &[])
                        } else {
                            body.last_pose.clone()
                        };
                        body.ragdoll = Some(crate::zeds::ragdoll::spawn(&mut commands, def, &pose, frame, &launch, usize::MAX));
                        body.frozen_root = Some(actor);
                        body.base_kind = BaseKind::Death;
                        runlog::kv(
                            "body_ragdoll_started",
                            &format!("ragdoll={} velocity_unreal=({:.0}, {:.0}, {:.0}) hit_from={}", def.name, -velocity.z / SCALE, velocity.x / SCALE, velocity.y / SCALE, s.hit_from.is_some()),
                        );
                    }
                    _ => {
                        // xPawn.PlayDyingAnimation's fallback without a ragdoll.
                        let name = death_anim(flat_dir.or(hit_dir));
                        let play = seq(b, name).map(|seq| Play { seq, frame: 0.0, looping: false, rate: 1.0 });
                        body.set_base(b, BaseKind::Death, play, DEATH_TWEEN, name);
                    }
                }
            }
        } else {
            if body.base_kind == BaseKind::Death {
                body.base_kind = BaseKind::Idle;
                body.base = None;
                if let Some(r) = body.ragdoll.take() {
                    for e in r.joints.iter().chain(&r.bodies) {
                        commands.entity(*e).despawn();
                    }
                }
                body.frozen_root = None;
                runlog::kv("body_anim", &format!("who={} channel=0 kind=Idle reason=revived", body.who));
            }
            // Shots (FlashCount changes), firing stopped, reloads, hits.
            if s.flash_count != body.seen_flash {
                body.seen_flash = s.flash_count;
                let rapid = att.is_some_and(|a| a.rapid[usize::from(s.firing_mode.min(1))]);
                body.start_firing(b, names, s.firing_mode, rapid);
            }
            if !s.firing && body.fire_state == FireState::Looping {
                // StopFiring: FS_Looping -> FS_PlayOnce (ends at the loop's end).
                body.fire_state = FireState::PlayOnce;
            }
            if s.reloads != body.seen_reloads {
                body.seen_reloads = s.reloads;
                if let Some(r) = att.and_then(|a| seq(b, &a.reload_anim)) {
                    body.fire_state = FireState::Ready;
                    body.upper_timer = None;
                    body.play_upper(b, r, false, "reload");
                }
            }
            let new_hit = s.hits != body.seen_hits;
            body.seen_hits = s.hits;

            // The base channel.
            if s.on_ground {
                if !body.was_on_ground && body.air_time > LAND_AFTER_AIR {
                    let name = if speed < STILL_AIR_SPEED { &names.land[0] } else { &names.land[dir4] };
                    let play = seq(b, name).map(|seq| Play { seq, frame: 0.0, looping: false, rate: 1.0 });
                    body.set_base(b, BaseKind::Land, play, JUMP_TWEEN, "landed");
                }
                body.air_time = 0.0;
            } else {
                body.air_time += dt;
                if body.was_on_ground && s.velocity.y > 0.0 {
                    let name = if speed < STILL_AIR_SPEED { &names.takeoff_still } else { &names.takeoff[dir4] };
                    let play = seq(b, name).map(|seq| Play { seq, frame: 0.0, looping: false, rate: 1.0 });
                    body.set_base(b, BaseKind::Takeoff, play, JUMP_TWEEN, "jump");
                }
            }
            body.was_on_ground = s.on_ground;
            let finished = body.base.is_some_and(|p| !p.looping && p.frame >= b.model.last_frame(p.seq));
            let busy = matches!(body.base_kind, BaseKind::Takeoff | BaseKind::Land | BaseKind::Hit) && body.base.is_some() && !finished;
            if new_hit && s.on_ground && speed < IDLE_SPEED {
                // Over the whole body when standing (see DESIGN.md: while
                // moving the native movement channels cover it).
                let name = &names.hit[hit_index(hit_dir)];
                let play = seq(b, name).map(|seq| Play { seq, frame: 0.0, looping: false, rate: 1.0 });
                if play.is_some() {
                    body.base_kind = BaseKind::Idle; // restart even if a hit is playing
                    body.set_base(b, BaseKind::Hit, play, HIT_TWEEN, "hit");
                }
            } else if !s.on_ground {
                if !(body.base_kind == BaseKind::Takeoff && busy) {
                    let name = if speed < STILL_AIR_SPEED { &names.air_still } else { &names.air[dir4] };
                    let play = seq(b, name).map(|seq| Play { seq, frame: 0.0, looping: true, rate: 1.0 });
                    body.set_base(b, BaseKind::Air, play, BLEND_CHANGE_TIME, "falling");
                }
            } else if busy && (body.base_kind == BaseKind::Hit || speed < IDLE_SPEED) {
                // Let the landing / hit play out.
            } else if speed >= IDLE_SPEED {
                body.move_weights = move_weights(fwd, right);
                body.set_base(b, BaseKind::Move, None, BLEND_CHANGE_TIME, "moving");
            } else {
                if body.turn_rate.abs() > TURN_START_RATE {
                    body.turn_hold = TURN_HOLD;
                }
                body.turn_hold -= dt;
                // The direction of the turn; kept while the turn lingers.
                let left = match body.base_kind {
                    BaseKind::Turn(l) if body.turn_rate.abs() <= TURN_START_RATE => l,
                    _ => body.turn_rate > 0.0,
                };
                let turn_name = if left { &names.turn_left } else { &names.turn_right };
                match (body.turn_hold > 0.0, seq(b, turn_name)) {
                    (true, Some(t)) => {
                        // "Scaled by turn speed" (Pawn.uc); the scale is a guess.
                        let rate = (body.turn_rate.abs() / 16384.0).clamp(0.5, 2.0);
                        let kind = BaseKind::Turn(left);
                        body.set_base(b, kind, Some(Play { seq: t, frame: 0.0, looping: true, rate }), BLEND_CHANGE_TIME, "turning");
                        if let Some(p) = body.base.as_mut() {
                            p.rate = rate;
                        }
                    }
                    _ => {
                        let play = seq(b, &names.idle_weapon).map(|seq| Play { seq, frame: 0.0, looping: true, rate: 1.0 });
                        body.set_base(b, BaseKind::Idle, play, BLEND_CHANGE_TIME, "standing");
                    }
                }
            }
        }

        // Advance the base channel.
        let mut base_locals = if body.base_kind == BaseKind::Move {
            let rate = (speed / RUN_REFERENCE_SPEED).clamp(RUN_RATE_RANGE.0, RUN_RATE_RANGE.1);
            let seqs = names.movement.clone().map(|n| seq(b, &n));
            let weights = body.move_weights;
            let dominant = (0..4).filter(|&i| seqs[i].is_some()).max_by(|&a, &c| weights[a].total_cmp(&weights[c]));
            if let Some(d) = dominant.and_then(|d| seqs[d]) {
                let cycle = b.model.length(d) / b.model.rate(d).max(1e-3);
                body.phase = (body.phase + dt * rate / cycle.max(1e-3)).fract();
            }
            let mut acc = 0.0;
            let mut locals: Option<Vec<(Quat, Vec3)>> = None;
            for i in 0..4 {
                let (Some(sq), w) = (seqs[i], weights[i]) else { continue };
                if w <= 0.0 {
                    continue;
                }
                acc += w;
                let l = b.model.sample_locals(Some(sq), body.phase * b.model.length(sq));
                match locals.as_mut() {
                    None => locals = Some(l),
                    Some(into) => b.model.blend_locals(into, &l, w / acc, None),
                }
            }
            locals.unwrap_or_else(|| b.model.sample_locals(None, 0.0))
        } else {
            let mut p = body.base;
            if let Some(play) = p.as_mut() {
                advance(b, play, dt);
                // Death holds its last frame (no ragdoll yet).
            }
            body.base = p;
            b.model.sample_locals(p.map(|p| p.seq), p.map_or(0.0, |p| p.frame))
        };
        if body.tween_left > 0.0 && body.tween_from.len() == base_locals.len() {
            let alpha = 1.0 - body.tween_left / body.tween_time.max(1e-3);
            let mut from = body.tween_from.clone();
            b.model.blend_locals(&mut from, &base_locals, alpha, None);
            base_locals = from;
            body.tween_left -= dt;
        }
        body.last_base = base_locals.clone();

        // Channel 1.
        if let Some(mut u) = body.upper {
            let before = u.frame;
            let ended = advance(b, &mut u, dt);
            // A loop's end counts as AnimEnd once firing has stopped
            // (guess: not while FS_Looping, or sustained fire would fade).
            let wrapped = u.looping && u.frame < before && body.fire_state != FireState::Looping;
            if wrapped {
                u.frame = b.model.last_frame(u.seq);
                u.looping = false;
            }
            body.upper = Some(u);
            if (ended || wrapped) && !body.upper_ending {
                body.upper_anim_end(b, names);
            }
        }
        if let Some((target, per_sec)) = body.upper_blend {
            let step = per_sec * dt;
            body.upper_alpha = if body.upper_alpha > target { (body.upper_alpha - step).max(target) } else { (body.upper_alpha + step).min(target) };
            if (body.upper_alpha - target).abs() < 1e-4 {
                body.upper_blend = None;
                if target <= 0.0 {
                    body.upper = None;
                    body.upper_ending = false;
                }
            }
        }
        if let Some(t) = body.upper_timer {
            let t = t - dt;
            body.upper_timer = (t > 0.0).then_some(t);
            if t <= 0.0 {
                // KFPawn.AnimBlendTimer.
                body.blend_out_upper();
            }
        }
        let mut locals = base_locals;
        if let (Some(u), Some(root)) = (body.upper, b.fire_root)
            && body.upper_alpha > 0.0
        {
            let up = b.model.sample_locals(Some(u.seq), u.frame);
            b.model.blend_locals(&mut locals, &up, body.upper_alpha, Some(root));
        }
        if body.ragdoll.is_none() {
            body.last_locals = locals.clone();
        }

        // Aim pitch: shared by SpineBone1 and SpineBone2 (guess), turning
        // about the pawn's right (Unreal Y) axis, given in mesh space.
        let pitch = if s.dead { 0.0 } else { s.pitch.clamp(-MAX_AIM_PITCH_DEG.to_radians(), MAX_AIM_PITCH_DEG.to_radians()) };
        let spines: Vec<usize> = b.spine.iter().flatten().copied().collect();
        let mut turns = Vec::new();
        if !spines.is_empty() && pitch != 0.0 {
            let share = pitch / spines.len() as f32;
            let r = b.model.mesh.rot_origin;
            let mesh_axes = coords::ue_rotation_matrix(Rotator { pitch: r[0], yaw: r[1], roll: r[2] })
                * Mat3::from_diagonal(Vec3::from_array(b.model.mesh.scale).signum());
            let turn_actor = coords::ue_rotation_matrix(Rotator { pitch: (share * 65536.0 / std::f32::consts::TAU) as i32, yaw: 0, roll: 0 });
            let turn_mesh = Quat::from_mat3(&(mesh_axes.inverse() * turn_actor * mesh_axes)).normalize();
            turns = spines.iter().map(|&i| (i, turn_mesh)).collect();
        }
        let mut pose = b.model.pose_from_locals(&locals, &turns);
        let ragdoll = body.ragdoll.take();
        if let (Some(mut r), Some(def)) = (ragdoll, b.ragdoll.as_ref()) {
            // Bones from the ragdoll's bodies (the first frame they are not
            // spawned yet: keep the pose at death).
            pose = r.pose(def, |e| rag_bodies.get(e).ok().map(|(t, _)| *t)).unwrap_or_else(|| body.last_pose.clone());
            // The pawn's Location follows the ragdoll's root part (Karma
            // moves the actor with it): behind view looks at it.
            body.ragdoll_location = r.bodies.get(def.root).and_then(|&e| rag_bodies.get(e).ok()).map(|(t, _)| t.translation);
            // Diagnostics, every half second for the first 6 s.
            let age_before = r.age;
            r.age += dt;
            let age = r.age;
            let first = r.ball_joints.iter().all(|w| w.max_swing == 0.0 && w.max_twist == 0.0);
            r.watch_limits(|e| rag_bodies.get(e).ok().map(|(t, _)| *t));
            if first && r.ball_joints.iter().any(|w| w.max_swing > 0.0 || w.max_twist > 0.0) {
                runlog::kv("body_ragdoll_limits_at_death", &format!("deg {}", r.limit_report(def)));
            }
            if age <= 6.0 && (age * 2.0).floor() != (age_before * 2.0).floor() {
                let (fastest, max_speed) = r
                    .bodies
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &e)| rag_bodies.get(e).ok().map(|(_, v)| (i, v.0.length() / SCALE)))
                    .fold((0, 0.0f32), |a, x| if x.1 > a.1 { x } else { a });
                let gap = r.worst_joint_gap(def, |e| rag_bodies.get(e).ok().map(|(t, _)| *t));
                runlog::kv(
                    "body_ragdoll",
                    &format!(
                        "age={age:.1} root_unreal={:?} max_body_speed_unreal={max_speed:.0} fastest={} worst_joint_gap={:?}",
                        body.ragdoll_location.map(|p| {
                            let u = p / SCALE;
                            [(-u.z).round(), u.x.round(), u.y.round()]
                        }),
                        def.part_name(fastest),
                        gap.map(|(n, g)| format!("{n}:{g:.1}")),
                    ),
                );
            }
            body.ragdoll = Some(r);
        } else {
            body.ragdoll_location = None;
        }
        body.last_pose = pose.clone();
        let to_actor = b.model.mesh_to_actor(b.pre_pivot, b.draw_scale);
        // The weapon bone's frame in the pawn's actor space (AttachToBone:
        // the bone's origin and axes; attachments have no relative offset).
        let hand = b.weapon_bone.map(|wb| {
            let (q, p) = pose[wb];
            let origin = to_actor(p);
            let axes = [Vec3::X, Vec3::Y, Vec3::Z].map(|ax| (to_actor(p + q * ax) - origin).normalize_or_zero());
            (origin, Mat3::from_cols(axes[0], axes[1], axes[2]))
        });
        let att_model = att.and_then(|a| a.model.as_ref().map(|m| (a, m, m.mesh_to_actor(Vec3::ZERO, a.draw_scale))));
        // The attachment's `tip` bone in Unreal world space (the root is at
        // the pawn's Location, turned by its yaw), for the muzzle flash.
        body.tip = match (&att_model, hand) {
            (Some((a, _, att_to_actor)), Some((origin, frame))) => a.tip.map(|(q, p)| {
                let o = origin + frame * att_to_actor(p);
                let axes = [Vec3::X, Vec3::Y, Vec3::Z].map(|ax| (origin + frame * att_to_actor(p + q * ax) - o).normalize_or_zero());
                let rot = Quat::from_rotation_y(s.yaw);
                let to_ue = |b: Vec3| Vec3::new(-b.z, b.x, b.y);
                let at = to_ue(s.location + rot * coords::pos(o.to_array())) / SCALE;
                let axes = axes.map(|ax| to_ue(rot * coords::dir(ax.to_array())));
                (at, Mat3::from_cols(axes[0], axes[1], axes[2]))
            }),
            _ => None,
        };
        if visible {
            let skinned = b.model.skin(&pose, &[]);
            b.model.upload_to(&body.meshes, &skinned, |p| coords::pos(to_actor(p).to_array()), &mut meshes);
            if let (Some((a, m, att_to_actor)), Some((origin, frame))) = (&att_model, hand) {
                m.upload_to(&body.attachment_meshes, &a.points, |pt| coords::pos((origin + frame * att_to_actor(pt)).to_array()), &mut meshes);
            }
        }

        let second = time.elapsed_secs() as i64;
        if second != body.log_second {
            body.log_second = second;
            let u = s.location / SCALE;
            // The attachment's bounds in the pawn's actor space.
            let att_bounds = match (&att_model, hand) {
                (Some((a, _, att_to_actor)), Some((origin, frame))) if !a.points.is_empty() => {
                    let pts: Vec<Vec3> = a.points.iter().map(|&pt| origin + frame * att_to_actor(pt)).collect();
                    let lo = pts.iter().fold(Vec3::splat(f32::MAX), |m, p| m.min(*p));
                    let hi = pts.iter().fold(Vec3::splat(f32::MIN), |m, p| m.max(*p));
                    Some((lo.round().to_array(), hi.round().to_array()))
                }
                _ => None,
            };
            let hand = hand.map(|(o, _)| [o.x.round(), o.y.round(), o.z.round()]);
            runlog::kv(
                "body_state",
                &format!(
                    "who={} location_unreal=({:.0}, {:.0}, {:.0}) speed={speed:.0} forward={fwd:.0} right={right:.0} on_ground={} base={:?} base_seq={} weights={:?} phase={:.2} upper={} upper_alpha={:.2} fire_state={:?} pitch_deg={:.1} turn_rate={:.0} visible={visible} weapon={} hand_actor={:?} attachment_bounds_actor={:?} ragdoll_root_unreal={:?}",
                    body.who,
                    -u.z,
                    u.x,
                    u.y,
                    s.on_ground,
                    body.base_kind,
                    body.base.and_then(|p| b.model.sequence_name(p.seq)).unwrap_or(if body.base_kind == BaseKind::Move { "blend" } else { "-" }),
                    body.move_weights.map(|w| (w * 100.0).round() / 100.0),
                    body.phase,
                    body.upper.and_then(|p| b.model.sequence_name(p.seq)).unwrap_or("-"),
                    body.upper_alpha,
                    body.fire_state,
                    pitch.to_degrees(),
                    body.turn_rate,
                    body.weapon_class.as_deref().unwrap_or("none"),
                    hand,
                    att_bounds,
                    body.ragdoll_location.map(|p| {
                        let u = p / SCALE;
                        [(-u.z).round(), u.x.round(), u.y.round()]
                    }),
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_way_picks_the_largest_part() {
        assert_eq!(four_way(100.0, 20.0), 0);
        assert_eq!(four_way(-100.0, 20.0), 1);
        assert_eq!(four_way(10.0, -50.0), 2);
        assert_eq!(four_way(10.0, 50.0), 3);
    }

    #[test]
    fn move_weights_follow_the_direction() {
        assert_eq!(move_weights(200.0, 0.0), [1.0, 0.0, 0.0, 0.0]);
        let w = move_weights(100.0, 100.0);
        assert!((w[0] - 0.5).abs() < 1e-6 && (w[3] - 0.5).abs() < 1e-6 && w[1] == 0.0 && w[2] == 0.0);
        assert_eq!(move_weights(0.0, -10.0), [0.0, 0.0, 1.0, 0.0]);
    }

    #[test]
    fn hit_and_death_directions_match_the_scripts() {
        // Hit from straight ahead: HitAnims[0]; behind: [1]; right: [3]; left: [2].
        assert_eq!(hit_index(Some((1.0, 0.0))), 0);
        assert_eq!(hit_index(Some((-1.0, 0.0))), 1);
        assert_eq!(hit_index(Some((0.0, 1.0))), 3);
        assert_eq!(hit_index(Some((0.0, -1.0))), 2);
        assert_eq!(hit_index(None), 0);
        // Moving forward when killed: DeathB (falls back).
        assert_eq!(death_anim(Some((1.0, 0.0))), "DeathB");
        assert_eq!(death_anim(Some((-1.0, 0.0))), "DeathF");
        assert_eq!(death_anim(Some((0.0, 1.0))), "DeathL");
        assert_eq!(death_anim(Some((0.0, -1.0))), "DeathR");
    }
}
