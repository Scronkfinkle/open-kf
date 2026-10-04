//! Static (non-animated) meshes.
//!
//! Layout after the property list (Unreal Engine 2, as found in KF files):
//! - bounding box (min, max, valid byte) and bounding sphere (centre, radius)
//! - sections: one per material, each a range of triangles in the index list
//! - a second bounding box
//! - vertex stream: position + normal per vertex, then a revision number
//! - colour stream and alpha stream: one colour per vertex (often empty)
//! - UV streams: one or more sets of texture coordinates
//! - index stream: triangle list, 3 vertex indices per triangle
//! - a second index stream (wireframe lines, unused here)
//!
//! Collision data and other trailing data are not read.

use crate::package::{ObjectRef, Package};
use crate::properties::{Value, read_export_properties};
use crate::reader::{ReadError, ReadErrorKind, Reader, Result};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoundingBox {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

#[derive(Debug, Clone)]
pub struct Section {
    /// Offset into `indices` of the first index of this section.
    pub first_index: usize,
    pub num_triangles: usize,
}

#[derive(Debug, Clone)]
pub struct StaticMesh {
    pub bounds: BoundingBox,
    pub sections: Vec<Section>,
    /// Material per section (same order), from the `Materials` property.
    pub materials: Vec<ObjectRef>,
    /// Per section: whether it collides (the material entry's EnableCollision;
    /// true when the entry or flag is missing).
    pub section_collides: Vec<bool>,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// UV sets; the first is the main texture coordinates.
    pub uvs: Vec<Vec<[f32; 2]>>,
    pub indices: Vec<u16>,
}

fn invalid(r: &Reader, msg: String) -> ReadError {
    r.error(ReadErrorKind::Invalid(msg))
}

fn vec3(r: &mut Reader) -> Result<[f32; 3]> {
    Ok([r.f32()?, r.f32()?, r.f32()?])
}

fn read_box(r: &mut Reader) -> Result<BoundingBox> {
    let min = vec3(r)?;
    let max = vec3(r)?;
    let _valid = r.u8()?;
    Ok(BoundingBox { min, max })
}

/// Reads an array length and checks the elements could fit in what is left.
fn array_len(r: &mut Reader, element_size: usize, what: &str) -> Result<usize> {
    let n = r.compact_index()?;
    if n < 0 || (n as usize).saturating_mul(element_size) > r.remaining() {
        return Err(invalid(r, format!("{what} count {n} does not fit")));
    }
    Ok(n as usize)
}

pub fn read_static_mesh(pkg: &Package, index: usize) -> Result<StaticMesh> {
    let props = read_export_properties(pkg, index)?;
    let (materials, material_collides): (Vec<ObjectRef>, Vec<bool>) = match props.get(pkg, "Materials") {
        Some(Value::StructArray(items)) => items
            .iter()
            .map(|item| {
                let rf = match item.get(pkg, "Material") {
                    Some(Value::Object(rf)) => *rf,
                    _ => ObjectRef::Null,
                };
                let collides = !matches!(item.get(pkg, "EnableCollision"), Some(Value::Bool(false)));
                (rf, collides)
            })
            .unzip(),
        _ => (Vec::new(), Vec::new()),
    };

    let mut r = Reader::new(pkg.export_data(index));
    r.seek(props.end)?;

    let bounds = read_box(&mut r)?;
    let _sphere = r.bytes(16)?;

    let section_count = array_len(&mut r, 14, "section")?;
    let mut sections = Vec::with_capacity(section_count);
    for _ in 0..section_count {
        let _is_strip = r.i32()?;
        let first_index = r.u16()? as usize;
        let _first_vertex = r.u16()?;
        let _last_vertex = r.u16()?;
        let _unknown = r.u16()?;
        let num_triangles = r.u16()? as usize;
        sections.push(Section {
            first_index,
            num_triangles,
        });
    }
    let _bounds2 = read_box(&mut r)?;

    let vertex_count = array_len(&mut r, 24, "vertex")?;
    let mut positions = Vec::with_capacity(vertex_count);
    let mut normals = Vec::with_capacity(vertex_count);
    for _ in 0..vertex_count {
        positions.push(vec3(&mut r)?);
        normals.push(vec3(&mut r)?);
    }
    let _revision = r.i32()?;

    for what in ["colour", "alpha"] {
        let n = array_len(&mut r, 4, what)?;
        r.bytes(n * 4)?;
        let _revision = r.i32()?;
    }

    let uv_sets = array_len(&mut r, 1, "UV stream")?;
    if uv_sets > 8 {
        return Err(invalid(&r, format!("implausible UV stream count {uv_sets}")));
    }
    let mut uvs = Vec::with_capacity(uv_sets);
    for _ in 0..uv_sets {
        let n = array_len(&mut r, 8, "UV")?;
        let mut set = Vec::with_capacity(n);
        for _ in 0..n {
            set.push([r.f32()?, r.f32()?]);
        }
        let _coordinate_index = r.i32()?;
        let _revision = r.i32()?;
        uvs.push(set);
    }

    let index_count = array_len(&mut r, 2, "index")?;
    let mut indices = Vec::with_capacity(index_count);
    for _ in 0..index_count {
        indices.push(r.u16()?);
    }
    let _revision = r.i32()?;

    let section_collides = (0..sections.len())
        .map(|i| material_collides.get(i).copied().unwrap_or(true))
        .collect();
    let mesh = StaticMesh {
        bounds,
        sections,
        materials,
        section_collides,
        positions,
        normals,
        uvs,
        indices,
    };
    mesh.validate().map_err(|msg| invalid(&r, msg))?;
    Ok(mesh)
}

impl StaticMesh {
    /// Consistency checks that would catch a wrong guess about the layout.
    pub fn validate(&self) -> std::result::Result<(), String> {
        let nv = self.positions.len();
        if let Some(&bad) = self.indices.iter().find(|&&i| i as usize >= nv) {
            return Err(format!("index {bad} >= vertex count {nv}"));
        }
        for (si, s) in self.sections.iter().enumerate() {
            if s.first_index + s.num_triangles * 3 > self.indices.len() {
                return Err(format!(
                    "section {si} (first index {}, {} triangles) exceeds {} indices",
                    s.first_index,
                    s.num_triangles,
                    self.indices.len()
                ));
            }
        }
        for (ui, set) in self.uvs.iter().enumerate() {
            if set.len() != nv {
                return Err(format!("UV set {ui} has {} entries for {nv} vertices", set.len()));
            }
        }
        Ok(())
    }

    /// Number of vertices outside the stored bounding box (allowing a small margin).
    pub fn vertices_outside_bounds(&self) -> usize {
        let margin = 1.0;
        self.positions
            .iter()
            .filter(|p| (0..3).any(|a| p[a] < self.bounds.min[a] - margin || p[a] > self.bounds.max[a] + margin))
            .count()
    }
}
