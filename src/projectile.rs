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

use crate::coords::{self, SCALE};
use crate::runlog;
use crate::zed::Zed;

/// PhysicsVolume gravity (Unreal units/s^2), for nails after a bounce.
const GRAVITY: f32 = 950.0;

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
    pub fire: Option<crate::combat::FireType>,
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
    pub fire: Option<crate::combat::FireType>,
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
    pub life_span: f32,
    /// ZombieFleshPound.TakeDamage's multiplier for this damage type, if it
    /// is in its explosives list (None: the small-arms rule).
    pub fleshpound_mult: Option<f32>,
    /// Explode: the effect and decal.
    pub effect: &'static str,
    pub decal: crate::decals::DecalKind,
    /// The trail: PanzerfaustTrail (turned backward), or the Husk Gun's
    /// FlameThrowerHusk_*.
    pub trail: Option<&'static str>,
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
    /// HealBoostAmount: what it gives a teammate it touches (none here).
    pub heal: f32,
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
    fire: Option<crate::combat::FireType>,
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
const MODEL_CLASSES: [&str; 17] = [
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
    "KFMod.PipeBombProjectile",
    "KFMod.NailGunProjectile",
];

#[derive(Resource, Default)]
struct ProjectileModels(Vec<(String, crate::gore::PieceModel)>);

impl ProjectileModels {
    fn get(&self, class: &str) -> Option<&crate::gore::PieceModel> {
        self.0.iter().find(|(c, _)| c.eq_ignore_ascii_case(class)).map(|(_, m)| m)
    }
}

fn load_models(
    request: Res<crate::map::MapRequest>,
    mut models: ResMut<ProjectileModels>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    use ue_assets::properties::Value;
    let set = ue_assets::package_set::PackageSet::new(&request.install_root);
    let defaults = ue_assets::class_defaults::ClassDefaults::new(&set);
    for class_path in MODEL_CLASSES {
        let loaded = crate::gore::find_class(&set, class_path).ok_or_else(|| "class not found".to_string()).and_then(|class| {
            match defaults.get(&class, "StaticMeshRef") {
                Some((Value::Str(path), _)) if !path.is_empty() => crate::gore::load_piece_with_mesh(
                    &set,
                    &defaults,
                    &class,
                    &path,
                    class_path,
                    &mut meshes,
                    &mut images,
                    &mut materials,
                ),
                _ => crate::gore::load_piece(&set, &defaults, &class, class_path, &mut meshes, &mut images, &mut materials),
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
            .add_systems(Update, (spawn_projectiles, move_projectiles, move_explosives, move_thrown, move_flames, move_darts, kill_effects_after, sync_bodies).chain());
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
    mut bullet_fx: MessageWriter<crate::bullet_fx::BulletFx>,
    library: Option<Res<crate::particles::EffectLibrary>>,
    mut meshes: ResMut<Assets<Mesh>>,
    models: Res<ProjectileModels>,
) {
    for s in spawns.read() {
        *next_id += 1;
        let to_bevy = |v: Vec3| coords::pos(v.to_array());
        let mut origin = s.origin;
        let (a, b) = (to_bevy(s.trace_from), to_bevy(s.origin));
        if let Ok(d) = Dir3::new(b - a)
            && let Some(h) = spatial.cast_ray(a, d, (b - a).length(), true, &crate::collision::world_filter())
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
            runlog::kv("dart_fired", &format!("id={} weapon={} speed={} heal={}", *next_id, s.weapon, d.speed, d.heal));
            continue;
        }
        if let Some(fl) = s.flame {
            // PostBeginPlay: Velocity = Speed x the aim, + TossZ upward.
            let mut vel = s.dir.normalize_or_zero() * fl.speed;
            vel.z += fl.toss_z;
            let trail = library.as_deref().and_then(|lib| {
                crate::particles::spawn_effect(&mut commands, lib, &mut meshes, FLAME_TRAIL, origin, crate::fireball::axes_along(vel), *next_id)
            });
            commands.spawn(PlayerFlame {
                pos: origin,
                vel,
                stats: fl,
                fire: s.stats.fire,
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
                let axes = crate::fireball::axes_along(if backward { -dir } else { dir });
                crate::particles::spawn_effect(&mut commands, lib, &mut meshes, class, origin, axes, *next_id)
            });
            let e = commands.spawn(PlayerExplosive {
                pos: origin,
                vel: dir * x.speed,
                stats: x,
                weapon: s.weapon,
                age: 0.0,
                falling: false,
                dud: None,
                trail,
                id: *next_id,
            }).id();
            attach_model(&mut commands, &models, e, x.class, origin, dir);
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
                let wall = spatial.cast_ray(from, d, max, true, &crate::collision::world_filter()).map_or(max, |h| h.distance);
                let mut zed_t: Vec<f32> = zeds
                    .iter()
                    .filter(|z| z.health > 0.0)
                    .filter_map(|z| crate::combat::zed_hit(z, from, dir))
                    .filter(|&t| t < wall)
                    .collect();
                zed_t.sort_by(f32::total_cmp);
                let end_t = zed_t.get(penetration_limit(&s.stats).saturating_sub(1)).copied().unwrap_or(wall);
                bullet_fx.write(crate::bullet_fx::BulletFx {
                    shooter: crate::bullet_fx::Shooter::PlayerPellet((*next_id % PELLET_TRACERS) as u8),
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
    mut kills: ResMut<crate::combat::KillCount>,
    mut bullet_fx: MessageWriter<crate::bullet_fx::BulletFx>,
    player: Query<&Transform, With<crate::camera::FlyCamera>>,
) {
    let dt = time.delta_secs();
    let attacker = player.single().map_or(Vec3::ZERO, |t| t.translation - Vec3::Y * crate::combat::PLAYER_EYE_HEIGHT * SCALE);
    for (entity, mut p) in &mut projectiles {
        p.age += dt;
        if p.age >= p.stats.life_span {
            runlog::kv("projectile_expired", &format!("id={} weapon={} age={:.2}", p.id, p.weapon, p.age));
            commands.entity(entity).despawn();
            continue;
        }
        if p.falling {
            p.vel.z -= GRAVITY * dt;
        }
        let step = p.vel * dt;
        let len = step.length();
        if len <= 0.0 {
            continue;
        }
        let dir_ue = step / len;
        let from = coords::pos(p.pos.to_array());
        let dir = coords::dir(dir_ue.to_array()).normalize_or_zero();
        let Ok(dir3) = Dir3::new(dir) else { continue };
        let world = spatial.cast_ray(from, dir3, len * SCALE, true, &crate::collision::world_filter());
        let world_t = world.map_or(len * SCALE, |h| h.distance);
        // Zeds along this step, before the wall, nearest first.
        let mut hits: Vec<(f32, Mut<Zed>)> = Vec::new();
        for z in &mut zeds {
            if z.health <= 0.0 || p.hit.contains(&z.id) {
                continue;
            }
            if let Some(t) = crate::combat::zed_hit(&z, from, dir)
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
            let head = crate::combat::is_headshot(&z, point, dir, 1.0);
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
            let source = crate::combat::HitSource { point, attacker, melee: false, explosive: None, fire: p.stats.fire };
            crate::combat::damage_zed(&mut z, damage, head, p.stats.damage_type_headshot_mult, p.weapon, t, source, &mut kills);
            if p.stats.rule == PenRule::Bolt {
                p.damage /= 1.25;
                p.vel *= 0.85;
                continue;
            }
            p.damage *= p.stats.pen_damage_reduction;
            if p.damage / p.stats.damage <= p.stats.pen_damage_reduction / p.stats.max_penetrations.max(1e-3) {
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
                let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
                let n = if h.normal.dot(dir) > 0.0 { -h.normal } else { h.normal };
                let n_ue = to_ue(n).normalize_or_zero();
                let hit_ue = p.pos + dir_ue * (h.distance / SCALE);
                if p.bounces_left > 0 {
                    // NailGunProjectile.HitWall: reflect at 0.65 speed, fall.
                    p.bounces_left -= 1;
                    p.vel = 0.65 * (p.vel - 2.0 * n_ue * p.vel.dot(n_ue));
                    p.pos = hit_ue + n_ue;
                    p.falling = true;
                    runlog::kv("projectile_bounce", &format!("id={} weapon={} bounces_left={}", p.id, p.weapon, p.bounces_left));
                    continue;
                }
                // HitWall: ImpactEffect (ROBulletHitEffect) at the wall.
                bullet_fx.write(crate::bullet_fx::BulletFx {
                    shooter: crate::bullet_fx::Shooter::Player,
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
        .cast_ray(a, d, (b - a).length(), true, &crate::collision::world_filter())
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
    mut kills: ResMut<crate::combat::KillCount>,
    player: Query<(&Transform, Option<&crate::walk::Walker>), With<crate::camera::FlyCamera>>,
    mut effects: Query<&mut crate::particles::ParticleEffect>,
    library: Option<Res<crate::particles::EffectLibrary>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut decals: MessageWriter<crate::decals::SpawnDecal>,
    mut player_damage: MessageWriter<crate::combat::PlayerDamaged>,
) {
    let dt = time.delta_secs().min(0.1);
    let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
    // Instigator.Location: the player's cylinder centre, Unreal units.
    let player_ue = player.single().ok().map(|(t, w)| {
        to_ue(w.map_or(t.translation - Vec3::Y * crate::combat::PLAYER_EYE_HEIGHT * SCALE, |w| w.center)) / SCALE
    });
    for (entity, mut p) in &mut explosives {
        p.age += dt;
        let kill_trail = |effects: &mut Query<&mut crate::particles::ParticleEffect>, trail: Option<Entity>| {
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
            p.falling = true;
        }
        if p.falling {
            p.vel.z -= GRAVITY * dt;
        }
        let step = p.vel * dt;
        let len = step.length();
        if len <= 0.0 {
            continue;
        }
        let dir_ue = step / len;
        let from = coords::pos(p.pos.to_array());
        let dir = coords::dir(dir_ue.to_array()).normalize_or_zero();
        let Ok(dir3) = Dir3::new(dir) else { continue };
        let world = spatial.cast_ray(from, dir3, len * SCALE, true, &crate::collision::world_filter());
        let world_t = world.map_or(len * SCALE, |h| h.distance);
        let touched = if p.dud.is_some() {
            None
        } else {
            zeds.iter()
                .filter(|z| z.health > 0.0)
                .filter_map(|z| crate::combat::zed_hit(z, from, dir).filter(|&t| t <= world_t).map(|t| (t, z.id)))
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
                let head = crate::combat::is_headshot(&z, point, dir, 1.0);
                z.last_hit = Some((point, dir));
                let attacker = coords::pos(player_ue.unwrap_or(at).to_array());
                let source = crate::combat::HitSource { point, attacker, melee: false, explosive: None, fire: None };
                crate::combat::damage_zed(&mut z, p.stats.impact_damage, head, p.stats.impact_headshot_mult, p.weapon, t, source, &mut kills);
            }
            runlog::kv(
                "explosive_dud",
                &format!("id={} weapon={} hit={} distance_from_player_unreal={from_player:.0}", p.id, p.weapon, if zed.is_some() { "zed" } else { "level" }),
            );
            p.dud = Some(1.0);
            p.vel = Vec3::ZERO;
            p.pos = at;
            continue;
        }
        // HuskGunProjectile.ProcessTouch: ImpactDamage to the zed touched
        // (x HeadShotDamageMult on a headshot), then Explode.
        if let (Some(head_mult), Some(id)) = (p.stats.impact_on_touch, zed)
            && let Some(mut z) = zeds.iter_mut().find(|z| z.id == id)
        {
            let point = coords::pos(at.to_array());
            let head = crate::combat::is_headshot(&z, point, dir, 1.0);
            z.last_hit = Some((point, dir));
            let attacker = coords::pos(player_ue.unwrap_or(at).to_array());
            let source = crate::combat::HitSource { point, attacker, melee: false, explosive: None, fire: None };
            let damage = if head { p.stats.impact_damage * head_mult } else { p.stats.impact_damage };
            runlog::kv("explosive_impact", &format!("id={} weapon={} zed={id} damage={damage:.1} headshot={head}", p.id, p.weapon));
            crate::combat::damage_zed(&mut z, damage, head, p.stats.impact_headshot_mult, p.weapon, t, source, &mut kills);
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
                weapon: p.weapon,
                id: p.id,
            },
        );
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
    mut kills: ResMut<crate::combat::KillCount>,
    player: Query<(&Transform, Option<&crate::walk::Walker>), With<crate::camera::FlyCamera>>,
    mut effects: Query<&mut crate::particles::ParticleEffect>,
    library: Option<Res<crate::particles::EffectLibrary>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut decals: MessageWriter<crate::decals::SpawnDecal>,
    mut player_damage: MessageWriter<crate::combat::PlayerDamaged>,
    mut rng: Local<u32>,
) {
    let dt = time.delta_secs().min(0.1);
    let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
    let player_ue = player.single().ok().map(|(t, w)| {
        to_ue(w.map_or(t.translation - Vec3::Y * crate::combat::PLAYER_EYE_HEIGHT * SCALE, |w| w.center)) / SCALE
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
            if p.runs >= FLAME_TIMER_RUNS {
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
                    let world = spatial.cast_ray(from, dir3, len * SCALE, true, &crate::collision::world_filter());
                    let world_t = world.map_or(len * SCALE, |h| h.distance);
                    let touched = zeds
                        .iter()
                        .filter(|z| z.health > 0.0)
                        .filter_map(|z| crate::combat::zed_hit(z, from, dir).filter(|&t| t <= world_t))
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
        );
        runlog::kv(
            "flame_burst",
            &format!(
                "id={} weapon={} hit={hit} at_unreal=({:.0}, {:.0}, {:.0}) age={:.2} zeds_hit={zeds_hit} self_damage={self_damage}",
                p.id, p.weapon, at.x, at.y, at.z, p.age
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
    kills: &mut crate::combat::KillCount,
    player_ue: Option<Vec3>,
    library: Option<&crate::particles::EffectLibrary>,
    meshes: &mut Assets<Mesh>,
    decals: &mut MessageWriter<crate::decals::SpawnDecal>,
    player_damage: &mut MessageWriter<crate::combat::PlayerDamaged>,
    p: &PlayerFlame,
    at: Vec3,
    normal: Vec3,
) -> (u32, f32) {
    let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
    let (damage, radius) = (p.stats.damage, p.stats.radius);
    decals.write(crate::decals::SpawnDecal {
        kind: crate::decals::DecalKind::Scorch,
        at,
        dir: -normal,
        trace: false,
    });
    if let Some(lib) = library
        && let Some(e) = crate::particles::spawn_effect(commands, lib, meshes, FUEL_FLAME, at, Mat3::IDENTITY, p.id)
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
        let source = crate::combat::HitSource { point, attacker: at_bevy, melee: false, explosive: None, fire: p.fire };
        crate::combat::damage_zed(&mut z, scale * damage, false, 1.0, p.weapon, dist * SCALE, source, kills);
        zeds_hit += 1;
    }
    let mut self_damage = 0.0;
    if let Some(pl) = player_ue {
        let dist = (pl - at).length().max(1.0);
        if dist - PLAYER_RADIUS <= radius && in_sight(spatial, at_bevy, coords::pos(pl.to_array())) {
            self_damage = ((1.0 - ((dist - PLAYER_RADIUS) / radius).max(0.0)) * damage).floor();
            if self_damage > 0.0 {
                player_damage.write(crate::combat::PlayerDamaged {
                    amount: self_damage,
                    zed_id: crate::combat::SELF_DAMAGE,
                    kind: crate::combat::HurtKind::Fire,
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
    mut bullet_fx: MessageWriter<crate::bullet_fx::BulletFx>,
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
        let world = spatial.cast_ray(from, dir3, len * SCALE, true, &crate::collision::world_filter());
        let world_t = world.map_or(len * SCALE, |h| h.distance);
        let zed = zeds
            .iter()
            .filter(|z| z.health > 0.0)
            .filter_map(|z| crate::combat::zed_hit(z, from, dir).filter(|&t| t <= world_t).map(|t| (t, z.id)))
            .min_by(|a, b| a.0.total_cmp(&b.0));
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
        bullet_fx.write(crate::bullet_fx::BulletFx {
            shooter: crate::bullet_fx::Shooter::Player,
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
        let world = spatial.cast_ray(from, dir3, max, true, &crate::collision::world_filter()).map(|h| h.distance);
        let wall_t = world.unwrap_or(max);
        let hit = zeds
            .iter()
            .filter(|z| z.health > 0.0)
            .filter_map(|z| crate::combat::zed_hit(z, from, dir).filter(|&t| t <= wall_t).map(|t| (t, z.id)))
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
    mut effects: Query<&mut crate::particles::ParticleEffect>,
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
    decal: crate::decals::DecalKind,
    damage: f32,
    radius: f32,
    fleshpound_mult: Option<f32>,
    /// A burning damage type (the Husk Gun's).
    fire: Option<crate::combat::FireType>,
    hurts_self: bool,
    zap: Option<f32>,
    weapon: &'static str,
    id: u32,
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
    kills: &mut crate::combat::KillCount,
    player_ue: Option<Vec3>,
    library: Option<&crate::particles::EffectLibrary>,
    meshes: &mut Assets<Mesh>,
    decals: &mut MessageWriter<crate::decals::SpawnDecal>,
    player_damage: &mut MessageWriter<crate::combat::PlayerDamaged>,
    b: &Blast,
) -> (u32, u32, f32) {
    let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
    let at = b.at;
    if let Some(lib) = library {
        crate::particles::spawn_effect(commands, lib, meshes, b.effect, at + b.normal * b.effect_offset, crate::fireball::axes_along(b.normal), b.id);
    }
    decals.write(crate::decals::SpawnDecal {
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
        let source = crate::combat::HitSource { point, attacker: at_bevy, melee: false, explosive: b.fleshpound_mult, fire: b.fire };
        let before = z.health;
        crate::combat::damage_zed(&mut z, scale * b.damage, false, 1.0, b.weapon, dist * SCALE, source, kills);
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
            self_damage = crate::combat::reduce_self_damage(raw);
            if raw >= 1.0 {
                player_damage.write(crate::combat::PlayerDamaged {
                    amount: raw,
                    zed_id: crate::combat::SELF_DAMAGE,
                    kind: crate::combat::HurtKind::Plain,
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
    pub speed: f32,
    pub damage: f32,
    pub radius: f32,
    /// HitWall: Velocity = -VNorm x DampenFactor + (Velocity - VNorm) x
    /// DampenFactorParallel; at rest under 20.
    pub dampen_normal: f32,
    pub dampen_parallel: f32,
    pub fleshpound_mult: f32,
    pub effect: &'static str,
    pub decal: crate::decals::DecalKind,
    pub kind: ThrownKind,
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
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)] // Bevy system parameters
fn move_thrown(
    mut commands: Commands,
    time: Res<Time>,
    spatial: SpatialQuery,
    mut thrown: Query<(Entity, &mut PlayerThrown)>,
    mut zeds: Query<&mut Zed>,
    mut kills: ResMut<crate::combat::KillCount>,
    player: Query<(&Transform, Option<&crate::walk::Walker>), With<crate::camera::FlyCamera>>,
    library: Option<Res<crate::particles::EffectLibrary>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut decals: MessageWriter<crate::decals::SpawnDecal>,
    mut player_damage: MessageWriter<crate::combat::PlayerDamaged>,
) {
    let dt = time.delta_secs().min(0.1);
    let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
    let player_ue = player.single().ok().map(|(t, w)| {
        to_ue(w.map_or(t.translation - Vec3::Y * crate::combat::PLAYER_EYE_HEIGHT * SCALE, |w| w.center)) / SCALE
    });
    for (entity, mut p) in &mut thrown {
        p.age += dt;
        // Fly: PHYS_Falling, bouncing off the level; a zed stops it dead.
        if !p.resting {
            p.vel.z -= GRAVITY * dt;
            let step = p.vel * dt;
            let len = step.length();
            if len > 0.0 {
                let dir_ue = step / len;
                let from = coords::pos(p.pos.to_array());
                let dir = coords::dir(dir_ue.to_array()).normalize_or_zero();
                if let Ok(dir3) = Dir3::new(dir) {
                    let world = spatial.cast_ray(from, dir3, len * SCALE, true, &crate::collision::world_filter());
                    let world_t = world.map_or(len * SCALE, |h| h.distance);
                    let zed_touch = zeds.iter().any(|z| {
                        z.health > 0.0 && crate::combat::zed_hit(z, from, dir).is_some_and(|t| t <= world_t)
                    });
                    if zed_touch {
                        // ProcessTouch: Velocity = 0 (then falls).
                        p.vel = Vec3::ZERO;
                    } else if let Some(h) = world {
                        let n = if h.normal.dot(dir) > 0.0 { -h.normal } else { h.normal };
                        let n = to_ue(n).normalize_or_zero();
                        p.pos += dir_ue * (h.distance / SCALE) + n;
                        let v_norm = p.vel.dot(n) * n;
                        p.vel = -v_norm * p.stats.dampen_normal + (p.vel - v_norm) * p.stats.dampen_parallel;
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
                            // Fast beeps, then the explosion.
                            *c = c.saturating_sub(1);
                            explode = *c == 0;
                            p.timer = 0.15;
                        } else {
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
        let normal = Vec3::Z;
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
            &Blast {
                at: p.pos,
                normal,
                effect_offset: 0.0,
                effect: p.stats.effect,
                decal: p.stats.decal,
                damage: p.stats.damage,
                radius: p.stats.radius,
                fleshpound_mult: Some(p.stats.fleshpound_mult),
                fire: None,
                hurts_self: true,
                zap: None,
                weapon: p.weapon,
                id: p.id,
            },
        );
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


/// CrossbowArrow state OnWall: the player touching a stuck bolt (collision
/// 25 x 25 against the player's cylinder) takes it if the Crossbow has
/// room; it is gone at its LifeSpan.
fn pick_up_bolts(
    mut commands: Commands,
    time: Res<Time>,
    mut bolts: Query<(Entity, &mut StuckBolt)>,
    room: Res<BoltRoom>,
    player: Query<(&Transform, Option<&crate::walk::Walker>), With<crate::camera::FlyCamera>>,
    mut picked: MessageWriter<BoltPickedUp>,
) {
    let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
    let player_ue = player.single().ok().map(|(t, w)| {
        to_ue(w.map_or(t.translation - Vec3::Y * crate::combat::PLAYER_EYE_HEIGHT * SCALE, |w| w.center)) / SCALE
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
                runlog::kv("bolt_picked_up", &format!("id={}", b.id));
                commands.entity(e).despawn();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
