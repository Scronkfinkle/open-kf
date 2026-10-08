//! The player's flying projectiles: shotgun pellets (ShotgunBullet and its
//! subclasses) and nails (NailGunProjectile). They fly in a straight line at
//! Speed (nails fall after their first bounce), pass through zeds losing
//! damage, and stop at walls (nails bounce first). Grenades and rockets come
//! in W6 and will reuse this.
//!
//! Positions and velocities are Unreal units; collision tests convert to
//! Bevy space.

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::engine::coords::{self, SCALE};
use crate::engine::runlog;
use crate::zeds::zed::Zed;

/// PhysicsVolume gravity (Unreal units/s^2).
const GRAVITY: f32 = 950.0;

/// What a bouncing object (bBounce: frag, fire / medic nade, pipe bomb, a
/// nail after its first bounce) really falls at in KF: the engine's
/// falling step adds only half of gravity x dt to such an object's
/// velocity (the full-gravity correction it applies to everything else is
/// skipped for bouncers), and never caps its speed. See DESIGN.md, "Combat
/// physics fixes", CP-1.
const BOUNCE_GRAVITY: f32 = 0.5 * GRAVITY;

/// One falling step of a bouncing object, as KF does it: velocity first,
/// then the move with the new velocity. Returns the step to move by.
fn bouncer_fall_step(vel: &mut Vec3, dt: f32) -> Vec3 {
    vel.z -= BOUNCE_GRAVITY * dt;
    *vel * dt
}

/// PhysicsVolume TerminalVelocity: KF caps the speed of falling things that
/// do not bounce at this, every falling step.
const TERMINAL_VELOCITY: f32 = 2500.0;

/// The longest falling step KF's engine takes (longer frames are split).
const MAX_FALL_STEP: f32 = 0.05;

/// Falling for things that do not bounce (an M79 grenade out of propellant,
/// a dud), as KF's engine does it: move with the velocity at mid-step
/// (full gravity, 950), then cap the speed at TerminalVelocity. Returns
/// the whole move (frames over 0.05 s are split as KF does).
fn fall_step(vel: &mut Vec3, dt: f32) -> Vec3 {
    let mut step = Vec3::ZERO;
    let mut left = dt;
    while left > 1e-6 {
        let h = if left <= MAX_FALL_STEP { left } else { (left * 0.5).min(MAX_FALL_STEP) };
        left -= h;
        step += (*vel - Vec3::Z * (0.5 * GRAVITY * h)) * h;
        vel.z -= GRAVITY * h;
        if vel.length() > TERMINAL_VELOCITY {
            *vel = vel.normalize() * TERMINAL_VELOCITY;
        }
    }
    step
}

/// ROBallisticProjectile's "true ballistics" (the M79, M32 and M203
/// grenades; the LAW family switches it off). Values from the class
/// defaults.
#[derive(Clone, Copy, Debug)]
pub struct Ballistics {
    /// 1 / BallisticCoefficient.
    pub bc_inverse: f32,
    /// SpeedFudgeScale, MinFudgeScale, InitialAccelerationTime: the
    /// projectile's speed and movement are scaled from MinFudgeScale up to
    /// SpeedFudgeScale over its first InitialAccelerationTime seconds.
    pub speed_fudge: f32,
    pub min_fudge: f32,
    pub accel_time: f32,
}

/// The drag coefficient KF's engine uses for true-ballistics projectiles
/// (a standard G1 table): (lowest Mach number, coefficient), fastest
/// first; under Mach 0.05 it is 0.2629. Read from the engine (details in
/// the local RE.md).
const G1_TABLE: [(f32, f32); 78] = [
    (5.0, 0.4988), (4.8, 0.4990), (4.6, 0.4992), (4.4, 0.4995), (4.2, 0.4998), (4.0, 0.5006),
    (3.9, 0.5010), (3.8, 0.5016), (3.7, 0.5022), (3.6, 0.5030), (3.5, 0.5040), (3.4, 0.5054),
    (3.3, 0.5067), (3.2, 0.5084), (3.1, 0.5105), (3.0, 0.5133), (2.9, 0.5168), (2.8, 0.5211),
    (2.7, 0.5264), (2.6, 0.5325), (2.5, 0.5397), (2.45, 0.5438), (2.4, 0.5481), (2.35, 0.5527),
    (2.3, 0.5577), (2.25, 0.5630), (2.2, 0.5685), (2.15, 0.5743), (2.1, 0.5804), (2.05, 0.5867),
    (2.0, 0.5934), (1.95, 0.6003), (1.9, 0.6072), (1.85, 0.6141), (1.8, 0.6210), (1.75, 0.6280),
    (1.7, 0.6347), (1.65, 0.6413), (1.6, 0.6474), (1.55, 0.6528), (1.5, 0.6573), (1.45, 0.6607),
    (1.4, 0.6625), (1.35, 0.6621), (1.3, 0.6589), (1.25, 0.6518), (1.2, 0.6393), (1.15, 0.6191),
    (1.125, 0.6053), (1.1, 0.5883), (1.075, 0.5677), (1.05, 0.5427), (1.025, 0.5136), (1.0, 0.4805),
    (0.975, 0.4448), (0.95, 0.4084), (0.925, 0.3734), (0.9, 0.3415), (0.875, 0.3136), (0.85, 0.2901),
    (0.825, 0.2706), (0.8, 0.2546), (0.775, 0.2417), (0.75, 0.2313), (0.725, 0.2230), (0.7, 0.2165),
    (0.6, 0.2034), (0.55, 0.2020), (0.5, 0.2032), (0.45, 0.2061), (0.4, 0.2104), (0.35, 0.2155),
    (0.3, 0.2214), (0.25, 0.2278), (0.2, 0.2344), (0.15, 0.2413), (0.1, 0.2487), (0.05, 0.2558),
];

fn g1(mach: f32) -> f32 {
    G1_TABLE.iter().find(|(m, _)| mach >= *m).map_or(0.2629, |(_, cd)| *cd)
}

/// One step of KF's true-ballistics flight (before the propellant runs
/// out). `flight` is the time flown so far (updated). The engine works in
/// feet (18.4 units per foot): drag = (speed in ft/s)^2 x G1(Mach) /
/// BallisticCoefficient x dt x 0.00384 taken off the speed (as the engine
/// does, without converting back to units), gravity 591.45 units/s^2
/// (32.144 ft/s^2), both and the move scaled by the start-up fudge.
fn ballistic_step(vel: &mut Vec3, flight: &mut f32, b: &Ballistics, dt: f32) -> Vec3 {
    let fudge = if *flight < b.accel_time {
        (b.speed_fudge - b.min_fudge) / b.accel_time * *flight + b.min_fudge
    } else {
        b.speed_fudge
    };
    *flight += dt;
    let v_fps = vel.length() * 0.0543;
    let mach = v_fps * 0.000_895_824_6;
    let drag = v_fps * v_fps * g1(mach) * b.bc_inverse * dt * 0.003_840_841;
    *vel -= vel.normalize_or_zero() * drag * fudge;
    vel.z -= dt * 591.4496 * fudge;
    *vel * dt * fudge
}

/// A projectile class's values (from its defaults).
#[derive(Clone, Copy, Debug, Default)]
pub struct ProjectileStats {
    /// The projectile class (for its model, if it has one).
    pub class: &'static str,
    /// Speed (Unreal units/s).
    pub speed: f32,
    pub damage: f32,
    /// ShotgunBullet.ProcessTouch: damage x PenDamageReduction per zed; gone
    /// once damage / default damage <= PenDamageReduction / MaxPenetrations.
    pub max_penetrations: f32,
    pub pen_damage_reduction: f32,
    /// The projectile's HeadShotDamageMult (ProcessTouch).
    pub headshot_mult: f32,
    /// The damage type's HeadShotDamageMult (KFMonster.TakeDamage applies it
    /// again on headshots: a KF quirk, kept).
    pub damage_type_headshot_mult: f32,
    pub life_span: f32,
    /// NailGunProjectile Bounces (0 for pellets).
    pub bounces: u32,
    /// How it goes through zeds.
    pub rule: PenRule,
    /// CrossbowArrow: stuck in a wall it can be picked up (+1 bolt).
    pub pickup: bool,
    /// The damage type burns (TrenchgunBullet; W7).
    pub fire: Option<crate::game::combat::FireType>,
    /// MyDamageType, for the perks.
    pub dam: Option<crate::game::perks::DamType>,
}

/// How a projectile passes through zeds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum PenRule {
    /// ShotgunBullet.ProcessTouch: damage x PenDamageReduction per zed,
    /// gone at PenDamageReduction / MaxPenetrations of the start.
    #[default]
    Pellet,
    /// CrossbowArrow / M99Bullet.ProcessTouch: damage / 1.25 and speed x
    /// 0.85 per zed, through every zed; sticks in the wall.
    Bolt,
}

/// A stuck Crossbow bolt (CrossbowArrow state OnWall): touching it (within
/// 25 units) picks it up if the Crossbow has room (ProcessTouch).
#[derive(Component)]
struct StuckBolt {
    pos: Vec3,
    life: f32,
    id: u32,
}

/// Whether the Crossbow can take a bolt back (weapon.rs sets it).
#[derive(Resource, Default)]
pub struct BoltRoom(pub bool);

/// A bolt picked up: the weapon code adds one round.
#[derive(Message, Clone, Copy, Debug)]
pub struct BoltPickedUp;

/// A grenade or rocket's values (M79GrenadeProjectile family, LAWProj).
#[derive(Clone, Copy, Debug)]
pub struct ExplosiveStats {
    /// The projectile class (for its model).
    pub class: &'static str,
    pub sounds: ProjectileSounds,
    pub speed: f32,
    /// HurtRadius: Damage, DamageRadius, MomentumTransfer.
    pub damage: f32,
    pub radius: f32,
    pub momentum: f32,
    /// ImpactDamage and its damage type's HeadShotDamageMult: what a zed
    /// takes from a dud (touched closer than ArmDist to the player).
    pub impact_damage: f32,
    pub impact_headshot_mult: f32,
    /// HuskGunProjectile.ProcessTouch: every zed touched takes ImpactDamage
    /// (x this HeadShotDamageMult on a headshot) before the explosion.
    pub impact_on_touch: Option<f32>,
    /// The blast's damage type burns (DamTypeHuskGun).
    pub fire: Option<crate::game::combat::FireType>,
    /// MyDamageType (the blast) and ImpactDamageType (a dud or the Husk
    /// Gun's touch), for the perks.
    pub dam: Option<crate::game::perks::DamType>,
    pub impact_dam: Option<crate::game::perks::DamType>,
    /// ZEDMKIISecondaryProjectile.HurtRadius: zaps (SetZapped(ZapAmount))
    /// every living zed in the radius instead of hurting anything.
    pub zap: Option<f32>,
    /// The blast hurts the player (HuskGunProjectile.HurtRadius skips the
    /// Instigator; the LAW's and M79's do not).
    pub hurts_self: bool,
    /// sqrt(ArmDistSquared).
    pub arm_dist: f32,
    /// StraightFlightTime: flies straight this long, then falls (M79
    /// family; None = straight until it hits, the LAW).
    pub straight_time: Option<f32>,
    /// True ballistics while the propellant lasts (M79 family); None:
    /// straight at constant speed (LAWProj family).
    pub ballistics: Option<Ballistics>,
    pub life_span: f32,
    /// ZombieFleshPound.TakeDamage's multiplier for this damage type, if it
    /// is in its explosives list (None: the small-arms rule).
    pub fleshpound_mult: Option<f32>,
    /// Explode: the effect and decal.
    pub effect: &'static str,
    pub decal: crate::render::decals::DecalKind,
    /// The trail: PanzerfaustTrail (turned backward), or the Husk Gun's
    /// FlameThrowerHusk_*.
    pub trail: Option<&'static str>,
}

/// A projectile class's sounds (S5c), as full object paths; see DESIGN.md,
/// "Sound and music".
#[derive(Clone, Copy, Debug, Default)]
pub struct ProjectileSounds {
    /// AmbientSound while flying, with SoundVolume and SoundRadius
    /// (Projectile's SoundVolume default is 0: no loop).
    pub flight: Option<(&'static str, u8, f32)>,
    /// ExplodeSounds (one at random) or ExplosionSound, at its volume
    /// (2.0, or ExplosionSoundVolume) and TransientSoundRadius.
    pub explode: &'static [&'static str],
    pub explode_volume: f32,
    pub explode_radius: f32,
    /// Nade / PipeBombProjectile ImpactSound (bounces faster than 50) at
    /// TransientSoundVolume, SLOT_Misc.
    pub bounce: Option<&'static str>,
    pub bounce_volume: f32,
    /// PipeBombProjectile.BeepSound.
    pub beep: Option<&'static str>,
    /// LAWProj / M79GrenadeProjectile duds: PTRD_deflect04 at 2.0.
    pub dud: Option<&'static str>,
    /// DisintegrateSound (or DisintegrateSoundRef): a Siren's scream
    /// destroyed it (Disintegrate: PlaySound(DisintegrateSound,, 2.0)).
    pub disintegrate: Option<&'static str>,
}

impl ProjectileSounds {
    fn pick_explosion(&self, roll: u32) -> Option<&'static str> {
        (!self.explode.is_empty()).then(|| self.explode[roll as usize % self.explode.len()])
    }
}

/// Fire a projectile (KFShotgunFire.SpawnProjectile).
#[derive(Message, Clone, Copy, Debug)]
pub struct SpawnPlayerProjectile {
    /// Unreal world: StartProj.
    pub origin: Vec3,
    /// StartTrace (the eye): if a wall is between it and `origin`, the
    /// projectile starts at the wall (KFShotgunFire.DoFireEffect).
    pub trace_from: Vec3,
    pub dir: Vec3,
    pub stats: ProjectileStats,
    pub weapon: &'static str,
    /// Where its tracer starts (the weapon's tip); None: no tracer.
    pub tracer_start: Option<Vec3>,
    /// A grenade or rocket instead of a pellet (`stats` unused).
    pub explosive: Option<ExplosiveStats>,
    /// A thrown frag or pipe bomb instead (`stats` unused).
    pub thrown: Option<ThrownStats>,
    /// A Flamethrower flame instead (`stats` gives only its damage type).
    pub flame: Option<FlameStats>,
    /// A medic dart instead.
    pub dart: Option<DartStats>,
    /// Added to the velocity (FragFire: the player's forward speed).
    pub extra_speed: f32,
}

/// One frame of the ZED Gun's beam (ZEDGunAltFire.ModeTick), Unreal units.
#[derive(Message, Clone, Copy, Debug)]
pub struct BeamZap {
    pub start: Vec3,
    pub dir: Vec3,
    pub range: f32,
    /// The frame time: SetZapped(dt) on the zed the beam touches.
    pub dt: f32,
    /// bDoHit (a FireRate tick): the splash zaps the zeds near the end.
    pub do_hit: bool,
    pub sphere_radius: f32,
    pub sphere_amount: f32,
}

/// Where the beam is this frame, for drawing (Unreal units).
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct BeamView {
    pub active: bool,
    pub start: Vec3,
    pub end: Vec3,
}

/// A HealingProjectile's values (medic gun alt fire).
#[derive(Clone, Copy, Debug)]
pub struct DartStats {
    /// The class (for its model, KF_pickups2_Trip.MP7_Dart).
    pub class: &'static str,
    pub speed: f32,
    pub life_span: f32,
    /// HealBoostAmount: what it gives a teammate it touches (x the
    /// shooter's GetHealPotency; healing.rs).
    pub heal: f32,
    /// AmbientSound while flying (MP7_DartFlyLoop, 128, 250).
    pub flight: Option<(&'static str, u8, f32)>,
}

/// A projectile's flight loop (AmbientSound, SoundVolume, SoundRadius).
fn flight_sound(flight: Option<(&'static str, u8, f32)>) -> Option<crate::audio::mixer::AmbientSound> {
    flight.map(|(sound, volume, radius)| crate::audio::mixer::AmbientSound { sound: sound.into(), volume, radius, ..default() })
}

/// One sound at an Unreal-space point.
fn sound_at(sound: &'static str, at: Vec3) -> crate::audio::mixer::PlaySound {
    crate::audio::mixer::PlaySound::new(sound, crate::audio::mixer::Emitter::Point(coords::pos(at.to_array())))
}

/// A dart in flight. HealingProjectile flies straight at Speed (its
/// native "true ballistics" for the first 0.1 s are not reproduced;
/// after them it is a plain PHYS_Projectile). It heals only a player it
/// touches; a zed or the level makes it Explode: a ROBulletHitEffect and
/// nothing else (its HurtRadius is empty), so it never hurts zeds.
#[derive(Component)]
struct PlayerDart {
    pos: Vec3,
    vel: Vec3,
    stats: DartStats,
    weapon: &'static str,
    age: f32,
    id: u32,
}

/// A FlameTendril's values (the Flamethrower).
#[derive(Clone, Copy, Debug)]
pub struct FlameStats {
    /// Speed; PostBeginPlay adds TossZ upward; PHYS_Falling.
    pub speed: f32,
    pub toss_z: f32,
    /// Explode: HurtRadius(Damage, DamageRadius, DamTypeBurned, 0).
    pub damage: f32,
    pub radius: f32,
    pub life_span: f32,
}

/// FlameTendril's Timer: every 0.2 s the speed is set back to Speed; on the
/// second it explodes (no perk: TimerRunCount >= 2).
const FLAME_TIMER: f32 = 0.2;
const FLAME_TIMER_RUNS: u32 = 2;

/// FlameTendril's trail (FlameThrowerFlameB; the HitFlame xEmitter trail,
/// FlameThrowerFlame, is not drawn: xEmitters are not supported) and
/// Explode's FuelFlame, which Kills itself on its first Timer (1 s: it
/// has no Parent).
const FLAME_TRAIL: &str = "KFMod.FlameThrowerFlameB";
const FUEL_FLAME: &str = "KFMod.FuelFlame";
const FUEL_FLAME_TIME: f32 = 1.0;

#[derive(Component)]
struct PlayerFlame {
    pos: Vec3,
    vel: Vec3,
    stats: FlameStats,
    fire: Option<crate::game::combat::FireType>,
    /// MyDamageType (DamTypeBurned), for the perks.
    dam: Option<crate::game::perks::DamType>,
    weapon: &'static str,
    timer: f32,
    runs: u32,
    age: f32,
    trail: Option<Entity>,
    id: u32,
}

/// An effect to Kill after a time (FuelFlame).
#[derive(Component)]
struct KillEffectAfter {
    effect: Entity,
    time: f32,
}

#[derive(Component)]
struct PlayerExplosive {
    pos: Vec3,
    vel: Vec3,
    stats: ExplosiveStats,
    weapon: &'static str,
    age: f32,
    /// FlightTime: time flown under true ballistics (its start-up fudge).
    flight: f32,
    falling: bool,
    /// bDud: armed too close; falls and vanishes a second later.
    dud: Option<f32>,
    trail: Option<Entity>,
    id: u32,
}

#[derive(Component)]
struct PlayerProjectile {
    pos: Vec3,
    vel: Vec3,
    damage: f32,
    stats: ProjectileStats,
    weapon: &'static str,
    /// Zeds already hit (each once).
    hit: Vec<usize>,
    bounces_left: u32,
    falling: bool,
    age: f32,
    id: u32,
}

/// Projectile classes whose models are drawn (their StaticMesh or
/// StaticMeshRef): grenades, the LAW rocket, the frag, the pipe bomb and
/// nails. Pellets and the M99 bullet are tiny and fast (their tracers show
/// them); the Crossbow bolt is a skeletal mesh (not drawn yet).
const MODEL_CLASSES: [&str; 19] = [
    "KFMod.ZEDGunProjectile",
    "KFMod.ZEDMKIIPrimaryProjectile",
    "KFMod.ZEDMKIISecondaryProjectile",
    "KFMod.MP7MHealinglProjectile",
    "KFMod.MP5MHealinglProjectile",
    "KFMod.M7A3MHealinglProjectile",
    "KFMod.KrissMHealingProjectile",
    "KFMod.HuskGunProjectile",
    "KFMod.HuskGunProjectile_Weak",
    "KFMod.HuskGunProjectile_Strong",
    "KFMod.M79GrenadeProjectile",
    "KFMod.M32GrenadeProjectile",
    "KFMod.M203GrenadeProjectile",
    "KFMod.LAWProj",
    "KFMod.Nade",
    "KFMod.FlameNade",
    "KFMod.MedicNade",
    "KFMod.PipeBombProjectile",
    "KFMod.NailGunProjectile",
];

#[derive(Resource, Default)]
struct ProjectileModels(Vec<(String, crate::zeds::gore::PieceModel)>);

impl ProjectileModels {
    fn get(&self, class: &str) -> Option<&crate::zeds::gore::PieceModel> {
        self.0.iter().find(|(c, _)| c.eq_ignore_ascii_case(class)).map(|(_, m)| m)
    }
}

fn load_models(
    request: Res<crate::world::map::MapRequest>,
    mut models: ResMut<ProjectileModels>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    use ue_assets::properties::Value;
    let set = ue_assets::package_set::PackageSet::new(&request.install_root);
    let defaults = ue_assets::class_defaults::ClassDefaults::new(&set);
    for class_path in MODEL_CLASSES {
        let loaded = crate::zeds::gore::find_class(&set, class_path).ok_or_else(|| "class not found".to_string()).and_then(|class| {
            match defaults.get(&class, "StaticMeshRef") {
                Some((Value::Str(path), _)) if !path.is_empty() => crate::zeds::gore::load_piece_with_mesh(
                    &set,
                    &defaults,
                    &class,
                    &path,
                    class_path,
                    &mut meshes,
                    &mut images,
                    &mut materials,
                ),
                _ => crate::zeds::gore::load_piece(&set, &defaults, &class, class_path, &mut meshes, &mut images, &mut materials),
            }
        });
        match loaded {
            Ok(m) => {
                runlog::kv("projectile_model_loaded", &format!("class={class_path} draw_scale={}", m.draw_scale()));
                models.0.push((class_path.to_string(), m));
            }
            Err(e) => runlog::kv("projectile_model_error", &format!("class={class_path} error=\"{e}\"")),
        }
    }
}

/// The Bevy transform of a projectile at `pos` (Unreal) pointing along `dir`.
fn pose(pos: Vec3, dir: Vec3, scale: f32) -> Transform {
    let d = dir.normalize_or(Vec3::X);
    let k = 65536.0 / std::f32::consts::TAU;
    let rot = ue_assets::properties::Rotator {
        pitch: (d.z.clamp(-1.0, 1.0).asin() * k) as i32,
        yaw: (d.y.atan2(d.x) * k) as i32,
        roll: 0,
    };
    Transform {
        translation: coords::pos(pos.to_array()),
        rotation: coords::rotation(rot),
        scale: Vec3::splat(scale),
    }
}

/// Gives a projectile entity a transform and, if its class has one, its
/// model (as children).
fn attach_model(commands: &mut Commands, models: &ProjectileModels, entity: Entity, class: &str, pos: Vec3, dir: Vec3) {
    let scale = models.get(class).map_or(1.0, |m| m.draw_scale());
    commands.entity(entity).insert((pose(pos, dir, scale), Visibility::Visible, BodyScale(scale)));
    if let Some(m) = models.get(class) {
        m.spawn_parts(commands, entity);
    }
}

/// The model's DrawScale, kept for updating the transform.
#[derive(Component)]
struct BodyScale(f32);

/// Keeps each projectile's transform on its position, pointing along its
/// velocity (at rest: as it was).
#[allow(clippy::type_complexity)] // Bevy system parameters
fn sync_bodies(
    mut pellets: Query<(&PlayerProjectile, &BodyScale, &mut Transform), (Without<PlayerExplosive>, Without<PlayerThrown>, Without<PlayerDart>)>,
    mut darts: Query<(&PlayerDart, &BodyScale, &mut Transform), (Without<PlayerProjectile>, Without<PlayerExplosive>, Without<PlayerThrown>)>,
    mut explosives: Query<(&PlayerExplosive, &BodyScale, &mut Transform), (Without<PlayerProjectile>, Without<PlayerThrown>, Without<PlayerDart>)>,
    mut thrown: Query<(&PlayerThrown, &BodyScale, &mut Transform), (Without<PlayerProjectile>, Without<PlayerExplosive>, Without<PlayerDart>)>,
) {
    let update = |t: &mut Transform, pos: Vec3, vel: Vec3, scale: f32| {
        if vel.length_squared() > 1.0 {
            *t = pose(pos, vel, scale);
        } else {
            t.translation = coords::pos(pos.to_array());
        }
    };
    for (p, s, mut t) in &mut pellets {
        update(&mut t, p.pos, p.vel, s.0);
    }
    for (p, s, mut t) in &mut explosives {
        update(&mut t, p.pos, p.vel, s.0);
    }
    for (p, s, mut t) in &mut darts {
        update(&mut t, p.pos, p.vel, s.0);
    }
    for (p, s, mut t) in &mut thrown {
        if p.resting {
            // HitWall at rest: DesiredRotation with pitch and roll 0.
            *t = pose(p.pos, p.throw_dir.with_z(0.0), s.0);
        } else {
            update(&mut t, p.pos, p.vel, s.0);
        }
    }
}

pub struct ProjectilePlugin;

impl Plugin for ProjectilePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SpawnPlayerProjectile>()
            .add_message::<BeamZap>()
            .init_resource::<BeamView>()
            .add_systems(Update, beam_zap)
            .init_resource::<ProjectileModels>()
            .add_systems(PostStartup, load_models)
            .add_message::<BoltPickedUp>()
            .init_resource::<BoltRoom>()
            .add_systems(Update, pick_up_bolts)
            .add_systems(Update, (spawn_projectiles, scream_explosives, move_projectiles, move_explosives, move_thrown, move_flames, move_darts, kill_effects_after, sync_bodies).chain());
    }
}

/// Pellet tracers, cycled (KF spawns a KFTracer per pellet).
const PELLET_TRACERS: u32 = 32;

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn spawn_projectiles(
    mut commands: Commands,
    mut spawns: MessageReader<SpawnPlayerProjectile>,
    mut next_id: Local<u32>,
    spatial: SpatialQuery,
    zeds: Query<&Zed>,
    mut bullet_fx: MessageWriter<crate::weapons::bullet_fx::BulletFx>,
    library: Option<Res<crate::render::particles::EffectLibrary>>,
    mut meshes: ResMut<Assets<Mesh>>,
    models: Res<ProjectileModels>,
) {
    for s in spawns.read() {
        *next_id += 1;
        let to_bevy = |v: Vec3| coords::pos(v.to_array());
        let mut origin = s.origin;
        let (a, b) = (to_bevy(s.trace_from), to_bevy(s.origin));
        if let Ok(d) = Dir3::new(b - a)
            && let Some(h) = spatial.cast_ray(a, d, (b - a).length(), true, &crate::world::collision::world_filter())
        {
            origin = s.trace_from + (s.origin - s.trace_from).normalize_or_zero() * (h.distance / SCALE);
        }
        if let Some(t) = s.thrown {
            let dir = s.dir.normalize_or_zero();
            let fuse = match t.kind {
                ThrownKind::Frag { fuse } => fuse,
                ThrownKind::Pipe { .. } => 1.0,
            };
            let e = commands.spawn(PlayerThrown {
                pos: origin,
                vel: dir * (t.speed + s.extra_speed),
                stats: t,
                weapon: s.weapon,
                age: 0.0,
                resting: false,
                bounced: false,
                timer: fuse,
                arming: None,
                countdown: None,
                throw_dir: dir,
                id: *next_id,
                cloud: None,
            }).id();
            attach_model(&mut commands, &models, e, t.class, origin, dir);
            runlog::kv(
                "thrown_spawned",
                &format!("id={} weapon={} speed={:.0} damage={} radius={}", *next_id, s.weapon, t.speed + s.extra_speed, t.damage, t.radius),
            );
            continue;
        }
        if let Some(d) = s.dart {
            let dir = s.dir.normalize_or_zero();
            let e = commands
                .spawn(PlayerDart { pos: origin, vel: dir * d.speed, stats: d, weapon: s.weapon, age: 0.0, id: *next_id })
                .id();
            attach_model(&mut commands, &models, e, d.class, origin, dir);
            if let Some(a) = flight_sound(d.flight) {
                commands.entity(e).insert(a);
            }
            runlog::kv("dart_fired", &format!("id={} weapon={} speed={} heal={}", *next_id, s.weapon, d.speed, d.heal));
            continue;
        }
        if let Some(fl) = s.flame {
            // PostBeginPlay: Velocity = Speed x the aim, + TossZ upward.
            let mut vel = s.dir.normalize_or_zero() * fl.speed;
            vel.z += fl.toss_z;
            let trail = library.as_deref().and_then(|lib| {
                crate::render::particles::spawn_effect(&mut commands, lib, &mut meshes, FLAME_TRAIL, origin, crate::zeds::fireball::axes_along(vel), *next_id)
            });
            commands.spawn(PlayerFlame {
                pos: origin,
                vel,
                stats: fl,
                fire: s.stats.fire,
                dam: s.stats.dam,
                weapon: s.weapon,
                timer: FLAME_TIMER,
                runs: 0,
                age: 0.0,
                trail,
                id: *next_id,
            });
            continue;
        }
        if let Some(x) = s.explosive {
            let dir = s.dir.normalize_or_zero();
            let trail = x.trail.and_then(|class| {
                let lib = library.as_deref()?;
                // LAWProj turns PanzerfaustTrail backward (RelativeRotation
                // pitch 32768); the Husk Gun's flame trail points along.
                let backward = class.ends_with("PanzerfaustTrail");
                let axes = crate::zeds::fireball::axes_along(if backward { -dir } else { dir });
                crate::render::particles::spawn_effect(&mut commands, lib, &mut meshes, class, origin, axes, *next_id)
            });
            let e = commands.spawn(PlayerExplosive {
                pos: origin,
                vel: dir * x.speed,
                stats: x,
                weapon: s.weapon,
                age: 0.0,
                flight: 0.0,
                falling: false,
                dud: None,
                trail,
                id: *next_id,
            }).id();
            attach_model(&mut commands, &models, e, x.class, origin, dir);
            if let Some(a) = flight_sound(x.sounds.flight) {
                commands.entity(e).insert(a);
            }
            runlog::kv(
                "explosive_fired",
                &format!(
                    "id={} weapon={} at_unreal=({:.0}, {:.0}, {:.0}) speed={} damage={} radius={} momentum={}",
                    *next_id, s.weapon, origin.x, origin.y, origin.z, x.speed, x.damage, x.radius, x.momentum
                ),
            );
            continue;
        }
        // The tracer flies along the first straight path, to the wall or to
        // the zed where the projectile will stop (zeds as they are now;
        // nails' bounces not drawn).
        if let Some(start) = s.tracer_start {
            let from = to_bevy(origin);
            let dir = coords::dir(s.dir.normalize_or_zero().to_array()).normalize_or_zero();
            if let Ok(d) = Dir3::new(dir) {
                let max = s.stats.speed * s.stats.life_span * SCALE;
                let wall = spatial.cast_ray(from, d, max, true, &crate::world::collision::world_filter()).map_or(max, |h| h.distance);
                let mut zed_t: Vec<f32> = zeds
                    .iter()
                    .filter(|z| z.health > 0.0)
                    .filter_map(|z| crate::game::combat::zed_hit(z, from, dir))
                    .filter(|&t| t < wall)
                    .collect();
                zed_t.sort_by(f32::total_cmp);
                let end_t = zed_t.get(penetration_limit(&s.stats).saturating_sub(1)).copied().unwrap_or(wall);
                bullet_fx.write(crate::weapons::bullet_fx::BulletFx {
                    shooter: crate::weapons::bullet_fx::Shooter::PlayerPellet((*next_id % PELLET_TRACERS) as u8),
                    start: Some(start),
                    hit: origin + s.dir.normalize_or_zero() * (end_t / SCALE),
                    into: s.dir,
                    impact: false,
                    tracer_speed: s.stats.speed,
                    min_distance: 0.0,
                });
            }
        }
        let e = commands.spawn(PlayerProjectile {
            pos: origin,
            vel: s.dir.normalize_or_zero() * s.stats.speed,
            damage: s.stats.damage,
            stats: s.stats,
            weapon: s.weapon,
            hit: Vec::new(),
            bounces_left: s.stats.bounces,
            falling: false,
            age: 0.0,
            id: *next_id,
        }).id();
        attach_model(&mut commands, &models, e, s.stats.class, origin, s.dir);
    }
}

/// How many zeds a projectile passes before it stops (ProcessTouch's rule),
/// for drawing its tracer only as far as it goes.
pub fn penetration_limit(stats: &ProjectileStats) -> usize {
    if stats.rule == PenRule::Bolt {
        return usize::MAX;
    }
    let r = stats.pen_damage_reduction;
    if !(0.0..1.0).contains(&r) || stats.max_penetrations <= 0.0 {
        return 1;
    }
    let stop = r / stats.max_penetrations;
    let mut ratio = 1.0;
    for n in 1..=16 {
        ratio *= r;
        if ratio <= stop {
            return n;
        }
    }
    16
}

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn move_projectiles(
    mut commands: Commands,
    time: Res<Time>,
    spatial: SpatialQuery,
    mut projectiles: Query<(Entity, &mut PlayerProjectile)>,
    mut zeds: Query<&mut Zed>,
    mut kills: ResMut<crate::game::combat::KillCount>,
    mut bullet_fx: MessageWriter<crate::weapons::bullet_fx::BulletFx>,
    player: Query<&Transform, With<crate::engine::camera::FlyCamera>>,
    (glass, mut glass_damage): (Query<&crate::world::glass::GlassCollider>, MessageWriter<crate::world::glass::GlassDamage>),
    (mut sounds, mut rng): (MessageWriter<crate::audio::mixer::PlaySound>, Local<u32>),
    vet: Res<crate::game::perks::Veterancy>,
) {
    let dt = time.delta_secs();
    let attacker = player.single().map_or(Vec3::ZERO, |t| t.translation - Vec3::Y * crate::game::combat::PLAYER_EYE_HEIGHT * SCALE);
    for (entity, mut p) in &mut projectiles {
        p.age += dt;
        if p.age >= p.stats.life_span {
            runlog::kv("projectile_expired", &format!("id={} weapon={} age={:.2}", p.id, p.weapon, p.age));
            commands.entity(entity).despawn();
            continue;
        }
        // Only nails fall (after their first bounce): NailGunProjectile is
        // bBounce, so half gravity (CP-1).
        let step = if p.falling { bouncer_fall_step(&mut p.vel, dt) } else { p.vel * dt };
        let len = step.length();
        if len <= 0.0 {
            continue;
        }
        let dir_ue = step / len;
        let from = coords::pos(p.pos.to_array());
        let dir = coords::dir(dir_ue.to_array()).normalize_or_zero();
        let Ok(dir3) = Dir3::new(dir) else { continue };
        let world = spatial.cast_ray(from, dir3, len * SCALE, true, &crate::world::collision::world_filter());
        let world_t = world.map_or(len * SCALE, |h| h.distance);
        // Zeds along this step, before the wall, nearest first.
        let mut hits: Vec<(f32, Mut<Zed>)> = Vec::new();
        for z in &mut zeds {
            if z.health <= 0.0 || p.hit.contains(&z.id) {
                continue;
            }
            if let Some(t) = crate::game::combat::zed_hit(&z, from, dir)
                && t <= world_t
            {
                hits.push((t, z));
            }
        }
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut stopped = false;
        for (t, mut z) in hits {
            let point = from + dir * t;
            // ProcessTouch: x HeadShotDamageMult on a headshot; then
            // KFMonster.TakeDamage checks again and applies the damage
            // type's multiplier too.
            let head = crate::game::combat::is_headshot(&z, point, dir, 1.0);
            let damage = if head { p.damage * p.stats.headshot_mult } else { p.damage };
            z.last_hit = Some((point, dir));
            p.hit.push(z.id);
            runlog::kv(
                "projectile_hit",
                &format!(
                    "id={} weapon={} zed={} hit_number={} damage={damage:.1} headshot={head} flight_unreal={:.0}",
                    p.id,
                    p.weapon,
                    z.id,
                    p.hit.len(),
                    p.age * p.stats.speed
                ),
            );
            let source = crate::game::combat::HitSource { point, attacker, melee: false, explosive: None, fire: p.stats.fire, dam: p.stats.dam, vet: vet.vet };
            crate::game::combat::damage_zed(&mut z, damage, head, p.stats.damage_type_headshot_mult, p.weapon, t, source, &mut kills);
            if p.stats.rule == PenRule::Bolt {
                // CrossbowArrow / M99Bullet.PlayhitNoise: Arrow_hitflesh
                // (bullethitflesh4) at the defaults (0.3, radius 300).
                sounds.write(crate::audio::mixer::PlaySound::new("KFWeaponSound.bullethitflesh4", crate::audio::mixer::Emitter::Point(point)));
                p.damage /= 1.25;
                p.vel *= 0.85;
                continue;
            }
            // ShotgunBullet.ProcessTouch: PenDamageReduction from the perk's
            // GetShotgunPenetrationDamageMulti.
            let pen = vet.vet.shotgun_penetration(p.stats.pen_damage_reduction);
            if pen != p.stats.pen_damage_reduction {
                runlog::kv("perk_mod", &format!("kind=shotgun_penetration perk={} weapon={} pen_damage_reduction={}->{pen:.4}", vet.vet.label(), p.weapon, p.stats.pen_damage_reduction));
            }
            p.damage *= pen;
            if p.damage / p.stats.damage <= pen / p.stats.max_penetrations.max(1e-3) {
                stopped = true;
                break;
            }
        }
        if stopped {
            commands.entity(entity).despawn();
            continue;
        }
        match world {
            Some(h) => {
                // HitWall on a non-static actor: Wall.TakeDamage(Damage) (a
                // glass pane).
                if let Ok(g) = glass.get(h.entity) {
                    glass_damage.write(crate::world::glass::GlassDamage { pane: g.0, damage: p.damage, by: p.weapon });
                }
                let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
                let n = if h.normal.dot(dir) > 0.0 { -h.normal } else { h.normal };
                let n_ue = to_ue(n).normalize_or_zero();
                let hit_ue = p.pos + dir_ue * (h.distance / SCALE);
                if p.bounces_left > 0 {
                    // NailGunProjectile.HitWall: reflect at 0.65 speed, fall;
                    // 40% of the time ImpactSounds[Rand(6)] (all Impact_Metal)
                    // at the defaults.
                    *rng = rng.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    if (*rng >> 16) % 100 < 40 {
                        sounds.write(sound_at("ProjectileSounds.Bullets.Impact_Metal", hit_ue));
                    }
                    p.bounces_left -= 1;
                    p.vel = 0.65 * (p.vel - 2.0 * n_ue * p.vel.dot(n_ue));
                    p.pos = hit_ue + n_ue;
                    p.falling = true;
                    runlog::kv("projectile_bounce", &format!("id={} weapon={} bounces_left={}", p.id, p.weapon, p.bounces_left));
                    continue;
                }
                // CrossbowArrow / M99Bullet.HitWall: Arrow_hitwall[Rand(3)]
                // (bullethitflesh2/3/4: KF's names) at 2.5 x 0.3, radius 300.
                if p.stats.rule == PenRule::Bolt {
                    *rng = rng.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    let pick = ["KFWeaponSound.bullethitflesh2", "KFWeaponSound.bullethitflesh3", "KFWeaponSound.bullethitflesh4"][(*rng >> 16) as usize % 3];
                    sounds.write(sound_at(pick, hit_ue).volume(0.75));
                }
                // HitWall: ImpactEffect (ROBulletHitEffect) at the wall.
                bullet_fx.write(crate::weapons::bullet_fx::BulletFx {
                    shooter: crate::weapons::bullet_fx::Shooter::Player,
                    start: None,
                    hit: hit_ue + 2.0 * n_ue,
                    into: -n_ue,
                    impact: true,
                    tracer_speed: p.stats.speed,
                    min_distance: 0.0,
                });
                runlog::kv(
                    "projectile_wall",
                    &format!("id={} weapon={} flight_unreal={:.0} zeds_hit={}", p.id, p.weapon, p.age * p.stats.speed, p.hit.len()),
                );
                // CrossbowArrow.Stick: stays on the wall until LifeSpan.
                if p.stats.pickup {
                    commands.spawn(StuckBolt {
                        pos: hit_ue + n_ue,
                        life: p.stats.life_span - p.age,
                        id: p.id,
                    });
                }
                commands.entity(entity).despawn();
            }
            None => p.pos += step,
        }
    }
}


/// KFMonster.GetExposureTo: the share of the zed in sight of the blast,
/// traced to its head (0.4), root (0.3) and feet (0.15 each). The feet
/// are approximated as points at the bottom of its cylinder, 10 units to
/// either side (no foot bones looked up).
fn zed_exposure(spatial: &SpatialQuery, z: &Zed, at: Vec3) -> f32 {
    let head = z.head.map_or(z.centre + Vec3::Y * z.half_height * 0.9 * SCALE, |(h, _)| h);
    let foot = z.centre - Vec3::Y * z.half_height * 0.95 * SCALE;
    let side = Vec3::X * 10.0 * SCALE;
    let from = coords::pos(at.to_array());
    [(head, 0.4), (z.centre, 0.3), (foot + side, 0.15), (foot - side, 0.15)]
        .iter()
        .filter(|(p, _)| in_sight(spatial, from, *p))
        .map(|(_, w)| w)
        .sum()
}

/// No level geometry between two Bevy points.
fn in_sight(spatial: &SpatialQuery, a: Vec3, b: Vec3) -> bool {
    let Ok(d) = Dir3::new(b - a) else { return true };
    spatial
        .cast_ray(a, d, (b - a).length(), true, &crate::world::collision::world_filter())
        .is_none()
}

/// The player's cylinder (KFPawn), Unreal units.
const PLAYER_RADIUS: f32 = 20.0;
const PLAYER_HEAD: f32 = 40.0;

#[allow(clippy::too_many_arguments, clippy::type_complexity)] // Bevy system parameters
fn move_explosives(
    mut commands: Commands,
    time: Res<Time>,
    spatial: SpatialQuery,
    mut explosives: Query<(Entity, &mut PlayerExplosive)>,
    mut zeds: Query<&mut Zed>,
    mut kills: ResMut<crate::game::combat::KillCount>,
    player: Query<(&Transform, Option<&crate::player::walk::Walker>), With<crate::engine::camera::FlyCamera>>,
    mut effects: Query<&mut crate::render::particles::ParticleEffect>,
    library: Option<Res<crate::render::particles::EffectLibrary>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut decals: MessageWriter<crate::render::decals::SpawnDecal>,
    mut player_damage: MessageWriter<crate::game::combat::PlayerDamaged>,
    mut door_blasts: MessageWriter<crate::world::door::DoorBlast>,
    (mut dramatic, mut sounds, mut rng): (MessageWriter<crate::game::zed_time::DramaticEvent>, MessageWriter<crate::audio::mixer::PlaySound>, Local<u32>),
    vet: Res<crate::game::perks::Veterancy>,
) {
    let dt = time.delta_secs().min(0.1);
    let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
    // Instigator.Location: the player's cylinder centre, Unreal units.
    let player_ue = player.single().ok().map(|(t, w)| {
        to_ue(w.map_or(t.translation - Vec3::Y * crate::game::combat::PLAYER_EYE_HEIGHT * SCALE, |w| w.center)) / SCALE
    });
    for (entity, mut p) in &mut explosives {
        p.age += dt;
        let kill_trail = |effects: &mut Query<&mut crate::render::particles::ParticleEffect>, trail: Option<Entity>| {
            if let Some(t) = trail
                && let Ok(mut fx) = effects.get_mut(t)
            {
                fx.kill();
            }
        };
        if let Some(t) = p.dud.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                kill_trail(&mut effects, p.trail);
                commands.entity(entity).despawn();
                continue;
            }
        }
        if p.age >= p.stats.life_span {
            kill_trail(&mut effects, p.trail);
            commands.entity(entity).despawn();
            continue;
        }
        // M79GrenadeProjectile.Tick: out of propellant after
        // StraightFlightTime, then PHYS_Falling.
        if p.stats.straight_time.is_some_and(|s| p.age > s) || p.dud.is_some() {
            if !p.falling && p.dud.is_none() {
                runlog::kv(
                    "explosive_propellant_out",
                    &format!(
                        "id={} weapon={} at_unreal=({:.0}, {:.0}, {:.0}) age={:.3} speed={:.0} vel_z={:.0}",
                        p.id, p.weapon, p.pos.x, p.pos.y, p.pos.z, p.age, p.vel.length(), p.vel.z
                    ),
                );
            }
            p.falling = true;
        }
        // Falling: full gravity, speed capped at 2500 (CP-3). Before that the
        // M79 family flies by KF's true ballistics, the LAW family straight.
        let step = if p.falling {
            fall_step(&mut p.vel, dt)
        } else if let Some(b) = p.stats.ballistics {
            let p = &mut *p;
            ballistic_step(&mut p.vel, &mut p.flight, &b, dt)
        } else {
            p.vel * dt
        };
        let len = step.length();
        if len <= 0.0 {
            continue;
        }
        let dir_ue = step / len;
        let from = coords::pos(p.pos.to_array());
        let dir = coords::dir(dir_ue.to_array()).normalize_or_zero();
        let Ok(dir3) = Dir3::new(dir) else { continue };
        let world = spatial.cast_ray(from, dir3, len * SCALE, true, &crate::world::collision::world_filter());
        let world_t = world.map_or(len * SCALE, |h| h.distance);
        let touched = if p.dud.is_some() {
            None
        } else {
            zeds.iter()
                .filter(|z| z.health > 0.0)
                .filter_map(|z| crate::game::combat::zed_hit(z, from, dir).filter(|&t| t <= world_t).map(|t| (t, z.id)))
                .min_by(|a, b| a.0.total_cmp(&b.0))
        };
        let (t, normal, zed) = match (touched, world) {
            (Some((t, id)), _) => (t, -dir_ue, Some(id)),
            (None, Some(h)) => {
                let n = if h.normal.dot(dir) > 0.0 { -h.normal } else { h.normal };
                (h.distance, to_ue(n).normalize_or_zero(), None)
            }
            (None, None) => {
                p.pos += step;
                if let Some(trail) = p.trail
                    && let Ok(mut fx) = effects.get_mut(trail)
                {
                    fx.frame.0 = p.pos;
                }
                continue;
            }
        };
        let at = p.pos + dir_ue * (t / SCALE);
        if p.dud.is_some() {
            // A dud resting on the floor.
            p.vel = Vec3::ZERO;
            continue;
        }
        // ProcessTouch / HitWall: closer than ArmDist to the player (where
        // the player is now) it is a dud: a zed takes ImpactDamage.
        let from_player = player_ue.map_or(f32::MAX, |pl| (at - pl).length());
        if from_player < p.stats.arm_dist {
            if let Some(id) = zed
                && let Some(mut z) = zeds.iter_mut().find(|z| z.id == id)
            {
                let point = coords::pos(at.to_array());
                let head = crate::game::combat::is_headshot(&z, point, dir, 1.0);
                z.last_hit = Some((point, dir));
                let attacker = coords::pos(player_ue.unwrap_or(at).to_array());
                let source = crate::game::combat::HitSource { point, attacker, melee: false, explosive: None, fire: None, dam: p.stats.impact_dam, vet: vet.vet };
                crate::game::combat::damage_zed(&mut z, p.stats.impact_damage, head, p.stats.impact_headshot_mult, p.weapon, t, source, &mut kills);
            }
            runlog::kv(
                "explosive_dud",
                &format!("id={} weapon={} hit={} distance_from_player_unreal={from_player:.0}", p.id, p.weapon, if zed.is_some() { "zed" } else { "level" }),
            );
            p.dud = Some(1.0);
            p.vel = Vec3::ZERO;
            p.pos = at;
            // LAWProj: AmbientSound = none, PTRD_deflect04 at 2.0 (the M79
            // family only on a pawn; on a wall it just drops).
            commands.entity(entity).remove::<crate::audio::mixer::AmbientSound>();
            if let Some(d) = p.stats.sounds.dud
                && (zed.is_some() || p.stats.straight_time.is_none())
            {
                sounds.write(sound_at(d, at).volume(2.0));
            }
            continue;
        }
        // HuskGunProjectile.ProcessTouch: ImpactDamage to the zed touched
        // (x HeadShotDamageMult on a headshot), then Explode.
        if let (Some(head_mult), Some(id)) = (p.stats.impact_on_touch, zed)
            && let Some(mut z) = zeds.iter_mut().find(|z| z.id == id)
        {
            let point = coords::pos(at.to_array());
            let head = crate::game::combat::is_headshot(&z, point, dir, 1.0);
            z.last_hit = Some((point, dir));
            let attacker = coords::pos(player_ue.unwrap_or(at).to_array());
            let source = crate::game::combat::HitSource { point, attacker, melee: false, explosive: None, fire: None, dam: p.stats.impact_dam, vet: vet.vet };
            let damage = if head { p.stats.impact_damage * head_mult } else { p.stats.impact_damage };
            runlog::kv("explosive_impact", &format!("id={} weapon={} zed={id} damage={damage:.1} headshot={head}", p.id, p.weapon));
            crate::game::combat::damage_zed(&mut z, damage, head, p.stats.impact_headshot_mult, p.weapon, t, source, &mut kills);
        }
        // Explode: PlaySound(ExplosionSound, , 2.0 or ExplosionSoundVolume).
        *rng = rng.wrapping_mul(1_103_515_245).wrapping_add(12345);
        if let Some(snd) = p.stats.sounds.pick_explosion(*rng >> 16) {
            sounds.write(sound_at(snd, at).volume(p.stats.sounds.explode_volume).radius(p.stats.sounds.explode_radius));
        }
        // Explode: the effect 20 units out, the decal, HurtRadius.
        let (zeds_hit, zeds_killed, self_damage) = blast(
            &mut commands,
            &spatial,
            &mut zeds,
            &mut kills,
            player_ue,
            library.as_deref(),
            &mut meshes,
            &mut decals,
            &mut player_damage,
            &mut door_blasts,
            &Blast {
                at,
                normal,
                effect_offset: 20.0,
                effect: p.stats.effect,
                decal: p.stats.decal,
                damage: p.stats.damage,
                radius: p.stats.radius,
                fleshpound_mult: p.stats.fleshpound_mult,
                fire: p.stats.fire,
                hurts_self: p.stats.hurts_self,
                zap: p.stats.zap,
                frag: false,
                weapon: p.weapon,
                id: p.id,
                dam: p.stats.dam,
                vet: vet.vet,
            },
        );
        if let Some(e) = crate::game::zed_time::blast_event(zeds_killed as usize) {
            dramatic.write(e);
        }
        runlog::kv(
            "explosive_exploded",
            &format!(
                "id={} weapon={} hit={} at_unreal=({:.0}, {:.0}, {:.0}) flight={:.2}s zeds_hit={zeds_hit} zeds_killed={zeds_killed} self_damage={self_damage}",
                p.id,
                p.weapon,
                if zed.is_some() { "zed" } else { "level" },
                at.x,
                at.y,
                at.z,
                p.age
            ),
        );
        kill_trail(&mut effects, p.trail);
        commands.entity(entity).despawn();
    }
}


/// FlameTendril in flight: falling, its speed reset every 0.2 s, exploding
/// on the second reset, on touching a zed (at its own location) or on
/// hitting the level.
#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn move_flames(
    mut commands: Commands,
    time: Res<Time>,
    spatial: SpatialQuery,
    mut flames: Query<(Entity, &mut PlayerFlame)>,
    mut zeds: Query<&mut Zed>,
    mut kills: ResMut<crate::game::combat::KillCount>,
    player: Query<(&Transform, Option<&crate::player::walk::Walker>), With<crate::engine::camera::FlyCamera>>,
    mut effects: Query<&mut crate::render::particles::ParticleEffect>,
    library: Option<Res<crate::render::particles::EffectLibrary>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut decals: MessageWriter<crate::render::decals::SpawnDecal>,
    mut player_damage: MessageWriter<crate::game::combat::PlayerDamaged>,
    mut rng: Local<u32>,
    vet: Res<crate::game::perks::Veterancy>,
) {
    let dt = time.delta_secs().min(0.1);
    let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
    let player_ue = player.single().ok().map(|(t, w)| {
        to_ue(w.map_or(t.translation - Vec3::Y * crate::game::combat::PLAYER_EYE_HEIGHT * SCALE, |w| w.center)) / SCALE
    });
    for (entity, mut p) in &mut flames {
        p.age += dt;
        let mut burst: Option<(Vec3, Vec3, &str)> = None;
        if p.age >= p.stats.life_span {
            commands.entity(entity).despawn();
            continue;
        }
        p.timer -= dt;
        if p.timer <= 0.0 {
            p.timer += FLAME_TIMER;
            p.runs += 1;
            let speed = p.stats.speed;
            p.vel = p.vel.normalize_or_zero() * speed;
            // FlameTendril.Timer: TimerRunCount >= 2 + the perk's ExtraRange.
            if p.runs >= FLAME_TIMER_RUNS + vet.vet.flame_extra_range() {
                // Explode(Location, VRand()).
                *rng = rng.wrapping_mul(1_103_515_245).wrapping_add(12345 + p.id);
                let mut r = || {
                    *rng = rng.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    ((*rng >> 8) & 0xffff) as f32 / 65535.0 * 2.0 - 1.0
                };
                let v = Vec3::new(r(), r(), r()).normalize_or(Vec3::Z);
                burst = Some((p.pos, v, "timer"));
            }
        }
        if burst.is_none() {
            p.vel.z -= GRAVITY * dt;
            let step = p.vel * dt;
            let len = step.length();
            if len > 0.0 {
                let dir_ue = step / len;
                let from = coords::pos(p.pos.to_array());
                let dir = coords::dir(dir_ue.to_array()).normalize_or_zero();
                if let Ok(dir3) = Dir3::new(dir) {
                    let world = spatial.cast_ray(from, dir3, len * SCALE, true, &crate::world::collision::world_filter());
                    let world_t = world.map_or(len * SCALE, |h| h.distance);
                    let touched = zeds
                        .iter()
                        .filter(|z| z.health > 0.0)
                        .filter_map(|z| crate::game::combat::zed_hit(z, from, dir).filter(|&t| t <= world_t))
                        .min_by(f32::total_cmp);
                    burst = match (touched, world) {
                        // ProcessTouch: Explode(Location, Location) where it touched.
                        (Some(t), _) => {
                            let at = p.pos + dir_ue * (t / SCALE);
                            Some((at, at.normalize_or(Vec3::Z), "zed"))
                        }
                        // HitWall / Landed: Explode at the wall (ExploWallOut 0).
                        (None, Some(h)) => {
                            let n = if h.normal.dot(dir) > 0.0 { -h.normal } else { h.normal };
                            Some((p.pos + dir_ue * (h.distance / SCALE), to_ue(n).normalize_or_zero(), "level"))
                        }
                        (None, None) => {
                            p.pos += step;
                            None
                        }
                    };
                }
            }
        }
        if let Some(trail) = p.trail
            && let Ok(mut fx) = effects.get_mut(trail)
        {
            fx.frame.0 = burst.map_or(p.pos, |b| b.0);
            if burst.is_some() {
                // Destroyed: FlameTrail.Kill(), left where it is.
                fx.kill();
            }
        }
        let Some((at, normal, hit)) = burst else { continue };
        let (zeds_hit, self_damage) = flame_burst(
            &mut commands,
            &spatial,
            &mut zeds,
            &mut kills,
            player_ue,
            library.as_deref(),
            &mut meshes,
            &mut decals,
            &mut player_damage,
            &p,
            at,
            normal,
            vet.vet,
        );
        runlog::kv(
            "flame_burst",
            &format!(
                "id={} weapon={} hit={hit} at_unreal=({:.0}, {:.0}, {:.0}) age={:.2} timer_runs={} zeds_hit={zeds_hit} self_damage={self_damage}",
                p.id, p.weapon, at.x, at.y, at.z, p.age, p.runs
            ),
        );
        commands.entity(entity).despawn();
    }
}

/// FlameTendril.Explode: Projectile.HurtRadius (every zed whose cylinder
/// reaches the radius and whose centre is in sight takes Damage x (1 -
/// max(0, (distance - its radius) / radius)), no exposure, no push), the
/// player too (own damage), the burn mark decal and a FuelFlame.
/// Returns (zeds hit, the player's damage before ReduceDamage).
#[allow(clippy::too_many_arguments)]
fn flame_burst(
    commands: &mut Commands,
    spatial: &SpatialQuery,
    zeds: &mut Query<&mut Zed>,
    kills: &mut crate::game::combat::KillCount,
    player_ue: Option<Vec3>,
    library: Option<&crate::render::particles::EffectLibrary>,
    meshes: &mut Assets<Mesh>,
    decals: &mut MessageWriter<crate::render::decals::SpawnDecal>,
    player_damage: &mut MessageWriter<crate::game::combat::PlayerDamaged>,
    p: &PlayerFlame,
    at: Vec3,
    normal: Vec3,
    vet: crate::game::perks::Vet,
) -> (u32, f32) {
    let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
    let (damage, radius) = (p.stats.damage, p.stats.radius);
    decals.write(crate::render::decals::SpawnDecal {
        kind: crate::render::decals::DecalKind::Scorch,
        at,
        dir: -normal,
        trace: false,
    });
    if let Some(lib) = library
        && let Some(e) = crate::render::particles::spawn_effect(commands, lib, meshes, FUEL_FLAME, at, Mat3::IDENTITY, p.id)
    {
        commands.spawn(KillEffectAfter { effect: e, time: FUEL_FLAME_TIME });
    }
    let at_bevy = coords::pos(at.to_array());
    let mut zeds_hit = 0;
    for mut z in zeds.iter_mut() {
        if z.health <= 0.0 {
            continue;
        }
        let centre = to_ue(z.centre) / SCALE;
        let d = centre - at;
        let dist = d.length().max(1.0);
        if dist - z.radius > radius || !in_sight(spatial, at_bevy, z.centre) {
            continue;
        }
        let dirs = d / dist;
        let scale = 1.0 - ((dist - z.radius) / radius).max(0.0);
        let hit_ue = centre - 0.5 * (z.half_height + z.radius) * dirs;
        let point = coords::pos(hit_ue.to_array());
        let dir_b = coords::dir(dirs.to_array()).normalize_or_zero();
        z.last_hit = Some((point, dir_b));
        let source = crate::game::combat::HitSource { point, attacker: at_bevy, melee: false, explosive: None, fire: p.fire, dam: p.dam, vet };
        crate::game::combat::damage_zed(&mut z, scale * damage, false, 1.0, p.weapon, dist * SCALE, source, kills);
        zeds_hit += 1;
    }
    let mut self_damage = 0.0;
    if let Some(pl) = player_ue {
        let dist = (pl - at).length().max(1.0);
        if dist - PLAYER_RADIUS <= radius && in_sight(spatial, at_bevy, coords::pos(pl.to_array())) {
            self_damage = ((1.0 - ((dist - PLAYER_RADIUS) / radius).max(0.0)) * damage).floor();
            if self_damage > 0.0 {
                player_damage.write(crate::game::combat::PlayerDamaged {
                    amount: self_damage,
                    armor_stops: true,
                    zed_id: crate::game::combat::SELF_DAMAGE,
                    kind: crate::game::combat::HurtKind::Fire,
                    dam_type: crate::game::combat::DamType::Other,
                    source: Some(at_bevy),
                    dam: p.dam,
                    to_peer: None,
                });
            }
        }
    }
    (zeds_hit, self_damage)
}

/// Medic darts in flight: straight; a zed or the level ends them with the
/// bullet-hit effect.
fn move_darts(
    mut commands: Commands,
    time: Res<Time>,
    spatial: SpatialQuery,
    mut darts: Query<(Entity, &mut PlayerDart)>,
    zeds: Query<&Zed>,
    mut bullet_fx: MessageWriter<crate::weapons::bullet_fx::BulletFx>,
    (teammates, vet, mut heal_out): (Res<crate::game::healing::Teammates>, Res<crate::game::perks::Veterancy>, MessageWriter<crate::game::healing::HealTeammate>),
) {
    let dt = time.delta_secs().min(0.1);
    let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
    for (entity, mut p) in &mut darts {
        p.age += dt;
        if p.age >= p.stats.life_span {
            commands.entity(entity).despawn();
            continue;
        }
        let step = p.vel * dt;
        let len = step.length();
        let dir_ue = step / len.max(1e-6);
        let from = coords::pos(p.pos.to_array());
        let dir = coords::dir(dir_ue.to_array()).normalize_or_zero();
        let Ok(dir3) = Dir3::new(dir) else { continue };
        let world = spatial.cast_ray(from, dir3, len * SCALE, true, &crate::world::collision::world_filter());
        let world_t = world.map_or(len * SCALE, |h| h.distance);
        let zed = zeds
            .iter()
            .filter(|z| z.health > 0.0)
            .filter_map(|z| crate::game::combat::zed_hit(z, from, dir).filter(|&t| t <= world_t).map(|t| (t, z.id)))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        // Another player's pawn (HealingProjectile.ProcessTouch on a
        // KFHumanPawn; the shooter's own pawn is skipped).
        let mate = teammates
            .0
            .iter()
            .filter(|m| m.alive)
            .filter_map(|m| {
                let t = crate::game::healing::ray_cylinder(from, dir, world_t, m.centre, crate::game::healing::PAWN_RADIUS, crate::game::healing::PAWN_HALF_HEIGHT, SCALE)?;
                Some((t, m))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .filter(|(t, _)| zed.is_none_or(|(zt, _)| *t < zt));
        if let Some((_, m)) = mate {
            // Healed.Health > 0 and < HealthMax: MedicReward =
            // HealBoostAmount x GetHealPotency (an int), GiveHealth(HealSum).
            let healed = if m.health > 0.0 && m.health < crate::game::combat::PLAYER_HEALTH_MAX {
                let amount = (p.stats.heal * vet.vet.heal_potency()).trunc();
                heal_out.write(crate::game::healing::HealTeammate { peer: m.peer, heal_sum: amount, source: "dart" });
                format!("{amount}")
            } else {
                "none".into()
            };
            runlog::kv("dart_hit", &format!("id={} weapon={} hit=player peer={} healed={healed} flight_unreal={:.0}", p.id, p.weapon, m.peer, p.age * p.stats.speed));
            commands.entity(entity).despawn();
            continue;
        }
        let (t, normal, what) = match (zed, world) {
            (Some((t, id)), _) => (t, -dir_ue, format!("zed={id}")),
            (None, Some(h)) => {
                let n = if h.normal.dot(dir) > 0.0 { -h.normal } else { h.normal };
                (h.distance, to_ue(n).normalize_or_zero(), "level".to_string())
            }
            (None, None) => {
                p.pos += step;
                continue;
            }
        };
        let at = p.pos + dir_ue * (t / SCALE);
        // Explode: ROBulletHitEffect at the spot, facing back along the dart.
        bullet_fx.write(crate::weapons::bullet_fx::BulletFx {
            shooter: crate::weapons::bullet_fx::Shooter::Player,
            start: None,
            hit: at + 2.0 * normal,
            into: -normal,
            impact: true,
            tracer_speed: p.stats.speed,
            min_distance: 0.0,
        });
        runlog::kv("dart_hit", &format!("id={} weapon={} hit={what} flight_unreal={:.0} healed=none", p.id, p.weapon, p.age * p.stats.speed));
        commands.entity(entity).despawn();
    }
}

/// ZEDGunAltFire.ModeTick: trace TraceRange from the beam start; the zed
/// it reaches first (before the level) gets SetZapped(dt); on a FireRate
/// tick that hit something, every other living zed in sight within the
/// splash radius of the end gets SetZapped(FireRate x 0.75).
fn beam_zap(mut zaps: MessageReader<BeamZap>, spatial: SpatialQuery, mut zeds: Query<&mut Zed>, mut view: ResMut<BeamView>) {
    view.active = false;
    let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
    for m in zaps.read() {
        let from = coords::pos(m.start.to_array());
        let dir = coords::dir(m.dir.to_array()).normalize_or_zero();
        let Ok(dir3) = Dir3::new(dir) else { continue };
        let max = m.range * SCALE;
        let world = spatial.cast_ray(from, dir3, max, true, &crate::world::collision::world_filter()).map(|h| h.distance);
        let wall_t = world.unwrap_or(max);
        let hit = zeds
            .iter()
            .filter(|z| z.health > 0.0)
            .filter_map(|z| crate::game::combat::zed_hit(z, from, dir).filter(|&t| t <= wall_t).map(|t| (t, z.id)))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let t = hit.map_or(wall_t, |h| h.0);
        let end_b = from + dir * t;
        if let Some((_, id)) = hit
            && let Some(mut z) = zeds.iter_mut().find(|z| z.id == id)
        {
            z.set_zapped(m.dt);
        }
        let hit_something = hit.is_some() || world.is_some();
        let mut splashed = 0;
        if m.do_hit && hit_something && m.sphere_radius > 0.0 {
            for mut z in zeds.iter_mut() {
                let d = (z.centre - end_b).length() / SCALE;
                if z.health > 0.0 && Some(z.id) != hit.map(|h| h.1) && d - z.radius <= m.sphere_radius && in_sight(&spatial, end_b, z.centre) {
                    z.set_zapped(m.sphere_amount);
                    splashed += 1;
                }
            }
        }
        if m.do_hit {
            runlog::kv(
                "beam_zap",
                &format!(
                    "hit={} distance_unreal={:.0} splash_radius={:.0} splashed={splashed}",
                    hit.map_or(if world.is_some() { "level".to_string() } else { "nothing".to_string() }, |h| format!("zed={}", h.1)),
                    t / SCALE,
                    m.sphere_radius
                ),
            );
        }
        *view = BeamView { active: true, start: m.start, end: to_ue(end_b) / SCALE };
    }
}

/// Kills effects whose time is up (FuelFlame's Timer).
fn kill_effects_after(
    mut commands: Commands,
    time: Res<Time>,
    mut timers: Query<(Entity, &mut KillEffectAfter)>,
    mut effects: Query<&mut crate::render::particles::ParticleEffect>,
) {
    for (e, mut k) in &mut timers {
        k.time -= time.delta_secs();
        if k.time <= 0.0 {
            if let Ok(mut fx) = effects.get_mut(k.effect) {
                fx.kill();
            }
            commands.entity(e).despawn();
        }
    }
}

/// One explosion (Explode + HurtRadius), Unreal units.
struct Blast {
    at: Vec3,
    normal: Vec3,
    /// The effect is spawned this far out along the normal (M79 / LAW 20;
    /// Nade and pipe bomb at the spot).
    effect_offset: f32,
    effect: &'static str,
    decal: crate::render::decals::DecalKind,
    damage: f32,
    radius: f32,
    fleshpound_mult: Option<f32>,
    /// A burning damage type (the Husk Gun's).
    fire: Option<crate::game::combat::FireType>,
    hurts_self: bool,
    zap: Option<f32>,
    /// MyDamageType is DamTypeFrag (the Nade): the only player blast
    /// KFDoorMover.TakeDamage accepts.
    frag: bool,
    weapon: &'static str,
    id: u32,
    /// The damage type and the instigator's perk.
    dam: Option<crate::game::perks::DamType>,
    vet: crate::game::perks::Vet,
}

/// The effect, the decal, and HurtRadius: every zed whose cylinder reaches
/// the radius takes Damage x (1 - max(0, (distance - its radius) / radius))
/// x its exposure; the player the same with KFPawn's exposure, reduced as
/// own damage (combat::reduce_self_damage), with no push. Returns (zeds
/// hit, zeds killed, self damage after the reduction).
#[allow(clippy::too_many_arguments)]
fn blast(
    commands: &mut Commands,
    spatial: &SpatialQuery,
    zeds: &mut Query<&mut Zed>,
    kills: &mut crate::game::combat::KillCount,
    player_ue: Option<Vec3>,
    library: Option<&crate::render::particles::EffectLibrary>,
    meshes: &mut Assets<Mesh>,
    decals: &mut MessageWriter<crate::render::decals::SpawnDecal>,
    player_damage: &mut MessageWriter<crate::game::combat::PlayerDamaged>,
    door_blasts: &mut MessageWriter<crate::world::door::DoorBlast>,
    b: &Blast,
) -> (u32, u32, f32) {
    let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
    let at = b.at;
    if let Some(lib) = library {
        crate::render::particles::spawn_effect(commands, lib, meshes, b.effect, at + b.normal * b.effect_offset, crate::zeds::fireball::axes_along(b.normal), b.id);
    }
    decals.write(crate::render::decals::SpawnDecal {
        kind: b.decal,
        at,
        dir: -b.normal,
        trace: false,
    });
    let at_bevy = coords::pos(at.to_array());
    let (mut zeds_hit, mut zeds_killed) = (0, 0);
    // ZED gun bolts: no HurtRadius.
    if b.radius <= 0.0 {
        return (0, 0, 0.0);
    }
    // ZEDMKIISecondaryProjectile.HurtRadius (CollidingActors: no line of
    // sight): SetZapped on every living zed, nothing else.
    if let Some(amount) = b.zap {
        for mut z in zeds.iter_mut() {
            let dist = (to_ue(z.centre) / SCALE - at).length();
            if z.health > 0.0 && dist - z.radius <= b.radius {
                z.set_zapped(amount);
                zeds_hit += 1;
            }
        }
        return (zeds_hit, 0, 0.0);
    }
    // HurtRadius (CollidingActors) reaches doors too; KFDoorMover decides.
    door_blasts.write(crate::world::door::DoorBlast {
        at,
        radius: b.radius,
        damage: b.damage,
        zed: None,
        direct: None,
        line_of_sight: false,
        frag: b.frag,
        source: b.weapon,
    });
    for mut z in zeds.iter_mut() {
        if z.health <= 0.0 {
            continue;
        }
        let centre = to_ue(z.centre) / SCALE;
        let d = centre - at;
        let dist = d.length().max(1.0);
        // CollidingActors: the cylinder touches the radius.
        if dist - z.radius > b.radius {
            continue;
        }
        let dirs = d / dist;
        let scale = (1.0 - ((dist - z.radius) / b.radius).max(0.0)) * zed_exposure(spatial, &z, at);
        if scale <= 0.0 {
            continue;
        }
        let hit_ue = centre - 0.5 * (z.half_height + z.radius) * dirs;
        let point = coords::pos(hit_ue.to_array());
        let dir_b = coords::dir(dirs.to_array()).normalize_or_zero();
        z.last_hit = Some((point, dir_b));
        let source = crate::game::combat::HitSource { point, attacker: at_bevy, melee: false, explosive: b.fleshpound_mult, fire: b.fire, dam: b.dam, vet: b.vet };
        let before = z.health;
        crate::game::combat::damage_zed(&mut z, scale * b.damage, false, 1.0, b.weapon, dist * SCALE, source, kills);
        zeds_hit += 1;
        if before > 0.0 && z.health <= 0.0 {
            zeds_killed += 1;
        }
    }
    // The player: KFPawn.GetExposureTo (head and root, half each);
    // KFGameType.ReduceDamage reduces self damage; KFHumanPawn.TakeDamage
    // drops the momentum of a player's damage (no push).
    let mut self_damage = 0.0;
    if let Some(pl) = player_ue.filter(|_| b.hurts_self) {
        let dist = (pl - at).length().max(1.0);
        if dist - PLAYER_RADIUS <= b.radius {
            let exposure = 0.5 * in_sight(spatial, at_bevy, coords::pos((pl + Vec3::Z * PLAYER_HEAD).to_array())) as u8 as f32
                + 0.5 * in_sight(spatial, at_bevy, coords::pos(pl.to_array())) as u8 as f32;
            let scale = (1.0 - ((dist - PLAYER_RADIUS) / b.radius).max(0.0)) * exposure;
            let raw = scale * b.damage;
            self_damage = crate::game::combat::reduce_self_damage(raw);
            if raw >= 1.0 {
                player_damage.write(crate::game::combat::PlayerDamaged {
                    amount: raw,
                    armor_stops: true,
                    zed_id: crate::game::combat::SELF_DAMAGE,
                    // KFPawn.TakeDamage: a DamTypeBurned / DamTypeFlamethrower
                    // hit over 2 sets the player on fire (FlameNade, Husk Gun).
                    kind: if b.fire.is_some() { crate::game::combat::HurtKind::Fire } else { crate::game::combat::HurtKind::Plain },
                    dam_type: crate::game::combat::DamType::Other,
                    source: Some(at_bevy),
                    dam: b.dam,
                    to_peer: None,
                });
            }
        }
    }
    (zeds_hit, zeds_killed, self_damage)
}

/// A thrown explosive's values: the frag's Nade or a PipeBombProjectile.
#[derive(Clone, Copy, Debug)]
pub struct ThrownStats {
    /// The projectile class (for its model).
    pub class: &'static str,
    pub sounds: ProjectileSounds,
    pub speed: f32,
    pub damage: f32,
    pub radius: f32,
    /// HitWall: Velocity = -VNorm x DampenFactor + (Velocity - VNorm) x
    /// DampenFactorParallel; at rest under 20.
    pub dampen_normal: f32,
    pub dampen_parallel: f32,
    pub fleshpound_mult: f32,
    pub effect: &'static str,
    pub decal: crate::render::decals::DecalKind,
    pub kind: ThrownKind,
    /// MyDamageType (DamTypeFrag, DamTypePipeBomb), for the perks.
    pub dam: Option<crate::game::perks::DamType>,
    /// A burning MyDamageType (the Firebug's FlameNade: DamTypeFlameNade).
    pub fire: Option<crate::game::combat::FireType>,
    /// The Medic's MedicNade: instead of one blast, a cloud that hurts
    /// zeds and heals players (HealOrHurt) at the explosion and then
    /// every HealInterval, MaxHeals more times.
    pub medic: Option<MedicCloud>,
}

/// MedicNade's HealBoostAmount, MaxHeals and HealInterval.
#[derive(Clone, Copy, Debug)]
pub struct MedicCloud {
    pub heal: f32,
    pub max_heals: u32,
    pub interval: f32,
}

#[derive(Clone, Copy, Debug)]
pub enum ThrownKind {
    /// Nade: explodes ExplodeTimer after the throw, restarted by its first
    /// bounce (bTimerSet is not set at the throw: KF quirk, kept).
    Frag { fuse: f32 },
    /// PipeBombProjectile: at rest, armed after ArmingCountDown; checks for
    /// zeds within DetectionRadius in sight every second (0.5 s while some
    /// threat but under ThreatThreshhold); detected: CountDown beeps every
    /// 0.15 s, then explodes.
    Pipe { arming: f32, detection_radius: f32, countdown: u32, threshold: f32 },
}

#[derive(Component)]
struct PlayerThrown {
    pos: Vec3,
    vel: Vec3,
    stats: ThrownStats,
    weapon: &'static str,
    age: f32,
    resting: bool,
    bounced: bool,
    /// Frag: seconds to the explosion. Pipe: the next check.
    timer: f32,
    /// Pipe: seconds of arming left; None until at rest.
    arming: Option<f32>,
    /// Pipe: beeps left once a zed is detected.
    countdown: Option<u32>,
    /// The throw's direction (its yaw is kept at rest).
    throw_dir: Vec3,
    id: u32,
    /// MedicNade after its explosion: (TotalHeals, seconds to the next).
    cloud: Option<(u32, f32)>,
}

/// What a MedicNade pulse needs besides the zeds.
#[derive(bevy::ecs::system::SystemParam)]
struct HealParams<'w> {
    teammates: Res<'w, crate::game::healing::Teammates>,
    health: Res<'w, crate::game::combat::PlayerHealth>,
    heal_out: MessageWriter<'w, crate::game::healing::HealTeammate>,
    give: MessageWriter<'w, crate::game::combat::GiveHealth>,
    dosh: ResMut<'w, crate::game::dosh::Dosh>,
}

/// MedicNade.HealOrHurt: every pawn whose cylinder reaches DamageRadius
/// (CollidingActors: no distance falloff) and that the nade can see
/// (GetExposureTo > 0, from 15 units above it). Zeds take Damage x
/// exposure (DamTypeMedicNade; ZombieFleshPound x 2); players under
/// HealthMax get GiveHealth(HealBoostAmount x the thrower's
/// GetHealPotency, an int), the thrower included (KF's code does not
/// leave him out, and pays him for it too). Nothing heals once the
/// thrower is dead. Returns (zeds hit, zeds killed, players healed).
#[allow(clippy::too_many_arguments)]
fn medic_pulse(
    spatial: &SpatialQuery,
    zeds: &mut Query<&mut Zed>,
    kills: &mut crate::game::combat::KillCount,
    player_ue: Option<Vec3>,
    p: &PlayerThrown,
    cloud: MedicCloud,
    vet: crate::game::perks::Vet,
    hp: &mut HealParams,
) -> (u32, u32, u32) {
    let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
    let from_ue = p.pos + Vec3::Z * 15.0;
    let from = coords::pos(from_ue.to_array());
    let r = p.stats.radius;
    let (mut hit, mut killed, mut healed) = (0, 0, 0);
    for mut z in zeds.iter_mut() {
        if z.health <= 0.0 {
            continue;
        }
        let centre = to_ue(z.centre) / SCALE;
        if (centre - p.pos).length() - z.radius > r {
            continue;
        }
        let exposure = zed_exposure(spatial, &z, from_ue);
        if exposure <= 0.0 {
            continue;
        }
        let source = crate::game::combat::HitSource { point: z.centre, attacker: from, melee: false, explosive: Some(p.stats.fleshpound_mult), fire: None, dam: p.stats.dam, vet };
        let before = z.health;
        crate::game::combat::damage_zed(&mut z, exposure * p.stats.damage, false, 1.0, p.weapon, (centre - p.pos).length() * SCALE, source, kills);
        hit += 1;
        if before > 0.0 && z.health <= 0.0 {
            killed += 1;
        }
    }
    if hp.health.dead || hp.health.health <= 0.0 {
        return (hit, killed, 0);
    }
    let heal_sum = (cloud.heal * vet.heal_potency()).trunc();
    let sees = |at: Vec3| {
        0.5 * in_sight(spatial, from, coords::pos((at + Vec3::Z * PLAYER_HEAD).to_array())) as u8 as f32 + 0.5 * in_sight(spatial, from, coords::pos(at.to_array())) as u8 as f32
    };
    // This game's own player (the thrower).
    if let Some(pl) = player_ue
        && (pl - p.pos).length() - PLAYER_RADIUS <= r
        && sees(pl) > 0.0
        && hp.health.health < crate::game::combat::PLAYER_HEALTH_MAX
    {
        hp.give.write(crate::game::combat::GiveHealth { amount: heal_sum, max: crate::game::combat::PLAYER_HEALTH_MAX, source: "medic_nade" });
        let reward = crate::game::healing::medic_reward(heal_sum, hp.health.health, hp.health.to_give.max(0.0));
        hp.dosh.score += reward as f32;
        hp.dosh.team += reward as f32;
        healed += 1;
        runlog::kv("perk_effect", &format!("perk={} effect=medic_nade_heal who=self heal={heal_sum} reward={reward}", vet.label()));
    }
    for m in hp.teammates.0.iter().filter(|m| m.alive && m.health > 0.0 && m.health < crate::game::combat::PLAYER_HEALTH_MAX) {
        let at = to_ue(m.centre) / SCALE;
        if (at - p.pos).length() - PLAYER_RADIUS <= r && sees(at) > 0.0 {
            hp.heal_out.write(crate::game::healing::HealTeammate { peer: m.peer, heal_sum, source: "medic_nade" });
            healed += 1;
        }
    }
    (hit, killed, healed)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)] // Bevy system parameters
fn move_thrown(
    mut commands: Commands,
    time: Res<Time>,
    spatial: SpatialQuery,
    mut thrown: Query<(Entity, &mut PlayerThrown)>,
    mut zeds: Query<&mut Zed>,
    mut kills: ResMut<crate::game::combat::KillCount>,
    player: Query<(&Transform, Option<&crate::player::walk::Walker>), With<crate::engine::camera::FlyCamera>>,
    library: Option<Res<crate::render::particles::EffectLibrary>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut decals: MessageWriter<crate::render::decals::SpawnDecal>,
    mut player_damage: MessageWriter<crate::game::combat::PlayerDamaged>,
    mut door_blasts: MessageWriter<crate::world::door::DoorBlast>,
    (mut dramatic, mut sounds, mut rng): (MessageWriter<crate::game::zed_time::DramaticEvent>, MessageWriter<crate::audio::mixer::PlaySound>, Local<u32>),
    vet: Res<crate::game::perks::Veterancy>,
    mut hp: HealParams,
) {
    let dt = time.delta_secs().min(0.1);
    let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
    let player_ue = player.single().ok().map(|(t, w)| {
        to_ue(w.map_or(t.translation - Vec3::Y * crate::game::combat::PLAYER_EYE_HEIGHT * SCALE, |w| w.center)) / SCALE
    });
    for (entity, mut p) in &mut thrown {
        p.age += dt;
        // MedicNade.Tick after the explosion: a pulse every HealInterval
        // until MaxHeals, then the nade is gone (AmbientSound off).
        if let (Some((done, next)), Some(cloud)) = (p.cloud, p.stats.medic) {
            let next = next - dt;
            if next > 0.0 {
                p.cloud = Some((done, next));
                continue;
            }
            let done = done + 1;
            let (hit, killed, healed) = medic_pulse(&spatial, &mut zeds, &mut kills, player_ue, &p, cloud, vet.vet, &mut hp);
            runlog::kv("medic_nade_pulse", &format!("id={} pulse={done} of={} zeds_hit={hit} zeds_killed={killed} players_healed={healed}", p.id, cloud.max_heals));
            if done >= cloud.max_heals {
                commands.entity(entity).despawn();
            } else {
                p.cloud = Some((done, cloud.interval));
            }
            continue;
        }
        // Fly: PHYS_Falling, bouncing off the level; a zed stops it dead.
        if !p.resting {
            // Nade and PipeBombProjectile are bBounce: half gravity (CP-1).
            let step = bouncer_fall_step(&mut p.vel, dt);
            let len = step.length();
            if len > 0.0 {
                let dir_ue = step / len;
                let from = coords::pos(p.pos.to_array());
                let dir = coords::dir(dir_ue.to_array()).normalize_or_zero();
                if let Ok(dir3) = Dir3::new(dir) {
                    let world = spatial.cast_ray(from, dir3, len * SCALE, true, &crate::world::collision::world_filter());
                    let world_t = world.map_or(len * SCALE, |h| h.distance);
                    let zed_touch = zeds.iter().any(|z| {
                        z.health > 0.0 && crate::game::combat::zed_hit(z, from, dir).is_some_and(|t| t <= world_t)
                    });
                    if zed_touch {
                        // ProcessTouch: Velocity = 0 (then falls).
                        p.vel = Vec3::ZERO;
                    } else if let Some(h) = world {
                        let n = if h.normal.dot(dir) > 0.0 { -h.normal } else { h.normal };
                        let n = to_ue(n).normalize_or_zero();
                        p.pos += dir_ue * (h.distance / SCALE) + n;
                        runlog::kv(
                            "thrown_bounce",
                            &format!(
                                "id={} weapon={} at_unreal=({:.0}, {:.0}, {:.0}) age={:.2} speed_in={:.0}",
                                p.id, p.weapon, p.pos.x, p.pos.y, p.pos.z, p.age, p.vel.length()
                            ),
                        );
                        let v_norm = p.vel.dot(n) * n;
                        p.vel = -v_norm * p.stats.dampen_normal + (p.vel - v_norm) * p.stats.dampen_parallel;
                        // HitWall: ImpactSound (SLOT_Misc, TransientSoundVolume)
                        // when Speed > 50 after the damping.
                        if p.vel.length() > 50.0
                            && let Some(b) = p.stats.sounds.bounce
                        {
                            sounds.write(sound_at(b, p.pos).slot(crate::audio::mixer::Slot::Misc).volume(p.stats.sounds.bounce_volume));
                        }
                        if let ThrownKind::Frag { fuse } = p.stats.kind
                            && !p.bounced
                        {
                            p.timer = fuse;
                        }
                        p.bounced = true;
                        if p.vel.length() < 20.0 {
                            p.resting = true;
                            p.vel = Vec3::ZERO;
                            if let ThrownKind::Pipe { arming, .. } = p.stats.kind {
                                p.arming = Some(arming);
                            }
                            runlog::kv(
                                "thrown_rest",
                                &format!("id={} weapon={} at_unreal=({:.0}, {:.0}, {:.0}) age={:.2}", p.id, p.weapon, p.pos.x, p.pos.y, p.pos.z, p.age),
                            );
                        }
                    } else {
                        p.pos += step;
                    }
                }
            }
        }
        // Timers.
        let mut explode = false;
        match p.stats.kind {
            ThrownKind::Frag { .. } => {
                p.timer -= dt;
                explode = p.timer <= 0.0;
            }
            ThrownKind::Pipe { detection_radius, threshold, .. } => {
                if let Some(a) = p.arming.as_mut() {
                    *a -= dt;
                    if *a <= 0.0 {
                        p.arming = None;
                        p.timer = 0.0;
                        runlog::kv("pipebomb_armed", &format!("id={}", p.id));
                    }
                } else if p.resting {
                    p.timer -= dt;
                    if p.timer <= 0.0 {
                        if let Some(c) = p.countdown.as_mut() {
                            // Fast beeps, then the explosion: while Countdown > 0
                            // PlaySound(BeepSound, SLOT_Misc, 2.0, , 150).
                            *c = c.saturating_sub(1);
                            explode = *c == 0;
                            p.timer = 0.15;
                            if !explode && let Some(b) = p.stats.sounds.beep {
                                sounds.write(sound_at(b, p.pos).slot(crate::audio::mixer::Slot::Misc).volume(2.0).radius(150.0));
                            }
                        } else {
                            // Each check: PlaySound(BeepSound, , 0.5, , 50) first.
                            if let Some(b) = p.stats.sounds.beep {
                                sounds.write(sound_at(b, p.pos).volume(0.5).radius(50.0));
                            }
                            // VisibleCollidingActors within DetectionRadius.
                            let at_bevy = coords::pos(p.pos.to_array());
                            let threat: f32 = zeds
                                .iter()
                                .filter(|z| z.health > 0.0)
                                .filter(|z| (to_ue(z.centre) / SCALE - p.pos).length() - z.radius <= detection_radius)
                                .filter(|z| in_sight(&spatial, at_bevy, z.centre))
                                .map(|z| z.motion_threat)
                                .sum();
                            if threat >= threshold {
                                let ThrownKind::Pipe { countdown, .. } = p.stats.kind else { unreachable!() };
                                p.countdown = Some(countdown);
                                p.timer = 0.15;
                                runlog::kv("pipebomb_detected", &format!("id={} threat={threat:.2}", p.id));
                            } else {
                                p.timer = if threat > 0.0 { 0.5 } else { 1.0 };
                            }
                        }
                    }
                }
            }
        }
        if !explode {
            continue;
        }
        // Nade.Explode: PlaySound(ExplodeSounds[rand(...)], , 2.0);
        // PipeBombProjectile the same.
        *rng = rng.wrapping_mul(1_103_515_245).wrapping_add(12345);
        if let Some(snd) = p.stats.sounds.pick_explosion(*rng >> 16) {
            sounds.write(sound_at(snd, p.pos).volume(2.0).radius(p.stats.sounds.explode_radius));
        }
        let normal = Vec3::Z;
        // MedicNade.Explode: BlowUp (the first HealOrHurt), KFNadeHealing,
        // the decal; the nade stays for the pulses.
        if let Some(cloud) = p.stats.medic {
            if let Some(lib) = library.as_deref() {
                crate::render::particles::spawn_effect(&mut commands, lib, &mut meshes, p.stats.effect, p.pos, crate::zeds::fireball::axes_along(normal), p.id);
            }
            decals.write(crate::render::decals::SpawnDecal { kind: p.stats.decal, at: p.pos, dir: -normal, trace: false });
            let (hit, killed, healed) = medic_pulse(&spatial, &mut zeds, &mut kills, player_ue, &p, cloud, vet.vet, &mut hp);
            if let Some(e) = crate::game::zed_time::blast_event(killed as usize) {
                dramatic.write(e);
            }
            runlog::kv(
                "thrown_exploded",
                &format!(
                    "id={} weapon={} class={} at_unreal=({:.0}, {:.0}, {:.0}) age={:.2} zeds_hit={hit} zeds_killed={killed} players_healed={healed} medic_cloud=true",
                    p.id, p.weapon, p.stats.class, p.pos.x, p.pos.y, p.pos.z, p.age
                ),
            );
            p.cloud = Some((0, cloud.interval));
            continue;
        }
        let (zeds_hit, zeds_killed, self_damage) = blast(
            &mut commands,
            &spatial,
            &mut zeds,
            &mut kills,
            player_ue,
            library.as_deref(),
            &mut meshes,
            &mut decals,
            &mut player_damage,
            &mut door_blasts,
            &Blast {
                at: p.pos,
                normal,
                effect_offset: 0.0,
                effect: p.stats.effect,
                decal: p.stats.decal,
                damage: p.stats.damage,
                radius: p.stats.radius,
                // ZombieFleshPound.TakeDamage's explosive list has
                // DamTypeFrag and DamTypePipeBomb, not DamTypeFlameNade.
                fleshpound_mult: p.stats.fire.is_none().then_some(p.stats.fleshpound_mult),
                fire: p.stats.fire,
                hurts_self: true,
                zap: None,
                // Nade.MyDamageType DamTypeFrag; PipeBombProjectile's is
                // DamTypePipeBomb; FlameNade's DamTypeFlameNade (doors take
                // only DamTypeFrag).
                frag: matches!(p.stats.kind, ThrownKind::Frag { .. }) && p.stats.fire.is_none(),
                weapon: p.weapon,
                id: p.id,
                dam: p.stats.dam,
                vet: vet.vet,
            },
        );
        if let Some(e) = crate::game::zed_time::blast_event(zeds_killed as usize) {
            dramatic.write(e);
        }
        runlog::kv(
            "thrown_exploded",
            &format!(
                "id={} weapon={} at_unreal=({:.0}, {:.0}, {:.0}) age={:.2} bounced={} zeds_hit={zeds_hit} zeds_killed={zeds_killed} self_damage={self_damage}",
                p.id, p.weapon, p.pos.x, p.pos.y, p.pos.z, p.age, p.bounced
            ),
        );
        commands.entity(entity).despawn();
    }
}

/// Which TakeDamage a projectile has, for a Siren's scream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScreamTarget {
    /// Nade (frag, FlameNade, MedicNade): disintegrates (the Siren is a
    /// Monster, which its TakeDamage asks for).
    Nade,
    /// PipeBombProjectile: ignores damage under 25, else disintegrates
    /// from 5 up.
    Pipe,
    /// LAWProj and M79GrenadeProjectile families (LAW, M79, M32, M203,
    /// Husk Gun, ZED guns): always disintegrates, a dud too.
    Launched,
}

impl ScreamTarget {
    /// CollisionRadius: PipeBombProjectile 8; the others keep
    /// Projectile's 0.
    fn collision_radius(self) -> f32 {
        if self == ScreamTarget::Pipe { 8.0 } else { 0.0 }
    }

    fn label(self) -> &'static str {
        match self {
            ScreamTarget::Nade => "frag",
            ScreamTarget::Pipe => "pipe",
            ScreamTarget::Launched => "launched",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScreamResult {
    Disintegrated,
    /// In reach, but its TakeDamage does nothing (a pipe bomb under 25).
    Ignored,
    /// The level is between the Siren and it (FastTrace).
    Blocked,
    OutOfRange,
}

impl ScreamResult {
    fn label(self) -> &'static str {
        match self {
            ScreamResult::Disintegrated => "disintegrated",
            ScreamResult::Ignored => "ignored",
            ScreamResult::Blocked => "blocked",
            ScreamResult::OutOfRange => "out_of_range",
        }
    }
}

/// ZombieSiren.HurtRadius on one projectile `dist` units from her centre:
/// VisibleCollidingActors within ScreamRadius (from the projectile's
/// centre: a guess for the engine's range test) and in sight, then
/// TakeDamage(int(damageScale x ScreamDamage), SirenScreamDamage) with
/// damageScale = 1 - max(0, (dist - CollisionRadius) / ScreamRadius).
/// Returns what happens and that damage.
fn scream_result(target: ScreamTarget, dist: f32, radius: f32, damage: f32, visible: bool) -> (ScreamResult, i32) {
    if dist > radius {
        return (ScreamResult::OutOfRange, 0);
    }
    if !visible {
        return (ScreamResult::Blocked, 0);
    }
    let scale = 1.0 - ((dist - target.collision_radius()) / radius).max(0.0);
    let amount = (scale * damage) as i32;
    let result = match target {
        ScreamTarget::Nade | ScreamTarget::Launched => ScreamResult::Disintegrated,
        // PipeBombProjectile.TakeDamage: (Damage < 25 && SirenScreamDamage)
        // returns; then Disintegrate if Damage >= 5 (always, past 25).
        // ScreamDamage is at most 14 (8 x 1.75), so KF's Sirens never
        // destroy a pipe bomb.
        ScreamTarget::Pipe if amount < 25 => ScreamResult::Ignored,
        ScreamTarget::Pipe => ScreamResult::Disintegrated,
    };
    (result, amount)
}

/// A Siren's scream pulse (the DoorBlast her HurtRadius sends, source
/// "siren_scream") reaching the player's grenades, rockets, frags and pipe
/// bombs: Disintegrate removes them without an explosion, with
/// DisintegrateSound (volume 2.0) and the SirenNadeDeflect emitter facing
/// up. KF hides the projectile at once and destroys it 0.1 s later; we
/// remove it at once. Also ends a MedicNade's healing cloud.
#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn scream_explosives(
    mut commands: Commands,
    mut screams: MessageReader<crate::world::door::DoorBlast>,
    spatial: SpatialQuery,
    explosives: Query<(Entity, &PlayerExplosive)>,
    thrown: Query<(Entity, &PlayerThrown)>,
    mut effects: Query<&mut crate::render::particles::ParticleEffect>,
    library: Option<Res<crate::render::particles::EffectLibrary>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut sounds: MessageWriter<crate::audio::mixer::PlaySound>,
) {
    let mut gone: Vec<Entity> = Vec::new();
    for s in screams.read().filter(|b| b.source == "siren_scream") {
        let from = coords::pos(s.at.to_array());
        // (entity, kind, class, weapon, id, position, sound radius, trail)
        let targets = explosives
            .iter()
            .map(|(e, p)| (e, ScreamTarget::Launched, p.stats.class, p.weapon, p.id, p.pos, p.stats.sounds, p.trail))
            .chain(thrown.iter().map(|(e, p)| {
                let kind = if matches!(p.stats.kind, ThrownKind::Pipe { .. }) { ScreamTarget::Pipe } else { ScreamTarget::Nade };
                (e, kind, p.stats.class, p.weapon, p.id, p.pos, p.stats.sounds, None)
            }));
        for (entity, kind, class, weapon, id, pos, snd, trail) in targets {
            if gone.contains(&entity) {
                continue;
            }
            let dist = (pos - s.at).length();
            let visible = dist <= s.radius && in_sight(&spatial, from, coords::pos(pos.to_array()));
            let (result, amount) = scream_result(kind, dist, s.radius, s.damage, visible);
            runlog::kv(
                "scream_explosive",
                &format!(
                    "result={} kind={} class={class} weapon={weapon} id={id} at_unreal=({:.0}, {:.0}, {:.0}) distance_unreal={dist:.0} damage={amount} scream_damage={} zed={}",
                    result.label(),
                    kind.label(),
                    pos.x,
                    pos.y,
                    pos.z,
                    s.damage,
                    s.zed.map_or("none".to_string(), |z| z.to_string())
                ),
            );
            if result != ScreamResult::Disintegrated {
                continue;
            }
            gone.push(entity);
            if let Some(t) = trail
                && let Ok(mut fx) = effects.get_mut(t)
            {
                fx.kill();
            }
            if let Some(d) = snd.disintegrate {
                sounds.write(sound_at(d, pos).volume(2.0).radius(snd.explode_radius));
            }
            if let Some(lib) = library.as_deref() {
                crate::render::particles::spawn_effect(&mut commands, lib, &mut meshes, "KFMod.SirenNadeDeflect", pos, crate::zeds::fireball::axes_along(Vec3::Z), id);
            }
            commands.entity(entity).despawn();
        }
    }
}

/// CrossbowArrow state OnWall: the player touching a stuck bolt (collision
/// 25 x 25 against the player's cylinder) takes it if the Crossbow has
/// room; it is gone at its LifeSpan.
fn pick_up_bolts(
    mut commands: Commands,
    time: Res<Time>,
    mut bolts: Query<(Entity, &mut StuckBolt)>,
    room: Res<BoltRoom>,
    player: Query<(&Transform, Option<&crate::player::walk::Walker>), With<crate::engine::camera::FlyCamera>>,
    mut picked: MessageWriter<BoltPickedUp>,
    mut sounds: MessageWriter<crate::audio::mixer::PlaySound>,
) {
    let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
    let player_ue = player.single().ok().map(|(t, w)| {
        to_ue(w.map_or(t.translation - Vec3::Y * crate::game::combat::PLAYER_EYE_HEIGHT * SCALE, |w| w.center)) / SCALE
    });
    for (e, mut b) in &mut bolts {
        b.life -= time.delta_secs();
        if b.life <= 0.0 {
            commands.entity(e).despawn();
            continue;
        }
        if let Some(pl) = player_ue {
            let d = b.pos - pl;
            let touching = d.truncate().length() <= 25.0 + PLAYER_RADIUS && d.z.abs() <= 25.0 + 50.0;
            if touching && room.0 {
                picked.write(BoltPickedUp);
                // CrossbowArrow: Ammo_GenericPickup, SLOT_Pain, 2 x 0.3, radius 400.
                sounds.write(sound_at("KF_InventorySnd.Ammo_GenericPickup", b.pos).slot(crate::audio::mixer::Slot::Pain).volume(0.6).radius(400.0));
                runlog::kv("bolt_picked_up", &format!("id={}", b.id));
                commands.entity(e).despawn();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scream_rules() {
        // A frag or a grenade 300 units away in sight: gone.
        assert_eq!(scream_result(ScreamTarget::Nade, 300.0, 700.0, 6.0, true), (ScreamResult::Disintegrated, 3));
        assert_eq!(scream_result(ScreamTarget::Launched, 10.0, 700.0, 14.0, true).0, ScreamResult::Disintegrated);
        // Behind a wall or past ScreamRadius: untouched.
        assert_eq!(scream_result(ScreamTarget::Nade, 300.0, 700.0, 6.0, false).0, ScreamResult::Blocked);
        assert_eq!(scream_result(ScreamTarget::Launched, 701.0, 700.0, 6.0, true).0, ScreamResult::OutOfRange);
        // A pipe bomb at her feet on Hell on Earth (14): under 25, ignored.
        assert_eq!(scream_result(ScreamTarget::Pipe, 5.0, 700.0, 14.0, true), (ScreamResult::Ignored, 14));
        // A (modded) scream of 25 or more would destroy it.
        assert_eq!(scream_result(ScreamTarget::Pipe, 8.0, 700.0, 30.0, true), (ScreamResult::Disintegrated, 30));
    }

    /// Flies a bouncer from z = 0 until it is back below 0; returns the
    /// distance and the apex height.
    fn bouncer_first_arc(speed: f32, angle_deg: f32, dt: f32) -> (f32, f32) {
        let a = angle_deg.to_radians();
        let mut vel = Vec3::new(speed * a.cos(), 0.0, speed * a.sin());
        let mut pos = Vec3::ZERO;
        let mut apex: f32 = 0.0;
        loop {
            pos += bouncer_fall_step(&mut vel, dt);
            apex = apex.max(pos.z);
            if pos.z < 0.0 {
                return (pos.x, apex);
            }
        }
    }

    #[test]
    fn frag_throw_falls_at_half_gravity() {
        // A quick frag throw (mHoldSpeedMin 850) at 45 degrees over flat
        // ground: range v^2 / g with g = 475 is 1521 units (it was 761 at
        // 950); apex v^2 sin^2 / 2g = 380 (was 190).
        let (range, apex) = bouncer_first_arc(850.0, 45.0, 1.0 / 60.0);
        assert!((range - 850.0 * 850.0 / 475.0).abs() < 25.0, "range {range}");
        assert!((apex - 850.0 * 850.0 * 0.5 / 950.0).abs() < 8.0, "apex {apex}");
        // Frame rate barely matters (KF steps at most 0.05 s).
        let (range20, _) = bouncer_first_arc(850.0, 45.0, 0.05);
        assert!((range20 - range).abs() < 40.0, "range at 20 fps {range20}");
    }

    /// Flies an M79 grenade (Speed 8000, StraightFlightTime 0.25, the
    /// ROBallisticProjectile defaults) fired level from z = 0 at 60 fps.
    /// Returns (time, position, speed) per frame until it is 300 below.
    fn m79_flight(new: bool) -> Vec<(f32, Vec3, f32)> {
        let b = Ballistics { bc_inverse: 1.0 / 0.3, speed_fudge: 1.0, min_fudge: 0.025, accel_time: 0.1 };
        let dt = 1.0 / 60.0;
        let (mut pos, mut vel, mut age, mut flight, mut falling) = (Vec3::ZERO, Vec3::new(8000.0, 0.0, 0.0), 0.0f32, 0.0f32, false);
        let mut out = Vec::new();
        while pos.z > -300.0 && age < 10.0 {
            age += dt;
            if age > 0.25 {
                falling = true;
            }
            let step = if new {
                if falling { fall_step(&mut vel, dt) } else { ballistic_step(&mut vel, &mut flight, &b, dt) }
            } else {
                // The old model: straight, then plain gravity, no cap.
                if falling {
                    vel.z -= GRAVITY * dt;
                }
                vel * dt
            };
            pos += step;
            out.push((age, pos, vel.length()));
        }
        out
    }

    #[test]
    fn m79_flies_by_kf_ballistics() {
        // Drag at the M79's Mach 0.39: G1 0.2155.
        assert!((g1(0.389) - 0.2155).abs() < 1e-4);
        assert!((g1(0.01) - 0.2629).abs() < 1e-4);
        let new = m79_flight(true);
        let old = m79_flight(false);
        let at = |f: &[(f32, Vec3, f32)], t: f32| f.iter().find(|s| s.0 >= t - 1e-4).copied().unwrap();
        // The first 0.1 s it ramps up from 2.5% speed: about half the
        // distance (old: 800).
        let (_, p01, _) = at(&new, 0.1);
        assert!((300.0..450.0).contains(&p01.x), "x at 0.1 s: {}", p01.x);
        // At 0.25 s: about 1600 units out, about 15 lower.
        let (_, p25, v25) = at(&new, 0.25);
        assert!((1450.0..1650.0).contains(&p25.x), "x at 0.25 s: {}", p25.x);
        assert!((-25.0..-8.0).contains(&p25.z), "z at 0.25 s: {}", p25.z);
        assert!(v25 > 7800.0 && v25 < 7950.0, "speed at 0.25 s: {v25}");
        // First falling step: capped at TerminalVelocity.
        let (_, _, v_fall) = at(&new, 0.26);
        assert!((v_fall - 2500.0).abs() < 1.0, "speed falling: {v_fall}");
        // Distance until 300 units below the muzzle: far shorter.
        let (t_new, end_new, _) = *new.last().unwrap();
        let (t_old, end_old, _) = *old.last().unwrap();
        println!("m79 to 300 below: new {:.0} units in {t_new:.2} s, old {:.0} units in {t_old:.2} s", end_new.x, end_old.x);
        assert!(end_new.x < 0.5 * end_old.x);
    }

    fn stats(r: f32, max: f32) -> ProjectileStats {
        ProjectileStats {
            pen_damage_reduction: r,
            max_penetrations: max,
            ..default()
        }
    }

    #[test]
    fn pellets_stop_after_kf_penetration_rule() {
        // Shotgun / Benelli / Trenchgun (0.5, 2): 1 -> 0.5 -> 0.25 <= 0.25.
        assert_eq!(penetration_limit(&stats(0.5, 2.0)), 2);
        // Hunting shotgun (0.65): 0.65, 0.42, 0.27 <= 0.325.
        assert_eq!(penetration_limit(&stats(0.65, 2.0)), 3);
        // AA12, KSG, nails (0.75): 0.75, 0.56, 0.42, 0.32 <= 0.375.
        assert_eq!(penetration_limit(&stats(0.75, 2.0)), 4);
    }
}
