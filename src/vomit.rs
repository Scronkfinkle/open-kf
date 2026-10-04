//! The Bloat's vomit: KFMod.KFBloatVomit globs (Old2k4.BioGlob). Each glob
//! flies with gravity (PHYS_Falling) at its spawn velocity. Touching the
//! player in flight it does HurtRadius(Damage 4, DamageRadius 120) and flies
//! on; reaching the level it lands (Flying.Landed / HitWall), leaves a
//! VomitDecal and, in OnGround.BeginState, blows up at once: HurtRadius of
//! BaseDamage 3 + Damage 4 x GoopLevel 1 = 7 within 120. All vomit damage is
//! DamTypeVomit, which starts the player's bile burn (combat.rs). Normal
//! difficulty (damage modifier 1).

use avian3d::prelude::*;
use bevy::prelude::*;
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package_set::PackageSet;

use crate::camera::FlyCamera;
use crate::coords::{self, SCALE};
use crate::decals::{DecalKind, SpawnDecal};
use crate::gore::{self, PieceModel};
use crate::map::MapRequest;
use crate::runlog;

const CLASS: &str = "KFMod.KFBloatVomit";
/// KFBloatVomit / BioGlob defaults.
pub const SPEED: f32 = 400.0;
const DAMAGE: f32 = 4.0;
const BASE_DAMAGE: f32 = 3.0;
const GOOP_LEVEL: f32 = 1.0;
const DAMAGE_RADIUS: f32 = 120.0;
const LIFE_SPAN: f32 = 8.0;
const GLOB_RADIUS: f32 = 2.0;
const GRAVITY: f32 = 950.0;
/// Player cylinder (KFPawn), Unreal units.
const PLAYER_RADIUS: f32 = 20.0;
const PLAYER_HALF_HEIGHT: f32 = 50.0;
const PLAYER_EYE: f32 = 44.0;

/// A glob to spawn: start and velocity in Unreal units (and per second).
#[derive(Message, Clone, Copy, Debug)]
pub struct SpawnVomit {
    pub at: Vec3,
    pub velocity: Vec3,
    pub zed_id: usize,
}

#[derive(Resource, Default)]
struct GlobModel(Option<PieceModel>);

#[derive(Component)]
struct Glob {
    id: u32,
    zed_id: usize,
    /// Unreal units.
    at: Vec3,
    velocity: Vec3,
    age: f32,
    touched_player: bool,
}

pub struct VomitPlugin;

impl Plugin for VomitPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SpawnVomit>()
            .init_resource::<GlobModel>()
            .add_systems(PostStartup, load_glob)
            .add_systems(Update, (spawn_globs, move_globs).chain());
    }
}

fn load_glob(
    request: Res<MapRequest>,
    mut model: ResMut<GlobModel>,
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
            runlog::kv("vomit_loaded", &format!("class={CLASS} draw_scale={}", m.draw_scale()));
            model.0 = Some(m);
        }
        Err(e) => runlog::kv("vomit_load_error", &format!("class={CLASS} error=\"{e}\"")),
    }
}

fn spawn_globs(mut commands: Commands, mut requests: MessageReader<SpawnVomit>, model: Res<GlobModel>, mut next_id: Local<u32>) {
    for r in requests.read() {
        *next_id += 1;
        let k = 65536.0 / std::f32::consts::TAU;
        let d = r.velocity.normalize_or_zero();
        let rot = ue_assets::properties::Rotator {
            pitch: (d.z.clamp(-1.0, 1.0).asin() * k) as i32,
            yaw: (d.y.atan2(d.x) * k) as i32,
            roll: 0,
        };
        let scale = model.0.as_ref().map_or(1.0, |m| m.draw_scale());
        let e = commands
            .spawn((
                Transform {
                    translation: coords::pos(r.at.to_array()),
                    rotation: coords::rotation(rot),
                    scale: Vec3::splat(scale),
                },
                Visibility::Visible,
                Glob {
                    id: *next_id,
                    zed_id: r.zed_id,
                    at: r.at,
                    velocity: r.velocity,
                    age: 0.0,
                    touched_player: false,
                },
            ))
            .id();
        if let Some(m) = &model.0 {
            m.spawn_parts(&mut commands, e);
        }
        runlog::kv(
            "vomit_spawned",
            &format!(
                "glob={} zed={} at_unreal=({:.0}, {:.0}, {:.0}) velocity_unreal=({:.0}, {:.0}, {:.0})",
                *next_id, r.zed_id, r.at.x, r.at.y, r.at.z, r.velocity.x, r.velocity.y, r.velocity.z
            ),
        );
    }
}

/// HurtRadius on the player: damage x (1 - (distance - radius) / DamageRadius),
/// whole points (TakeDamage takes an int), if the player is in range and in
/// sight of the point.
fn hurt_radius(spatial: &SpatialQuery, player: Vec3, at: Vec3, damage: f32) -> Option<f32> {
    let dist = (player - at).length();
    if dist - PLAYER_RADIUS > DAMAGE_RADIUS {
        return None;
    }
    let (from, to) = (coords::pos(at.to_array()), coords::pos(player.to_array()));
    if let Ok(d) = Dir3::new(to - from)
        && spatial
            .cast_ray(from, d, (to - from).length(), true, &crate::collision::world_filter())
            .is_some()
    {
        return None;
    }
    let scale = 1.0 - ((dist - PLAYER_RADIUS) / DAMAGE_RADIUS).max(0.0);
    let amount = (scale * damage).floor();
    (amount > 0.0).then_some(amount)
}

#[allow(clippy::type_complexity)]
fn move_globs(
    mut commands: Commands,
    time: Res<Time>,
    spatial: SpatialQuery,
    player: Query<(&Transform, Option<&crate::walk::Walker>), (With<FlyCamera>, Without<Glob>)>,
    mut globs: Query<(Entity, &mut Glob, &mut Transform)>,
    mut damage: MessageWriter<crate::combat::PlayerDamaged>,
    mut decals: MessageWriter<SpawnDecal>,
) {
    let dt = time.delta_secs().min(0.1);
    // The player's centre, Unreal units.
    let player = player.single().ok().map(|(t, w)| {
        let c = w.map_or(t.translation - Vec3::Y * PLAYER_EYE * SCALE, |w| w.center);
        Vec3::new(-c.z, c.x, c.y) / SCALE
    });
    for (e, mut g, mut t) in &mut globs {
        g.age += dt;
        if g.age > LIFE_SPAN {
            commands.entity(e).despawn();
            runlog::kv("vomit_expired", &format!("glob={}", g.id));
            continue;
        }
        g.velocity.z -= GRAVITY * dt;
        let step = g.velocity * dt;
        let from = coords::pos(g.at.to_array());
        let hit = Dir3::new(coords::dir(step.to_array())).ok().and_then(|d| {
            spatial
                .cast_ray(from, d, step.length() * SCALE + GLOB_RADIUS * SCALE, true, &crate::collision::world_filter())
                .map(|h| (h, d))
        });
        // Flying.ProcessTouch with the player: HurtRadius(Damage, ...), and
        // the glob flies on.
        if let Some(p) = player
            && !g.touched_player
        {
            let end = g.at + step;
            let flat = (end - p).truncate().length();
            if flat <= PLAYER_RADIUS + GLOB_RADIUS && (end.z - p.z).abs() <= PLAYER_HALF_HEIGHT + GLOB_RADIUS {
                g.touched_player = true;
                if let Some(amount) = hurt_radius(&spatial, p, end, DAMAGE) {
                    damage.write(crate::combat::PlayerDamaged {
                        amount,
                        zed_id: g.zed_id,
                        kind: crate::combat::HurtKind::Vomit,
                    });
                }
                runlog::kv("vomit_touch", &format!("glob={} player=true", g.id));
            }
        }
        if let Some((h, d)) = hit {
            // Landed: VomitDecal facing into the surface, then BlowUp.
            let n_bevy = if h.normal.dot(*d) > 0.0 { -h.normal } else { h.normal };
            let n = Vec3::new(-n_bevy.z, n_bevy.x, n_bevy.y);
            let at_bevy = from + *d * (h.distance - GLOB_RADIUS * SCALE).max(0.0);
            let at = Vec3::new(-at_bevy.z, at_bevy.x, at_bevy.y) / SCALE;
            decals.write(SpawnDecal {
                kind: DecalKind::Vomit,
                at,
                dir: -n,
                trace: false,
            });
            let hurt = player.and_then(|p| hurt_radius(&spatial, p, at, BASE_DAMAGE + DAMAGE * GOOP_LEVEL));
            if let Some(amount) = hurt {
                damage.write(crate::combat::PlayerDamaged {
                    amount,
                    zed_id: g.zed_id,
                    kind: crate::combat::HurtKind::Vomit,
                });
            }
            runlog::kv(
                "vomit_landed",
                &format!(
                    "glob={} at_unreal=({:.0}, {:.0}, {:.0}) age={:.2} player_damage={}",
                    g.id,
                    at.x,
                    at.y,
                    at.z,
                    g.age,
                    hurt.unwrap_or(0.0)
                ),
            );
            commands.entity(e).despawn();
            continue;
        }
        g.at += step;
        t.translation = coords::pos(g.at.to_array());
    }
}
