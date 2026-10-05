//! Baked (static) lighting stored in maps.
//!
//! `StaticMeshInstance` (one per lit StaticMeshActor, its
//! `StaticMeshInstance` property): after the property list, a colour per
//! vertex of the mesh's vertex stream (compact count, then R, G, B, A per
//! colour, A always 255 in KF maps; first read as B, G, R, which turned
//! KF-WestLondon's warm walls blue against your game screenshots), then
//! data not read here (an int, then
//! the lights that touch the mesh). Worked out from the data; checked by
//! `kfpkg lighting <map>` (colour count = mesh vertex count).

use crate::package::Package;
use crate::properties::read_export_properties;
use crate::reader::{ReadErrorKind, Reader, Result};

/// A placed mesh's baked vertex colours, as R, G, B, A.
pub fn read_mesh_instance_colors(pkg: &Package, export: usize) -> Result<Vec<[u8; 4]>> {
    let props = read_export_properties(pkg, export)?;
    let mut r = Reader::new(pkg.export_data(export));
    r.seek(props.end)?;
    let n = r.compact_index()?;
    if !(0..=1_000_000).contains(&n) {
        return Err(r.error(ReadErrorKind::Invalid(format!("implausible colour count {n}"))));
    }
    (0..n)
        .map(|_| {
            r.array::<4>()
        })
        .collect()
}
