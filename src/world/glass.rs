//! Breakable windows: KFMod.KFGlassMover. A pane blocks pawns and bullets
//! until its Health runs out; then it is gone (BreakWindowGlassEmitter) and
//! the panes sharing its Tag crack. Damage comes from shots, melee, blasts,
//! and bumps (a zed bumping a pane swings at it). See DESIGN.md, "Map
//! fixes" M1.

use avian3d::prelude::*;
use bevy::prelude::*;
use ue_assets::level::GlassInfo;

use crate::world::collision::{GameLayer, TriSoup};
use crate::engine::coords;
use crate::engine::runlog;

/// Filled by the map loader.
#[derive(Resource, Default)]
pub struct GlassSetup {
    pub panes: Vec<GlassSpawn>,
    /// KFGlassMover.ShatteredTexture (ShaderCrackedGlass).
    pub cracked: Option<Handle<StandardMaterial>>,
}

pub struct GlassSpawn {
    pub info: GlassInfo,
    pub root: Entity,
    /// The mesh part drawn with Skins[0] (CrackWindow replaces it).
    pub skin0: Option<Entity>,
    /// Unreal units (the actor's Location).
    pub location: [f32; 3],
    pub translation: Vec3,
    pub rotation: Quat,
    /// Local-space triangles, scale applied.
    pub collision: TriSoup,
}

/// Marks a pane's collider; the index is into `Glass::panes`.
#[derive(Component)]
pub struct GlassCollider(pub usize);

pub struct Pane {
    pub info: GlassInfo,
    root: Entity,
    skin0: Option<Entity>,
    collider: Option<Entity>,
    /// Unreal units.
    pub location: Vec3,
    pub health: i32,
    pub cracked: bool,
    pub broken: bool,
}

#[derive(Resource, Default)]
pub struct Glass {
    pub panes: Vec<Pane>,
    cracked: Option<Handle<StandardMaterial>>,
}

/// Damage to a pane (KFGlassMover.TakeDamage): shots, melee.
#[derive(Message, Clone, Copy, Debug)]
pub struct GlassDamage {
    pub pane: usize,
    pub damage: f32,
    pub by: &'static str,
}

/// A pawn walking into a pane (KFGlassMover.Bump). `melee`: a zed's
/// MeleeDamage (None for the player); `speed` in Unreal units/s.
#[derive(Message, Clone, Copy, Debug)]
pub struct GlassBump {
    pub pane: usize,
    pub speed: f32,
    pub melee: Option<f32>,
}

pub struct GlassPlugin;

impl Plugin for GlassPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GlassSetup>()
            .init_resource::<Glass>()
            .add_message::<GlassDamage>()
            .add_message::<GlassBump>()
            .add_systems(PostStartup, spawn_glass)
            .add_systems(Update, damage_glass);
    }
}

fn spawn_glass(mut commands: Commands, mut setup: ResMut<GlassSetup>, mut glass: ResMut<Glass>, mut materials: Query<&mut MeshMaterial3d<StandardMaterial>>) {
    glass.cracked = setup.cracked.take();
    let mut colliders = 0;
    for (i, s) in std::mem::take(&mut setup.panes).into_iter().enumerate() {
        let collider = (!s.collision.triangles.is_empty()).then(|| {
            colliders += 1;
            commands
                .spawn((
                    RigidBody::Static,
                    Collider::trimesh(s.collision.vertices, s.collision.triangles),
                    CollisionLayers::new(LayerMask::from(GameLayer::Door) | GameLayer::DoorTraces, LayerMask::ALL),
                    Transform::from_translation(s.translation).with_rotation(s.rotation),
                    GlassCollider(i),
                    Name::new(s.info.name.clone()),
                ))
                .id()
        });
        let mut pane = Pane {
            health: s.info.health,
            info: s.info,
            root: s.root,
            skin0: s.skin0,
            collider,
            location: Vec3::from_array(s.location),
            cracked: false,
            broken: false,
        };
        // PostBeginPlay: "glass that starts out broken": Health 1 = cracked.
        if pane.health == 1 {
            crack(&mut pane, glass.cracked.as_ref(), &mut materials);
        }
        glass.panes.push(pane);
    }
    let cracked = glass.panes.iter().filter(|p| p.cracked).count();
    runlog::kv(
        "glass_loaded",
        &format!(
            "panes={} colliders={colliders} cracked={cracked} cracked_material={} health={:?}",
            glass.panes.len(),
            glass.cracked.is_some(),
            {
                let mut h: Vec<i32> = glass.panes.iter().map(|p| p.health).collect();
                h.sort();
                h.dedup();
                h
            }
        ),
    );
}

/// CrackWindow: Skins[0] becomes the cracked glass.
fn crack(p: &mut Pane, material: Option<&Handle<StandardMaterial>>, materials: &mut Query<&mut MeshMaterial3d<StandardMaterial>>) {
    p.cracked = true;
    if let (Some(e), Some(m)) = (p.skin0, material)
        && let Ok(mut mm) = materials.get_mut(e)
    {
        mm.0 = m.clone();
    }
}

#[allow(clippy::too_many_arguments)]
fn damage_glass(
    mut damage: MessageReader<GlassDamage>,
    mut bumps: MessageReader<GlassBump>,
    mut blasts: MessageReader<crate::world::door::DoorBlast>,
    spatial: SpatialQuery,
    mut glass: ResMut<Glass>,
    mut commands: Commands,
    mut visibility: Query<&mut Visibility>,
    mut materials: Query<&mut MeshMaterial3d<StandardMaterial>>,
    library: Option<Res<crate::render::particles::EffectLibrary>>,
    (mut meshes, mut sounds): (ResMut<Assets<Mesh>>, MessageWriter<crate::audio::mixer::PlaySound>),
    mut seed: Local<u32>,
) {
    // WindowGlassEmitter / BreakWindowGlassEmitter (GlassHitEmitters):
    // ImpactSounds bullethitglass / bullethitglass2 at random, KFHitEmitter
    // TransientSoundVolume 150 (capped) and TransientSoundRadius 80.
    let mut glass_sound = |seed: u32, at: Vec3| {
        let snd = if seed.is_multiple_of(2) { "KFWeaponSound.bullethitglass" } else { "KFWeaponSound.bullethitglass2" };
        sounds.write(crate::audio::mixer::PlaySound::new(snd, crate::audio::mixer::Emitter::Point(coords::pos(at.to_array()))).volume(150.0).radius(80.0));
    };
    // (pane, damage, cause) in order.
    let mut hits: Vec<(usize, f32, String)> = damage.read().map(|d| (d.pane, d.damage, d.by.to_string())).collect();
    for b in bumps.read() {
        let Some(p) = glass.panes.get(b.pane) else { continue };
        // Bump: the player does nothing to an uncracked pane.
        if b.melee.is_none() && !p.cracked {
            continue;
        }
        if b.speed >= 10.0 {
            hits.push((b.pane, b.speed, "bump_speed".into()));
        }
        if let Some(m) = b.melee {
            hits.push((b.pane, m, "zed_bump_melee".into()));
        }
    }
    // HurtRadius reaches panes too (CollidingActors, or the Siren's
    // VisibleCollidingActors, which always shatters glass: 100000).
    let level = SpatialQueryFilter::from_mask([GameLayer::World, GameLayer::TraceBlocking]);
    for b in blasts.read() {
        for (i, p) in glass.panes.iter().enumerate() {
            if p.broken {
                continue;
            }
            let dist = (p.location - b.at).length().max(1.0);
            if dist > b.radius {
                continue;
            }
            if b.line_of_sight {
                let (from, to) = (coords::pos(b.at.to_array()), coords::pos(p.location.to_array()));
                if let Ok(dir) = Dir3::new(to - from)
                    && spatial.cast_ray(from, dir, (to - from).length(), true, &level).is_some()
                {
                    continue;
                }
            }
            let amount = if b.source == "siren_scream" { 100000.0 } else { (1.0 - dist / b.radius) * b.damage };
            hits.push((i, amount, format!("{} distance={dist:.0}", b.source)));
        }
    }
    for (i, amount, cause) in hits {
        let cracked_material = glass.cracked.clone();
        let Some(p) = glass.panes.get_mut(i) else { continue };
        if p.broken {
            continue;
        }
        // TakeDamage(int Damage).
        p.health -= amount as i32;
        let lib = library.as_deref();
        if p.health > 0 {
            // ShardWindow.
            *seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            glass_sound(*seed >> 16, p.location);
            if let Some(lib) = lib {
                *seed = seed.wrapping_add(1);
                crate::render::particles::spawn_effect(&mut commands, lib, &mut meshes, "KFMod.WindowGlassEmitter", p.location, Mat3::IDENTITY, *seed);
            }
            runlog::kv("glass_hit", &format!("pane={} cause={cause} damage={} health={}", p.info.name, amount as i32, p.health));
            continue;
        }
        // BreakWindow.
        p.broken = true;
        *seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
        glass_sound(*seed >> 16, p.location);
        if let Some(c) = p.collider {
            commands.entity(c).insert(CollisionLayers::NONE);
        }
        if let Ok(mut v) = visibility.get_mut(p.root) {
            *v = Visibility::Hidden;
        }
        if let Some(lib) = lib {
            *seed = seed.wrapping_add(1);
            crate::render::particles::spawn_effect(&mut commands, lib, &mut meshes, "KFMod.BreakWindowGlassEmitter", p.location, Mat3::IDENTITY, *seed);
        }
        let tag = p.info.tag.clone();
        runlog::kv("glass_broken", &format!("pane={} cause={cause} damage={}", p.info.name, amount as i32));
        // ShatterOtherWindows: panes with the same Tag crack (Health 1),
        // unless the Tag is empty or the class name.
        if !tag.is_empty() && !tag.eq_ignore_ascii_case("KFGlassMover") {
            let mut n = 0;
            for q in glass.panes.iter_mut() {
                if !q.broken && q.info.tag.eq_ignore_ascii_case(&tag) {
                    q.health = 1;
                    crack(q, cracked_material.as_ref(), &mut materials);
                    n += 1;
                }
            }
            runlog::kv("glass_cracked", &format!("tag={tag} panes={n}"));
        }
    }
}
