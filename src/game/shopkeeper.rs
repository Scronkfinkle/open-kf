//! The trader woman (T4a): one per WeaponLocker actor the map places in its
//! shops (class default Mesh KF_Soldier_Trip.ShopKeeper_Trip; maps may set
//! Skins, DrawScale3D and CullDistance per actor).
//!
//! KF's WeaponLocker.PostBeginPlay and AnimEnd: `LoopAnim('Idle')`, so she
//! idles forever. SetOpen's `PlayAnim('Gesture')` is commented out in KF's
//! script: opening the shop or walking in changes nothing on her. She is
//! never hidden (waves and trader time alike), only culled beyond her
//! CullDistance. See DESIGN.md, T4.

use std::rc::Rc;

use bevy::camera::visibility::VisibilityRange;
use bevy::prelude::*;
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package::ObjectRef;
use ue_assets::package_set::{LoadedPackage, PackageSet};
use ue_assets::properties::{Rotator, Value, read_export_properties};

use crate::engine::camera::FlyCamera;
use crate::engine::coords::{self, SCALE};
use crate::engine::runlog;
use crate::render::skinned::{SkinnedModel, Skins};

/// One placed trader.
#[derive(Component)]
pub struct Shopkeeper {
    pub name: String,
    /// Index into `ShopkeeperModels`.
    model: usize,
    meshes: Vec<Handle<Mesh>>,
    /// The Idle sequence (None: the model has none; she stands in her bind pose).
    idle: Option<usize>,
    frame: f32,
    /// Mesh space -> actor space (Unreal units, Unreal axes).
    rot_origin: Mat3,
    mesh_origin: Vec3,
    mesh_scale: Vec3,
    draw_scale: Vec3,
    pre_pivot: Vec3,
    /// Actor Location, Unreal units.
    pub location: Vec3,
    /// 0 = never culled.
    cull_distance: f32,
}

impl Shopkeeper {
    /// UE2 mesh-to-actor: point - Origin, x MeshScale, turned by RotOrigin,
    /// x DrawScale x DrawScale3D, + PrePivot (as the zeds, zed/effects.rs).
    fn to_actor(&self, p: Vec3) -> Vec3 {
        self.pre_pivot + self.draw_scale * (self.rot_origin * ((p - self.mesh_origin) * self.mesh_scale))
    }
}

/// The loaded trader models, one per distinct (mesh, skins).
#[derive(Resource, Default)]
pub struct ShopkeeperModels(Vec<SkinnedModel>);

pub struct ShopkeeperPlugin;

impl Plugin for ShopkeeperPlugin {
    fn build(&self, app: &mut App) {
        use crate::world::map_change::MapResourceExt;
        // Spawned by the map loader; per map (world/map_change.rs).
        app.init_resource::<ShopkeeperModels>().reset_on_map_unload::<ShopkeeperModels>().add_systems(Update, (animate_shopkeepers, log_shop_traders));
    }
}

/// An object reference default: the actor's own value (map package), else
/// the class default (its own package).
fn object_value(
    defaults: &ClassDefaults,
    map: &Rc<LoadedPackage>,
    export: usize,
    own: &ue_assets::properties::PropertyList,
    prop: &str,
) -> Option<(ObjectRef, Rc<LoadedPackage>)> {
    if let Some(Value::Object(r)) = own.get(&map.pkg, prop) {
        return Some((*r, map.clone()));
    }
    let class = defaults.class_of(map, export)?;
    match defaults.get(&class, prop)? {
        (Value::Object(r), p) => Some((r, p)),
        _ => None,
    }
}

/// Reads every WeaponLocker in the map and spawns its trader. Called by the
/// map loader (all modes: she is part of the map).
#[allow(clippy::too_many_arguments)]
pub fn spawn_shopkeepers(
    commands: &mut Commands,
    set: &PackageSet,
    defaults: &ClassDefaults,
    map: &Rc<LoadedPackage>,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) {
    let pkg = &map.pkg;
    let mut models: Vec<SkinnedModel> = Vec::new();
    // (mesh path, skin refs) of each loaded model.
    let mut keys: Vec<(String, Vec<ObjectRef>)> = Vec::new();
    let mut spawned = 0;
    for i in pkg.level_actor_exports() {
        if !defaults.class_of(map, i).is_some_and(|c| defaults.is_a(&c, "WeaponLocker")) {
            continue;
        }
        let name = pkg.object_name(ObjectRef::Export(i)).to_string();
        let Ok(props) = read_export_properties(pkg, i) else {
            runlog::kv("shopkeeper_error", &format!("actor={name} error=\"properties unreadable\""));
            continue;
        };
        let value = |n: &str| defaults.actor_value(map, i, &props, n);
        if defaults.actor_bool(map, i, &props, "bHidden") {
            runlog::kv("shopkeeper_skipped", &format!("actor={name} reason=bHidden"));
            continue;
        }
        let float = |n: &str, d: f32| match value(n) {
            Some(Value::Float(f)) => f,
            Some(Value::Int(x)) => x as f32,
            _ => d,
        };
        let vector = |n: &str, d: Vec3| match value(n) {
            Some(Value::Vector(v)) => Vec3::from_array(v),
            _ => d,
        };
        let location = vector("Location", Vec3::ZERO);
        let rotation = match props.get(pkg, "Rotation") {
            Some(Value::Rotator(r)) => *r,
            _ => Rotator { pitch: 0, yaw: 0, roll: 0 },
        };
        let Some((mesh_ref, mesh_pkg)) = object_value(defaults, map, i, &props, "Mesh") else {
            runlog::kv("shopkeeper_error", &format!("actor={name} error=\"no Mesh\""));
            continue;
        };
        let Some(mesh_h) = set.resolve(&mesh_pkg, mesh_ref) else {
            runlog::kv("shopkeeper_error", &format!("actor={name} error=\"mesh not found\""));
            continue;
        };
        // Skins: the actor's own array (map package); WeaponLocker sets none
        // by default, so the mesh's own material is used otherwise.
        let skins = match props.get(pkg, "Skins") {
            Some(Value::Array { count, raw }) => {
                let mut r = ue_assets::reader::Reader::new(raw);
                Skins {
                    refs: (0..*count).filter_map(|_| r.compact_index().ok().map(ObjectRef::from_raw)).collect(),
                    package: Some(map.clone()),
                    named: Vec::new(),
                }
            }
            _ => Skins { refs: Vec::new(), package: None, named: Vec::new() },
        };
        let key = (mesh_h.path(), skins.refs.clone());
        let model = match keys.iter().position(|k| *k == key) {
            Some(m) => m,
            None => match SkinnedModel::load(set, &mesh_h, &skins, true, meshes, images, materials) {
                Ok(m) => {
                    let idle = m.sequence("Idle");
                    let seqs: Vec<String> = m.anim.as_ref().map_or(Vec::new(), |a| a.sequences.iter().map(|s| s.name.clone()).collect());
                    runlog::kv(
                        "shopkeeper_model",
                        &format!(
                            "mesh={} skins={:?} bones={} triangles={} parts={} sequences={seqs:?} idle={:?} idle_frames={:.0} idle_rate={:.1} mesh_origin={:?} mesh_scale={:?} rot_origin={:?}",
                            key.0,
                            skins.refs.iter().map(|&r| skins.package.as_ref().map_or(String::new(), |p| p.pkg.object_name(r).to_string())).collect::<Vec<_>>(),
                            m.mesh.bones.len(),
                            m.mesh.triangles.len(),
                            m.parts.len(),
                            idle,
                            idle.map_or(0.0, |s| m.length(s)),
                            idle.map_or(0.0, |s| m.rate(s)),
                            m.mesh.origin,
                            m.mesh.scale,
                            m.mesh.rot_origin,
                        ),
                    );
                    models.push(m);
                    keys.push(key);
                    models.len() - 1
                }
                Err(e) => {
                    runlog::kv("shopkeeper_error", &format!("actor={name} mesh={} error=\"{e}\"", key.0));
                    continue;
                }
            },
        };
        let m = &models[model];
        let r = m.mesh.rot_origin;
        let keeper = Shopkeeper {
            name: name.clone(),
            model,
            meshes: m.new_instance(meshes),
            idle: m.sequence("Idle"),
            frame: 0.0,
            rot_origin: coords::ue_rotation_matrix(Rotator { pitch: r[0], yaw: r[1], roll: r[2] }),
            mesh_origin: Vec3::from_array(m.mesh.origin),
            mesh_scale: Vec3::from_array(m.mesh.scale),
            draw_scale: float("DrawScale", 1.0) * vector("DrawScale3D", Vec3::ONE),
            pre_pivot: vector("PrePivot", Vec3::ZERO),
            location,
            cull_distance: float("CullDistance", 0.0),
        };
        // First pose now, so the meshes' bounds are right from the start.
        let (skinned, _) = m.pose_with_bones(keeper.idle, 0.0);
        m.upload_to(&keeper.meshes, &skinned, |p| coords::pos(keeper.to_actor(p).to_array()), meshes);
        // Her feet and head in the world, against the bottom of her
        // collision cylinder (Location.Z - CollisionHeight).
        let turn = coords::ue_rotation_matrix(rotation);
        let (lo, hi) = skinned.iter().map(|&p| (turn * keeper.to_actor(p)).z).fold((f32::MAX, f32::MIN), |(a, b), z| (a.min(z), b.max(z)));
        let col_height = float("CollisionHeight", 0.0);
        runlog::kv(
            "shopkeeper_spawned",
            &format!(
                "actor={name} at_unreal=({:.0}, {:.0}, {:.0}) rotation=({}, {}, {}) draw_scale=({:.2}, {:.2}, {:.2}) cull_distance={:.0} feet_z={:.1} head_z={:.1} cylinder_bottom_z={:.1} collide={} block={}",
                location.x,
                location.y,
                location.z,
                rotation.pitch,
                rotation.yaw,
                rotation.roll,
                keeper.draw_scale.x,
                keeper.draw_scale.y,
                keeper.draw_scale.z,
                keeper.cull_distance,
                location.z + lo,
                location.z + hi,
                location.z - col_height,
                defaults.actor_bool(map, i, &props, "bCollideActors"),
                defaults.actor_bool(map, i, &props, "bBlockActors"),
            ),
        );
        let parts: Vec<(Handle<Mesh>, Handle<StandardMaterial>)> = m.parts.iter().zip(&keeper.meshes).map(|(p, h)| (h.clone(), p.material.clone())).collect();
        let cull = keeper.cull_distance;
        // Lit by the map (render/actor_light.rs): her own MaxLights and
        // AmbientGlow (class defaults: Actor 4 and 0).
        let byte = |n: &str, d: u8| match defaults.actor_value(map, i, &props, n) {
            Some(Value::Byte(b)) => b,
            _ => d,
        };
        let light = crate::render::actor_light::ActorLight::new(format!("trader_{name}"), Vec3::ZERO, byte("MaxLights", 4).max(1) as usize, byte("AmbientGlow", 0));
        let parent = commands
            .spawn((
                Transform { translation: coords::pos(location.to_array()), rotation: coords::rotation(rotation), ..default() },
                Visibility::Visible,
                Name::new(name.clone()),
                keeper,
                light,
                crate::world::map_change::MapScoped,
            ))
            .id();
        for (mesh, material) in parts {
            let mut e = commands.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::IDENTITY,
                ChildOf(parent),
                crate::render::actor_light::LitPart { owner: parent, animated: true, own: None },
            ));
            if cull > 0.0 {
                e.insert(VisibilityRange::abrupt(0.0, cull * SCALE));
            }
        }
        spawned += 1;
    }
    runlog::kv("shopkeepers_ready", &format!("count={spawned} models={}", models.len()));
    commands.insert_resource(ShopkeeperModels(models));
}

/// LoopAnim('Idle') at the sequence's own rate (game time: zed time slows
/// her too, as KF's TimeDilation does). Skinned only while the view is
/// within her CullDistance (beyond it she is not drawn).
fn animate_shopkeepers(
    time: Res<Time>,
    models: Res<ShopkeeperModels>,
    mut keepers: Query<&mut Shopkeeper>,
    view: Query<&Transform, With<FlyCamera>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut since_log: Local<f32>,
) {
    let dt = time.delta_secs();
    *since_log += dt;
    let log_now = *since_log >= 5.0;
    if log_now {
        *since_log = 0.0;
    }
    let eye = view.single().ok().map(|t| t.translation / SCALE);
    for mut k in keepers.iter_mut() {
        let Some(m) = models.0.get(k.model) else { continue };
        let Some(s) = k.idle else { continue };
        let len = m.length(s).max(1e-3);
        k.frame = (k.frame + dt * m.rate(s)) % len;
        // Distance from the view to her Location, Unreal units.
        let dist = eye.map_or(0.0, |e| (e - coords::pos(k.location.to_array()) / SCALE).length());
        let in_range = k.cull_distance <= 0.0 || dist <= k.cull_distance;
        if in_range {
            let (skinned, _) = m.pose_with_bones(Some(s), k.frame);
            m.upload_to(&k.meshes, &skinned, |p| coords::pos(k.to_actor(p).to_array()), &mut meshes);
        }
        if log_now {
            runlog::kv("shopkeeper_anim", &format!("actor={} frame={:.1}/{len:.0} distance={dist:.0} drawn={in_range}", k.name, k.frame));
        }
    }
}

/// WeaponLocker.PreBeginPlay: every ShopVolume within 1000 units becomes
/// hers (Shop.MyTrader; the last one found wins). KF also needs the shop
/// visible from her (VisibleCollidingActors); not checked here. Logged once,
/// wave mode only (the shops are loaded there).
fn log_shop_traders(shops: Option<Res<crate::game::trader::Shops>>, keepers: Query<&Shopkeeper>, (epoch, mut done): (Res<crate::world::map_change::MapEpoch>, Local<crate::world::map_change::OncePerMap>)) {
    let Some(shops) = shops else { return };
    if done.done(&epoch) || keepers.is_empty() || shops.shops.is_empty() {
        return;
    }
    done.set(&epoch);
    let lines: Vec<String> = shops
        .shops
        .iter()
        .map(|s| {
            let mine: Vec<String> = keepers
                .iter()
                .filter(|k| (k.location - s.location).length() <= 1000.0)
                .map(|k| format!("{}@{:.0}", k.name, (k.location - s.location).length()))
                .collect();
            format!("{}:[{}]", s.name, mine.join(" "))
        })
        .collect();
    runlog::kv("shop_traders", &lines.join(" "));
}
