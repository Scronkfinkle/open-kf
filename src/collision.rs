//! Collision world for walking: static colliders built from the map, plus a
//! one-off check that they sit where the ground actually is.
//!
//! The map loader fills `CollisionGeometry` (Bevy space, metres); this plugin
//! turns it into avian3d static colliders, used by spatial queries (ray and
//! shape casts) for movement and shots, and as the ground for ragdolls.

use avian3d::prelude::*;
use bevy::diagnostic::FrameCount;
use bevy::prelude::*;

use crate::coords::SCALE;
use crate::runlog;

/// Triangles collected for one collider.
#[derive(Default)]
pub struct TriSoup {
    pub vertices: Vec<Vec3>,
    pub triangles: Vec<[u32; 3]>,
}

impl TriSoup {
    /// Adds a convex polygon as a triangle fan.
    pub fn push_polygon(&mut self, pts: &[Vec3]) {
        if pts.len() < 3 {
            return;
        }
        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(pts);
        for k in 1..pts.len() as u32 - 1 {
            self.triangles.push([base, base + k, base + k + 1]);
        }
    }

    pub fn push_triangle(&mut self, a: Vec3, b: Vec3, c: Vec3) {
        self.push_polygon(&[a, b, c]);
    }
}

/// Everything that blocks the player, gathered while loading the map.
#[derive(Resource, Default)]
pub struct CollisionGeometry {
    pub bsp: TriSoup,
    pub meshes: TriSoup,
    pub terrain: TriSoup,
    /// Blocking brush volumes (with a label for logs and what they block),
    /// as their actual polygons. Not convex hulls: some volumes are hollow
    /// shapes, e.g. the arch around the KF-WestLondon car tunnels, which a
    /// hull would fill in.
    pub volumes: Vec<(String, VolumeBlocks, TriSoup)>,
    /// PathNode positions, used to check the colliders.
    pub nav_points: Vec<Vec3>,
}

/// What a blocking volume blocks (BlockingVolume: movement and Karma
/// bodies; bBlockZeroExtentTraces off by default, so not bullets;
/// bClassBlocker limits it to BlockedClasses).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeBlocks {
    pub players: bool,
    pub zeds: bool,
    pub traces: bool,
}

/// Collision layers. Level geometry (BSP, static meshes, terrain) is on
/// `World` (the default layer) and blocks everything. Blocking volumes go
/// on `Blocking` (players, zeds, ragdolls, flying gore), or for class
/// blockers `PlayerBlocking` / `ZedBlocking`; `TraceBlocking` marks volumes
/// that also stop bullets. Ragdoll bodies are on `Ragdoll`; movement and
/// shot queries never see corpses.
#[derive(PhysicsLayer, Clone, Copy, Debug, Default)]
pub enum GameLayer {
    #[default]
    World,
    Ragdoll,
    Blocking,
    PlayerBlocking,
    ZedBlocking,
    TraceBlocking,
}

/// Zero-extent traces (bullets, blood traces, particle collision, floor
/// probes): the level and volumes that block traces.
pub fn world_filter() -> SpatialQueryFilter {
    SpatialQueryFilter::from_mask([GameLayer::World, GameLayer::TraceBlocking])
}

/// The player's movement.
pub fn player_filter() -> SpatialQueryFilter {
    SpatialQueryFilter::from_mask([GameLayer::World, GameLayer::Blocking, GameLayer::PlayerBlocking])
}

/// Zed movement and the zeds' walk tests.
pub fn zed_filter() -> SpatialQueryFilter {
    SpatialQueryFilter::from_mask([GameLayer::World, GameLayer::Blocking, GameLayer::ZedBlocking])
}

/// Flying gore and other bodies (BlockingVolume bBlockKarma).
pub fn body_filter() -> SpatialQueryFilter {
    SpatialQueryFilter::from_mask([GameLayer::World, GameLayer::Blocking])
}

/// Which classes count as the player and as zeds when a volume lists
/// BlockedClasses (their class chains, from the class defaults).
const PLAYER_CLASSES: [&str; 6] = ["KFHumanPawn", "KFPawn", "xPawn", "UnrealPawn", "Pawn", "Actor"];
const ZED_CLASSES: [&str; 7] = ["KFMonster", "Skaarj", "Monster", "xPawn", "UnrealPawn", "Pawn", "Actor"];

/// What a volume with these BlockedClasses (None = not a class blocker)
/// blocks. A specific zed class (e.g. ZombieBloat) counts as blocking zeds.
pub fn volume_blocks(blocked: Option<&[String]>, traces: bool) -> VolumeBlocks {
    match blocked {
        None => VolumeBlocks { players: true, zeds: true, traces },
        Some(list) => VolumeBlocks {
            players: list.iter().any(|c| PLAYER_CLASSES.iter().any(|p| p.eq_ignore_ascii_case(c))),
            zeds: list
                .iter()
                .any(|c| ZED_CLASSES.iter().any(|p| p.eq_ignore_ascii_case(c)) || c.to_ascii_lowercase().starts_with("zombie")),
            traces,
        },
    }
}

/// PhysicsVolume gravity (950 Unreal units/s^2), for ragdolls. Karma's own
/// KarmaTimeScale (0.9) is not applied.
const GRAVITY: f32 = 950.0;

pub struct CollisionPlugin;

impl Plugin for CollisionPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(PhysicsPlugins::default())
            .insert_resource(Gravity(Vec3::NEG_Y * GRAVITY * SCALE))

            .init_resource::<CollisionGeometry>()
            .add_systems(PostStartup, spawn_colliders)
            .add_systems(Update, check_colliders_once);
    }
}

fn spawn_colliders(mut commands: Commands, mut geo: ResMut<CollisionGeometry>) {
    // Level surfaces for blood decals (Projectors draw on BSP, static meshes
    // and terrain; we only have the blocking ones).
    commands.insert_resource(crate::decals::DecalSurfaces::new(&[&geo.bsp, &geo.meshes, &geo.terrain]));
    let mut spawned = Vec::new();
    for (name, soup) in [
        ("bsp", std::mem::take(&mut geo.bsp)),
        ("meshes", std::mem::take(&mut geo.meshes)),
        ("terrain", std::mem::take(&mut geo.terrain)),
    ] {
        if soup.triangles.is_empty() {
            continue;
        }
        spawned.push(format!("{name}_triangles={}", soup.triangles.len()));
        commands.spawn((
            RigidBody::Static,
            Collider::trimesh(soup.vertices, soup.triangles),
            Transform::IDENTITY,
            Name::new(format!("collision_{name}")),
        ));
    }
    let (mut volumes, mut volume_triangles) = (0usize, 0usize);
    let mut kinds: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for (label, blocks, soup) in std::mem::take(&mut geo.volumes) {
        if soup.triangles.is_empty() {
            continue;
        }
        let mut layers: Vec<GameLayer> = Vec::new();
        match (blocks.players, blocks.zeds) {
            (true, true) => layers.push(GameLayer::Blocking),
            (true, false) => layers.push(GameLayer::PlayerBlocking),
            (false, true) => layers.push(GameLayer::ZedBlocking),
            (false, false) => {}
        }
        if blocks.traces {
            layers.push(GameLayer::TraceBlocking);
        }
        *kinds.entry(format!("{layers:?}")).or_default() += 1;
        if layers.is_empty() {
            continue;
        }
        volume_triangles += soup.triangles.len();
        commands.spawn((
            RigidBody::Static,
            Collider::trimesh(soup.vertices, soup.triangles),
            CollisionLayers::new(layers.iter().fold(LayerMask::NONE, |m, l| m | *l), LayerMask::ALL),
            Transform::IDENTITY,
            Name::new(label),
        ));
        volumes += 1;
    }
    runlog::kv(
        "collision_spawned",
        &format!("{} volumes={volumes} volume_triangles={volume_triangles} volume_layers={kinds:?}", spawned.join(" ")),
    );
}

/// A few frames in (once avian has registered the colliders), cast a ray
/// down from every PathNode. PathNodes sit about 44 units above the floor,
/// so most rays should hit at that distance; misses mean missing geometry.
fn check_colliders_once(
    frames: Res<FrameCount>,
    spatial: SpatialQuery,
    geo: Res<CollisionGeometry>,
    names: Query<&Name>,
    mut done: Local<bool>,
) {
    if *done || frames.0 < 5 {
        return;
    }
    *done = true;
    let max = 300.0 * SCALE;
    let mut dists: Vec<f32> = Vec::new();
    let mut misses = 0usize;
    // Rays that start inside a collider report distance 0; count them by collider.
    let mut inside: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for &p in &geo.nav_points {
        match spatial.cast_ray(p, Dir3::NEG_Y, max, true, &world_filter()) {
            Some(hit) if hit.distance == 0.0 => {
                let name = names.get(hit.entity).map_or("unnamed".to_string(), |n| n.to_string());
                *inside.entry(name).or_default() += 1;
                // Measure to the floor anyway, treating colliders as hollow.
                if let Some(h) = spatial.cast_ray(p, Dir3::NEG_Y, max, false, &world_filter()) {
                    dists.push(h.distance / SCALE);
                }
            }
            Some(hit) => dists.push(hit.distance / SCALE),
            None => misses += 1,
        }
    }
    dists.sort_by(|a, b| a.total_cmp(b));
    let pct = |q: f32| {
        if dists.is_empty() {
            f32::NAN
        } else {
            dists[((dists.len() - 1) as f32 * q) as usize]
        }
    };
    let near = dists.iter().filter(|d| (**d - 44.0).abs() < 8.0).count();
    runlog::kv(
        "collision_check",
        &format!(
            "pathnodes={} hits={} misses={misses} distance_unreal p10={:.1} median={:.1} p90={:.1} within_8_of_44={near} started_inside={inside:?}",
            geo.nav_points.len(),
            dists.len(),
            pct(0.1),
            pct(0.5),
            pct(0.9)
        ),
    );
}
