//! Skeletal mesh models with CPU skinning, shared by weapons and zeds.
//!
//! Skinning math stays in Unreal mesh space. Non-root bone rotations
//! (reference skeleton and animation keys) are stored inverted and are
//! conjugated here; see DESIGN.md, "Weapons in first person". Each caller
//! converts skinned points to its own Bevy-space placement.

use std::collections::HashMap;
use std::rc::Rc;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;

use ue_assets::material::{Blend, resolve_skinned};
use ue_assets::package::ObjectRef;
use ue_assets::package_set::{LoadedPackage, ObjectHandle, PackageSet};
use ue_assets::skeletal::{MeshAnimation, SkeletalMesh, Track, read_mesh_animation, read_skeletal_mesh};
use ue_assets::texture::{decode_rgba, read_texture};

pub struct SkinnedPart {
    pub mesh: Handle<Mesh>,
    pub material: Handle<StandardMaterial>,
    /// The mesh material slot this part draws (Skins[index]).
    pub material_index: usize,
    /// Point index of each vertex (vertex = one wedge).
    vertex_points: Vec<usize>,
    /// Triangle indices into this part's vertices, for normals.
    indices: Vec<u32>,
}

pub struct SkinnedModel {
    pub mesh: SkeletalMesh,
    pub anim: Option<MeshAnimation>,
    /// Per mesh bone: index of its track in the animation, if any.
    bone_track: Vec<Option<usize>>,
    /// Lower-case sequence name -> index.
    sequences: HashMap<String, usize>,
    /// Per influence: point position in its bone's bind-pose space.
    influence_local: Vec<Vec3>,
    pub parts: Vec<SkinnedPart>,
    /// Sign that makes computed normals point outward (decided at load).
    normal_sign: f32,
    lit: bool,
}

/// Unreal rotation of a stored bone quaternion; non-root ones are conjugated.
fn bone_quat(index: usize, q: [f32; 4]) -> Quat {
    let q = Quat::from_xyzw(q[0], q[1], q[2], q[3]).normalize();
    if index == 0 { q } else { q.conjugate() }
}

fn globals(mesh: &SkeletalMesh, locals: &[(Quat, Vec3)]) -> Vec<(Quat, Vec3)> {
    let mut out: Vec<(Quat, Vec3)> = Vec::with_capacity(locals.len());
    for (i, b) in mesh.bones.iter().enumerate() {
        let (lq, lp) = locals[i];
        if i == 0 {
            out.push((lq, lp));
        } else {
            let (pq, pp) = out[b.parent];
            out.push((pq * lq, pp + pq * lp));
        }
    }
    out
}

/// Samples a track at `frame` (interpolating between keys; wraps for loops).
fn sample(track: &Track, frame: f32, track_time: f32) -> (Quat, Vec3) {
    let q = |i: usize| {
        let r = track.rotations[i.min(track.rotations.len() - 1)];
        Quat::from_xyzw(r[0], r[1], r[2], r[3]).normalize()
    };
    let p = |i: usize| Vec3::from_array(track.positions[i.min(track.positions.len() - 1)]);
    let n = track.times.len();
    if n <= 1 || track.rotations.len() <= 1 && track.positions.len() <= 1 {
        return (q(0), p(0));
    }
    let k = track.times.iter().rposition(|&t| t <= frame).unwrap_or(0);
    let (k2, t0, t1) = if k + 1 < n {
        (k + 1, track.times[k], track.times[k + 1])
    } else {
        (0, track.times[k], track_time.max(track.times[k] + 1e-3))
    };
    let a = ((frame - t0) / (t1 - t0).max(1e-4)).clamp(0.0, 1.0);
    (q(k).slerp(q(k2), a), p(k).lerp(p(k2), a))
}

/// Decodes a texture to RGBA (mip 0 only).
pub fn decode_image(h: &ObjectHandle, images: &mut Assets<Image>) -> Option<Handle<Image>> {
    use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    let tex = read_texture(&h.package.pkg, h.export).ok()?;
    let mip = tex.mips.first()?;
    let rgba = decode_rgba(tex.format, mip, None)?;
    let mut image = Image::new(
        Extent3d {
            width: mip.width as u32,
            height: mip.height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        ..default()
    });
    Some(images.add(image))
}

/// Where the model's skins come from: references plus the package they belong to.
pub struct Skins {
    pub refs: Vec<ObjectRef>,
    pub package: Option<Rc<LoadedPackage>>,
    /// Skins named by path instead (KFWeapon SkinRefs, loaded with
    /// DynamicLoadObject); these win over `refs`.
    pub named: Vec<Option<ObjectHandle>>,
}

impl SkinnedModel {
    /// Loads a skeletal mesh (and its animation set) and creates one Bevy
    /// mesh + material per material index. `lit` adds normals and lighting.
    pub fn load(
        set: &PackageSet,
        mesh_h: &ObjectHandle,
        skins: &Skins,
        lit: bool,
        meshes: &mut Assets<Mesh>,
        images: &mut Assets<Image>,
        materials: &mut Assets<StandardMaterial>,
    ) -> Result<SkinnedModel, String> {
        let mesh = read_skeletal_mesh(&mesh_h.package.pkg, mesh_h.export).map_err(|e| e.to_string())?;
        let anim = set
            .resolve(&mesh_h.package, mesh.animation)
            .and_then(|a| read_mesh_animation(&a.package.pkg, a.export).ok());

        let bind_locals: Vec<(Quat, Vec3)> = mesh
            .bones
            .iter()
            .enumerate()
            .map(|(i, b)| (bone_quat(i, b.rotation), Vec3::from_array(b.position)))
            .collect();
        let bind_global = globals(&mesh, &bind_locals);
        let influence_local = mesh
            .influences
            .iter()
            .map(|inf| {
                let (gq, gp) = bind_global[inf.bone];
                gq.conjugate() * (Vec3::from_array(mesh.points[inf.point]) - gp)
            })
            .collect();
        let (bone_track, sequences) = match &anim {
            Some(a) => {
                let idx: HashMap<&str, usize> = a.bones.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
                (
                    mesh.bones.iter().map(|b| idx.get(b.name.as_str()).copied()).collect(),
                    a.sequences
                        .iter()
                        .enumerate()
                        .map(|(i, s)| (s.name.to_ascii_lowercase(), i))
                        .collect(),
                )
            }
            None => (vec![None; mesh.bones.len()], HashMap::new()),
        };

        // Normal orientation: in the bind pose, do triangle normals (from the
        // stored winding) mostly point away from the mesh centre?
        let centre = mesh.points.iter().map(|p| Vec3::from_array(*p)).sum::<Vec3>() / mesh.points.len().max(1) as f32;
        let mut outward = 0.0f32;
        for t in &mesh.triangles {
            let p = |k: usize| Vec3::from_array(mesh.points[mesh.wedges[t.wedges[k]].vertex]);
            let n = (p(1) - p(0)).cross(p(2) - p(0));
            outward += n.dot((p(0) + p(1) + p(2)) / 3.0 - centre).signum();
        }
        let normal_sign = if outward >= 0.0 { 1.0 } else { -1.0 };

        let mut parts = Vec::new();
        for mat_index in 0..mesh.material_slots.len().max(1) {
            let tris: Vec<_> = mesh.triangles.iter().filter(|t| t.material == mat_index).collect();
            if tris.is_empty() {
                continue;
            }
            let mut remap: HashMap<usize, u32> = HashMap::new();
            let (mut positions, mut uvs, mut vertex_points, mut indices) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for t in tris {
                for &w in &t.wedges {
                    let idx = *remap.entry(w).or_insert_with(|| {
                        let wedge = mesh.wedges[w];
                        positions.push([0.0f32; 3]);
                        uvs.push(wedge.uv);
                        vertex_points.push(wedge.vertex);
                        (positions.len() - 1) as u32
                    });
                    indices.push(idx);
                }
            }
            let n = positions.len();
            let mut bevy_mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
                .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
                .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
                .with_inserted_indices(Indices::U32(indices.clone()));
            if lit {
                bevy_mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; n]);
            }

            // Material: Skins[i] if set, else the mesh's texture slot.
            let slot = mesh.material_slots.get(mat_index).copied().unwrap_or(mat_index);
            let (rf, from) = match (skins.named.get(mat_index), skins.refs.get(mat_index), &skins.package) {
                (Some(Some(h)), _, _) => (ObjectRef::Export(h.export), h.package.clone()),
                (_, Some(&s), Some(p)) if s != ObjectRef::Null => (s, p.clone()),
                _ => (
                    mesh.textures.get(slot).copied().unwrap_or(ObjectRef::Null),
                    mesh_h.package.clone(),
                ),
            };
            let simple = resolve_skinned(set, &ObjectHandle { package: from, export: 0 }, rf);
            crate::engine::runlog::kv(
                "skinned_material",
                &format!(
                    "mesh={} part={mat_index} chain={:?} texture={:?} blend={:?}",
                    mesh_h.path(),
                    simple.chain,
                    simple.texture.as_ref().map(|t| t.path()),
                    simple.blend
                ),
            );
            let image = simple.texture.as_ref().and_then(|t| decode_image(t, images));
            let material = materials.add(StandardMaterial {
                base_color_texture: image,
                unlit: !lit,
                perceptual_roughness: 0.9,
                reflectance: 0.1,
                alpha_mode: match simple.blend {
                    Blend::Masked => AlphaMode::Mask(0.5),
                    Blend::Additive => AlphaMode::Add,
                    Blend::Translucent => AlphaMode::Blend,
                    _ => AlphaMode::Opaque,
                },
                cull_mode: None,
                double_sided: true,
                ..default()
            });
            parts.push(SkinnedPart {
                mesh: meshes.add(bevy_mesh),
                material,
                material_index: mat_index,
                vertex_points,
                indices,
            });
        }

        Ok(SkinnedModel {
            mesh,
            anim,
            bone_track,
            sequences,
            influence_local,
            parts,
            normal_sign,
            lit,
        })
    }

    /// Sequence index by name (case-insensitive).
    pub fn sequence_name(&self, seq: usize) -> Option<&str> {
        self.anim.as_ref().and_then(|a| a.sequences.get(seq)).map(|s| s.name.as_str())
    }

    pub fn sequence(&self, name: &str) -> Option<usize> {
        self.sequences.get(&name.to_ascii_lowercase()).copied()
    }

    /// Length of a sequence in frames.
    pub fn length(&self, seq: usize) -> f32 {
        self.anim.as_ref().map_or(1.0, |a| a.sequences[seq].track_time)
    }

    /// Every sound named by the animations' sound notifies (for preloading).
    pub fn all_notify_sounds(&self) -> Vec<String> {
        self.anim.as_ref().map_or(Vec::new(), |a| a.sequences.iter().flat_map(|s| s.notifies.iter()).filter_map(|n| n.sound.as_ref().map(|s| s.sound.clone())).collect())
    }

    /// A sequence's animation notifies (timed events).
    pub fn notifies(&self, seq: usize) -> &[ue_assets::skeletal::Notify] {
        self.anim.as_ref().and_then(|a| a.sequences.get(seq)).map_or(&[], |s| s.notifies.as_slice())
    }

    /// A bone's frame for a pose: origin and X, Y, Z axes in mesh space.
    pub fn bone_frame(&self, pose: &[(Quat, Vec3)], bone: usize) -> Option<(Vec3, [Vec3; 3])> {
        let (q, p) = *pose.get(bone)?;
        Some((p, [q * Vec3::X, q * Vec3::Y, q * Vec3::Z]))
    }

    /// Frames per second of a sequence.
    pub fn rate(&self, seq: usize) -> f32 {
        self.anim.as_ref().map_or(30.0, |a| a.sequences[seq].rate)
    }

    /// Index of the first bone whose name matches, ignoring case, either
    /// exactly or as a `_name` suffix (KF's HeadBone 'head' vs the Clot's
    /// 'CHR_Head'; the mesh's tag table says the same, see `tag_frame`).
    pub fn find_bone(&self, name: &str) -> Option<usize> {
        let n = name.to_ascii_lowercase();
        let suffix = format!("_{n}");
        self.mesh
            .bones
            .iter()
            .position(|b| b.name.eq_ignore_ascii_case(name))
            .or_else(|| self.mesh.bones.iter().position(|b| b.name.to_ascii_lowercase().ends_with(&suffix)))
    }

    /// Like `pose`, also returning each bone's global rotation and position
    /// (Unreal mesh space).
    pub fn pose_with_bones(&self, seq: Option<usize>, frame: f32) -> (Vec<Vec3>, Vec<(Quat, Vec3)>) {
        self.pose_collapsed(seq, frame, &[])
    }

    /// True if `bone` is `root` or one of its descendants.
    pub fn is_under(&self, bone: usize, root: usize) -> bool {
        let mut b = bone;
        for _ in 0..64 {
            if b == root {
                return true;
            }
            if b == 0 {
                return false;
            }
            b = self.mesh.bones[b].parent;
        }
        false
    }

    /// Like `pose_with_bones`, but points weighted to a bone in `collapse`
    /// (or below it) shrink onto that bone's origin (UE2 SetBoneScale 0);
    /// used to remove a zed's head and severed limbs.
    pub fn pose_collapsed(
        &self,
        seq: Option<usize>,
        frame: f32,
        collapse: &[usize],
    ) -> (Vec<Vec3>, Vec<(Quat, Vec3)>) {
        self.pose_layered(seq, frame, None, collapse)
    }

    /// Like `pose_collapsed`, with an optional second animation layer
    /// `(sequence, frame, root bone)` that fully replaces the base animation
    /// for the root bone and everything under it (UE2 AnimBlendParams on
    /// channel 1 with alpha 1 from a bone, e.g. a zed's upper-body flinch).
    pub fn pose_layered(
        &self,
        seq: Option<usize>,
        frame: f32,
        overlay: Option<(usize, f32, usize)>,
        collapse: &[usize],
    ) -> (Vec<Vec3>, Vec<(Quat, Vec3)>) {
        let locals: Vec<(Quat, Vec3)> = self
            .mesh
            .bones
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let (seq, frame) = match overlay {
                    Some((os, of, root)) if self.is_under(i, root) => (Some(os), of),
                    _ => (seq, frame),
                };
                match (seq, self.anim.as_ref(), self.bone_track[i]) {
                    (Some(s), Some(a), Some(t)) => {
                        let seq = &a.sequences[s];
                        let (q, p) = sample(&seq.tracks[t], frame, seq.track_time);
                        (if i == 0 { q } else { q.conjugate() }, p)
                    }
                    _ => (bone_quat(i, b.rotation), Vec3::from_array(b.position)),
                }
            })
            .collect();
        let pose = globals(&self.mesh, &locals);
        (self.skin(&pose, collapse), pose)
    }

    /// Skins the mesh from bone transforms in mesh space (rotation, origin),
    /// e.g. a sampled animation or a ragdoll. Bones in or under a
    /// `collapse` bone shrink into that bone's origin (decapitation, limbs).
    pub fn skin(&self, pose: &[(Quat, Vec3)], collapse: &[usize]) -> Vec<Vec3> {
        let mut skinned = vec![Vec3::ZERO; self.mesh.points.len()];
        for (inf, local) in self.mesh.influences.iter().zip(&self.influence_local) {
            let (q, p) = pose[inf.bone];
            let pos = match collapse.iter().find(|&&c| self.is_under(inf.bone, c)) {
                Some(&c) => pose[c].1,
                None => q * *local + p,
            };
            skinned[inf.point] += inf.weight * pos;
        }
        skinned
    }

    /// A mesh tag's frame (`neck`, `head`, ...: UE2 attach aliases) for a
    /// pose: origin and X, Y, Z axes in mesh space. None if the mesh has no
    /// such tag or lacks its bone.
    pub fn tag_frame(&self, pose: &[(Quat, Vec3)], alias: &str) -> Option<(Vec3, [Vec3; 3])> {
        let tag = self.mesh.tags.iter().find(|t| t.alias.eq_ignore_ascii_case(alias))?;
        let bone = self.mesh.bones.iter().position(|b| b.name.eq_ignore_ascii_case(&tag.bone))?;
        let (q, p) = *pose.get(bone)?;
        let axes = tag.axes.map(|a| q * Vec3::from_array(a));
        Some((p + q * Vec3::from_array(tag.origin), axes))
    }

    /// The bone a tag alias stands for, and the alias of a bone (the first
    /// tag naming it), from the mesh's tag table.
    pub fn tag_bone(&self, alias: &str) -> Option<usize> {
        let tag = self.mesh.tags.iter().find(|t| t.alias.eq_ignore_ascii_case(alias))?;
        self.mesh.bones.iter().position(|b| b.name.eq_ignore_ascii_case(&tag.bone))
    }

    pub fn bone_tag(&self, bone: usize) -> Option<&str> {
        let name = &self.mesh.bones.get(bone)?.name;
        self.mesh.tags.iter().find(|t| t.bone.eq_ignore_ascii_case(name)).map(|t| t.alias.as_str())
    }

    /// Each bone's local rotation and position (relative to its parent,
    /// Unreal mesh space) for `seq` at `frame`; the bind pose for bones the
    /// animation lacks, or with no sequence. Used for blending several
    /// sequences (the player's body).
    pub fn sample_locals(&self, seq: Option<usize>, frame: f32) -> Vec<(Quat, Vec3)> {
        self.mesh
            .bones
            .iter()
            .enumerate()
            .map(|(i, b)| match (seq, self.anim.as_ref(), self.bone_track[i]) {
                (Some(s), Some(a), Some(t)) => {
                    let seq = &a.sequences[s];
                    let (q, p) = sample(&seq.tracks[t], frame, seq.track_time);
                    (if i == 0 { q } else { q.conjugate() }, p)
                }
                _ => (bone_quat(i, b.rotation), Vec3::from_array(b.position)),
            })
            .collect()
    }

    /// Blends `other` into `into` by `alpha` (0 keeps `into`, 1 takes
    /// `other`), for every bone, or only `root` and the bones under it
    /// (UE2 AnimBlendParams from a bone).
    pub fn blend_locals(&self, into: &mut [(Quat, Vec3)], other: &[(Quat, Vec3)], alpha: f32, root: Option<usize>) {
        if alpha <= 0.0 {
            return;
        }
        let alpha = alpha.min(1.0);
        for (i, (dst, src)) in into.iter_mut().zip(other).enumerate() {
            if root.is_some_and(|r| !self.is_under(i, r)) {
                continue;
            }
            *dst = if alpha >= 1.0 { *src } else { (dst.0.slerp(src.0, alpha), dst.1.lerp(src.1, alpha)) };
        }
    }

    /// Bone transforms in mesh space from local transforms, with extra
    /// turns: `(bone, rotation)` rotates that bone and everything under it
    /// about the bone's origin, the rotation given in mesh space (e.g. the
    /// aim pitch on the spine).
    pub fn pose_from_locals(&self, locals: &[(Quat, Vec3)], turns: &[(usize, Quat)]) -> Vec<(Quat, Vec3)> {
        let mut out: Vec<(Quat, Vec3)> = Vec::with_capacity(locals.len());
        for (i, b) in self.mesh.bones.iter().enumerate() {
            let (lq, lp) = locals[i];
            let (mut q, p) = if i == 0 {
                (lq, lp)
            } else {
                let (pq, pp) = out[b.parent];
                (pq * lq, pp + pq * lp)
            };
            for (bone, turn) in turns {
                if *bone == i {
                    q = *turn * q;
                }
            }
            out.push((q, p));
        }
        out
    }

    /// UE2's mesh-to-actor transform (Unreal units, actor axes):
    /// PrePivot + DrawScale x RotOrigin-rotation of ((point - Origin) x
    /// Scale). Shared by zeds and the player's body.
    pub fn mesh_to_actor(&self, pre_pivot: Vec3, draw_scale: f32) -> impl Fn(Vec3) -> Vec3 + use<> {
        let r = self.mesh.rot_origin;
        // Applied as stored: checked on the Clot, whose feet point along mesh +Y;
        // RotOrigin yaw -16384 turns that to +X, the actor's forward.
        let rot = crate::engine::coords::ue_rotation_matrix(ue_assets::properties::Rotator {
            pitch: r[0],
            yaw: r[1],
            roll: r[2],
        });
        let scale = Vec3::from_array(self.mesh.scale);
        let origin = Vec3::from_array(self.mesh.origin);
        move |p| pre_pivot + draw_scale * (rot * ((p - origin) * scale))
    }

    /// Bone transforms in mesh space for the reference (bind) pose.
    pub fn bind_pose(&self) -> Vec<(Quat, Vec3)> {
        self.pose_collapsed(None, 0.0, &[]).1
    }

    /// Fresh copies of this model's meshes, so several actors can be posed
    /// independently (materials stay shared).
    pub fn new_instance(&self, meshes: &mut Assets<Mesh>) -> Vec<Handle<Mesh>> {
        let copies: Vec<Mesh> = self.parts.iter().filter_map(|p| meshes.get(&p.mesh).cloned()).collect();
        copies.into_iter().map(|m| meshes.add(m)).collect()
    }

    /// Writes skinned points into the model's own meshes (see `upload_to`).
    pub fn upload(&self, skinned: &[Vec3], to_local: impl Fn(Vec3) -> Vec3, meshes: &mut Assets<Mesh>) {
        let handles: Vec<Handle<Mesh>> = self.parts.iter().map(|p| p.mesh.clone()).collect();
        self.upload_to(&handles, skinned, to_local, meshes);
    }

    /// Writes skinned points into the given meshes (one per part), converting
    /// each with `to_local` (Unreal mesh space -> Bevy space of the owning
    /// entity). Lit models also get fresh normals.
    pub fn upload_to(
        &self,
        handles: &[Handle<Mesh>],
        skinned: &[Vec3],
        to_local: impl Fn(Vec3) -> Vec3,
        meshes: &mut Assets<Mesh>,
    ) {
        let local: Vec<Vec3> = skinned.iter().map(|&p| to_local(p)).collect();
        for (part, handle) in self.parts.iter().zip(handles) {
            let Some(mut mesh) = meshes.get_mut(handle) else {
                continue;
            };
            let positions: Vec<Vec3> = part.vertex_points.iter().map(|&pi| local[pi]).collect();
            if self.lit {
                let mut normals = vec![Vec3::ZERO; positions.len()];
                for t in part.indices.as_chunks::<3>().0 {
                    let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
                    let n = (positions[b] - positions[a]).cross(positions[c] - positions[a]);
                    normals[a] += n;
                    normals[b] += n;
                    normals[c] += n;
                }
                // The axis change to Bevy flips handedness, so the bind-pose
                // sign (decided in Unreal space) is inverted here.
                let sign = -self.normal_sign;
                let normals: Vec<[f32; 3]> = normals.iter().map(|n| (*n * sign).normalize_or_zero().to_array()).collect();
                mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, VertexAttributeValues::Float32x3(normals));
            }
            let positions: Vec<[f32; 3]> = positions.iter().map(|p| p.to_array()).collect();
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, VertexAttributeValues::Float32x3(positions));
        }
    }
}
