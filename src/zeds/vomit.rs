//! The Bloat's vomit: KFMod.KFBloatVomit globs (Old2k4.BioGlob). Each glob
//! flies with gravity (PHYS_Falling) at its spawn velocity. Touching the
//! player in flight it does HurtRadius(Damage 4, DamageRadius 120) and flies
//! on; reaching the level it lands (Flying.Landed / HitWall), leaves a
//! VomitDecal and, in OnGround.BeginState, blows up at once: HurtRadius of
//! BaseDamage 3 + Damage 4 x GoopLevel 1 = 7 within 120. All vomit damage is
//! DamTypeVomit, which starts the player's bile burn (combat.rs). Damage
//! and BaseDamage x the difficulty's scale (`vomit_damage`).

use avian3d::prelude::*;
use bevy::prelude::*;
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package_set::PackageSet;

use crate::engine::camera::FlyCamera;
use crate::engine::coords::{self, SCALE};
use crate::render::decals::{DecalKind, SpawnDecal};
use crate::zeds::gore::{self, PieceModel};
use crate::world::map::MapRequest;
use crate::engine::runlog;

const CLASS: &str = "KFMod.KFBloatVomit";
/// KFBloatVomit / BioGlob defaults.
pub const SPEED: f32 = 400.0;
const DAMAGE: f32 = 4.0;
const BASE_DAMAGE: f32 = 3.0;

/// KFBloatVomit.PostBeginPlay: BaseDamage and Damage = Max(int(x
/// DifficultyDamageModifer), 1) (Beginner 0.3, Normal 1, Hard 1.5,
/// Suicidal 2, Hell on Earth 2.5).
fn vomit_damage(d: f32) -> f32 {
    (d * crate::game::difficulty::current().vomit_damage_scale()).trunc().max(1.0)
}
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
    /// The players already touched in flight (None: this game's own).
    touched: Vec<Option<u64>>,
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
                    touched: Vec::new(),
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
            .cast_ray(from, d, (to - from).length(), true, &crate::world::collision::world_filter())
            .is_some()
    {
        return None;
    }
    let scale = 1.0 - ((dist - PLAYER_RADIUS) / DAMAGE_RADIUS).max(0.0);
    let amount = (scale * damage).floor();
    (amount > 0.0).then_some(amount)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn move_globs(
    mut commands: Commands,
    time: Res<Time>,
    spatial: SpatialQuery,
    player: Query<(&Transform, Option<&crate::player::walk::Walker>), (With<FlyCamera>, Without<Glob>)>,
    mut globs: Query<(Entity, &mut Glob, &mut Transform)>,
    mut damage: MessageWriter<crate::game::combat::PlayerDamaged>,
    (mut decals, mut sounds): (MessageWriter<SpawnDecal>, MessageWriter<crate::audio::mixer::PlaySound>),
    (remote, net): (Res<crate::game::combat::RemotePlayers>, Option<Res<crate::net::NetMode>>),
) {
    let dt = time.delta_secs().min(0.1);
    // A network client's globs only show (the host's hit the players and
    // send the hits to their games).
    let harmless = net.is_some_and(|n| matches!(*n, crate::net::NetMode::Client { .. }));
    // The players' centres, Unreal units (on a host also the others').
    let local = player.single().ok().map(|(t, w)| w.map_or(t.translation - Vec3::Y * PLAYER_EYE * SCALE, |w| w.center));
    let targets = if harmless { Vec::new() } else { remote.targets(local) };
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
                .cast_ray(from, d, step.length() * SCALE + GLOB_RADIUS * SCALE, true, &crate::world::collision::world_filter())
                .map(|h| (h, d))
        });
        // Flying.ProcessTouch with a player: HurtRadius(Damage, ...), and
        // the glob flies on.
        for &(who, p) in &targets {
            if g.touched.contains(&who) {
                continue;
            }
            let end = g.at + step;
            let flat = (end - p).truncate().length();
            if flat <= PLAYER_RADIUS + GLOB_RADIUS && (end.z - p.z).abs() <= PLAYER_HALF_HEIGHT + GLOB_RADIUS {
                g.touched.push(who);
                let hurt = hurt_radius(&spatial, p, end, vomit_damage(DAMAGE));
                if let Some(amount) = hurt {
                    damage.write(crate::game::combat::PlayerDamaged {
                        amount,
                        armor_stops: true,
                        zed_id: g.zed_id,
                        kind: crate::game::combat::HurtKind::Vomit,
                        dam_type: crate::game::combat::DamType::Vomit,
                        source: Some(coords::pos(end.to_array())),
                        dam: Some(crate::game::perks::known_dam_type("DamTypeVomit")),
                        to_peer: who,
                    });
                }
                runlog::kv("vomit_touch", &format!("glob={} player=true target={} damage={}", g.id, who.map_or("local".to_string(), |p| p.to_string()), hurt.unwrap_or(0.0)));
            }
        }
        if let Some((h, d)) = hit {
            // Landed: VomitDecal facing into the surface, then BlowUp.
            let n_bevy = if h.normal.dot(*d) > 0.0 { -h.normal } else { h.normal };
            let n = Vec3::new(-n_bevy.z, n_bevy.x, n_bevy.y);
            let at_bevy = from + *d * (h.distance - GLOB_RADIUS * SCALE).max(0.0);
            let at = Vec3::new(-at_bevy.z, at_bevy.x, at_bevy.y) / SCALE;
            // KFBloatVomit.Landed: PlaySound(ImpactSound, SLOT_Misc) with the
            // Actor defaults (volume 0.3, radius 300).
            sounds.write(crate::audio::mixer::PlaySound::new("KF_EnemiesFinalSnd.Bloat.Bloat_AcidSplash", crate::audio::mixer::Emitter::Point(at_bevy)));
            decals.write(SpawnDecal {
                kind: DecalKind::Vomit,
                at,
                dir: -n,
                trace: false,
            });
            let mut hurt = None;
            let mut hit_others = Vec::new();
            for &(who, p) in &targets {
                let Some(amount) = hurt_radius(&spatial, p, at, vomit_damage(BASE_DAMAGE) + vomit_damage(DAMAGE) * GOOP_LEVEL) else { continue };
                damage.write(crate::game::combat::PlayerDamaged {
                    amount,
                    armor_stops: true,
                    zed_id: g.zed_id,
                    kind: crate::game::combat::HurtKind::Vomit,
                    dam_type: crate::game::combat::DamType::Vomit,
                    source: Some(at_bevy),
                    dam: Some(crate::game::perks::known_dam_type("DamTypeVomit")),
                    to_peer: who,
                });
                match who {
                    None => hurt = Some(amount),
                    Some(peer) => hit_others.push(format!("{peer}:{amount}")),
                }
            }
            runlog::kv(
                "vomit_landed",
                &format!(
                    "glob={} at_unreal=({:.0}, {:.0}, {:.0}) age={:.2} player_damage={} other_players=[{}] harmless={harmless}",
                    g.id,
                    at.x,
                    at.y,
                    at.z,
                    g.age,
                    hurt.unwrap_or(0.0),
                    hit_others.join(" ")
                ),
            );
            commands.entity(e).despawn();
            continue;
        }
        g.at += step;
        t.translation = coords::pos(g.at.to_array());
    }
}
