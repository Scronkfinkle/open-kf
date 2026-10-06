//! Skeletal (animated) meshes and their animation sets (`.ukx` packages).
//!
//! Layouts worked out from the data (KF version 128/29) and checked by
//! `kfpkg skelmeshes` / `kfpkg anims` across the install.
//!
//! **SkeletalMesh**: property list; bounding box and sphere; a mesh header
//! (version, vertex count, an unused vertex array, texture list, mesh scale,
//! origin, rotation); legacy LOD-mesh arrays; 62 bytes of impostor settings
//! (all zero in the files examined); the reference skeleton (name, flags,
//! rotation, position, length, size, child count, parent index per bone);
//! the animation set reference; skeleton depth; then level-of-detail models
//! (not read) and finally the raw source mesh as six "lazy arrays" (each
//! prefixed with the file offset where it ends): points, wedges (vertex +
//! UV), triangles, bone influences, and two index lists. The raw arrays are
//! found by locating that self-checking chain of six.
//!
//! **MeshAnimation**: property list; version; bones (name, flags, parent);
//! motion chunks (one per sequence: per-bone tracks of rotation keys,
//! position keys and key times, plus a root track); sequences (an unknown
//! float, name, groups, start frame, frame count, notifies, rate).

use crate::package::{ObjectRef, Package};
use crate::properties::read_export_properties;
use crate::reader::{ReadError, ReadErrorKind, Reader, Result};

#[derive(Debug, Clone)]
pub struct Bone {
    pub name: String,
    /// Rotation relative to the parent (x, y, z, w), Unreal convention.
    pub rotation: [f32; 4],
    /// Position relative to the parent.
    pub position: [f32; 3],
    /// Index of the parent bone; the root is its own parent (0).
    pub parent: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct Wedge {
    pub vertex: usize,
    pub uv: [f32; 2],
}

#[derive(Debug, Clone, Copy)]
pub struct Triangle {
    pub wedges: [usize; 3],
    pub material: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct Influence {
    pub weight: f32,
    pub point: usize,
    pub bone: usize,
}

#[derive(Debug, Clone)]
pub struct SkeletalMesh {
    /// Material slots: texture references; triangles use `material_slots[i]`.
    pub textures: Vec<ObjectRef>,
    /// Texture slot used by each material index.
    pub material_slots: Vec<usize>,
    pub scale: [f32; 3],
    pub origin: [f32; 3],
    /// Pitch, yaw, roll in Unreal units.
    pub rot_origin: [i32; 3],
    pub bones: Vec<Bone>,
    pub animation: ObjectRef,
    pub points: Vec<[f32; 3]>,
    pub wedges: Vec<Wedge>,
    pub triangles: Vec<Triangle>,
    pub influences: Vec<Influence>,
    /// Attachment tags (`neck`, `head`, `rarm`, ...), see `read_attach_tags`.
    pub tags: Vec<AttachTag>,
}

/// A mesh attachment tag (UE2 TagAliases / TagNames / TagCoords): an alias
/// such as `neck` for a bone, with a frame in that bone's space.
#[derive(Clone, Debug)]
pub struct AttachTag {
    pub alias: String,
    pub bone: String,
    pub origin: [f32; 3],
    /// X, Y and Z axes.
    pub axes: [[f32; 3]; 3],
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

/// Element sizes of the six raw lazy arrays, in order.
const RAW_ARRAYS: [usize; 6] = [12, 10, 12, 8, 2, 2];

/// Finds the start of the raw arrays: the first offset from `from` where six
/// lazy arrays chain, each one's stored end offset matching its contents.
fn find_raw_arrays(data: &[u8], base: usize, from: usize) -> Option<usize> {
    'outer: for start in from..data.len().saturating_sub(8) {
        let mut pos = start;
        for &size in &RAW_ARRAYS {
            let mut r = Reader::new(data);
            if r.seek(pos).is_err() {
                continue 'outer;
            }
            let Ok(end_abs) = r.i32() else { continue 'outer };
            let Ok(n) = r.compact_index() else { continue 'outer };
            if n < 0 || end_abs < 0 {
                continue 'outer;
            }
            let end = r.pos() + n as usize * size;
            if end_abs as usize != base + end || end > data.len() {
                continue 'outer;
            }
            pos = end;
        }
        return Some(start);
    }
    None
}

pub fn read_skeletal_mesh(pkg: &Package, export: usize) -> Result<SkeletalMesh> {
    let props = read_export_properties(pkg, export)?;
    let data = pkg.export_data(export);
    let base = pkg.exports[export].serial_offset;
    let mut r = Reader::new(data);
    r.seek(props.end)?;
    let _bounds = r.bytes(25 + 16)?;
    let version = r.i32()?;
    if version != 4 {
        return Err(invalid(&r, format!("mesh version {version}, expected 4")));
    }
    let _vertex_count = r.i32()?;
    let n = count(&mut r, 4, "vertex")?;
    r.bytes(n * 4)?;
    let n = count(&mut r, 1, "texture")?;
    let textures = (0..n)
        .map(|_| r.compact_index().map(ObjectRef::from_raw))
        .collect::<Result<Vec<_>>>()?;
    let scale = vec3(&mut r)?;
    let origin = vec3(&mut r)?;
    let rot_origin = [r.i32()?, r.i32()?, r.i32()?];
    // Legacy LOD-mesh arrays: face levels, faces, collapse list, wedges.
    for (what, size) in [("face level", 2), ("face", 8), ("collapse", 2), ("lod wedge", 10)] {
        let n = count(&mut r, size, what)?;
        r.bytes(n * size)?;
    }
    let n = count(&mut r, 8, "material")?;
    let mut material_slots = Vec::with_capacity(n);
    for _ in 0..n {
        let _poly_flags = r.u32()?;
        let slot = r.i32()?;
        material_slots.push(slot.max(0) as usize);
    }
    // Scale max, LOD settings (hysteresis, strength, min verts, morph, z displace).
    r.bytes(4 * 3 + 4 + 4 * 2)?;
    // Impostor settings and further fields: 62 bytes, zero in every file examined.
    r.bytes(62)?;

    let n = count(&mut r, 57, "bone")?;
    let mut bones = Vec::with_capacity(n);
    for i in 0..n {
        let name_index = r.compact_index()?;
        if name_index < 0 || name_index as usize >= pkg.names.len() {
            return Err(invalid(&r, format!("bone {i} name index {name_index} out of range")));
        }
        let _flags = r.u32()?;
        let rotation = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
        let position = vec3(&mut r)?;
        let _length = r.f32()?;
        let _size = vec3(&mut r)?;
        let _children = r.i32()?;
        let parent = r.i32()?;
        if parent < 0 || (parent as usize >= i && i > 0) {
            return Err(invalid(&r, format!("bone {i} parent {parent} not before it")));
        }
        let len2: f32 = rotation.iter().map(|c| c * c).sum();
        if (len2 - 1.0).abs() > 0.01 {
            return Err(invalid(&r, format!("bone {i} rotation not unit length ({len2})")));
        }
        bones.push(Bone {
            name: pkg.name(name_index as usize).to_string(),
            rotation,
            position,
            parent: parent as usize,
        });
    }
    let animation = ObjectRef::from_raw(r.compact_index()?);

    let raw = find_raw_arrays(data, base, r.pos()).ok_or_else(|| invalid(&r, "raw mesh arrays not found".into()))?;
    let tags = read_attach_tags(pkg, export, &bones, raw);
    r.seek(raw)?;
    let lazy = |r: &mut Reader| -> Result<usize> {
        let _end = r.i32()?;
        Ok(r.compact_index()? as usize)
    };
    let n = lazy(&mut r)?;
    let points = (0..n).map(|_| vec3(&mut r)).collect::<Result<Vec<_>>>()?;
    let n = lazy(&mut r)?;
    let mut wedges = Vec::with_capacity(n);
    for _ in 0..n {
        let vertex = r.u16()? as usize;
        wedges.push(Wedge {
            vertex,
            uv: [r.f32()?, r.f32()?],
        });
    }
    let n = lazy(&mut r)?;
    let mut triangles = Vec::with_capacity(n);
    for _ in 0..n {
        let w = [r.u16()? as usize, r.u16()? as usize, r.u16()? as usize];
        let material = r.u8()? as usize;
        let _aux_material = r.u8()?;
        let _smoothing = r.u32()?;
        triangles.push(Triangle { wedges: w, material });
    }
    let n = lazy(&mut r)?;
    let mut influences = Vec::with_capacity(n);
    for _ in 0..n {
        let weight = r.f32()?;
        let point = r.u16()? as usize;
        let bone = r.u16()? as usize;
        influences.push(Influence { weight, point, bone });
    }

    let mesh = SkeletalMesh {
        textures,
        material_slots,
        scale,
        origin,
        rot_origin,
        bones,
        animation,
        points,
        wedges,
        triangles,
        influences,
        tags,
    };
    mesh.validate().map_err(|m| invalid(&r, m))?;
    Ok(mesh)
}

impl SkeletalMesh {
    pub fn validate(&self) -> std::result::Result<(), String> {
        let (np, nw, nb) = (self.points.len(), self.wedges.len(), self.bones.len());
        if let Some(w) = self.wedges.iter().find(|w| w.vertex >= np) {
            return Err(format!("wedge vertex {} >= {np} points", w.vertex));
        }
        if let Some(t) = self.triangles.iter().find(|t| t.wedges.iter().any(|&w| w >= nw)) {
            return Err(format!("triangle wedge {:?} >= {nw}", t.wedges));
        }
        if let Some(i) = self.influences.iter().find(|i| i.point >= np || i.bone >= nb) {
            return Err(format!("influence point {} bone {} out of range", i.point, i.bone));
        }
        Ok(())
    }

    /// Per point: total influence weight (should be about 1).
    pub fn weight_sums(&self) -> Vec<f32> {
        let mut sums = vec![0.0; self.points.len()];
        for i in &self.influences {
            sums[i.point] += i.weight;
        }
        sums
    }
}

#[derive(Debug, Clone)]
pub struct Track {
    pub rotations: Vec<[f32; 4]>,
    pub positions: Vec<[f32; 3]>,
    /// Key times in frames (shared by rotations and positions when counts match).
    pub times: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct Sequence {
    pub name: String,
    pub num_frames: usize,
    /// Frames per second.
    pub rate: f32,
    /// One track per animation bone.
    pub tracks: Vec<Track>,
    /// Length of the motion chunk in frames.
    pub track_time: f32,
    /// Timed events in the animation (MeshAnimation notifies).
    pub notifies: Vec<Notify>,
}

/// An animation notify: at `time` (0..1 of the sequence) the engine calls
/// `function` on the actor, or fires the notify object. For
/// AnimNotify_Script objects `name` is their NotifyName, the script function
/// called (e.g. ZombieBloat's SpawnTwoShots).
#[derive(Debug, Clone)]
pub struct Notify {
    pub time: f32,
    pub function: String,
    pub object_class: String,
    pub name: String,
    /// AnimNotify_Effect: the effect class path, the bone it starts at,
    /// its offset and rotation relative to that bone, whether it stays
    /// attached, and its DrawScale.
    pub effect: Option<NotifyEffect>,
    /// A sound notify (CustomSoundNotify subclasses such as
    /// KFWeaponSoundNotify, or AnimNotify_Sound).
    pub sound: Option<NotifySound>,
}

#[derive(Debug, Clone)]
pub struct NotifySound {
    /// Full object path, e.g. `KF_9MMSnd.9mm_Single_Reload_000`.
    pub sound: String,
    /// Volume (class default 1 for both notify classes) and Radius
    /// (default 0).
    pub volume: f32,
    pub radius: f32,
    /// CustomSoundNotify.bAttenuate.
    pub attenuate: bool,
}

#[derive(Debug, Clone)]
pub struct NotifyEffect {
    pub class: String,
    pub bone: String,
    pub offset: [f32; 3],
    /// Pitch, yaw, roll.
    pub rotation: [i32; 3],
    pub attach: bool,
    pub draw_scale: f32,
}

#[derive(Debug, Clone)]
pub struct MeshAnimation {
    /// Bone names, in track order.
    pub bones: Vec<String>,
    pub sequences: Vec<Sequence>,
}

fn read_track(r: &mut Reader) -> Result<Track> {
    let _flags = r.u32()?;
    let n = count(r, 16, "rotation key")?;
    let rotations = (0..n)
        .map(|_| Ok([r.f32()?, r.f32()?, r.f32()?, r.f32()?]))
        .collect::<Result<Vec<_>>>()?;
    let n = count(r, 12, "position key")?;
    let positions = (0..n).map(|_| vec3(r)).collect::<Result<Vec<_>>>()?;
    let n = count(r, 4, "key time")?;
    let times = (0..n).map(|_| r.f32()).collect::<Result<Vec<_>>>()?;
    Ok(Track {
        rotations,
        positions,
        times,
    })
}

pub fn read_mesh_animation(pkg: &Package, export: usize) -> Result<MeshAnimation> {
    let props = read_export_properties(pkg, export)?;
    let mut r = Reader::new(pkg.export_data(export));
    r.seek(props.end)?;
    let _version = r.i32()?;
    let n = count(&mut r, 9, "animation bone")?;
    let mut bones = Vec::with_capacity(n);
    for _ in 0..n {
        let name_index = r.compact_index()?;
        if name_index < 0 || name_index as usize >= pkg.names.len() {
            return Err(invalid(&r, format!("animation bone name index {name_index}")));
        }
        let _flags = r.u32()?;
        let _parent = r.i32()?;
        bones.push(pkg.name(name_index as usize).to_string());
    }
    let n_moves = count(&mut r, 20, "motion chunk")?;
    let mut moves = Vec::with_capacity(n_moves);
    for _ in 0..n_moves {
        let _root_speed = vec3(&mut r)?;
        let track_time = r.f32()?;
        let _start_bone = r.i32()?;
        let _flags = r.u32()?;
        let nbi = count(&mut r, 4, "bone index")?;
        r.bytes(nbi * 4)?;
        let nt = count(&mut r, 13, "track")?;
        let tracks = (0..nt).map(|_| read_track(&mut r)).collect::<Result<Vec<_>>>()?;
        let _root_track = read_track(&mut r)?;
        moves.push((track_time, tracks));
    }
    let n_seqs = count(&mut r, 15, "sequence")?;
    if n_seqs != n_moves {
        return Err(invalid(&r, format!("{n_seqs} sequences but {n_moves} motion chunks")));
    }
    let mut sequences = Vec::with_capacity(n_seqs);
    for (track_time, tracks) in moves {
        let _unknown = r.f32()?;
        let name_index = r.compact_index()?;
        if name_index < 0 || name_index as usize >= pkg.names.len() {
            return Err(invalid(&r, format!("sequence name index {name_index}")));
        }
        let ng = count(&mut r, 1, "group")?;
        for _ in 0..ng {
            r.compact_index()?;
        }
        let _start_frame = r.i32()?;
        let num_frames = r.i32()?;
        let nn = count(&mut r, 6, "notify")?;
        let mut notifies = Vec::with_capacity(nn);
        for _ in 0..nn {
            let time = r.f32()?;
            let function = r.compact_index()?;
            let object = ObjectRef::from_raw(r.compact_index()?);
            let function = match usize::try_from(function) {
                Ok(i) if i < pkg.names.len() => pkg.name(i).to_string(),
                _ => String::new(),
            };
            let (mut object_class, mut name, mut effect, mut sound) = (String::new(), String::new(), None, None);
            if let ObjectRef::Export(e) = object {
                use crate::properties::Value;
                object_class = pkg.export_class_name(e).to_string();
                if let Ok(props) = read_export_properties(pkg, e) {
                    if let Some(Value::Name(n)) = props.get(pkg, "NotifyName") {
                        name = pkg.name(*n).to_string();
                    }
                    // Sounds live in other packages (imports, whose path
                    // includes the package).
                    if let Some(Value::Object(snd @ ObjectRef::Import(_))) = props.get(pkg, "Sound") {
                        sound = Some(NotifySound {
                            sound: pkg.object_path(*snd),
                            volume: match props.get(pkg, "Volume") {
                                Some(Value::Float(f)) => *f,
                                _ => 1.0,
                            },
                            radius: match props.get(pkg, "Radius") {
                                Some(Value::Int(i)) => *i as f32,
                                _ => 0.0,
                            },
                            attenuate: matches!(props.get(pkg, "bAttenuate"), Some(Value::Bool(true))),
                        });
                    }
                    if let Some(Value::Object(class)) = props.get(pkg, "EffectClass") {
                        effect = Some(NotifyEffect {
                            class: pkg.object_path(*class),
                            bone: match props.get(pkg, "Bone") {
                                Some(Value::Name(n)) => pkg.name(*n).to_string(),
                                _ => String::new(),
                            },
                            offset: match props.get(pkg, "OffsetLocation") {
                                Some(Value::Vector(v)) => *v,
                                _ => [0.0; 3],
                            },
                            rotation: match props.get(pkg, "OffsetRotation") {
                                Some(Value::Rotator(r)) => [r.pitch, r.yaw, r.roll],
                                _ => [0; 3],
                            },
                            attach: matches!(props.get(pkg, "Attach"), Some(Value::Bool(true))),
                            draw_scale: match props.get(pkg, "DrawScale") {
                                Some(Value::Float(f)) => *f,
                                _ => 1.0,
                            },
                        });
                    }
                }
            }
            notifies.push(Notify { time, function, object_class, name, effect, sound });
        }
        let rate = r.f32()?;
        sequences.push(Sequence {
            name: pkg.name(name_index as usize).to_string(),
            num_frames: num_frames.max(0) as usize,
            rate,
            tracks,
            track_time,
            notifies,
        });
    }
    if r.remaining() != 0 {
        return Err(invalid(&r, format!("{} bytes left after sequences", r.remaining())));
    }
    Ok(MeshAnimation { bones, sequences })
}

/// A candidate tag table: start and end byte offsets, (alias, bone) pairs.
pub type TagCandidate = (usize, usize, Vec<(String, String)>);

/// Candidate attachment tag tables in a skeletal mesh's data: UE2 meshes can
/// store attach aliases (TagAliases) and the bones they stand for
/// (TagNames) as two equal-length name arrays. Not read by the main reader;
/// found by scanning for such a pair whose second array names mostly bones.
/// Returns (start, end byte offset, [(alias, bone)]) for every candidate.
pub fn find_attach_tags(pkg: &Package, export: usize, bones: &[Bone]) -> Vec<TagCandidate> {
    find_attach_tags_before(pkg, export, bones, usize::MAX)
}

fn find_attach_tags_before(
    pkg: &Package,
    export: usize,
    bones: &[Bone],
    limit: usize,
) -> Vec<TagCandidate> {
    let data = pkg.export_data(export);
    let data = &data[..limit.min(data.len())];
    let is_bone = |n: &str| bones.iter().any(|b| b.name.eq_ignore_ascii_case(n));
    let name = |i: i32| (i >= 0 && (i as usize) < pkg.names.len()).then(|| pkg.name(i as usize).to_string());
    let mut found = Vec::new();
    for start in 0..data.len() {
        let mut r = Reader::new(data);
        if r.seek(start).is_err() {
            continue;
        }
        let Ok(n) = r.compact_index() else { continue };
        if !(1..=64).contains(&n) {
            continue;
        }
        let aliases: Option<Vec<String>> = (0..n).map(|_| r.compact_index().ok().and_then(name)).collect();
        let Some(aliases) = aliases else { continue };
        if r.compact_index().ok() != Some(n) {
            continue;
        }
        let targets: Option<Vec<String>> = (0..n).map(|_| r.compact_index().ok().and_then(name)).collect();
        let Some(targets) = targets else { continue };
        // Most targets must be bones (a mesh may keep a tag for a bone it
        // lacks, e.g. the Gorefast's missing left forearm).
        if targets.iter().filter(|t| is_bone(t)).count() * 4 >= targets.len() * 3 {
            found.push((start, r.pos(), aliases.into_iter().zip(targets).collect()));
        }
    }
    found
}

/// The mesh's attachment tags: the longest candidate table (at least two
/// distinct aliases) found before byte `limit` that is followed by a
/// matching count of tag frames (origin and three axes, 12 floats each).
/// Checked on the Clot (16 tags, `neck` -> CHR_Neck at (-1, 0, 0)) and the
/// Gorefast (15). Empty if none.
pub fn read_attach_tags(pkg: &Package, export: usize, bones: &[Bone], limit: usize) -> Vec<AttachTag> {
    let data = pkg.export_data(export);
    let mut best: Vec<AttachTag> = Vec::new();
    for (_, end, pairs) in find_attach_tags_before(pkg, export, bones, limit) {
        let distinct: std::collections::HashSet<String> = pairs.iter().map(|(a, _)| a.to_ascii_lowercase()).collect();
        if pairs.len() < 2 || distinct.len() != pairs.len() || pairs.len() <= best.len() {
            continue;
        }
        let mut r = Reader::new(data);
        if r.seek(end).is_err() || r.compact_index().ok() != Some(pairs.len() as i32) {
            continue;
        }
        let frames: Option<Vec<[[f32; 3]; 4]>> =
            (0..pairs.len()).map(|_| Some([vec3(&mut r).ok()?, vec3(&mut r).ok()?, vec3(&mut r).ok()?, vec3(&mut r).ok()?])).collect();
        let Some(frames) = frames else { continue };
        best = pairs
            .into_iter()
            .zip(frames)
            .map(|((alias, bone), f)| AttachTag {
                alias,
                bone,
                origin: f[0],
                axes: [f[1], f[2], f[3]],
            })
            .collect();
    }
    best
}
