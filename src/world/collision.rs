//! Collision world for walking: static colliders built from the map, plus a
//! one-off check that they sit where the ground actually is.
//!
//! The map loader fills `CollisionGeometry` (Bevy space, metres); this plugin
//! turns it into avian3d static colliders, used by spatial queries (ray and
//! shape casts) for movement and shots, and as the ground for ragdolls.

use avian3d::prelude::*;
use bevy::diagnostic::FrameCount;
use bevy::prelude::*;

use crate::engine::coords::SCALE;
use crate::engine::runlog;

/// Triangles collected for one collider.
#[derive(Default)]
pub struct TriSoup {
    pub vertices: Vec<Vec3>,
    pub triangles: Vec<[u32; 3]>,
    /// Per triangle: (the material's SurfaceType, the actor's SurfaceType;
    /// 0 for the level). See DESIGN.md, "Surface types".
    pub surfaces: Vec<[u8; 2]>,
    /// What the next pushed triangles get.
    pub surface: [u8; 2],
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
            self.surfaces.push(self.surface);
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
    /// The BSP walls that block light for actor lighting (render/actor_light.rs):
    /// as `bsp`, without fake-backdrop (sky) walls.
    pub light_bsp: TriSoup,
    /// Static mesh collision triangles, which also block light (actor
    /// lighting).
    pub light_meshes: TriSoup,
    /// For logs: (first triangle in `light_meshes`, the actor's name).
    pub light_mesh_owners: Vec<(u32, String)>,
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
    /// Doors (door.rs): block players, zeds and bodies.
    Door,
    /// Doors that also stop bullets (bBlockZeroExtentTraces).
    DoorTraces,
}

/// Zero-extent traces (bullets, blood traces, particle collision, floor
/// probes): the level and volumes that block traces.
pub fn world_filter() -> SpatialQueryFilter {
    SpatialQueryFilter::from_mask([GameLayer::World, GameLayer::TraceBlocking, GameLayer::DoorTraces])
}

/// The player's movement.
pub fn player_filter() -> SpatialQueryFilter {
    SpatialQueryFilter::from_mask([GameLayer::World, GameLayer::Blocking, GameLayer::PlayerBlocking, GameLayer::Door])
}

/// Zed movement and the zeds' walk tests.
pub fn zed_filter() -> SpatialQueryFilter {
    SpatialQueryFilter::from_mask([GameLayer::World, GameLayer::Blocking, GameLayer::ZedBlocking, GameLayer::Door])
}

/// Path checks (nav.rs): as zed_filter, but doors do not count, since
/// they open (KF builds paths through doorways with the doors ignored).
pub fn zed_path_filter() -> SpatialQueryFilter {
    SpatialQueryFilter::from_mask([GameLayer::World, GameLayer::Blocking, GameLayer::ZedBlocking])
}

/// Flying gore and other bodies (BlockingVolume bBlockKarma).
pub fn body_filter() -> SpatialQueryFilter {
    SpatialQueryFilter::from_mask([GameLayer::World, GameLayer::Blocking, GameLayer::Door])
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
    commands.insert_resource(crate::render::decals::DecalSurfaces::new(&[&geo.bsp, &geo.meshes, &geo.terrain]));
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
        let mut counts = [0usize; 32];
        for s in &soup.surfaces {
            counts[(s[0] as usize).min(31)] += 1;
        }
        let by_surface: Vec<String> = counts.iter().enumerate().filter(|(_, n)| **n > 0).map(|(k, n)| format!("{}:{n}", surface_name(k as u8))).collect();
        runlog::kv("collision_surfaces", &format!("collider={name} triangles_by_material_surface=[{}]", by_surface.join(" ")));
        commands.spawn((
            RigidBody::Static,
            SurfaceMap(std::sync::Arc::new(soup.surfaces)),
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

/// Actor.ESurfaceTypes names (Material.uc), for logs.
pub fn surface_name(k: u8) -> &'static str {
    const NAMES: [&str; 20] = [
        "Default", "Rock", "Dirt", "Metal", "Wood", "Plant", "Flesh", "Ice", "Snow", "Water", "Glass", "Gravel", "Concrete", "HollowWood", "Mud",
        "MetalArmor", "Paper", "Cloth", "Rubber", "Poop",
    ];
    NAMES.get(k as usize).copied().unwrap_or("Custom")
}

/// Per triangle of a level collider: (material SurfaceType, actor
/// SurfaceType), in the collider's triangle order.
#[derive(Component, Clone)]
pub struct SurfaceMap(pub std::sync::Arc<Vec<[u8; 2]>>);

/// The (material, actor) SurfaceType where a ray hits the level collider
/// `entity` (from a spatial query): the ray is cast again against that
/// collider's triangle mesh to learn which triangle it hit (avian's hit
/// does not say). None for colliders without a SurfaceMap (doors, glass,
/// volumes).
pub fn surface_of_hit(colliders: &Query<(&Collider, &GlobalTransform, &SurfaceMap)>, entity: Entity, origin: Vec3, dir: Vec3, max: f32) -> Option<[u8; 2]> {
    use avian3d::parry::query::{Ray, RayCast};
    let (collider, transform, map) = colliders.get(entity).ok()?;
    let mesh = collider.shape().as_trimesh()?;
    // Static level colliders sit at the origin (Transform::IDENTITY).
    let inv = transform.affine().inverse();
    let o = inv.transform_point3(origin);
    let d = inv.transform_vector3(dir);
    let ray = Ray::new(o.to_array().into(), d.to_array().into());
    let hit = mesh.cast_local_ray_and_get_normal(&ray, max + 0.01, true)?;
    let n = map.0.len().max(1);
    let tri = match hit.feature {
        avian3d::parry::shape::FeatureId::Face(i) => i as usize % n,
        _ => return None,
    };
    map.0.get(tri).copied()
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
