//! The Husk's fireball: KFChar.HuskFireProjectile (a KFMod.LAWProj). It
//! flies straight at Speed 1800 (no gravity) for up to LifeSpan 10 s with a
//! FlameThrowerFlameB trail, and explodes on the first thing it touches
//! (ArmDistSquared 0: always armed): the level, the player, or another zed.
//! Explode: FlameImpact 20 out from the surface, a FlameThrowerBurnMark
//! decal, and HurtRadius(Damage 25, DamageRadius 150, DamTypeBurned,
//! MomentumTransfer 125000), Damage x the difficulty's scale
//! (HuskFireProjectile.PostBeginPlay: Normal 1.0). On the player:
//! damageScale = 1 - (distance - 20) / 150, times KFPawn.GetExposureTo
//! (half for each of head and root in sight of the blast), damage x scale;
//! momentum pushes away from the blast. Burn damage sets the player on fire
//! (combat.rs). Damage to other zeds and the view shake are not done.
//!
//! The Patriarch's rocket, KFChar.BossLAWProj (a LAWProj too), flies and
//! explodes the same way with its own values: Speed 2600, Damage 200 x the
//! difficulty's scale (BossLAWProj.PostBeginPlay, one player: Normal
//! 0.375 = 75) in DamageRadius 500, DamTypeFrag, the
//! KillingFloorStatics.LAWRocket mesh (StaticMeshRef) at DrawScale 0.7, a
//! PanzerfaustTrail (turned to point backward), LawExplosion and a
//! RocketMarkDirt decal.

use avian3d::prelude::*;
use bevy::prelude::*;
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package_set::PackageSet;

use crate::engine::camera::FlyCamera;
use crate::engine::coords::{self, SCALE};
use crate::render::decals::{DecalKind, SpawnDecal};
use crate::zeds::gore::{self, PieceModel};
use crate::world::map::MapRequest;
use crate::render::particles::{self, EffectLibrary, ParticleEffect};
use crate::engine::runlog;

const LIFE_SPAN: f32 = 10.0;
const RADIUS: f32 = 2.0;
/// Player cylinder (KFPawn) and its head and root bones above the centre
/// (approximate: no player skeleton yet), Unreal units.
const PLAYER_RADIUS: f32 = 20.0;
const PLAYER_HALF_HEIGHT: f32 = 50.0;
const PLAYER_EYE: f32 = 44.0;
const PLAYER_HEAD: f32 = 40.0;

/// Which projectile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Projectile {
    /// HuskFireProjectile.
    HuskFire,
    /// BossLAWProj.
    BossRocket,
}

/// A projectile's values (class defaults, the game's difficulty, one player).
struct Spec {
    class: &'static str,
    /// StaticMeshRef, for classes that only name their mesh.
    mesh: Option<&'static str>,
    speed: f32,
    damage: f32,
    radius: f32,
    momentum: f32,
    trail: &'static str,
    /// The trail points backward (PanzerfaustTrail: RelativeRotation pitch 32768).
    trail_backward: bool,
    impact: &'static str,
    /// AmbientSound while flying (LAWProj SoundVolume 255, SoundRadius
    /// 250) and ExplosionSound (Explode: PlaySound(ExplosionSound,,2.0),
    /// TransientSoundRadius 500); from the class defaults.
    flight_sound: &'static str,
    explosion_sound: &'static str,
    decal: DecalKind,
    hurt: crate::game::combat::HurtKind,
    /// MyDamageType (HuskFireProjectile DamTypeBurned, BossLAWProj
    /// DamTypeFrag), for the perks' ReduceDamage.
    dam: &'static str,
}

impl Projectile {
    const ALL: [Projectile; 2] = [Projectile::HuskFire, Projectile::BossRocket];

    fn spec(self) -> Spec {
        match self {
            Projectile::HuskFire => Spec {
                class: "KFChar.HuskFireProjectile",
                mesh: None,
                speed: 1800.0,
                damage: 25.0 * crate::game::difficulty::current().husk_fire_damage_scale(),
                radius: 150.0,
                momentum: 125000.0,
                trail: "KFMod.FlameThrowerFlameB",
                trail_backward: false,
                impact: "KFMod.FlameImpact",
                flight_sound: "KF_BaseHusk.Fire.husk_fireball_loop",
                explosion_sound: "KF_EnemiesFinalSnd.Husk.Husk_FireImpact",
                decal: DecalKind::Scorch,
                hurt: crate::game::combat::HurtKind::Fire,
                dam: "DamTypeBurned",
            },
            Projectile::BossRocket => Spec {
                class: "KFChar.BossLAWProj",
                mesh: Some("KillingFloorStatics.LAWRocket"),
                speed: 2600.0,
                damage: 200.0 * crate::game::difficulty::current().boss_rocket_scale(true),
                radius: 500.0,
                momentum: 125000.0,
                trail: "ROEffects.PanzerfaustTrail",
                trail_backward: true,
                impact: "KFMod.LawExplosion",
                flight_sound: "KF_LAWSnd.Rocket_Propel",
                explosion_sound: "KF_LAWSnd.Rocket_Explode",
                decal: DecalKind::RocketMark,
                hurt: crate::game::combat::HurtKind::Plain,
                dam: "DamTypeFrag",
            },
        }
    }

    /// Flight speed (Unreal units/s), for leading the target.
    pub fn speed(self) -> f32 {
        self.spec().speed
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// A projectile to spawn: start and direction, Unreal units.
#[derive(Message, Clone, Copy, Debug)]
pub struct SpawnFireball {
    pub at: Vec3,
    pub dir: Vec3,
    pub zed_id: usize,
    pub kind: Projectile,
}

#[derive(Resource, Default)]
struct FireballModel([Option<PieceModel>; 2]);

#[derive(Component)]
#[require(crate::world::map_change::MapScoped)]
struct Fireball {
    id: u32,
    kind: Projectile,
    zed_id: usize,
    at: Vec3,
    velocity: Vec3,
    age: f32,
    trail: Option<Entity>,
}

pub struct FireballPlugin;

impl Plugin for FireballPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SpawnFireball>()
            .init_resource::<FireballModel>()
            .add_systems(PostStartup, load_model)
            .add_systems(Update, (spawn_fireballs, move_fireballs).chain());
    }
}

fn load_model(
    request: Res<MapRequest>,
    mut model: ResMut<FireballModel>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut preload: MessageWriter<crate::audio::mixer::PreloadSounds>,
) {
    let set = PackageSet::new(&request.install_root);
    let defaults = ClassDefaults::new(&set);
    preload.write(crate::audio::mixer::PreloadSounds {
        what: "fireballs".into(),
        per_map: false,
        sounds: Projectile::ALL.iter().flat_map(|k| [k.spec().flight_sound.to_string(), k.spec().explosion_sound.to_string()]).collect(),
    });
    for kind in Projectile::ALL {
        let spec = kind.spec();
        let class_path = spec.class;
        let loaded = gore::find_class(&set, class_path)
            .ok_or_else(|| "class not found".to_string())
            .and_then(|class| match spec.mesh {
                Some(mesh) => gore::load_piece_with_mesh(&set, &defaults, &class, mesh, class_path, &mut meshes, &mut images, &mut materials),
                None => gore::load_piece(&set, &defaults, &class, class_path, &mut meshes, &mut images, &mut materials),
            });
        match loaded {
            Ok(m) => {
                runlog::kv("fireball_loaded", &format!("class={class_path} draw_scale={}", m.draw_scale()));
                model.0[kind.index()] = Some(m);
            }
            Err(e) => runlog::kv("fireball_load_error", &format!("class={class_path} error=\"{e}\"")),
        }
    }
}

pub(crate) fn axes_along(d: Vec3) -> Mat3 {
    let k = 65536.0 / std::f32::consts::TAU;
    coords::ue_rotation_matrix(ue_assets::properties::Rotator {
        pitch: (d.z.clamp(-1.0, 1.0).asin() * k) as i32,
        yaw: (d.y.atan2(d.x) * k) as i32,
        roll: 0,
    })
}

#[allow(clippy::too_many_arguments)]
fn spawn_fireballs(
    mut commands: Commands,
    mut requests: MessageReader<SpawnFireball>,
    model: Res<FireballModel>,
    library: Option<Res<EffectLibrary>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut next_id: Local<u32>,
) {
    for r in requests.read() {
        *next_id += 1;
        let d = r.dir.normalize_or_zero();
        let axes = axes_along(d);
        let k = 65536.0 / std::f32::consts::TAU;
        let rot = ue_assets::properties::Rotator {
            pitch: (d.z.clamp(-1.0, 1.0).asin() * k) as i32,
            yaw: (d.y.atan2(d.x) * k) as i32,
            roll: 0,
        };
        let spec = r.kind.spec();
        let trail_axes = if spec.trail_backward { axes_along(-d) } else { axes };
        let trail = library
            .as_deref()
            .and_then(|lib| particles::spawn_effect(&mut commands, lib, &mut meshes, spec.trail, r.at, trail_axes, *next_id));
        let model = model.0[r.kind.index()].as_ref();
        let scale = model.map_or(1.0, |m| m.draw_scale());
        let e = commands
            .spawn((
                Transform {
                    translation: coords::pos(r.at.to_array()),
                    rotation: coords::rotation(rot),
                    scale: Vec3::splat(scale),
                },
                Visibility::Visible,
                crate::audio::mixer::AmbientSound { sound: spec.flight_sound.into(), volume: 255, radius: 250.0, pitch: 64, at_listener: false, ..default() },
                Fireball {
                    id: *next_id,
                    kind: r.kind,
                    zed_id: r.zed_id,
                    at: r.at,
                    velocity: d * spec.speed,
                    age: 0.0,
                    trail,
                },
            ))
            .id();
        if let Some(m) = model {
            m.spawn_parts(&mut commands, e);
        }
        runlog::kv(
            "fireball_spawned",
            &format!(
                "fireball={} kind={:?} zed={} at_unreal=({:.0}, {:.0}, {:.0}) dir=({:.2}, {:.2}, {:.2})",
                *next_id, r.kind, r.zed_id, r.at.x, r.at.y, r.at.z, d.x, d.y, d.z
            ),
        );
    }
}

fn in_sight(spatial: &SpatialQuery, a: Vec3, b: Vec3) -> bool {
    let (from, to) = (coords::pos(a.to_array()), coords::pos(b.to_array()));
    let Ok(d) = Dir3::new(to - from) else {
        return true;
    };
    spatial
        .cast_ray(from, d, (to - from).length(), true, &crate::world::collision::world_filter())
        .is_none()
}

/// Where a segment first enters an upright cylinder (centre, radius, half
/// height; Unreal units): the fraction along it, if it does.
fn segment_hits_cylinder(a: Vec3, b: Vec3, centre: Vec3, radius: f32, half: f32) -> Option<f32> {
    let steps = 8;
    (0..=steps).map(|i| i as f32 / steps as f32).find(|&f| {
        let p = a + (b - a) * f;
        (p - centre).truncate().length() <= radius && (p.z - centre.z).abs() <= half
    })
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn move_fireballs(
    mut commands: Commands,
    time: Res<Time>,
    spatial: SpatialQuery,
    player: Query<(&Transform, Option<&crate::player::walk::Walker>), (With<FlyCamera>, Without<Fireball>)>,
    zeds: Query<&crate::zeds::zed::Zed>,
    mut fireballs: Query<(Entity, &mut Fireball, &mut Transform)>,
    mut effects: Query<&mut ParticleEffect>,
    library: Option<Res<EffectLibrary>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut damage: MessageWriter<crate::game::combat::PlayerDamaged>,
    mut push: MessageWriter<crate::player::walk::PlayerPush>,
    mut decals: MessageWriter<SpawnDecal>,
    (door_colliders, mut door_blasts, mut sounds): (Query<&crate::world::door::DoorCollider>, MessageWriter<crate::world::door::DoorBlast>, MessageWriter<crate::audio::mixer::PlaySound>),
    (remote, net): (Res<crate::game::combat::RemotePlayers>, Option<Res<crate::net::NetMode>>),
) {
    let dt = time.delta_secs().min(0.1);
    let to_ue = |c: Vec3| Vec3::new(-c.z, c.x, c.y) / SCALE;
    // A network client's projectiles are copies of the host's: they fly
    // and explode on the client's screen but hurt nobody (the host's
    // explosion hurts the players and sends the hits to their games).
    let harmless = net.is_some_and(|n| matches!(*n, crate::net::NetMode::Client { .. }));
    let local = player.single().ok().map(|(t, w)| w.map_or(t.translation - Vec3::Y * PLAYER_EYE * SCALE, |w| w.center));
    // Every player it can touch (Unreal units); a host also the others.
    let targets = remote.targets(local);
    for (e, mut f, mut t) in &mut fireballs {
        f.age += dt;
        if f.age > LIFE_SPAN {
            if let Some(trail) = f.trail
                && let Ok(mut fx) = effects.get_mut(trail)
            {
                fx.kill();
            }
            commands.entity(e).despawn();
            continue;
        }
        let step = f.velocity * dt;
        let (a, b) = (f.at, f.at + step);
        // The first thing touched: the level, the player or a zed.
        let mut hit: Option<(f32, Vec3, &str)> = None;
        let mut level_door = None;
        if let Ok(d) = Dir3::new(coords::dir(step.to_array()))
            && let Some(h) = spatial.cast_ray(coords::pos(a.to_array()), d, (step.length() + RADIUS) * SCALE, true, &crate::world::collision::world_filter())
        {
            let n = h.normal;
            let n = to_ue(if n.dot(*d) > 0.0 { -n } else { n }) * SCALE;
            level_door = door_colliders.get(h.entity).ok().map(|c| c.0);
            hit = Some(((h.distance / SCALE / step.length()).min(1.0), n, if level_door.is_some() { "door" } else { "level" }));
        }
        for &(_, p) in &targets {
            if let Some(frac) = segment_hits_cylinder(a, b, p, PLAYER_RADIUS + RADIUS, PLAYER_HALF_HEIGHT + RADIUS)
                && hit.is_none_or(|h| frac < h.0)
            {
                let at = a + step * frac;
                hit = Some((frac, (at - p).normalize_or_zero(), "player"));
            }
        }
        for z in &zeds {
            if z.id == f.zed_id {
                continue;
            }
            if let Some(c) = z.blocking_cylinder() {
                let centre = to_ue(c.centre);
                if let Some(frac) = segment_hits_cylinder(a, b, centre, c.radius / SCALE + RADIUS, c.half_height / SCALE + RADIUS)
                    && hit.is_none_or(|h| frac < h.0)
                {
                    let at = a + step * frac;
                    hit = Some((frac, (at - centre).normalize_or_zero(), "zed"));
                }
            }
        }
        let Some((frac, normal, what)) = hit else {
            f.at = b;
            t.translation = coords::pos(f.at.to_array());
            if let Some(trail) = f.trail
                && let Ok(mut fx) = effects.get_mut(trail)
            {
                fx.frame.0 = f.at;
            }
            continue;
        };
        // Explode.
        let spec = f.kind.spec();
        let at = a + step * frac;
        sounds.write(crate::audio::mixer::PlaySound::new(spec.explosion_sound, crate::audio::mixer::Emitter::Point(coords::pos(at.to_array()))).volume(2.0).radius(500.0));
        if let Some(lib) = library.as_deref() {
            particles::spawn_effect(&mut commands, lib, &mut meshes, spec.impact, at + normal * 20.0, axes_along(normal), f.id);
        }
        decals.write(SpawnDecal {
            kind: spec.decal,
            at,
            dir: -normal,
            trace: false,
        });
        // Projectile.HitWall on a door, then LAWProj.HurtRadius
        // (CollidingActors: no line-of-sight test) on the doors around.
        if !harmless {
            door_blasts.write(crate::world::door::DoorBlast {
                at,
                radius: spec.radius,
                damage: spec.damage,
                zed: Some(f.zed_id),
                direct: if what == "door" { level_door } else { None },
                line_of_sight: false,
                frag: false,
                source: if f.kind == Projectile::BossRocket { "boss_rocket" } else { "husk_fireball" },
            });
        }
        let mut dealt = 0.0;
        let mut others = Vec::new();
        for &(who, p) in targets.iter().filter(|_| !harmless) {
            let dist = (p - at).length().max(1.0);
            if dist - PLAYER_RADIUS <= spec.radius {
                let exposure = 0.5 * in_sight(&spatial, at, p + Vec3::Z * PLAYER_HEAD) as u8 as f32 + 0.5 * in_sight(&spatial, at, p) as u8 as f32;
                let scale = (1.0 - ((dist - PLAYER_RADIUS) / spec.radius).max(0.0)) * exposure;
                if scale > 0.0 {
                    let amount = (scale * spec.damage).floor();
                    match who {
                        None => dealt = amount,
                        Some(peer) => others.push(format!("{peer}:{amount}")),
                    }
                    if amount > 0.0 {
                        damage.write(crate::game::combat::PlayerDamaged {
                            amount,
                            armor_stops: true,
                            zed_id: f.zed_id,
                            kind: spec.hurt,
                            // DamTypeBurned (Husk), DamTypeFrag (Patriarch rocket).
                            dam_type: crate::game::combat::DamType::Other,
                            source: Some(coords::pos(at.to_array())),
                            dam: Some(crate::game::perks::known_dam_type(spec.dam)),
                            to_peer: who,
                        });
                    }
                    push.write(crate::player::walk::PlayerPush {
                        momentum: (p - at) / dist * (scale * spec.momentum),
                        to_peer: who,
                    });
                }
            }
        }
        runlog::kv(
            "fireball_exploded",
            &format!(
                "fireball={} kind={:?} hit={what} at_unreal=({:.0}, {:.0}, {:.0}) age={:.2} player_damage={dealt} other_players=[{}] harmless={harmless}",
                f.id, f.kind, at.x, at.y, at.z, f.age, others.join(" ")
            ),
        );
        if let Some(trail) = f.trail
            && let Ok(mut fx) = effects.get_mut(trail)
        {
            fx.kill();
        }
        commands.entity(e).despawn();
    }
}
