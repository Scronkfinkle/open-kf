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
                });
            }
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
        });
    }

    let candidates: Vec<usize> = models.into_iter().filter(|m| !brush_models.contains(m)).collect();
    out.bsp_candidates = candidates.len();
    out.bsp_model = candidates.into_iter().max_by_key(|&m| pkg.exports[m].serial_size);
    out
}
