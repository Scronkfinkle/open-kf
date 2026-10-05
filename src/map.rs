//! Loads a Killing Floor map into the Bevy world: BSP geometry, placed static
//! meshes, their textures and materials. Everything is logged as numbers.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::image::{
    CompressedImageFormatSupport, CompressedImageFormats, Image, ImageAddressMode, ImageFilterMode, ImageSampler,
    ImageSamplerDescriptor,
};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, Face, TextureDimension, TextureFormat};

use ue_assets::bsp::{poly_flags, read_model};
use ue_assets::bsp::read_polys;
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::level::read_level_with;
use ue_assets::material::{Blend, SimpleMaterial, resolve};
use ue_assets::package::ObjectRef;
use ue_assets::package_set::{ObjectHandle, PackageSet};
use ue_assets::properties::{Rotator, Value, read_export_properties};
use ue_assets::static_mesh::read_static_mesh;
use ue_assets::terrain::{Terrain, read_terrains};
use ue_assets::texture::{TextureFormat as UeFormat, decode_rgba, read_palette, read_texture};

use crate::collision::{CollisionGeometry, TriSoup};
use crate::coords;
use crate::runlog;

/// Which map to load and where the install is. Inserted by `main`.
#[derive(Resource)]
pub struct MapRequest {
    pub install_root: PathBuf,
    pub map: String,
}

/// Where the camera should start, in Bevy coordinates.
#[derive(Resource, Default)]
pub struct SpawnPoint {
    pub position: Vec3,
    pub forward: Vec3,
}

/// Surface flag: not solid (does not block movement).
const NOT_SOLID: u32 = 0x0000_0008;

/// The render layer for sky-zone geometry, seen only by the sky camera.
pub const SKY_LAYER: usize = 1;

/// Where the sky camera sits (the SkyZoneInfo actor), if the map has a sky zone.
#[derive(Resource, Default)]
pub struct SkyInfo {
    pub camera_position: Option<Vec3>,
}

/// Each zone's fog: its ZoneInfo (zone 0 and zones without one: the
/// LevelInfo, which is a ZoneInfo), own values else class defaults.
fn zone_fog(lp: &std::rc::Rc<ue_assets::package_set::LoadedPackage>, defaults: &ClassDefaults, model: &ue_assets::bsp::Model) -> Vec<crate::zones::ZoneFog> {
    let pkg = &lp.pkg;
    let level_info = (0..pkg.exports.len()).find(|&i| pkg.export_class_name(i).ends_with("LevelInfo"));
    (0..model.num_zones.max(1))
        .map(|z| {
            let export = match model.zone_actors.get(z) {
                Some(ObjectRef::Export(e)) => Some(*e),
                _ => level_info,
            };
            let Some(e) = export else {
                return crate::zones::ZoneFog { name: "none".into(), fog: false, start: 0.0, end: 0.0, color: [128; 4] };
            };
            let props = read_export_properties(pkg, e).ok();
            let value = |n: &str| props.as_ref().and_then(|p| defaults.actor_value(lp, e, p, n));
            let float = |n: &str, d: f32| match value(n) {
                Some(Value::Float(f)) => f,
                _ => d,
            };
            crate::zones::ZoneFog {
                name: pkg.object_name(ObjectRef::Export(e)).to_string(),
                fog: matches!(value("bDistanceFog"), Some(Value::Bool(true))),
                start: float("DistanceFogStart", 3000.0),
                end: float("DistanceFogEnd", 8000.0),
                color: match value("DistanceFogColor") {
                    Some(Value::Color(c)) => c,
                    _ => [128, 128, 128, 0],
                },
            }
        })
        .collect()
}

/// Map actor classes that change where pawns can go or what happens to
/// them, and whether we simulate them (docs/map-audit.md). Logged at load
/// so a misbehaving zed or player can be checked against the map first.
const MAP_FEATURES: &[(&str, bool)] = &[
    ("KFDoorMover", true),
    ("KFUseTrigger", true),
    ("ZombieVolume", true),
    ("UTJumppad", true),
    ("BlockingVolume", true),
    ("KFZombieZoneVolume", true),
    ("TerrainInfo", true),
    ("KFGlassMover", true),
    ("LavaVolume", true),
    ("Mover", false),
    ("ClientMover", false),
    ("KFElevator", false),
    ("KFTraderDoor", false),
    ("ShopVolume", false),
    ("KFTraderTeleporter", false),
    ("Teleporter", false),
    ("JumpSpot", false),
    ("WaterVolume", false),
    ("PhysicsVolume", false),
    ("KFPhysicsVolume", false),
    ("xKicker", false),
    ("KFDecoTrampoline", false),
    ("ScriptedTrigger", false),
    ("Trigger", false),
    ("KFProxyTrigger", false),
    ("UseTrigger", false),
    ("BlockingVolume_Toggleable", false),
    ("KActor", false),
    ("KFRandomItemSpawn", false),
    ("KFAmmoPickup", false),
    ("ZoneInfo", false),
];

fn log_map_features(pkg: &ue_assets::package::Package) {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for i in 0..pkg.exports.len() {
        let class = pkg.export_class_name(i);
        if let Some((name, _)) = MAP_FEATURES.iter().find(|(n, _)| *n == class) {
            *counts.entry(name).or_default() += 1;
        }
    }
    let list = |sim: bool| {
        MAP_FEATURES
            .iter()
            .filter(|(n, s)| *s == sim && counts.contains_key(n))
            .map(|(n, _)| format!("{n}:{}", counts[n]))
            .collect::<Vec<_>>()
            .join(" ")
    };
    // ZoneInfo is listed for its distance fog, which KF's sight checks use.
    runlog::kv("map_features", &format!("simulated=[{}] not_simulated=[{}]", list(true), list(false)));
}

/// Every map entity gets this, so they can be counted or removed later.
#[derive(Component)]
pub struct MapGeometry;

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpawnPoint>()
            .init_resource::<SkyInfo>()
            .add_systems(Startup, load_map);
    }
}

/// Per-load caches and counters.
struct Loader<'a> {
    set: &'a PackageSet,
    images: &'a mut Assets<Image>,
    materials: &'a mut Assets<StandardMaterial>,
    texture_cache: HashMap<String, Option<(Handle<Image>, UVec2)>>,
    material_cache: HashMap<String, Option<(Handle<StandardMaterial>, UVec2)>>,
    textures_uploaded: usize,
    texture_bytes: usize,
    textures_failed: usize,
    /// Textures kept DXT-compressed on the GPU (vs decoded to RGBA).
    textures_compressed: usize,
    materials_without_texture: usize,
    /// The GPU accepts DXT (BC1-3) textures directly.
    bc_supported: bool,
    /// Unlit copies of materials (sky zone, bUnlit actors, PF_Unlit faces).
    unlit_cache: HashMap<AssetId<StandardMaterial>, Handle<StandardMaterial>>,
}

impl Loader<'_> {
    /// The same material drawn without lighting.
    fn unlit(&mut self, h: &Handle<StandardMaterial>) -> Handle<StandardMaterial> {
        if let Some(u) = self.unlit_cache.get(&h.id()) {
            return u.clone();
        }
        let mut m = self.materials.get(h).cloned().unwrap_or_default();
        m.unlit = true;
        let u = self.materials.add(m);
        self.unlit_cache.insert(h.id(), u.clone());
        u
    }

    /// Decodes a texture (all usable mips) and uploads it. Returns the handle
    /// and the base size in texels.
    fn texture(&mut self, h: &ObjectHandle) -> Option<(Handle<Image>, UVec2)> {
        let key = h.path();
        if let Some(cached) = self.texture_cache.get(&key) {
            return cached.clone();
        }
        let result = self.decode_texture(h);
        match &result {
            Some(_) => self.textures_uploaded += 1,
            None => self.textures_failed += 1,
        }
        self.texture_cache.insert(key, result.clone());
        result
    }

    fn decode_texture(&mut self, h: &ObjectHandle) -> Option<(Handle<Image>, UVec2)> {
        let tex = read_texture(&h.package.pkg, h.export).ok()?;
        let palette = match tex.palette_ref {
            ObjectRef::Null => None,
            rf => self
                .set
                .resolve(&h.package, rf)
                .and_then(|p| read_palette(&p.package.pkg, p.export).ok()),
        };
        let mip0 = tex.mips.first()?;
        let (w, h0) = (mip0.width, mip0.height);
        // DXT data can go to the GPU as-is (4-8x smaller than RGBA) if the GPU
        // supports it and the size is a whole number of 4x4 blocks.
        let gpu_format = match tex.format {
            UeFormat::Dxt1 => Some(TextureFormat::Bc1RgbaUnormSrgb),
            UeFormat::Dxt3 => Some(TextureFormat::Bc2RgbaUnormSrgb),
            UeFormat::Dxt5 => Some(TextureFormat::Bc3RgbaUnormSrgb),
            _ => None,
        }
        .filter(|_| self.bc_supported && w % 4 == 0 && h0 % 4 == 0);
        let mut data = Vec::new();
        let mut levels = 0u32;
        for (k, mip) in tex.mips.iter().enumerate() {
            // Use the mip chain only while it halves exactly and has data.
            let (ew, eh) = ((w >> k).max(1), (h0 >> k).max(1));
            if mip.width != ew || mip.height != eh {
                break;
            }
            if gpu_format.is_some() {
                if tex.format.data_size(ew, eh) != Some(mip.data.len()) {
                    break;
                }
                data.extend_from_slice(&mip.data);
            } else {
                let Some(rgba) = decode_rgba(tex.format, mip, palette.as_deref()) else {
                    break;
                };
                data.extend_from_slice(&rgba);
            }
            levels += 1;
        }
        if levels == 0 {
            return None;
        }
        if gpu_format.is_some() {
            self.textures_compressed += 1;
        }
        let mut image = Image::new_uninit(
            Extent3d {
                width: w as u32,
                height: h0 as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            gpu_format.unwrap_or(TextureFormat::Rgba8UnormSrgb),
            RenderAssetUsages::RENDER_WORLD,
        );
        self.texture_bytes += data.len();
        image.data = Some(data);
        image.texture_descriptor.mip_level_count = levels;
        // Unreal textures tile, so wrap rather than clamp.
        image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            mipmap_filter: ImageFilterMode::Linear,
            anisotropy_clamp: 8,
            ..default()
        });
        Some((self.images.add(image), UVec2::new(w as u32, h0 as u32)))
    }

    /// The texture with alpha forced to fully opaque, decoded to RGBA. Used
    /// for terrain layers, whose own alpha channel (often a specular mask)
    /// must not weaken the layer blend.
    fn texture_opaque(&mut self, h: &ObjectHandle) -> Option<Handle<Image>> {
        let key = format!("{}|opaque", h.path());
        if let Some(cached) = self.texture_cache.get(&key) {
            return cached.as_ref().map(|t| t.0.clone());
        }
        let result = (|| {
            let tex = read_texture(&h.package.pkg, h.export).ok()?;
            let palette = match tex.palette_ref {
                ObjectRef::Null => None,
                rf => self
                    .set
                    .resolve(&h.package, rf)
                    .and_then(|p| read_palette(&p.package.pkg, p.export).ok()),
            };
            let mip0 = tex.mips.first()?;
            let (w, h0) = (mip0.width, mip0.height);
            let mut data = Vec::new();
            let mut levels = 0u32;
            for (k, mip) in tex.mips.iter().enumerate() {
                if mip.width != (w >> k).max(1) || mip.height != (h0 >> k).max(1) {
                    break;
                }
                let Some(mut rgba) = decode_rgba(tex.format, mip, palette.as_deref()) else {
                    break;
                };
                for px in rgba.as_chunks_mut::<4>().0 {
                    px[3] = 255;
                }
                data.extend_from_slice(&rgba);
                levels += 1;
            }
            if levels == 0 {
                return None;
            }
            let mut image = Image::new_uninit(
                Extent3d {
                    width: w as u32,
                    height: h0 as u32,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::RENDER_WORLD,
            );
            self.texture_bytes += data.len();
            image.data = Some(data);
            image.texture_descriptor.mip_level_count = levels;
            image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::Repeat,
                address_mode_v: ImageAddressMode::Repeat,
                mag_filter: ImageFilterMode::Linear,
                min_filter: ImageFilterMode::Linear,
                mipmap_filter: ImageFilterMode::Linear,
                anisotropy_clamp: 8,
                ..default()
            });
            Some((self.images.add(image), UVec2::new(w as u32, h0 as u32)))
        })();
        self.texture_cache.insert(key, result.clone());
        result.map(|t| t.0)
    }

    /// A Bevy material for an Unreal material reference. `None` means "do not
    /// draw" (invisible materials). The size is the texture size for BSP UVs.
    fn material(
        &mut self,
        from: &ObjectHandle,
        rf: ObjectRef,
        force_two_sided: bool,
    ) -> Option<(Handle<StandardMaterial>, UVec2)> {
        let key = format!("{}|{:?}|{force_two_sided}", from.package.name, from.package.pkg.object_path(rf));
        if let Some(cached) = self.material_cache.get(&key) {
            return cached.clone();
        }
        let simple: SimpleMaterial = resolve(self.set, from, rf);
        let result = if simple.blend == Blend::Invisible {
            None
        } else {
            let tex = simple.texture.as_ref().and_then(|t| self.texture(t));
            if tex.is_none() {
                self.materials_without_texture += 1;
            }
            let size = tex.as_ref().map_or(UVec2::splat(256), |t| t.1);
            let two_sided = simple.two_sided || force_two_sided;
            let mat = StandardMaterial {
                base_color: if tex.is_some() { Color::WHITE } else { Color::srgb(0.5, 0.5, 0.5) },
                base_color_texture: tex.map(|t| t.0),
                alpha_mode: match simple.blend {
                    Blend::Masked => AlphaMode::Mask(0.5),
                    // Additive kept as before for the map (alpha blend).
                Blend::Translucent | Blend::Additive => AlphaMode::Blend,
                    _ => AlphaMode::Opaque,
                },
                perceptual_roughness: 1.0,
                reflectance: 0.1,
                cull_mode: if two_sided { None } else { Some(Face::Back) },
                double_sided: two_sided,
                ..default()
            };
            Some((self.materials.add(mat), size))
        };
        self.material_cache.insert(key, result.clone());
        result
    }
}

/// Polygon normal by Newell's method (right-hand rule over the vertex order).
fn newell_normal(pts: &[Vec3]) -> Vec3 {
    let mut n = Vec3::ZERO;
    for (i, a) in pts.iter().enumerate() {
        let b = pts[(i + 1) % pts.len()];
        n.x += (a.y - b.y) * (a.z + b.z);
        n.y += (a.z - b.z) * (a.x + b.x);
        n.z += (a.x - b.x) * (a.y + b.y);
    }
    n
}

/// Collects triangles for one Bevy mesh.
#[derive(Default)]
struct MeshBuilder {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    /// Per-vertex colour; left empty when not used.
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

impl MeshBuilder {
    fn build(self) -> Mesh {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs)
            .with_inserted_indices(Indices::U32(self.indices));
        if !self.colors.is_empty() {
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colors);
        }
        mesh
    }
}

// Bevy systems receive each resource as a parameter, so long lists are normal.
#[allow(clippy::too_many_arguments)]
fn load_map(
    mut commands: Commands,
    request: Res<MapRequest>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut spawn: ResMut<SpawnPoint>,
    mut sky: ResMut<SkyInfo>,
    mut collision: ResMut<CollisionGeometry>,
    mut door_setup: ResMut<crate::door::DoorSetup>,
    mut glass_setup: ResMut<crate::glass::GlassSetup>,
    game_options: Res<crate::game::GameOptions>,
    compressed: Option<Res<CompressedImageFormatSupport>>,
) {
    let started = Instant::now();
    let set = PackageSet::new(&request.install_root);
    let path = request.install_root.join("Maps").join(format!("{}.rom", request.map));
    let lp = match set.load_path(&path) {
        Ok(lp) => lp,
        Err(e) => {
            error!("cannot load map {}: {e}", path.display());
            runlog::kv("map_error", &format!("map={} error=\"{e}\"", request.map));
            return;
        }
    };
    let defaults_started = Instant::now();
    let class_defaults = ClassDefaults::new(&set);
    let contents = read_level_with(&lp, &class_defaults);
    door_setup.triggers = contents.use_triggers.clone();
    commands.insert_resource(crate::decals::MapProjectors(contents.projectors.clone()));
    log_map_features(&lp.pkg);
    commands.insert_resource(crate::pain::load(&lp, &class_defaults));
    if game_options.mode == crate::game::GameMode::Waves {
        commands.insert_resource(crate::game::load_game_data(&set, &class_defaults, &lp, game_options.length));
    }
    let mut nav = crate::nav::NavNetwork::from_graph(&ue_assets::nav::read_nav(&lp.pkg));
    nav.add_jump_pads(&lp.pkg);
    commands.insert_resource(nav);
    runlog::kv(
        "class_defaults",
        &format!(
            "seconds={:.2} not_found={:?}",
            defaults_started.elapsed().as_secs_f64(),
            class_defaults.not_found.borrow()
        ),
    );
    let mut loader = Loader {
        set: &set,
        images: &mut images,
        materials: &mut materials,
        texture_cache: HashMap::new(),
        material_cache: HashMap::new(),
        unlit_cache: HashMap::new(),
        textures_uploaded: 0,
        texture_bytes: 0,
        textures_failed: 0,
        textures_compressed: 0,
        materials_without_texture: 0,
        bc_supported: compressed.is_some_and(|c| c.0.contains(CompressedImageFormats::BC)),
    };

    // --- BSP: one mesh per material ---
    let (mut bsp_polys, mut bsp_tris, mut bsp_meshes, mut bsp_flipped, mut bsp_skipped) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    // Sky zone: index, actor name, and the Unreal-space bounds of its BSP polygons.
    let mut sky_zone: Option<(u8, String)> = None;
    let (mut sky_lo, mut sky_hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    let mut sky_polys = 0usize;
    if let Some(m) = contents.bsp_model {
        let level_handle = ObjectHandle {
            package: lp.clone(),
            export: m,
        };
        match read_model(&lp.pkg, m) {
            Ok(model) => {
                sky_zone = model.zone_actors.iter().enumerate().find_map(|(z, rf)| match rf {
                    ObjectRef::Export(e) if lp.pkg.export_class_name(*e).contains("SkyZone") => {
                        Some((z as u8, lp.pkg.object_path(*rf)))
                    }
                    _ => None,
                });
                if let Some((_, name)) = &sky_zone {
                    let actor = (0..lp.pkg.exports.len())
                        .find(|&i| lp.pkg.object_path(ObjectRef::Export(i)) == *name);
                    let location = actor
                        .and_then(|i| read_export_properties(&lp.pkg, i).ok())
                        .and_then(|p| match p.get(&lp.pkg, "Location") {
                            Some(Value::Vector(v)) => Some(*v),
                            _ => None,
                        });
                    sky.camera_position = location.map(coords::pos);
                }
                // Zones and their fog (zones.rs).
                let zone_fog = zone_fog(&lp, &class_defaults, &model);
                runlog::kv(
                    "zones",
                    &format!(
                        "zones={} fog=[{}]",
                        zone_fog.len(),
                        zone_fog
                            .iter()
                            .enumerate()
                            .map(|(i, z)| if z.fog { format!("{i}:{}..{}", z.start, z.end) } else { format!("{i}:off") })
                            .collect::<Vec<_>>()
                            .join(" ")
                    ),
                );
                commands.insert_resource(crate::zones::Zones { bsp: model.clone(), zones: zone_fog });
                let mut groups: HashMap<String, (Handle<StandardMaterial>, bool, MeshBuilder)> = HashMap::new();
                for (i, node) in model.nodes.iter().enumerate() {
                    let surf = &model.surfs[node.surf];
                    if surf.flags & (poly_flags::INVISIBLE | poly_flags::PORTAL | poly_flags::FAKE_BACKDROP) != 0
                        || node.num_verts < 3
                    {
                        bsp_skipped += 1;
                        continue;
                    }
                    let two_sided = surf.flags & poly_flags::TWO_SIDED != 0;
                    let Some((mat, size)) = loader.material(&level_handle, surf.material, two_sided) else {
                        bsp_skipped += 1;
                        continue;
                    };
                    // Polygons facing into the sky zone belong to the sky layer.
                    let in_sky = sky_zone.as_ref().is_some_and(|(z, _)| node.zone[1] == *z);
                    if in_sky {
                        sky_polys += 1;
                        for p in model.node_polygon(i) {
                            for a in 0..3 {
                                sky_lo[a] = sky_lo[a].min(p[a]);
                                sky_hi[a] = sky_hi[a].max(p[a]);
                            }
                        }
                    }
                    // PF_Unlit faces, and the sky zone (its dome was lit by
                    // baked light we do not have; our one sun made it change
                    // colour with the view): drawn unlit.
                    let unlit = in_sky || surf.flags & poly_flags::UNLIT != 0;
                    let mat = if unlit { loader.unlit(&mat) } else { mat };
                    let key = format!("{:?}|{two_sided}|{in_sky}|{unlit}", surf.material);
                    let builder = &mut groups
                        .entry(key)
                        .or_insert_with(|| (mat, in_sky, MeshBuilder::default()))
                        .2;
                    let normal = coords::dir(model.vectors[surf.normal]).normalize_or_zero();
                    let ue_pts: Vec<[f32; 3]> = model.node_polygon(i).collect();
                    let pts: Vec<Vec3> = ue_pts.iter().map(|&p| coords::pos(p)).collect();
                    // Fan-triangulate; choose the vertex order whose face
                    // normal matches the surface normal (Bevy front faces are
                    // counter-clockwise). The polygon's facing comes from
                    // Newell's method over all edges, because BSP polygons
                    // often start with three points in a line.
                    let face = newell_normal(&pts);
                    let flip = face.dot(normal) < 0.0;
                    if flip {
                        bsp_flipped += 1;
                    }
                    let base = builder.positions.len() as u32;
                    for (k, p) in pts.iter().enumerate() {
                        lo = lo.min(*p);
                        hi = hi.max(*p);
                        builder.positions.push(p.to_array());
                        builder.normals.push(normal.to_array());
                        builder
                            .uvs
                            .push(model.uv(surf, ue_pts[k], size.x as f32, size.y as f32));
                    }
                    for k in 1..pts.len() as u32 - 1 {
                        if flip {
                            builder.indices.extend([base, base + k + 1, base + k]);
                        } else {
                            builder.indices.extend([base, base + k, base + k + 1]);
                        }
                        bsp_tris += 1;
                    }
                    bsp_polys += 1;
                }
                // Collision: every polygon except not-solid and portal
                // surfaces; invisible and sky-backdrop walls still block.
                for (i, node) in model.nodes.iter().enumerate() {
                    let flags = model.surfs[node.surf].flags;
                    let in_sky = sky_zone.as_ref().is_some_and(|(z, _)| node.zone[1] == *z);
                    if flags & (NOT_SOLID | poly_flags::PORTAL) != 0 || in_sky {
                        continue;
                    }
                    let pts: Vec<Vec3> = model.node_polygon(i).map(coords::pos).collect();
                    collision.bsp.push_polygon(&pts);
                }
                for (_, (mat, in_sky, builder)) in groups {
                    let mut e = commands.spawn((
                        Mesh3d(meshes.add(builder.build())),
                        MeshMaterial3d(mat),
                        Transform::IDENTITY,
                        MapGeometry,
                    ));
                    if in_sky {
                        e.insert(RenderLayers::layer(SKY_LAYER));
                    }
                    bsp_meshes += 1;
                }
            }
            Err(e) => {
                error!("BSP failed: {e}");
                runlog::kv("bsp_error", &format!("error=\"{e}\""));
            }
        }
    }
    runlog::kv(
        "bsp_loaded",
        &format!(
            "polygons={bsp_polys} triangles={bsp_tris} meshes={bsp_meshes} skipped_polygons={bsp_skipped} \
             flipped_to_match_normal={bsp_flipped} bounds_min={lo} bounds_max={hi}"
        ),
    );

    // --- Static meshes: one Bevy mesh per (mesh, section), shared by all actors ---
    struct Part {
        /// Section index in the mesh, so an actor's `Skins[section]` can replace the material.
        section: usize,
        /// Local-space (Bevy) triangle corners of this section, if it collides.
        collision: Option<std::rc::Rc<Vec<[Vec3; 3]>>>,
        mesh: Handle<Mesh>,
        /// The mesh's own material; `None` means invisible.
        material: Option<Handle<StandardMaterial>>,
    }
    let mut mesh_cache: HashMap<String, Option<Vec<Part>>> = HashMap::new();
    let (mut actors_spawned, mut entities, mut actors_unresolved) = (0usize, 0usize, 0usize);
    let mut sky_actors = 0usize;
    let mut blocking_actors = 0usize;
    let mut movers_not_blocking = 0usize;
    // A static mesh belongs to the sky if it stands inside the sky zone's BSP bounds.
    let in_sky_bounds = |p: [f32; 3]| sky_polys > 0 && (0..3).all(|a| p[a] >= sky_lo[a] && p[a] <= sky_hi[a]);
    let (mut tri_agree, mut tri_total) = (0usize, 0usize);
    let mut mesh_parts = 0usize;
    let mut mirrored_actors = 0usize;
    let (mut skins_applied, mut invisible_parts) = (0usize, 0usize);
    // KFGlassMover.ShatteredTexture (class default), for cracked panes.
    glass_setup.cracked = set.load("KillingFloorLabTextures").and_then(|tlp| {
        let export = (0..tlp.pkg.exports.len()).find(|&i| tlp.pkg.object_name(ObjectRef::Export(i)).eq_ignore_ascii_case("ShaderCrackedGlass"))?;
        loader.material(&ObjectHandle { package: tlp, export }, ObjectRef::Export(export), false).map(|m| m.0)
    });
    for actor in &contents.mesh_actors {
        // A negative scale on an odd number of axes mirrors the mesh, which
        // turns every triangle's winding around. Unreal compensates; Bevy
        // does not, so mirrored actors get their own copy with reversed
        // triangles (otherwise back-face culling hides their front faces).
        let mirrored = actor.scale[0] * actor.scale[1] * actor.scale[2] < 0.0;
        if mirrored {
            mirrored_actors += 1;
        }
        let key = format!("{}|{mirrored}", lp.pkg.object_path(actor.mesh));
        let parts = mesh_cache.entry(key).or_insert_with(|| {
            let h = set.resolve(&lp, actor.mesh)?;
            let sm = read_static_mesh(&h.package.pkg, h.export).ok()?;
            let mut parts = Vec::new();
            for (si, section) in sm.sections.iter().enumerate() {
                let rf = sm.materials.get(si).copied().unwrap_or(ObjectRef::Null);
                let mat = loader.material(&h, rf, false).map(|m| m.0);
                let mut b = MeshBuilder::default();
                let mut remap: HashMap<u16, u32> = HashMap::new();
                let tris = &sm.indices[section.first_index..section.first_index + section.num_triangles * 3];
                for tri in tris.as_chunks::<3>().0 {
                    // Kept in file order: measured on KF-WestLondon, reversing
                    // made 111697 of 111985 triangles face against their vertex
                    // normals. The winding_agree log line keeps checking this.
                    let order = if mirrored {
                        [tri[0], tri[2], tri[1]]
                    } else {
                        [tri[0], tri[1], tri[2]]
                    };
                    let mut idx = [0u32; 3];
                    for (n, &vi) in order.iter().enumerate() {
                        idx[n] = *remap.entry(vi).or_insert_with(|| {
                            let v = vi as usize;
                            b.positions.push(coords::pos(sm.positions[v]).to_array());
                            b.normals.push(coords::dir(sm.normals[v]).normalize_or_zero().to_array());
                            b.uvs.push(sm.uvs.first().map_or([0.0, 0.0], |u| u[v]));
                            (b.positions.len() - 1) as u32
                        });
                    }
                    // Check: does the triangle face the way its vertex normals point?
                    let p = |i: u32| Vec3::from_array(b.positions[i as usize]);
                    let face = (p(idx[1]) - p(idx[0])).cross(p(idx[2]) - p(idx[0]));
                    let vn: Vec3 = idx.iter().map(|&i| Vec3::from_array(b.normals[i as usize])).sum();
                    // Only checked for unmirrored copies, whose winding should agree.
                    if !mirrored && face.length_squared() > 0.0 && vn.length_squared() > 0.0 {
                        tri_total += 1;
                        if face.dot(vn) > 0.0 {
                            tri_agree += 1;
                        }
                    }
                    b.indices.extend(idx);
                }
                if b.indices.is_empty() {
                    continue;
                }
                let collision = sm.section_collides.get(si).copied().unwrap_or(true).then(|| {
                    std::rc::Rc::new(
                        tris.as_chunks::<3>()
                            .0
                            .iter()
                            .map(|t| t.map(|v| coords::pos(sm.positions[v as usize])))
                            .collect::<Vec<_>>(),
                    )
                });
                parts.push(Part {
                    section: si,
                    collision,
                    mesh: meshes.add(b.build()),
                    material: mat,
                });
            }
            Some(parts)
        });
        let Some(parts) = parts else {
            actors_unresolved += 1;
            continue;
        };
        let mut transform = Transform {
            translation: coords::pos(actor.location),
            rotation: coords::rotation(actor.rotation),
            scale: coords::scale(actor.scale),
        };
        // Doors move: their own entity (moved by door.rs) with the mesh
        // parts as children, and their own collider, built later from
        // these local-space triangles.
        if let Some(info) = &actor.door {
            let k = info.key_num as usize;
            let (pos, rot) = crate::door::key_pose(info, k);
            transform.translation = coords::pos(pos);
            transform.rotation = crate::door::rotation_of(rot);
            let mut soup = crate::collision::TriSoup::default();
            if actor.blocks_player {
                for part in parts.iter() {
                    if let Some(tris) = &part.collision {
                        for t in tris.iter() {
                            let [a, b, c] = t.map(|p| p * transform.scale);
                            soup.push_triangle(a, b, c);
                        }
                    }
                }
            }
            let root = commands.spawn((transform, Visibility::default(), MapGeometry, Name::new(info.name.clone()))).id();
            for part in parts.iter() {
                let material = match actor.skins.get(part.section) {
                    Some(&skin) if skin != ObjectRef::Null => {
                        skins_applied += 1;
                        loader.material(&ObjectHandle { package: lp.clone(), export: actor.export }, skin, false).map(|m| m.0)
                    }
                    _ => part.material.clone(),
                };
                let Some(material) = material else {
                    invisible_parts += 1;
                    continue;
                };
                commands.spawn((Mesh3d(part.mesh.clone()), MeshMaterial3d(material), Transform::IDENTITY, ChildOf(root)));
                entities += 1;
            }
            door_setup.doors.push(crate::door::DoorSpawn {
                info: info.clone(),
                root,
                collision: soup,
            });
            actors_spawned += 1;
            continue;
        }
        // Breakable windows (glass.rs): own entity and collider, so a pane
        // can block, crack and break.
        if let Some(info) = &actor.glass {
            let mut soup = crate::collision::TriSoup::default();
            for part in parts.iter() {
                if let Some(tris) = &part.collision {
                    for t in tris.iter() {
                        let [a, b, c] = t.map(|p| p * transform.scale);
                        soup.push_triangle(a, b, c);
                    }
                }
            }
            let root = commands.spawn((transform, Visibility::default(), MapGeometry, Name::new(info.name.clone()))).id();
            let mut first_part = None;
            for part in parts.iter() {
                let material = match actor.skins.get(part.section) {
                    Some(&skin) if skin != ObjectRef::Null => {
                        skins_applied += 1;
                        loader.material(&ObjectHandle { package: lp.clone(), export: actor.export }, skin, false).map(|m| m.0)
                    }
                    _ => part.material.clone(),
                };
                let Some(material) = material else {
                    invisible_parts += 1;
                    continue;
                };
                let e = commands.spawn((Mesh3d(part.mesh.clone()), MeshMaterial3d(material), Transform::IDENTITY, ChildOf(root))).id();
                if part.section == 0 {
                    first_part = Some(e);
                }
                entities += 1;
            }
            glass_setup.panes.push(crate::glass::GlassSpawn {
                info: info.clone(),
                root,
                skin0: first_part,
                location: actor.location,
                rotation: transform.rotation,
                translation: transform.translation,
                collision: soup,
            });
            actors_spawned += 1;
            continue;
        }
        let in_sky = in_sky_bounds(actor.location);
        if in_sky {
            sky_actors += 1;
        }
        // Movers (breakable windows, scripted barriers, lifts) move during
        // play, e.g. the KF-WestLondon street barrier that rises when the
        // helicopter leaves. Apart from doors (door.rs, handled above) they
        // are not simulated, so they do not block; otherwise such barriers
        // would block forever. See docs/map-audit.md.
        let is_mover = actor.class.contains("Mover");
        if is_mover && actor.blocks_player {
            movers_not_blocking += 1;
        }
        if actor.blocks_player && !in_sky && !is_mover {
            blocking_actors += 1;
            for part in parts.iter() {
                if let Some(tris) = &part.collision {
                    for t in tris.iter() {
                        let [a, b, c] = t.map(|p| transform.transform_point(p));
                        collision.meshes.push_triangle(a, b, c);
                    }
                }
            }
        }
        let map_handle = ObjectHandle {
            package: lp.clone(),
            export: actor.export,
        };
        for part in parts.iter() {
            // Skins[section] on the actor replaces the mesh's material.
            let material = match actor.skins.get(part.section) {
                Some(&skin) if skin != ObjectRef::Null => {
                    skins_applied += 1;
                    loader.material(&map_handle, skin, false).map(|m| m.0)
                }
                _ => part.material.clone(),
            };
            let Some(material) = material else {
                invisible_parts += 1;
                continue;
            };
            // bUnlit actors and everything in the sky zone: unlit.
            let mut material = if in_sky || actor.unlit { loader.unlit(&material) } else { material };
            // See-through sky layers (dome, fog shells) all sit within a few
            // hundred units of the sky camera; Bevy sorts transparent meshes
            // by depth along the view, so their order flipped as you looked
            // around and the sky changed colour. Draw them back to front by
            // distance from the sky camera, fixed (assumed to be Unreal's
            // order; for nested shells it is outermost first): a sort bias
            // far larger than the view-depth differences.
            if in_sky
                && let Some(cam) = sky.camera_position
                && let Some(m) = loader.materials.get(&material).cloned()
                && !matches!(m.alpha_mode, AlphaMode::Opaque | AlphaMode::Mask(_))
            {
                let d = (transform.translation - cam).length();
                material = loader.materials.add(StandardMaterial { depth_bias: d * 100.0, ..m });
                runlog::kv("sky_layer_order", &format!("actor={} distance_unreal={:.0}", lp.pkg.object_name(ObjectRef::Export(actor.export)), d / coords::SCALE));
            }
            let mut e = commands.spawn((
                Mesh3d(part.mesh.clone()),
                MeshMaterial3d(material),
                transform,
                MapGeometry,
            ));
            if in_sky {
                e.insert(RenderLayers::layer(SKY_LAYER));
            }
            entities += 1;
        }
        actors_spawned += 1;
    }
    for parts in mesh_cache.values().flatten() {
        mesh_parts += parts.len();
    }
    let unique_ok = mesh_cache.values().filter(|p| p.is_some()).count();
    runlog::kv(
        "static_meshes_loaded",
        &format!(
            "actors={} spawned={actors_spawned} unresolved={actors_unresolved} entities={entities} \
             unique_meshes={} unique_ok={unique_ok} mesh_parts={mesh_parts} \
             winding_agree={tri_agree}/{tri_total} mirrored_actors={mirrored_actors} skins_applied={skins_applied} \
             invisible_parts={invisible_parts} blocking_actors={blocking_actors} \
             movers_not_blocking={movers_not_blocking} skipped_actors={:?}",
            contents.mesh_actors.len(),
            mesh_cache.len(),
            contents.skipped
        ),
    );
    // --- Blocking volumes: brush polygons in world space, one convex hull each ---
    let (mut volumes_ok, mut volumes_failed) = (0usize, 0usize);
    for b in &contents.blocking_brushes {
        let polys = match b.model {
            ObjectRef::Export(m) => read_model(&lp.pkg, m)
                .ok()
                .and_then(|model| match model.polys {
                    ObjectRef::Export(p) => read_polys(&lp.pkg, p).ok(),
                    _ => None,
                }),
            _ => None,
        };
        let Some(polys) = polys else {
            volumes_failed += 1;
            continue;
        };
        // Brush space to world: subtract the pivot, rotate, add Location.
        // Checked on KF-WestLondon BlockingVolume44, which then lines up
        // with the car tunnel shell to within a few units.
        let rot = coords::ue_rotation_matrix(b.rotation);
        let to_world = |v: &[f32; 3]| {
            let local = Vec3::from_array(*v) - Vec3::from_array(b.pre_pivot);
            coords::pos((rot * local + Vec3::from_array(b.location)).to_array())
        };
        let mut soup = TriSoup::default();
        for poly in &polys {
            let pts: Vec<Vec3> = poly.vertices.iter().map(to_world).collect();
            soup.push_polygon(&pts);
        }
        if soup.triangles.is_empty() {
            volumes_failed += 1;
        } else {
            let blocks = crate::collision::volume_blocks(b.blocked_classes.as_deref(), b.blocks_traces);
            if b.blocked_classes.is_some() {
                runlog::kv(
                    "class_blocker",
                    &format!(
                        "volume={} blocked_classes={:?} blocks_players={} blocks_zeds={}",
                        lp.pkg.object_name(ObjectRef::Export(b.export)),
                        b.blocked_classes,
                        blocks.players,
                        blocks.zeds
                    ),
                );
            }
            collision
                .volumes
                .push((format!("{}:{}", b.class, lp.pkg.object_name(ObjectRef::Export(b.export))), blocks, soup));
            volumes_ok += 1;
        }
    }
    collision.nav_points = contents.path_nodes.iter().map(|&p| coords::pos(p)).collect();
    runlog::kv(
        "collision_geometry",
        &format!(
            "bsp_triangles={} mesh_triangles={} volumes={volumes_ok} volumes_failed={volumes_failed} path_nodes={}",
            collision.bsp.triangles.len(),
            collision.meshes.triangles.len(),
            collision.nav_points.len()
        ),
    );

    // --- Terrain ---
    let sky_zone_index = sky_zone.as_ref().map(|z| z.0);
    spawn_terrains(&mut commands, &mut meshes, &mut loader, &set, &lp, sky_zone_index, &mut collision.terrain);

    runlog::kv(
        "sky_zone",
        &format!(
            "zone={:?} actor={:?} camera_position={:?} sky_bsp_polygons={sky_polys} sky_actors={sky_actors} \
             sky_bounds_unreal={sky_lo:?}..{sky_hi:?}",
            sky_zone.as_ref().map(|z| z.0),
            sky_zone.as_ref().map(|z| z.1.as_str()),
            sky.camera_position,
        ),
    );
    runlog::kv(
        "textures_loaded",
        &format!(
            "uploaded={} compressed={} bc_supported={} failed={} megabytes={:.1} materials={} materials_without_texture={}",
            loader.textures_uploaded,
            loader.textures_compressed,
            loader.bc_supported,
            loader.textures_failed,
            loader.texture_bytes as f64 / 1e6,
            loader.material_cache.len(),
            loader.materials_without_texture
        ),
    );

    // --- Lighting: no baked lightmaps yet, so a sun plus ambient light ---
    commands.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 600.0,
        affects_lightmapped_meshes: true,
    });
    commands.spawn((
        DirectionalLight {
            illuminance: 4000.0,
            ..default()
        },
        Transform::from_xyz(0.0, 0.0, 0.0).looking_to(Vec3::new(-0.4, -1.0, -0.3), Vec3::Y),
        // The sun lights both the main scene and the sky zone.
        RenderLayers::from_layers(&[0, SKY_LAYER]),
        MapGeometry,
    ));

    // --- Camera start: first player start, else above the middle of the BSP ---
    let (position, forward) = match contents.player_starts.first() {
        Some(ps) => {
            let fwd = coords::ue_rotation_matrix(Rotator {
                pitch: 0,
                yaw: ps.rotation.yaw,
                roll: 0,
            }) * Vec3::X;
            // Player starts sit at collision-cylinder centre height; add eye height.
            (coords::pos(ps.location) + Vec3::Y * 0.6, coords::dir(fwd.to_array()))
        }
        None if lo.x < hi.x => ((lo + hi) * 0.5, Vec3::NEG_Z),
        None => (Vec3::ZERO, Vec3::NEG_Z),
    };
    spawn.position = position;
    spawn.forward = forward;

    let level_name = level_title(&lp);
    runlog::kv(
        "map_loaded",
        &format!(
            "map={} title=\"{level_name}\" packages_loaded={} missing_packages={:?} camera_start={position} \
             camera_forward={forward} seconds={:.2}",
            request.map,
            set.loaded_count(),
            set.missing.borrow(),
            started.elapsed().as_secs_f64()
        ),
    );
    info!("loaded {} in {:.2}s", request.map, started.elapsed().as_secs_f64());
}

/// The map's display name from LevelInfo.Title, if set.
fn level_title(lp: &ue_assets::package_set::LoadedPackage) -> String {
    let pkg = &lp.pkg;
    (0..pkg.exports.len())
        .find(|&i| pkg.export_class_name(i) == "LevelInfo")
        .and_then(|i| read_export_properties(pkg, i).ok())
        .and_then(|p| match p.get(pkg, "Title") {
            Some(Value::Str(s)) => Some(s.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// Builds every TerrainInfo in the map. Layers are drawn the way UE2 does in
/// effect: each layer's weight at a vertex is its AlphaMap alpha times the
/// coverage left by the layers above it, so weights sum to 1. Layer 0 is
/// drawn opaque scaled by its weight; the others are added on top
/// (additive blending), which makes the result independent of draw order.
fn spawn_terrains(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    loader: &mut Loader,
    set: &PackageSet,
    lp: &std::rc::Rc<ue_assets::package_set::LoadedPackage>,
    sky_zone: Option<u8>,
    collision: &mut TriSoup,
) {
    for (ti, result) in read_terrains(set, lp).into_iter().enumerate() {
        let t = match result {
            Ok(t) => t,
            Err(e) => {
                runlog::kv("terrain_error", &format!("terrain={ti} error=\"{e}\""));
                continue;
            }
        };
        let started = Instant::now();
        let (w, h) = (t.width, t.height);
        let in_sky = sky_zone.is_some() && t.zone_number == sky_zone;
        let handle = ObjectHandle {
            package: lp.clone(),
            export: t.export,
        };

        // Vertex positions (Bevy space) and normals from height differences.
        let ue: Vec<[f32; 3]> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| t.vertex(x, y)).collect();
        let at = |x: usize, y: usize| ue[y * w + x];
        let normals: Vec<[f32; 3]> = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x, y)))
            .map(|(x, y)| {
                let (x0, x1) = (x.saturating_sub(1), (x + 1).min(w - 1));
                let (y0, y1) = (y.saturating_sub(1), (y + 1).min(h - 1));
                let dzdx = (at(x1, y)[2] - at(x0, y)[2]) / (at(x1, y)[0] - at(x0, y)[0]).max(1e-3);
                let dzdy = (at(x, y1)[2] - at(x, y0)[2]) / (at(x, y1)[1] - at(x, y0)[1]).max(1e-3);
                let n = Vec3::new(-dzdx, -dzdy, 1.0).normalize();
                let n = if t.inverted { -n } else { n };
                coords::dir(n.to_array()).to_array()
            })
            .collect();
        let pos: Vec<Vec3> = ue.iter().map(|&p| coords::pos(p)).collect();

        // Triangles of visible quads, wound to face up (or down if inverted).
        let mut tris: Vec<[usize; 3]> = Vec::new();
        for y in 0..h - 1 {
            for x in 0..w - 1 {
                if !t.quad_visible(x, y) {
                    continue;
                }
                let (a, b, c, d) = (y * w + x, y * w + x + 1, (y + 1) * w + x, (y + 1) * w + x + 1);
                let pair = if t.edge_turned(x, y) { [[a, b, c], [b, d, c]] } else { [[a, b, d], [a, d, c]] };
                for mut tri in pair {
                    let face = (pos[tri[1]] - pos[tri[0]]).cross(pos[tri[2]] - pos[tri[0]]);
                    if (face.y < 0.0) != t.inverted {
                        tri.swap(1, 2);
                    }
                    tris.push(tri);
                }
            }
        }

        if !in_sky {
            for tri in &tris {
                collision.push_triangle(pos[tri[0]], pos[tri[1]], pos[tri[2]]);
            }
        }

        // Layer alpha at each vertex, sampled from the layer's AlphaMap.
        let alphas: Vec<Vec<f32>> = t
            .layers
            .iter()
            .map(|layer| layer_alpha(set, &handle, layer.alpha_map, &t))
            .collect();
        // Effective weights: a_i * product over j > i of (1 - a_j); layer 0 counts as fully covering.
        let n_layers = t.layers.len();
        let mut weights = vec![vec![0f32; w * h]; n_layers];
        for v in 0..w * h {
            let mut remaining = 1.0f32;
            for i in (0..n_layers).rev() {
                let a = if i == 0 { 1.0 } else { alphas[i][v] };
                weights[i][v] = a * remaining;
                remaining *= 1.0 - a;
            }
        }

        let (mut layer_tris, mut layers_drawn) = (0usize, 0usize);
        for (li, layer) in t.layers.iter().enumerate() {
            let Some(tex_handle) = resolve(set, &handle, layer.texture).texture else {
                continue;
            };
            let Some(image) = loader.texture_opaque(&tex_handle) else {
                continue;
            };
            let mut b = MeshBuilder::default();
            let mut remap: HashMap<usize, u32> = HashMap::new();
            for tri in &tris {
                if tri.iter().all(|&v| weights[li][v] <= 0.002) {
                    continue;
                }
                for &v in tri {
                    let idx = *remap.entry(v).or_insert_with(|| {
                        b.positions.push(pos[v].to_array());
                        b.normals.push(normals[v]);
                        b.uvs.push(layer.uv(ue[v]));
                        let wgt = weights[li][v];
                        // Layer 0 is opaque: scale its colour. Others: additive, scaled by alpha.
                        b.colors.push(if li == 0 { [wgt, wgt, wgt, 1.0] } else { [1.0, 1.0, 1.0, wgt] });
                        (b.positions.len() - 1) as u32
                    });
                    b.indices.push(idx);
                }
                layer_tris += 1;
            }
            if b.indices.is_empty() {
                continue;
            }
            let material = loader.materials.add(StandardMaterial {
                base_color_texture: Some(image),
                alpha_mode: if li == 0 { AlphaMode::Opaque } else { AlphaMode::Add },
                perceptual_roughness: 1.0,
                reflectance: 0.1,
                unlit: in_sky,
                ..default()
            });
            let mut e = commands.spawn((Mesh3d(meshes.add(b.build())), MeshMaterial3d(material), Transform::IDENTITY, MapGeometry));
            if in_sky {
                e.insert(RenderLayers::layer(SKY_LAYER));
            }
            layers_drawn += 1;
        }
        runlog::kv(
            "terrain_loaded",
            &format!(
                "terrain={ti} heightmap={w}x{h} visible_triangles={} layers={n_layers} layers_drawn={layers_drawn} \
                 layer_triangles={layer_tris} in_sky={in_sky} inverted={} seconds={:.2}",
                tris.len(),
                t.inverted,
                started.elapsed().as_secs_f64()
            ),
        );
    }
}

/// Alpha (0..1) of a terrain layer at each heightmap vertex, from its
/// AlphaMap texture (nearest texel; the map may differ in size from the
/// heightmap). A layer without an AlphaMap was never painted and contributes
/// nothing (seen on KF-Crash, KF-Wyre, KF-Steamland; layer 0 is always full).
fn layer_alpha(set: &PackageSet, from: &ObjectHandle, alpha_map: ObjectRef, t: &Terrain) -> Vec<f32> {
    let n = t.width * t.height;
    if alpha_map == ObjectRef::Null {
        return vec![0.0; n];
    }
    let Some(h) = set.resolve(&from.package, alpha_map) else {
        return vec![1.0; n];
    };
    let Ok(tex) = read_texture(&h.package.pkg, h.export) else {
        return vec![1.0; n];
    };
    let Some(mip) = tex.mips.first() else {
        return vec![1.0; n];
    };
    let Some(rgba) = decode_rgba(tex.format, mip, None) else {
        return vec![1.0; n];
    };
    let (aw, ah) = (mip.width, mip.height);
    (0..t.height)
        .flat_map(|y| (0..t.width).map(move |x| (x, y)))
        .map(|(x, y)| {
            let ax = (x * aw / t.width).min(aw - 1);
            let ay = (y * ah / t.height).min(ah - 1);
            rgba[(ay * aw + ax) * 4 + 3] as f32 / 255.0
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newell_handles_collinear_start() {
        // A square whose first three points are in a line; a cross product of
        // the first three points would be zero, Newell's method is not.
        let pts = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(2.0, 2.0, 0.0),
            Vec3::new(0.0, 2.0, 0.0),
        ];
        let n = newell_normal(&pts).normalize();
        assert!((n - Vec3::Z).length() < 1e-5, "counter-clockwise in XY faces +Z, got {n}");
        let mut rev = pts;
        rev.reverse();
        assert!((newell_normal(&rev).normalize() + Vec3::Z).length() < 1e-5);
    }
}
