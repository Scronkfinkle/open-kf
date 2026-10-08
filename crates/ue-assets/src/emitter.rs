//! UE2 particle effects: an `Emitter` actor class lists sub-emitter objects
//! (SpriteEmitter, MeshEmitter, ...) in its `Emitters` default. Each is a
//! plain property list; unset properties take the engine's class defaults
//! (Engine.ParticleEmitter, Engine.SpriteEmitter, ...). This module only
//! reads them; simulating the particles is up to the caller.
//!
//! Enum values follow Engine/ParticleEmitter.uc (EParticleDrawStyle,
//! EParticleCoordinateSystem, ...).

use std::rc::Rc;

use crate::class_defaults::ClassDefaults;
use crate::package::ObjectRef;
use crate::package_set::{LoadedPackage, ObjectHandle, PackageSet};
use crate::properties::{PropertyList, Rotator, Value, read_export_properties_ext};
use crate::reader::Reader;

/// A (min, max) range.
pub type Range = (f32, f32);

#[derive(Clone, Debug, PartialEq)]
pub enum EmitterKind {
    Sprite,
    Mesh,
    Spark,
    Beam,
    Trail,
    Other(String),
}

/// One sub-emitter with every value we use resolved (own value, else the
/// class default). Units: Unreal units and seconds; vectors as [X, Y, Z].
#[derive(Clone, Debug)]
pub struct EmitterDef {
    pub name: String,
    pub kind: EmitterKind,
    // Spawning.
    pub max_particles: i32,
    pub respawn_dead_particles: bool,
    pub automatic_initial_spawning: bool,
    pub initial_particles_per_second: f32,
    pub particles_per_second: f32,
    pub lifetime: Range,
    /// Seconds the emitter waits before it starts (random in the range).
    pub initial_delay_range: Range,
    pub seconds_before_inactive: f32,
    pub reset_after_change: bool,
    // Start location.
    /// Added to every start position before the shapes (and turned with them).
    pub start_location_offset: [f32; 3],
    pub start_location_range: [Range; 3],
    /// EParticleStartLocationShape: 0 box, 1 sphere, 2 polar, 3 all.
    pub start_location_shape: u8,
    pub sphere_radius_range: Range,
    /// Index of another emitter whose particles' positions are added; -1 none.
    pub add_location_from_other_emitter: i32,
    /// EParticleCoordinateSystem: 0 independent (world), 1 relative (moves
    /// with the effect), 2 absolute.
    pub coordinate_system: u8,
    // Motion.
    pub start_velocity_range: [Range; 3],
    /// EParticleVelocityDirection: 0 none, 1 start position and owner, 2
    /// owner and start position, 3 add radial.
    pub get_velocity_direction_from: u8,
    /// Speed added along the particle-to-effect direction (AddRadial).
    pub start_velocity_radial_range: Range,
    pub velocity_loss_range: [Range; 3],
    pub max_abs_velocity: [f32; 3],
    pub acceleration: [f32; 3],
    pub use_collision: bool,
    pub damping_factor_range: [Range; 3],
    pub use_velocity_scale: bool,
    /// (relative time, velocity multiplier).
    pub velocity_scale: Vec<(f32, [f32; 3])>,
    // Size.
    pub start_size_range: [Range; 3],
    pub uniform_size: bool,
    pub use_size_scale: bool,
    pub use_regular_size_scale: bool,
    /// (relative time, relative size).
    pub size_scale: Vec<(f32, f32)>,
    pub scale_size_by_velocity_multiplier: [f32; 3],
    pub scale_size_by_velocity_max: f32,
    /// ScaleSizeXByVelocity, Y, Z.
    pub scale_size_by_velocity: [bool; 3],
    // Colour and fading.
    pub use_color_scale: bool,
    /// (relative time, RGBA).
    pub color_scale: Vec<(f32, [u8; 4])>,
    pub opacity: f32,
    pub fade_in: bool,
    pub fade_in_end_time: f32,
    pub fade_out: bool,
    pub fade_out_start_time: f32,
    // Spin.
    pub spin_particles: bool,
    pub start_spin_range: [Range; 3],
    pub spins_per_second_range: [Range; 3],
    pub damp_rotation: bool,
    /// EParticleRotationSource: 0 none, 1 actor, 2 offset, 3 normal.
    pub use_rotation_from: u8,
    /// Used by UseRotationFrom Offset (and with Actor).
    pub rotation_offset: Rotator,
    /// Used by UseRotationFrom Normal: the rotation of this vector.
    pub rotation_normal: [f32; 3],
    /// EParticleEffectAxis: 0 negative X, 1 positive Z.
    pub effect_axis: u8,
    /// Turn VelocityLossRange with the start rotation too.
    pub rotate_velocity_loss_range: bool,
    // Drawing.
    /// EParticleDrawStyle: 0 regular, 1 alpha blend, 2 modulated, 3
    /// translucent (additive), 4 alpha modulate, 5 darken, 6 brighten.
    pub draw_style: u8,
    /// Sprites: EParticleDirectionUsage (0 none = face the camera).
    pub use_direction_as: u8,
    pub projection_normal: [f32; 3],
    /// Paths of the texture and (mesh emitters) the mesh; the objects
    /// themselves are in `EmitterAssets`.
    pub texture: Option<String>,
    pub texture_u_subdivisions: i32,
    pub texture_v_subdivisions: i32,
    pub blend_between_subdivisions: bool,
    pub use_random_subdivision: bool,
    pub static_mesh: Option<String>,
    pub disabled: bool,
    // Trigger (ParticleEmitter.Trigger): with TriggerDisabled off, a
    // trigger spawns SpawnOnTriggerRange particles at SpawnOnTriggerPPS;
    // with it on (the default), a trigger switches Disabled.
    pub trigger_disabled: bool,
    pub reset_on_trigger: bool,
    pub spawn_on_trigger: Range,
    pub spawn_on_trigger_pps: f32,
}

/// The objects a sub-emitter draws with.
#[derive(Clone, Debug)]
pub struct EmitterAssets {
    pub texture: Option<ObjectHandle>,
    pub static_mesh: Option<ObjectHandle>,
}

/// An Emitter actor class: its sub-emitters and its own actor settings.
#[derive(Clone, Debug)]
pub struct EmitterEffect {
    pub class: String,
    pub emitters: Vec<(EmitterDef, EmitterAssets)>,
    pub auto_destroy: bool,
    /// Actor LifeSpan (0 = forever).
    pub life_span: f32,
}

/// Reads one sub-emitter object.
fn read_def(set: &PackageSet, defaults: &ClassDefaults, h: &ObjectHandle) -> Result<(EmitterDef, EmitterAssets), String> {
    let pkg = &h.package.pkg;
    let own = read_export_properties_ext(pkg, h.export, &["ColorScale", "SizeScale", "VelocityScale"])
        .map_err(|e| e.to_string())?;
    let class = defaults.class_of(&h.package, h.export).ok_or("emitter class not found")?;
    let get = |p: &str| -> Option<(Value, Rc<LoadedPackage>)> {
        match own.get(pkg, p) {
            Some(v) => Some((v.clone(), h.package.clone())),
            None => defaults.get(&class, p),
        }
    };
    let float = |p: &str, d: f32| match get(p) {
        Some((Value::Float(f), _)) => f,
        Some((Value::Int(i), _)) => i as f32,
        Some((Value::Byte(b), _)) => b as f32,
        _ => d,
    };
    let int = |p: &str, d: i32| match get(p) {
        Some((Value::Int(i), _)) => i,
        Some((Value::Byte(b), _)) => b as i32,
        _ => d,
    };
    let byte = |p: &str| match get(p) {
        Some((Value::Byte(b), _)) => b,
        _ => 0,
    };
    let boolean = |p: &str| matches!(get(p), Some((Value::Bool(true), _)));
    let vector = |p: &str, d: [f32; 3]| match get(p) {
        Some((Value::Vector(v), _)) => v,
        Some((Value::TaggedStruct { props, .. }, lp)) => {
            let f = |n: &str| struct_float(&lp, &props, n).unwrap_or(0.0);
            [f("X"), f("Y"), f("Z")]
        }
        _ => d,
    };
    let range = |p: &str, d: Range| match get(p) {
        Some((Value::TaggedStruct { props, .. }, lp)) => struct_range(&lp, &props).unwrap_or(d),
        _ => d,
    };
    // A struct property only stores the members that differ, so an axis
    // the emitter leaves out keeps the class default's (e.g. StartSizeRange
    // Y and Z stay 100 when only X is set).
    let axes_of = |v: Option<(Value, Rc<LoadedPackage>)>| -> [Option<Range>; 3] {
        match v {
            Some((Value::TaggedStruct { props, .. }, lp)) => ["X", "Y", "Z"].map(|axis| match props.get(&lp.pkg, axis) {
                Some(Value::TaggedStruct { props: inner, .. }) => Some(struct_range(&lp, inner).unwrap_or((0.0, 0.0))),
                _ => None,
            }),
            _ => [None; 3],
        }
    };
    let range_vector = |p: &str, d: Range| {
        let own_axes = axes_of(own.get(pkg, p).map(|v| (v.clone(), h.package.clone())));
        let class_axes = axes_of(defaults.get(&class, p));
        [0, 1, 2].map(|i| own_axes[i].or(class_axes[i]).unwrap_or(d))
    };
    let object = |p: &str| match get(p) {
        Some((Value::Object(r), lp)) if r != ObjectRef::Null => set.resolve(&lp, r),
        _ => None,
    };
    let elements = |p: &str| -> Option<(Vec<PropertyList>, Rc<LoadedPackage>)> {
        match get(p) {
            Some((Value::StructArray(items), lp)) => Some((items, lp)),
            _ => None,
        }
    };
    let size_scale = elements("SizeScale").map_or_else(Vec::new, |(items, lp)| {
        items
            .iter()
            .map(|e| (struct_float(&lp, e, "RelativeTime").unwrap_or(0.0), struct_float(&lp, e, "RelativeSize").unwrap_or(0.0)))
            .collect()
    });
    let color_scale = elements("ColorScale").map_or_else(Vec::new, |(items, lp)| {
        items
            .iter()
            .map(|e| {
                let color = match e.get(&lp.pkg, "Color") {
                    Some(Value::Color(c)) => *c,
                    _ => [0, 0, 0, 0],
                };
                (struct_float(&lp, e, "RelativeTime").unwrap_or(0.0), color)
            })
            .collect()
    });
    let velocity_scale = elements("VelocityScale").map_or_else(Vec::new, |(items, lp)| {
        items
            .iter()
            .map(|e| {
                let v = match e.get(&lp.pkg, "RelativeVelocity") {
                    Some(Value::Vector(v)) => *v,
                    _ => [0.0; 3],
                };
                (struct_float(&lp, e, "RelativeTime").unwrap_or(0.0), v)
            })
            .collect()
    });
    let kind = match h.class_name() {
        "SpriteEmitter" => EmitterKind::Sprite,
        "MeshEmitter" => EmitterKind::Mesh,
        "SparkEmitter" => EmitterKind::Spark,
        "BeamEmitter" => EmitterKind::Beam,
        "TrailEmitter" => EmitterKind::Trail,
        other => EmitterKind::Other(other.to_string()),
    };
    let assets = EmitterAssets {
        texture: object("Texture"),
        static_mesh: object("StaticMesh"),
    };
    let def = EmitterDef {
        name: pkg.object_name(ObjectRef::Export(h.export)).to_string(),
        kind,
        max_particles: int("MaxParticles", 10),
        respawn_dead_particles: boolean("RespawnDeadParticles"),
        automatic_initial_spawning: boolean("AutomaticInitialSpawning"),
        initial_particles_per_second: float("InitialParticlesPerSecond", 0.0),
        particles_per_second: float("ParticlesPerSecond", 0.0),
        lifetime: range("LifetimeRange", (4.0, 4.0)),
        initial_delay_range: range("InitialDelayRange", (0.0, 0.0)),
        seconds_before_inactive: float("SecondsBeforeInactive", 1.0),
        reset_after_change: boolean("ResetAfterChange"),
        start_location_offset: vector("StartLocationOffset", [0.0; 3]),
        start_location_range: range_vector("StartLocationRange", (0.0, 0.0)),
        start_location_shape: byte("StartLocationShape"),
        sphere_radius_range: range("SphereRadiusRange", (0.0, 0.0)),
        add_location_from_other_emitter: int("AddLocationFromOtherEmitter", -1),
        coordinate_system: byte("CoordinateSystem"),
        start_velocity_range: range_vector("StartVelocityRange", (0.0, 0.0)),
        get_velocity_direction_from: byte("GetVelocityDirectionFrom"),
        start_velocity_radial_range: range("StartVelocityRadialRange", (0.0, 0.0)),
        velocity_loss_range: range_vector("VelocityLossRange", (0.0, 0.0)),
        max_abs_velocity: vector("MaxAbsVelocity", [0.0; 3]),
        acceleration: vector("Acceleration", [0.0; 3]),
        use_collision: boolean("UseCollision"),
        damping_factor_range: range_vector("DampingFactorRange", (1.0, 1.0)),
        use_velocity_scale: boolean("UseVelocityScale"),
        velocity_scale,
        start_size_range: range_vector("StartSizeRange", (100.0, 100.0)),
        uniform_size: boolean("UniformSize"),
        use_size_scale: boolean("UseSizeScale"),
        use_regular_size_scale: boolean("UseRegularSizeScale"),
        size_scale,
        scale_size_by_velocity_multiplier: vector("ScaleSizeByVelocityMultiplier", [1.0; 3]),
        scale_size_by_velocity_max: float("ScaleSizeByVelocityMax", 10_000_000.0),
        scale_size_by_velocity: [boolean("ScaleSizeXByVelocity"), boolean("ScaleSizeYByVelocity"), boolean("ScaleSizeZByVelocity")],
        use_color_scale: boolean("UseColorScale"),
        color_scale,
        opacity: float("Opacity", 1.0),
        fade_in: boolean("FadeIn"),
        fade_in_end_time: float("FadeInEndTime", 0.0),
        fade_out: boolean("FadeOut"),
        fade_out_start_time: float("FadeOutStartTime", 0.0),
        spin_particles: boolean("SpinParticles"),
        start_spin_range: range_vector("StartSpinRange", (0.0, 0.0)),
        spins_per_second_range: range_vector("SpinsPerSecondRange", (0.0, 0.0)),
        damp_rotation: boolean("DampRotation"),
        use_rotation_from: byte("UseRotationFrom"),
        rotation_offset: match get("RotationOffset") {
            Some((Value::Rotator(r), _)) => r,
            _ => Rotator::default(),
        },
        rotation_normal: vector("RotationNormal", [0.0; 3]),
        effect_axis: byte("EffectAxis"),
        rotate_velocity_loss_range: boolean("RotateVelocityLossRange"),
        draw_style: byte("DrawStyle"),
        use_direction_as: byte("UseDirectionAs"),
        projection_normal: vector("ProjectionNormal", [0.0, 0.0, 1.0]),
        texture: assets.texture.as_ref().map(|h| h.path()),
        texture_u_subdivisions: int("TextureUSubdivisions", 0),
        texture_v_subdivisions: int("TextureVSubdivisions", 0),
        blend_between_subdivisions: boolean("BlendBetweenSubdivisions"),
        use_random_subdivision: boolean("UseRandomSubdivision"),
        static_mesh: assets.static_mesh.as_ref().map(|h| h.path()),
        disabled: boolean("Disabled"),
        trigger_disabled: boolean("TriggerDisabled"),
        reset_on_trigger: boolean("ResetOnTrigger"),
        spawn_on_trigger: range("SpawnOnTriggerRange", (0.0, 0.0)),
        spawn_on_trigger_pps: float("SpawnOnTriggerPPS", 0.0),
    };
    Ok((def, assets))
}

fn struct_float(lp: &LoadedPackage, list: &PropertyList, name: &str) -> Option<f32> {
    match list.get(&lp.pkg, name) {
        Some(Value::Float(f)) => Some(*f),
        Some(Value::Int(i)) => Some(*i as f32),
        _ => None,
    }
}

/// A Range struct; a missing Min or Max is 0 (the struct default).
fn struct_range(lp: &LoadedPackage, list: &PropertyList) -> Option<Range> {
    Some((struct_float(lp, list, "Min").unwrap_or(0.0), struct_float(lp, list, "Max").unwrap_or(0.0)))
}

/// Reads an Emitter actor placed in a map (export `export` of `map`): its
/// own `Emitters` (sub-objects in the map), else its class's.
pub fn read_emitter_actor(
    set: &PackageSet,
    defaults: &ClassDefaults,
    map: &std::rc::Rc<LoadedPackage>,
    export: usize,
) -> Result<EmitterEffect, String> {
    let props = crate::properties::read_export_properties(&map.pkg, export).map_err(|e| e.to_string())?;
    let class = defaults.class_of(map, export);
    let (refs, from) = match props.get(&map.pkg, "Emitters") {
        Some(Value::Array { count, raw }) => {
            let mut r = Reader::new(raw);
            ((0..*count).filter_map(|_| r.compact_index().ok().map(ObjectRef::from_raw)).collect::<Vec<_>>(), map.clone())
        }
        _ => match class.as_ref().and_then(|c| defaults.get(c, "Emitters")) {
            Some((Value::Array { count, raw }, p)) => {
                let mut r = Reader::new(&raw);
                ((0..count).filter_map(|_| r.compact_index().ok().map(ObjectRef::from_raw)).collect(), p)
            }
            _ => return Err("no Emitters".into()),
        },
    };
    let mut emitters = Vec::new();
    for (i, rf) in refs.into_iter().enumerate() {
        // An empty slot (Emitters(i)=None, e.g. KFIncendiaryExplosion).
        if rf == ObjectRef::Null {
            continue;
        }
        let h = set.resolve(&from, rf).ok_or_else(|| format!("emitter {i} not found"))?;
        emitters.push(read_def(set, defaults, &h).map_err(|e| format!("emitter {i}: {e}"))?);
    }
    let value = |n: &str| defaults.actor_value(map, export, &props, n);
    Ok(EmitterEffect {
        class: map.pkg.object_name(ObjectRef::Export(export)).to_string(),
        emitters,
        auto_destroy: matches!(value("AutoDestroy"), Some(Value::Bool(true))),
        life_span: match value("LifeSpan") {
            Some(Value::Float(f)) => f,
            _ => 0.0,
        },
    })
}

/// Reads an Emitter actor class (e.g. `KFMod.DismembermentJetHead`).
pub fn read_emitter_class(set: &PackageSet, defaults: &ClassDefaults, class_path: &str) -> Result<EmitterEffect, String> {
    let (pkg_name, class_name) = class_path.split_once('.').ok_or("class path must be Package.Class")?;
    let lp = set.load(pkg_name).ok_or("package not found")?;
    let export = (0..lp.pkg.exports.len())
        .find(|&i| {
            lp.pkg.export_class_name(i) == "Class" && lp.pkg.object_name(ObjectRef::Export(i)).eq_ignore_ascii_case(class_name)
        })
        .ok_or("class not found")?;
    let class = ObjectHandle { package: lp, export };
    let (refs, from) = match defaults.get(&class, "Emitters") {
        Some((Value::Array { count, raw }, p)) => {
            let mut r = Reader::new(&raw);
            let refs: Vec<ObjectRef> = (0..count).filter_map(|_| r.compact_index().ok().map(ObjectRef::from_raw)).collect();
            (refs, p)
        }
        _ => return Err("no Emitters default".into()),
    };
    let mut emitters = Vec::new();
    for (i, rf) in refs.into_iter().enumerate() {
        // An empty slot (Emitters(i)=None, e.g. KFIncendiaryExplosion).
        if rf == ObjectRef::Null {
            continue;
        }
        let h = set.resolve(&from, rf).ok_or_else(|| format!("emitter {i} not found"))?;
        emitters.push(read_def(set, defaults, &h).map_err(|e| format!("emitter {i}: {e}"))?);
    }
    let auto_destroy = matches!(defaults.get(&class, "AutoDestroy"), Some((Value::Bool(true), _)));
    let life_span = match defaults.get(&class, "LifeSpan") {
        Some((Value::Float(f), _)) => f,
        _ => 0.0,
    };
    Ok(EmitterEffect {
        class: class_path.to_string(),
        emitters,
        auto_destroy,
        life_span,
    })
}
