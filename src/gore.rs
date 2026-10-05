//! Gore (see DESIGN.md, "Gore steps A-C"): stumps left where a head or limb
//! came off (ROEffects.Severed*Attachment), severed pieces
//! (ROEffects.SeveredAppendage: limbs, the knife's head) and brain chunks
//! (KFGibBrain / KFGibBrainb, moved as Old2k4.Gib does).
//!
//! Positions and rotations given to the spawn functions are in Unreal world
//! space (units, rotators); entities are placed in Bevy space.

use avian3d::prelude::*;
use bevy::prelude::*;

use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package::ObjectRef;
use ue_assets::package_set::{ObjectHandle, PackageSet};
use ue_assets::properties::{Rotator, Value};
use ue_assets::static_mesh::read_static_mesh;

use crate::coords::{self, SCALE};
use crate::decals::{DecalKind, SpawnDecal};
use crate::map::MapRequest;
use crate::particles::{self, EffectLibrary, ParticleEffect};
use crate::runlog;
use crate::skinned::{SkinnedModel, Skins, decode_image};

/// Gravity for gibs and pieces (Unreal units/s^2).
const GRAVITY: f32 = 950.0;
/// Below this speed (Unreal units/s) after a bounce a piece stops (both
/// Gib.HitWall and SeveredAppendage.HitWall).
const REST_SPEED: f32 = 20.0;
/// RandSpin rates: at spawn (SpawnTrail) and on each bounce (HitWall).
const SPIN_SPAWN: f32 = 64000.0;
const SPIN_BOUNCE: f32 = 100000.0;

/// How a flying piece moves.
#[derive(Clone, Copy)]
struct Motion {
    /// DampenFactor: share of the speed kept on each bounce.
    dampen: f32,
    /// LifeSpan, seconds.
    life: f32,
    /// Collision sphere radius (KF uses small cylinders), Unreal units.
    radius: f32,
    /// SeveredAppendage: on stopping, lie flat (pitch = floor pitch + 16384).
    lie_flat: bool,
}

/// KFGib (brain chunks): DampenFactor 0.4, LifeSpan 6 (KFGibBrain); KF's
/// cylinder is radius 5, height 2.5.
const GIB_MOTION: Motion = Motion {
    dampen: 0.4,
    life: 6.0,
    radius: 2.5,
    lie_flat: false,
};
/// SeveredAppendage: DampenFactor 0.35, LifeSpan 8; KF's cylinder is radius
/// 6, height 3-4.
const PIECE_MOTION: Motion = Motion {
    dampen: 0.35,
    life: 8.0,
    radius: 3.5,
    lie_flat: true,
};
/// SeveredAppendage MaxSpeed: pieces leave at MaxSpeed to 1.5 x MaxSpeed.
const PIECE_MAX_SPEED: f32 = 100.0;

pub struct GorePlugin;

impl Plugin for GorePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, load_gore_assets).add_systems(Update, move_pieces);
    }
}

/// A rigid model for a flying piece: one Bevy mesh and material per section.
#[derive(Clone)]
pub struct PieceModel {
    pub name: String,
    parts: Vec<(Handle<Mesh>, Handle<StandardMaterial>)>,
    draw_scale: f32,
}

impl PieceModel {
    pub fn draw_scale(&self) -> f32 {
        self.draw_scale
    }

    /// Spawns the model's parts as children of `parent`.
    pub fn spawn_parts(&self, commands: &mut Commands, parent: Entity) {
        for (mesh, material) in &self.parts {
            commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), Transform::IDENTITY, ChildOf(parent)));
        }
    }
}

/// Stumps: where a head, arm or leg came off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StumpKind {
    Neck,
    Arm,
    Leg,
}

struct StumpModel {
    model: SkinnedModel,
    /// Bind-pose points (the stumps have one bone each).
    points: Vec<Vec3>,
}

#[derive(Resource)]
pub struct GoreAssets {
    /// SeveredHeadAttachment, SeveredArmAttachment, SeveredLegAttachment.
    stumps: [Option<StumpModel>; 3],
    /// KFGibBrain, KFGibBrainb.
    gibs: Vec<PieceModel>,
}

#[derive(Component)]
struct Piece {
    id: usize,
    motion: Motion,
    /// Bevy space, m/s.
    velocity: Vec3,
    /// Unreal rotator (pitch, yaw, roll) and its rate per second.
    rotation: Vec3,
    spin: Vec3,
    age: f32,
    resting: bool,
    bounces: u32,
    rng: u32,
    /// Its blood trail effect (SpawnTrail), which follows it.
    trail: Option<Entity>,
}

/// Random numbers for gore, as FRand() (0..1).
pub struct Rng(pub u32);

impl Rng {
    pub fn frand(&mut self) -> f32 {
        let r = &mut self.0;
        *r ^= *r << 13;
        *r ^= *r >> 17;
        *r ^= *r << 5;
        (*r % 10000) as f32 / 10000.0
    }

    /// VRand(): a random unit vector.
    fn vrand(&mut self) -> Vec3 {
        loop {
            let v = Vec3::new(self.frand() * 2.0 - 1.0, self.frand() * 2.0 - 1.0, self.frand() * 2.0 - 1.0);
            if v.length_squared() > 1e-4 && v.length_squared() <= 1.0 {
                return v.normalize();
            }
        }
    }

    /// Old2k4.Gib.RandSpin: each rotation rate random in +-rate.
    fn spin(&mut self, rate: f32) -> Vec3 {
        Vec3::new(
            rate * 2.0 * self.frand() - rate,
            rate * 2.0 * self.frand() - rate,
            rate * 2.0 * self.frand() - rate,
        )
    }

    /// GibPerterbation: pitch, yaw and roll each moved by up to
    /// +-perturbation x 32768.
    fn jitter(&mut self, rot: Vec3, perturbation: f32) -> Vec3 {
        let j = perturbation * 32768.0;
        rot + Vec3::new(
            self.frand() * 2.0 * j - j,
            self.frand() * 2.0 * j - j,
            self.frand() * 2.0 * j - j,
        )
    }
}

fn rotator(r: Vec3) -> Rotator {
    Rotator {
        pitch: r.x as i32,
        yaw: r.y as i32,
        roll: r.z as i32,
    }
}

/// GetAxes(Rotation): the Z axis, Unreal space.
fn z_axis(rot: Vec3) -> Vec3 {
    coords::ue_rotation_matrix(rotator(rot)).col(2)
}

/// Rotator(v) for a direction: pitch and yaw, roll 0 (Unreal units).
fn rotator_of_dir(v: Vec3) -> Vec3 {
    let k = 65536.0 / std::f32::consts::TAU;
    let v = v.normalize_or_zero();
    Vec3::new(v.z.clamp(-1.0, 1.0).asin() * k, v.y.atan2(v.x) * k, 0.0)
}

/// KFMonster.TakeDamage's HitNormal for effects: from the hit toward the
/// attacker, a little random, strongly upward; as a rotator (Unreal space).
pub fn hit_normal_rotator(hit: Vec3, attacker: Vec3, rng: &mut Rng) -> Vec3 {
    let n = ((attacker - hit).normalize_or_zero() + rng.vrand() * 0.2 + Vec3::new(0.0, 0.0, 2.8)).normalize();
    rotator_of_dir(n)
}

pub fn find_class(set: &PackageSet, path: &str) -> Option<ObjectHandle> {
    let (pkg_name, class_name) = path.split_once('.')?;
    let lp = set.load(pkg_name)?;
    let export = (0..lp.pkg.exports.len()).find(|&i| {
        lp.pkg.export_class_name(i) == "Class" && lp.pkg.object_name(ObjectRef::Export(i)).eq_ignore_ascii_case(class_name)
    })?;
    Some(ObjectHandle { package: lp, export })
}

fn skins_of(defaults: &ClassDefaults, class: &ObjectHandle) -> Skins {
    match defaults.get(class, "Skins") {
        Some((Value::Array { count, raw }, p)) => {
            let mut r = ue_assets::reader::Reader::new(&raw);
            Skins {
                refs: (0..count).filter_map(|_| r.compact_index().ok().map(ObjectRef::from_raw)).collect(),
                package: Some(p),
                named: Vec::new(),
            }
        }
        _ => Skins {
            refs: Vec::new(),
            package: None,
            named: Vec::new(),
        },
    }
}

fn load_stump(
    set: &PackageSet,
    defaults: &ClassDefaults,
    path: &str,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> Option<StumpModel> {
    let result = (|| {
        let class = find_class(set, path).ok_or("class not found")?;
        let (Value::Object(mesh_ref), mesh_pkg) = defaults.get(&class, "Mesh").ok_or("no Mesh default")? else {
            return Err("Mesh is not an object".to_string());
        };
        let mesh_h = set.resolve(&mesh_pkg, mesh_ref).ok_or("mesh not found")?;
        let model = SkinnedModel::load(set, &mesh_h, &skins_of(defaults, &class), true, meshes, images, materials)?;
        let points = model.skin(&model.bind_pose(), &[]);
        Ok(StumpModel { model, points })
    })();
    match result {
        Ok(s) => {
            let (lo, hi) = s.points.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
            runlog::kv(
                "gore_stump_loaded",
                &format!(
                    "class={path} bones={} triangles={} rot_origin={:?} origin={:?} bounds_min={:?} bounds_max={:?}",
                    s.model.mesh.bones.len(),
                    s.model.mesh.triangles.len(),
                    s.model.mesh.rot_origin,
                    s.model.mesh.origin,
                    lo.to_array(),
                    hi.to_array()
                ),
            );
            Some(s)
        }
        Err(e) => {
            runlog::kv("gore_stump_error", &format!("class={path} error=\"{e}\""));
            None
        }
    }
}

fn load_gore_assets(
    mut commands: Commands,
    request: Res<MapRequest>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let set = PackageSet::new(&request.install_root);
    let defaults = ClassDefaults::new(&set);
    let stumps = [
        "ROEffects.SeveredHeadAttachment",
        "ROEffects.SeveredArmAttachment",
        "ROEffects.SeveredLegAttachment",
    ]
    .map(|path| load_stump(&set, &defaults, path, &mut meshes, &mut images, &mut materials));
    let mut gibs = Vec::new();
    for path in ["KFMod.KFGibBrain", "KFMod.KFGibBrainb"] {
        match find_class(&set, path)
            .ok_or_else(|| "class not found".to_string())
            .and_then(|class| load_piece(&set, &defaults, &class, path, &mut meshes, &mut images, &mut materials))
        {
            Ok(g) => {
                runlog::kv("gore_gib_loaded", &format!("class={path} parts={} draw_scale={}", g.parts.len(), g.draw_scale));
                gibs.push(g);
            }
            Err(e) => runlog::kv("gore_gib_error", &format!("class={path} error=\"{e}\"")),
        }
    }
    commands.insert_resource(GoreAssets { stumps, gibs });
}

/// Loads a gib or severed-piece class's model: its StaticMesh, or (DrawType
/// 2, e.g. SeveredArmClot) its skeletal Mesh in the bind pose.
#[allow(clippy::too_many_arguments)]
pub fn load_piece(
    set: &PackageSet,
    defaults: &ClassDefaults,
    class: &ObjectHandle,
    name: &str,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> Result<PieceModel, String> {
    let draw_scale = match defaults.get(class, "DrawScale") {
        Some((Value::Float(f), _)) => f,
        _ => 1.0,
    };
    let skins = skins_of(defaults, class);
    let skeletal = matches!(defaults.get(class, "DrawType"), Some((Value::Byte(2), _)));
    if skeletal {
        let (Value::Object(mesh_ref), mesh_pkg) = defaults.get(class, "Mesh").ok_or("no Mesh default")? else {
            return Err("Mesh is not an object".into());
        };
        let mesh_h = set.resolve(&mesh_pkg, mesh_ref).ok_or("mesh not found")?;
        let model = SkinnedModel::load(set, &mesh_h, &skins, true, meshes, images, materials)?;
        let points = model.skin(&model.bind_pose(), &[]);
        let to_local = mesh_to_local(&model, 1.0);
        model.upload(&points, |p| coords::pos(to_local(p).to_array()), meshes);
        return Ok(PieceModel {
            name: name.to_string(),
            parts: model.parts.iter().map(|p| (p.mesh.clone(), p.material.clone())).collect(),
            draw_scale,
        });
    }
    let (Value::Object(mesh_ref), mesh_pkg) = defaults.get(class, "StaticMesh").ok_or("no StaticMesh default")? else {
        return Err("StaticMesh is not an object".into());
    };
    let h = set.resolve(&mesh_pkg, mesh_ref).ok_or("static mesh not found")?;
    static_piece(set, &h, &skins, name, draw_scale, meshes, images, materials)
}

/// Like `load_piece` for a class whose StaticMesh is only named in a string
/// (LAWProj.StaticMeshRef, e.g. "KillingFloorStatics.LAWRocket", loaded by
/// PreloadAssets in KF).
#[allow(clippy::too_many_arguments)]
pub fn load_piece_with_mesh(
    set: &PackageSet,
    defaults: &ClassDefaults,
    class: &ObjectHandle,
    mesh_path: &str,
    name: &str,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> Result<PieceModel, String> {
    let draw_scale = match defaults.get(class, "DrawScale") {
        Some((Value::Float(f), _)) => f,
        _ => 1.0,
    };
    let skins = skins_of(defaults, class);
    let (pkg_name, object) = mesh_path.split_once('.').ok_or("mesh path needs Package.Name")?;
    let package = set.load(pkg_name).ok_or("mesh package not found")?;
    let export = package.find(object, Some("StaticMesh")).ok_or("static mesh not found")?;
    static_piece(set, &ObjectHandle { package, export }, &skins, name, draw_scale, meshes, images, materials)
}

#[allow(clippy::too_many_arguments)]
fn static_piece(
    set: &PackageSet,
    h: &ObjectHandle,
    skins: &Skins,
    name: &str,
    draw_scale: f32,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> Result<PieceModel, String> {
    let sm = read_static_mesh(&h.package.pkg, h.export).map_err(|e| e.to_string())?;
    let mut parts = Vec::new();
    for (si, section) in sm.sections.iter().enumerate() {
        let tris = &sm.indices[section.first_index..section.first_index + section.num_triangles * 3];
        if tris.is_empty() {
            continue;
        }
        // Unreal-space vertices; the entity's scale applies DrawScale.
        let positions: Vec<[f32; 3]> = sm.positions.iter().map(|p| coords::pos(*p).to_array()).collect();
        let normals: Vec<[f32; 3]> = sm.normals.iter().map(|n| coords::dir(*n).normalize_or_zero().to_array()).collect();
        let uvs: Vec<[f32; 2]> = sm.uvs.first().cloned().unwrap_or_else(|| vec![[0.0, 0.0]; sm.positions.len()]);
        let mesh = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
            .with_inserted_indices(bevy::mesh::Indices::U32(tris.iter().map(|&i| i as u32).collect()));
        // Skins[section] if set, else the mesh's own material.
        let (rf, from) = match (skins.refs.get(si), &skins.package) {
            (Some(&s), Some(p)) if s != ObjectRef::Null => (s, p.clone()),
            _ => (sm.materials.get(si).copied().unwrap_or(ObjectRef::Null), h.package.clone()),
        };
        let simple = ue_assets::material::resolve(set, &ObjectHandle { package: from, export: 0 }, rf);
        let image = simple.texture.as_ref().and_then(|t| decode_image(t, images));
        let material = materials.add(StandardMaterial {
            base_color_texture: image,
            perceptual_roughness: 0.6,
            reflectance: 0.2,
            cull_mode: None,
            double_sided: true,
            ..default()
        });
        parts.push((meshes.add(mesh), material));
    }
    if parts.is_empty() {
        return Err("no triangles".into());
    }
    Ok(PieceModel {
        name: name.to_string(),
        parts,
        draw_scale,
    })
}

/// A skeletal model's mesh space -> its actor space (MeshOrigin, MeshScale,
/// RotOrigin, then `draw_scale`), Unreal units.
fn mesh_to_local(model: &SkinnedModel, draw_scale: f32) -> impl Fn(Vec3) -> Vec3 + use<> {
    let r = model.mesh.rot_origin;
    let rot = coords::ue_rotation_matrix(Rotator {
        pitch: r[0],
        yaw: r[1],
        roll: r[2],
    });
    let scale = Vec3::from_array(model.mesh.scale);
    let origin = Vec3::from_array(model.mesh.origin);
    move |p| draw_scale * (rot * ((p - origin) * scale))
}

/// Spawns one flying piece at `at` (Unreal world) with rotation `rot`
/// (Unreal rotator) and velocity `velocity` (Unreal units/s), and its
/// trail (SpawnTrail): KFGib a BloodTrail living 1.8 s; SeveredAppendage
/// its BleedingEmitterClass (ROBloodSpurt) living as long as the piece.
#[allow(clippy::too_many_arguments)]
fn spawn_piece(
    commands: &mut Commands,
    fx: &mut Effects,
    model: &PieceModel,
    scale: f32,
    motion: Motion,
    at: Vec3,
    rot: Vec3,
    velocity: Vec3,
    id: usize,
    rng: &mut Rng,
) {
    let translation = coords::pos(at.to_array());
    let spin = rng.spin(SPIN_SPAWN);
    let (trail_class, trail_life) = if motion.lie_flat { ("ROEffects.ROBloodSpurt", motion.life) } else { ("ROEffects.BloodTrail", 1.8) };
    let trail = fx.library.and_then(|lib| {
        let axes = coords::ue_rotation_matrix(rotator(rot));
        particles::spawn_effect_for(commands, lib, fx.meshes, trail_class, at, axes, rng.0, Some(trail_life))
    });
    let parent = commands
        .spawn((
            Transform {
                translation,
                rotation: coords::rotation(rotator(rot)),
                scale: Vec3::splat(scale),
            },
            Visibility::Visible,
            Piece {
                id,
                motion,
                velocity: coords::dir(velocity.to_array()) * SCALE,
                rotation: rot,
                spin,
                age: 0.0,
                resting: false,
                bounces: 0,
                rng: rng.0 ^ 0x5bd1_e995,
                trail,
            },
        ))
        .id();
    for (mesh, material) in &model.parts {
        commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), Transform::IDENTITY, ChildOf(parent)));
    }
    runlog::kv(
        "piece_spawned",
        &format!(
            "piece={id} class={} at_unreal=({:.0}, {:.0}, {:.0}) velocity_unreal=({:.0}, {:.0}, {:.0}) speed_unreal={:.0} scale={scale:.3}",
            model.name,
            at.x,
            at.y,
            at.z,
            velocity.x,
            velocity.y,
            velocity.z,
            velocity.length()
        ),
    );
}

/// KFMonster.KFSpawnGiblet for brain chunks (KFGibBrain, KFGibBrainb,
/// KFGibBrain, ... `count` of them) at `at`, rotated `rot`: each flies along
/// the Z axis of `rot` jittered by `perturbation`, at `speed` + 0-25%, plus
/// the zed's `velocity` (all Unreal). `size` = CollisionRadius x
/// CollisionHeight / 1100 scales them. Returns how many were spawned.
#[allow(clippy::too_many_arguments)]
pub fn spawn_giblets(
    commands: &mut Commands,
    fx: &mut Effects,
    gore: &GoreAssets,
    count: usize,
    at: Vec3,
    rot: Vec3,
    perturbation: f32,
    speed: f32,
    velocity: Vec3,
    size: f32,
    next_id: &mut usize,
    rng: &mut Rng,
) -> usize {
    let mut spawned = 0;
    for n in 0..count {
        let Some(model) = gore.gibs.get(n % 2).or(gore.gibs.first()) else {
            break;
        };
        let dir = z_axis(rng.jitter(rot, perturbation));
        let v = velocity + dir * speed * (1.0 + 0.25 * rng.frand());
        spawn_piece(commands, fx, model, model.draw_scale * size, GIB_MOTION, at, rot, v, *next_id, rng);
        *next_id += 1;
        spawned += 1;
    }
    spawned
}

/// KFMonster.SpawnSeveredGiblet: a severed limb or head flies from `at`,
/// rotated `spawn_rot` (the bone's rotation), along the Z axis of `rot_dir`
/// jittered by `perturbation`, at MaxSpeed to 1.5 x MaxSpeed plus the zed's
/// `velocity`; a head gets 50 extra upward (all Unreal).
#[allow(clippy::too_many_arguments)]
pub fn spawn_severed(
    commands: &mut Commands,
    fx: &mut Effects,
    model: &PieceModel,
    at: Vec3,
    rot_dir: Vec3,
    perturbation: f32,
    spawn_rot: Vec3,
    velocity: Vec3,
    head: bool,
    next_id: &mut usize,
    rng: &mut Rng,
) {
    let dir = z_axis(rng.jitter(rot_dir, perturbation));
    let mut v = velocity + dir * (PIECE_MAX_SPEED + PIECE_MAX_SPEED / 2.0 * rng.frand());
    if head {
        v.z += 50.0;
    }
    spawn_piece(commands, fx, model, model.draw_scale, PIECE_MOTION, at, spawn_rot, v, *next_id, rng);
    *next_id += 1;
}

/// What spawning effects needs: the effect library (if loaded) and meshes.
pub struct Effects<'a> {
    pub library: Option<&'a EffectLibrary>,
    pub meshes: &'a mut Assets<Mesh>,
}

/// Spawns the meshes for a stump as children of `parent`; place them each
/// frame with `place_stump`.
pub fn new_stump(
    gore: &GoreAssets,
    kind: StumpKind,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    parent: Entity,
) -> Option<Vec<Handle<Mesh>>> {
    let stump = gore.stumps[kind as usize].as_ref()?;
    let handles = stump.model.new_instance(meshes);
    for (part, handle) in stump.model.parts.iter().zip(&handles) {
        commands.spawn((Mesh3d(handle.clone()), MeshMaterial3d(part.material.clone()), Transform::IDENTITY, ChildOf(parent)));
    }
    Some(handles)
}

/// Places a stump (meshes `handles`) at the zed's tag `alias` for `pose`,
/// drawn at `scale` (Severed*AttachScale). `to_actor` maps the zed's mesh
/// space to its actor space (Unreal units).
#[allow(clippy::too_many_arguments)]
pub fn place_stump(
    gore: &GoreAssets,
    kind: StumpKind,
    zed_model: &SkinnedModel,
    pose: &[(Quat, Vec3)],
    to_actor: &impl Fn(Vec3) -> Vec3,
    alias: &str,
    scale: f32,
    handles: &[Handle<Mesh>],
    meshes: &mut Assets<Mesh>,
) -> bool {
    let Some(stump) = &gore.stumps[kind as usize] else {
        return false;
    };
    let Some((o, axes)) = zed_model.tag_frame(pose, alias) else {
        return false;
    };
    let origin = to_actor(o);
    let [ax, ay, az] = axes.map(|a| (to_actor(o + a) - origin).normalize_or_zero());
    let local = mesh_to_local(&stump.model, scale);
    let place = |p: Vec3| {
        let s = local(p);
        coords::pos((origin + ax * s.x + ay * s.y + az * s.z).to_array())
    };
    stump.model.upload_to(handles, &stump.points, place, meshes);
    true
}

/// Old2k4.Gib / SeveredAppendage movement: falling, bouncing off the level
/// with damping (HitWall), a fixed spin, removed after LifeSpan.
fn move_pieces(
    time: Res<Time>,
    spatial: SpatialQuery,
    mut commands: Commands,
    mut pieces: Query<(Entity, &mut Piece, &mut Transform)>,
    mut effects: Query<&mut ParticleEffect>,
    mut decals: MessageWriter<SpawnDecal>,
) {
    let dt = time.delta_secs().min(0.1);
    for (entity, mut g, mut t) in &mut pieces {
        g.age += dt;
        let u = t.translation / SCALE;
        let at = format!("({:.0}, {:.0}, {:.0})", -u.z, u.x, u.y);
        // PHYS_Trailer: the trail follows the piece (not its rotation).
        if let Some(trail) = g.trail
            && let Ok(mut fx) = effects.get_mut(trail)
        {
            fx.frame.0 = Vec3::new(-u.z, u.x, u.y);
        }
        if g.age > g.motion.life {
            // Destroyed(): a severed piece destroys its trail, a gib kills it
            // (it stops spawning and fades out).
            if let Some(trail) = g.trail {
                if g.motion.lie_flat {
                    commands.entity(trail).try_despawn();
                } else if let Ok(mut fx) = effects.get_mut(trail) {
                    fx.kill();
                }
            }
            runlog::kv(
                "piece_removed",
                &format!("piece={} at_unreal={at} bounces={} resting={}", g.id, g.bounces, g.resting),
            );
            commands.entity(entity).despawn();
            continue;
        }
        if g.resting {
            continue;
        }
        g.velocity.y -= GRAVITY * SCALE * dt;
        let motion = g.velocity * dt;
        if let Ok(dir) = Dir3::new(motion) {
            let config = ShapeCastConfig {
                max_distance: motion.length(),
                target_distance: 0.0,
                compute_contact_on_penetration: true,
                ignore_origin_penetration: true,
            };
            let shape = Collider::sphere(g.motion.radius * SCALE);
            match spatial.cast_shape(&shape, t.translation, Quat::IDENTITY, dir, &config, &crate::collision::body_filter()) {
                Some(hit) => {
                    t.translation += *dir * hit.distance;
                    let n = if hit.normal1.dot(*dir) > 0.0 { -hit.normal1 } else { hit.normal1 }.normalize_or_zero();
                    // HitWall: Velocity = DampenFactor * ((V . N) N (-2) + V).
                    g.velocity = g.motion.dampen * (g.velocity - 2.0 * g.velocity.dot(n) * n);
                    let mut rng = Rng(g.rng);
                    g.spin = rng.spin(SPIN_BOUNCE);
                    g.rng = rng.0;
                    g.bounces += 1;
                    let speed = g.velocity.length() / SCALE;
                    let pos_ue = Vec3::new(-t.translation.z, t.translation.x, t.translation.y) / SCALE;
                    let into_surface = -Vec3::new(-n.z, n.x, n.y);
                    // SeveredAppendage.HitWall: a drip where it bounces fast
                    // (over MaxSpeed / 3) and where it stops.
                    if g.motion.lie_flat && (speed > PIECE_MAX_SPEED / 3.0 || speed < REST_SPEED) {
                        decals.write(SpawnDecal {
                            kind: DecalKind::Drip,
                            at: pos_ue,
                            dir: into_surface,
                            trace: false,
                        });
                    }
                    // KFGib.HitWall: on stopping, a KFBloodPuff, which 20% of the
                    // time (BloodSpurt.WallSplat) splats the floor 350 below.
                    if !g.motion.lie_flat && speed < REST_SPEED {
                        let mut rng = Rng(g.rng ^ 0x2545_f491);
                        if rng.frand() <= 0.2 {
                            decals.write(SpawnDecal {
                                kind: DecalKind::FloorSplat,
                                at: pos_ue,
                                dir: Vec3::NEG_Z,
                                trace: true,
                            });
                        }
                    }
                    if speed < REST_SPEED {
                        g.resting = true;
                        if g.motion.lie_flat {
                            // SeveredAppendage.HitWall: the trail is destroyed on landing.
                            if let Some(trail) = g.trail.take() {
                                commands.entity(trail).try_despawn();
                            }
                            // LandRot.Pitch = rotator(HitNormal).Pitch + 16384.
                            let n_ue = Vec3::new(-n.z, n.x, n.y);
                            g.rotation.x = rotator_of_dir(n_ue).x + 16384.0;
                            t.rotation = coords::rotation(rotator(g.rotation));
                        }
                        runlog::kv(
                            "piece_rest",
                            &format!("piece={} age={:.2} at_unreal={at} bounces={}", g.id, g.age, g.bounces),
                        );
                        continue;
                    }
                    runlog::kv(
                        "piece_bounce",
                        &format!("piece={} age={:.2} speed_after_unreal={speed:.0} normal_up={:.2}", g.id, g.age, n.y),
                    );
                }
                None => t.translation += motion,
            }
        }
        // bFixedRotationDir: turn at RotationRate.
        let spin = g.spin;
        g.rotation += spin * dt;
        t.rotation = coords::rotation(rotator(g.rotation));
    }
}
