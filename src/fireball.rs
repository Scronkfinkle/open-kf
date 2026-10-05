//! The Husk's fireball: KFChar.HuskFireProjectile (a KFMod.LAWProj). It
//! flies straight at Speed 1800 (no gravity) for up to LifeSpan 10 s with a
//! FlameThrowerFlameB trail, and explodes on the first thing it touches
//! (ArmDistSquared 0: always armed): the level, the player, or another zed.
//! Explode: FlameImpact 20 out from the surface, a FlameThrowerBurnMark
//! decal, and HurtRadius(Damage 25, DamageRadius 150, DamTypeBurned,
//! MomentumTransfer 125000), Normal difficulty. On the player:
//! damageScale = 1 - (distance - 20) / 150, times KFPawn.GetExposureTo
//! (half for each of head and root in sight of the blast), damage x scale;
//! momentum pushes away from the blast. Burn damage sets the player on fire
//! (combat.rs). Damage to other zeds and the view shake are not done.
//!
//! The Patriarch's rocket, KFChar.BossLAWProj (a LAWProj too), flies and
//! explodes the same way with its own values: Speed 2600, Damage 200 x 0.375
//! (one player, Normal) = 75 in DamageRadius 500, DamTypeFrag, the
//! KillingFloorStatics.LAWRocket mesh (StaticMeshRef) at DrawScale 0.7, a
//! PanzerfaustTrail (turned to point backward), LawExplosion and a
//! RocketMarkDirt decal.

use avian3d::prelude::*;
use bevy::prelude::*;
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package_set::PackageSet;

use crate::camera::FlyCamera;
use crate::coords::{self, SCALE};
use crate::decals::{DecalKind, SpawnDecal};
use crate::gore::{self, PieceModel};
use crate::map::MapRequest;
use crate::particles::{self, EffectLibrary, ParticleEffect};
use crate::runlog;

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

/// A projectile's values (class defaults, Normal difficulty, one player).
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
    decal: DecalKind,
    hurt: crate::combat::HurtKind,
}

impl Projectile {
    const ALL: [Projectile; 2] = [Projectile::HuskFire, Projectile::BossRocket];

    fn spec(self) -> Spec {
        match self {
            Projectile::HuskFire => Spec {
                class: "KFChar.HuskFireProjectile",
                mesh: None,
                speed: 1800.0,
                damage: 25.0,
                radius: 150.0,
                momentum: 125000.0,
                trail: "KFMod.FlameThrowerFlameB",
                trail_backward: false,
                impact: "KFMod.FlameImpact",
                decal: DecalKind::Scorch,
                hurt: crate::combat::HurtKind::Fire,
            },
            Projectile::BossRocket => Spec {
                class: "KFChar.BossLAWProj",
                mesh: Some("KillingFloorStatics.LAWRocket"),
                speed: 2600.0,
                damage: 200.0 * 0.375,
                radius: 500.0,
                momentum: 125000.0,
                trail: "ROEffects.PanzerfaustTrail",
                trail_backward: true,
                impact: "KFMod.LawExplosion",
                decal: DecalKind::RocketMark,
                hurt: crate::combat::HurtKind::Plain,
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
) {
    let set = PackageSet::new(&request.install_root);
    let defaults = ClassDefaults::new(&set);
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
        .cast_ray(from, d, (to - from).length(), true, &crate::collision::world_filter())
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
    player: Query<(&Transform, Option<&crate::walk::Walker>), (With<FlyCamera>, Without<Fireball>)>,
    zeds: Query<&crate::zed::Zed>,
    mut fireballs: Query<(Entity, &mut Fireball, &mut Transform)>,
    mut effects: Query<&mut ParticleEffect>,
    library: Option<Res<EffectLibrary>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut damage: MessageWriter<crate::combat::PlayerDamaged>,
    mut push: MessageWriter<crate::walk::PlayerPush>,
    mut decals: MessageWriter<SpawnDecal>,
    (door_colliders, mut door_blasts): (Query<&crate::door::DoorCollider>, MessageWriter<crate::door::DoorBlast>),
) {
    let dt = time.delta_secs().min(0.1);
    let to_ue = |c: Vec3| Vec3::new(-c.z, c.x, c.y) / SCALE;
    let player = player
        .single()
        .ok()
        .map(|(t, w)| to_ue(w.map_or(t.translation - Vec3::Y * PLAYER_EYE * SCALE, |w| w.center)));
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
            && let Some(h) = spatial.cast_ray(coords::pos(a.to_array()), d, (step.length() + RADIUS) * SCALE, true, &crate::collision::world_filter())
        {
            let n = h.normal;
            let n = to_ue(if n.dot(*d) > 0.0 { -n } else { n }) * SCALE;
            level_door = door_colliders.get(h.entity).ok().map(|c| c.0);
            hit = Some(((h.distance / SCALE / step.length()).min(1.0), n, if level_door.is_some() { "door" } else { "level" }));
        }
        if let Some(p) = player
            && let Some(frac) = segment_hits_cylinder(a, b, p, PLAYER_RADIUS + RADIUS, PLAYER_HALF_HEIGHT + RADIUS)
            && hit.is_none_or(|h| frac < h.0)
        {
            let at = a + step * frac;
            hit = Some((frac, (at - p).normalize_or_zero(), "player"));
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
        door_blasts.write(crate::door::DoorBlast {
            at,
            radius: spec.radius,
            damage: spec.damage,
            zed: Some(f.zed_id),
            direct: if what == "door" { level_door } else { None },
            line_of_sight: false,
            frag: false,
            source: if f.kind == Projectile::BossRocket { "boss_rocket" } else { "husk_fireball" },
        });
        let mut dealt = 0.0;
        if let Some(p) = player {
            let dist = (p - at).length().max(1.0);
            if dist - PLAYER_RADIUS <= spec.radius {
                let exposure = 0.5 * in_sight(&spatial, at, p + Vec3::Z * PLAYER_HEAD) as u8 as f32 + 0.5 * in_sight(&spatial, at, p) as u8 as f32;
                let scale = (1.0 - ((dist - PLAYER_RADIUS) / spec.radius).max(0.0)) * exposure;
                if scale > 0.0 {
                    dealt = (scale * spec.damage).floor();
                    if dealt > 0.0 {
                        damage.write(crate::combat::PlayerDamaged {
                            amount: dealt,
                            armor_stops: true,
                            zed_id: f.zed_id,
                            kind: spec.hurt,
                        });
                    }
                    push.write(crate::walk::PlayerPush {
                        momentum: (p - at) / dist * (scale * spec.momentum),
                    });
                }
            }
        }
        runlog::kv(
            "fireball_exploded",
            &format!(
                "fireball={} kind={:?} hit={what} at_unreal=({:.0}, {:.0}, {:.0}) age={:.2} player_damage={dealt}",
                f.id, f.kind, at.x, at.y, at.z, f.age
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
