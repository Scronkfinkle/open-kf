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
    /// Blocking brush volumes (with a label for logs), as their actual
    /// polygons. Not convex hulls: some volumes are hollow shapes, e.g. the
    /// arch around the KF-WestLondon car tunnels, which a hull would fill in.
    pub volumes: Vec<(String, TriSoup)>,
    /// PathNode positions, used to check the colliders.
    pub nav_points: Vec<Vec3>,
}

/// Collision layers. Level geometry is on `World` (the default layer).
/// Ragdoll bodies collide with the world only; movement and shot queries use
/// `world_filter` so corpses never block them.
#[derive(PhysicsLayer, Clone, Copy, Debug, Default)]
pub enum GameLayer {
    #[default]
    World,
    Ragdoll,
}

/// Spatial query filter for the level only (ignores ragdolls).
pub fn world_filter() -> SpatialQueryFilter {
    SpatialQueryFilter::from_mask(GameLayer::World)
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
    for (label, soup) in std::mem::take(&mut geo.volumes) {
        if soup.triangles.is_empty() {
            continue;
        }
        volume_triangles += soup.triangles.len();
        commands.spawn((
            RigidBody::Static,
            Collider::trimesh(soup.vertices, soup.triangles),
            Transform::IDENTITY,
            Name::new(label),
        ));
        volumes += 1;
    }
    runlog::kv(
        "collision_spawned",
        &format!("{} volumes={volumes} volume_triangles={volume_triangles}", spawned.join(" ")),
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
