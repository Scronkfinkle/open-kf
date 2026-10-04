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

const CLASS: &str = "KFChar.HuskFireProjectile";
pub const SPEED: f32 = 1800.0;
const DAMAGE: f32 = 25.0;
const DAMAGE_RADIUS: f32 = 150.0;
const MOMENTUM: f32 = 125000.0;
const LIFE_SPAN: f32 = 10.0;
const RADIUS: f32 = 2.0;
/// Player cylinder (KFPawn) and its head and root bones above the centre
/// (approximate: no player skeleton yet), Unreal units.
const PLAYER_RADIUS: f32 = 20.0;
const PLAYER_HALF_HEIGHT: f32 = 50.0;
const PLAYER_EYE: f32 = 44.0;
const PLAYER_HEAD: f32 = 40.0;

/// A fireball to spawn: start and direction, Unreal units.
#[derive(Message, Clone, Copy, Debug)]
pub struct SpawnFireball {
    pub at: Vec3,
    pub dir: Vec3,
    pub zed_id: usize,
}

#[derive(Resource, Default)]
struct FireballModel(Option<PieceModel>);

#[derive(Component)]
struct Fireball {
    id: u32,
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
    match gore::find_class(&set, CLASS)
        .ok_or_else(|| "class not found".to_string())
        .and_then(|class| gore::load_piece(&set, &defaults, &class, CLASS, &mut meshes, &mut images, &mut materials))
    {
        Ok(m) => {
            runlog::kv("fireball_loaded", &format!("class={CLASS} draw_scale={}", m.draw_scale()));
            model.0 = Some(m);
        }
        Err(e) => runlog::kv("fireball_load_error", &format!("class={CLASS} error=\"{e}\"")),
    }
}

fn axes_along(d: Vec3) -> Mat3 {
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
        let trail = library
            .as_deref()
            .and_then(|lib| particles::spawn_effect(&mut commands, lib, &mut meshes, "KFMod.FlameThrowerFlameB", r.at, axes, *next_id));
        let scale = model.0.as_ref().map_or(1.0, |m| m.draw_scale());
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
                    zed_id: r.zed_id,
                    at: r.at,
                    velocity: d * SPEED,
                    age: 0.0,
                    trail,
                },
            ))
            .id();
        if let Some(m) = &model.0 {
            m.spawn_parts(&mut commands, e);
        }
        runlog::kv(
            "fireball_spawned",
            &format!(
                "fireball={} zed={} at_unreal=({:.0}, {:.0}, {:.0}) dir=({:.2}, {:.2}, {:.2})",
                *next_id, r.zed_id, r.at.x, r.at.y, r.at.z, d.x, d.y, d.z
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
        if let Ok(d) = Dir3::new(coords::dir(step.to_array()))
            && let Some(h) = spatial.cast_ray(coords::pos(a.to_array()), d, (step.length() + RADIUS) * SCALE, true, &crate::collision::world_filter())
        {
            let n = h.normal;
            let n = to_ue(if n.dot(*d) > 0.0 { -n } else { n }) * SCALE;
            hit = Some(((h.distance / SCALE / step.length()).min(1.0), n, "level"));
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
        let at = a + step * frac;
        if let Some(lib) = library.as_deref() {
            particles::spawn_effect(&mut commands, lib, &mut meshes, "KFMod.FlameImpact", at + normal * 20.0, axes_along(normal), f.id);
        }
        decals.write(SpawnDecal {
            kind: DecalKind::Scorch,
            at,
            dir: -normal,
            trace: false,
        });
        let mut dealt = 0.0;
        if let Some(p) = player {
            let dist = (p - at).length().max(1.0);
            if dist - PLAYER_RADIUS <= DAMAGE_RADIUS {
                let exposure = 0.5 * in_sight(&spatial, at, p + Vec3::Z * PLAYER_HEAD) as u8 as f32 + 0.5 * in_sight(&spatial, at, p) as u8 as f32;
                let scale = (1.0 - ((dist - PLAYER_RADIUS) / DAMAGE_RADIUS).max(0.0)) * exposure;
                if scale > 0.0 {
                    dealt = (scale * DAMAGE).floor();
                    if dealt > 0.0 {
                        damage.write(crate::combat::PlayerDamaged {
                            amount: dealt,
                            zed_id: f.zed_id,
                            kind: crate::combat::HurtKind::Fire,
                        });
                    }
                    push.write(crate::walk::PlayerPush {
                        momentum: (p - at) / dist * (scale * MOMENTUM),
                    });
                }
            }
        }
        runlog::kv(
            "fireball_exploded",
            &format!(
                "fireball={} hit={what} at_unreal=({:.0}, {:.0}, {:.0}) age={:.2} player_damage={dealt}",
                f.id, at.x, at.y, at.z, f.age
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
