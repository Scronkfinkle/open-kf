//! What a map contains: the level's BSP model, placed static meshes, player starts.
//!
//! Actors are found by scanning the map's exports rather than reading the
//! level's actor list. A saved map only contains objects that are in use, so
//! the result is the same, and it avoids another binary layout.

use std::collections::{BTreeMap, HashSet};

use std::rc::Rc;

use crate::class_defaults::ClassDefaults;
use crate::package::{ObjectRef, Package};
use crate::package_set::LoadedPackage;
use crate::properties::{PropertyList, Rotator, Value, read_export_properties};

/// `EDrawType::DT_StaticMesh`.
const DT_STATIC_MESH: u8 = 8;

#[derive(Debug, Clone)]
pub struct MeshActor {
    pub export: usize,
    pub class: String,
    pub mesh: ObjectRef,
    pub location: [f32; 3],
    pub rotation: Rotator,
    /// DrawScale multiplied into DrawScale3D.
    pub scale: [f32; 3],
    pub pre_pivot: [f32; 3],
    /// Per-section material overrides from the `Skins` property (Null = keep
    /// the mesh's own material for that section).
    pub skins: Vec<ObjectRef>,
    /// Blocks the player: bCollideActors, bBlockActors and
    /// bBlockNonZeroExtentTraces all true (own value or class default).
    /// Always false when class defaults were not available.
    pub blocks_player: bool,
    /// Set for KFDoorMover actors: what the door needs to move.
    pub door: Option<DoorInfo>,
    /// Set for KFGlassMover actors (breakable windows).
    pub glass: Option<GlassInfo>,
    /// bUnlit: drawn without lighting.
    pub unlit: bool,
}

/// A KFGlassMover: a pane of breakable glass.
#[derive(Debug, Clone)]
pub struct GlassInfo {
    pub name: String,
    /// Panes sharing a Tag crack together when one breaks.
    pub tag: String,
    pub health: i32,
}

/// A KFDoorMover's mover and door settings (own value, else class default).
#[derive(Debug, Clone)]
pub struct DoorInfo {
    /// Object name, e.g. `KFDoorMover6`, for logs.
    pub name: String,
    /// Tag: the KFUseTrigger whose Event matches owns this door.
    pub tag: String,
    pub base_pos: [f32; 3],
    pub base_rot: Rotator,
    /// KeyPos / KeyRot, all 24 (unset = zero).
    pub key_pos: Vec<[f32; 3]>,
    pub key_rot: Vec<Rotator>,
    pub key_num: u8,
    pub num_keys: u8,
    pub move_time: f32,
    pub delay_time: f32,
    /// MoverGlideType: 0 MV_MoveByTime, 1 MV_GlideByTime.
    pub glide_type: u8,
    pub initial_state: String,
    pub start_sealed: bool,
    pub start_sealed_weld_prc: f32,
    pub disallow_weld: bool,
    pub key_locked: bool,
    pub no_seal: bool,
    pub small_arms_damage: bool,
    pub zombies_ignore: bool,
    pub block_damaging_of_weld: bool,
    /// EST_Metal (4) or anything else (wood break effects).
    pub surface_type: u8,
    pub is_leader: bool,
    pub return_group: String,
    /// bBlockZeroExtentTraces: bullets stop on it.
    pub blocks_traces: bool,
}

/// A KFUseTrigger: the cylinder players press USE in, and zeds walk into,
/// to open the doors whose Tag is its Event.
#[derive(Debug, Clone)]
pub struct UseTriggerInfo {
    pub name: String,
    pub event: String,
    pub location: [f32; 3],
    pub rotation: Rotator,
    pub radius: f32,
    pub height: f32,
    pub refire_delay: i32,
    pub max_weld_strength: f32,
    pub combat_seal_reduction: f32,
    pub directional_open: bool,
    pub message: String,
    pub always_show_message: bool,
}

/// A brush actor that blocks movement (BlockingVolume and similar).
#[derive(Debug, Clone)]
pub struct BlockingBrush {
    pub export: usize,
    pub class: String,
    /// The brush's Model export (its Polys hold the shape).
    pub model: ObjectRef,
    pub location: [f32; 3],
    pub rotation: Rotator,
    pub pre_pivot: [f32; 3],
    /// BlockingVolume.bClassBlocker: only these classes are blocked (by
    /// name, e.g. KFHumanPawn for KFZombieZoneVolume). None = blocks all.
    pub blocked_classes: Option<Vec<String>>,
    /// bBlockZeroExtentTraces (BlockingVolume default false: bullets pass).
    pub blocks_traces: bool,
}

#[derive(Debug, Clone)]
pub struct PlayerStart {
    pub location: [f32; 3],
    pub rotation: Rotator,
}

#[derive(Debug, Default)]
pub struct LevelContents {
    /// The `Model` export holding the level geometry.
    pub bsp_model: Option<usize>,
    /// How many Model exports were not owned by a brush actor. Normally 1.
    pub bsp_candidates: usize,
    pub mesh_actors: Vec<MeshActor>,
    pub player_starts: Vec<PlayerStart>,
    /// Actors with a StaticMesh that were not drawn, by class, with the reason.
    pub skipped: BTreeMap<String, usize>,
    /// Objects whose properties failed to read.
    pub unreadable: usize,
    /// Brush volumes that block the player (only with class defaults).
    pub blocking_brushes: Vec<BlockingBrush>,
    /// Locations of PathNodes (navigation points placed above the floor).
    pub path_nodes: Vec<[f32; 3]>,
    /// KFUseTriggers (only with class defaults).
    pub use_triggers: Vec<UseTriggerInfo>,
}

fn vector(props: &PropertyList, pkg: &Package, name: &str, default: [f32; 3]) -> [f32; 3] {
    match props.get(pkg, name) {
        Some(Value::Vector(v)) => *v,
        _ => default,
    }
}

/// Decodes an array of object references (e.g. `Skins`): each element is a
/// compact index.
fn object_array(value: Option<&Value>) -> Vec<ObjectRef> {
    let Some(Value::Array { count, raw }) = value else {
        return Vec::new();
    };
    let mut r = crate::reader::Reader::new(raw);
    (0..*count)
        .map_while(|_| r.compact_index().ok().map(ObjectRef::from_raw))
        .collect()
}

/// An actor's effective values (own saved property, else the class
/// default), with names and strings read from the right package.
struct Effective<'x, 'd> {
    lp: &'x Rc<LoadedPackage>,
    d: &'x ClassDefaults<'d>,
    export: usize,
    props: &'x PropertyList,
}

impl Effective<'_, '_> {
    fn value(&self, name: &str) -> Option<(Value, Rc<LoadedPackage>)> {
        if let Some(v) = self.props.get(&self.lp.pkg, name) {
            return Some((v.clone(), self.lp.clone()));
        }
        let class = self.d.class_of(self.lp, self.export)?;
        self.d.get(&class, name)
    }
    fn float(&self, name: &str, default: f32) -> f32 {
        match self.value(name) {
            Some((Value::Float(f), _)) => f,
            _ => default,
        }
    }
    fn int(&self, name: &str, default: i32) -> i32 {
        match self.value(name) {
            Some((Value::Int(n), _)) => n,
            _ => default,
        }
    }
    fn byte(&self, name: &str, default: u8) -> u8 {
        match self.value(name) {
            Some((Value::Byte(b), _)) => b,
            _ => default,
        }
    }
    fn bool(&self, name: &str) -> bool {
        matches!(self.value(name), Some((Value::Bool(true), _)))
    }
    fn name(&self, name: &str) -> String {
        match self.value(name) {
            Some((Value::Name(n), from)) => from.pkg.name(n).to_string(),
            _ => String::new(),
        }
    }
    fn string(&self, name: &str) -> String {
        match self.value(name) {
            Some((Value::Str(s), _)) => s,
            _ => String::new(),
        }
    }
    fn rotator(&self, name: &str) -> Rotator {
        match self.value(name) {
            Some((Value::Rotator(r), _)) => r,
            _ => Rotator::default(),
        }
    }
}

/// True if the class draws a static mesh by default. Class default values are
/// not decoded yet, so this is a list of classes known to do so.
fn draws_static_mesh_by_default(class: &str) -> bool {
    class == "StaticMeshActor" || class.ends_with("Mover")
}

/// Reads a level without class defaults: drawing decisions fall back to a
/// list of classes known to draw static meshes, and nothing is marked as
/// blocking. Enough for diagnostics that only need the BSP.
pub fn read_level(pkg: &Package) -> LevelContents {
    read_level_impl(pkg, None)
}

/// Reads a level using class defaults for DrawType, bHidden and collision.
pub fn read_level_with(lp: &Rc<LoadedPackage>, defaults: &ClassDefaults) -> LevelContents {
    read_level_impl(&lp.pkg, Some((lp, defaults)))
}

fn read_level_impl(pkg: &Package, defaults: Option<(&Rc<LoadedPackage>, &ClassDefaults)>) -> LevelContents {
    let mut out = LevelContents::default();
    let mut brush_models = HashSet::new();
    let mut models = Vec::new();

    for i in 0..pkg.exports.len() {
        let class = pkg.export_class_name(i);
        if class == "Model" {
            models.push(i);
            continue;
        }
        if !crate::properties::has_tagged_properties(class) {
            continue;
        }
        let Ok(props) = read_export_properties(pkg, i) else {
            out.unreadable += 1;
            continue;
        };
        if let Some(Value::Object(ObjectRef::Export(m))) = props.get(pkg, "Brush") {
            brush_models.insert(*m);
            // CSG brushes ("Brush") are already part of the BSP; other brush
            // actors (volumes) block only if their collision flags say so.
            if class != "Brush"
                && let Some((lp, d)) = defaults
                && d.actor_bool(lp, i, &props, "bCollideActors")
                && d.actor_bool(lp, i, &props, "bBlockActors")
                && d.actor_bool(lp, i, &props, "bBlockNonZeroExtentTraces")
            {
                out.blocking_brushes.push(BlockingBrush {
                    export: i,
                    class: class.to_string(),
                    model: ObjectRef::Export(*m),
                    location: vector(&props, pkg, "Location", [0.0; 3]),
                    rotation: match props.get(pkg, "Rotation") {
                        Some(Value::Rotator(r)) => *r,
                        _ => Rotator::default(),
                    },
                    pre_pivot: vector(&props, pkg, "PrePivot", [0.0; 3]),
                    blocks_traces: d.actor_bool(lp, i, &props, "bBlockZeroExtentTraces"),
                    blocked_classes: d.actor_bool(lp, i, &props, "bClassBlocker").then(|| {
                        // The map's own list, else the class default (whose
                        // references belong to the class's package).
                        match props.get(pkg, "BlockedClasses") {
                            Some(v) => object_array(Some(v)).into_iter().map(|r| pkg.object_name(r).to_string()).collect(),
                            None => d
                                .class_of(lp, i)
                                .and_then(|c| d.get(&c, "BlockedClasses"))
                                .map(|(v, from)| {
                                    object_array(Some(&v)).into_iter().map(|r| from.pkg.object_name(r).to_string()).collect()
                                })
                                .unwrap_or_default(),
                        }
                    }),
                });
            }
        }
        if class == "KFUseTrigger"
            && let Some((lp, d)) = defaults
        {
            let v = Effective { lp, d, export: i, props: &props };
            out.use_triggers.push(UseTriggerInfo {
                name: pkg.object_name(ObjectRef::Export(i)).to_string(),
                event: v.name("Event"),
                location: vector(&props, pkg, "Location", [0.0; 3]),
                rotation: v.rotator("Rotation"),
                radius: v.float("CollisionRadius", 0.0),
                height: v.float("CollisionHeight", 0.0),
                refire_delay: v.int("ReFireDelay", 0),
                max_weld_strength: v.float("MaxWeldStrength", 0.0),
                combat_seal_reduction: v.float("CombatSealReduction", 1.0),
                directional_open: v.bool("bDirectionalOpen"),
                message: v.string("Message"),
                always_show_message: v.bool("bAlwaysShowMessage"),
            });
        }
        if class == "PathNode" {
            out.path_nodes.push(vector(&props, pkg, "Location", [0.0; 3]));
        }
        if class.contains("PlayerStart") {
            out.player_starts.push(PlayerStart {
                location: vector(&props, pkg, "Location", [0.0; 3]),
                rotation: match props.get(pkg, "Rotation") {
                    Some(Value::Rotator(r)) => *r,
                    _ => Rotator::default(),
                },
            });
        }
        let Some(Value::Object(mesh)) = props.get(pkg, "StaticMesh") else {
            continue;
        };
        if *mesh == ObjectRef::Null {
            continue;
        }
        let effective = |name: &str| match defaults {
            Some((lp, d)) => d.actor_value(lp, i, &props, name),
            None => props.get(pkg, name).cloned(),
        };
        let draw_type = match effective("DrawType") {
            Some(Value::Byte(b)) => Some(b),
            _ => None,
        };
        let hidden = matches!(effective("bHidden"), Some(Value::Bool(true)));
        let reason = if hidden {
            Some("hidden")
        } else {
            match draw_type {
                Some(DT_STATIC_MESH) => None,
                Some(_) => Some("other_draw_type"),
                None if defaults.is_none() && draws_static_mesh_by_default(class) => None,
                None => Some("no_draw_type"),
            }
        };
        let blocks_player = defaults.is_some()
            && [
                "bCollideActors",
                "bBlockActors",
                "bBlockNonZeroExtentTraces",
            ]
            .iter()
            .all(|f| matches!(effective(f), Some(Value::Bool(true))));
        if let Some(reason) = reason {
            *out.skipped.entry(format!("{class}:{reason}")).or_default() += 1;
            continue;
        }
        let draw_scale = match props.get(pkg, "DrawScale") {
            Some(Value::Float(f)) => *f,
            _ => 1.0,
        };
        let s3 = vector(&props, pkg, "DrawScale3D", [1.0; 3]);
        let glass = match defaults {
            Some((lp, d)) if d.class_of(lp, i).is_some_and(|c| d.is_a(&c, "KFGlassMover")) => {
                let v = Effective { lp, d, export: i, props: &props };
                Some(GlassInfo {
                    name: pkg.object_name(ObjectRef::Export(i)).to_string(),
                    tag: v.name("Tag"),
                    health: v.int("Health", 50),
                })
            }
            _ => None,
        };
        let door = match defaults {
            Some((lp, d)) if d.class_of(lp, i).is_some_and(|c| d.is_a(&c, "KFDoorMover")) => {
                let v = Effective { lp, d, export: i, props: &props };
                Some(DoorInfo {
                    name: pkg.object_name(ObjectRef::Export(i)).to_string(),
                    tag: v.name("Tag"),
                    base_pos: vector(&props, pkg, "BasePos", [0.0; 3]),
                    base_rot: v.rotator("BaseRot"),
                    key_pos: (0..24)
                        .map(|k| match props.get_at(pkg, "KeyPos", k) {
                            Some(Value::Vector(p)) => *p,
                            _ => [0.0; 3],
                        })
                        .collect(),
                    key_rot: (0..24)
                        .map(|k| match props.get_at(pkg, "KeyRot", k) {
                            Some(Value::Rotator(r)) => *r,
                            _ => Rotator::default(),
                        })
                        .collect(),
                    key_num: v.byte("KeyNum", 0),
                    num_keys: v.byte("NumKeys", 2),
                    move_time: v.float("MoveTime", 1.0),
                    delay_time: v.float("DelayTime", 0.0),
                    glide_type: v.byte("MoverGlideType", 1),
                    initial_state: v.name("InitialState"),
                    start_sealed: v.bool("bStartSealed"),
                    start_sealed_weld_prc: v.float("StartSealedWeldPrc", 0.0),
                    disallow_weld: v.bool("bDisallowWeld"),
                    key_locked: v.bool("bKeyLocked"),
                    no_seal: v.bool("bNoSeal"),
                    small_arms_damage: v.bool("bSmallArmsDamage"),
                    zombies_ignore: v.bool("bZombiesIgnore"),
                    block_damaging_of_weld: v.bool("bBlockDamagingOfWeld"),
                    surface_type: v.byte("SurfaceType", 0),
                    is_leader: v.bool("bIsLeader"),
                    return_group: v.name("ReturnGroup"),
                    blocks_traces: v.bool("bBlockZeroExtentTraces"),
                })
            }
            _ => None,
        };
        out.mesh_actors.push(MeshActor {
            export: i,
            class: class.to_string(),
            mesh: *mesh,
            location: vector(&props, pkg, "Location", [0.0; 3]),
            rotation: match props.get(pkg, "Rotation") {
                Some(Value::Rotator(r)) => *r,
                _ => Rotator::default(),
            },
            scale: [s3[0] * draw_scale, s3[1] * draw_scale, s3[2] * draw_scale],
            pre_pivot: vector(&props, pkg, "PrePivot", [0.0; 3]),
            skins: object_array(props.get(pkg, "Skins")),
            blocks_player,
            door,
            glass,
            unlit: matches!(effective("bUnlit"), Some(Value::Bool(true))),
        });
    }

    let candidates: Vec<usize> = models.into_iter().filter(|m| !brush_models.contains(m)).collect();
    out.bsp_candidates = candidates.len();
    out.bsp_model = candidates.into_iter().max_by_key(|&m| pkg.exports[m].serial_size);
    out
}
