//! Zeds (KF monsters): every base specimen and the Patriarch, hunting the
//! player over the map's navigation points, attacking, taking damage, gore
//! and ragdolls (see DESIGN.md: Zeds, Combat, All specimens, Patriarch).
//!
//! Mesh-to-world (UE2): point - MeshOrigin, scaled by MeshScale, rotated by
//! the mesh's RotOrigin, scaled by DrawScale, plus PrePivot; then the actor's
//! rotation and location (the centre of its collision cylinder).
//!
//! Keys: Z spawns a zed in front of you (N picks the type),
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
#[derive(Resource, Default, Clone)]
pub struct ZedSettings {
    pub spawn_at_start: bool,
    pub gorefast_at_start: bool,
    /// Test switch (`--always-sever`): a killing hit on a limb always takes
    /// it off, instead of KF's chance.
    pub always_sever: bool,
    /// Test switch (`--zed-at X,Y,Z`): the start zed appears there (Unreal
    /// units, the cylinder centre) instead of in front of you.
    pub spawn_at: Option<[f32; 3]>,
    /// `--spawn NAME`: a zed of that kind at the start (e.g. "crawler").
    pub spawn_kind: Option<String>,
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
    Crawler,
    Stalker,
    Bloat,
    Siren,
    Husk,
    Scrake,
    Fleshpound,
    Patriarch,
}

/// The classes we load: kind and class path (KF's ten specimens).
const ZED_CLASSES: [(ZedKind, &str); 10] = [
    (ZedKind::Clot, "KFChar.ZombieClot_STANDARD"),
    (ZedKind::Gorefast, "KFChar.ZombieGorefast_STANDARD"),
    (ZedKind::Crawler, "KFChar.ZombieCrawler_STANDARD"),
    (ZedKind::Stalker, "KFChar.ZombieStalker_STANDARD"),
    (ZedKind::Bloat, "KFChar.ZombieBloat_STANDARD"),
    (ZedKind::Siren, "KFChar.ZombieSiren_STANDARD"),
    (ZedKind::Husk, "KFChar.ZombieHusk_STANDARD"),
    (ZedKind::Scrake, "KFChar.ZombieScrake_STANDARD"),
    (ZedKind::Fleshpound, "KFChar.ZombieFleshPound_STANDARD"),
    (ZedKind::Patriarch, "KFChar.ZombieBoss_STANDARD"),
];

/// A kind by its name, ignoring case ("crawler", "Patriarch", ...).
fn kind_named(name: &str) -> Option<ZedKind> {
    ZED_CLASSES.iter().map(|(k, _)| *k).find(|k| format!("{k:?}").eq_ignore_ascii_case(name))
}

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
    /// HiddenGroundSpeed: an unseen zed's speed (KFMonster 300).
    hidden_speed: f32,
    /// Turn speed, Unreal rotation units per second (RotationRate.Yaw).
    turn_rate: f32,
    melee_range: f32,
    idle: Option<usize>,
    walk: Option<usize>,
    melee: Vec<usize>,
    /// The DoorBash animation (KFMonster.DoorAttack), if the mesh has one.
    door_bash: Option<usize>,
    /// bCanDistanceAttackDoors (Bloat, Husk): attack a welded door seen on
    /// the way from range (lost with the head).
    distance_door_attack: bool,
    /// Pawn.Intelligence (BRAINS_None 0 .. BRAINS_Human 3): from Mammal
    /// (2) up, a door-bashing zed leaves the door for a reachable enemy.
    intelligence: u8,
    /// Death animation and the frame where the body is
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
    /// JumpZ (KFMonster 320).
    jump_z: f32,
    /// Crawler: PounceSpeed (0 = cannot pounce).
    pounce_speed: f32,
    /// FlipOver returns false (ZombieCrawler): no knock-down.
    no_flip: bool,
    /// Bone the hit flinches play from, if not SpineBone1 (ZombieCrawler
    /// plays HitF from NeckBone).
    flinch_root: Option<usize>,
    /// Stalker: the cloaked look (KF: Shader stalker_invisible, a refraction
    /// effect; here a faint see-through skin, an approximation).
    cloak_material: Option<Handle<StandardMaterial>>,
    /// Patriarch: one cloaked look per part (KF: patriarch_invisible_gun and
    /// patriarch_invisible, refraction shaders we cannot draw; approximated
    /// like the Stalker's, from each part's own texture).
    cloak_parts: Vec<Handle<StandardMaterial>>,
    /// Scrake: SawImpaleLoop (the attack repeated in state SawingLoop) and
    /// ChargeF (the walk while charging or raging).
    saw_impale: Option<usize>,
    charge_anim: Option<usize>,
    /// Smallest hit that plays a flinch (KFMonster 5; ZombieScrake 150;
    /// ZombieFleshPound 10).
    flinch_min_damage: f32,
    /// Fleshpound: PoundRage, the charge walk (ChargingAnim PoundRun), the
    /// rage attack (FPRageAttack), RageDamageThreshold and the red device
    /// material (DeviceGoRed: KFCharacters.FPRedBloomShader on Skins[1]).
    fp_rage_anim: Option<usize>,
    fp_charge_walk: Option<usize>,
    fp_rage_attack: Option<usize>,
    fp_rage_threshold: f32,
    fp_red_device: Option<Handle<StandardMaterial>>,
    /// Share of non-explosive damage taken (ZombieFleshPound 0.5).
    small_arms_scale: f32,
    /// ZombieHusk BurnDamageScale (Normal difficulty), else 1.
    fire_resist: f32,
    zap: ZapValues,
    /// BurningWalkFAnims[0] (all three are the same).
    burning_walk: Option<usize>,
    /// MotionDetectorThreat: how much this zed counts toward setting off a pipe bomb.
    motion_threat: f32,
    /// The ranged attack (RangedAttack beyond melee reach, head on): the
    /// Bloat's ZombieBarf within 250, the Siren's Siren_Scream within
    /// ScreamRadius. Its SpawnTwoShots notify times (0..1: the vomit, or
    /// each scream damage pulse), its AnimNotify_Effects (KFVomitJet,
    /// SirenScream), and the chance it plays on the upper body while
    /// walking (Bloat 0.4; the Siren always).
    ranged_anim: Option<usize>,
    ranged_distance: f32,
    ranged_shots: Vec<f32>,
    ranged_effects: Vec<(f32, ue_assets::skeletal::NotifyEffect)>,
    ranged_moving_chance: f32,
    /// Husk: ProjectileFireInterval (5.5, Normal), the wait after a shot
    /// before the next (plus FRand() x 2), and the bone shots start at.
    ranged_interval: f32,
    barrel_bone: Option<usize>,
    /// Bloat: the bone hidden when he bursts on death (SpineBone2).
    burst_bone: Option<usize>,
    /// Siren: ScreamDamage, ScreamRadius, ScreamForce.
    scream: Option<(f32, f32, f32)>,
    /// Patriarch: his own attacks (`boss.rs`).
    boss: Option<crate::boss::BossClass>,
}

#[derive(Resource)]
struct ZedClasses(Vec<ZedClass>);

/// A zed's mesh-part entities (one per material), for material swaps.
#[derive(Component)]
struct ZedParts(Vec<Entity>);

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
    /// Fleshpound: playing PoundRage (state BeginRaging), not moving.
    Enraging,
    /// Patriarch: a full-body action run by `boss.rs` (the chaingun).
    BossBusy,
    /// KFMonsterController state DoorBashing: standing at a welded door,
    /// hitting it (`door_bashing`).
    DoorBashing,
    Dead,
}

/// KFMonsterController.DoorBashing's loop for one zed.
#[derive(Clone, Copy, Debug)]
struct DoorBash {
    /// TargetDoor (index into `door::Doors`).
    door: usize,
    /// Seconds until the loop runs again (its Sleeps).
    wait: f32,
    /// A DoorBash animation is playing (bShotAnim).
    in_anim: bool,
    /// Which of its ClawDamageTarget notifies have fired (bits).
    hits: u8,
    /// Seconds since the animation started.
    anim_time: f32,
    /// bDistanceAttackingDoor: BreakUpDoor from the path check (Bloat,
    /// Husk), so DoorAttack uses the ranged attack.
    distance: bool,
    /// The animation playing is the ranged attack (hits on SpawnTwoShots).
    ranged: bool,
}

/// An attack in progress.
#[derive(Clone, Copy, Debug)]
struct Attack {
    seq: usize,
    /// On the upper-body layer (the zed keeps walking) or the whole body.
    layered: bool,
    /// The damage check (or, for a barf, the shots) is done.
    hit_done: bool,
    /// The ranged attack (vomit, scream) instead of a melee hit.
    ranged: bool,
    /// Which of the animation's effect and shot notifies have fired (bits).
    fx_fired: u8,
    shots_fired: u8,
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
/// ZombieScrake: RunningState GroundSpeed x 3.5; AttackChargeRate 2.5.
const SCRAKE_RAGE_SPEED: f32 = 3.5;
const SCRAKE_ATTACK_CHARGE_RATE: f32 = 2.5;
/// ZombieBloat.RangedAttack: vomit within 250 units; a moving vomit with
/// ChargeChance 0.4 (Normal difficulty).
const BLOAT_BARF_DISTANCE: f32 = 250.0;
const BLOAT_CHARGE_CHANCE: f32 = 0.4;
/// ZombieFleshPound RageCharging: GroundSpeed x 2.3.
const FLESHPOUND_RAGE_SPEED: f32 = 2.3;

/// KFMonster.PostBeginPlay: MeleeDamage and ScreamDamage (ints) =
/// Max(DifficultyDamageModifer x damage, 1); Normal 1.0, x 0.75 with one
/// player.
fn solo_damage(d: f32) -> f32 {
    (d * 0.75).trunc().max(1.0)
}

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
    /// Particle effects attached to the zed (AttachEmitterEffect on a mesh
    /// tag, or an AnimNotify_Effect on a bone).
    effects: Vec<(Entity, EffectAnchor)>,
    /// Seconds since the last hit (TakeDamage's bRecentHit: under 0.2 s).
    since_hit: f32,
    /// Hunting route (KFMonsterController ZombieHunt).
    router: crate::nav::Router,
    /// Horizontal velocity kept while falling or jumping (Bevy, m/s).
    air_velocity: Vec3,
    /// Seconds before the zed may try another jump.
    jump_cooldown: f32,
    /// Crawler: in a pounce (bPouncing), and seconds since the last one.
    pouncing: bool,
    since_pounce: f32,
    /// FlipOver allowed (false for the Crawler).
    can_flip: bool,
    /// Scrake: in state SawingLoop, charging while sawing (speed x
    /// AttackChargeRate), raging (RunningState, speed x 3.5); the smallest
    /// flinching hit.
    sawing: bool,
    saw_charging: bool,
    raging: bool,
    flinch_min_damage: f32,
    /// Fleshpound: damage in the current 2 s window (TwoSecondDamageTotal),
    /// seconds since the last damage, a rage to start, rage seconds left
    /// (RageCharging), chasing without reaching (RageFrustrationTimer) and
    /// its threshold, frustrated rage (no time-out).
    fp_two_sec_damage: f32,
    fp_since_damaged: f32,
    fp_start_rage: bool,
    fp_rage: Option<f32>,
    fp_frustration: f32,
    fp_frustration_limit: f32,
    fp_frustrated: bool,
    /// RageDamageThreshold (0 = never rages).
    fp_rage_threshold: f32,
    /// Share of non-explosive damage taken.
    pub small_arms_scale: f32,
    /// Burning (KFMonster): BurnDown ticks left, LastBurnDamage, HeatAmount,
    /// FireDamageClass, seconds to the next tick, the flames effect.
    pub(crate) burn_down: u32,
    pub(crate) last_burn_damage: f32,
    pub(crate) heat: u32,
    pub(crate) fire_class: crate::combat::FireType,
    pub(crate) burn_timer: f32,
    burn_fx: Option<Entity>,
    /// ZombieBloat: DamTypeBurned x 1.5; ZombieHusk: BurnDamageScale for
    /// DamTypeBurned / DamTypeFlamethrower (1 for others).
    pub(crate) burned_scale: f32,
    pub(crate) fire_resist: f32,
    /// ZED gun zap (KFMonster): TotalZap, RemainingZap (> 0 = bZapped),
    /// seconds since LastZapTime, ZapThreshold (grows x ZapResistanceScale
    /// after each zap), and the class's values.
    pub(crate) total_zap: f32,
    pub(crate) remaining_zap: f32,
    since_zap: f32,
    pub(crate) zap_threshold: f32,
    pub(crate) zap: ZapValues,
    /// A run, rage or charge was going when a zap wore off: KF's
    /// UnSetZappedBehavior set the normal GroundSpeed and the state kept it.
    run_speed_lost: bool,
    /// MotionDetectorThreat (pipe bombs).
    pub(crate) motion_threat: f32,
    /// Patriarch: the head never comes off (ZombieBoss.RemoveHead is empty)
    /// and hits never flinch or stun him (PlayDirectionalHit is empty).
    pub keeps_head: bool,
    no_hit_reactions: bool,
    /// Patriarch: his own state (charge, ...).
    boss: Option<crate::boss::BossState>,
    /// Patriarch: the chaingun's MuzzleFlash3rdMG (mMuzzleFlash) on `tip`,
    /// and AddTraceHitFX calls not yet shown on it.
    mg_flash: Option<Entity>,
    mg_flash_shots: u32,
    /// Bloat: hits do not interrupt his attacks (HitCanInterruptAction);
    /// died by bleeding out (no burst); the death burst is done; notify
    /// effects to start (animate_zeds has the effect library).
    uninterruptible: bool,
    /// Siren: headless she dies at once or within 10 s (ZombieSiren.RemoveHead).
    quick_headless_death: bool,
    /// Husk: seconds until he may shoot again (NextFireProjectileTime).
    ranged_wait: f32,
    bled_out: bool,
    burst_done: bool,
    pending_fx: Vec<ue_assets::skeletal::NotifyEffect>,
    /// Stalker cloak: cloaked now, seconds since it last uncloaked, seconds
    /// to the next 0.5 s check, and whether the look must be updated.
    cloaked: bool,
    since_uncloak: f32,
    cloak_check: f32,
    cloak_dirty: bool,

    /// Facing, Unreal rotation units.
    pub(crate) yaw: f32,
    state: ZedState,
    vertical_speed: f32,
    sequence: Option<usize>,
    frame: f32,
    looping: bool,
    meshes: Vec<Handle<Mesh>>,
    /// At a welded door (state DoorBashing).
    door_bash: Option<DoorBash>,
    /// The path point last checked for a welded door in the way (Bloat,
    /// Husk), Bevy space.
    door_checked: Option<Vec3>,
    /// The JumpPad being touched (Touch fires on entering).
    on_pad: Option<usize>,
    /// LastSeenOrRelevantTime, LastRenderTime and LastViewCheckTime (game
    /// seconds), and whether it moves at HiddenGroundSpeed.
    last_seen: f32,
    last_render: f32,
    last_view_check: f32,
    hidden: bool,
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
        z.effects.push((e, EffectAnchor::Tag(tag)));
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

/// Where an attached effect sits: a mesh tag, or a bone with an offset and
/// rotation relative to it (AnimNotify_Effect's OffsetLocation and
/// OffsetRotation, Unreal units and axes).
#[derive(Clone, Copy, Debug)]
pub enum EffectAnchor {
    Tag(&'static str),
    Bone { bone: usize, offset: Vec3, rotation: Mat3 },
}

fn anchor_frame(c: &ZedClass, z: &Zed, t: &Transform, anchor: &EffectAnchor) -> Option<(Vec3, Mat3)> {
    match anchor {
        EffectAnchor::Tag(tag) => c.model.tag_frame(&z.last_pose, tag).map(|f| world_axes(c, t, f)),
        EffectAnchor::Bone { bone, offset, rotation } => {
            let (pos, axes) = world_axes(c, t, c.model.bone_frame(&z.last_pose, *bone)?);
            // The bone frame from world_axes is in actor scale; the offset is
            // in the bone's axes (Unreal units, DrawScale applied).
            Some((pos + axes * (*offset * c.draw_scale), axes * *rotation))
        }
    }
}

/// AnimNotify_Effect: starts the notify's effect at its bone; attached
/// effects follow the bone from then on.
#[allow(clippy::too_many_arguments)]
fn notify_effect(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    library: Option<&EffectLibrary>,
    c: &ZedClass,
    z: &mut Zed,
    t: &Transform,
    effect: &ue_assets::skeletal::NotifyEffect,
    seed: u32,
) {
    let Some(library) = library else {
        return;
    };
    let Some(bone) = c.model.find_bone(&effect.bone) else {
        runlog::kv("notify_effect_error", &format!("id={} class={} bone={} error=\"no bone\"", z.id, effect.class, effect.bone));
        return;
    };
    let [pitch, yaw, roll] = effect.rotation;
    let anchor = EffectAnchor::Bone {
        bone,
        offset: Vec3::from_array(effect.offset),
        rotation: coords::ue_rotation_matrix(Rotator { pitch, yaw, roll }),
    };
    let Some((pos, axes)) = anchor_frame(c, z, t, &anchor) else {
        return;
    };
    let spawned = particles::spawn_effect(commands, library, meshes, &effect.class, pos, axes, seed);
    runlog::kv(
        "notify_effect",
        &format!(
            "id={} class={} bone={} at_unreal=({:.0}, {:.0}, {:.0}) spawned={}",
            z.id,
            effect.class,
            effect.bone,
            pos.x,
            pos.y,
            pos.z,
            spawned.is_some()
        ),
    );
    if let Some(e) = spawned
        && effect.attach
    {
        z.effects.push((e, anchor));
    }
}

/// ZombieBloat.SpawnTwoShots: three KFBloatVomit globs from 30 ahead and 64
/// up (x DrawScale), aimed at the target (AdjustAim; KF also leads a moving
/// target: not done), the side ones half a CollisionRadius out and turned
/// 1200 yaw units (6.6 degrees).
fn spawn_two_shots(z: &Zed, c: &ZedClass, target: Vec3, out: &mut MessageWriter<crate::vomit::SpawnVomit>) {
    let k = std::f32::consts::TAU / 65536.0;
    let a = z.yaw * k;
    let (x, y) = (Vec3::new(a.cos(), a.sin(), 0.0), Vec3::new(-a.sin(), a.cos(), 0.0));
    let start = ue_pos(z.centre) + (x * 30.0 + Vec3::Z * 64.0) * c.draw_scale;
    let to = ue_pos(target) - start;
    let (yaw, pitch) = (to.y.atan2(to.x), to.z.atan2(to.truncate().length()));
    for (side, turn) in [(0.0, 0.0), (-0.5, -1200.0), (0.5, 1200.0)] {
        let yw = yaw + turn * k;
        let dir = Vec3::new(pitch.cos() * yw.cos(), pitch.cos() * yw.sin(), pitch.sin());
        out.write(crate::vomit::SpawnVomit {
            at: start + y * side * c.collision_radius,
            velocity: dir * crate::vomit::SPEED,
            zed_id: z.id,
        });
    }
    runlog::kv(
        "bloat_shots",
        &format!("id={} start_unreal=({:.0}, {:.0}, {:.0}) pitch_deg={:.1}", z.id, start.x, start.y, start.z, pitch.to_degrees()),
    );
}

/// One ZombieSiren.SpawnTwoShots: HurtRadius(ScreamDamage, ScreamRadius,
/// ScreamForce) on the player, if within the radius and in sight:
/// damageScale = 1 - (distance - 20) / ScreamRadius, damage x scale (whole
/// points), momentum damageScale x ScreamForce along the line from her to
/// the player (negative: a pull toward her). Screen shake and blur
/// (DoShakeEffect) not done.
#[allow(clippy::too_many_arguments)]
fn scream_pulse(
    z: &Zed,
    damage: f32,
    radius: f32,
    force: f32,
    target: Vec3,
    spatial: &SpatialQuery,
    out: &mut MessageWriter<crate::combat::PlayerDamaged>,
    push: &mut MessageWriter<crate::walk::PlayerPush>,
) {
    let (from, to) = (ue_pos(z.centre), ue_pos(target));
    let dist = (to - from).length().max(1.0);
    if dist - PLAYER_RADIUS > radius {
        return;
    }
    if let Ok(d) = Dir3::new(target - z.centre)
        && spatial
            .cast_ray(z.centre, d, (target - z.centre).length(), true, &crate::collision::world_filter())
            .is_some()
    {
        runlog::kv("siren_scream", &format!("id={} distance_unreal={dist:.0} blocked=true", z.id));
        return;
    }
    let scale = 1.0 - ((dist - PLAYER_RADIUS) / radius).max(0.0);
    let amount = (scale * damage).floor();
    if amount > 0.0 {
        out.write(crate::combat::PlayerDamaged {
            amount,
            zed_id: z.id,
            kind: crate::combat::HurtKind::Plain,
        });
    }
    let momentum = (to - from) / dist * (scale * force);
    push.write(crate::walk::PlayerPush { momentum });
    runlog::kv(
        "siren_scream",
        &format!(
            "id={} distance_unreal={dist:.0} scale={scale:.2} damage={amount} momentum_unreal=({:.0}, {:.0}, {:.0})",
            z.id, momentum.x, momentum.y, momentum.z
        ),
    );
}

/// A clear line between two points (Bevy space) through the level.
/// The Patriarch in state FireChaingun, one frame: turn to the player, run
/// the bursts (`boss::Chaingun`), play its animations and fire its shots.
/// Shot from closer than 100: charge instead (FireChaingun.TakeDamage).
#[allow(clippy::too_many_arguments)]
fn boss_busy(
    z: &mut Zed,
    c: &ZedClass,
    t: &Transform,
    target: Vec3,
    player_velocity: Vec3,
    dt: f32,
    spatial: &SpatialQuery,
    player_damage: &mut MessageWriter<crate::combat::PlayerDamaged>,
    push: &mut MessageWriter<crate::walk::PlayerPush>,
    fireball: &mut MessageWriter<crate::fireball::SpawnFireball>,
    bullet_fx: &mut MessageWriter<crate::bullet_fx::BulletFx>,
) {
    let (Some(bc), Some(mut b)) = (c.boss.as_ref(), z.boss) else {
        z.state = ZedState::Chase;
        return;
    };
    // State KnockDown: when the animation is done, cloak and escape.
    if b.knockdown.is_some() {
        b.tick(dt);
        if b.knockdown_step(dt) {
            z.state = ZedState::Chase;
            z.sequence = None;
            // CloakBoss: not while zapped.
            z.cloaked = !z.zapped();
            z.cloak_dirty = true;
            z.router = Default::default();
            runlog::kv("boss_escape", &format!("id={} start health={:.0}", z.id, z.health));
        }
        b.hit_from_close = false;
        z.boss = Some(b);
        return;
    }
    // State Healing: the Heal animation and its syringe notifies.
    if b.heal.is_some()
        && let Some((_, secs)) = bc.heal_anim
    {
        b.tick(dt);
        let (syringe, added, done) = b.heal_step(dt, secs);
        if syringe {
            if let Some(bone) = bc.syringe_bones.get(b.syringes - 1).copied().flatten() {
                z.hidden_bones.push(bone);
            }
            runlog::kv("boss_heal", &format!("id={} syringe={}", z.id, b.syringes));
        }
        if let Some(amount) = added {
            let before = z.health;
            z.health += amount;
            runlog::kv("boss_heal", &format!("id={} health={before:.0}->{:.0}", z.id, z.health));
        }
        if done {
            z.state = ZedState::Chase;
            z.sequence = None;
            runlog::kv("boss_heal", &format!("id={} done health={:.0} syringes={}", z.id, z.health, b.syringes));
        }
        b.hit_from_close = false;
        z.boss = Some(b);
        return;
    }
    // State FireMissile: turn to the player, fire when PreFireMissile ends.
    if let (Some(mut m), Some(anims)) = (b.missile, bc.missile_anims) {
        b.tick(dt);
        turn_toward(z, c, target, dt);
        match m.step(dt, anims[1].1) {
            Some(crate::boss::MissileEvent::Fire) => {
                shoot_fireball(z, c, t, crate::fireball::Projectile::BossRocket, bc.tip_bone, target, player_velocity, spatial, fireball);
                z.sequence = None;
                start_anim(z, Some(anims[1].0), false);
                b.missile = Some(m);
            }
            Some(crate::boss::MissileEvent::Done) => {
                b.missile = None;
                // Back to the DoorBashing loop if the rocket was at a door.
                z.state = if z.door_bash.is_some() { ZedState::DoorBashing } else { ZedState::Chase };
                z.sequence = None;
                runlog::kv("boss_missile", &format!("id={} done next_in={:.1}", z.id, b.missile_wait));
            }
            None => b.missile = Some(m),
        }
        b.hit_from_close = false;
        z.boss = Some(b);
        return;
    }
    let Some((mut mg, anims)) = b.chaingun.zip(bc.mg_anims) else {
        z.state = ZedState::Chase;
        return;
    };
    b.tick(dt);
    // FireChaingun.TakeDamage: only "shot from closer than 100" works. Its
    // "ChargeDamage > 200 within 500" half never fires (ChargeDamage is
    // always 0, see boss::BossState::decide), and its "a closer attacker
    // than the enemy" half needs a second player.
    if std::mem::take(&mut b.hit_from_close) {
        let roll = (z.random() % 1000) as f32 / 1000.0;
        b.end_chaingun(roll);
        let attacks = if z.random().is_multiple_of(2) { 1 } else { 2 };
        b.charge = Some(crate::boss::Charge { seconds: 0.0, attacks_left: attacks });
        z.state = ZedState::Chase;
        z.sequence = None;
        if let (Some(seq), Some(root)) = (bc.transition, c.fire_root_bone) {
            z.overlay = Some((seq, 0.0, root));
        }
        runlog::kv("boss_chaingun", &format!("id={} end reason=shot_from_close shots_left={}", z.id, mg.shots_left));
        runlog::kv("boss_charge", &format!("id={} start attacks={attacks} reason=shot_from_close", z.id));
        z.boss = Some(b);
        return;
    }
    // The controller turns him toward Focus (the player) or FocalPoint.
    let aim_at = if mg.has_focus() { target } else { b.mg_focal };
    let want_yaw = turn_toward(z, c, aim_at, dt);
    let tip = bc
        .tip_bone
        .and_then(|bone| c.model.bone_frame(&z.last_pose, bone))
        .map_or(ue_pos(z.centre), |f| world_axes(c, t, f).0);
    let tip_bevy = coords::pos(tip.to_array());
    // AnimEnd: LineOfSightTo and FastTrace from the tip to the enemy.
    let in_sight = sees(spatial, z.centre, target) && sees(spatial, tip_bevy, target);
    let seconds = anims.map(|a| a.1);
    let events = {
        let mut roll = || (z.random() % 10000) as f32 / 10000.0;
        mg.step(dt, in_sight, seconds, &mut roll)
    };
    if mg.has_focus() {
        b.mg_focal = target;
    }
    let mut done = false;
    for e in events {
        match e {
            crate::boss::MgEvent::Play(a) => {
                let i = match a {
                    crate::boss::MgAnim::Fire => 1,
                    crate::boss::MgAnim::End => 2,
                };
                z.sequence = None; // restart even if the same (FireMG again)
                start_anim(z, Some(anims[i].0), false);
                if a == crate::boss::MgAnim::End {
                    // FireChaingun.EndState: LastChainGunTime = now + 5 + FRand() x 10.
                    b.chaingun_wait = 5.0 + 10.0 * (z.random() % 1000) as f32 / 1000.0;
                    runlog::kv(
                        "boss_chaingun",
                        &format!("id={} end shots_left={} seconds={:.2} next_in={:.1}", z.id, mg.shots_left, mg.clock, b.chaingun_wait),
                    );
                }
            }
            crate::boss::MgEvent::Shoot => {
                boss_mg_shot(z, tip, aim_at, want_yaw, target, spatial, player_damage, push, bullet_fx, mg.shots_left);
            }
            crate::boss::MgEvent::Done => done = true,
        }
    }
    if done {
        b.chaingun = None;
        z.state = ZedState::Chase;
        z.sequence = None;
        runlog::kv("boss_chaingun", &format!("id={} done", z.id));
    } else {
        b.chaingun = Some(mg);
    }
    z.boss = Some(b);
}

/// The Patriarch in state Escaping, once per think: Escaping's Begin loop
/// (cloak again), SyrRetreat.FindHideSpot once, and BeginHealing on
/// arriving (or after ESCAPE_GIVE_UP, our addition). Returns where to run
/// (Bevy), or None when not escaping.
fn boss_escape(z: &mut Zed, c: &ZedClass, player: Vec3, dt: f32, nav: &crate::nav::NavNetwork, spatial: &SpatialQuery) -> Option<Vec3> {
    let (Some(bc), Some(mut b)) = (c.boss.as_ref(), z.boss) else {
        return None;
    };
    b.escape?;
    if z.state != ZedState::Chase {
        return None;
    }
    if b.escape_step(dt, z.attack.is_some()) && !z.zapped() {
        z.cloaked = true;
        z.cloak_dirty = true;
        runlog::kv("boss_sneak", &format!("id={} cloak reason=escaping", z.id));
    }
    let mut e = b.escape.expect("checked");
    if e.goal.is_none() {
        // SyrRetreat.BeginState clears Enemy; SeePlayer sets it again. Seen:
        // FindHideSpot scores points; not: FindRandomDest.
        let enemy = sees(spatial, z.centre, player).then_some(player);
        let mut seed = z.random() | 1;
        e.goal = find_hide_spot(nav, spatial, z.centre, enemy, &mut seed);
        match e.goal {
            Some(g) => runlog::kv(
                "boss_escape",
                &format!(
                    "id={} hide_spot={} distance_unreal={:.0} player_known={} spot_seen_by_player={}",
                    z.id,
                    nav.points[g].name,
                    (nav.points[g].pos - z.centre).length() / SCALE,
                    enemy.is_some(),
                    sees(spatial, nav.points[g].pos, player)
                ),
            ),
            None => runlog::kv("boss_escape", &format!("id={} hide_spot=none", z.id)),
        }
    }
    let goal = e.goal.map(|g| nav.points[g].pos);
    let arrived = goal.is_none_or(|g| {
        let d = g - z.centre;
        d.with_y(0.0).length() / SCALE <= crate::nav::HUNT_RADIUS + 8.0 && (d.y / SCALE).abs() <= 2.0 * c.collision_height
    });
    let gave_up = e.seconds > crate::boss::ESCAPE_GIVE_UP;
    if (arrived || gave_up)
        && z.attack.is_none()
        && let Some((seq, _)) = bc.heal_anim
    {
        b.begin_healing();
        z.boss = Some(b);
        z.cloaked = false;
        z.cloak_dirty = true;
        z.overlay = None;
        z.state = ZedState::BossBusy;
        z.sequence = None;
        start_anim(z, Some(seq), false);
        let why = if gave_up && !arrived { "gave_up" } else { "arrived" };
        runlog::kv(
            "boss_heal",
            &format!("id={} start reason={why} escape_seconds={:.1} health={:.0} player_sees={}", z.id, e.seconds, z.health, sees(spatial, z.centre, player)),
        );
        return None;
    }
    b.escape = Some(e);
    z.boss = Some(b);
    goal
}

/// SyrRetreat.FindHideSpot: of the navigation points within 2500 that the
/// enemy cannot see and that have a path, the best by distance from the
/// enemy / max(distance from him / 800, 1.5), divided by 10 when the
/// direction from the point to the enemy is not within 0.2 (dot) of his own
/// direction to the enemy (a point past or beside the enemy). No enemy:
/// FindRandomDest, a random point with a path. "Has a path": reachable over
/// the network from a point near him that he can see (our stand-in for
/// FindPathToward).
fn find_hide_spot(nav: &crate::nav::NavNetwork, spatial: &SpatialQuery, pawn: Vec3, enemy: Option<Vec3>, seed: &mut u32) -> Option<usize> {
    let mut reachable = vec![false; nav.points.len()];
    let mut queue: Vec<usize> = nav
        .near(pawn, 800.0 * SCALE)
        .into_iter()
        .filter(|(i, _)| sees(spatial, pawn, nav.points[*i].pos))
        .map(|(i, _)| i)
        .collect();
    for &i in &queue {
        reachable[i] = true;
    }
    while let Some(u) = queue.pop() {
        for &(v, _) in &nav.links[u] {
            if !reachable[v] {
                reachable[v] = true;
                queue.push(v);
            }
        }
    }
    let Some(enemy) = enemy else {
        let all: Vec<usize> = (0..nav.points.len()).filter(|&i| reachable[i]).collect();
        if all.is_empty() {
            return None;
        }
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;
        return Some(all[*seed as usize % all.len()]);
    };
    let enemy_dir = (enemy - pawn).normalize_or_zero();
    let mut best: Option<(usize, f32)> = None;
    for (i, n) in nav.points.iter().enumerate() {
        let mdist = (n.pos - pawn).length() / SCALE;
        if mdist >= 2500.0 || !reachable[i] || sees(spatial, n.pos, enemy) {
            continue;
        }
        let mut score = (n.pos - enemy).length() / SCALE / (mdist / 800.0).max(1.5);
        if enemy_dir.dot((enemy - n.pos).normalize_or_zero()) < 0.2 {
            score /= 10.0;
        }
        if best.is_none_or(|(_, b)| b < score) {
            best = Some((i, score));
        }
    }
    best.map(|(i, _)| i)
}

/// Turns a zed toward `at` (Bevy) at its RotationRate; returns the wanted yaw.
fn turn_toward(z: &mut Zed, c: &ZedClass, at: Vec3, dt: f32) -> f32 {
    let to = (at - z.centre).with_y(0.0);
    let want_yaw = yaw_of(to);
    if to.length() / SCALE > 1.0 {
        let mut delta = (want_yaw - z.yaw).rem_euclid(65536.0);
        if delta > 32768.0 {
            delta -= 65536.0;
        }
        let step = c.turn_rate * dt;
        z.yaw = (z.yaw + delta.clamp(-step, step)).rem_euclid(65536.0);
    }
    want_yaw
}

/// FireChaingun.FireMGShot: from the tip at the aim point (or straight
/// ahead if he still has to turn more than 2000), VRand() x 0.06 spread, a
/// 10000-unit trace; the player takes MGDamage + Rand(3) in whole points
/// and momentum 500 along the shot. `tip` in Unreal units, the rest Bevy.
#[allow(clippy::too_many_arguments)]
fn boss_mg_shot(
    z: &mut Zed,
    tip: Vec3,
    aim_at: Vec3,
    want_yaw: f32,
    player: Vec3,
    spatial: &SpatialQuery,
    player_damage: &mut MessageWriter<crate::combat::PlayerDamaged>,
    push: &mut MessageWriter<crate::walk::PlayerPush>,
    bullet_fx: &mut MessageWriter<crate::bullet_fx::BulletFx>,
    shots_left: i32,
) {
    let yaw_err = (want_yaw - z.yaw).rem_euclid(65536.0);
    let turning = !(yaw_err < 2000.0 || yaw_err > 63535.0);
    let aim = if turning {
        let a = z.yaw * std::f32::consts::TAU / 65536.0;
        Vec3::new(a.cos(), a.sin(), 0.0)
    } else {
        (ue_pos(aim_at) - tip).normalize_or_zero()
    };
    // VRand(): a random unit vector.
    let spread = loop {
        let mut r = || (z.random() % 20001) as f32 / 10000.0 - 1.0;
        let v = Vec3::new(r(), r(), r());
        if v.length_squared() > 1e-4 && v.length_squared() <= 1.0 {
            break v.normalize();
        }
    };
    let dir = (aim + spread * crate::boss::MG_SPREAD).normalize_or_zero();
    let origin = coords::pos(tip.to_array());
    let dir_bevy = coords::dir(dir.to_array());
    let max = crate::boss::MG_RANGE * SCALE;
    let world = Dir3::new(dir_bevy)
        .ok()
        .and_then(|d| spatial.cast_ray(origin, d, max, true, &crate::collision::world_filter()))
        .map(|h| h.distance);
    let on_player = crate::combat::ray_cylinder(origin, dir_bevy, player, PLAYER_RADIUS * SCALE, PLAYER_HALF_HEIGHT * SCALE)
        .filter(|d| *d <= world.unwrap_or(max));
    let what = if on_player.is_some() {
        let amount = (crate::boss::MG_DAMAGE + (z.random() % 3) as f32).floor();
        player_damage.write(crate::combat::PlayerDamaged {
            amount,
            zed_id: z.id,
            kind: crate::combat::HurtKind::Plain,
        });
        push.write(crate::walk::PlayerPush { momentum: dir * crate::boss::MG_MOMENTUM });
        format!("player damage={amount}")
    } else if world.is_some() {
        "world".to_string()
    } else {
        "nothing".to_string()
    };
    // AddTraceHitFX (only when the trace hit something: A != None): the
    // muzzle flash on `tip`, a tracer at 10000 and ROBulletHitEffect at the
    // hit point, rotated along the shot, whatever was hit (the player too).
    if let Some(d) = on_player.or(world) {
        let hit = ue_pos(origin + dir_bevy * d);
        z.mg_flash_shots += 1;
        bullet_fx.write(crate::bullet_fx::BulletFx {
            shooter: crate::bullet_fx::Shooter::Zed(z.id),
            start: Some(tip),
            hit,
            into: (hit - tip).normalize_or_zero(),
            impact: true,
            tracer_speed: crate::boss::MG_TRACER_SPEED,
            min_distance: 10.0,
        });
    }
    runlog::kv("boss_mg_shot", &format!("id={} left={shots_left} hit={what} turning={turning}", z.id));
}

fn sees(spatial: &SpatialQuery, a: Vec3, b: Vec3) -> bool {
    let Ok(d) = Dir3::new(b - a) else {
        return true;
    };
    spatial.cast_ray(a, d, (b - a).length(), true, &crate::collision::world_filter()).is_none()
}

/// ZombieHusk.SpawnTwoShots: a HuskFireProjectile from the Barrel bone,
/// aimed by HuskZombieController.AdjustAim: lead the target by its velocity
/// x distance / speed x min(1, 0.7 + 0.6 FRand()) (no higher than the target),
/// and, with bTrySplash at Skill 2 (assumed for Normal) half the time when
/// the target is not more than 19 above him (the Husk only: the Patriarch's
/// rocket has bTrySplash off), at the floor under the target;
/// otherwise its middle, else its head, whichever is in sight. Aim error and
/// the wall checks are not done.
#[allow(clippy::too_many_arguments)]
fn shoot_fireball(
    z: &mut Zed,
    c: &ZedClass,
    t: &Transform,
    kind: crate::fireball::Projectile,
    bone: Option<usize>,
    target: Vec3,
    target_velocity: Vec3,
    spatial: &SpatialQuery,
    out: &mut MessageWriter<crate::fireball::SpawnFireball>,
) {
    // bTrySplash (aim at the feet): the Husk's fireball; the Patriarch's
    // rocket has it off.
    let try_splash = kind == crate::fireball::Projectile::HuskFire;
    let start = bone
        .and_then(|b| c.model.bone_frame(&z.last_pose, b))
        .map_or(ue_pos(z.centre), |f| world_axes(c, t, f).0);
    let tgt = ue_pos(target);
    let dist = (tgt - ue_pos(z.centre)).length();
    let lead = (0.7 + 0.6 * (z.random() % 1000) as f32 / 1000.0).min(1.0);
    let mut spot = tgt + target_velocity * (lead * dist / kind.speed());
    spot.z = spot.z.min(tgt.z);
    let visible = |p: Vec3| sees(spatial, coords::pos(start.to_array()), coords::pos(p.to_array()));
    let feet = try_splash && (z.random() % 1000) as f32 / 1000.0 > 0.5 && ue_pos(z.centre).z + 19.0 >= tgt.z;
    let mut aim = "middle";
    let mut clean = false;
    if feet {
        let from = coords::pos(spot.to_array());
        if let Some(h) = spatial.cast_ray(from, Dir3::NEG_Y, (PLAYER_HALF_HEIGHT + 10.0) * SCALE, true, &crate::collision::world_filter()) {
            let floor = spot - Vec3::Z * (h.distance / SCALE) + Vec3::Z * 3.0;
            if visible(floor) {
                spot = floor;
                clean = true;
                aim = "feet";
            }
        }
    }
    if !clean {
        spot.z = tgt.z;
        if !visible(spot) {
            spot.z = tgt.z + 0.9 * PLAYER_HALF_HEIGHT;
            aim = "head";
        }
    }
    let dir = (spot - start).normalize_or_zero();
    out.write(crate::fireball::SpawnFireball {
        at: start,
        dir,
        zed_id: z.id,
        kind,
    });
    runlog::kv(
        if try_splash { "husk_shot" } else { "boss_rocket_shot" },
        &format!(
            "id={} aim={aim} start_unreal=({:.0}, {:.0}, {:.0}) spot_unreal=({:.0}, {:.0}, {:.0}){}",
            z.id,
            start.x,
            start.y,
            start.z,
            spot.x,
            spot.y,
            spot.z,
            if try_splash { format!(" next_in={:.1}", z.ranged_wait) } else { String::new() }
        ),
    );
}

/// Moves a zed's attached effects to their anchors' current frames.
fn follow_tags(c: &ZedClass, z: &Zed, t: &Transform, effects: &mut Query<&mut ParticleEffect>) {
    for (e, anchor) in &z.effects {
        if let (Ok(mut fx), Some(frame)) = (effects.get_mut(*e), anchor_frame(c, z, t, anchor)) {
            fx.frame = frame;
        }
    }
}

/// Player cylinder size (KFPawn), for melee reach.
const PLAYER_RADIUS: f32 = 20.0;
const PLAYER_EYE: f32 = 44.0;
const PLAYER_HALF_HEIGHT: f32 = 50.0;
/// ZombieBossBase damageForce: momentum of the Patriarch's melee hits.
const BOSS_DAMAGE_FORCE: f32 = 170000.0;
/// Seconds a corpse stays (KF's RagdollLifeSpan is 30).
const CORPSE_SECONDS: f32 = 30.0;
/// KFMonster ragdoll launch values (Clot defaults; same for all KF zeds).
const RAG_DEATH_VEL: f32 = 100.0;
const RAG_SPIN_SCALE: f32 = 7.5;
const RAG_MAX_SPIN_AMOUNT: f32 = 100.0;
const RAG_INV_INERTIA: f32 = 4.0;
const GRAVITY: f32 = 950.0;

/// KFMonster zap values (ZED guns), from the class defaults.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ZapValues {
    /// ZapDuration, ZappedSpeedMod, ZapThreshold, ZappedDamageMod,
    /// ZapResistanceScale.
    pub duration: f32,
    pub speed_mod: f32,
    pub threshold: f32,
    pub damage_mod: f32,
    pub resistance: f32,
}

impl Default for ZapValues {
    /// KFMonster's defaults.
    fn default() -> Self {
        ZapValues { duration: 4.0, speed_mod: 0.5, threshold: 0.25, damage_mod: 2.0, resistance: 2.0 }
    }
}

/// KFMonster.CrispUpThreshhold (no zed changes it).
const CRISP_UP_THRESHOLD: u32 = 5;

/// KFMonster.BurnEffect.
const BURN_EFFECT: &str = "KFMod.KFMonsterFlame";

pub struct ZedPlugin;

impl Plugin for ZedPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ZedSettings>()
            .init_resource::<ZSpawn>()
            .insert_resource(ZedsActive(true))
            .add_systems(PostStartup, load_zed_classes)
            .add_systems(Update, (spawn_zeds, think_and_move, burn_zeds, animate_zeds, apply_cloaks).chain());
    }
}

/// KFMonster.Timer while burning: every second, TakeFireDamage with
/// LastBurnDamage plus 3 or 4 and FireDamageClass (back through TakeDamage,
/// so the burn grows), and BurnDown goes down by 1; the fire goes out at 0. The flames
/// (StartBurnFX / StopBurnFX) follow the zed while it burns.
/// KF spawns the flames on the skeleton (UseSkeletalLocationAs); here they
/// come from the zed's centre: an approximation.
fn burn_zeds(
    mut commands: Commands,
    time: Res<Time>,
    library: Option<Res<crate::particles::EffectLibrary>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut effects: Query<&mut crate::particles::ParticleEffect>,
    mut zeds: Query<(&mut Zed, &Transform)>,
    mut kills: ResMut<crate::combat::KillCount>,
) {
    let dt = time.delta_secs();
    for (mut z, t) in &mut zeds {
        let b = t.translation;
        let ue = Vec3::new(-b.z, b.x, b.y) / SCALE;
        let alive = z.health > 0.0;
        // Zap (KFMonster.Tick). SetZappedBehavior uncloaks (the Stalker
        // and the Patriarch do not cloak while zapped); when it wears off
        // during a run, rage or charge, that state keeps the normal speed.
        if alive {
            if z.zapped() && z.cloaked {
                z.cloaked = false;
                z.cloak_dirty = true;
            }
            if z.zap_tick(dt) {
                z.run_speed_lost = z.running || z.raging || z.fp_rage.is_some() || z.boss.is_some_and(|b| b.charge.is_some() || b.escaping());
                runlog::kv("zed_unzapped", &format!("zed={} next_threshold={:.2} run_speed_lost={}", z.id, z.zap_threshold, z.run_speed_lost));
            }
        }
        if z.burn_down > 0 && alive {
            z.burn_timer -= dt;
            if z.burn_timer <= 0.0 {
                z.burn_timer += 1.0;
                let damage = (z.last_burn_damage + (z.random() % 2) as f32 + 3.0).floor();
                let fire = z.fire_class;
                let source = crate::combat::HitSource { point: b, attacker: b, melee: false, explosive: None, fire: Some(fire) };
                crate::combat::damage_zed(&mut z, damage, false, 1.0, "fire", 0.0, source, &mut kills);
                z.burn_down -= 1;
                runlog::kv(
                    "zed_burn_tick",
                    &format!("zed={} damage={damage} fire={fire:?} left={} health={:.1}", z.id, z.burn_down, z.health),
                );
                if z.burn_down == 0 {
                    runlog::kv("zed_burn_out", &format!("zed={}", z.id));
                }
            }
        }
        let burning = z.burn_down > 0 && z.health > 0.0;
        match (burning, z.burn_fx) {
            (true, None) => {
                if let Some(lib) = library.as_deref() {
                    let id = z.id as u32;
                    let opts = crate::particles::SpawnOptions { persistent: true, ..default() };
                    z.burn_fx = crate::particles::spawn_effect_with(&mut commands, lib, &mut meshes, BURN_EFFECT, ue, Mat3::IDENTITY, id, opts);
                    runlog::kv("zed_burn_fx", &format!("zed={} started={}", z.id, z.burn_fx.is_some()));
                }
            }
            (true, Some(e)) => {
                if let Ok(mut fx) = effects.get_mut(e) {
                    fx.frame.0 = ue;
                }
            }
            (false, Some(e)) => {
                if let Ok(mut fx) = effects.get_mut(e) {
                    fx.kill();
                }
                z.burn_fx = None;
            }
            (false, None) => {}
        }
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
                    "zed_class_fire",
                    &format!("class={} burning_walk={:?} fire_resist={}", c.name, c.burning_walk, c.fire_resist),
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
                named: Vec::new(),
            }
        }
        _ => Skins {
            refs: Vec::new(),
            package: None,
            named: Vec::new(),
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
    // Attacks played on the upper body while walking (DoAnimAction
    // overrides): the Clot's grapples, the Scrake's saw swings.
    let layered_attacks: Vec<usize> = match kind {
        ZedKind::Clot => ["ClotGrapple", "ClotGrappleTwo", "ClotGrappleThree"].iter().filter_map(|n| model.sequence(n)).collect(),
        ZedKind::Scrake => ["SawZombieAttack1", "SawZombieAttack2"].iter().filter_map(|n| model.sequence(n)).collect(),
        ZedKind::Fleshpound => ["PoundAttack1", "PoundAttack2", "PoundAttack3", "FPRageAttack"].iter().filter_map(|n| model.sequence(n)).collect(),
        // ZombieSiren.DoAnimAction: bites and the scream from SpineBone1.
        ZedKind::Siren => ["Siren_Bite", "Siren_Bite2"].iter().filter_map(|n| model.sequence(n)).collect(),
        _ => Vec::new(),
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
    let ranged_anim = match kind {
        ZedKind::Bloat => model.sequence("ZombieBarf"),
        ZedKind::Siren => model.sequence("Siren_Scream"),
        ZedKind::Husk => model.sequence("ShootBurns"),
        _ => None,
    };
    let run_attack_seconds = model.sequence("GoreAttack1").map_or(0.0, |s| model.length(s) / model.rate(s).max(1e-3));
    let first_name = |p: &str| defaults.get_array_names(&class, p).first().and_then(|n| model.sequence(n));
    let air_anim = first_name("AirAnims");
    let land_anim = first_name("LandAnims");
    let turn_left = name_of("TurnLeftAnim").and_then(|n| model.sequence(&n));
    let turn_right = name_of("TurnRightAnim").and_then(|n| model.sequence(&n));
    let fire_root_bone = name_of("FireRootBone").and_then(|n| model.find_bone(&n));
    let spine_bone = name_of("SpineBone1").and_then(|n| model.find_bone(&n));
    // The upper-body layer's root (FireRootBone; the Siren and the Patriarch
    // layer from SpineBone1).
    let fire_root_bone = if matches!(kind, ZedKind::Siren | ZedKind::Patriarch) { spine_bone.or(fire_root_bone) } else { fire_root_bone };
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
        jump_z: float("JumpZ", 320.0),
        pounce_speed: if kind == ZedKind::Crawler { float("PounceSpeed", 0.0) } else { 0.0 },
        no_flip: matches!(kind, ZedKind::Crawler | ZedKind::Fleshpound | ZedKind::Bloat | ZedKind::Siren | ZedKind::Patriarch),
        flinch_root: if kind == ZedKind::Crawler { name_of("NeckBone").and_then(|n| model.find_bone(&n)) } else { None },
        saw_impale: if kind == ZedKind::Scrake { model.sequence("SawImpaleLoop") } else { None },
        charge_anim: if kind == ZedKind::Scrake { model.sequence("ChargeF") } else { None },
        flinch_min_damage: match kind {
            ZedKind::Scrake => 150.0,
            ZedKind::Fleshpound => 10.0,
            _ => 5.0,
        },
        fp_rage_anim: if kind == ZedKind::Fleshpound { model.sequence("PoundRage") } else { None },
        fp_charge_walk: if kind == ZedKind::Fleshpound { name_of("ChargingAnim").and_then(|n| model.sequence(&n)) } else { None },
        fp_rage_attack: if kind == ZedKind::Fleshpound { model.sequence("FPRageAttack") } else { None },
        fp_rage_threshold: float("RageDamageThreshold", 0.0),
        fp_red_device: if kind == ZedKind::Fleshpound { load_named_material(set, "KFCharacters", "FPRedBloomShader", images, materials) } else { None },
        small_arms_scale: if kind == ZedKind::Fleshpound { 0.5 } else { 1.0 },
        motion_threat: float("MotionDetectorThreat", 1.0),
        fire_resist: if kind == ZedKind::Husk { float("BurnDamageScale", 1.0) } else { 1.0 },
        zap: {
            let d = ZapValues::default();
            ZapValues {
                duration: float("ZapDuration", d.duration),
                speed_mod: float("ZappedSpeedMod", d.speed_mod),
                threshold: float("ZapThreshold", d.threshold),
                damage_mod: float("ZappedDamageMod", d.damage_mod),
                resistance: float("ZapResistanceScale", d.resistance),
            }
        },
        burning_walk: defaults.get_array_names(&class, "BurningWalkFAnims").first().and_then(|n| model.sequence(n)),
        ranged_anim,
        ranged_distance: match kind {
            ZedKind::Siren => float("ScreamRadius", 700.0),
            // ZombieHusk.RangedAttack: anywhere within 65535 (no distance fog).
            ZedKind::Husk => 65535.0,
            _ => BLOAT_BARF_DISTANCE,
        },
        ranged_interval: float("ProjectileFireInterval", 0.0),
        barrel_bone: if kind == ZedKind::Husk { model.find_bone("Barrel") } else { None },
        ranged_shots: ranged_anim.map_or(Vec::new(), |s| {
            model.notifies(s).iter().filter(|n| n.name.eq_ignore_ascii_case("SpawnTwoShots")).map(|n| n.time).collect()
        }),
        ranged_effects: ranged_anim.map_or(Vec::new(), |s| {
            model.notifies(s).iter().filter_map(|n| n.effect.clone().map(|e| (n.time, e))).collect()
        }),
        ranged_moving_chance: match kind {
            ZedKind::Siren => 1.0,
            ZedKind::Bloat => BLOAT_CHARGE_CHANCE,
            _ => 0.0,
        },
        scream: (kind == ZedKind::Siren).then(|| (solo_damage(float("ScreamDamage", 8.0)), float("ScreamRadius", 700.0), float("ScreamForce", -150000.0))),
        boss: if kind == ZedKind::Patriarch {
            match crate::boss::BossClass::load(&model, name_of("ChargingAnim").as_deref()) {
                Ok(b) => {
                    runlog::kv(
                        "boss_loaded",
                        &format!("claw_hits={:?} claw_range={} impale_hits={:?} impale_range={}", b.claw.hits, b.claw.range, b.impale.hits, b.impale.range),
                    );
                    Some(b)
                }
                Err(e) => {
                    runlog::kv("boss_error", &format!("error=\"{e}\""));
                    None
                }
            }
        } else {
            None
        },
        burst_bone: if kind == ZedKind::Bloat { name_of("SpineBone2").and_then(|n| model.find_bone(&n)) } else { None },
        cloak_parts: if kind == ZedKind::Patriarch {
            model
                .parts
                .iter()
                .map(|p| {
                    let texture = materials.get(&p.material).and_then(|m| m.base_color_texture.clone());
                    materials.add(StandardMaterial {
                        base_color: Color::srgba(0.85, 0.9, 1.0, 0.15),
                        base_color_texture: texture,
                        alpha_mode: AlphaMode::Blend,
                        perceptual_roughness: 0.2,
                        reflectance: 0.5,
                        cull_mode: None,
                        double_sided: true,
                        ..default()
                    })
                })
                .collect()
        } else {
            Vec::new()
        },
        cloak_material: (kind == ZedKind::Stalker).then(|| {
            let texture = model.parts.first().and_then(|p| materials.get(&p.material)).and_then(|m| m.base_color_texture.clone());
            materials.add(StandardMaterial {
                base_color: Color::srgba(0.85, 0.9, 1.0, 0.15),
                base_color_texture: texture,
                alpha_mode: AlphaMode::Blend,
                perceptual_roughness: 0.2,
                reflectance: 0.5,
                cull_mode: None,
                double_sided: true,
                ..default()
            })
        }),
        ext_collision,
        death,
        death_hold_frame,
        health: float("Health", 100.0),
        head_health: float("HeadHealth", 25.0),
        head_radius: float("HeadRadius", 7.0) * head_scale,
        head_offset: float("HeadHeight", 2.0) * head_scale,
        head_bone,
        melee_damage: solo_damage(float("MeleeDamage", 6.0)),
        name: class_path.to_string(),
        draw_scale: float("DrawScale", 1.0),
        pre_pivot,
        collision_radius: float("CollisionRadius", 22.0),
        collision_height: float("CollisionHeight", 22.0),
        ground_speed: float("GroundSpeed", 440.0),
        hidden_speed: float("HiddenGroundSpeed", 300.0),
        turn_rate,
        melee_range: float("MeleeRange", 50.0),
        idle: name_of("IdleRestAnim").and_then(|n| model.sequence(&n)),
        walk: name_of("MovementAnims").and_then(|n| model.sequence(&n)),
        melee,
        door_bash: {
            let seq = model.sequence("DoorBash");
            if let Some(s) = seq {
                let notes: Vec<String> = model.notifies(s).iter().map(|n| format!("{}@{:.2}", n.name, n.time)).collect();
                runlog::kv("zed_door_bash_anim", &format!("class={class_path} frames={} notifies=[{}]", model.length(s), notes.join(" ")));
            }
            seq
        },
        distance_door_attack: matches!(get("bCanDistanceAttackDoors"), Some((Value::Bool(true), _))),
        intelligence: match get("Intelligence") {
            Some((Value::Byte(b), _)) => b,
            _ => 3,
        },
        model,
    })
}

/// A material by package and object name, drawn the way skinned models
/// draw it (e.g. the Fleshpound's red device shader).
fn load_named_material(
    set: &PackageSet,
    package: &str,
    name: &str,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> Option<Handle<StandardMaterial>> {
    let lp = set.load(package)?;
    let export = (0..lp.pkg.exports.len()).find(|&i| lp.pkg.object_name(ObjectRef::Export(i)).eq_ignore_ascii_case(name))?;
    let h = ObjectHandle { package: lp, export };
    let simple = ue_assets::material::resolve(set, &h, ObjectRef::Export(export));
    let image = simple.texture.as_ref().and_then(|t| crate::skinned::decode_image(t, images))?;
    Some(materials.add(StandardMaterial {
        base_color_texture: Some(image),
        alpha_mode: match simple.blend {
            ue_assets::material::Blend::Additive => AlphaMode::Add,
            ue_assets::material::Blend::Masked => AlphaMode::Mask(0.5),
            _ => AlphaMode::Opaque,
        },
        cull_mode: None,
        double_sided: true,
        ..default()
    }))
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
pub(crate) fn dir_of(yaw: f32) -> Vec3 {
    let a = yaw * std::f32::consts::TAU / 65536.0;
    coords::dir([a.cos(), a.sin(), 0.0])
}

impl Zed {
    /// Marks the zed dead (death animation is started by the think system).
    pub fn kill(&mut self) {
        // ZombieStalker.PlayDying: the corpse shows the normal skin.
        if self.cloaked {
            self.cloaked = false;
            self.cloak_dirty = true;
        }
        self.health = 0.0;
        self.bleed_out = None;
        self.state = ZedState::Dead;
        self.dead_for = 0.0;
    }

    /// KFMonster.RemoveHead (the damage part is in combat.rs): the head is
    /// gone; if the zed is still alive it bleeds out after BleedOutDuration.
    pub fn remove_head(&mut self) {
        // ZombieStalker.RemoveHead: back to the normal skin ("No head, no
        // cloak").
        if self.cloaked {
            self.cloaked = false;
            self.cloak_dirty = true;
        }
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
            // ZombieSiren.RemoveHead: half the time she dies at once
            // (KilledBy), else within 10 x FRand() s.
            if self.quick_headless_death {
                let roll = (self.random() % 1000) as f32 / 1000.0;
                let when = if roll < 0.5 { 0.0 } else { 10.0 * (self.random() % 1000) as f32 / 1000.0 };
                self.bleed_out = Some(when);
            }
        }
        // ZombieScrake RunningState.RemoveHead: the rage ends.
        self.raging = false;
    }

    /// ZombieBoss.TakeDamage: knocked down below the next healing level.
    pub fn note_boss_health(&mut self) {
        let health = self.health;
        if let Some(b) = self.boss.as_mut()
            && b.check_knockdown(health)
        {
            runlog::kv(
                "boss_knockdown",
                &format!("id={} asked health={health:.0} level={} syringes={}", self.id, b.healing_levels[b.syringes], b.syringes),
            );
        }
    }

    /// ZombieBoss FireChaingun.TakeDamage: who shot him, `distance` from
    /// him in Unreal units.
    pub fn note_attacker_distance(&mut self, distance: f32) {
        if let Some(b) = self.boss.as_mut()
            && distance < crate::boss::MG_CLOSE_DAMAGE_DISTANCE
        {
            b.hit_from_close = true;
        }
    }

    /// ZombieFleshPound.TakeDamage: health lost within 2 s of the previous
    /// hit adds up (TwoSecondDamageTotal); over the threshold, with the head
    /// on and not raging already, the Fleshpound starts to rage.
    pub fn note_damage(&mut self, lost: f32) {
        let threshold = self.fp_rage_threshold;
        if threshold <= 0.0 {
            return;
        }
        if self.fp_since_damaged > 2.0 {
            self.fp_two_sec_damage = 0.0;
        }
        self.fp_since_damaged = 0.0;
        self.fp_two_sec_damage += lost;
        if self.fp_two_sec_damage > threshold && !self.decapitated && self.fp_rage.is_none() && !self.zapped() {
            self.fp_start_rage = true;
        }
    }

    /// bZapped.
    pub(crate) fn zapped(&self) -> bool {
        self.remaining_zap > 0.0
    }

    /// KFMonster.SetZapped: zapped already: back to a full ZapDuration;
    /// otherwise TotalZap grows, and at ZapThreshold the zed is zapped.
    pub(crate) fn set_zapped(&mut self, amount: f32) {
        self.since_zap = 0.0;
        if self.zapped() {
            self.total_zap = self.zap_threshold;
            self.remaining_zap = self.zap.duration;
        } else {
            self.total_zap += amount;
            if self.total_zap >= self.zap_threshold {
                self.remaining_zap = self.zap.duration;
                runlog::kv(
                    "zed_zapped",
                    &format!("zed={} threshold={:.2} duration={}", self.id, self.zap_threshold, self.zap.duration),
                );
            }
        }
    }

    /// KFMonster.Tick: a zap runs out (the threshold then grows x
    /// ZapResistanceScale); zap taken but not enough fades 1 per second
    /// once none came for 0.1 s. Returns true when a zap wears off.
    fn zap_tick(&mut self, dt: f32) -> bool {
        self.since_zap += dt;
        if self.zapped() {
            self.remaining_zap -= dt;
            if self.remaining_zap <= 0.0 {
                self.remaining_zap = 0.0;
                self.zap_threshold *= self.zap.resistance;
                return true;
            }
        } else if self.total_zap > 0.0 && self.since_zap > 0.1 {
            self.total_zap = (self.total_zap - dt).max(0.0);
        }
        false
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
            // RunningState.BeginState: not while zapped.
            if self.zapped() {
                return None;
            }
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
        // ZombieBloat.HitCanInterruptAction: no hit reaction mid-attack.
        if self.no_hit_reactions {
            return None;
        }
        if self.uninterruptible && self.attack.is_some() {
            return None;
        }
        // PlayHit: a hit of more than Health / 1.5 knocks the zed down.
        if damage > self.default_health / 1.5 && self.can_flip {
            self.pending_reaction = Some(HitReaction::KnockDown);
            return self.pending_reaction;
        }
        // PlayTakeHit: at most one pain animation every 0.5 s; under 5
        // damage no animation for our damage types (9mm, knife).
        if self.since_pain_anim < MIN_TIME_BETWEEN_PAIN_ANIMS {
            return None;
        }
        self.since_pain_anim = 0.0;
        if damage < self.flinch_min_damage {
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

    /// A Patriarch for the hit rules (health 4000, head 25 x 1.3 x ...;
    /// only the fields the hit code reads), for tests.
    #[cfg(test)]
    pub fn test_patriarch() -> Zed {
        Zed {
            health: 4000.0,
            health_max: 4000.0,
            default_health: 4000.0,
            keeps_head: true,
            no_hit_reactions: true,
            ..Zed::test_clot()
        }
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
            router: Default::default(),
            air_velocity: Vec3::ZERO,
            jump_cooldown: 0.0,
            door_bash: None,
            door_checked: None,
            on_pad: None,
            last_seen: f32::MIN,
            last_render: f32::MIN,
            last_view_check: f32::MIN,
            hidden: false,
            pouncing: false,
            since_pounce: f32::MAX,
            can_flip: true,
            sawing: false,
            saw_charging: false,
            raging: false,
            flinch_min_damage: 5.0,
            uninterruptible: false,
            quick_headless_death: false,
            ranged_wait: 0.0,
            bled_out: false,
            burst_done: false,
            pending_fx: Vec::new(),
            fp_two_sec_damage: 0.0,
            fp_since_damaged: f32::MAX,
            fp_start_rage: false,
            fp_rage: None,
            fp_frustration: 0.0,
            fp_frustration_limit: 10.0,
            fp_frustrated: false,
            fp_rage_threshold: 0.0,
            small_arms_scale: 1.0,
            motion_threat: 1.0,
            burn_down: 0,
            last_burn_damage: 0.0,
            heat: 0,
            fire_class: crate::combat::FireType::Flamethrower,
            burn_timer: 0.0,
            burn_fx: None,
            burned_scale: 1.0,
            fire_resist: 1.0,
            total_zap: 0.0,
            remaining_zap: 0.0,
            since_zap: 1e6,
            zap_threshold: ZapValues::default().threshold,
            zap: ZapValues::default(),
            run_speed_lost: false,
            keeps_head: false,
            no_hit_reactions: false,
            boss: None,
            mg_flash: None,
            mg_flash_shots: 0,
            cloaked: false,
            since_uncloak: f32::MAX,
            cloak_check: 0.0,
            cloak_dirty: false,
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

    /// Seconds since the player last saw this zed (CanKillMeYet's test).
    pub fn unseen_for(&self, now: f32) -> f32 {
        now - self.last_seen
    }

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

/// A door's Location (Unreal units) as an array, for aiming.
fn d_pos(d: &crate::door::Door) -> [f32; 3] {
    d.location()
}

/// KFMonsterController.DoorBashing for one frame. The loop: while the
/// door is sealed, visible and not bZombiesIgnore, AttackDoor, wait for
/// the animation (polled every 0.25 s), Sleep(0.1); after each, a zed of
/// Intelligence BRAINS_Mammal or more leaves for an enemy it can reach.
/// When the loop ends: WhatToDoNext (back to the chase).
///
/// DoorAttack by class: KFMonster plays the full-body DoorBash, whose
/// ClawDamageTarget notifies each hit the door for MeleeDamage -5% ..
/// +5%. ZombieBloat / ZombieHusk with bDistanceAttackingDoor (and a head):
/// ZombieBarf / ShootBurns, whose SpawnTwoShots each do 22 to the door.
/// ZombieSiren (with a head): Siren_Scream, each SpawnTwoShots
/// ScreamDamage x 0.6 to the door (nothing while zapped). ZombieBoss:
/// PreFireMissile and a rocket at the door (state FireMissile).
fn door_bashing(
    z: &mut Zed,
    c: &ZedClass,
    dt: f32,
    doors: &crate::door::Doors,
    spatial: &SpatialQuery,
    target: Vec3,
    hits: &mut MessageWriter<crate::door::ZedDoorHit>,
) {
    let Some(mut b) = z.door_bash else {
        z.state = ZedState::Chase;
        return;
    };
    let leave = |z: &mut Zed, why: &str| {
        runlog::kv("zed_door_bash", &format!("id={} door={} end reason={why}", z.id, b.door));
        z.door_bash = None;
        z.state = ZedState::Chase;
        z.sequence = None;
    };
    // The loop's ActorReachable(Enemy) check, with doors in the way.
    let reachable = |z: &Zed| {
        let hunt = crate::nav::hunt_size(c.collision_radius, c.collision_height);
        let touch = crate::nav::HUNT_RADIUS + PLAYER_RADIUS;
        c.intelligence >= 2 && crate::nav::probe_with(spatial, crate::collision::zed_filter(), z.centre, target, touch, hunt.0, hunt.1).is_ok()
    };
    if b.in_anim {
        b.anim_time += dt;
        let Some(seq) = z.sequence else {
            b.in_anim = false;
            z.door_bash = Some(b);
            return;
        };
        let len = c.model.length(seq).max(1.0);
        let p = z.frame / len;
        let times: Vec<f32> = if b.ranged {
            c.ranged_shots.clone()
        } else {
            c.model.notifies(seq).iter().filter(|n| n.name.eq_ignore_ascii_case("ClawDamageTarget")).map(|n| n.time).collect()
        };
        for (i, at) in times.iter().enumerate().take(8) {
            if p >= *at && b.hits & (1 << i) == 0 {
                b.hits |= 1 << i;
                let (damage, kind) = if !b.ranged {
                    let roll = (z.random() % 1000) as f32 / 1000.0;
                    (if z.melee_damage > 1.0 { z.melee_damage * 0.95 + z.melee_damage * 0.1 * roll } else { z.melee_damage }, "claw")
                } else if let Some((scream, _, _)) = c.scream {
                    if z.zapped() {
                        continue;
                    }
                    (scream * 0.6, "scream")
                } else {
                    // DamTypeVomit for both the Bloat and the Husk.
                    (22.0, "ranged")
                };
                hits.write(crate::door::ZedDoorHit { door: b.door, damage, zed: z.id, kind });
            }
        }
        // The ranged animation's own effects (the vomit jet, the scream).
        if b.ranged {
            for (at, effect) in c.ranged_effects.iter().take(8) {
                if p >= *at && p - dt * c.model.rate(seq) / len < *at {
                    z.pending_fx.push(effect.clone());
                }
            }
        }
        if z.frame >= len - 0.5 {
            // While(bShotAnim) Sleep(0.25), then Sleep(0.1).
            b.in_anim = false;
            b.wait = (b.anim_time / 0.25).ceil() * 0.25 - b.anim_time + 0.1;
            z.door_bash = Some(b);
            if reachable(z) {
                return leave(z, "enemy_reachable");
            }
            return;
        }
        z.door_bash = Some(b);
        return;
    }
    if b.wait > 0.0 {
        b.wait -= dt;
        z.door_bash = Some(b);
        return;
    }
    let Some(d) = doors.doors.get(b.door) else {
        return leave(z, "no_door");
    };
    if d.hidden {
        return leave(z, "door_broken");
    }
    if !d.sealed {
        return leave(z, "door_unsealed");
    }
    if d.info.zombies_ignore {
        return leave(z, "zombies_ignore");
    }
    // AttackDoor.
    let (seq, ranged) = if let (Some(bc), Some(mut boss)) = (c.boss.as_ref(), z.boss) {
        let Some(anims) = bc.missile_anims else {
            return leave(z, "no_missile_animation");
        };
        // PreFireMissile (full body), state FireMissile; boss_busy aims
        // at the door and comes back here when it is done.
        boss.missile = Some(crate::boss::Missile::start(anims[0].1));
        z.boss = Some(boss);
        z.state = ZedState::BossBusy;
        z.sequence = None;
        start_anim(z, Some(anims[0].0), false);
        b.wait = 0.1;
        z.door_bash = Some(b);
        runlog::kv("zed_door_attack", &format!("id={} door={} kind=rocket", z.id, d.info.name));
        return;
    } else if c.scream.is_some() {
        (if z.decapitated { None } else { c.ranged_anim }, true)
    } else if b.distance && !z.decapitated && c.ranged_anim.is_some() {
        (c.ranged_anim, true)
    } else {
        (c.door_bash, false)
    };
    let Some(seq) = seq else {
        // ZombieSiren.DoorAttack does nothing without a head; the loop
        // goes on (Sleep(0.1)) while the door stays sealed.
        b.wait = 0.1;
        z.door_bash = Some(b);
        if reachable(z) {
            leave(z, "enemy_reachable");
        }
        return;
    };
    b.in_anim = true;
    b.ranged = ranged;
    b.hits = 0;
    b.anim_time = 0.0;
    z.door_bash = Some(b);
    z.sequence = None;
    start_anim(z, Some(seq), false);
    runlog::kv(
        "zed_door_attack",
        &format!("id={} door={} kind={} sequence={}", z.id, d.info.name, if ranged { "ranged" } else { "bash" }, c.model.sequence_name(seq).unwrap_or("?")),
    );
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
    in_line: bool,
) {
    let c = &classes.0[class];
    let forward = Vec3::new(-cam.yaw.sin(), 0.0, -cam.yaw.cos());
    let right = Vec3::new(cam.yaw.cos(), 0.0, -cam.yaw.sin());
    // Side by side, 60 units apart (0, +60, -60, +120, ...), so zeds spawned
    // in a row do not start inside each other.
    let side = if in_line { 0.0 } else { (id.div_ceil(2) as f32) * if id % 2 == 1 { 60.0 } else { -60.0 } };
    let probe = cam_t.translation + (forward * distance + right * side) * SCALE;
    let Some(hit) = spatial.cast_ray(probe, Dir3::NEG_Y, 20.0, true, &crate::collision::world_filter()) else {
        runlog::kv("zed_spawn_failed", "reason=no_floor_below");
        return;
    };
    let centre = probe - Vec3::Y * hit.distance + Vec3::Y * (c.collision_height + 1.0 + lift) * SCALE;
    spawn_zed(commands, meshes, classes, class, id, centre, yaw_of(-forward));
}

/// Spawns a zed of class `class` with its cylinder centre at `centre`
/// (Bevy space), facing `yaw` (Unreal units).
fn spawn_zed(commands: &mut Commands, meshes: &mut Assets<Mesh>, classes: &ZedClasses, class: usize, id: usize, centre: Vec3, yaw: f32) {
    let c = &classes.0[class];
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
                router: Default::default(),
                air_velocity: Vec3::ZERO,
                jump_cooldown: 0.0,
                door_bash: None,
                door_checked: None,
                on_pad: None,
                last_seen: f32::MIN,
                last_render: f32::MIN,
                last_view_check: f32::MIN,
                hidden: false,
                pouncing: false,
                since_pounce: f32::MAX,
                can_flip: !c.no_flip,
                sawing: false,
                saw_charging: false,
                raging: false,
                flinch_min_damage: c.flinch_min_damage,
                uninterruptible: matches!(c.kind, ZedKind::Bloat | ZedKind::Husk),
                quick_headless_death: c.kind == ZedKind::Siren,
                ranged_wait: 0.0,
                bled_out: false,
                burst_done: false,
                pending_fx: Vec::new(),
                fp_two_sec_damage: 0.0,
                fp_since_damaged: f32::MAX,
                fp_start_rage: false,
                fp_rage: None,
                fp_frustration: 0.0,
                fp_frustration_limit: 10.0 + 5.0 * ((id as u32).wrapping_mul(2_654_435_761) % 1000) as f32 / 1000.0,
                fp_frustrated: false,
                fp_rage_threshold: c.fp_rage_threshold,
                small_arms_scale: c.small_arms_scale,
                motion_threat: c.motion_threat,
                burn_down: 0,
                last_burn_damage: 0.0,
                heat: 0,
                fire_class: crate::combat::FireType::Flamethrower,
                burn_timer: 0.0,
                burn_fx: None,
                burned_scale: if c.kind == ZedKind::Bloat { 1.5 } else { 1.0 },
                fire_resist: c.fire_resist,
                total_zap: 0.0,
                remaining_zap: 0.0,
                since_zap: 1e6,
                zap_threshold: c.zap.threshold,
                zap: c.zap,
                run_speed_lost: false,
                keeps_head: c.boss.is_some(),
                no_hit_reactions: c.boss.is_some(),
                boss: c.boss.is_some().then(|| crate::boss::BossState::new(c.health)),
                mg_flash: None,
                mg_flash_shots: 0,
                // ZombieStalker.PostBeginPlay: CloakStalker.
                cloaked: c.cloak_material.is_some() || c.boss.is_some(),
                since_uncloak: f32::MAX,
                cloak_check: 0.0,
                cloak_dirty: c.cloak_material.is_some() || c.boss.is_some(),
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
    let parts: Vec<Entity> = c
        .model
        .parts
        .iter()
        .zip(handles)
        .map(|(part, handle)| commands.spawn((Mesh3d(handle), MeshMaterial3d(part.material.clone()), Transform::IDENTITY, ChildOf(parent))).id())
        .collect();
    commands.entity(parent).insert(ZedParts(parts));
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
    mut wave_spawns: MessageReader<crate::game::SpawnZedAt>,
    mut next_id: Local<usize>,
) {
    // Test action "toggle_zeds": the same as X.
    let toggle = script.0.iter().any(|(f, a)| *f == frames.0 && a == "toggle_zeds");
    if keys.just_pressed(KeyCode::KeyX) || toggle {
        active.0 = !active.0;
        runlog::kv("zeds_active", &format!("active={}", active.0));
    }
    let Some(classes) = classes else {
        return;
    };
    if classes.0.is_empty() || frames.0 < 6 {
        return;
    }
    // Zeds from the wave loop (game.rs): a class standing on a floor point.
    for w in wave_spawns.read() {
        let Some(class) = classes.0.iter().position(|c| c.name.eq_ignore_ascii_case(&w.class)) else {
            runlog::kv("zed_spawn_failed", &format!("reason=class_not_loaded class={}", w.class));
            continue;
        };
        let centre = coords::pos(w.centre.to_array());
        spawn_zed(&mut commands, &mut meshes, &classes, class, *next_id, centre, w.yaw);
        *next_id += 1;
    }
    let start = frames.0 == 6;
    let scripted = |action: &str| script.0.iter().any(|(f, a)| *f == frames.0 && a == action);
    if keys.just_pressed(KeyCode::KeyN) || scripted("cycle_zed") {
        z_spawn.class = (z_spawn.class + 1) % classes.0.len();
        z_spawn.label = format!("{:?}", classes.0[z_spawn.class].kind);
        runlog::kv("z_spawn_selected", &format!("kind={}", z_spawn.label));
    }
    // What to spawn this frame: (kind, distance in front, lift).
    // (kind, distance ahead, lift, in line: no sideways offset)
    let mut wanted: Vec<(ZedKind, f32, f32, bool)> = Vec::new();
    // Z spawns the type picked with N.
    if keys.just_pressed(KeyCode::KeyZ) {
        wanted.push((classes.0[z_spawn.class.min(classes.0.len() - 1)].kind, 300.0, 0.0, false));
    }
    // H (G is KF's grenade key).
    if keys.just_pressed(KeyCode::KeyH) {
        wanted.push((ZedKind::Gorefast, 300.0, 0.0, false));
    }
    if start {
        if settings.spawn_at_start {
            wanted.push((ZedKind::Clot, 300.0, 0.0, false));
        }
        if settings.gorefast_at_start {
            wanted.push((ZedKind::Gorefast, 300.0, 0.0, false));
        }
        if let Some(name) = &settings.spawn_kind {
            match kind_named(name) {
                Some(k) => wanted.push((k, 300.0, 0.0, false)),
                None => runlog::kv("zed_spawn_failed", &format!("reason=unknown_kind name={name}")),
            }
        }
    }
    // Test actions: "zed" (Clot), "zed_drop" (a Clot 200 units up),
    // "gorefast", "gorefast_far" (900 away), "zed_line", "spawn_<kind>".
    for (f, a) in &script.0 {
        if *f != frames.0 {
            continue;
        }
        match a.as_str() {
            "zed" => wanted.push((ZedKind::Clot, 300.0, 0.0, false)),
            "zed_drop" => wanted.push((ZedKind::Clot, 300.0, 200.0, false)),
            "gorefast" => wanted.push((ZedKind::Gorefast, 300.0, 0.0, false)),
            "gorefast_far" => wanted.push((ZedKind::Gorefast, 900.0, 0.0, false)),
            // Three Clots straight ahead, one behind the other (penetration tests).
            "zed_line" => {
                for d in [250.0, 400.0, 550.0] {
                    wanted.push((ZedKind::Clot, d, 0.0, true));
                }
            }
            // The same farther away (explosives arm after 300-500 units).
            "zed_line_far" => {
                for d in [700.0, 800.0, 900.0] {
                    wanted.push((ZedKind::Clot, d, 0.0, true));
                }
            }
            other => {
                if let Some(k) = other.strip_prefix("spawn_").and_then(kind_named) {
                    wanted.push((k, 300.0, 0.0, false));
                }
            }
        }
    }
    let Ok((t, cam)) = cams.single() else {
        return;
    };
    for (kind, distance, lift, in_line) in wanted {
        let Some(class) = classes.0.iter().position(|c| c.kind == kind) else {
            runlog::kv("zed_spawn_failed", &format!("reason=class_not_loaded kind={kind:?}"));
            continue;
        };
        match settings.spawn_at {
            // --zed-at: the start zed at a given place, facing you.
            Some(at) if start => {
                let centre = coords::pos(at);
                spawn_zed(&mut commands, &mut meshes, &classes, class, *next_id, centre, yaw_of(t.translation - centre));
            }
            _ => spawn_in_front(&mut commands, &mut meshes, &classes, &spatial, t, cam, class, *next_id, distance, lift, in_line),
        }
        *next_id += 1;
    }
}

/// Doors and game messages `think_and_move` uses.
#[derive(bevy::ecs::system::SystemParam)]
struct ZedWorld<'w, 's> {
    doors: Res<'w, crate::door::Doors>,
    door_colliders: Query<'w, 's, &'static crate::door::DoorCollider>,
    door_hits: MessageWriter<'w, crate::door::ZedDoorHit>,
    door_blasts: MessageWriter<'w, crate::door::DoorBlast>,
    clear_zeds: MessageReader<'w, 's, crate::game::ClearZeds>,
    kill_stuck: MessageReader<'w, 's, crate::game::KillStuckZed>,
    glass: Query<'w, 's, &'static crate::glass::GlassCollider>,
    glass_bumps: MessageWriter<'w, crate::glass::GlassBump>,
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
    (mut vomit, mut push, mut fireball, mut bullet_fx): (
        MessageWriter<crate::vomit::SpawnVomit>,
        MessageWriter<crate::walk::PlayerPush>,
        MessageWriter<crate::fireball::SpawnFireball>,
        MessageWriter<crate::bullet_fx::BulletFx>,
    ),
    mut kills: ResMut<crate::combat::KillCount>,
    mut pinned: ResMut<crate::combat::PlayerPinned>,
    nav: Res<crate::nav::NavNetwork>,
    script: Res<crate::weapon::ScriptedInput>,
    frames: Res<bevy::diagnostic::FrameCount>,
    mut world: ZedWorld,
    mut log_timer: Local<f32>,
) {
    let ZedWorld { doors, door_colliders, door_hits, door_blasts, clear_zeds, kill_stuck, glass, glass_bumps } = &mut world;
    let stuck: Vec<usize> = kill_stuck.read().map(|k| k.0).collect();
    let (doors, door_colliders) = (&*doors, &*door_colliders);
    let Some(classes) = classes else {
        return;
    };
    // Test action "hurt_zeds": 100 damage to every living zed (no hit
    // reaction), to test rules that depend on health.
    let hurt = script.0.iter().any(|(f, a)| *f == frames.0 && a == "hurt_zeds");
    // Test action "zap_zeds": SetZapped(10) on every living zed.
    let zap_all = script.0.iter().any(|(f, a)| *f == frames.0 && a == "zap_zeds");
    // Test action "kill_zeds": every living zed dies (wave tests); also a
    // wave game's restart.
    let kill_all = script.0.iter().any(|(f, a)| *f == frames.0 && a == "kill_zeds") || clear_zeds.read().count() > 0;
    // Test action "kill_near_zeds": zeds within 500 units of the player die
    // (the ones that reached you), others live on (cleanup tests).
    let kill_near = script.0.iter().any(|(f, a)| *f == frames.0 && a == "kill_near_zeds");
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
        if kill_all || (kill_near && (z.centre - target).length() / SCALE < 500.0 && !z.is_dead()) {
            z.last_hit = None;
            z.kill();
            kills.0 += 1;
            runlog::kv("zed_killed_test", &format!("id={}", z.id));
            continue;
        }
        if stuck.contains(&z.id) && !z.is_dead() {
            z.last_hit = None;
            z.kill();
            runlog::kv("zed_killed_stuck", &format!("id={}", z.id));
            continue;
        }
        // KFMonster.Tick (standalone), when CanSpeedAdjust (head on, not
        // zapped): seen within the last 5 s of being drawn, else a sight
        // check from its eyes to the player's every second; unseen zeds
        // move at HiddenGroundSpeed. LastRenderTime (native: drawn this
        // frame) is approximated as within 60 degrees of the view and in
        // clear sight; the zed's eyes as 0.8 of its half height up.
        let now = time.elapsed_secs();
        if !z.decapitated && !z.zapped() {
            let eye = z.centre + Vec3::Y * c.collision_height * 0.8 * SCALE;
            let to = eye - pt.translation;
            let in_view = to.normalize_or_zero().dot(*pt.forward()) > 0.5;
            if in_view && sees(&spatial, pt.translation, eye) {
                z.last_render = now;
            }
            if now - z.last_render > 5.0 {
                if now - z.last_view_check > 1.0 {
                    z.last_view_check = now;
                    let was = z.hidden;
                    z.hidden = !sees(&spatial, eye, pt.translation);
                    if !z.hidden {
                        z.last_seen = now;
                    }
                    if was != z.hidden && log_now {
                        runlog::kv("zed_hidden", &format!("id={} hidden={}", z.id, z.hidden));
                    }
                }
            } else {
                z.last_seen = now;
                z.hidden = false;
            }
        } else {
            z.hidden = false;
        }
        // Bleeding out (KFMonster.Tick): dies when the time is up. No hit
        // momentum, so the ragdoll gets no push.
        if let Some(left) = z.bleed_out {
            let left = left - dt;
            if left <= 0.0 {
                z.last_hit = None;
                z.bled_out = true;
                z.kill();
                kills.0 += 1; // credited to the player, as KF credits LastDamagedBy
                runlog::kv("zed_bled_out", &format!("id={} health_left={:.1}", z.id, z.health));
                continue;
            }
            z.bleed_out = Some(left);
        }
        z.since_pain_anim = (z.since_pain_anim + dt).min(1e6);
        z.since_hit = (z.since_hit + dt).min(1e6);
        if zap_all && z.health > 0.0 {
            z.set_zapped(10.0);
        }
        if hurt && z.health > 100.0 {
            z.health -= 100.0;
            z.note_damage(100.0);
            runlog::kv("zed_hurt_test", &format!("id={} health={:.0}", z.id, z.health));
            z.note_boss_health();
        }
        if c.fp_rage_anim.is_some() {
            z.fp_since_damaged = (z.fp_since_damaged + dt).min(1e6);
            // StartCharging: PoundRage (full body, waits), then RageCharging.
            if std::mem::take(&mut z.fp_start_rage)
                && z.fp_rage.is_none()
                && !matches!(z.state, ZedState::Enraging | ZedState::Falling | ZedState::Dead)
            {
                z.attack = None;
                z.overlay = None;
                z.state = ZedState::Enraging;
                z.sequence = None;
                start_anim(&mut z, c.fp_rage_anim, false);
                z.cloak_dirty = true; // device colour (DeviceGoRed)
                runlog::kv(
                    "fleshpound_rage",
                    &format!("id={} start two_sec_damage={:.0} frustrated={}", z.id, z.fp_two_sec_damage, z.fp_frustrated),
                );
            }
            // RageCharging.Tick: the rage ends when its time is up (not while
            // attacking, never when frustrated).
            if let Some(left) = z.fp_rage {
                let left = left - dt;
                if left <= 0.0 && z.attack.is_none() && !z.fp_frustrated {
                    z.fp_rage = None;
                    z.cloak_dirty = true;
                    runlog::kv("fleshpound_rage", &format!("id={} end reason=time", z.id));
                } else {
                    z.fp_rage = Some(left);
                }
            }
        }
        // ZombieStalker.Tick: every 0.5 s, cloak again 1.2 s after the last
        // uncloak (never once headless).
        if c.cloak_material.is_some() {
            z.since_uncloak = (z.since_uncloak + dt).min(1e6);
            z.cloak_check -= dt;
            if z.cloak_check <= 0.0 {
                z.cloak_check = 0.5;
                if !z.cloaked && !z.decapitated && !z.zapped() && z.since_uncloak > 1.2 {
                    z.cloaked = true;
                    z.cloak_dirty = true;
                    runlog::kv("stalker_cloak", &format!("id={}", z.id));
                }
            }
        }
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
        if matches!(z.state, ZedState::KnockedDown | ZedState::Landing | ZedState::Enraging) {
            if z.sequence.is_some_and(|s| z.frame < c.model.length(s) - 0.5) {
                t.translation = z.centre;
                continue;
            }
            runlog::kv("zed_state", &format!("id={} from={:?} to=Chase reason=animation_done", z.id, z.state));
            if z.state == ZedState::Enraging {
                // BeginRaging -> RageCharging: 5 + FRand() x 6 s (Normal).
                z.fp_rage = Some(5.0 + 6.0 * (z.random() % 1000) as f32 / 1000.0);
                runlog::kv("fleshpound_rage", &format!("id={} charging=true seconds={:.1}", z.id, z.fp_rage.unwrap_or(0.0)));
            }
            z.state = ZedState::Chase;
        }
        // Patriarch: TakeDamage asked for a knockdown (full body, waits).
        if let (Some(bc), Some(mut b)) = (c.boss.as_ref(), z.boss)
            && b.pending_knockdown
            && !matches!(z.state, ZedState::Dead | ZedState::Falling)
            && let Some((seq, secs)) = bc.knockdown_anim
        {
            let roll = (z.random() % 1000) as f32 / 1000.0;
            b.start_knockdown(secs, roll);
            z.boss = Some(b);
            z.attack = None;
            z.overlay = None;
            z.state = ZedState::BossBusy;
            z.sequence = None;
            start_anim(&mut z, Some(seq), false);
            runlog::kv("boss_knockdown", &format!("id={} start health={:.0} seconds={secs:.2}", z.id, z.health));
        }
        // Patriarch, a full-body action (chaingun, rocket, knockdown, heal):
        // `boss_busy` runs it.
        if z.state == ZedState::BossBusy {
            if active.0 {
                let player_velocity = walker.map_or(Vec3::ZERO, |w| ue_dir(w.velocity) / SCALE);
                // ZombieBoss.DoorAttack: the rocket goes at the door
                // (Controller.Target, its Location), not the player.
                let (aim, aim_velocity) = match z.door_bash.and_then(|b| doors.doors.get(b.door)) {
                    Some(d) => (coords::pos(d_pos(d)), Vec3::ZERO),
                    None => (target, player_velocity),
                };
                boss_busy(&mut z, c, &t, aim, aim_velocity, dt, &spatial, &mut player_damage, &mut push, &mut fireball, &mut bullet_fx);
            }
            t.translation = z.centre;
            continue;
        }
        if z.state == ZedState::DoorBashing {
            if active.0 {
                door_bashing(&mut z, c, dt, doors, &spatial, target, door_hits);
            }
            t.translation = z.centre;
            continue;
        }
        let old_yaw = z.yaw;
        let old_sequence = z.sequence;
        let old_centre = z.centre;
        let mover = Mover::new(&spatial, c.collision_radius, c.collision_height, crate::collision::zed_filter());
        let to = (target - z.centre).with_y(0.0);
        let dist = to.length() / SCALE;
        // Attack once within MeleeRange of touching (KF's melee start); the
        // damage check later allows MeleeRange x 1.4.
        let reach = z.melee_range + c.collision_radius + PLAYER_RADIUS;
        let old_state = z.state;

        if active.0 && z.state != ZedState::Falling {
            // Where to head: the player when in reach or attacking, else the
            // hunting route's target (a navigation point, or the player when
            // it can be walked to directly).
            // Patriarch, state Escaping (BossZombieController SyrRetreat): to a
            // hiding spot instead of the player; BeginHealing when there.
            let escape_goal = boss_escape(&mut z, c, target, dt, &nav, &spatial);
            let (goal, touch) = match escape_goal {
                Some(g) => (g, crate::nav::HUNT_RADIUS),
                None => (target, crate::nav::HUNT_RADIUS + PLAYER_RADIUS),
            };
            let steer = if (escape_goal.is_none() && (z.attack.is_some() || dist <= reach)) || nav.points.is_empty() {
                goal
            } else {
                let others: Vec<(Vec3, f32)> = blockers
                    .iter()
                    .filter(|(e, _)| *e != Some(entity) && e.is_some())
                    .map(|(_, cyl)| (cyl.centre, cyl.radius))
                    .collect();
                let mut seed = z.random() | 1;
                let mut frand = move || {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    (seed % 10000) as f32 / 10000.0
                };
                let speed = if z.running { c.ground_speed * GOREFAST_RUN_SPEED } else { c.ground_speed };
                let hunt = crate::nav::hunt_size(c.collision_radius, c.collision_height);
                let input = crate::nav::RouteInput {
                    id: z.id,
                    pos: z.centre,
                    player: goal,
                    touch_player: touch,
                    speed,
                    radius: hunt.0,
                    half_height: hunt.1,
                    others: &others,
                };
                z.router.update(&nav, &spatial, &input, dt, &mut frand)
            };
            // KFMonsterController.FindPath: a zed with bCanDistanceAttackDoors
            // traces to its new MoveTarget; a sealed door in the way means
            // BreakUpDoor(door, true): attack it from here.
            if c.distance_door_attack && !z.decapitated && z.attack.is_none() && steer != goal && z.door_checked != Some(steer) {
                z.door_checked = Some(steer);
                if let Ok(dir) = Dir3::new(steer - z.centre)
                    && let Some(h) = spatial.cast_ray(z.centre, dir, (steer - z.centre).length(), true, &crate::collision::world_filter())
                    && let Ok(dc) = door_colliders.get(h.entity)
                    && let Some(d) = doors.doors.get(dc.0)
                    && d.sealed
                    && !d.hidden
                {
                    z.door_bash = Some(DoorBash { door: dc.0, wait: 0.0, in_anim: false, hits: 0, anim_time: 0.0, distance: true, ranged: false });
                    z.state = ZedState::DoorBashing;
                    z.overlay = None;
                    runlog::kv(
                        "zed_door_bash",
                        &format!("id={} door={} start distance=true distance_unreal={:.0} weld={:.0}", z.id, d.info.name, h.distance / SCALE, d.weld),
                    );
                    t.translation = z.centre;
                    continue;
                }
            }
            let to_steer = (steer - z.centre).with_y(0.0);
            // Turn toward it at RotationRate.
            if to_steer.length() / SCALE > 1.0 {
                let want = yaw_of(to_steer);
                let mut delta = (want - z.yaw).rem_euclid(65536.0);
                if delta > 32768.0 {
                    delta -= 65536.0;
                }
                let step = c.turn_rate * dt;
                z.yaw = (z.yaw + delta.clamp(-step, step)).rem_euclid(65536.0);
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
            // The ranged attack's notifies: its AnimNotify_Effects
            // (KFVomitJet on the head, SirenScream) and each SpawnTwoShots
            // (the Bloat's vomit, the Siren's scream pulses).
            if let (Some(p), Some(a)) = (progress, z.attack)
                && a.ranged
            {
                let mut fired = a.fx_fired;
                for (i, (at, effect)) in c.ranged_effects.iter().enumerate().take(8) {
                    if p >= *at && fired & (1 << i) == 0 {
                        fired |= 1 << i;
                        z.pending_fx.push(effect.clone());
                    }
                }
                let mut shots = a.shots_fired;
                let mut due = 0;
                for (i, at) in c.ranged_shots.iter().enumerate().take(8) {
                    if p >= *at && shots & (1 << i) == 0 {
                        shots |= 1 << i;
                        due += 1;
                    }
                }
                z.attack = Some(Attack {
                    fx_fired: fired,
                    shots_fired: shots,
                    ..a
                });
                for _ in 0..due {
                    if let Some((damage, radius, force)) = c.scream {
                        // ZombieSiren.SpawnTwoShots: nothing while zapped.
                        if !z.zapped() {
                            scream_pulse(&z, damage, radius, force, target, &spatial, &mut player_damage, &mut push);
                            // ZombieSiren.HurtRadius reaches doors too (any
                            // non-zed actor in sight within ScreamRadius).
                            door_blasts.write(crate::door::DoorBlast {
                                at: ue_pos(z.centre),
                                radius,
                                damage,
                                zed: Some(z.id),
                                direct: None,
                                line_of_sight: true,
                                frag: false,
                                source: "siren_scream",
                            });
                        }
                    } else if c.kind == ZedKind::Husk {
                        let player_velocity = walker.map_or(Vec3::ZERO, |w| ue_dir(w.velocity) / SCALE);
                        shoot_fireball(&mut z, c, &t, crate::fireball::Projectile::HuskFire, c.barrel_bone, target, player_velocity, &spatial, &mut fireball);
                    } else {
                        spawn_two_shots(&z, c, target, &mut vomit);
                    }
                }
            }
            // ZombieBoss.ClawDamageTarget: his hits land at the animation's
            // notifies, with the attack's own reach, and push the player.
            if let (Some(p), Some(a), Some(boss)) = (progress, z.attack, c.boss.as_ref())
                && !a.ranged
                && let Some(m) = boss.melee_for(a.seq)
            {
                let mut shots = a.shots_fired;
                let mut due = 0;
                for (i, at) in m.hits.iter().enumerate().take(8) {
                    if p >= *at && shots & (1 << i) == 0 {
                        shots |= 1 << i;
                        due += 1;
                    }
                }
                z.attack = Some(Attack {
                    shots_fired: shots,
                    hit_done: shots.count_ones() as usize >= m.hits.len(),
                    ..a
                });
                for _ in 0..due {
                    let in_range = dist <= m.range * 1.4 + c.collision_radius + PLAYER_RADIUS;
                    let dz = ((target.y - z.centre.y) / SCALE).abs();
                    let level = dz <= c.collision_height.max(PLAYER_HALF_HEIGHT) + 0.5 * c.collision_height.min(PLAYER_HALF_HEIGHT);
                    // Charging.MeleeDamageTarget: push x 1.5; each check uses
                    // one of the charge's attacks, a landed hit ends it.
                    let charging = z.boss.is_some_and(|b| b.charge.is_some());
                    // Escaping.MeleeDamageTarget (the sneak states): push x 1.5
                    // too; SneakAround.MeleeDamageTarget then ends the sneak,
                    // hit or miss.
                    let sneaking = z.boss.is_some_and(|b| b.sneaking());
                    let escaping = z.boss.is_some_and(|b| b.escaping());
                    let push_scale = if charging || escaping { crate::boss::CHARGE_PUSH } else { 1.0 };
                    let id = z.id;
                    if sneaking && let Some(b) = z.boss.as_mut() {
                        b.end_sneak();
                        z.cloaked = false;
                        z.cloak_dirty = true;
                        runlog::kv("boss_sneak", &format!("id={id} end reason=melee landed={}", in_range && level));
                    }
                    if let Some(b) = z.boss.as_mut()
                        && charging
                    {
                        let ended = b.charge_hit(in_range && level);
                        runlog::kv(
                            "boss_charge_attack",
                            &format!("id={id} landed={} attacks_left={} ended={ended}", in_range && level, b.charge.map_or(0, |c| c.attacks_left)),
                        );
                        if ended {
                            runlog::kv("boss_charge", &format!("id={id} end reason=hit"));
                        }
                    }
                    if in_range && level {
                        let roll = (z.random() % 1000) as f32 / 1000.0;
                        let amount = z.melee_damage * 0.95 + z.melee_damage * 0.1 * roll;
                        player_damage.write(crate::combat::PlayerDamaged {
                            amount,
                            zed_id: z.id,
                            kind: crate::combat::HurtKind::Plain,
                        });
                        let (from, to) = (ue_pos(z.centre), ue_pos(target));
                        let momentum = (to - from).normalize_or_zero() * BOSS_DAMAGE_FORCE * push_scale;
                        push.write(crate::walk::PlayerPush { momentum });
                        runlog::kv(
                            "boss_melee_hit",
                            &format!(
                                "id={} sequence={} hit={} damage={amount:.1} distance_unreal={dist:.0} reach_unreal={:.0}",
                                z.id,
                                c.model.sequence_name(a.seq).unwrap_or("?"),
                                shots.count_ones(),
                                m.range * 1.4 + c.collision_radius + PLAYER_RADIUS
                            ),
                        );
                    } else {
                        runlog::kv("zed_attack_missed", &format!("id={} in_range={in_range} level={level} distance_unreal={dist:.0}", z.id));
                    }
                }
            }
            if let (Some(p), Some(a)) = (progress, z.attack)
                && p >= 0.5
                && !a.hit_done
                && !a.ranged
                && c.boss.is_none()
            {
                z.attack = Some(Attack { hit_done: true, ..a });
                let in_range = dist <= z.melee_range * 1.4 + c.collision_radius + PLAYER_RADIUS;
                let dz = ((target.y - z.centre.y) / SCALE).abs();
                let level = dz <= c.collision_height.max(50.0) + 0.5 * c.collision_height.min(50.0);
                let dazed = z.since_decap.is_some_and(|s| s < 2.0);
                if in_range && level && z.stunned <= 0.0 && !dazed {
                    // ClawDamageTarget: MeleeDamage -5% .. +5%.
                    let roll = (z.random() % 1000) as f32 / 1000.0;
                    let mut amount = z.melee_damage * 0.95 + z.melee_damage * 0.1 * roll;
                    // ZombieFleshPound.ClawDamageTarget: repeated-hit attacks do
                    // less per hit (PoundAttack1 x 0.5, PoundAttack2 x 0.25; we
                    // land one hit per attack); raging, MeleeDamageTarget x 1.75
                    // and a landed hit ends the rage.
                    if c.fp_rage_anim.is_some() {
                        match c.model.sequence_name(a.seq) {
                            Some("PoundAttack1") => amount *= 0.5,
                            Some("PoundAttack2") => amount *= 0.25,
                            _ => {}
                        }
                        if z.fp_rage.is_some() {
                            amount *= 1.75;
                            z.fp_rage = None;
                            z.fp_frustrated = false;
                            z.cloak_dirty = true;
                            runlog::kv("fleshpound_rage", &format!("id={} end reason=hit", z.id));
                        }
                    }
                    player_damage.write(crate::combat::PlayerDamaged {
                        amount,
                        zed_id: z.id,
                        kind: crate::combat::HurtKind::Plain,
                    });
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
            // CrawlerController.FireWeaponAt / ZombieCrawler.DoPounce: out of
            // reach, roughly facing the target (KF compares the facing with
            // the un-normalised vector to it, so nearly any forward angle
            // passes), after 4.5 - FRand() x 3 s since the last pounce, and
            // IsInPounceDist (within MeleeRange x 5, landing at its height):
            // leap at PounceSpeed with JumpZ upward.
            z.since_pounce = (z.since_pounce + dt).min(1e6);
            // ZombieCrawler.DoPounce: not while zapped.
            if c.pounce_speed > 0.0 && z.attack.is_none() && dist > reach && z.state == ZedState::Chase && !z.decapitated && !z.zapped() {
                let wait = 4.5 - (z.random() % 1000) as f32 / 1000.0 * 3.0;
                let to_ue = (target - z.centre) / SCALE;
                let ahead = dir_of(z.yaw).dot(to_ue) > 0.85;
                let t_air = dist / c.pounce_speed;
                let end_z = z.centre.y / SCALE + c.jump_z * t_air - 0.5 * GRAVITY * t_air * t_air;
                let lands_level = (end_z - target.y / SCALE).abs() < c.collision_height + PLAYER_HALF_HEIGHT;
                if z.since_pounce > wait && ahead && lands_level && to_ue.length() < z.melee_range * 5.0 {
                    let dir3 = to_ue.normalize_or_zero();
                    z.state = ZedState::Falling;
                    z.air_velocity = dir3.with_y(0.0) * c.pounce_speed * SCALE;
                    z.vertical_speed = c.jump_z * SCALE;
                    z.pouncing = true;
                    z.since_pounce = 0.0;
                    z.sequence = None;
                    start_anim(&mut z, c.air_anim, false);
                    runlog::kv("crawler_pounce", &format!("id={} distance_unreal={dist:.0} wait={wait:.1}", z.id));
                }
            }
            // ZombieScrake: SawingLoop ends when the target is out of reach
            // (RangedAttack -> GoToState('')): damage and speed back to normal.
            if c.saw_impale.is_some() && z.sawing && z.attack.is_none() && dist > reach {
                z.sawing = false;
                z.saw_charging = false;
                z.melee_damage = c.melee_damage;
                runlog::kv("scrake_sawing", &format!("id={} sawing=false distance_unreal={dist:.0}", z.id));
            }
            // ZombieScrake.RangedAttack: not attacking, has a head, under
            // half health: RunningState (rage), GroundSpeed x 3.5.
            // RunningState.BeginState: not while zapped.
            if c.saw_impale.is_some()
                && !z.raging
                && !z.zapped()
                && !z.sawing
                && z.attack.is_none()
                && !z.decapitated
                && z.health / z.health_max < 0.5
            {
                z.raging = true;
                runlog::kv("scrake_rage", &format!("id={} health={:.0}", z.id, z.health));
            }
            // FleshpoundZombieController ZombieCharge: chasing without an
            // attack for RageFrustrationThreshhold (10) + FRand() x 5 s
            // makes him rage (frustrated: the rage only ends on a hit).
            if c.fp_rage_anim.is_some() && z.fp_rage.is_none() && z.state == ZedState::Chase {
                if z.attack.is_some() {
                    z.fp_frustration = 0.0;
                } else {
                    z.fp_frustration += dt;
                    if z.fp_frustration >= z.fp_frustration_limit && !z.decapitated && !z.zapped() {
                        z.fp_frustration = 0.0;
                        z.fp_frustrated = true;
                        z.fp_start_rage = true;
                    }
                }
            }
            // RangedAttack (ZombieBloat, ZombieSiren): out of melee reach,
            // in range and with the head on. The Bloat vomits within 250,
            // with ChargeChance 0.4 (Normal) on the upper body while walking,
            // else standing; the Siren screams within ScreamRadius, always on
            // the upper body (her Tick keeps her walking at 0.65 speed).
            // MonsterController.FireWeaponAt: only at an enemy in view
            // (Focus); HuskZombieController: not before NextFireProjectileTime.
            let dist3 = (target - z.centre).length() / SCALE;
            z.ranged_wait = (z.ranged_wait - dt).max(0.0);
            // ZombieSiren.RangedAttack: no scream while zapped.
            if let Some(ranged) = c.ranged_anim
                && !(c.scream.is_some() && z.zapped())
                && z.attack.is_none()
                && z.stunned <= 0.0
                && !z.decapitated
                && z.state == ZedState::Chase
                && dist > reach
                && dist3 <= c.ranged_distance
                && z.ranged_wait <= 0.0
                && sees(&spatial, z.centre, target)
            {
                if c.ranged_interval > 0.0 {
                    z.ranged_wait = c.ranged_interval + 2.0 * (z.random() % 1000) as f32 / 1000.0;
                }
                let moving = c.fire_root_bone.is_some() && (z.random() % 1000) as f32 / 1000.0 < c.ranged_moving_chance;
                if moving {
                    z.overlay = Some((ranged, 0.0, c.fire_root_bone.expect("checked")));
                } else {
                    z.state = ZedState::Melee;
                    z.sequence = None;
                    start_anim(&mut z, Some(ranged), false);
                }
                z.attack = Some(Attack {
                    seq: ranged,
                    layered: moving,
                    hit_done: false,
                    ranged: true,
                    fx_fired: 0,
                    shots_fired: 0,
                });
                runlog::kv(
                    "zed_ranged_attack",
                    &format!(
                        "id={} sequence={} moving={moving} distance_unreal={dist3:.0}",
                        z.id,
                        c.model.sequence_name(ranged).unwrap_or("?")
                    ),
                );
            }
            // ZombieBoss.RangedAttack: close enough (IsCloseEnuf), MeleeImpale
            // or MeleeClaw on the upper body from SpineBone1.
            if let Some(boss) = c.boss.as_ref()
                && z.attack.is_none()
                && z.state == ZedState::Chase
                && let Some(root) = c.fire_root_bone
                && crate::boss::is_close_enough(dist, (target.y - z.centre.y) / SCALE, c.collision_radius, c.collision_height, PLAYER_RADIUS, PLAYER_HALF_HEIGHT)
            {
                let roll = (z.random() % 1000) as f32 / 1000.0;
                // Escaping.RangedAttack (the sneak states): MeleeClaw only,
                // and UnCloakBoss first.
                let escaping = z.boss.is_some_and(|b| b.escaping());
                let seq = if escaping { boss.claw.seq } else { boss.choose_melee(z.health, roll).seq };
                if escaping && let Some(b) = z.boss.as_mut() {
                    b.cloaked = false;
                    z.cloaked = false;
                    z.cloak_dirty = true;
                    runlog::kv("boss_sneak", &format!("id={} uncloak reason=attack", z.id));
                }
                z.overlay = Some((seq, 0.0, root));
                z.attack = Some(Attack {
                    seq,
                    layered: true,
                    hit_done: false,
                    ranged: false,
                    fx_fired: 0,
                    shots_fired: 0,
                });
                runlog::kv(
                    "zed_attack",
                    &format!(
                        "id={} sequence={} layered=true charge=false length_frames={} rate={} health={:.0} distance_unreal={dist:.0}",
                        z.id,
                        c.model.sequence_name(seq).unwrap_or("?"),
                        c.model.length(seq),
                        c.model.rate(seq),
                        z.health
                    ),
                );
            }
            // ZombieBoss.RangedAttack beyond melee, about every frame while he
            // sees the player (`boss.rs`), and the Charging state's ends.
            if c.boss.is_some()
                && let Some(mut b) = z.boss
            {
                if let Some(why) = b.tick(dt) {
                    runlog::kv("boss_charge", &format!("id={} end reason={why} distance_unreal={dist3:.0}", z.id));
                }
                let initial = b.sneak.is_some_and(|s| s.initial);
                let seen = b.sneaking() && sees(&spatial, z.centre, target);
                match b.sneak_step(dt, seen, z.attack.is_some()) {
                    Some(crate::boss::SneakChange::Cloaked) => {
                        runlog::kv("boss_sneak", &format!("id={} cloak", z.id));
                    }
                    Some(crate::boss::SneakChange::Ended(why)) => {
                        runlog::kv("boss_sneak", &format!("id={} end reason={why} initial={initial} distance_unreal={dist3:.0}", z.id));
                    }
                    None => {}
                }
                if z.cloaked != b.cloaked {
                    z.cloaked = b.cloaked;
                    z.cloak_dirty = true;
                }
                // (Only FireChaingun reacts to being shot from close.)
                b.hit_from_close = false;
                if z.state == ZedState::Chase && sees(&spatial, z.centre, target) {
                    let attacking = z.attack.is_some();
                    let decision = {
                        let mut roll = || (z.random() % 10000) as f32 / 10000.0;
                        b.decide(dist3, attacking, &mut roll)
                    };
                    match decision {
                        crate::boss::Decision::StartCharge { attacks } => {
                            // SetAnimAction('transition'): upper body.
                            if z.attack.is_none()
                                && let (Some(seq), Some(root)) = (c.boss.as_ref().and_then(|bc| bc.transition), c.fire_root_bone)
                            {
                                z.overlay = Some((seq, 0.0, root));
                            }
                            runlog::kv("boss_charge", &format!("id={} start attacks={attacks} distance_unreal={dist3:.0}", z.id));
                        }
                        crate::boss::Decision::EndCharge(why) => {
                            runlog::kv("boss_charge", &format!("id={} end reason={why} distance_unreal={dist3:.0}", z.id));
                        }
                        crate::boss::Decision::StartChaingun { shots } => {
                            if let Some(anims) = c.boss.as_ref().and_then(|bc| bc.mg_anims) {
                                // PreFireMG (full body, waits), state FireChaingun.
                                b.chaingun = Some(crate::boss::Chaingun::start(shots, anims[0].1));
                                b.mg_focal = target;
                                z.attack = None;
                                z.overlay = None;
                                z.state = ZedState::BossBusy;
                                z.sequence = None;
                                start_anim(&mut z, Some(anims[0].0), false);
                                runlog::kv(
                                    "boss_chaingun",
                                    &format!("id={} start shots={shots} next_in={:.1} distance_unreal={dist3:.0}", z.id, b.chaingun_wait),
                                );
                            }
                        }
                        crate::boss::Decision::StartMissile => {
                            if let Some(anims) = c.boss.as_ref().and_then(|bc| bc.missile_anims) {
                                // PreFireMissile (full body, waits), state FireMissile.
                                b.missile = Some(crate::boss::Missile::start(anims[0].1));
                                z.attack = None;
                                z.overlay = None;
                                z.state = ZedState::BossBusy;
                                z.sequence = None;
                                start_anim(&mut z, Some(anims[0].0), false);
                                runlog::kv(
                                    "boss_missile",
                                    &format!("id={} start next_in={:.1} distance_unreal={dist3:.0}", z.id, b.missile_wait),
                                );
                            }
                        }
                        crate::boss::Decision::DelayMissile(wait) => {
                            runlog::kv("boss_missile", &format!("id={} put_off seconds={wait:.1}", z.id));
                        }
                        crate::boss::Decision::StartSneak => {
                            // SetAnimAction('transition'), GoToState('SneakAround'):
                            // CloakBoss at once.
                            if z.attack.is_none()
                                && let (Some(seq), Some(root)) = (c.boss.as_ref().and_then(|bc| bc.transition), c.fire_root_bone)
                            {
                                z.overlay = Some((seq, 0.0, root));
                            }
                            z.cloaked = !z.zapped();
                            z.cloak_dirty = true;
                            runlog::kv("boss_sneak", &format!("id={} start distance_unreal={dist3:.0}", z.id));
                        }
                        crate::boss::Decision::DelaySneak => {
                            runlog::kv("boss_sneak", &format!("id={} put_off seconds=20", z.id));
                        }
                        crate::boss::Decision::DelayChaingun(wait) => {
                            runlog::kv("boss_chaingun", &format!("id={} put_off seconds={wait:.1}", z.id));
                        }
                        crate::boss::Decision::Nothing => {}
                    }
                }
                z.boss = Some(b);
            }
            let melee = if z.decapitated && !c.headless_melee.is_empty() { &c.headless_melee } else { &c.melee };
            // ZombieSiren.RemoveHead: MeleeRange -500, no more bites.
            let no_melee = c.scream.is_some() && z.decapitated;
            if z.attack.is_none() && dist <= reach && !melee.is_empty() && z.stunned <= 0.0 && !no_melee {
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
                // ZombieFleshPound.PostNetReceive: raging, every attack is
                // FPRageAttack.
                if z.fp_rage.is_some()
                    && let Some(rage_attack) = c.fp_rage_attack
                {
                    seq = rage_attack;
                }
                // ZombieScrake: the first swing enters SawingLoop (charging
                // with ChargeChance 0.5, or 0.7 under half health: Normal
                // difficulty); while sawing the attack is SawImpaleLoop (full
                // body) at MeleeDamage x 0.6, repeated while in reach.
                if let Some(impale) = c.saw_impale {
                    if z.sawing {
                        seq = impale;
                        z.melee_damage = c.melee_damage * 0.6;
                    } else {
                        z.sawing = true;
                        z.raging = false;
                        let roll1 = (z.random() % 1000) as f32 / 1000.0;
                        let roll2 = (z.random() % 1000) as f32 / 1000.0;
                        z.saw_charging = (z.health / z.health_max < 0.5 && roll1 <= 0.7) || roll2 <= 0.5;
                        runlog::kv("scrake_sawing", &format!("id={} sawing=true charging={}", z.id, z.saw_charging));
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
                    ranged: false,
                    fx_fired: 0,
                    shots_fired: 0,
                });
                // ZombieStalker.SetAnimAction: a melee attack uncloaks her.
                if c.cloak_material.is_some() {
                    z.since_uncloak = 0.0;
                    if z.cloaked {
                        z.cloaked = false;
                        z.cloak_dirty = true;
                        runlog::kv("stalker_uncloak", &format!("id={} reason=attack", z.id));
                    }
                }
                runlog::kv(
                    "zed_attack",
                    &format!(
                        "id={} sequence={} layered={layered} charge={charge} length_frames={} rate={}",
                        z.id,
                        c.model.sequence_name(seq).unwrap_or("?"),
                        c.model.length(seq),
                        c.model.rate(seq)
                    ),
                );
            }
            let attacking = z.attack.is_some();
            if let Some(what) = z.update_running(dist, attacking, dt) {
                runlog::kv(
                    "gorefast_run",
                    &format!("id={} running={} reason={what} distance_unreal={dist:.0}", z.id, z.running),
                );
            }
            if z.state == ZedState::Falling {
                // Just left the ground (a pounce): no walking this frame.
            } else if z.attack.is_some_and(|a| !a.layered) {
                // Full-body attack: stand and finish it.
            } else if z.state == ZedState::BossBusy {
                // The Patriarch just started a full-body action (PreFireMG).
            } else {
                // Walking, also during a grab (ZombieClot.Tick keeps
                // accelerating toward the target while attacking).
                z.state = ZedState::Chase;
                // RemoveHead: GroundSpeed x 0.8 once headless; a running
                // Gorefast x 1.875.
                let any_run = z.running || z.raging || z.fp_rage.is_some() || z.boss.is_some_and(|b| b.charge.is_some() || b.escaping());
                if !any_run {
                    z.run_speed_lost = false;
                }
                let speed = if z.zapped() && z.boss.is_some_and(|b| b.charge.is_some() || b.escaping()) {
                    // ZombieBoss Charging / Escaping: "Zapping slows him
                    // down, but doesn't stop him": x 1.5.
                    c.ground_speed * 1.5
                } else if z.zapped() {
                    // SetZappedBehavior: OriginalGroundSpeed x ZappedSpeedMod.
                    c.ground_speed * z.zap.speed_mod
                } else if z.decapitated {
                    c.ground_speed * 0.8
                } else if z.run_speed_lost && any_run {
                    c.ground_speed
                } else if z.running {
                    c.ground_speed * GOREFAST_RUN_SPEED
                } else if z.fp_rage.is_some() {
                    c.ground_speed * FLESHPOUND_RAGE_SPEED
                } else if z.raging {
                    c.ground_speed * SCRAKE_RAGE_SPEED
                } else if z.saw_charging {
                    c.ground_speed * SCRAKE_ATTACK_CHARGE_RATE
                } else if c.scream.is_some() && z.attack.is_some() {
                    // ZombieSiren.Tick: GroundSpeed x 0.65 while attacking.
                    c.ground_speed * 0.65
                } else if z.boss.is_some_and(|b| b.escaping()) {
                    // Escaping.Tick (and the sneak states): x 2.5, normal speed
                    // while attacking.
                    let scale = if z.attack.is_some() { 1.0 } else { crate::boss::CHARGE_SPEED };
                    c.ground_speed * scale
                } else if z.boss.is_some_and(|b| b.charge.is_some()) {
                    // ZombieBoss Charging.Tick: x 2.5, x 1.25 while attacking.
                    let scale = if z.attack.is_some() { crate::boss::CHARGE_ATTACK_SPEED } else { crate::boss::CHARGE_SPEED };
                    c.ground_speed * scale
                } else if z.hidden {
                    // KFMonster.Tick: unseen, SetGroundSpeed(HiddenGroundSpeed).
                    c.hidden_speed
                } else {
                    c.ground_speed
                };
                // KFMonster.TakeDamage on catching fire: GroundSpeed x 0.8
                // (of the current speed, so headless zeds slow down more).
                let speed = if z.burn_down > 0 && !z.zapped() { speed * 0.8 } else { speed };
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
                let (moved, wall) = mover.ground_move(z.centre, delta);
                // KFDoorMover.Bump: a zed walking into a closed or sealed
                // door is told to BreakUpDoor (state DoorBashing). Its loop
                // only runs while the door is sealed, visible and not
                // bZombiesIgnore, so only then does the zed stay.
                if let Some(h) = &wall
                    && let Ok(dc) = door_colliders.get(h.entity)
                    && let Some(d) = doors.doors.get(dc.0)
                    && d.sealed
                    && !d.hidden
                    && !d.info.zombies_ignore
                {
                    // ZombieSiren / ZombieBoss have no DoorBash: their
                    // DoorAttack screams or fires a rocket.
                    if c.door_bash.is_some() || c.scream.is_some() || c.boss.is_some() {
                        z.door_bash = Some(DoorBash { door: dc.0, wait: 0.0, in_anim: false, hits: 0, anim_time: 0.0, distance: false, ranged: false });
                        z.state = ZedState::DoorBashing;
                        z.attack = None;
                        z.overlay = None;
                        z.running = false;
                        runlog::kv("zed_door_bash", &format!("id={} door={} start weld={:.0}", z.id, d.info.name, d.weld));
                        t.translation = z.centre;
                        continue;
                    }
                }
                // KFGlassMover.Bump: the pane takes the zed's speed and its
                // MeleeDamage; HandleBumpGlass: the zed stops and plays
                // MeleeAnims[0] (WaitForAnim).
                if let Some(h) = &wall
                    && let Ok(g) = glass.get(h.entity)
                {
                    glass_bumps.write(crate::glass::GlassBump { pane: g.0, speed, melee: Some(z.melee_damage) });
                    if let Some(&seq) = c.melee.first() {
                        z.state = ZedState::Melee;
                        z.sequence = None;
                        start_anim(&mut z, Some(seq), false);
                        z.attack = Some(Attack { seq, layered: false, hit_done: true, ranged: false, fx_fired: 0, shots_fired: 0 });
                    }
                    runlog::kv("zed_bump_glass", &format!("id={} speed={speed:.0}", z.id));
                    t.translation = z.centre;
                    continue;
                }
                let progress = (moved - z.centre).with_y(0.0).length();
                // Blocked by the level (not a pawn) while heading somewhere:
                // jump it if a jump clears it (native PickWallAdjust jumps
                // obstacles; rule from memory, not the scripts). The jump
                // carries the zed forward at its ground speed.
                let jump = wall.is_some()
                    && blocked.is_none()
                    && progress < 0.3 * delta.length()
                    && z.jump_cooldown <= 0.0
                    && mover
                        .jump_over(z.centre, dir_of(z.yaw) * 2.0 * c.collision_radius * SCALE, crate::nav::JUMP_APEX * SCALE)
                        .is_some();
                if jump {
                    z.state = ZedState::Falling;
                    z.vertical_speed = c.jump_z * SCALE;
                    z.air_velocity = dir_of(z.yaw) * speed * SCALE;
                    z.jump_cooldown = 1.0;
                    let u = z.centre / SCALE;
                    runlog::kv("zed_jump", &format!("id={} at_unreal=({:.0}, {:.0}, {:.0}) jump_z={}", z.id, -u.z, u.x, u.y, c.jump_z));
                } else {
                    match mover.snap_to_floor(moved) {
                        Some(on_floor) => z.centre = on_floor,
                        None => {
                            // Walked off a ledge: fall, keeping the walking speed.
                            z.centre = moved;
                            z.state = ZedState::Falling;
                            z.vertical_speed = 0.0;
                            z.air_velocity = if dt > 0.0 { (moved - old_centre).with_y(0.0) / dt } else { Vec3::ZERO };
                        }
                    }
                }
            }
        } else if !active.0 && z.state != ZedState::Falling {
            z.state = ZedState::Idle;
        }
        z.jump_cooldown = (z.jump_cooldown - dt).max(0.0);
        // ZombieCrawler.Bump: a pouncing Crawler that touches the player
        // hurts it once (MeleeDamage -5% .. +5%).
        if z.pouncing && z.state == ZedState::Falling && walker.is_some() {
            let d = target - z.centre;
            let touching = d.with_y(0.0).length() / SCALE <= c.collision_radius + PLAYER_RADIUS + 2.0
                && (d.y / SCALE).abs() <= c.collision_height + PLAYER_HALF_HEIGHT;
            if touching {
                let roll = (z.random() % 1000) as f32 / 1000.0;
                let amount = z.melee_damage * 0.95 + z.melee_damage * 0.1 * roll;
                player_damage.write(crate::combat::PlayerDamaged {
                        amount,
                        zed_id: z.id,
                        kind: crate::combat::HurtKind::Plain,
                    });
                z.pouncing = false;
                runlog::kv("crawler_pounce_hit", &format!("id={} damage={amount:.1}", z.id));
            }
        }
        // JumpPad.Touch / PostTouch: a pawn touching a pad is thrown with
        // its JumpVelocity (falling), heading for its JumpTarget.
        let touching_pad = nav.jump_pads.iter().position(|p| {
            let d = nav.points[p.point].pos - z.centre;
            d.with_y(0.0).length() / SCALE < crate::nav::JUMP_PAD_RADIUS + c.collision_radius
                && (d.y / SCALE).abs() < crate::nav::JUMP_PAD_HALF_HEIGHT + c.collision_height
        });
        if let Some(i) = touching_pad
            && z.on_pad != Some(i)
            && z.health > 0.0
        {
            let pad = nav.jump_pads[i];
            // Launched from the pad's centre: the editor worked JumpVelocity
            // out for a jump from there. KF launches where the zed first
            // touches (up to 66 units off), which on the KF-WestLondon
            // fence pad hits the fence; whatever native detail gets KF's
            // zeds over is not known, so this is an approximation.
            let pc = nav.points[pad.point].pos;
            z.centre = Vec3::new(pc.x, z.centre.y, pc.z);
            z.state = ZedState::Falling;
            z.vertical_speed = pad.velocity.y;
            z.air_velocity = pad.velocity.with_y(0.0);
            z.yaw = yaw_of(pad.velocity.with_y(0.0));
            z.attack = None;
            runlog::kv(
                "zed_jump_pad",
                &format!("id={} pad={} target={} up_unreal={:.0}", z.id, nav.points[pad.point].name, nav.points[pad.target].name, pad.velocity.y / SCALE),
            );
        }
        z.on_pad = touching_pad;
        if z.state == ZedState::Falling {
            // PHYS_Falling: gravity, the horizontal velocity kept (AirControl
            // 0.05 is not applied), sliding along whatever is hit.
            z.vertical_speed -= GRAVITY * SCALE * dt;
            // Pawns block falling zeds too (sideways part), as walking ones.
            let mut air = z.air_velocity * dt;
            if let Some(me) = z.blocking_cylinder() {
                let shapes: Vec<Cylinder> = blockers.iter().filter(|(e, _)| *e != Some(entity)).map(|(_, c)| *c).collect();
                let (clipped, by) = clip_move(&me, air, &shapes);
                if by.is_some() {
                    z.air_velocity = Vec3::ZERO;
                }
                air = clipped;
            }
            let (moved, hit) = mover.slide(z.centre, air + Vec3::Y * z.vertical_speed * dt);
            z.centre = moved;
            if hit.as_ref().is_some_and(|h| h.normal.y < -0.7) && z.vertical_speed > 0.0 {
                z.vertical_speed = 0.0; // head hit a ceiling
            }
            if hit.is_some_and(|h| h.normal.y > 0.7) {
                let impact = -z.vertical_speed / SCALE;
                z.pouncing = false; // Landed
                z.vertical_speed = 0.0;
                z.air_velocity = Vec3::ZERO;
                // Landed (LandAnims) only after a real fall: faster than half
                // the jump speed (assumed; the engine decides natively). A
                // drop of a few units goes straight back to walking.
                if impact < 0.5 * c.jump_z {
                    z.state = ZedState::Chase;
                } else if c.land_anim.is_some() {
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
                    } else if z.zapped() || (z.burn_down > 0 && z.burn_down < CRISP_UP_THRESHOLD) {
                        // SetZappedBehavior also sets the burning walk.
                        // ZombieCrispUp (BurnDown below CrispUpThreshhold) ->
                        // SetBurningBehavior: MovementAnims[0] = BurningWalkFAnims.
                        c.burning_walk.or(c.walk)
                    } else if z.running {
                        c.run_anim.or(c.walk)
                    } else if z.fp_rage.is_some() {
                        c.fp_charge_walk.or(c.walk)
                    } else if z.raging || z.saw_charging {
                        c.charge_anim.or(c.walk)
                    } else if z.boss.is_some_and(|b| b.charge.is_some() || b.escaping()) && z.attack.is_none() {
                        // ZombieBoss.PostNetReceive: charging, MovementAnims = ChargingAnim.
                        c.boss.as_ref().and_then(|b| b.charge_walk).or(c.walk)
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
            ZedState::Melee | ZedState::KnockedDown | ZedState::Landing | ZedState::Enraging | ZedState::BossBusy | ZedState::DoorBashing | ZedState::Dead => {}
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
    mut vomit: MessageWriter<crate::vomit::SpawnVomit>,
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
        let seed = z.random();
        for effect in std::mem::take(&mut z.pending_fx) {
            notify_effect(&mut commands, &mut meshes, library.as_deref(), c, &mut z, t, &effect, seed);
        }
        // ZombieBoss.AddTraceHitFX: mMuzzleFlash. KF quirk, kept: the first
        // call only spawns and attaches it ("if( mMuzzleFlash==None )
        // Spawn... else SpawnParticle(1)"), so the first shot of his life
        // shows no flash. Later shots: Emitter.SpawnParticle(1), one
        // particle on each of its 8 emitters.
        if z.mg_flash_shots > 0
            && let (Some(bc), Some(lib)) = (c.boss.as_ref(), library.as_deref())
        {
            match z.mg_flash {
                None => {
                    z.mg_flash_shots -= 1;
                    if let Some(bone) = bc.tip_bone {
                        let anchor = EffectAnchor::Bone {
                            bone,
                            offset: Vec3::ZERO,
                            rotation: Mat3::IDENTITY,
                        };
                        let frame = anchor_frame(c, &z, t, &anchor).unwrap_or((ue_pos(z.centre), Mat3::IDENTITY));
                        let options = crate::particles::SpawnOptions {
                            persistent: true,
                            ..default()
                        };
                        if let Some(e) = crate::particles::spawn_effect_with(&mut commands, lib, &mut meshes, "ROEffects.MuzzleFlash3rdMG", frame.0, frame.1, seed, options) {
                            z.mg_flash = Some(e);
                            z.effects.push((e, anchor));
                        }
                    }
                }
                Some(e) => {
                    if let Ok(mut fx) = effects.get_mut(e) {
                        for _ in 0..std::mem::take(&mut z.mg_flash_shots) {
                            fx.spawn_all(1);
                        }
                    }
                }
            }
        }
        // Dead: the flash goes once its particles are gone.
        if z.state == ZedState::Dead
            && let Some(e) = z.mg_flash.take()
            && let Ok(mut fx) = effects.get_mut(e)
        {
            fx.kill();
        }
        // ZombieBloat.PlayDyingAnimation and Tick: unless he bled out
        // headless he bursts: BileExplosion (BileExplosionHeadless without a
        // head) half his height up, SpineBone2 hidden, and BileBomb's BileJet
        // throws 4 globs up (Rotator(-Gravity) turned by Pitch 2000 and a
        // random yaw).
        if z.state == ZedState::Dead && c.burst_bone.is_some() && !z.burst_done {
            z.burst_done = true;
            if !z.bled_out {
                if let Some(b) = c.burst_bone {
                    z.hidden_bones.push(b);
                }
                let at = ue_pos(z.centre) + Vec3::Z * (0.5 * c.collision_height);
                let class = if z.decapitated { "KFMod.BileExplosionHeadless" } else { "KFMod.BileExplosion" };
                let k = std::f32::consts::TAU / 65536.0;
                let axes = coords::ue_rotation_matrix(Rotator { pitch: 0, yaw: z.yaw as i32, roll: 0 });
                let spawned = library
                    .as_deref()
                    .and_then(|lib| particles::spawn_effect(&mut commands, lib, &mut meshes, class, at, axes, seed));
                let tilt = 2000.0 * k;
                for _ in 0..4 {
                    let yaw = (z.random() % 65536) as f32 * k;
                    let dir = Vec3::new(tilt.sin() * yaw.cos(), tilt.sin() * yaw.sin(), tilt.cos());
                    vomit.write(crate::vomit::SpawnVomit {
                        at: ue_pos(z.centre),
                        velocity: dir * crate::vomit::SPEED,
                        zed_id: z.id,
                    });
                }
                runlog::kv(
                    "bloat_burst",
                    &format!("id={} effect={class} spawned={} at_unreal=({:.0}, {:.0}, {:.0})", z.id, spawned.is_some(), at.x, at.y, at.z),
                );
            }
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
            if let (Some(seq), Some(root)) = (seq, c.flinch_root.or(c.spine_bone)) {
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

/// Swaps a Stalker's materials when her cloak changes.
fn apply_cloaks(mut commands: Commands, classes: Option<Res<ZedClasses>>, mut zeds: Query<(&mut Zed, &ZedParts)>) {
    let Some(classes) = classes else {
        return;
    };
    for (mut z, parts) in &mut zeds {
        if !z.cloak_dirty {
            continue;
        }
        z.cloak_dirty = false;
        let c = &classes.0[z.class];
        for (i, &e) in parts.0.iter().enumerate() {
            let material = match (&c.cloak_material, z.cloaked, &c.fp_red_device) {
                (Some(cloak), true, _) => cloak.clone(),
                (_, true, _) if !c.cloak_parts.is_empty() => c.cloak_parts[i.min(c.cloak_parts.len() - 1)].clone(),
                // DeviceGoRed while raging (Skins[1]).
                (_, _, Some(red)) if i == 1 && (z.fp_rage.is_some() || z.state == ZedState::Enraging) => red.clone(),
                _ => c.model.parts[i].material.clone(),
            };
            commands.entity(e).insert(MeshMaterial3d(material));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zap_builds_up_lasts_and_raises_the_threshold() {
        let mut z = Zed::test_clot();
        // Under ZapThreshold (0.25): not zapped; the zap fades after 0.1 s.
        z.set_zapped(0.2);
        assert!(!z.zapped());
        z.zap_tick(0.05);
        assert!((z.total_zap - 0.2).abs() < 1e-6, "no fade within 0.1 s");
        z.zap_tick(0.1);
        assert!((z.total_zap - 0.1).abs() < 1e-6, "{}", z.total_zap);
        // Reaching it: zapped for ZapDuration (4 s).
        z.set_zapped(0.15);
        assert!(z.zapped());
        assert!(!z.zap_tick(3.9));
        // Zapped again: back to the full 4 s.
        z.set_zapped(0.01);
        assert!((z.remaining_zap - 4.0).abs() < 1e-6);
        assert!(!z.zap_tick(3.9));
        assert!(z.zap_tick(0.2), "wears off");
        // ZapResistanceScale 2: the next zap needs 0.5.
        assert!((z.zap_threshold - 0.5).abs() < 1e-6);
    }

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

    #[test]
    fn solo_damage_is_three_quarters_rounded_down() {
        assert_eq!(solo_damage(6.0), 4.0); // Clot claw: 4.5 -> 4
        assert_eq!(solo_damage(8.0), 6.0); // Siren scream
        assert_eq!(solo_damage(1.0), 1.0); // at least 1
    }
}
