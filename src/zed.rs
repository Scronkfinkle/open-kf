//! Zeds (KF monsters): Clots that chase the player (straight line, no
//! pathfinding yet) and play melee animations when close. No damage yet.
//!
//! Mesh-to-world (UE2): point - MeshOrigin, scaled by MeshScale, rotated by
//! the mesh's RotOrigin, scaled by DrawScale, plus PrePivot; then the actor's
//! rotation and location (the centre of its collision cylinder).
//!
//! Keys: Z spawns a zed in front of you (N picks the type: Clot, Gorefast),
//! G a Gorefast, X pauses/resumes all zeds.

use avian3d::prelude::*;
use bevy::prelude::*;

use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package::ObjectRef;
use ue_assets::package_set::{ObjectHandle, PackageSet};
use ue_assets::properties::{Rotator, Value};

use crate::camera::FlyCamera;
use crate::gore::{self, GoreAssets, PieceModel, StumpKind};
use crate::decals::{DecalKind, SpawnDecal};
use crate::particles::{self, EffectLibrary, ParticleEffect};
use crate::coords::{self, SCALE};
use crate::map::MapRequest;
use crate::pawn_collision::{Cylinder, clip_move};
use crate::ragdoll::{self, Launch, MeshFrame, RagdollBody, RagdollDef, RagdollState};
use crate::runlog;
use crate::skinned::{SkinnedModel, Skins};
use crate::walk::{Mover, Walker};

/// Spawn a Clot (`--zed`) or a Gorefast (`--gorefast`) at startup.
#[derive(Resource, Default, Clone, Copy)]
pub struct ZedSettings {
    pub spawn_at_start: bool,
    pub gorefast_at_start: bool,
    /// Test switch (`--always-sever`): a killing hit on a limb always takes
    /// it off, instead of KF's chance.
    pub always_sever: bool,
}

/// What Z spawns (an index into the loaded classes) and its name for the
/// HUD; N cycles through the loaded types.
#[derive(Resource)]
pub struct ZSpawn {
    class: usize,
    pub label: String,
}

impl Default for ZSpawn {
    fn default() -> Self {
        ZSpawn {
            class: 0,
            label: "Clot".into(),
        }
    }
}

/// Zed types with rules of their own in their scripts.
#[derive(Clone, Copy, PartialEq, Debug)]
enum ZedKind {
    Clot,
    Gorefast,
}

/// The classes we load: kind and class path.
const ZED_CLASSES: [(ZedKind, &str); 2] = [
    (ZedKind::Clot, "KFChar.ZombieClot_STANDARD"),
    (ZedKind::Gorefast, "KFChar.ZombieGorefast_STANDARD"),
];

/// Static data for one zed class.
struct ZedClass {
    kind: ZedKind,
    name: String,
    model: SkinnedModel,
    draw_scale: f32,
    pre_pivot: Vec3,
    collision_radius: f32,
    collision_height: f32,
    ground_speed: f32,
    /// Turn speed, Unreal rotation units per second (RotationRate.Yaw).
    turn_rate: f32,
    melee_range: f32,
    idle: Option<usize>,
    walk: Option<usize>,
    melee: Vec<usize>,
    /// Death animation (no ragdolls yet) and the frame where the body is
    /// lowest, where it stops (KnockDown ends standing back up).
    death: Option<usize>,
    death_hold_frame: f32,
    health: f32,
    head_health: f32,
    /// HeadRadius x HeadScale.
    head_radius: f32,
    /// HeadHeight x HeadScale: head centre offset along the head bone's X axis.
    head_offset: f32,
    head_bone: Option<usize>,
    melee_damage: f32,
    /// KFMonster extended collision: (offset from centre, radius, half-height),
    /// Unreal units. A second cylinder at head height.
    ext_collision: Option<(Vec3, f32, f32)>,
    /// Ragdoll used on death (KFRagdollName), if found.
    ragdoll: Option<RagdollDef>,
    /// HealthMax, for the head-explosion damage.
    health_max: f32,
    /// Seconds a headless zed lives on (BleedOutDuration).
    bleed_out_duration: f32,
    /// HeadlessWalkAnims[0] (forward), if the mesh has it.
    headless_walk: Option<usize>,
    /// Hit reactions (KFMonster.PlayDirectionalHit): KFHitFront, KFHitBack,
    /// KFHitLeft, KFHitRight, played on the upper body from SpineBone1 up;
    /// HitAnims (stuns, picked at random); KnockDown (FlipOver, full body).
    hit_reactions: [Option<usize>; 4],
    hit_anims: Vec<usize>,
    knock_down: Option<usize>,
    spine_bone: Option<usize>,
    /// Attacks played on the upper body from FireRootBone while the zed keeps
    /// walking (ZombieClot.DoAnimAction: the three ClotGrapple animations);
    /// other attacks play on the whole body.
    layered_attacks: Vec<usize>,
    fire_root_bone: Option<usize>,
    /// ZombieClot.RemoveHead: headless, the Clot attacks with Claw, Claw,
    /// Claw2 (full body) at double MeleeDamage and MeleeRange. Empty for
    /// other zeds.
    headless_melee: Vec<usize>,
    /// Falling and landing (Skaarj AirAnims / LandAnims: InAir, Landed) and
    /// turning in place (KFMonster TurnLeftAnim / TurnRightAnim).
    air_anim: Option<usize>,
    land_anim: Option<usize>,
    turn_left: Option<usize>,
    turn_right: Option<usize>,
    /// A landed grab pins the player this long (ZombieClot GrappleDuration;
    /// 0 for zeds that do not grab).
    grapple_duration: f32,
    /// Gorefast: run animation (ZombieRun) and the length of a moving attack
    /// in seconds (RunAttackTimeout = GetAnimDuration('GoreAttack1')).
    run_anim: Option<usize>,
    run_attack_seconds: f32,
    /// DetachedArmClass, DetachedLegClass, DetachedHeadClass models.
    severed_pieces: [Option<PieceModel>; 3],
    /// SeveredHeadAttachScale, SeveredArmAttachScale, SeveredLegAttachScale.
    attach_scale: [f32; 3],
    /// bLeftArmGibbed (the Gorefast has no left forearm).
    left_arm_gibbed: bool,
}

#[derive(Resource)]
struct ZedClasses(Vec<ZedClass>);

/// X toggles zed thinking/moving (animation keeps playing).
#[derive(Resource)]
struct ZedsActive(bool);

#[derive(Clone, Copy, PartialEq, Debug)]
enum ZedState {
    Idle,
    Chase,
    Melee,
    Falling,
    /// FlipOver: playing KnockDown, not moving (WaitForAnim).
    KnockedDown,
    /// Just landed from a fall: playing Landed once.
    Landing,
    Dead,
}

/// An attack in progress.
#[derive(Clone, Copy, Debug)]
struct Attack {
    seq: usize,
    /// On the upper-body layer (the zed keeps walking) or the whole body.
    layered: bool,
    hit_done: bool,
}

/// A zed's reaction to being hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HitReaction {
    Front,
    Back,
    Left,
    Right,
    /// HitAnims (random) and bSTUNNED for StunTime.
    Stun,
    /// FlipOver: full-body KnockDown; the zed waits for it to end.
    KnockDown,
}

/// Below this ground speed (Unreal units/s) a chasing zed counts as standing,
/// and above this yaw rate (rotation units/s) as turning in place.
const STANDING_SPEED: f32 = 10.0;
const TURNING_RATE: f32 = 2000.0;

/// ZombieGoreFast: runs when its target is within this many units, at
/// GroundSpeed x 1.875; a run attack is a moving one with ChargeChance
/// (0.2 at Normal difficulty, GameDifficulty 2).
const GOREFAST_RUN_DISTANCE: f32 = 700.0;
const GOREFAST_RUN_SPEED: f32 = 1.875;
const GOREFAST_CHARGE_CHANCE: f32 = 0.2;

/// KFMonster MinTimeBetweenPainAnims and StunTime (seconds).
const MIN_TIME_BETWEEN_PAIN_ANIMS: f32 = 0.5;
const STUN_TIME: f32 = 1.0;

#[derive(Component)]
pub struct Zed {
    pub id: usize,
    class: usize,
    /// Collision cylinder centre, Bevy space (metres).
    pub centre: Vec3,
    /// Collision cylinder, Unreal units.
    pub radius: f32,
    pub half_height: f32,
    pub health: f32,
    pub health_max: f32,
    pub head_health: f32,
    /// Head removed (KFMonster bDecapitated). The zed may live on, bleeding out.
    pub decapitated: bool,
    /// Seconds until a headless zed dies (BleedOutTime), and the class's
    /// BleedOutDuration to start it from.
    pub bleed_out: Option<f32>,
    pub bleed_out_duration: f32,
    /// Upper-body animation layer (sequence, frame, root bone): a hit
    /// reaction or a grab. One layer, as KF's channel 1: a new one replaces
    /// the old (a flinch interrupts a grab).
    overlay: Option<(usize, f32, usize)>,
    /// The attack in progress, if any.
    attack: Option<Attack>,
    /// Seconds since the head came off (DECAP lasts 2 s: no melee hits).
    since_decap: Option<f32>,
    /// A hit reaction waiting to be started by the animation system.
    pending_reaction: Option<HitReaction>,
    /// Seconds since the last pain animation (KFMonster LastPainAnim).
    since_pain_anim: f32,
    /// Seconds of stun left (bSTUNNED): the zed cannot start an attack.
    stunned: f32,
    /// Class values the hit reaction rules need.
    default_health: f32,
    /// MeleeRange and MeleeDamage (doubled for a headless Clot).
    melee_range: f32,
    melee_damage: f32,
    /// The class switches to claws at double damage and range when headless.
    headless_claws: bool,
    /// Gorefast only: may run (ZombieGoreFast RunningState).
    can_run: bool,
    /// In RunningState; seconds to the next CheckCharge; seconds of a moving
    /// attack left (RunAttackTimeout).
    running: bool,
    charge_check: f32,
    run_attack_timeout: f32,
    /// Random number state (stun animation choice, random hit direction).
    rng: u32,
    /// Head sphere radius, Unreal units.
    pub head_radius: f32,
    /// Head sphere centre and the head bone's X axis, Bevy space (updated each frame).
    pub head: Option<(Vec3, Vec3)>,
    /// Extended collision cylinder: centre (Bevy space, updated each frame),
    /// radius and half-height (Unreal units).
    pub ext: Option<(Vec3, f32, f32)>,
    /// Seconds since death.
    dead_for: f32,
    /// Last hit: point and shot direction (Bevy space), for the ragdoll push.
    pub last_hit: Option<(Vec3, Vec3)>,
    /// Current velocity (Bevy space, m/s), for the ragdoll start velocity.
    velocity: Vec3,
    /// Damage events whose gore effects are still to be spawned.
    pub gore_hits: Vec<GoreHit>,
    /// Stumps: kind, the tag they sit on, scale, meshes.
    stumps: Vec<(StumpKind, &'static str, f32, Vec<Handle<Mesh>>)>,
    /// Bones scaled to 0 for severed limbs (HideBone), and which limbs.
    hidden_bones: Vec<usize>,
    severed: Vec<&'static str>,
    /// Last frame's bone transforms (mesh space), for gore placement.
    last_pose: Vec<(Quat, Vec3)>,
    /// Pieces and chunks spawned so far (their ids are id x 100 + this).
    next_piece: usize,
    /// Particle effects attached to a mesh tag (AttachEmitterEffect).
    effects: Vec<(Entity, &'static str)>,
    /// Seconds since the last hit (TakeDamage's bRecentHit: under 0.2 s).
    since_hit: f32,

    /// Facing, Unreal rotation units.
    yaw: f32,
    state: ZedState,
    vertical_speed: f32,
    sequence: Option<usize>,
    frame: f32,
    looping: bool,
    meshes: Vec<Handle<Mesh>>,
}

/// One damage event, for gore effects (KFMonster.DoDamageFX). Bevy space.
#[derive(Clone, Copy, Debug)]
pub struct GoreHit {
    /// Damage of the hit (after the headshot multiplier).
    pub damage: f32,
    pub health_after: f32,
    pub melee: bool,
    /// This hit took the head off.
    pub decapitated: bool,
    pub point: Vec3,
    pub dir: Vec3,
    pub attacker: Vec3,
}

/// Limbs that can come off (KFMonster LeftThighBone, RightThighBone,
/// LeftFArmBone, RightFArmBone: mesh tags), with the tag their stump sits
/// on, the stump, the piece (0 arm, 1 leg) and how many brain chunks
/// ProcessHitFX throws with it.
const LIMBS: [(&str, &str, StumpKind, usize, usize); 4] = [
    ("lthigh", "lleg", StumpKind::Leg, 1, 3),
    ("rthigh", "rleg", StumpKind::Leg, 1, 3),
    ("lfarm", "larm", StumpKind::Arm, 0, 2),
    ("rfarm", "rarm", StumpKind::Arm, 0, 2),
];

/// DoDamageFX on a killing hit: the hit bone's name as KF maps it
/// (KFMonster default bone names; neither the Clot nor the Gorefast
/// overrides them).
fn bone_on_death(name: &str) -> String {
    let n = name.to_ascii_lowercase();
    match n.as_str() {
        "neck" => "head".into(),
        "lfoot" | "lleg" => "lthigh".into(),
        "rfoot" | "rleg" => "rthigh".into(),
        "righthand" | "rshoulder" | "rarm" => "rfarm".into(),
        "lefthand" | "lshoulder" | "larm" => "lfarm".into(),
        _ => n,
    }
}

/// Closest distance between the line through `p` along unit `d` and the
/// segment `a`-`b`.
fn line_segment_distance(p: Vec3, d: Vec3, a: Vec3, b: Vec3) -> f32 {
    let e = b - a;
    let w = a - p;
    let (ee, ed) = (e.dot(e), e.dot(d));
    let denom = ee - ed * ed;
    let u = if ee < 1e-12 || denom < 1e-12 { 0.0 } else { ((ed * d.dot(w) - e.dot(w)) / denom).clamp(0.0, 1.0) };
    let x = a + e * u - p;
    (x - d * x.dot(d)).length()
}

/// Bevy position -> Unreal world position (units), and directions.
fn ue_pos(v: Vec3) -> Vec3 {
    Vec3::new(-v.z, v.x, v.y) / SCALE
}

fn ue_dir(v: Vec3) -> Vec3 {
    Vec3::new(-v.z, v.x, v.y)
}

/// A mesh-space frame (origin, axes) of a zed at `t` in Unreal world space:
/// position and axes (columns X, Y, Z).
fn world_axes(c: &ZedClass, t: &Transform, (o, axes): (Vec3, [Vec3; 3])) -> (Vec3, Mat3) {
    let to_actor = mesh_to_actor(c);
    let origin = to_actor(o);
    let axes = axes.map(|a| ue_dir(t.rotation * coords::dir((to_actor(o + a) - origin).normalize_or_zero().to_array())));
    let pos = ue_pos(t.transform_point(coords::pos(origin.to_array())));
    (pos, Mat3::from_cols(axes[0], axes[1], axes[2]))
}

/// Like `world_axes`, with the rotation as a rotator.
fn world_frame(c: &ZedClass, t: &Transform, frame: (Vec3, [Vec3; 3])) -> (Vec3, Vec3) {
    let (pos, m) = world_axes(c, t, frame);
    (pos, coords::ue_rotator_of(m))
}

/// AttachEmitterEffect: starts `class` at the zed's tag `tag`; it follows
/// the tag from then on.
#[allow(clippy::too_many_arguments)]
fn attach_effect(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    library: Option<&EffectLibrary>,
    c: &ZedClass,
    z: &mut Zed,
    t: &Transform,
    class: &str,
    tag: &'static str,
    seed: u32,
) {
    let (Some(library), Some(frame)) = (library, c.model.tag_frame(&z.last_pose, tag)) else {
        return;
    };
    let (pos, axes) = world_axes(c, t, frame);
    if let Some(e) = particles::spawn_effect(commands, library, meshes, class, pos, axes, seed) {
        z.effects.push((e, tag));
    }
}

/// The bone a shot hit (KF: native CalcHitLoc; here the bone segment, bone
/// to its first child, nearest the shot's line), its distance from the line
/// in Unreal units, and its name as KF sees it (its tag, else its name).
fn hit_bone(c: &ZedClass, pose: &[(Quat, Vec3)], t: &Transform, point: Vec3, dir: Vec3) -> Option<(usize, f32, String)> {
    let to_actor = mesh_to_actor(c);
    let world: Vec<Vec3> = pose.iter().map(|(_, p)| t.transform_point(coords::pos(to_actor(*p).to_array()))).collect();
    let bones = &c.model.mesh.bones;
    let d = dir.normalize_or_zero();
    (0..world.len())
        .map(|i| {
            let child = (i + 1..bones.len()).find(|&k| bones[k].parent == i);
            let end = child.map_or(world[i], |k| world[k]);
            (i, line_segment_distance(point, d, world[i], end) / SCALE)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, dist)| (i, dist, c.model.bone_tag(i).unwrap_or(&bones[i].name).to_string()))
}

/// KFMonster.DoDamageFX / ProcessHitFX for a zed's new damage events:
/// decapitation effects (gun: neck stump and brain chunks; knife: neck stump
/// and a flying head) and, on the killing hit, maybe a severed limb.
#[allow(clippy::too_many_arguments)]
fn apply_gore(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    gore: &GoreAssets,
    library: Option<&EffectLibrary>,
    c: &ZedClass,
    z: &mut Zed,
    entity: Entity,
    t: &Transform,
    clock: u32,
    always_sever: bool,
    decals: &mut MessageWriter<SpawnDecal>,
) {
    let hits = std::mem::take(&mut z.gore_hits);
    let velocity = ue_dir(z.velocity) / SCALE;
    let size = c.collision_radius * c.collision_height / 1100.0;
    for hit in hits {
        // Seeded with the clock too: KF's FRand differs every game.
        let mut rng = gore::Rng((z.random() ^ clock.wrapping_mul(2_654_435_761)) | 1);
        // KFMonster.OldPlayHit: the damage type's PawnDamageEmitter
        // (ROBloodPuff for the 9mm and knife) at the hit point pushed one
        // CollisionRadius away from the attacker, X axis toward the attacker.
        if hit.damage > 0.0
            && let Some(library) = library
        {
            let n = ue_dir(-hit.dir).normalize_or_zero();
            let at = ue_pos(hit.point) + n - n * c.collision_radius;
            let k = 65536.0 / std::f32::consts::TAU;
            let axes = coords::ue_rotation_matrix(Rotator {
                pitch: (n.z.clamp(-1.0, 1.0).asin() * k) as i32,
                yaw: (n.y.atan2(n.x) * k) as i32,
                roll: 0,
            });
            particles::spawn_effect(commands, library, meshes, "ROEffects.ROBloodPuff", at, axes, rng.0);
        }
        // KFMonster.TakeDamage: a ProjectileBloodSplat for blood-causing
        // damage, only 20% of the time for a hit within 0.2 s of the last;
        // it traces 350 units along the shot and splats the wall it reaches.
        let recent = z.since_hit < 0.2;
        z.since_hit = 0.0;
        if !recent || rng.frand() > 0.8 {
            decals.write(SpawnDecal {
                kind: DecalKind::WallSplat,
                at: ue_pos(hit.point),
                dir: ue_dir(hit.dir),
                trace: true,
            });
        }
        if z.last_pose.is_empty() {
            continue;
        }
        let mut next = z.id * 100 + z.next_piece;
        let first = next;
        let (point, attacker) = (ue_pos(hit.point), ue_pos(hit.attacker));
        if hit.decapitated {
            // HideBone(HeadBone) / SpecialHideHead: the neck stump.
            if !z.stumps.iter().any(|s| s.0 == StumpKind::Neck)
                && let Some(handles) = gore::new_stump(gore, StumpKind::Neck, commands, meshes, entity)
            {
                z.stumps.push((StumpKind::Neck, "neck", c.attach_scale[0], handles));
            }
            let Some(head) = c.model.tag_frame(&z.last_pose, "head") else {
                continue;
            };
            let (at, head_rot) = world_frame(c, t, head);
            // Neck spurt (NeckSpurtEmitterClass, or NeckSpurtNoGibEmitterClass
            // for the knife's SpecialHideHead) and DecapFX's BrainSplash.
            let jet = if hit.melee { "KFMod.DismembermentJetDecapitate" } else { "KFMod.DismembermentJetHead" };
            attach_effect(commands, meshes, library, c, z, t, jet, "neck", rng.0);
            if let Some(library) = library {
                particles::spawn_effect(commands, library, meshes, "ROEffects.BrainSplash", at, Mat3::IDENTITY, rng.0 ^ 0x9e37);
            }
            let (mut chunks, mut flying_head) = (0, false);
            if hit.melee {
                // DecapFX(.., bSpawnDetachedHead): the head flies off.
                if let Some(piece) = &c.severed_pieces[2] {
                    let rot_dir = gore::hit_normal_rotator(point, attacker, &mut rng);
                    gore::spawn_severed(commands, &mut gore::Effects { library, meshes: &mut *meshes }, piece, at, rot_dir, 0.06, head_rot, velocity, true, &mut next, &mut rng);
                    flying_head = true;
                }
            } else {
                // Brain chunks along the zed's own rotation.
                let rot = Vec3::new(0.0, z.yaw, 0.0);
                chunks = gore::spawn_giblets(commands, &mut gore::Effects { library, meshes: &mut *meshes }, gore, 3, at, rot, 0.06, 250.0, velocity, size, &mut next, &mut rng);
            }
            runlog::kv(
                "decap_fx",
                &format!(
                    "id={} melee={} stump={} brain_chunks={chunks} flying_head={flying_head} head_unreal=({:.0}, {:.0}, {:.0}) dead={}",
                    z.id,
                    hit.melee,
                    z.stumps.iter().any(|s| s.0 == StumpKind::Neck),
                    at.x,
                    at.y,
                    at.z,
                    hit.health_after <= 0.0
                ),
            );
        } else if hit.health_after <= 0.0 {
            let Some((bone, dist, name)) = hit_bone(c, &z.last_pose, t, hit.point, hit.dir) else {
                continue;
            };
            let mapped = bone_on_death(&name);
            runlog::kv(
                "gore_hit_bone",
                &format!("id={} bone={} tag={name} as={mapped} distance_to_shot_line_unreal={dist:.1}", z.id, c.model.mesh.bones[bone].name),
            );
            let Some(&(limb, stump_tag, stump_kind, piece, giblets)) = LIMBS.iter().find(|l| l.0 == mapped) else {
                continue;
            };
            if z.severed.contains(&limb) || (limb == "lfarm" && c.left_arm_gibbed) {
                continue;
            }
            // DismemberProbability, GibModifier 1 for our weapons.
            let chance = if always_sever { 1.0 } else { (hit.health_after - hit.damage).abs() / 130.0 };
            let roll = rng.frand();
            runlog::kv(
                "limb_roll",
                &format!(
                    "id={} limb={limb} health_after={:.1} damage={:.1} chance={chance:.2} roll={roll:.2} severed={}",
                    z.id,
                    hit.health_after,
                    hit.damage,
                    roll < chance
                ),
            );
            if roll >= chance {
                continue;
            }
            let Some(frame) = c.model.tag_frame(&z.last_pose, limb) else {
                continue;
            };
            let (at, bone_rot) = world_frame(c, t, frame);
            let rot_dir = gore::hit_normal_rotator(point, attacker, &mut rng);
            let piece_model = c.severed_pieces[piece].as_ref();
            if let Some(m) = piece_model {
                gore::spawn_severed(commands, &mut gore::Effects { library, meshes: &mut *meshes }, m, at, rot_dir, 0.25, bone_rot, velocity, false, &mut next, &mut rng);
            }
            let chunks = gore::spawn_giblets(commands, &mut gore::Effects { library, meshes: &mut *meshes }, gore, giblets, at, rot_dir, 0.25, 250.0, velocity, size, &mut next, &mut rng);
            if let Some(b) = c.model.tag_bone(limb) {
                z.hidden_bones.push(b);
            }
            let scale = c.attach_scale[if stump_kind == StumpKind::Arm { 1 } else { 2 }];
            // HideBone: LimbSpurtEmitterClass on the stump's tag.
            attach_effect(commands, meshes, library, c, z, t, "KFMod.DismembermentJetLimb", stump_tag, rng.0);
            let stump = gore::new_stump(gore, stump_kind, commands, meshes, entity);
            let has_stump = stump.is_some();
            if let Some(handles) = stump {
                z.stumps.push((stump_kind, stump_tag, scale, handles));
            }
            z.severed.push(limb);
            runlog::kv(
                "limb_severed",
                &format!(
                    "id={} limb={limb} hidden_bone={} piece={} brain_chunks={chunks} stump={has_stump} stump_tag={stump_tag} stump_scale={scale} at_unreal=({:.0}, {:.0}, {:.0}) piece_ids={first}..{}",
                    z.id,
                    c.model.tag_bone(limb).map_or("none", |b| c.model.mesh.bones[b].name.as_str()),
                    piece_model.map_or("none", |m| m.name.as_str()),
                    at.x,
                    at.y,
                    at.z,
                    next
                ),
            );
        }
        z.next_piece = next - z.id * 100;
    }
}

/// Moves a zed's attached effects to their tags' current frames.
fn follow_tags(c: &ZedClass, z: &Zed, t: &Transform, effects: &mut Query<&mut ParticleEffect>) {
    for (e, tag) in &z.effects {
        if let (Ok(mut fx), Some(frame)) = (effects.get_mut(*e), c.model.tag_frame(&z.last_pose, tag)) {
            fx.frame = world_axes(c, t, frame);
        }
    }
}

/// Player cylinder size (KFPawn), for melee reach.
const PLAYER_RADIUS: f32 = 20.0;
const PLAYER_EYE: f32 = 44.0;
const PLAYER_HALF_HEIGHT: f32 = 50.0;
/// Seconds a corpse stays (KF's RagdollLifeSpan is 30).
const CORPSE_SECONDS: f32 = 30.0;
/// KFMonster ragdoll launch values (Clot defaults; same for all KF zeds).
const RAG_DEATH_VEL: f32 = 100.0;
const RAG_SPIN_SCALE: f32 = 7.5;
const RAG_MAX_SPIN_AMOUNT: f32 = 100.0;
const RAG_INV_INERTIA: f32 = 4.0;
const GRAVITY: f32 = 950.0;

pub struct ZedPlugin;

impl Plugin for ZedPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ZedSettings>()
            .init_resource::<ZSpawn>()
            .insert_resource(ZedsActive(true))
            .add_systems(PostStartup, load_zed_classes)
            .add_systems(Update, (spawn_zeds, think_and_move, animate_zeds).chain());
    }
}

fn load_zed_classes(
    mut commands: Commands,
    request: Res<MapRequest>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let started = std::time::Instant::now();
    let set = PackageSet::new(&request.install_root);
    let defaults = ClassDefaults::new(&set);
    // KF's zed ragdolls all live in one Karma file.
    let ka_path = request.install_root.join("KarmaData").join("KF_Characters_Trip.ka");
    let ragdolls = match std::fs::read(&ka_path)
        .map_err(|e| e.to_string())
        .and_then(|b| ue_assets::karma::parse_ka(&String::from_utf8_lossy(&b)))
    {
        Ok(r) => r,
        Err(e) => {
            runlog::kv("ragdoll_file_error", &format!("path={} error=\"{e}\"", ka_path.display()));
            Default::default()
        }
    };
    let mut classes = Vec::new();
    for (kind, class_path) in ZED_CLASSES {
        match load_class(&set, &defaults, kind, class_path, &ragdolls, &mut meshes, &mut images, &mut materials) {
            Ok(c) => {
                runlog::kv(
                    "zed_class_loaded",
                    &format!(
                        "class={} bones={} triangles={} draw_scale={} pre_pivot={:?} collision={}x{} ground_speed={} turn_rate={} melee_range={} idle={:?} walk={:?} melee={:?} death={:?} death_hold_frame={} health={} head_health={} head_radius={} head_bone={:?} melee_damage={} ext_collision={:?}",
                        c.name,
                        c.model.mesh.bones.len(),
                        c.model.mesh.triangles.len(),
                        c.draw_scale,
                        c.pre_pivot.to_array(),
                        c.collision_radius,
                        c.collision_height,
                        c.ground_speed,
                        c.turn_rate,
                        c.melee_range,
                        c.idle,
                        c.walk,
                        c.melee,
                        c.death,
                        c.death_hold_frame,
                        c.health,
                        c.head_health,
                        c.head_radius,
                        c.head_bone.map(|b| c.model.mesh.bones[b].name.clone()),
                        c.melee_damage,
                        c.ext_collision
                    ),
                );
                runlog::kv(
                    "zed_class_sequences",
                    &format!(
                        "class={} sequences={:?}",
                        c.name,
                        c.model.anim.as_ref().map(|a| a.sequences.iter().map(|s| s.name.clone()).collect::<Vec<_>>())
                    ),
                );
                classes.push(c);
            }
            Err(e) => runlog::kv("zed_class_error", &format!("class={class_path} error=\"{e}\"")),
        }
    }
    runlog::kv(
        "zed_classes_ready",
        &format!("count={} seconds={:.2}", classes.len(), started.elapsed().as_secs_f64()),
    );
    commands.insert_resource(ZedClasses(classes));
}

#[allow(clippy::too_many_arguments)]
fn load_class(
    set: &PackageSet,
    defaults: &ClassDefaults,
    kind: ZedKind,
    class_path: &str,
    ragdolls: &std::collections::HashMap<String, ue_assets::karma::Ragdoll>,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> Result<ZedClass, String> {
    let (pkg_name, class_name) = class_path.split_once('.').ok_or("bad class path")?;
    let lp = set.load(pkg_name).ok_or("package not found")?;
    let export = (0..lp.pkg.exports.len())
        .find(|&i| {
            lp.pkg.export_class_name(i) == "Class"
                && lp.pkg.object_name(ObjectRef::Export(i)).eq_ignore_ascii_case(class_name)
        })
        .ok_or("class not found")?;
    let class = ObjectHandle { package: lp, export };
    let get = |p: &str| defaults.get(&class, p);
    // Some values (Health, MeleeDamage, ...) are stored as integers.
    let float = |p: &str, d: f32| match get(p) {
        Some((Value::Float(f), _)) => f,
        Some((Value::Int(i), _)) => i as f32,
        _ => d,
    };
    let name_of = |p: &str| match get(p) {
        Some((Value::Name(n), np)) => Some(np.pkg.name(n).to_string()),
        _ => None,
    };

    let (Value::Object(mesh_ref), mesh_pkg) = get("Mesh").ok_or("no Mesh default")? else {
        return Err("Mesh is not an object".into());
    };
    let mesh_h = set.resolve(&mesh_pkg, mesh_ref).ok_or("mesh not found")?;
    let skins = match get("Skins") {
        Some((Value::Array { count, raw }, p)) => {
            let mut r = ue_assets::reader::Reader::new(&raw);
            Skins {
                refs: (0..count)
                    .filter_map(|_| r.compact_index().ok().map(ObjectRef::from_raw))
                    .collect(),
                package: Some(p),
            }
        }
        _ => Skins {
            refs: Vec::new(),
            package: None,
        },
    };
    let model = SkinnedModel::load(set, &mesh_h, &skins, true, meshes, images, materials)?;
    let pre_pivot = match get("PrePivot") {
        Some((Value::Vector(v), _)) => Vec3::from_array(v),
        _ => Vec3::ZERO,
    };
    let turn_rate = match get("RotationRate") {
        Some((Value::Rotator(r), _)) => r.yaw as f32,
        _ => 20000.0,
    };
    // MeleeAnims is a fixed array of names: MeleeAnims, MeleeAnims[1], ...
    let melee = defaults
        .get_array_names(&class, "MeleeAnims")
        .iter()
        .filter_map(|n| model.sequence(n))
        .collect();
    let head_scale = float("HeadScale", 1.0);
    let head_bone = name_of("HeadBone").and_then(|n| model.find_bone(&n));
    let ext_collision = matches!(get("bUseExtendedCollision"), Some((Value::Bool(true), _))).then(|| {
        let offset = match get("ColOffset") {
            Some((Value::Vector(v), _)) => Vec3::from_array(v),
            _ => Vec3::ZERO,
        };
        (offset, float("ColRadius", 0.0), float("ColHeight", 0.0))
    });
    let death = model.sequence("KnockDown");
    // The frame of KnockDown where the root bone (pelvis) is lowest.
    let death_hold_frame = death.map_or(0.0, |d| {
        let len = model.length(d);
        (0..(len as usize).max(1))
            .map(|f| (f as f32, model.pose_with_bones(Some(d), f as f32).1[0].1.z))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(0.0, |(f, _)| f)
    });
    // KFMonster: RagdollOverride = KFRagdollName (a Karma asset name).
    let ragdoll = match get("KFRagdollName") {
        Some((Value::Str(name), _)) => match ragdolls.get(&name) {
            Some(ka) => match RagdollDef::new(ka, &model) {
                Ok((def, report)) => {
                    runlog::kv("ragdoll_loaded", &format!("class={class_path} {report}"));
                    log_hinge_check(&def, &model, name_of("MovementAnims").and_then(|n| model.sequence(&n)));
                    Some(def)
                }
                Err(e) => {
                    runlog::kv("ragdoll_error", &format!("class={class_path} ragdoll={name} error=\"{e}\""));
                    None
                }
            },
            None => {
                runlog::kv("ragdoll_error", &format!("class={class_path} ragdoll={name} error=\"not in file\""));
                None
            }
        },
        _ => None,
    };
    let hit_reactions = ["KFHitFront", "KFHitBack", "KFHitLeft", "KFHitRight"].map(|p| name_of(p).and_then(|n| model.sequence(&n)));
    let hit_anims: Vec<usize> = defaults
        .get_array_names(&class, "HitAnims")
        .iter()
        .filter_map(|n| model.sequence(n))
        .collect();
    let knock_down = model.sequence("KnockDown");
    let layered_attacks: Vec<usize> = if kind == ZedKind::Clot {
        ["ClotGrapple", "ClotGrappleTwo", "ClotGrappleThree"].iter().filter_map(|n| model.sequence(n)).collect()
    } else {
        Vec::new()
    };
    let headless_melee: Vec<usize> = if kind == ZedKind::Clot {
        ["Claw", "Claw", "Claw2"].iter().filter_map(|n| model.sequence(n)).collect()
    } else {
        Vec::new()
    };
    // Severed pieces (DetachedArmClass, DetachedLegClass, DetachedHeadClass).
    let severed_pieces = ["DetachedArmClass", "DetachedLegClass", "DetachedHeadClass"].map(|p| {
        let (Value::Object(r), pkg) = get(p)? else {
            return None;
        };
        let h = set.resolve(&pkg, r)?;
        let name = h.package.pkg.object_name(ObjectRef::Export(h.export)).to_string();
        match gore::load_piece(set, defaults, &h, &name, meshes, images, materials) {
            Ok(m) => {
                runlog::kv("gore_piece_loaded", &format!("class={class_path} slot={p} piece={name}"));
                Some(m)
            }
            Err(e) => {
                runlog::kv("gore_piece_error", &format!("class={class_path} slot={p} piece={name} error=\"{e}\""));
                None
            }
        }
    });
    // ZombieGoreFast: PostNetReceive swaps MovementAnims[0] for ZombieRun.
    let run_anim = if kind == ZedKind::Gorefast { model.sequence("ZombieRun") } else { None };
    let run_attack_seconds = model.sequence("GoreAttack1").map_or(0.0, |s| model.length(s) / model.rate(s).max(1e-3));
    let first_name = |p: &str| defaults.get_array_names(&class, p).first().and_then(|n| model.sequence(n));
    let air_anim = first_name("AirAnims");
    let land_anim = first_name("LandAnims");
    let turn_left = name_of("TurnLeftAnim").and_then(|n| model.sequence(&n));
    let turn_right = name_of("TurnRightAnim").and_then(|n| model.sequence(&n));
    let fire_root_bone = name_of("FireRootBone").and_then(|n| model.find_bone(&n));
    let spine_bone = name_of("SpineBone1").and_then(|n| model.find_bone(&n));
    let headless_walk = defaults
        .get_array_names(&class, "HeadlessWalkAnims")
        .first()
        .and_then(|n| model.sequence(n));
    let s = model.mesh.scale;
    if s[0] != s[1] || s[1] != s[2] || s[0] <= 0.0 {
        runlog::kv("ragdoll_warning", &format!("class={class_path} mesh_scale={s:?} (ragdolls assume a uniform positive scale)"));
    }
    Ok(ZedClass {
        kind,
        ragdoll,
        health_max: float("HealthMax", float("Health", 100.0)),
        bleed_out_duration: float("BleedOutDuration", 5.0),
        headless_walk,
        hit_reactions,
        hit_anims,
        knock_down,
        spine_bone,
        layered_attacks,
        headless_melee,
        air_anim,
        land_anim,
        turn_left,
        turn_right,
        fire_root_bone,
        grapple_duration: float("GrappleDuration", 0.0),
        run_anim,
        run_attack_seconds,
        severed_pieces,
        attach_scale: [
            float("SeveredHeadAttachScale", 1.0),
            float("SeveredArmAttachScale", 1.0),
            float("SeveredLegAttachScale", 1.0),
        ],
        left_arm_gibbed: matches!(get("bLeftArmGibbed"), Some((Value::Bool(true), _))),
        ext_collision,
        death,
        death_hold_frame,
        health: float("Health", 100.0),
        head_health: float("HeadHealth", 25.0),
        head_radius: float("HeadRadius", 7.0) * head_scale,
        head_offset: float("HeadHeight", 2.0) * head_scale,
        head_bone,
        melee_damage: float("MeleeDamage", 6.0),
        name: class_path.to_string(),
        draw_scale: float("DrawScale", 1.0),
        pre_pivot,
        collision_radius: float("CollisionRadius", 22.0),
        collision_height: float("CollisionHeight", 22.0),
        ground_speed: float("GroundSpeed", 440.0),
        turn_rate,
        melee_range: float("MeleeRange", 50.0),
        idle: name_of("IdleRestAnim").and_then(|n| model.sequence(&n)),
        walk: name_of("MovementAnims").and_then(|n| model.sequence(&n)),
        melee,
        model,
    })
}

/// Logs each hinge's angle range over the walk cycle next to its file limits,
/// with both sign conventions, to confirm which one contains the real motion.
fn log_hinge_check(def: &RagdollDef, model: &SkinnedModel, walk: Option<usize>) {
    let Some(walk) = walk else {
        return;
    };
    let len = model.length(walk) as usize;
    let mut ranges: Vec<(String, f32, f32, f32, f32)> = Vec::new();
    for f in 0..len.max(1) {
        let pose = model.pose_with_bones(Some(walk), f as f32).1;
        for (i, (name, phi, low, high)) in def.hinge_angles(&pose).into_iter().enumerate() {
            if ranges.len() <= i {
                ranges.push((name, phi, phi, low, high));
            }
            let r = &mut ranges[i];
            r.1 = r.1.min(phi);
            r.2 = r.2.max(phi);
        }
    }
    for (name, lo, hi, low, high) in ranges {
        let inside = |a: f32, b: f32| lo >= a - 0.05 && hi <= b + 0.05;
        runlog::kv(
            "ragdoll_hinge_check",
            &format!(
                "part={name} walk_angle_range=({lo:.2}, {hi:.2}) file_limits=({low:.2}, {high:.2}) inside_as_is={} inside_mirrored={}",
                inside(low, high),
                inside(-high, -low)
            ),
        );
    }
}

/// The mesh-to-world mapping for ragdoll bodies, for a zed at `actor`.
fn mesh_frame(c: &ZedClass, actor: &Transform) -> MeshFrame {
    let r = c.model.mesh.rot_origin;
    let rot = coords::ue_rotation_matrix(Rotator {
        pitch: r[0],
        yaw: r[1],
        roll: r[2],
    });
    MeshFrame::new(
        rot,
        c.model.mesh.scale[0],
        Vec3::from_array(c.model.mesh.origin),
        c.draw_scale,
        c.pre_pivot,
        actor,
    )
}

/// Mesh space -> actor-local Unreal space for a zed class.
fn mesh_to_actor(c: &ZedClass) -> impl Fn(Vec3) -> Vec3 + '_ {
    let r = c.model.mesh.rot_origin;
    // Applied as stored: checked on the Clot, whose feet point along mesh +Y;
    // RotOrigin yaw -16384 turns that to +X, the actor's forward.
    let rot = coords::ue_rotation_matrix(Rotator {
        pitch: r[0],
        yaw: r[1],
        roll: r[2],
    });
    let scale = Vec3::from_array(c.model.mesh.scale);
    let origin = Vec3::from_array(c.model.mesh.origin);
    move |p| c.pre_pivot + c.draw_scale * (rot * ((p - origin) * scale))
}

/// Unreal yaw (rotation units) of a horizontal Bevy direction.
fn yaw_of(dir: Vec3) -> f32 {
    // Bevy (x, _, z) -> Unreal (x, y) = (-z, x)
    (dir.x).atan2(-dir.z) * 65536.0 / std::f32::consts::TAU
}

/// Horizontal Bevy direction of an Unreal yaw.
fn dir_of(yaw: f32) -> Vec3 {
    let a = yaw * std::f32::consts::TAU / 65536.0;
    coords::dir([a.cos(), a.sin(), 0.0])
}

impl Zed {
    /// Marks the zed dead (death animation is started by the think system).
    pub fn kill(&mut self) {
        self.health = 0.0;
        self.bleed_out = None;
        self.state = ZedState::Dead;
        self.dead_for = 0.0;
    }

    /// KFMonster.RemoveHead (the damage part is in combat.rs): the head is
    /// gone; if the zed is still alive it bleeds out after BleedOutDuration.
    pub fn remove_head(&mut self) {
        self.decapitated = true;
        self.head_health = 0.0;
        self.since_decap = Some(0.0);
        if self.headless_claws {
            self.melee_damage *= 2.0;
            self.melee_range *= 2.0;
        }
        // RunningState.RemoveHead: stop running; RangedAttack never starts
        // it again once headless.
        self.running = false;
        self.run_attack_timeout = 0.0;
        if self.health > 0.0 {
            self.bleed_out = Some(self.bleed_out_duration);
        }
    }

    fn random(&mut self) -> u32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng
    }

    /// ZombieGoreFast running, once per think: `dist` to the target (Unreal
    /// units), whether an attack is in progress (bShotAnim). Returns
    /// "started" or the reason it stopped, for the log.
    fn update_running(&mut self, dist: f32, attacking: bool, dt: f32) -> Option<&'static str> {
        if !self.can_run || self.decapitated {
            return None;
        }
        if !self.running {
            // RangedAttack: no attack started and the target within 700.
            if attacking || dist > GOREFAST_RUN_DISTANCE {
                return None;
            }
            self.running = true;
            self.charge_check = 0.0;
            self.run_attack_timeout = 0.0;
            return Some("started");
        }
        // RunningState.Tick: a moving attack ends the run when its time is up.
        if self.run_attack_timeout > 0.0 {
            self.run_attack_timeout -= dt;
            if self.run_attack_timeout <= 0.0 {
                self.run_attack_timeout = 0.0;
                self.running = false;
                return Some("run_attack_done");
            }
        }
        // CheckCharge: still within 700? Then sleep 0.5 + FRand() x 0.5.
        self.charge_check -= dt;
        if self.charge_check <= 0.0 {
            if dist < GOREFAST_RUN_DISTANCE {
                self.charge_check = 0.5 + 0.5 * (self.random() % 1000) as f32 / 1000.0;
            } else {
                self.running = false;
                return Some("target_far");
            }
        }
        None
    }

    /// The reaction to one damage event that left the zed alive, after
    /// KFMonster.PlayHit (FlipOver), PlayTakeHit and PlayDirectionalHit.
    /// `damage` includes any headshot multiplier; `hit` is the hit point and
    /// `attacker` the attacker's centre (Bevy space). Returns what was chosen.
    pub fn take_hit(&mut self, damage: f32, hit: Vec3, attacker: Vec3, melee: bool) -> Option<HitReaction> {
        if self.health <= 0.0 || damage <= 0.0 {
            return None;
        }
        // PlayHit: a hit of more than Health / 1.5 knocks the zed down.
        if damage > self.default_health / 1.5 {
            self.pending_reaction = Some(HitReaction::KnockDown);
            return self.pending_reaction;
        }
        // PlayTakeHit: at most one pain animation every 0.5 s; under 5
        // damage no animation for our damage types (9mm, knife).
        if self.since_pain_anim < MIN_TIME_BETWEEN_PAIN_ANIMS {
            return None;
        }
        self.since_pain_anim = 0.0;
        if damage < 5.0 {
            return None;
        }
        // PlayDirectionalHit, in Unreal axes: X = facing, Y = right.
        let rel = (hit - self.centre) / SCALE;
        let mut dir = Vec2::new(-rel.z, rel.x);
        if dir.length() < 1.0 {
            let a = (self.random() % 3600) as f32 / 3600.0 * std::f32::consts::TAU;
            dir = Vec2::new(a.cos(), a.sin());
        }
        let dir = dir.normalize();
        let yaw = self.yaw * std::f32::consts::TAU / 65536.0;
        let (x, y) = (Vec2::new(yaw.cos(), yaw.sin()), Vec2::new(-yaw.sin(), yaw.cos()));
        let reaction = if dir.dot(x) > 0.7 {
            let close_melee = melee
                && (attacker - self.centre).length() / SCALE <= self.melee_range * 2.0
                && damage > 0.1 * self.default_health;
            if damage >= 0.5 * self.default_health || close_melee {
                self.stunned = STUN_TIME;
                HitReaction::Stun
            } else {
                HitReaction::Front
            }
        } else if dir.dot(x) < -0.7 {
            HitReaction::Back
        } else if dir.dot(y) > 0.0 {
            HitReaction::Right
        } else {
            HitReaction::Left
        };
        self.pending_reaction = Some(reaction);
        Some(reaction)
    }

    /// A Clot with KF's values, for tests.
    #[cfg(test)]
    pub fn test_clot() -> Zed {
        Zed {
            id: 0,
            class: 0,
            centre: Vec3::ZERO,
            radius: 26.0,
            half_height: 44.0,
            health: 130.0,
            health_max: 130.0,
            head_health: 25.0,
            decapitated: false,
            bleed_out: None,
            bleed_out_duration: 5.0,
            overlay: None,
            attack: None,
            since_decap: None,
            pending_reaction: None,
            since_pain_anim: f32::MAX,
            stunned: 0.0,
            default_health: 130.0,
            melee_range: 20.0,
            melee_damage: 6.0,
            headless_claws: true,
            can_run: false,
            running: false,
            charge_check: 0.0,
            run_attack_timeout: 0.0,
            rng: 12345,
            head_radius: 7.7,
            head: None,
            ext: None,
            dead_for: 0.0,
            last_hit: None,
            velocity: Vec3::ZERO,
            gore_hits: Vec::new(),
            stumps: Vec::new(),
            hidden_bones: Vec::new(),
            severed: Vec::new(),
            last_pose: Vec::new(),
            next_piece: 0,
            effects: Vec::new(),
            since_hit: f32::MAX,
            yaw: 0.0,
            state: ZedState::Idle,
            vertical_speed: 0.0,
            sequence: None,
            frame: 0.0,
            looping: true,
            meshes: Vec::new(),
        }
    }

    /// A Gorefast's running state with KF's values, for tests.
    #[cfg(test)]
    pub fn test_gorefast() -> Zed {
        Zed {
            can_run: true,
            headless_claws: false,
            ..Zed::test_clot()
        }
    }

    #[cfg(test)]
    pub fn melee_values(&self) -> (f32, f32) {
        (self.melee_range, self.melee_damage)
    }

    #[cfg(test)]
    pub fn is_dead(&self) -> bool {
        self.state == ZedState::Dead
    }

    /// The collision cylinder, if it still blocks (corpses do not).
    pub fn blocking_cylinder(&self) -> Option<Cylinder> {
        (self.state != ZedState::Dead).then_some(Cylinder {
            centre: self.centre,
            radius: self.radius * SCALE,
            half_height: self.half_height * SCALE,
        })
    }
}

/// KFMonster.PlayDyingAnimation's start motion: 0.6 x the zed's horizontal
/// velocity (full vertical) plus RagDeathVel along the shot; spin =
/// RagInvInertia x (hit offset, sideways part scaled by RagSpinScale and
/// capped at RagMaxSpinAmount) cross that push, capped at Karma's
/// KMaxAngularSpeed. RagDeathUpKick is 0 for KF zeds.
fn death_launch(z: &Zed) -> Launch {
    let Some((hit, dir)) = z.last_hit else {
        return Launch {
            velocity: Vec3::new(0.6 * z.velocity.x, z.velocity.y, 0.6 * z.velocity.z),
            angular_velocity: Vec3::ZERO,
        };
    };
    let push = dir.normalize_or_zero() * RAG_DEATH_VEL; // Unreal units/s, Bevy axes
    let mut velocity = Vec3::new(0.6 * z.velocity.x, z.velocity.y, 0.6 * z.velocity.z) + push * SCALE;
    let max_speed = ragdoll::MAX_SPEED * SCALE;
    if velocity.length() > max_speed {
        velocity = velocity.normalize() * max_speed;
    }
    // Hit offset in Unreal axes (X, Y horizontal), scaled and capped as KF does.
    let r = (hit - z.centre) / SCALE;
    let mut rel = Vec3::new(-r.z, r.x, r.y);
    rel.x = (rel.x * RAG_SPIN_SCALE).clamp(-RAG_MAX_SPIN_AMOUNT, RAG_MAX_SPIN_AMOUNT);
    rel.y = (rel.y * RAG_SPIN_SCALE).clamp(-RAG_MAX_SPIN_AMOUNT, RAG_MAX_SPIN_AMOUNT);
    // Back to Bevy axes (Unreal units) for a right-handed cross product.
    let rel_bevy = Vec3::new(rel.y, rel.z, -rel.x);
    // Units of RagInvInertia are not known; the result is capped anyway.
    let mut angular_velocity = RAG_INV_INERTIA * rel_bevy.cross(push) * SCALE * SCALE;
    if angular_velocity.length() > ragdoll::MAX_ANGULAR_SPEED {
        angular_velocity = angular_velocity.normalize() * ragdoll::MAX_ANGULAR_SPEED;
    }
    Launch {
        velocity,
        angular_velocity,
    }
}

fn start_anim(z: &mut Zed, seq: Option<usize>, looping: bool) {
    if z.sequence != seq {
        z.sequence = seq;
        z.frame = 0.0;
    }
    z.looping = looping;
}

/// Spawns a zed of class `class` `distance` units in front of the camera, on the floor.
#[allow(clippy::too_many_arguments)]
fn spawn_in_front(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    classes: &ZedClasses,
    spatial: &SpatialQuery,
    cam_t: &Transform,
    cam: &FlyCamera,
    class: usize,
    id: usize,
    distance: f32,
    lift: f32,
) {
    let c = &classes.0[class];
    let forward = Vec3::new(-cam.yaw.sin(), 0.0, -cam.yaw.cos());
    let right = Vec3::new(cam.yaw.cos(), 0.0, -cam.yaw.sin());
    // Side by side, 60 units apart (0, +60, -60, +120, ...), so zeds spawned
    // in a row do not start inside each other.
    let side = (id.div_ceil(2) as f32) * if id % 2 == 1 { 60.0 } else { -60.0 };
    let probe = cam_t.translation + (forward * distance + right * side) * SCALE;
    let Some(hit) = spatial.cast_ray(probe, Dir3::NEG_Y, 20.0, true, &crate::collision::world_filter()) else {
        runlog::kv("zed_spawn_failed", "reason=no_floor_below");
        return;
    };
    let centre = probe - Vec3::Y * hit.distance + Vec3::Y * (c.collision_height + 1.0 + lift) * SCALE;
    let yaw = yaw_of(-forward);
    let handles = c.model.new_instance(meshes);
    let parent = commands
        .spawn((
            Transform {
                translation: centre,
                rotation: coords::rotation(Rotator { pitch: 0, yaw: yaw as i32, roll: 0 }),
                ..default()
            },
            Visibility::Visible,
            Zed {
                id,
                class,
                centre,
                radius: c.collision_radius,
                half_height: c.collision_height,
                health: c.health,
                health_max: c.health_max,
                head_health: c.head_health,
                decapitated: false,
                bleed_out: None,
                bleed_out_duration: c.bleed_out_duration,
                overlay: None,
                attack: None,
                since_decap: None,
                pending_reaction: None,
                since_pain_anim: f32::MAX,
                stunned: 0.0,
                default_health: c.health,
                melee_range: c.melee_range,
                melee_damage: c.melee_damage,
                headless_claws: !c.headless_melee.is_empty(),
                can_run: c.run_anim.is_some(),
                running: false,
                charge_check: 0.0,
                run_attack_timeout: 0.0,
                rng: 0x9E37_79B9 ^ (id as u32).wrapping_mul(2_654_435_761),
                head_radius: c.head_radius,
                head: None,
                ext: None,
                dead_for: 0.0,
                last_hit: None,
                velocity: Vec3::ZERO,
                gore_hits: Vec::new(),
                stumps: Vec::new(),
                hidden_bones: Vec::new(),
                severed: Vec::new(),
                last_pose: Vec::new(),
                next_piece: 0,
                effects: Vec::new(),
                since_hit: f32::MAX,
                yaw,
                state: ZedState::Idle,
                vertical_speed: 0.0,
                sequence: None,
                frame: 0.0,
                looping: true,
                meshes: handles.clone(),
            },
        ))
        .id();
    for (part, handle) in c.model.parts.iter().zip(handles) {
        commands.spawn((Mesh3d(handle), MeshMaterial3d(part.material.clone()), Transform::IDENTITY, ChildOf(parent)));
    }
    let u = centre / SCALE;
    runlog::kv(
        "zed_spawned",
        &format!("id={id} class={} centre_unreal=({:.0}, {:.0}, {:.0}) yaw={yaw:.0}", c.name, -u.z, u.x, u.y),
    );
}

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn spawn_zeds(
    mut commands: Commands,
    frames: Res<bevy::diagnostic::FrameCount>,
    settings: Res<ZedSettings>,
    keys: Res<ButtonInput<KeyCode>>,
    classes: Option<Res<ZedClasses>>,
    spatial: SpatialQuery,
    cams: Query<(&Transform, &FlyCamera)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut active: ResMut<ZedsActive>,
    mut z_spawn: ResMut<ZSpawn>,
    script: Res<crate::weapon::ScriptedInput>,
    mut next_id: Local<usize>,
) {
    if keys.just_pressed(KeyCode::KeyX) {
        active.0 = !active.0;
        runlog::kv("zeds_active", &format!("active={}", active.0));
    }
    let Some(classes) = classes else {
        return;
    };
    if classes.0.is_empty() || frames.0 < 6 {
        return;
    }
    let start = frames.0 == 6;
    let scripted = |action: &str| script.0.iter().any(|(f, a)| *f == frames.0 && a == action);
    if keys.just_pressed(KeyCode::KeyN) || scripted("cycle_zed") {
        z_spawn.class = (z_spawn.class + 1) % classes.0.len();
        z_spawn.label = format!("{:?}", classes.0[z_spawn.class].kind);
        runlog::kv("z_spawn_selected", &format!("kind={}", z_spawn.label));
    }
    // Z spawns the type picked with N.
    let z = keys.just_pressed(KeyCode::KeyZ);
    let z_kind = classes.0[z_spawn.class.min(classes.0.len() - 1)].kind;
    // Test action: a Clot dropped from 200 units up (falling and landing).
    let dropped = scripted("zed_drop");
    let clot = (settings.spawn_at_start && start) || scripted("zed") || dropped || (z && z_kind == ZedKind::Clot);
    // Test action: a Gorefast 900 units away (outside its 700 running range).
    let far = scripted("gorefast_far");
    let gorefast = (settings.gorefast_at_start && start)
        || scripted("gorefast")
        || far
        || keys.just_pressed(KeyCode::KeyG)
        || (z && z_kind == ZedKind::Gorefast);
    let Ok((t, cam)) = cams.single() else {
        return;
    };
    for (wanted, kind) in [(clot, ZedKind::Clot), (gorefast, ZedKind::Gorefast)] {
        if !wanted {
            continue;
        }
        let Some(class) = classes.0.iter().position(|c| c.kind == kind) else {
            runlog::kv("zed_spawn_failed", &format!("reason=class_not_loaded kind={kind:?}"));
            continue;
        };
        let lift = if dropped && kind == ZedKind::Clot { 200.0 } else { 0.0 };
        let distance = if far && kind == ZedKind::Gorefast { 900.0 } else { 300.0 };
        spawn_in_front(&mut commands, &mut meshes, &classes, &spatial, t, cam, class, *next_id, distance, lift);
        *next_id += 1;
    }
}

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn think_and_move(
    mut commands: Commands,
    time: Res<Time>,
    active: Res<ZedsActive>,
    classes: Option<Res<ZedClasses>>,
    spatial: SpatialQuery,
    player: Query<(&Transform, Option<&Walker>), With<FlyCamera>>,
    mut zeds: Query<(Entity, &mut Zed, &mut Transform, Option<&RagdollState>), Without<FlyCamera>>,
    mut player_damage: MessageWriter<crate::combat::PlayerDamaged>,
    mut kills: ResMut<crate::combat::KillCount>,
    mut pinned: ResMut<crate::combat::PlayerPinned>,
    mut log_timer: Local<f32>,
) {
    let Some(classes) = classes else {
        return;
    };
    let Ok((pt, walker)) = player.single() else {
        return;
    };
    // Player cylinder centre: the walker's, or below the flying camera.
    let target = walker.map_or(pt.translation - Vec3::Y * PLAYER_EYE * SCALE, |w| w.center);
    let dt = time.delta_secs().min(0.1);
    *log_timer += dt;
    let log_now = *log_timer >= 1.0;
    if log_now {
        *log_timer = 0.0;
    }
    // Blocking cylinders: the player first, then every living zed (kept up
    // to date as each zed moves, so later zeds see the new positions).
    let player_cylinder = Cylinder {
        centre: target,
        radius: PLAYER_RADIUS * SCALE,
        half_height: PLAYER_HALF_HEIGHT * SCALE,
    };
    // A flying (no-clip) camera does not block.
    let mut blockers: Vec<(Option<Entity>, Cylinder)> = Vec::new();
    if walker.is_some() {
        blockers.push((None, player_cylinder));
    }
    let zeds_ids: std::collections::HashMap<Entity, usize> = zeds.iter().map(|(e, z, _, _)| (e, z.id)).collect();
    blockers.extend(zeds.iter().filter_map(|(e, z, _, _)| z.blocking_cylinder().map(|c| (Some(e), c))));
    for (entity, mut z, mut t, ragdoll_state) in &mut zeds {
        let c = &classes.0[z.class];
        if z.state == ZedState::Dead {
            // On death: a ragdoll if the class has one, else the death
            // animation. The corpse is removed after CORPSE_SECONDS.
            if z.dead_for == 0.0 {
                if pinned.by == Some(z.id) {
                    pinned.release("grabber_died");
                }
                runlog::kv("zed_died", &format!("id={} decapitated={}", z.id, z.decapitated));
                match &c.ragdoll {
                    Some(def) => {
                        let pose = c.model.pose_collapsed(z.sequence, z.frame, &[]).1;
                        let launch = death_launch(&z);
                        runlog::kv(
                            "ragdoll_started",
                            &format!(
                                "id={} ragdoll={} velocity_unreal={:.0} angular_velocity={:.1} hit={}",
                                z.id,
                                def.name,
                                launch.velocity.length() / SCALE,
                                launch.angular_velocity.length(),
                                z.last_hit.is_some()
                            ),
                        );
                        let state = ragdoll::spawn(&mut commands, def, &pose, mesh_frame(c, &t), &launch, z.id);
                        commands.entity(entity).insert(state);
                    }
                    None => {
                        z.sequence = None;
                        start_anim(&mut z, c.death, false);
                    }
                }
            }
            z.dead_for += dt;
            if log_now && ragdoll_state.is_none() {
                runlog::kv(
                    "zed_corpse",
                    &format!("id={} dead_for={:.1} sequence={:?} frame={:.1}", z.id, z.dead_for, z.sequence, z.frame),
                );
            }
            if z.dead_for > CORPSE_SECONDS {
                if let Some(r) = ragdoll_state {
                    for &e in r.joints.iter().chain(&r.bodies) {
                        commands.entity(e).despawn();
                    }
                }
                commands.entity(entity).despawn();
                runlog::kv("zed_removed", &format!("id={}", z.id));
            }
            continue;
        }
        // Bleeding out (KFMonster.Tick): dies when the time is up. No hit
        // momentum, so the ragdoll gets no push.
        if let Some(left) = z.bleed_out {
            let left = left - dt;
            if left <= 0.0 {
                z.last_hit = None;
                z.kill();
                kills.0 += 1; // credited to the player, as KF credits LastDamagedBy
                runlog::kv("zed_bled_out", &format!("id={} health_left={:.1}", z.id, z.health));
                continue;
            }
            z.bleed_out = Some(left);
        }
        z.since_pain_anim = (z.since_pain_anim + dt).min(1e6);
        z.since_hit = (z.since_hit + dt).min(1e6);
        if let Some(s) = z.since_decap.as_mut() {
            *s += dt;
        }
        // A grab ends when the Clot loses its head (ZombieClot.RemoveHead).
        if z.decapitated && pinned.by == Some(z.id) {
            pinned.release("grabber_decapitated");
        }
        z.stunned = (z.stunned - dt).max(0.0);
        // FlipOver: full-body KnockDown; stand still until it ends.
        if z.pending_reaction == Some(HitReaction::KnockDown) && c.knock_down.is_some() {
            z.pending_reaction = None;
            z.overlay = None;
            z.attack = None;
            z.state = ZedState::KnockedDown;
            z.sequence = None;
            start_anim(&mut z, c.knock_down, false);
            runlog::kv("zed_hit_reaction", &format!("id={} reaction=KnockDown", z.id));
        }
        if matches!(z.state, ZedState::KnockedDown | ZedState::Landing) {
            if z.sequence.is_some_and(|s| z.frame < c.model.length(s) - 0.5) {
                t.translation = z.centre;
                continue;
            }
            runlog::kv("zed_state", &format!("id={} from={:?} to=Chase reason=animation_done", z.id, z.state));
            z.state = ZedState::Chase;
        }
        let old_yaw = z.yaw;
        let old_sequence = z.sequence;
        let old_centre = z.centre;
        let mover = Mover::new(&spatial, c.collision_radius, c.collision_height);
        let to = (target - z.centre).with_y(0.0);
        let dist = to.length() / SCALE;
        // Attack once within MeleeRange of touching (KF's melee start); the
        // damage check later allows MeleeRange x 1.4.
        let reach = z.melee_range + c.collision_radius + PLAYER_RADIUS;
        let old_state = z.state;

        if active.0 && z.state != ZedState::Falling {
            // Turn toward the player at RotationRate.
            if dist > 1.0 {
                let want = yaw_of(to);
                let mut delta = (want - z.yaw).rem_euclid(65536.0);
                if delta > 32768.0 {
                    delta -= 65536.0;
                }
                let step = c.turn_rate * dt;
                z.yaw += delta.clamp(-step, step);
            }
            // The attack in progress: on the upper-body layer (the grab; ends
            // when its layer ends or is replaced by a flinch) or full body.
            if let Some(a) = z.attack
                && a.layered
                && !z.overlay.is_some_and(|(s, _, _)| s == a.seq)
            {
                z.attack = None;
            }
            let progress = z.attack.map(|a| {
                let frame = if a.layered { z.overlay.map_or(0.0, |(_, f, _)| f) } else { z.frame };
                frame / c.model.length(a.seq).max(1.0)
            });
            // Damage lands halfway through the attack (KFMonster.MeleeDamageTarget):
            // the target still within MeleeRange x 1.4 + both radii, roughly
            // level, the zed not stunned and not in its 2 s after losing its head.
            if let (Some(p), Some(a)) = (progress, z.attack)
                && p >= 0.5
                && !a.hit_done
            {
                z.attack = Some(Attack { hit_done: true, ..a });
                let in_range = dist <= z.melee_range * 1.4 + c.collision_radius + PLAYER_RADIUS;
                let dz = ((target.y - z.centre.y) / SCALE).abs();
                let level = dz <= c.collision_height.max(50.0) + 0.5 * c.collision_height.min(50.0);
                let dazed = z.since_decap.is_some_and(|s| s < 2.0);
                if in_range && level && z.stunned <= 0.0 && !dazed {
                    // ClawDamageTarget: MeleeDamage -5% .. +5%.
                    let roll = (z.random() % 1000) as f32 / 1000.0;
                    let amount = z.melee_damage * 0.95 + z.melee_damage * 0.1 * roll;
                    player_damage.write(crate::combat::PlayerDamaged { amount, zed_id: z.id });
                    // ZombieClot: a landed grab pins the player (not when headless).
                    if c.grapple_duration > 0.0 && !z.decapitated && walker.is_some() {
                        pinned.pin(c.grapple_duration, z.id);
                    }
                } else {
                    runlog::kv(
                        "zed_attack_missed",
                        &format!("id={} in_range={in_range} level={level} stunned={} dazed={dazed}", z.id, z.stunned > 0.0),
                    );
                }
            }
            // ZombieClot.Tick: the grab animation stops if the target gets out
            // of reach (MeleeRange + both radii).
            if c.kind == ZedKind::Clot && z.attack.is_some_and(|a| a.layered) && dist > z.melee_range + c.collision_radius + PLAYER_RADIUS {
                z.attack = None;
                z.overlay = None;
                runlog::kv("zed_grab_broken", &format!("id={} distance_unreal={dist:.0}", z.id));
            }
            let full_body_busy = z.attack.is_some_and(|a| !a.layered)
                && z.sequence.is_some_and(|s| z.frame < c.model.length(s) - 0.5);
            if z.attack.is_some_and(|a| !a.layered) && !full_body_busy {
                z.attack = None;
            }
            let melee = if z.decapitated && !c.headless_melee.is_empty() { &c.headless_melee } else { &c.melee };
            if z.attack.is_none() && dist <= reach && !melee.is_empty() && z.stunned <= 0.0 {
                // (CanAttack is false while stunned.) A random attack (Rand(3)).
                let slot = z.random() as usize % melee.len();
                let mut seq = melee[slot];
                // RemoveHead: "no head so biting is out": Claw3 becomes Claw2
                // (slot 2) or Claw1 (slot 1).
                if z.decapitated && c.model.sequence_name(seq).is_some_and(|n| n.eq_ignore_ascii_case("Claw3")) {
                    let swap = if slot == 2 { "Claw2" } else { "Claw1" };
                    seq = c.model.sequence(swap).unwrap_or(seq);
                }
                // ZombieGoreFast RunningState.RangedAttack: while running, a
                // moving attack with ChargeChance (upper body, keeps running
                // for RunAttackTimeout); otherwise full body, and the run ends.
                let charge = z.running
                    && c.fire_root_bone.is_some()
                    && (z.random() % 1000) as f32 / 1000.0 < GOREFAST_CHARGE_CHANCE;
                if z.running {
                    if charge {
                        z.run_attack_timeout = c.run_attack_seconds;
                    } else {
                        z.running = false;
                        runlog::kv("gorefast_run", &format!("id={} running=false reason=full_body_attack distance_unreal={dist:.0}", z.id));
                    }
                }
                let layered = (charge || c.layered_attacks.contains(&seq)) && c.fire_root_bone.is_some();
                if layered {
                    z.overlay = Some((seq, 0.0, c.fire_root_bone.expect("checked")));
                } else {
                    z.state = ZedState::Melee;
                    z.sequence = None; // force restart even if the same attack
                    start_anim(&mut z, Some(seq), false);
                }
                z.attack = Some(Attack {
                    seq,
                    layered,
                    hit_done: false,
                });
                runlog::kv(
                    "zed_attack",
                    &format!("id={} sequence={} layered={layered} charge={charge}", z.id, c.model.sequence_name(seq).unwrap_or("?")),
                );
            }
            let attacking = z.attack.is_some();
            if let Some(what) = z.update_running(dist, attacking, dt) {
                runlog::kv(
                    "gorefast_run",
                    &format!("id={} running={} reason={what} distance_unreal={dist:.0}", z.id, z.running),
                );
            }
            if z.attack.is_some_and(|a| !a.layered) {
                // Full-body attack: stand and finish it.
            } else {
                // Walking, also during a grab (ZombieClot.Tick keeps
                // accelerating toward the target while attacking).
                z.state = ZedState::Chase;
                // RemoveHead: GroundSpeed x 0.8 once headless; a running
                // Gorefast x 1.875.
                let speed = if z.decapitated {
                    c.ground_speed * 0.8
                } else if z.running {
                    c.ground_speed * GOREFAST_RUN_SPEED
                } else {
                    c.ground_speed
                };
                let delta = dir_of(z.yaw) * speed * SCALE * dt;
                let others: Vec<(Option<Entity>, Cylinder)> =
                    blockers.iter().filter(|(e, _)| *e != Some(entity)).copied().collect();
                let shapes: Vec<Cylinder> = others.iter().map(|(_, c)| *c).collect();
                let me = z.blocking_cylinder().expect("living zed");
                let (delta, blocked) = clip_move(&me, delta, &shapes);
                if let Some(i) = blocked
                    && log_now
                {
                    let by = others[i].0.and_then(|e| zeds_ids.get(&e).copied()).map_or("player".into(), |id| format!("zed {id}"));
                    runlog::kv("pawn_blocked", &format!("mover=zed {} by={by}", z.id));
                }
                let (moved, _) = mover.ground_move(z.centre, delta);
                match mover.snap_to_floor(moved) {
                    Some(on_floor) => z.centre = on_floor,
                    None => {
                        z.centre = moved;
                        z.state = ZedState::Falling;
                        z.vertical_speed = 0.0;
                    }
                }
            }
        } else if !active.0 && z.state != ZedState::Falling {
            z.state = ZedState::Idle;
        }
        if z.state == ZedState::Falling {
            z.vertical_speed -= GRAVITY * SCALE * dt;
            let (moved, hit) = mover.slide(z.centre, Vec3::Y * z.vertical_speed * dt);
            z.centre = moved;
            if hit.is_some_and(|h| h.normal.y > 0.7) {
                z.vertical_speed = 0.0;
                if c.land_anim.is_some() {
                    z.state = ZedState::Landing;
                    z.sequence = None;
                    start_anim(&mut z, c.land_anim, false);
                } else {
                    z.state = ZedState::Idle;
                }
            }
        }

        match z.state {
            ZedState::Chase => {
                // Not actually moving (e.g. pressed against the player): idle,
                // or a turn-in-place animation while turning. The engine picks
                // these natively; this is an approximation of that rule.
                let speed = if dt > 0.0 { (z.centre - old_centre).with_y(0.0).length() / SCALE / dt } else { 0.0 };
                let turn_rate = if dt > 0.0 { (z.yaw - old_yaw) / dt } else { 0.0 };
                let anim = if speed >= STANDING_SPEED {
                    if z.decapitated {
                        c.headless_walk.or(c.walk)
                    } else if z.running {
                        c.run_anim.or(c.walk)
                    } else {
                        c.walk
                    }
                } else if turn_rate > TURNING_RATE {
                    c.turn_right.or(c.idle)
                } else if turn_rate < -TURNING_RATE {
                    c.turn_left.or(c.idle)
                } else {
                    c.idle
                };
                start_anim(&mut z, anim, true)
            }
            ZedState::Falling => start_anim(&mut z, c.air_anim.or(c.idle), true),
            ZedState::Idle => start_anim(&mut z, c.idle, true),
            ZedState::Melee | ZedState::KnockedDown | ZedState::Landing | ZedState::Dead => {}
        }
        if z.sequence != old_sequence {
            runlog::kv(
                "zed_anim",
                &format!("id={} state={:?} sequence={}", z.id, z.state, z.sequence.and_then(|s| c.model.sequence_name(s)).unwrap_or("none")),
            );
        }
        if z.state != old_state {
            runlog::kv(
                "zed_state",
                &format!("id={} from={old_state:?} to={:?} distance_unreal={dist:.0}", z.id, z.state),
            );
        }
        if dt > 0.0 {
            z.velocity = (z.centre - old_centre) / dt;
        }
        if let Some(b) = blockers.iter_mut().find(|(e, _)| *e == Some(entity)) {
            b.1.centre = z.centre;
        }
        t.translation = z.centre;
        t.rotation = coords::rotation(Rotator { pitch: 0, yaw: z.yaw as i32, roll: 0 });
        if log_now {
            let u = z.centre / SCALE;
            runlog::kv(
                "zed",
                &format!(
                    "id={} state={:?} centre_unreal=({:.0}, {:.0}, {:.0}) distance_to_player={dist:.0} yaw={:.0} sequence={:?} frame={:.1} speed_unreal={:.0} running={}",
                    z.id,
                    z.state,
                    -u.z,
                    u.x,
                    u.y,
                    z.yaw,
                    z.sequence,
                    z.frame,
                    z.velocity.with_y(0.0).length() / SCALE,
                    z.running
                ),
            );
        }
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)] // Bevy system parameters
fn animate_zeds(
    mut commands: Commands,
    time: Res<Time>,
    classes: Option<Res<ZedClasses>>,
    gore: Option<Res<GoreAssets>>,
    library: Option<Res<EffectLibrary>>,
    mut effects: Query<&mut ParticleEffect>,
    settings: Res<ZedSettings>,
    mut decals: MessageWriter<SpawnDecal>,
    mut zeds: Query<(Entity, &mut Zed, &Transform, Option<&mut RagdollState>)>,
    bodies: Query<(&Transform, &LinearVelocity, Has<Sleeping>, &AngularVelocity), With<RagdollBody>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut log_timer: Local<f32>,
) {
    let Some(classes) = classes else {
        return;
    };
    let dt = time.delta_secs();
    *log_timer += dt;
    let log_now = *log_timer >= 1.0;
    if log_now {
        *log_timer = 0.0;
    }
    for (entity, mut z, t, ragdoll_state) in &mut zeds {
        let c = &classes.0[z.class];
        if let Some(gore) = gore.as_deref() {
            let clock = (time.elapsed_secs_f64() * 1000.0) as u32 ^ std::process::id();
            apply_gore(&mut commands, &mut meshes, gore, library.as_deref(), c, &mut z, entity, t, clock, settings.always_sever, &mut decals);
        }
        // Hidden bones: the head once decapitated, and severed limbs.
        let mut collapse = z.hidden_bones.clone();
        if z.decapitated
            && let Some(h) = c.head_bone
        {
            collapse.push(h);
        }
        // Ragdoll: bones come from the physics bodies.
        if let (Some(mut r), Some(def)) = (ragdoll_state, c.ragdoll.as_ref()) {
            r.age += dt;
            let Some(pose) = r.pose(def, |e| bodies.get(e).ok().map(|(bt, _, _, _)| *bt)) else {
                continue; // bodies not spawned yet (first frame)
            };
            let first = r.ball_joints.iter().all(|w| w.max_swing == 0.0 && w.max_twist == 0.0);
            r.watch_limits(|e| bodies.get(e).ok().map(|(bt, _, _, _)| *bt));
            if first {
                runlog::kv("ragdoll_limits_at_death", &format!("id={} deg {}", z.id, r.limit_report(def)));
            }
            if r.age >= 5.0 && r.age - dt < 5.0 {
                runlog::kv("ragdoll_limits", &format!("id={} largest_seen_deg {}", z.id, r.limit_report(def)));
            }
            let skinned = c.model.skin(&pose, &collapse);
            let to_actor = mesh_to_actor(c);
            c.model.upload_to(&z.meshes, &skinned, |p| coords::pos(to_actor(p).to_array()), &mut meshes);
            if let Some(gore) = gore.as_deref() {
                for (kind, tag, scale, handles) in &z.stumps {
                    gore::place_stump(gore, *kind, &c.model, &pose, &to_actor, tag, *scale, handles, &mut meshes);
                }
            }
            z.last_pose = pose.clone();
            follow_tags(c, &z, t, &mut effects);
            let (fastest, max_speed) = r
                .bodies
                .iter()
                .enumerate()
                .filter_map(|(i, &e)| bodies.get(e).ok().map(|(_, v, _, _)| (i, v.0.length() / SCALE)))
                .fold((0, 0.0), |a, b| if b.1 > a.1 { b } else { a });
            let root = bodies.get(r.bodies[def.root]).ok().map(|(bt, _, _, _)| bt.translation / SCALE);
            let (spinning, max_spin) = r
                .bodies
                .iter()
                .enumerate()
                .filter_map(|(i, &e)| bodies.get(e).ok().map(|(_, _, _, w)| (i, w.0.length())))
                .fold((0, 0.0), |a, b| if b.1 > a.1 { b } else { a });
            let sleeping = r.bodies.iter().filter(|&&e| bodies.get(e).is_ok_and(|(_, _, s, _)| s)).count();
            if (r.age < 0.3 || log_now)
                && let Some((name, gap)) = r.worst_joint_gap(def, |e| bodies.get(e).ok().map(|(bt, _, _, _)| *bt))
            {
                runlog::kv("ragdoll_joint_gap", &format!("id={} age={:.2} worst_joint={name} gap_unreal={gap:.1}", z.id, r.age));
            }
            if r.age >= 2.0 && r.age - dt < 2.0 {
                // One-off snapshot: every part's offset from the root part.
                let parts: Vec<String> = r
                    .bodies
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &e)| {
                        let (bt, v, _, _) = bodies.get(e).ok()?;
                        let rel = (bt.translation - root? * SCALE) / SCALE;
                        Some(format!("{}:dist={:.0},up={:.0},speed={:.0}", def.part_name(i), rel.length(), rel.y, v.0.length() / SCALE))
                    })
                    .collect();
                runlog::kv("ragdoll_parts", &format!("id={} {}", z.id, parts.join(" ")));
                let gaps = r.joint_gaps(def, |e| bodies.get(e).ok().map(|(bt, _, _, _)| *bt));
                runlog::kv("ragdoll_joint_gaps", &format!("id={} {}", z.id, gaps.join(" ")));
            }
            if r.rested_at.is_none() && r.age > 0.5 && max_speed < 5.0 {
                r.rested_at = Some(r.age);
                runlog::kv("ragdoll_rested", &format!("id={} seconds={:.2}", z.id, r.age));
            }
            if log_now && let Some(root) = root {
                runlog::kv(
                    "ragdoll",
                    &format!(
                        "id={} age={:.1} root_unreal=({:.0}, {:.0}, {:.0}) root_height_above_death_spot_unreal={:.0} max_body_speed_unreal={max_speed:.0} fastest={} max_spin_rad_s={max_spin:.2} spinning={} twist_now_deg=[{}] sleeping_bodies={sleeping}",
                        z.id,
                        r.age,
                        -root.z,
                        root.x,
                        root.y,
                        root.y - (z.centre.y / SCALE - c.collision_height),
                        def.part_name(fastest),
                        def.part_name(spinning),
                        r.twist_now(def),
                    ),
                );
            }
            continue;
        }
        if let Some(s) = z.sequence {
            let len = c.model.length(s).max(1e-3);
            z.frame += time.delta_secs() * c.model.rate(s);
            if z.looping {
                z.frame %= len;
            } else {
                z.frame = z.frame.min(len);
            }
            if z.health <= 0.0 && Some(s) == c.death {
                z.frame = z.frame.min(c.death_hold_frame);
            }
        }
        // Start a pending upper-body hit reaction (KnockDown is full body
        // and handled by the think system).
        if let Some(r) = z.pending_reaction
            && r != HitReaction::KnockDown
        {
            z.pending_reaction = None;
            let seq = match r {
                HitReaction::Front => c.hit_reactions[0],
                HitReaction::Back => c.hit_reactions[1],
                HitReaction::Left => c.hit_reactions[2],
                HitReaction::Right => c.hit_reactions[3],
                HitReaction::Stun if !c.hit_anims.is_empty() => {
                    let i = z.random() as usize % c.hit_anims.len();
                    Some(c.hit_anims[i])
                }
                _ => None,
            };
            if let (Some(seq), Some(root)) = (seq, c.spine_bone) {
                z.overlay = Some((seq, 0.0, root));
                runlog::kv(
                    "zed_hit_reaction",
                    &format!("id={} reaction={r:?} sequence={} stunned={:.1}", z.id, c.model.sequence_name(seq).unwrap_or("?"), z.stunned),
                );
            }
        }
        // Upper-body layer, played once.
        let overlay = z.overlay;
        if let Some((seq, f, root)) = z.overlay {
            let next = f + dt * c.model.rate(seq);
            z.overlay = (next < c.model.length(seq)).then_some((seq, next, root));
        }
        // Decapitated: the head (and anything under it) shrinks into the neck.
        let (skinned, bones) = c.model.pose_layered(z.sequence, z.frame, overlay, &collapse);
        let to_actor = mesh_to_actor(c);
        c.model.upload_to(&z.meshes, &skinned, |p| coords::pos(to_actor(p).to_array()), &mut meshes);
        if let Some(gore) = gore.as_deref() {
            for (kind, tag, scale, handles) in &z.stumps {
                gore::place_stump(gore, *kind, &c.model, &bones, &to_actor, tag, *scale, handles, &mut meshes);
            }
        }
        z.last_pose = bones.clone();
        follow_tags(c, &z, t, &mut effects);
        // Extended collision: centre + (ColOffset >> Rotation), hard-attached
        // to the actor (KFMonster.PostBeginPlay).
        z.ext = c.ext_collision.map(|(offset, r, h)| {
            (t.translation + t.rotation * coords::pos(offset.to_array()), r, h)
        });
        // Head sphere for headshots: head bone origin plus HeadHeight x
        // HeadScale along the bone's X axis (KFMonster.IsHeadShot).
        z.head = c.head_bone.map(|b| {
            let (q, p) = bones[b];
            let origin = to_actor(p);
            let axis = (to_actor(p + q * Vec3::X) - origin).normalize_or_zero();
            let centre = origin + axis * c.head_offset;
            let world = t.rotation * coords::pos(centre.to_array()) + t.translation;
            let world_axis = t.rotation * coords::dir(axis.to_array());
            (world, world_axis)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ZombieGoreFast.RangedAttack / RunningState: runs within 700 when not
    /// attacking, stops when the target is 700+ away at a CheckCharge, never
    /// runs headless.
    #[test]
    fn gorefast_runs_by_kf_rules() {
        let mut z = Zed::test_gorefast();
        assert_eq!(z.update_running(800.0, false, 0.1), None, "too far to start");
        assert_eq!(z.update_running(600.0, true, 0.1), None, "attacking: no start");
        assert_eq!(z.update_running(600.0, false, 0.1), Some("started"));
        // The first CheckCharge comes at once and sleeps 0.5-1.0 s.
        assert_eq!(z.update_running(600.0, false, 0.1), None);
        assert!((0.5..=1.0).contains(&z.charge_check), "sleep {}", z.charge_check);
        // Target moves away: noticed at the next CheckCharge, not before.
        assert_eq!(z.update_running(900.0, false, 0.2), None);
        assert_eq!(z.update_running(900.0, false, 1.0), Some("target_far"));
        assert!(!z.running);
        // Clots never run.
        let mut clot = Zed::test_clot();
        assert_eq!(clot.update_running(100.0, false, 0.1), None);
    }

    #[test]
    fn gorefast_moving_attack_and_head_loss_end_the_run() {
        let mut z = Zed::test_gorefast();
        z.update_running(300.0, false, 0.1);
        z.run_attack_timeout = 0.3;
        assert_eq!(z.update_running(50.0, true, 0.2), None);
        assert_eq!(z.update_running(50.0, true, 0.2), Some("run_attack_done"));
        assert!(!z.running);
        z.update_running(300.0, false, 0.1);
        assert!(z.running);
        z.remove_head();
        assert!(!z.running);
        assert_eq!(z.update_running(300.0, false, 0.1), None, "headless: no running");
    }
}
