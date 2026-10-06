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

use crate::engine::camera::FlyCamera;
use crate::zeds::gore::{self, GoreAssets, PieceModel, StumpKind};
use crate::render::decals::{DecalKind, SpawnDecal};
use crate::render::particles::{self, EffectLibrary, ParticleEffect};
use crate::engine::coords::{self, SCALE};
use crate::world::map::MapRequest;
use crate::player::pawn_collision::{Cylinder, clip_move};
use crate::zeds::ragdoll::{self, Launch, MeshFrame, RagdollBody, RagdollDef, RagdollState};
use crate::engine::runlog;
use crate::render::skinned::{SkinnedModel, Skins};
use crate::player::walk::{Mover, Walker};

mod load;
mod sounds;
mod spawn;
mod effects;
mod attacks;
mod boss_ai;
mod think;
mod animate;
mod methods;
use load::*;
use sounds::*;
use spawn::*;
use effects::*;
use attacks::*;
use boss_ai::*;
use think::*;
use animate::*;

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
/// A zed class's sounds (KFMonster and the specimen's _STANDARD class;
/// see DESIGN.md, "Sound and music", S4b), as full object paths.
#[derive(Clone, Debug, Default)]
struct ZedSounds {
    /// MoanVoice (ZombieMoan: SLOT_Misc, MoanVolume, radius 250).
    moan: Option<String>,
    moan_volume: f32,
    /// HitSound[0] (PlayTakeHit: SLOT_Pain, 1.25, radius 400; the
    /// Patriarch 2 x TransientSoundVolume).
    pain: Option<String>,
    pain_volume: f32,
    /// Pain also from fire damage (ZombieFleshPound, ZombieScrake and
    /// ZombieBoss override PlayTakeHit without KFMonster's fire check).
    pain_on_fire: bool,
    /// DeathSound[0], HeadlessDeathSound; ZombieBloat: Bloat_DeathPop at
    /// 2.0 unless headless.
    death: Option<String>,
    death_volume: f32,
    headless_death: Option<String>,
    decapitation: Option<String>,
    /// ChallengeSound[0..3] (Monster.PlayChallengeSound: SLOT_Talk, the
    /// TransientSound defaults 1.0 and 500).
    challenge: Vec<String>,
    /// MeleeAttackHitSound (ClawDamageTarget: SLOT_Interact, 2.0; the
    /// Fleshpound 1.25).
    melee_hit: Option<String>,
    melee_hit_volume: f32,
    /// AmbientSound with SoundVolume, SoundRadius x AmbientSoundScaling
    /// (a guess at how the native code uses the scaling).
    ambient: Option<AmbientLoop>,
    /// ZombieScrake: SawAttackLoopSound while in SawingLoop, and
    /// ChainSawOffSound when he dies.
    saw_loop: Option<String>,
    chainsaw_off: Option<String>,
    /// ZombieBoss: RocketFireSound, MeleeImpaleHitSound, MiniGunFireSound,
    /// MiniGunSpinSound.
    rocket_fire: Option<String>,
    impale_hit: Option<String>,
    mg_fire: Option<String>,
    mg_spin: Option<String>,
}

#[derive(Clone, Debug)]
struct AmbientLoop {
    sound: String,
    volume: u8,
    radius: f32,
}

/// Sound events a zed collected this frame (played by `animate_zeds`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZedSound {
    Pain,
    Death,
    Decapitation,
    /// KFMonster.TakeDamage: a headshot on a zed that keeps its head.
    Skull,
    MeleeHit,
    Moan,
    Challenge,
    /// A sound written in the script (the Patriarch's speech, Kev_SaveMe),
    /// with its PlaySound arguments.
    Line(Line),
    /// ZombieBoss: RocketFireSound (SLOT_Interact, 2.0, TransientSoundRadius).
    Rocket,
    /// ZombieBoss.ClawDamageTarget during MeleeImpale: MeleeImpaleHitSound.
    ImpaleHit,
    /// xPawn.Landed: GetSound(EST_Land) (Player_LandDirt for zeds),
    /// SLOT_Interact, min(1, -0.3 x Velocity.Z / JumpZ).
    Land(f32),
}

/// One PlaySound call written out in a zed script.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub sound: &'static str,
    pub slot: crate::audio::mixer::Slot,
    pub volume: f32,
    pub radius: f32,
    pub no_override: bool,
}

struct ZedClass {
    kind: ZedKind,
    sounds: ZedSounds,
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
    /// ScoringValue (dosh for the kill, before scaling).
    scoring_value: f32,
    head_health: f32,
    /// HeadRadius x HeadScale.
    head_radius: f32,
    /// HeadHeight x HeadScale: head centre offset along the head bone's X axis.
    head_offset: f32,
    head_bone: Option<usize>,
    melee_damage: f32,
    /// ZombieDamType (the three entries are the same for every zed):
    /// DamTypeSlashingAttack for the Stalker and Siren, ZombieMeleeDamage
    /// for the rest.
    melee_dam_type: crate::game::combat::DamType,
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
    /// Siren: DoShakeEffect's view shake and blur settings.
    scream_shake: Option<crate::player::hit_cam::ScreamShake>,
    /// Patriarch: his own attacks (`boss.rs`).
    boss: Option<crate::zeds::boss::BossClass>,
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
    router: crate::world::nav::Router,
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
    pub(crate) fire_class: crate::game::combat::FireType,
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
    boss: Option<crate::zeds::boss::BossState>,
    /// DoBossDeath: the controller went to state GameEnded and is gone.
    /// The body stays (KFMonster.TurnOff does nothing), can still be shot,
    /// but no longer thinks, moves or attacks, and is not a monster for
    /// the wave count.
    pub braindead: bool,
    /// The class's ScoringValue; Died with the player as Killer; the
    /// kill's dosh already paid (dosh.rs).
    pub scoring_value: f32,
    pub killed_by_player: bool,
    pub kill_paid: bool,
    /// The killing hit was a headshot (KFMonster.TakeDamage's DramaticEvent).
    pub headshot_kill: bool,
    /// Zed time's kill rolls done for this death (zed_time.rs).
    pub zed_time_rolled: bool,
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
    /// How far each animation layer's sound notifies have played: (main
    /// sequence, frame) and (upper-body sequence, frame).
    sounds_heard: (Option<usize>, f32),
    overlay_sounds_heard: Option<(usize, f32)>,
    /// Sound events of this frame (`ZedSound`), played by `animate_zeds`.
    pub sound_events: Vec<ZedSound>,
    /// KFMonsterController MoanTime (game seconds, whole: an int in KF);
    /// negative until the first is set.
    moan_at: f32,
    /// Seconds since the last pain sound (xPawn LastPainSound).
    since_pain_sound: f32,
    /// The last hit was fire damage (no pain sound for most zeds).
    pub hit_by_fire: bool,
    /// ChallengeTime (game seconds) and the next sight check.
    last_challenge: f32,
    challenge_check: f32,
    /// The AmbientSound now on the entity, if any.
    ambient_on: Option<String>,
    /// ZedSounds.pain_on_fire, copied from the class.
    pain_on_fire: bool,
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

/// Where an attached effect sits: a mesh tag, or a bone with an offset and
/// rotation relative to it (AnimNotify_Effect's OffsetLocation and
/// OffsetRotation, Unreal units and axes).
#[derive(Clone, Copy, Debug)]
pub enum EffectAnchor {
    Tag(&'static str),
    Bone { bone: usize, offset: Vec3, rotation: Mat3 },
}

fn sees(spatial: &SpatialQuery, a: Vec3, b: Vec3) -> bool {
    let Ok(d) = Dir3::new(b - a) else {
        return true;
    };
    spatial.cast_ray(a, d, (b - a).length(), true, &crate::world::collision::world_filter()).is_none()
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
            .add_message::<BossAction>()
            .add_systems(Update, (spawn_zeds, boss_actions, think_and_move, burn_zeds, animate_zeds, apply_cloaks).chain());
    }
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

/// What the wave game asks of the Patriarch (KFGameType's boss-wave
/// Timer: MakeGrandEntry; MatchOver: SetBossLaught).
#[derive(Message, Clone, Copy, Debug)]
pub enum BossAction {
    Entrance(usize),
    Laugh(usize),
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
