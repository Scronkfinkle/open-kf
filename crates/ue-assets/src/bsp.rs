//! BSP level geometry (`Model` objects): the brush-built walls, floors and rooms.
//!
//! Layout after the property list, as found in KF maps (worked out from the data
//! and checked by the consistency tests in `validate`):
//! - bounding box (25 bytes) and sphere (16)
//! - `Vectors`: normals and texture axes, 12 bytes each
//! - `Points`: vertex positions, 12 bytes each
//! - `Nodes`: one convex polygon each, plus BSP tree links (see `read_node`)
//! - `Surfs`: material, flags and texture axes shared by coplanar polygons
//! - `Verts`: (point index, side index) pairs, a pool the nodes point into
//! - shared side count, zone count, zones; then more data not read here
//!   (polys reference, bounds, leaves, lightmaps, render sections)
//!
//! The rest (`read_lighting`, from `Model::tail`; worked out from the
//! KF-WestLondon data, the walk ends exactly at the Model's last byte):
//! - Bounds (25-byte boxes), LeafHulls (ints), Leaves (3 compact + 8 bytes),
//!   Lights (compact refs), RootOutside and Linked (ints)
//! - Sections: vertices (40 bytes: position, texture UV, lightmap UV,
//!   normal), an int, material, node count, PolyFlags, lightmap texture
//! - LightMaps: 7 compact, a matrix, 3 vectors, lights (ref, shadow bits,
//!   7 ints), a compact and an int (skipped: the vertices carry the UVs)
//! - LightMapTextures: compact + lightmap list, 8 bytes, an int, two mips
//!   (int file offset, compact length, data), format, width, height, an int

use crate::package::{ObjectRef, Package};
use crate::properties::read_export_properties;
use crate::reader::{ReadError, ReadErrorKind, Reader, Result};

/// Surface flags (`EPolyFlags`).
pub mod poly_flags {
    pub const INVISIBLE: u32 = 0x0000_0001;
    pub const MASKED: u32 = 0x0000_0002;
    pub const TRANSLUCENT: u32 = 0x0000_0004;
    pub const MODULATED: u32 = 0x0000_0040;
    /// Shows the skybox through this surface.
    pub const FAKE_BACKDROP: u32 = 0x0000_0080;
    pub const TWO_SIDED: u32 = 0x0000_0100;
    pub const UNLIT: u32 = 0x0040_0000;
    /// Zone portal: never drawn.
    pub const PORTAL: u32 = 0x0400_0000;
}

#[derive(Debug, Clone)]
pub struct BspNode {
    pub plane: [f32; 4],
    pub vert_pool: usize,
    pub surf: usize,
    pub num_verts: usize,
    /// Zone behind ([0]) and in front ([1]) of the plane.
    pub zone: [u8; 2],
    /// Child nodes behind and in front (iBack, iFront; -1 = none).
    pub back: i32,
    pub front: i32,
    /// Render section and first vertex in it (see `BspLighting`), -1 if
    /// none.
    pub section: i32,
    pub first_vertex: i32,
    /// LightMaps entry, -1 if none.
    pub lightmap: i32,
}

#[derive(Debug, Clone)]
pub struct BspSurf {
    pub material: ObjectRef,
    pub flags: u32,
    pub base: usize,
    pub normal: usize,
    pub texture_u: usize,
    pub texture_v: usize,
    pub actor: ObjectRef,
}

#[derive(Debug, Clone)]
pub struct Model {
    pub vectors: Vec<[f32; 3]>,
    pub points: Vec<[f32; 3]>,
    pub nodes: Vec<BspNode>,
    pub surfs: Vec<BspSurf>,
    /// Point index of each entry in the vertex pool. Entries not used by any
    /// node may be stale and point past `points`; they are never read.
    pub vert_points: Vec<usize>,
    pub num_zones: usize,
    /// ZoneInfo actor of each zone (Null for zone 0, the default zone).
    pub zone_actors: Vec<ObjectRef>,
    /// The Polys object holding the source polygons (for brush models, the
    /// brush's shape in its own local space).
    pub polys: ObjectRef,
    /// Byte offset where the unread rest of the Model starts (leaves,
    /// lights, lightmaps, render sections).
    pub tail: usize,
}

/// One polygon of a `Polys` object.
#[derive(Debug, Clone)]
pub struct Poly {
    pub normal: [f32; 3],
    pub vertices: Vec<[f32; 3]>,
    pub flags: u32,
}

/// Reads a `Polys` export: property list, count, capacity, then each polygon:
/// vertex count, base, normal, texture U and V, vertices, flags, actor, item
/// name, material, link, brush polygon index, lightmap scale. The reader
/// checks it ends exactly at the end of the data.
pub fn read_polys(pkg: &Package, export: usize) -> Result<Vec<Poly>> {
    let props = read_export_properties(pkg, export)?;
    let mut r = Reader::new(pkg.export_data(export));
    r.seek(props.end)?;
    let num = r.i32()?;
    let _max = r.i32()?;
    if !(0..=100_000).contains(&num) {
        return Err(invalid(&r, format!("implausible polygon count {num}")));
    }
    let mut out = Vec::with_capacity(num as usize);
    for _ in 0..num {
        let nv = index(&mut r)?;
        if !(0..=64).contains(&nv) {
            return Err(invalid(&r, format!("implausible polygon vertex count {nv}")));
        }
        let _base = vec3(&mut r)?;
        let normal = vec3(&mut r)?;
        let _tex_u = vec3(&mut r)?;
        let _tex_v = vec3(&mut r)?;
        let vertices = (0..nv).map(|_| vec3(&mut r)).collect::<Result<Vec<_>>>()?;
        let flags = r.u32()?;
        let _actor = index(&mut r)?;
        let _item_name = index(&mut r)?;
        let _material = index(&mut r)?;
        let _link = index(&mut r)?;
        let _brush_poly = index(&mut r)?;
        let _lightmap_scale = r.f32()?;
        out.push(Poly { normal, vertices, flags });
    }
    if r.remaining() != 0 {
        return Err(invalid(&r, format!("{} bytes left after polygons", r.remaining())));
    }
    Ok(out)
}

fn invalid(r: &Reader, msg: String) -> ReadError {
    r.error(ReadErrorKind::Invalid(msg))
}

fn vec3(r: &mut Reader) -> Result<[f32; 3]> {
    Ok([r.f32()?, r.f32()?, r.f32()?])
}

fn count(r: &mut Reader, min_element: usize, what: &str) -> Result<usize> {
    let n = r.compact_index()?;
    if n < 0 || (n as usize).saturating_mul(min_element) > r.remaining() {
        return Err(invalid(r, format!("{what} count {n} does not fit")));
    }
    Ok(n as usize)
}

fn index(r: &mut Reader) -> Result<i32> {
    r.compact_index()
}

fn read_node(r: &mut Reader) -> Result<BspNode> {
    let plane = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
    let _zone_mask = r.bytes(8)?;
    let _node_flags = r.u8()?;
    let vert_pool = index(r)?;
    let surf = index(r)?;
    let back = index(r)?;
    let front = index(r)?;
    let _coplanar = index(r)?;
    let _collision_bound = index(r)?;
    let _render_bound = index(r)?;
    let _exclusive_sphere = r.bytes(16)?;
    let zone = [r.u8()?, r.u8()?];
    let num_verts = r.u8()? as usize;
    let _leaf = [r.i32()?, r.i32()?];
    let section = r.i32()?;
    let first_vertex = r.i32()?;
    let lightmap = r.i32()?;
    if vert_pool < 0 || surf < 0 {
        return Err(invalid(r, format!("node has negative vertex pool {vert_pool} or surface {surf}")));
    }
    Ok(BspNode {
        plane,
        vert_pool: vert_pool as usize,
        surf: surf as usize,
        num_verts,
        zone,
        back,
        front,
        section,
        first_vertex,
        lightmap,
    })
}

fn read_surf(r: &mut Reader) -> Result<BspSurf> {
    let material = ObjectRef::from_raw(index(r)?);
    let flags = r.u32()?;
    let mut idx = [0i32; 6];
    for v in idx.iter_mut() {
        *v = index(r)?;
    }
    let _plane = r.bytes(16)?;
    let _lightmap_scale = r.f32()?;
    let [base, normal, texture_u, texture_v, _brush_poly, actor] = idx;
    if base < 0 || normal < 0 || texture_u < 0 || texture_v < 0 {
        return Err(invalid(r, format!("surface has negative index in {idx:?}")));
    }
    Ok(BspSurf {
        material,
        flags,
        base: base as usize,
        normal: normal as usize,
        texture_u: texture_u as usize,
        texture_v: texture_v as usize,
        actor: ObjectRef::from_raw(actor),
    })
}

pub fn read_model(pkg: &Package, export: usize) -> Result<Model> {
    let props = read_export_properties(pkg, export)?;
    let mut r = Reader::new(pkg.export_data(export));
    r.seek(props.end)?;
    let _bounds = r.bytes(25 + 16)?;

    let n = count(&mut r, 12, "vector")?;
    let vectors = (0..n).map(|_| vec3(&mut r)).collect::<Result<Vec<_>>>()?;
    let n = count(&mut r, 12, "point")?;
    let points = (0..n).map(|_| vec3(&mut r)).collect::<Result<Vec<_>>>()?;
    let n = count(&mut r, 60, "node")?;
    let nodes = (0..n).map(|_| read_node(&mut r)).collect::<Result<Vec<_>>>()?;
    let n = count(&mut r, 30, "surf")?;
    let surfs = (0..n).map(|_| read_surf(&mut r)).collect::<Result<Vec<_>>>()?;
    let n = count(&mut r, 2, "vert")?;
    let mut vert_points = Vec::with_capacity(n);
    for _ in 0..n {
        let p = index(&mut r)?;
        let _side = index(&mut r)?;
        vert_points.push(p.max(0) as usize);
    }
    let _shared_sides = r.i32()?;
    let num_zones = r.i32()?;
    if !(0..=64).contains(&num_zones) {
        return Err(invalid(&r, format!("implausible zone count {num_zones}")));
    }

    // Zones: actor reference, two 64-bit masks (connectivity, visibility), last render time.
    let mut zone_actors = Vec::with_capacity(num_zones as usize);
    for _ in 0..num_zones {
        zone_actors.push(ObjectRef::from_raw(index(&mut r)?));
        let _masks = r.bytes(16)?;
        let _last_render_time = r.f32()?;
    }
    let polys = ObjectRef::from_raw(index(&mut r)?);

    let model = Model {
        vectors,
        points,
        nodes,
        surfs,
        vert_points,
        num_zones: num_zones as usize,
        zone_actors,
        polys,
        tail: r.pos(),
    };
    model.validate().map_err(|m| invalid(&r, m))?;
    Ok(model)
}

impl Model {
    /// Checks every index the nodes and surfaces use. A wrong guess about the
    /// layout makes these fail immediately.
    pub fn validate(&self) -> std::result::Result<(), String> {
        for (i, n) in self.nodes.iter().enumerate() {
            if n.surf >= self.surfs.len() {
                return Err(format!("node {i} surface {} >= {}", n.surf, self.surfs.len()));
            }
            if n.vert_pool + n.num_verts > self.vert_points.len() {
                return Err(format!("node {i} vertices run past the pool"));
            }
            for k in 0..n.num_verts {
                let p = self.vert_points[n.vert_pool + k];
                if p >= self.points.len() {
                    return Err(format!("node {i} uses point {p} >= {}", self.points.len()));
                }
            }
        }
        for (i, n) in self.nodes.iter().enumerate() {
            if n.zone.iter().any(|&z| z as usize >= self.num_zones.max(1)) {
                return Err(format!("node {i} zone {:?} >= zone count {}", n.zone, self.num_zones));
            }
        }
        for (i, s) in self.surfs.iter().enumerate() {
            let nv = self.vectors.len();
            if s.base >= self.points.len() || s.normal >= nv || s.texture_u >= nv || s.texture_v >= nv {
                return Err(format!("surface {i} index out of range"));
            }
        }
        Ok(())
    }

    /// The zone a point is in (UModel::PointRegion, as I remember it: walk
    /// the tree, front child when the point is in front of the plane, else
    /// the back; at a missing child, the node's zone on that side).
    /// 0 when the model has no nodes.
    pub fn point_zone(&self, p: [f32; 3]) -> usize {
        let mut i = 0usize;
        for _ in 0..self.nodes.len().max(1) {
            let Some(n) = self.nodes.get(i) else { return 0 };
            let d = n.plane[0] * p[0] + n.plane[1] * p[1] + n.plane[2] * p[2] - n.plane[3];
            let front = d > 0.0;
            let next = if front { n.front } else { n.back };
            if next < 0 {
                return n.zone[usize::from(front)] as usize;
            }
            i = next as usize;
        }
        0
    }

    /// The polygon of node `i`, as world positions (Unreal space).
    pub fn node_polygon(&self, i: usize) -> impl Iterator<Item = [f32; 3]> + '_ {
        let n = &self.nodes[i];
        (0..n.num_verts).map(move |k| self.points[self.vert_points[n.vert_pool + k]])
    }

    /// Texture coordinates of a point on surface `s`, for a texture of the
    /// given size in texels: U = (P - Base) . TextureU / width (same for V).
    pub fn uv(&self, s: &BspSurf, p: [f32; 3], width: f32, height: f32) -> [f32; 2] {
        let base = self.points[s.base];
        let d = [p[0] - base[0], p[1] - base[1], p[2] - base[2]];
        let dot = |v: [f32; 3]| d[0] * v[0] + d[1] * v[1] + d[2] * v[2];
        [
            dot(self.vectors[s.texture_u]) / width,
            dot(self.vectors[s.texture_v]) / height,
        ]
    }
}

/// A BSP render vertex.
#[derive(Debug, Clone, Copy)]
pub struct BspVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    /// In the section's lightmap texture, 0..1.
    pub lightmap_uv: [f32; 2],
    pub normal: [f32; 3],
}

#[derive(Debug, Clone)]
pub struct BspSection {
    pub vertices: Vec<BspVertex>,
    pub material: ObjectRef,
    pub poly_flags: u32,
    /// Index into `BspLighting::textures`, -1 = no lightmap.
    pub lightmap_texture: i32,
}

/// A lightmap page. `format` is the Unreal texture format (7 = DXT3).
#[derive(Debug, Clone)]
pub struct LightmapTexture {
    pub format: u8,
    pub width: u32,
    pub height: u32,
    /// Mip data, largest first. Empty if the map saved the page empty.
    pub mips: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, Default)]
pub struct BspLighting {
    pub sections: Vec<BspSection>,
    pub textures: Vec<LightmapTexture>,
    /// Surface lightmaps (LightMaps entries) read past.
    pub lightmaps: usize,
}

/// Reads the render sections and lightmap textures after `model.tail`.
pub fn read_lighting(pkg: &Package, export: usize, model: &Model) -> Result<BspLighting> {
    let mut r = Reader::new(pkg.export_data(export));
    r.seek(model.tail)?;
    let n = count(&mut r, 25, "bounds")?;
    r.bytes(n * 25)?;
    let n = count(&mut r, 4, "leaf hull")?;
    r.bytes(n * 4)?;
    let n = count(&mut r, 11, "leaf")?;
    for _ in 0..n {
        index(&mut r)?;
        index(&mut r)?;
        index(&mut r)?;
        r.bytes(8)?;
    }
    let n = count(&mut r, 1, "light")?;
    for _ in 0..n {
        index(&mut r)?;
    }
    let _root_outside = r.i32()?;
    let _linked = r.i32()?;

    let n = count(&mut r, 17, "section")?;
    let mut sections = Vec::with_capacity(n);
    for _ in 0..n {
        let nv = count(&mut r, 40, "section vertex")?;
        let mut vertices = Vec::with_capacity(nv);
        for _ in 0..nv {
            let position = vec3(&mut r)?;
            let uv = [r.f32()?, r.f32()?];
            let lightmap_uv = [r.f32()?, r.f32()?];
            let normal = vec3(&mut r)?;
            vertices.push(BspVertex { position, uv, lightmap_uv, normal });
        }
        let _revision = r.i32()?;
        let material = ObjectRef::from_raw(index(&mut r)?);
        let _num_nodes = r.i32()?;
        let poly_flags = r.u32()?;
        let lightmap_texture = r.i32()?;
        sections.push(BspSection { vertices, material, poly_flags, lightmap_texture });
    }

    let lightmaps = count(&mut r, 100, "lightmap")?;
    for _ in 0..lightmaps {
        for _ in 0..7 {
            index(&mut r)?;
        }
        r.bytes(64 + 36)?;
        let lights = count(&mut r, 29, "lightmap light")?;
        for _ in 0..lights {
            index(&mut r)?;
            let bits = count(&mut r, 1, "shadow bit")?;
            r.bytes(bits + 28)?;
        }
        index(&mut r)?;
        r.i32()?;
    }

    let n = count(&mut r, 1, "lightmap texture")?;
    let mut textures = Vec::with_capacity(n);
    for _ in 0..n {
        index(&mut r)?;
        let held = count(&mut r, 4, "lightmap index")?;
        r.bytes(held * 4 + 8)?;
        r.i32()?;
        let mut mips = Vec::new();
        for _ in 0..2 {
            let _end = r.i32()?;
            let len = count(&mut r, 1, "lightmap mip")?;
            mips.push(r.bytes(len)?.to_vec());
        }
        let format = r.u8()?;
        let width = r.i32()?;
        let height = r.i32()?;
        r.i32()?;
        // Some maps (KF-Clandestine, KF-Forgotten, KF-Hell) save the pages
        // empty, with junk sizes: the engine rebuilds them from the
        // LightMaps entries at load. Kept with no mips.
        if mips.iter().all(|m| m.is_empty()) {
            textures.push(LightmapTexture { format, width: 0, height: 0, mips: Vec::new() });
            continue;
        }
        if !(1..=4096).contains(&width) || !(1..=4096).contains(&height) {
            return Err(invalid(&r, format!("implausible lightmap size {width} x {height}")));
        }
        textures.push(LightmapTexture { format, width: width as u32, height: height as u32, mips });
    }
    if r.remaining() != 0 {
        return Err(invalid(&r, format!("{} bytes left after the lightmap textures", r.remaining())));
    }
    for (i, s) in sections.iter().enumerate() {
        if s.lightmap_texture >= textures.len() as i32 {
            return Err(invalid(&r, format!("section {i} lightmap texture {} >= {}", s.lightmap_texture, textures.len())));
        }
    }
    Ok(BspLighting { sections, textures, lightmaps })
}
