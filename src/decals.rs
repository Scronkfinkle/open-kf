//! Blood decals: KF's ProjectedDecal Projectors (DESIGN.md, "Gore step D"),
//! a texture projected onto level geometry inside a box. Built on the CPU:
//! the level triangles in the box are clipped to it and textured by their
//! position in the box, then drawn with the particles' 2x modulate material.
//!
//! Positions and directions in `SpawnDecal` are Unreal world space.

use std::collections::HashMap;

use avian3d::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package::ObjectRef;
use ue_assets::package_set::{ObjectHandle, PackageSet};
use ue_assets::properties::{Rotator, Value};
use ue_assets::texture::read_texture;

use crate::collision::TriSoup;
use crate::coords::{self, SCALE};
use crate::map::MapRequest;
use crate::particles::{self, ModulateMaterial};
use crate::runlog;

/// Grid cell size for looking up level triangles, Bevy metres.
const CELL: f32 = 4.0;
/// Decals end with a fade over this many seconds (assumed; the engine's
/// AbandonProjector is native).
const FADE_OUT: f32 = 1.0;
/// Distance the decal is lifted toward the projector, Unreal units, so it
/// draws over the surface it lies on.
const LIFT: f32 = 0.4;

/// The level surfaces decals can land on, with a grid for lookups.
#[derive(Resource, Default)]
pub struct DecalSurfaces {
    /// Triangles, Bevy space.
    triangles: Vec<[Vec3; 3]>,
    grid: HashMap<IVec3, Vec<u32>>,
}

fn cell(p: Vec3) -> IVec3 {
    (p / CELL).floor().as_ivec3()
}

impl DecalSurfaces {
    pub fn new(soups: &[&TriSoup]) -> Self {
        let mut s = DecalSurfaces::default();
        for soup in soups {
            for t in &soup.triangles {
                let tri = t.map(|i| soup.vertices[i as usize]);
                let id = s.triangles.len() as u32;
                let (lo, hi) = (tri[0].min(tri[1]).min(tri[2]), tri[0].max(tri[1]).max(tri[2]));
                let (a, b) = (cell(lo), cell(hi));
                for x in a.x..=b.x {
                    for y in a.y..=b.y {
                        for z in a.z..=b.z {
                            s.grid.entry(IVec3::new(x, y, z)).or_default().push(id);
                        }
                    }
                }
                s.triangles.push(tri);
            }
        }
        runlog::kv("decal_surfaces", &format!("triangles={} cells={}", s.triangles.len(), s.grid.len()));
        s
    }

    /// Triangles whose cells overlap the box `lo..hi` (Bevy space).
    fn near(&self, lo: Vec3, hi: Vec3) -> Vec<[Vec3; 3]> {
        let (a, b) = (cell(lo), cell(hi));
        let mut ids: Vec<u32> = Vec::new();
        for x in a.x..=b.x {
            for y in a.y..=b.y {
                for z in a.z..=b.z {
                    if let Some(list) = self.grid.get(&IVec3::new(x, y, z)) {
                        ids.extend_from_slice(list);
                    }
                }
            }
        }
        ids.sort_unstable();
        ids.dedup();
        ids.into_iter().map(|i| self.triangles[i as usize]).collect()
    }
}

/// Which KF decal class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DecalKind {
    /// ROBloodSplatter (ProjectileBloodSplat's wall splat after a hit).
    WallSplat,
    /// ROSmallBloodDrops (SeveredAppendage drips).
    Drip,
    /// KFBloodSplatterDecal (KFBloodPuff, under a stopped brain chunk).
    FloorSplat,
    /// KFBloodStreakDecal (ragdoll impacts).
    Streak,
    /// VomitDecal (where Bloat vomit lands).
    Vomit,
    /// FlameThrowerBurnMark (where a Husk fireball explodes).
    Scorch,
    /// RocketMarkDirt (where the Patriarch's rocket explodes).
    RocketMark,
    /// BulletHoleDirt (ROBulletHitEffect, default surface).
    BulletHole,
}

const DECAL_CLASSES: [(DecalKind, &str); 8] = [
    (DecalKind::BulletHole, "ROEffects.BulletHoleDirt"),
    (DecalKind::Scorch, "KFMod.FlameThrowerBurnMark"),
    (DecalKind::RocketMark, "ROEffects.RocketMarkDirt"),
    (DecalKind::Vomit, "KFMod.VomitDecal"),
    (DecalKind::WallSplat, "ROEffects.ROBloodSplatter"),
    (DecalKind::Drip, "ROEffects.ROSmallBloodDrops"),
    (DecalKind::FloorSplat, "KFMod.KFBloodSplatterDecal"),
    (DecalKind::Streak, "KFMod.KFBloodStreakDecal"),
];

/// A request for a decal. With `trace`, a 350-unit trace is made from `at`
/// along `dir` first and the decal goes where it hits, at WallHit + 20 x
/// (WallNormal + VRand()), facing into the surface (ProjectileBloodSplat /
/// BloodSpurt.WallSplat); without, the decal is at `at` facing along `dir`.
#[derive(Message, Clone, Copy, Debug)]
pub struct SpawnDecal {
    pub kind: DecalKind,
    pub at: Vec3,
    pub dir: Vec3,
    pub trace: bool,
}

struct DecalClass {
    name: String,
    /// One material and pixel size per Splats texture.
    textures: Vec<(Handle<ModulateMaterial>, Vec2)>,
    draw_scale: f32,
    push_back: f32,
    depth: f32,
    random_orient: bool,
    fade_in: f32,
    life: f32,
}

#[derive(Resource, Default)]
struct DecalLibrary(HashMap<DecalKind, DecalClass>);

#[derive(Component)]
struct Decal {
    id: u32,
    age: f32,
    life: f32,
    fade_in: f32,
    mesh: Handle<Mesh>,
    vertices: usize,
}

pub struct DecalPlugin;

impl Plugin for DecalPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SpawnDecal>()
            .init_resource::<DecalLibrary>()
            .add_systems(PostStartup, load_decals)
            .add_systems(Update, ragdoll_streaks)
            .add_systems(PostUpdate, (spawn_decals, fade_decals));
    }
}

fn load_decals(
    request: Res<MapRequest>,
    mut library: ResMut<DecalLibrary>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<ModulateMaterial>>,
) {
    let set = PackageSet::new(&request.install_root);
    let defaults = ClassDefaults::new(&set);
    for (kind, path) in DECAL_CLASSES {
        let Some(class) = find_class(&set, path) else {
            runlog::kv("decal_class_error", &format!("class={path} error=\"not found\""));
            continue;
        };
        let float = |p: &str, d: f32| match defaults.get(&class, p) {
            Some((Value::Float(f), _)) => f,
            Some((Value::Int(i), _)) => i as f32,
            _ => d,
        };
        let mut textures = Vec::new();
        // Splats (KF's blood decals), else the one ProjTexture (VomitDecal).
        let mut sources = defaults.get_array_objects(&class, "Splats");
        if sources.is_empty()
            && let Some((Value::Object(rf), pkg)) = defaults.get(&class, "ProjTexture")
            && let Some(h) = set.resolve(&pkg, rf)
        {
            sources.push(h);
        }
        for h in sources {
            let size = read_texture(&h.package.pkg, h.export)
                .ok()
                .and_then(|t| t.mips.first().map(|m| Vec2::new(m.width as f32, m.height as f32)));
            if let (Some(image), Some(size)) = (particles::decode(&h, true, true, &mut images), size) {
                textures.push((materials.add(ModulateMaterial { texture: image }), size));
            }
        }
        // Class LifeSpan; ProjectedDecal.PostBeginPlay makes it
        // FMax(0.5, LifeSpan + (Rand(1) - 1)), and Rand(1) is always 0.
        let life = (float("LifeSpan", 3.0) - 1.0).max(0.5);
        let c = DecalClass {
            name: path.to_string(),
            draw_scale: float("DrawScale", 1.0),
            push_back: float("PushBack", 0.0),
            depth: float("MaxTraceDistance", 1000.0),
            random_orient: matches!(defaults.get(&class, "RandomOrient"), Some((Value::Bool(true), _))),
            fade_in: float("FadeInTime", 0.0),
            life,
            textures,
        };
        runlog::kv(
            "decal_class_loaded",
            &format!(
                "class={path} textures={} draw_scale={} push_back={} depth={} random_orient={} fade_in={} life={}",
                c.textures.len(),
                c.draw_scale,
                c.push_back,
                c.depth,
                c.random_orient,
                c.fade_in,
                c.life
            ),
        );
        library.0.insert(kind, c);
    }
}

fn find_class(set: &PackageSet, path: &str) -> Option<ObjectHandle> {
    let (pkg_name, class_name) = path.split_once('.')?;
    let lp = set.load(pkg_name)?;
    let export = (0..lp.pkg.exports.len()).find(|&i| {
        lp.pkg.export_class_name(i) == "Class" && lp.pkg.object_name(ObjectRef::Export(i)).eq_ignore_ascii_case(class_name)
    })?;
    Some(ObjectHandle { package: lp, export })
}

fn frand(rng: &mut u32) -> f32 {
    *rng ^= *rng << 13;
    *rng ^= *rng >> 17;
    *rng ^= *rng << 5;
    (*rng % 100_000) as f32 / 100_000.0
}

fn to_ue(v: Vec3) -> Vec3 {
    Vec3::new(-v.z, v.x, v.y) / SCALE
}

/// Clips a convex polygon to the half-space `n . p <= d`.
fn clip(poly: &[Vec3], n: Vec3, d: f32) -> Vec<Vec3> {
    let mut out = Vec::with_capacity(poly.len() + 2);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (da, db) = (n.dot(a) - d, n.dot(b) - d);
        if da <= 0.0 {
            out.push(a);
        }
        if (da < 0.0) != (db < 0.0) {
            out.push(a + (b - a) * (da / (da - db)));
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn spawn_decals(
    mut commands: Commands,
    mut requests: MessageReader<SpawnDecal>,
    library: Res<DecalLibrary>,
    surfaces: Option<Res<DecalSurfaces>>,
    spatial: SpatialQuery,
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut next_id: Local<u32>,
) {
    let Some(surfaces) = surfaces else {
        requests.clear();
        return;
    };
    for req in requests.read() {
        let Some(class) = library.0.get(&req.kind) else {
            continue;
        };
        *next_id += 1;
        let id = *next_id;
        let mut rng = (id.wrapping_mul(2_654_435_761) ^ (time.elapsed_secs_f64() * 1000.0) as u32) | 1;
        let (mut at, mut dir) = (req.at, req.dir.normalize_or_zero());
        if req.trace {
            // Trace(WallHit, WallNormal, Location + 350 * HitDir, Location).
            let from = coords::pos(at.to_array());
            let Ok(d) = Dir3::new(coords::dir(dir.to_array())) else {
                continue;
            };
            let Some(hit) = spatial.cast_ray(from, d, 350.0 * SCALE, true, &crate::collision::world_filter()) else {
                runlog::kv("decal_no_surface", &format!("decal={id} kind={:?}", req.kind));
                continue;
            };
            let n_bevy = if hit.normal.dot(*d) > 0.0 { -hit.normal } else { hit.normal };
            let n = Vec3::new(-n_bevy.z, n_bevy.x, n_bevy.y);
            let vrand = loop {
                let v = Vec3::new(frand(&mut rng) * 2.0 - 1.0, frand(&mut rng) * 2.0 - 1.0, frand(&mut rng) * 2.0 - 1.0);
                if v.length_squared() > 1e-4 && v.length_squared() <= 1.0 {
                    break v.normalize();
                }
            };
            at = to_ue(from + *d * hit.distance) + 20.0 * (n + vrand);
            dir = -n;
        }
        if dir == Vec3::ZERO || class.textures.is_empty() {
            continue;
        }
        let pick = (frand(&mut rng) * class.textures.len() as f32) as usize % class.textures.len();
        let (material, px) = class.textures[pick].clone();
        // KF's own decals pick their DrawScale in PostBeginPlay:
        // KFBloodSplatterDecal (Rand(2) - 0.7) + (Rand(1) + 0.05),
        // KFBloodStreakDecal Rand(2) - 0.6. A negative scale is read as mirrored.
        let coin = if frand(&mut rng) < 0.5 { 0.0 } else { 1.0 };
        let scale = match req.kind {
            DecalKind::FloorSplat => coin - 0.65,
            DecalKind::Streak => coin - 0.6,
            _ => class.draw_scale,
        };
        // Projector frame: X along the projection, roll random if RandomOrient.
        let k = 65536.0 / std::f32::consts::TAU;
        let roll = if class.random_orient { frand(&mut rng) * 65536.0 } else { 0.0 };
        let axes = coords::ue_rotation_matrix(Rotator {
            pitch: (dir.z.clamp(-1.0, 1.0).asin() * k) as i32,
            yaw: (dir.y.atan2(dir.x) * k) as i32,
            roll: roll as i32,
        });
        let (x, y, z) = (axes.col(0), axes.col(1), axes.col(2));
        // SetLocation(Location - Vector(Rotation) * PushBack).
        let origin = at - x * class.push_back;
        let half = px * scale.abs() * 0.5;
        // The box in Bevy space, for the triangle lookup.
        let corners: Vec<Vec3> = [0.0, class.depth]
            .iter()
            .flat_map(|&dx| [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(sy, sz)| origin + x * dx + y * (sy * half.x) + z * (sz * half.y)))
            .map(|p| coords::pos(p.to_array()))
            .collect();
        let (lo, hi) = corners.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        let (mut positions, mut uvs, mut indices) = (Vec::new(), Vec::new(), Vec::new());
        let mut tris_used = 0;
        for tri in surfaces.near(lo, hi) {
            // Into the projector's frame (Unreal units).
            let local: Vec<Vec3> = tri.iter().map(|p| {
                let r = to_ue(*p) - origin;
                Vec3::new(r.dot(x), r.dot(y), r.dot(z))
            }).collect();
            // Skip surfaces seen edge-on (both sides accepted: collision
            // winding is not reliable).
            let n = (local[1] - local[0]).cross(local[2] - local[0]).normalize_or_zero();
            if n.x.abs() < 0.1 {
                continue;
            }
            let mut poly = local;
            for (pn, d) in [
                (Vec3::NEG_X, 0.0),
                (Vec3::X, class.depth),
                (Vec3::Y, half.x),
                (Vec3::NEG_Y, half.x),
                (Vec3::Z, half.y),
                (Vec3::NEG_Z, half.y),
            ] {
                poly = clip(&poly, pn, d);
                if poly.len() < 3 {
                    break;
                }
            }
            if poly.len() < 3 {
                continue;
            }
            tris_used += 1;
            let base = positions.len() as u32;
            for p in &poly {
                let world = origin + x * (p.x - LIFT) + y * p.y + z * p.z;
                positions.push(coords::pos(world.to_array()).to_array());
                let u = 0.5 + p.y / (2.0 * half.x) * scale.signum();
                let v = 0.5 - p.z / (2.0 * half.y);
                uvs.push([u, v]);
            }
            for k in 1..poly.len() as u32 - 1 {
                indices.extend([base, base + k, base + k + 1]);
            }
        }
        runlog::kv(
            "decal_spawned",
            &format!(
                "decal={id} kind={:?} class={} at_unreal=({:.0}, {:.0}, {:.0}) dir=({:.2}, {:.2}, {:.2}) size=({:.0}, {:.0}) scale={scale:.2} surfaces={tris_used} vertices={}",
                req.kind,
                class.name,
                at.x,
                at.y,
                at.z,
                dir.x,
                dir.y,
                dir.z,
                half.x * 2.0,
                half.y * 2.0,
                positions.len()
            ),
        );
        if indices.is_empty() {
            continue;
        }
        let n = positions.len();
        let mesh = meshes.add(
            Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
                .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
                .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; n])
                .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
                .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32, 1.0, 1.0, if class.fade_in > 0.0 { 0.0 } else { 1.0 }]; n])
                .with_inserted_indices(Indices::U32(indices)),
        );
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            bevy::light::NotShadowCaster,
            Decal {
                id,
                age: 0.0,
                life: class.life,
                fade_in: class.fade_in,
                mesh,
                vertices: n,
            },
        ));
    }
}

/// KFMonster.KImpact: when a ragdoll part hits something, trace 16 units
/// either side of the contact along its normal; if that finds a surface, at
/// most every BloodStreakInterval (0.25 s) and only if the impact is more
/// than sqrt(1400) (~37) units from the previous impact (the first impact
/// compares with (0,0,0) as KF does, so it never streaks), put a
/// KFBloodStreakDecal there facing into the surface.
#[allow(clippy::type_complexity, clippy::too_many_arguments)] // Bevy system parameters
fn ragdoll_streaks(
    mut starts: MessageReader<CollisionStart>,
    collisions: Collisions,
    bodies: Query<(), With<crate::ragdoll::RagdollBody>>,
    ragdolls: Query<(Entity, &crate::ragdoll::RagdollState)>,
    spatial: SpatialQuery,
    time: Res<Time>,
    mut decals: MessageWriter<SpawnDecal>,
    // Per corpse: last streak time, last impact position (Unreal units).
    mut memory: Local<HashMap<Entity, (f32, Vec3)>>,
) {
    let now = time.elapsed_secs();
    for start in starts.read() {
        // A ragdoll part against something that is not a ragdoll part.
        let (part, other) = match (bodies.contains(start.collider1), bodies.contains(start.collider2)) {
            (true, false) => (start.collider1, start.collider2),
            (false, true) => (start.collider2, start.collider1),
            _ => continue,
        };
        let Some(corpse) = ragdolls.iter().find(|(_, r)| r.bodies.contains(&part)).map(|(e, _)| e) else {
            continue;
        };
        let Some(pair) = collisions.get(part, other) else {
            continue;
        };
        let Some((point, normal)) = pair.manifolds.iter().find_map(|m| m.points.first().map(|p| (p.point, m.normal))) else {
            continue;
        };
        let pos = to_ue(point);
        let entry = memory.entry(corpse).or_insert((f32::MIN, Vec3::ZERO));
        let dist_sq = if entry.1 == Vec3::ZERO { 0.0 } else { (entry.1 - pos).length_squared() };
        entry.1 = pos;
        if now <= entry.0 + 0.25 || dist_sq < 1400.0 {
            continue;
        }
        // Trace(pos - impactNorm * 16, pos + impactNorm * 16).
        let n = normal.normalize_or_zero();
        let from = point + n * 16.0 * SCALE;
        let Ok(d) = Dir3::new(-n) else {
            continue;
        };
        let Some(hit) = spatial.cast_ray(from, d, 32.0 * SCALE, true, &crate::collision::world_filter()) else {
            continue;
        };
        let wall_n = if hit.normal.dot(*d) > 0.0 { -hit.normal } else { hit.normal };
        entry.0 = now;
        decals.write(SpawnDecal {
            kind: DecalKind::Streak,
            at: to_ue(from + *d * hit.distance),
            dir: -Vec3::new(-wall_n.z, wall_n.x, wall_n.y),
            trace: false,
        });
    }
    memory.retain(|e, _| ragdolls.contains(*e));
}

/// FadeInTime at the start, a fade over the last second, then removal.
fn fade_decals(mut commands: Commands, time: Res<Time>, mut decals: Query<(Entity, &mut Decal)>, mut meshes: ResMut<Assets<Mesh>>) {
    let dt = time.delta_secs();
    for (entity, mut d) in &mut decals {
        d.age += dt;
        if d.age >= d.life {
            runlog::kv("decal_removed", &format!("decal={} age={:.1}", d.id, d.age));
            commands.entity(entity).despawn();
            continue;
        }
        let fading_in = d.fade_in > 0.0 && d.age - dt < d.fade_in;
        let fading_out = d.age > d.life - FADE_OUT;
        if !(fading_in || fading_out) {
            continue;
        }
        let mut alpha = 1.0f32;
        if d.fade_in > 0.0 {
            alpha *= (d.age / d.fade_in).min(1.0);
        }
        if fading_out {
            alpha *= ((d.life - d.age) / FADE_OUT).clamp(0.0, 1.0);
        }
        if let Some(mut mesh) = meshes.get_mut(&d.mesh) {
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32, 1.0, 1.0, alpha]; d.vertices]);
        }
    }
}
