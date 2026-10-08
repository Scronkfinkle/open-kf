//! Terrain (`TerrainInfo` actors): heightmap grid, layers, holes.
//!
//! Vertex (x, y) of the heightmap sits at (Unreal world space):
//!   Location + ((x - W/2) * Scale.X, (y - H/2) * Scale.Y, (h - 32768) * Scale.Z / 256)
//! This is the standard UE2 formula; `kfpkg terrain` checks it against the
//! heights of PathNodes and PlayerStarts placed on the ground.

use crate::package::{ObjectRef, Package};
use crate::package_set::{LoadedPackage, PackageSet};
use crate::properties::{PropertyList, Value, read_export_properties};
use crate::texture::{TextureFormat, g16_values, read_texture};

#[derive(Debug, Clone)]
pub struct TerrainLayer {
    pub texture: ObjectRef,
    pub alpha_map: ObjectRef,
    /// Rows XPlane, YPlane, ZPlane, WPlane, each (X, Y, Z, W). Texture
    /// coordinates are u = wx*XPlane.X + wy*YPlane.X + wz*ZPlane.X + WPlane.X,
    /// v likewise with the .Y components.
    pub matrix: [[f32; 4]; 4],
}

impl TerrainLayer {
    pub fn uv(&self, p: [f32; 3]) -> [f32; 2] {
        let m = &self.matrix;
        [
            p[0] * m[0][0] + p[1] * m[1][0] + p[2] * m[2][0] + m[3][0],
            p[0] * m[0][1] + p[1] * m[1][1] + p[2] * m[2][1] + m[3][1],
        ]
    }
}

#[derive(Debug, Clone)]
pub struct Terrain {
    pub export: usize,
    pub location: [f32; 3],
    pub scale: [f32; 3],
    pub width: usize,
    pub height: usize,
    pub heights: Vec<u16>,
    pub layers: Vec<TerrainLayer>,
    /// One bit per quad, indexed y * width + x. Empty means all visible.
    quad_visibility: Vec<u32>,
    /// One bit per quad: set means the quad is split along the other diagonal.
    edge_turn: Vec<u32>,
    pub inverted: bool,
    /// Zone the actor is in, from its Region property.
    pub zone_number: Option<u8>,
}

fn float_field(list: &PropertyList, pkg: &Package, name: &str) -> f32 {
    match list.get(pkg, name) {
        Some(Value::Float(f)) => *f,
        _ => 0.0,
    }
}

fn plane(list: &PropertyList, pkg: &Package, name: &str) -> [f32; 4] {
    match list.get(pkg, name) {
        Some(Value::TaggedStruct { props, .. }) => [
            float_field(props, pkg, "X"),
            float_field(props, pkg, "Y"),
            float_field(props, pkg, "Z"),
            float_field(props, pkg, "W"),
        ],
        _ => [0.0; 4],
    }
}

fn int_array(value: Option<&Value>) -> Vec<u32> {
    match value {
        Some(Value::Array { count, raw }) if raw.len() >= count * 4 => raw[..count * 4]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&c| u32::from_le_bytes(c))
            .collect(),
        _ => Vec::new(),
    }
}

/// Reads every TerrainInfo in a map. Errors are returned per terrain as text.
pub fn read_terrains(set: &PackageSet, lp: &std::rc::Rc<LoadedPackage>) -> Vec<Result<Terrain, String>> {
    let pkg = &lp.pkg;
    pkg.level_actor_exports()
        .filter(|&i| pkg.export_class_name(i) == "TerrainInfo")
        .map(|i| read_terrain(set, lp, i))
        .collect()
}

fn read_terrain(set: &PackageSet, lp: &std::rc::Rc<LoadedPackage>, export: usize) -> Result<Terrain, String> {
    let pkg = &lp.pkg;
    let props = read_export_properties(pkg, export).map_err(|e| e.to_string())?;
    let vector = |name: &str, default: [f32; 3]| match props.get(pkg, name) {
        Some(Value::Vector(v)) => *v,
        _ => default,
    };
    let map_ref = match props.get(pkg, "TerrainMap") {
        Some(Value::Object(rf)) => *rf,
        _ => return Err("no TerrainMap".into()),
    };
    let tex_handle = set.resolve(lp, map_ref).ok_or("TerrainMap not found")?;
    let tex = read_texture(&tex_handle.package.pkg, tex_handle.export).map_err(|e| e.to_string())?;
    if tex.format != TextureFormat::G16 {
        return Err(format!("TerrainMap format {:?}, expected G16", tex.format));
    }
    let mip = tex.mips.first().ok_or("TerrainMap has no data")?;
    let heights = g16_values(mip).ok_or("TerrainMap data too short")?;

    let mut layers = Vec::new();
    for p in props.props.iter().filter(|p| pkg.name(p.name).eq_ignore_ascii_case("Layers")) {
        let Value::TaggedStruct { props: l, .. } = &p.value else {
            continue;
        };
        let obj = |name: &str| match l.get(pkg, name) {
            Some(Value::Object(rf)) => *rf,
            _ => ObjectRef::Null,
        };
        let matrix = match l.get(pkg, "TerrainMatrix") {
            Some(Value::TaggedStruct { props: m, .. }) => {
                [plane(m, pkg, "XPlane"), plane(m, pkg, "YPlane"), plane(m, pkg, "ZPlane"), plane(m, pkg, "WPlane")]
            }
            _ => [[0.0; 4]; 4],
        };
        if obj("Texture") == ObjectRef::Null {
            continue;
        }
        layers.push((p.array_index, TerrainLayer {
            texture: obj("Texture"),
            alpha_map: obj("AlphaMap"),
            matrix,
        }));
    }
    layers.sort_by_key(|(i, _)| *i);

    let zone_number = match props.get(pkg, "Region") {
        Some(Value::TaggedStruct { props: r, .. }) => match r.get(pkg, "ZoneNumber") {
            Some(Value::Byte(b)) => Some(*b),
            _ => None,
        },
        _ => None,
    };

    Ok(Terrain {
        export,
        location: vector("Location", [0.0; 3]),
        scale: vector("TerrainScale", [64.0, 64.0, 64.0]),
        width: mip.width,
        height: mip.height,
        heights,
        layers: layers.into_iter().map(|(_, l)| l).collect(),
        quad_visibility: int_array(props.get(pkg, "QuadVisibilityBitmap")),
        edge_turn: int_array(props.get(pkg, "EdgeTurnBitmap")),
        inverted: matches!(props.get(pkg, "Inverted"), Some(Value::Bool(true))),
        zone_number,
    })
}

fn bit(bits: &[u32], i: usize) -> Option<bool> {
    bits.get(i / 32).map(|w| w & (1 << (i % 32)) != 0)
}

impl Terrain {
    /// World position (Unreal space) of heightmap vertex (x, y).
    pub fn vertex(&self, x: usize, y: usize) -> [f32; 3] {
        let h = self.heights[y * self.width + x] as f32;
        [
            self.location[0] + (x as f32 - self.width as f32 / 2.0) * self.scale[0],
            self.location[1] + (y as f32 - self.height as f32 / 2.0) * self.scale[1],
            self.location[2] + (h - 32768.0) * self.scale[2] / 256.0,
        ]
    }

    /// Whether quad (x, y) is drawn (not a hole).
    pub fn quad_visible(&self, x: usize, y: usize) -> bool {
        bit(&self.quad_visibility, y * self.width + x).unwrap_or(true)
    }

    /// Whether quad (x, y) is split along the (x+1,y)-(x,y+1) diagonal
    /// instead of the default (x,y)-(x+1,y+1).
    pub fn edge_turned(&self, x: usize, y: usize) -> bool {
        bit(&self.edge_turn, y * self.width + x).unwrap_or(false)
    }

    /// Fraction of quads marked visible, for logging.
    pub fn visible_fraction(&self) -> f32 {
        let quads = (self.width - 1) * (self.height - 1);
        let visible = (0..self.height - 1)
            .flat_map(|y| (0..self.width - 1).map(move |x| (x, y)))
            .filter(|&(x, y)| self.quad_visible(x, y))
            .count();
        visible as f32 / quads.max(1) as f32
    }

    /// Heightmap quad containing world (wx, wy), if inside the grid (holes included).
    pub fn quad_at(&self, wx: f32, wy: f32) -> Option<(usize, usize)> {
        let fx = (wx - self.location[0]) / self.scale[0] + self.width as f32 / 2.0;
        let fy = (wy - self.location[1]) / self.scale[1] + self.height as f32 / 2.0;
        if fx < 0.0 || fy < 0.0 || fx >= (self.width - 1) as f32 || fy >= (self.height - 1) as f32 {
            return None;
        }
        Some((fx as usize, fy as usize))
    }

    /// Ground height (Unreal Z) at world (wx, wy), bilinear between the four
    /// surrounding vertices. None outside the terrain or over a hole.
    pub fn height_at(&self, wx: f32, wy: f32) -> Option<f32> {
        let fx = (wx - self.location[0]) / self.scale[0] + self.width as f32 / 2.0;
        let fy = (wy - self.location[1]) / self.scale[1] + self.height as f32 / 2.0;
        if fx < 0.0 || fy < 0.0 || fx >= (self.width - 1) as f32 || fy >= (self.height - 1) as f32 {
            return None;
        }
        let (x, y) = (fx as usize, fy as usize);
        if !self.quad_visible(x, y) {
            return None;
        }
        let (tx, ty) = (fx - x as f32, fy - y as f32);
        let z = |x, y| self.vertex(x, y)[2];
        let top = z(x, y) * (1.0 - tx) + z(x + 1, y) * tx;
        let bottom = z(x, y + 1) * (1.0 - tx) + z(x + 1, y + 1) * tx;
        Some(top * (1.0 - ty) + bottom * ty)
    }
}
