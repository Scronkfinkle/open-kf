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

use crate::world::collision::{CollisionGeometry, TriSoup};
use crate::engine::coords;
use crate::engine::runlog;

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

/// LevelInfo.Title (the scoreboard's title line).
#[derive(Resource, Default, Debug, Clone)]
pub struct LevelTitle(pub String);

/// The map's PlayerStarts (Unreal units; the location is the collision
/// cylinder's centre), for the network host's FindPlayerStart.
#[derive(Resource, Default, Debug, Clone)]
pub struct PlayerStarts(pub Vec<StartSpot>);

#[derive(Debug, Clone, Copy)]
pub struct StartSpot {
    pub location: Vec3,
    /// Rotation.Yaw, Unreal units (65536 a turn).
    pub yaw: i32,
    pub enabled: bool,
    pub primary: bool,
}

/// Surface flag: not solid (does not block movement).
const NOT_SOLID: u32 = 0x0000_0008;

/// The render layer for sky-zone geometry, seen only by the sky camera.
pub const SKY_LAYER: usize = 1;

/// Where the sky camera sits (the SkyZoneInfo actor), if the map has a sky zone.
#[derive(Resource, Default)]
pub struct SkyInfo {
    pub camera_position: Option<Vec3>,
    /// The sky zone's own fog (its SkyZoneInfo), if it has fog. KF draws
    /// the sky view with this fog, measured from the sky camera, never the
    /// player's zone fog (details in the local RE.md).
    pub fog: Option<crate::world::zones::ZoneFog>,
    /// The SkyZoneInfo's Rotation (Bevy space). KF turns the sky view by
    /// it: the sky camera's rotation is this times the view's rotation.
    pub rotation: Quat,
}

/// Each zone's fog: its ZoneInfo (zone 0 and zones without one: the
/// LevelInfo, which is a ZoneInfo), own values else class defaults.
fn zone_fog(lp: &std::rc::Rc<ue_assets::package_set::LoadedPackage>, defaults: &ClassDefaults, model: &ue_assets::bsp::Model) -> Vec<crate::world::zones::ZoneFog> {
    let pkg = &lp.pkg;
    let level_info = pkg.level_actor_exports().find(|&i| pkg.export_class_name(i).ends_with("LevelInfo"));
    (0..model.num_zones.max(1))
        .map(|z| {
            let export = match model.zone_actors.get(z) {
                Some(ObjectRef::Export(e)) => Some(*e),
                _ => level_info,
            };
            let Some(e) = export else {
                return crate::world::zones::ZoneFog { name: "none".into(), fog: false, start: 0.0, end: 0.0, color: [128; 4], clear_to_fog: false, blend_time: 1.0, overlay: None, ambient: [0, 0, 255], ambient_vector: None };
            };
            let props = read_export_properties(pkg, e).ok();
            let value = |n: &str| props.as_ref().and_then(|p| defaults.actor_value(lp, e, p, n));
            let float = |n: &str, d: f32| match value(n) {
                Some(Value::Float(f)) => f,
                _ => d,
            };
            let byte = |n: &str, d: u8| match value(n) {
                Some(Value::Byte(b)) => b,
                _ => d,
            };
            crate::world::zones::ZoneFog {
                name: pkg.object_name(ObjectRef::Export(e)).to_string(),
                // ZoneInfo defaults: AmbientSaturation 255, the others 0.
                ambient: [byte("AmbientBrightness", 0), byte("AmbientHue", 0), byte("AmbientSaturation", 255)],
                // Own saved value only (the class default is not computed).
                ambient_vector: match props.as_ref().and_then(|p| p.get(pkg, "AmbientVector")) {
                    Some(Value::Vector(v)) => Some(*v),
                    _ => None,
                },
                fog: matches!(value("bDistanceFog"), Some(Value::Bool(true))),
                start: float("DistanceFogStart", 3000.0),
                end: float("DistanceFogEnd", 8000.0),
                clear_to_fog: matches!(value("bClearToFogColor"), Some(Value::Bool(true))),
                blend_time: float("DistanceFogBlendTime", 1.0),
                color: match value("DistanceFogColor") {
                    Some(Value::Color(c)) => c,
                    _ => [128, 128, 128, 0],
                },
                overlay: {
                    let flag = |n: &str| matches!(value(n), Some(Value::Bool(true)));
                    let color = |n: &str| match value(n) {
                        Some(Value::Color(c)) => [c[0], c[1], c[2]],
                        _ => [128, 128, 128],
                    };
                    if flag("bNoKFColorCorrection") {
                        None
                    } else if flag("bNewKFColorCorrection") {
                        Some(color("KFOverlayColor"))
                    } else {
                        Some(color("DistanceFogColor"))
                    }
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
    ("KFTraderDoor", true),
    // trader.rs: shops, and every Teleporter (incl. KFTraderTeleporter) as
    // a spot to put a player outside a shop. Teleporters with a URL would
    // teleport, but no KF map enables one.
    ("ShopVolume", true),
    ("KFTraderTeleporter", true),
    ("Teleporter", true),
    ("JumpSpot", false),
    ("WaterVolume", false),
    ("PhysicsVolume", false),
    ("KFPhysicsVolume", false),
    ("xKicker", false),
    ("KFDecoTrampoline", false),
    ("ScriptedTrigger", false),
    ("Trigger", false),
    ("KFProxyTrigger", false),
    ("TriggerLight", false),
    ("UseTrigger", false),
    ("BlockingVolume_Toggleable", false),
    ("KActor", false),
    ("KFRandomItemSpawn", true),
    ("KFAmmoPickup", true),
    ("ZoneInfo", false),
];

/// The level's actor list: how many actors KF plays with, and which saved
/// objects of the same classes are left out (deleted in the editor).
fn log_level_actors(pkg: &ue_assets::package::Package) {
    match pkg.level_actor_list() {
        Ok(list) => {
            let classes: std::collections::HashSet<&str> = list.actors.iter().map(|&a| pkg.export_class_name(a)).collect();
            let mut dropped: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
            for i in 0..pkg.exports.len() {
                let class = pkg.export_class_name(i);
                if classes.contains(class) && !list.set.contains(&i) {
                    *dropped.entry(class).or_default() += 1;
                }
            }
            runlog::kv(
                "level_actors",
                &format!(
                    "actors={} exports={} dropped={} dropped_classes=[{}] bsp_model={:?}",
                    list.actors.len(),
                    pkg.exports.len(),
                    dropped.values().sum::<usize>(),
                    dropped.iter().map(|(c, n)| format!("{c}:{n}")).collect::<Vec<_>>().join(" "),
                    list.model
                ),
            );
        }
        Err(e) => runlog::kv("level_actors_error", &format!("error=\"{e}\" fallback=all_exports")),
    }
}

fn log_map_features(pkg: &ue_assets::package::Package) {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for i in pkg.level_actor_exports() {
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
    // Not simulated either, counted from the actors' own saved values:
    // physics volumes that change gravity or push pawns (ZoneVelocity),
    // movers that only map events move (InitialState Trigger*, not doors:
    // door.rs logs those), and player starts that fire an Event on spawn.
    let mut overrides = Vec::new();
    let mut event_movers = Vec::new();
    let mut start_events = 0usize;
    for i in 0..pkg.exports.len() {
        let class = pkg.export_class_name(i);
        let volume = matches!(class, "PhysicsVolume" | "KFPhysicsVolume" | "DefaultPhysicsVolume" | "WaterVolume" | "LavaVolume");
        let mover = matches!(class, "Mover" | "ClientMover" | "KFElevator");
        if !volume && !mover && class != "PlayerStart" {
            continue;
        }
        let Ok(props) = read_export_properties(pkg, i) else { continue };
        let name = || pkg.object_name(ObjectRef::Export(i)).to_string();
        if volume {
            let mut what = Vec::new();
            for field in ["Gravity", "ZoneVelocity"] {
                if let Some(Value::Vector(v)) = props.get(pkg, field) {
                    what.push(format!("{field}=({:.0},{:.0},{:.0})", v[0], v[1], v[2]));
                }
            }
            if !what.is_empty() {
                overrides.push(format!("{}:{}", name(), what.join(",")));
            }
        } else if mover {
            if let Some(Value::Name(n)) = props.get(pkg, "InitialState")
                && pkg.name(*n).starts_with("Trigger")
            {
                event_movers.push(format!("{}:{}", name(), pkg.name(*n)));
            }
        } else if matches!(props.get(pkg, "Event"), Some(Value::Name(n)) if !pkg.name(*n).is_empty() && pkg.name(*n) != "None") {
            start_events += 1;
        }
    }
    runlog::kv(
        "map_features",
        &format!(
            "simulated=[{}] not_simulated=[{}] physics_volume_overrides={} [{}] event_driven_movers={} [{}] player_start_events={start_events}",
            list(true),
            list(false),
            overrides.len(),
            overrides.join(" "),
            event_movers.len(),
            event_movers.join(" ")
        ),
    );
}

/// Every map entity gets this, so they can be counted or removed later.
#[derive(Component)]
pub struct MapGeometry;

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpawnPoint>()
            .init_resource::<PlayerStarts>()
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
    binary_alpha_cache: HashMap<String, bool>,
    material_cache: HashMap<String, Option<(Handle<StandardMaterial>, UVec2)>>,
    textures_uploaded: usize,
    texture_bytes: usize,
    textures_failed: usize,
    /// Textures kept DXT-compressed on the GPU (vs decoded to RGBA).
    textures_compressed: usize,
    materials_without_texture: usize,
    /// The GPU accepts DXT (BC1-3) textures directly.
    bc_supported: bool,
    /// Anisotropic filtering of the tiling textures (`--anisotropy`).
    anisotropy: u16,
    /// Unlit copies of materials (sky zone, bUnlit actors, PF_Unlit faces).
    unlit_cache: HashMap<AssetId<StandardMaterial>, Handle<StandardMaterial>>,
    /// Baked-mesh copies of materials (render/baked.rs).
    baked_materials: &'a mut Assets<crate::render::baked::BakedMaterial>,
    baked_cache: HashMap<AssetId<StandardMaterial>, Handle<crate::render::baked::BakedMaterial>>,
}

impl Loader<'_> {
    /// The material for a mesh with baked colours: as before (texture x
    /// colour x K) plus Bevy's dynamic lights (render/baked.rs).
    fn baked(&mut self, h: &Handle<StandardMaterial>) -> Handle<crate::render::baked::BakedMaterial> {
        if let Some(b) = self.baked_cache.get(&h.id()) {
            return b.clone();
        }
        let m = self.materials.get(h).cloned().unwrap_or_default();
        let b = self.baked_materials.add(crate::render::baked::baked_material(&m));
        self.baked_cache.insert(h.id(), b.clone());
        b
    }

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
            anisotropy_clamp: self.anisotropy,
            ..default()
        });
        Some((self.images.add(image), UVec2::new(w as u32, h0 as u32)))
    }

    /// True if the texture's alpha is on/off only (under 1% of pixels in
    /// between, top mip). Alpha blending such a texture looks the same as
    /// masking it, and masking writes depth (see `material`).
    fn alpha_is_binary(&mut self, h: &ObjectHandle) -> bool {
        let key = h.path();
        if let Some(b) = self.binary_alpha_cache.get(&key) {
            return *b;
        }
        let result = (|| {
            let tex = read_texture(&h.package.pkg, h.export).ok()?;
            let palette = match tex.palette_ref {
                ObjectRef::Null => None,
                rf => self.set.resolve(&h.package, rf).and_then(|p| read_palette(&p.package.pkg, p.export).ok()),
            };
            let rgba = decode_rgba(tex.format, tex.mips.first()?, palette.as_deref())?;
            let n = rgba.len() / 4;
            let between = rgba.as_chunks::<4>().0.iter().filter(|p| (8..248).contains(&p[3])).count();
            Some(n > 0 && between * 100 < n)
        })()
        .unwrap_or(false);
        self.binary_alpha_cache.insert(key, result);
        result
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
                anisotropy_clamp: self.anisotropy,
                ..default()
            });
            Some((self.images.add(image), UVec2::new(w as u32, h0 as u32)))
        })();
        self.texture_cache.insert(key, result.clone());
        result.map(|t| t.0)
    }

    /// Colour from one texture, alpha from another (a Shader's Opacity),
    /// sampled nearest at the colour texture's size. Top mip only.
    fn texture_with_alpha(&mut self, color: &ObjectHandle, alpha: &ObjectHandle) -> Option<(Handle<Image>, UVec2)> {
        let key = format!("{}|alpha:{}", color.path(), alpha.path());
        if let Some(cached) = self.texture_cache.get(&key) {
            return cached.clone();
        }
        let result = (|| {
            let decode = |h: &ObjectHandle| -> Option<(Vec<u8>, usize, usize)> {
                let tex = read_texture(&h.package.pkg, h.export).ok()?;
                let palette = match tex.palette_ref {
                    ObjectRef::Null => None,
                    rf => self.set.resolve(&h.package, rf).and_then(|p| read_palette(&p.package.pkg, p.export).ok()),
                };
                let mip = tex.mips.first()?;
                Some((decode_rgba(tex.format, mip, palette.as_deref())?, mip.width, mip.height))
            };
            let (mut rgba, w, h) = decode(color)?;
            let (a, aw, ah) = decode(alpha)?;
            for y in 0..h {
                for x in 0..w {
                    let (ax, ay) = (x * aw / w, y * ah / h);
                    rgba[(y * w + x) * 4 + 3] = a[(ay * aw + ax) * 4 + 3];
                }
            }
            let mut image = Image::new(
                Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
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
            Some((self.images.add(image), UVec2::new(w as u32, h as u32)))
        })();
        self.texture_cache.insert(key, result.clone());
        result
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
        let mut simple: SimpleMaterial = resolve(self.set, from, rf);
        // A blended material whose alpha (the texture's, or the Shader's
        // Opacity texture's) is a cut-out (e.g. kf_generic_t.Generic_Gibbs
        // on KF-WestLondon's Clot gibs): masked, so the mesh hides its own
        // far side (blending writes no depth). Looks the same.
        if simple.blend == Blend::Translucent
            && let Some(t) = simple.opacity.clone().or_else(|| simple.texture.clone())
            && self.alpha_is_binary(&t)
        {
            simple.blend = Blend::Masked;
        }
        if !matches!(simple.blend, Blend::Opaque) {
            runlog::kv("material_see_through", &format!("material={} blend={:?} chain={}", from.package.pkg.object_path(rf), simple.blend, simple.chain.join(">")));
        }
        let result = if simple.blend == Blend::Invisible {
            None
        } else {
            let tex = match (&simple.texture, &simple.opacity) {
                (Some(t), Some(o)) => self.texture_with_alpha(t, o),
                (Some(t), None) => self.texture(t),
                _ => None,
            };
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
    /// Lightmap UVs (UV_1); left empty when not used.
    uvs1: Vec<[f32; 2]>,
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
        if !self.uvs1.is_empty() {
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, self.uvs1);
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
    mut door_setup: ResMut<crate::world::door::DoorSetup>,
    mut glass_setup: ResMut<crate::world::glass::GlassSetup>,
    game_options: Res<crate::game::waves::GameOptions>,
    compressed: Option<Res<CompressedImageFormatSupport>>,
    graphics: Option<Res<crate::engine::graphics::GraphicsSettings>>,
    mut baked_materials: ResMut<Assets<crate::render::baked::BakedMaterial>>,
    black_lightmap: Option<Res<crate::render::baked::BlackLightmap>>,
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
    log_level_actors(&lp.pkg);
    let contents = read_level_with(&lp, &class_defaults);
    door_setup.triggers = contents.use_triggers.clone();
    commands.insert_resource(crate::render::decals::MapProjectors(contents.projectors.clone()));
    log_map_features(&lp.pkg);
    commands.insert_resource(crate::player::pain::load(&lp, &class_defaults));
    if game_options.mode == crate::game::waves::GameMode::Waves {
        commands.insert_resource(crate::game::waves::load_game_data(&set, &class_defaults, &lp, game_options.length));
        commands.insert_resource(crate::game::trader::load_shops(&class_defaults, &lp));
        commands.insert_resource(crate::game::buy_menu::load_catalogue(&set, &class_defaults, &lp));
    }
    // Lights for actor lighting (render/actor_light.rs).
    commands.insert_resource(crate::render::actor_light::MapLightList(contents.lights.clone()));
    // Weapons, ammo boxes and vests lying in the map (every mode).
    commands.insert_resource(crate::game::pickups::load(&set, &class_defaults, &lp, &contents.pickups));
    // The trader woman in each shop (WeaponLocker): map content, every mode.
    crate::game::shopkeeper::spawn_shopkeepers(&mut commands, &set, &class_defaults, &lp, &mut meshes, &mut images, &mut materials);
    let mut nav = crate::world::nav::NavNetwork::from_graph(&ue_assets::nav::read_nav(&lp.pkg));
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
        binary_alpha_cache: HashMap::new(),
        material_cache: HashMap::new(),
        unlit_cache: HashMap::new(),
        baked_materials: &mut baked_materials,
        baked_cache: HashMap::new(),
        textures_uploaded: 0,
        texture_bytes: 0,
        textures_failed: 0,
        textures_compressed: 0,
        materials_without_texture: 0,
        bc_supported: compressed.is_some_and(|c| c.0.contains(CompressedImageFormats::BC)),
        anisotropy: graphics.map_or(crate::engine::graphics::DEFAULT_ANISOTROPY, |g| g.anisotropy),
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
                    let props = actor.and_then(|i| read_export_properties(&lp.pkg, i).ok());
                    let location = props.as_ref().and_then(|p| match p.get(&lp.pkg, "Location") {
                        Some(Value::Vector(v)) => Some(*v),
                        _ => None,
                    });
                    sky.camera_position = location.map(coords::pos);
                    let rotation = props
                        .as_ref()
                        .and_then(|p| match p.get(&lp.pkg, "Rotation") {
                            Some(Value::Rotator(r)) => Some(*r),
                            _ => None,
                        })
                        .unwrap_or_default();
                    sky.rotation = coords::rotation(rotation);
                    runlog::kv(
                        "sky_rotation",
                        &format!("pitch={} yaw={} roll={} (65536 = full turn)", rotation.pitch, rotation.yaw, rotation.roll),
                    );
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
                // KFSPLevelInfo.bUseVisionOverlay (only KF-Crash has one).
                let vision_overlay = lp
                    .pkg
                    .level_actor_exports()
                    .filter(|&i| lp.pkg.export_class_name(i) == "KFSPLevelInfo")
                    .filter_map(|i| read_export_properties(&lp.pkg, i).ok())
                    .all(|p| !matches!(p.get(&lp.pkg, "bUseVisionOverlay"), Some(Value::Bool(false))));
                // The sky view's fog: the sky zone's own (KF: no blending,
                // no volume fog, distances from the sky camera).
                sky.fog = sky_zone.as_ref().and_then(|(z, _)| zone_fog.get(*z as usize)).filter(|f| f.fog).cloned();
                if let Some((z, _)) = &sky_zone {
                    let f = zone_fog.get(*z as usize);
                    runlog::kv(
                        "sky_fog",
                        &format!(
                            "zone={z} name={} fog={} start={} end={} colour={},{},{}",
                            f.map_or("?", |f| f.name.as_str()),
                            f.is_some_and(|f| f.fog),
                            f.map_or(0.0, |f| f.start),
                            f.map_or(0.0, |f| f.end),
                            f.map_or(0, |f| f.color[0]),
                            f.map_or(0, |f| f.color[1]),
                            f.map_or(0, |f| f.color[2]),
                        ),
                    );
                }
                commands.insert_resource(crate::world::zones::Zones { bsp: model.clone(), zones: zone_fog, vision_overlay });
                // Baked lighting (lighting.rs): the render sections carry
                // each polygon's lightmap UVs and page.
                let bsp_lighting = match ue_assets::bsp::read_lighting(&lp.pkg, m, &model) {
                    Ok(l) => Some(l),
                    Err(e) => {
                        runlog::kv("bsp_lighting_error", &format!("error=\"{e}\""));
                        None
                    }
                };
                // Pages whose saved copy is out of date are built here the
                // way KF builds them at load (zone ambient + each light's
                // softened shadow bits x falloff x colour; lightmap_build.rs).
                let started = std::time::Instant::now();
                let mut build_stats = ue_assets::lightmap_build::BuildStats::default();
                let mut built_pages = Vec::new();
                let lightmap_pages: Vec<Option<Handle<Image>>> = bsp_lighting
                    .as_ref()
                    .map(|l| {
                        let stale = l.textures.iter().any(|t| !t.saved_is_current());
                        let build = stale.then(|| {
                            use ue_assets::lightmap_build as lb;
                            let lights = lb::bake_lights(&contents.lights, lb::level_brightness(&lp, &class_defaults));
                            let ambients = lb::zone_ambients(&lp, &class_defaults, &model);
                            (lights, ambients)
                        });
                        l.textures
                            .iter()
                            .enumerate()
                            .map(|(i, t)| match &build {
                                Some((lights, ambients)) if !t.saved_is_current() => {
                                    use ue_assets::lightmap_build as lb;
                                    let light_of = |rf: ObjectRef| lights.get(lp.pkg.object_name(rf)).cloned();
                                    let ambient_of = |z: usize| ambients.get(z).copied().unwrap_or([0; 3]);
                                    let rgba = lb::build_page(&model, l, i, &light_of, &ambient_of, &mut build_stats);
                                    let cover = lb::page_coverage(l, i);
                                    let n = cover.iter().filter(|&&c| c).count().max(1);
                                    let mean = rgba.as_chunks::<4>().0.iter().zip(&cover).filter(|(_, c)| **c).map(|(p, _)| (p[0] as f64 + p[1] as f64 + p[2] as f64) / 3.0).sum::<f64>() / n as f64;
                                    built_pages.push(format!("{i}:{mean:.1}"));
                                    Some(crate::render::lighting::rgba_lightmap_image(rgba, lb::PAGE as u32, lb::PAGE as u32, loader.images))
                                }
                                _ => crate::render::lighting::lightmap_image(t, loader.images),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if !built_pages.is_empty() {
                    runlog::kv(
                        "bsp_lightmaps_built",
                        &format!(
                            "pages={} mean_by_page=[{}] lightmaps={} lights_used={} lights_missing={} lights_black={} lights_bad_box={} effects_approximated={:?} ms={:.0}",
                            built_pages.len(),
                            built_pages.join(" "),
                            build_stats.lightmaps,
                            build_stats.lights,
                            build_stats.lights_missing,
                            build_stats.lights_black,
                            build_stats.lights_bad_box,
                            build_stats.effects_approximated,
                            started.elapsed().as_secs_f64() * 1000.0
                        ),
                    );
                }
                let (mut lightmapped_polys, mut no_lightmap_polys) = (0usize, 0usize);
                let mut lightmapped_materials: HashMap<AssetId<StandardMaterial>, Handle<StandardMaterial>> = HashMap::new();
                // Per (material, flags, lightmap page): material, in sky, page, mesh.
                type BspGroup = (Handle<StandardMaterial>, bool, Option<Handle<Image>>, MeshBuilder);
                let mut groups: HashMap<String, BspGroup> = HashMap::new();
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
                    // The polygon's lightmap page and UVs, if it has one
                    // (out-of-date pages are built above, so every lit
                    // polygon has one; there is no sun fallback).
                    let lit = (!unlit)
                        .then(|| {
                            let sec = bsp_lighting.as_ref()?.sections.get(usize::try_from(node.section).ok()?)?;
                            let page = lightmap_pages.get(usize::try_from(sec.lightmap_texture).ok()?)?.clone()?;
                            let first = usize::try_from(node.first_vertex).ok()?;
                            let uvs: Vec<[f32; 2]> = (0..node.num_verts).map(|k| sec.vertices.get(first + k).map(|v| v.lightmap_uv)).collect::<Option<_>>()?;
                            Some((sec.lightmap_texture, page, uvs))
                        })
                        .flatten();
                    let mat = match &lit {
                        Some(_) => {
                            lightmapped_polys += 1;
                            lightmapped_materials
                                .entry(mat.id())
                                .or_insert_with(|| {
                                    let m = loader.materials.get(&mat).cloned().unwrap_or_default();
                                    loader.materials.add(crate::render::lighting::lightmapped(&m))
                                })
                                .clone()
                        }
                        // No lightmap (sky, PF_Unlit, see-through surfaces
                        // UE2 does not lightmap): unlit.
                        None => {
                            no_lightmap_polys += 1;
                            loader.unlit(&mat)
                        }
                    };
                    let page = lit.as_ref().map(|l| l.0).unwrap_or(-1);
                    let key = format!("{:?}|{two_sided}|{in_sky}|{unlit}|{page}", surf.material);
                    let builder = &mut groups
                        .entry(key)
                        .or_insert_with(|| (mat, in_sky, lit.as_ref().map(|l| l.1.clone()), MeshBuilder::default()))
                        .3;
                    if let Some((_, _, uvs)) = &lit {
                        builder.uvs1.extend_from_slice(uvs);
                    }
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
                // Each carries its material's SurfaceType (cached per material).
                let mut bsp_surfaces: HashMap<String, u8> = HashMap::new();
                for (i, node) in model.nodes.iter().enumerate() {
                    let flags = model.surfs[node.surf].flags;
                    let in_sky = sky_zone.as_ref().is_some_and(|(z, _)| node.zone[1] == *z);
                    if flags & (NOT_SOLID | poly_flags::PORTAL) != 0 || in_sky {
                        continue;
                    }
                    let pts: Vec<Vec3> = model.node_polygon(i).map(coords::pos).collect();
                    let rf = model.surfs[node.surf].material;
                    let surface = *bsp_surfaces.entry(format!("{rf:?}")).or_insert_with(|| ue_assets::material::surface_type(&set, &level_handle, rf));
                    collision.bsp.surface = [surface, 0];
                    collision.bsp.push_polygon(&pts);
                    // Actor lighting's line checks: the same walls minus the
                    // sky backdrop, so sunlight reaches outdoor actors.
                    if flags & poly_flags::FAKE_BACKDROP == 0 {
                        collision.light_bsp.push_polygon(&pts);
                    }
                }
                runlog::kv(
                    "bsp_lightmaps",
                    &format!(
                        "pages={} pages_stale={} stale=[{}] pages_decoded={} surface_lightmaps={} polygons_lightmapped={lightmapped_polys} polygons_unlit={no_lightmap_polys} brightness={} lightmap_exposure={:.1}",
                        lightmap_pages.len(),
                        bsp_lighting.as_ref().map_or(0, |l| l.textures.iter().filter(|t| !t.saved_is_current()).count()),
                        bsp_lighting.as_ref().map_or(String::new(), |l| l
                            .textures
                            .iter()
                            .enumerate()
                            .filter(|(_, t)| !t.saved_is_current())
                            .map(|(i, t)| format!("{i}:{}/{}", t.saved_revision, t.revision))
                            .collect::<Vec<_>>()
                            .join(" ")),
                        lightmap_pages.iter().filter(|p| p.is_some()).count(),
                        bsp_lighting.as_ref().map_or(0, |l| l.lightmaps),
                        crate::render::lighting::BRIGHTNESS,
                        crate::render::lighting::lightmap_exposure()
                    ),
                );
                for (_, (mat, in_sky, page, builder)) in groups {
                    let mut e = commands.spawn((
                        Mesh3d(meshes.add(builder.build())),
                        MeshMaterial3d(mat),
                        Transform::IDENTITY,
                        MapGeometry,
                    ));
                    if in_sky {
                        e.insert(RenderLayers::layer(SKY_LAYER));
                    }
                    if let Some(image) = page {
                        e.insert(bevy::pbr::Lightmap { image, uv_rect: Rect::new(0.0, 0.0, 1.0, 1.0), bicubic_sampling: false });
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
        /// The section material's SurfaceType.
        surface: u8,
        mesh: Handle<Mesh>,
        /// The mesh's own material; `None` means invisible.
        material: Option<Handle<StandardMaterial>>,
        /// The mesh vertex each Bevy vertex came from (for baked colours).
        source: std::rc::Rc<Vec<u16>>,
    }
    // KF_BAKED_UNLIT=1 (for comparing): baked meshes drawn unlit as before
    // FL2 (no flashlight on them).
    let use_baked_material = std::env::var_os("KF_BAKED_UNLIT").is_none();
    let mut calibration = std::env::var_os("KF_LIGHT_CALIBRATE").map(|_| crate::render::actor_light::CalibrationSamples::default());
    let k_lin_cal = crate::render::lighting::brightness_linear();
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
    // Baked vertex colours (lighting.rs): per actor, its own copy of each
    // part's mesh with the colours, drawn unlit (texture x colour x K).
    let (mut meshes_baked, mut meshes_not_baked, mut baked_mismatch) = (0usize, 0usize, 0usize);
    let k_lin = crate::render::lighting::brightness_linear();
    let mut baked = |actor: &ue_assets::level::MeshActor, parts: &[Part], meshes: &mut Assets<Mesh>| -> Option<Vec<Handle<Mesh>>> {
        let colors = actor.instance.and_then(|e| ue_assets::lighting::read_mesh_instance_colors(&lp.pkg, e).ok());
        let Some(colors) = colors.filter(|c| !c.is_empty()) else {
            meshes_not_baked += 1;
            return None;
        };
        let lin: Vec<[f32; 4]> = colors
            .iter()
            .map(|c| {
                let l = Color::srgb_u8(c[0], c[1], c[2]).to_linear();
                [l.red * k_lin, l.green * k_lin, l.blue * k_lin, 1.0]
            })
            .collect();
        let mut out = Vec::new();
        for part in parts {
            let Some(cols) = part.source.iter().map(|&v| lin.get(v as usize).copied()).collect::<Option<Vec<_>>>() else {
                baked_mismatch += 1;
                return None;
            };
            let mut mesh = meshes.get(&part.mesh)?.clone();
            // Lightmap UVs for the black lightmap (render/baked.rs).
            let n = cols.len();
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, cols);
            mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.5f32, 0.5]; n]);
            out.push(meshes.add(mesh));
        }
        meshes_baked += 1;
        Some(out)
    };
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
                let surface = ue_assets::material::surface_type(&set, &h, rf);
                let mut b = MeshBuilder::default();
                let mut remap: HashMap<u16, u32> = HashMap::new();
                let mut source: Vec<u16> = Vec::new();
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
                            source.push(vi);
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
                    surface,
                    collision,
                    mesh: meshes.add(b.build()),
                    material: mat,
                    source: std::rc::Rc::new(source),
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
            let (pos, rot) = crate::world::door::key_pose(info, k);
            transform.translation = coords::pos(pos);
            transform.rotation = crate::world::door::rotation_of(rot);
            let mut soup = crate::world::collision::TriSoup::default();
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
            let lit = if actor.unlit { None } else { baked(actor, parts, &mut meshes) };
            for (pi, part) in parts.iter().enumerate() {
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
                match &lit {
                    // Baked colours plus the flashlight (render/baked.rs).
                    Some(lit) if use_baked_material => {
                        let swap = crate::render::baked::BakedSwap::new(loader.unlit(&material), loader.baked(&material), meshes.get(&lit[pi]));
                        commands.spawn((
                            Mesh3d(lit[pi].clone()),
                            MeshMaterial3d(swap.unlit.clone()),
                            swap,
                            Transform::IDENTITY,
                            ChildOf(root),
                        ));
                    }
                    Some(lit) => {
                        commands.spawn((Mesh3d(lit[pi].clone()), MeshMaterial3d(loader.unlit(&material)), Transform::IDENTITY, ChildOf(root)));
                    }
                    None => {
                        commands.spawn((Mesh3d(part.mesh.clone()), MeshMaterial3d(material), Transform::IDENTITY, ChildOf(root)));
                    }
                }
                entities += 1;
            }
            door_setup.doors.push(crate::world::door::DoorSpawn {
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
            if let Some(&skin) = actor.skins.first() {
                let m = resolve(&set, &ObjectHandle { package: lp.clone(), export: actor.export }, skin);
                runlog::kv("glass_pane_material", &format!("pane={} skin={} blend={:?} chain={}", info.name, lp.pkg.object_path(skin), m.blend, m.chain.join(">")));
            }
            let mut soup = crate::world::collision::TriSoup::default();
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
            let lit = if actor.unlit { None } else { baked(actor, parts, &mut meshes) };
            for (pi, part) in parts.iter().enumerate() {
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
                let (mesh, material) = match &lit {
                    Some(lit) => (lit[pi].clone(), loader.unlit(&material)),
                    None => (part.mesh.clone(), material),
                };
                let e = commands.spawn((Mesh3d(mesh), MeshMaterial3d(material), Transform::IDENTITY, ChildOf(root))).id();
                if part.section == 0 {
                    first_part = Some(e);
                }
                entities += 1;
            }
            glass_setup.panes.push(crate::world::glass::GlassSpawn {
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
                    // The actor's Skins[section] replaces the material.
                    let material_surface = match actor.skins.get(part.section) {
                        Some(&skin) if skin != ObjectRef::Null => {
                            ue_assets::material::surface_type(&set, &ObjectHandle { package: lp.clone(), export: actor.export }, skin)
                        }
                        _ => part.surface,
                    };
                    collision.meshes.surface = [material_surface, actor.surface_type];
                    for t in tris.iter() {
                        let [a, b, c] = t.map(|p| transform.transform_point(p));
                        collision.meshes.push_triangle(a, b, c);
                    }
                }
            }
        }
        // Static meshes that cast shadows (bShadowCast) block light for
        // actor lighting's line checks (render/actor_light.rs): their
        // collision triangles.
        if !in_sky && !is_mover && actor.shadow_cast {
            let first = collision.light_meshes.triangles.len() as u32;
            collision.light_mesh_owners.push((first, lp.pkg.object_name(ObjectRef::Export(actor.export)).to_string()));
            for part in parts.iter() {
                if let Some(tris) = &part.collision {
                    for t in tris.iter() {
                        let [a, b, c] = t.map(|p| transform.transform_point(p));
                        collision.light_meshes.push_triangle(a, b, c);
                    }
                }
            }
        }
        let map_handle = ObjectHandle {
            package: lp.clone(),
            export: actor.export,
        };
        let lit = if in_sky || actor.unlit { None } else { baked(actor, parts, &mut meshes) };
        // KF_LIGHT_CALIBRATE: sample baked vertices (render/actor_light.rs).
        if let (Some(samples), Some(lit)) = (calibration.as_mut(), lit.as_ref()) {
            samples.add_actor(&transform, lit, &meshes, k_lin_cal);
        }
        for (pi, part) in parts.iter().enumerate() {
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
            // Baked colours outside the sky: texture x colour, plus the
            // flashlight (render/baked.rs).
            if !in_sky && !actor.unlit && use_baked_material && let Some(lit) = &lit {
                let swap = crate::render::baked::BakedSwap::new(loader.unlit(&material), loader.baked(&material), meshes.get(&lit[pi]));
                commands.spawn((
                    Mesh3d(lit[pi].clone()),
                    MeshMaterial3d(swap.unlit.clone()),
                    swap,
                    transform,
                    MapGeometry,
                ));
                entities += 1;
                continue;
            }
            // bUnlit actors and everything in the sky zone: unlit.
            let mut material = if in_sky || actor.unlit || lit.is_some() { loader.unlit(&material) } else { material };
            let part_mesh = lit.as_ref().map_or_else(|| part.mesh.clone(), |l| l[pi].clone());
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
                Mesh3d(part_mesh),
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
    if let Some(samples) = calibration.take() {
        commands.insert_resource(samples);
    }
    let unique_ok = mesh_cache.values().filter(|p| p.is_some()).count();
    runlog::kv(
        "mesh_lighting",
        &format!("actors_baked={meshes_baked} actors_not_baked={meshes_not_baked} colour_count_mismatch={baked_mismatch} brightness={}", crate::render::lighting::BRIGHTNESS),
    );
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
            let blocks = crate::world::collision::volume_blocks(b.blocked_classes.as_deref(), b.blocks_traces);
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
    let terrain_black = black_lightmap.as_deref().cloned();
    spawn_terrains(&mut commands, &mut meshes, &mut loader, &set, &lp, sky_zone_index, terrain_black.as_ref(), &mut collision.terrain);

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
            "uploaded={} compressed={} bc_supported={} failed={} megabytes={:.1} materials={} materials_without_texture={} anisotropy={}",
            loader.textures_uploaded,
            loader.textures_compressed,
            loader.bc_supported,
            loader.textures_failed,
            loader.texture_bytes as f64 / 1e6,
            loader.material_cache.len(),
            loader.materials_without_texture,
            loader.anisotropy
        ),
    );

    // --- Lighting: no baked lightmaps yet, so a sun plus ambient light ---
    commands.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 600.0,
        // Lightmapped BSP takes its light from the lightmap only.
        affects_lightmapped_meshes: false,
    });
    commands.spawn((
        DirectionalLight {
            illuminance: 4000.0,
            affects_lightmapped_mesh_diffuse: false,
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
    commands.insert_resource(PlayerStarts(
        contents
            .player_starts
            .iter()
            .map(|ps| StartSpot { location: Vec3::from_array(ps.location), yaw: ps.rotation.yaw, enabled: ps.enabled, primary: ps.primary })
            .collect(),
    ));

    let level_name = level_title(&lp);
    commands.insert_resource(LevelTitle(level_name.clone()));
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
    pkg.level_actor_exports()
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
///
/// Light: KF draws terrain as texture x the vertex light colour stored in
/// the map x 2 (plus dynamic lights such as the flashlight), the same rule
/// as baked placed meshes, so terrain uses the baked-mesh material
/// (render/baked.rs) with the colour carried in the vertex colour and the
/// layer weight in the vertex alpha. Only if the stored colours cannot be
/// read does it fall back to the sun.
///
/// Fog: KF alpha-blends each fogged layer over the one below, so the fog
/// colour counts once. With additive layers that means: layer 0 is fogged
/// normally, the added layers fade toward black with distance (their share
/// of the fog colour is already in layer 0's), which gives the same sum.
#[allow(clippy::too_many_arguments)]
fn spawn_terrains(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    loader: &mut Loader,
    set: &PackageSet,
    lp: &std::rc::Rc<ue_assets::package_set::LoadedPackage>,
    sky_zone: Option<u8>,
    black_lightmap: Option<&crate::render::baked::BlackLightmap>,
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

        // Layer alpha at each vertex, sampled from the layer's AlphaMap.
        let alphas: Vec<Vec<f32>> = t
            .layers
            .iter()
            .map(|layer| layer_alpha(set, &handle, layer.alpha_map, &t))
            .collect();
        let n_layers = t.layers.len();
        let weights = layer_weights(&alphas, w * h);

        // Collision, each triangle with the SurfaceType of the layer that
        // shows most at its corners (a guess: what a trace on terrain
        // returns as HitMaterial is native code).
        if !in_sky {
            let layer_surfaces: Vec<u8> = t.layers.iter().map(|l| ue_assets::material::surface_type(set, &handle, l.texture)).collect();
            for tri in &tris {
                let top = (0..n_layers).max_by(|&a, &b| {
                    let sum = |li: usize| tri.iter().map(|&v| weights[li][v]).sum::<f32>();
                    sum(a).total_cmp(&sum(b))
                });
                collision.surface = [top.map_or(0, |li| layer_surfaces[li]), 0];
                collision.push_triangle(pos[tri[0]], pos[tri[1]], pos[tri[2]]);
            }
        }

        // Stored vertex light x K, in linear light (as baked meshes).
        let k_lin = crate::render::lighting::brightness_linear();
        let light: Option<Vec<[f32; 3]>> = (!t.vertex_light.is_empty()).then(|| {
            t.vertex_light
                .iter()
                .map(|c| {
                    let l = Color::srgb_u8(c[0], c[1], c[2]).to_linear();
                    [l.red * k_lin, l.green * k_lin, l.blue * k_lin]
                })
                .collect()
        });
        log_terrain_light(ti, &t, &tris);

        // Stored light outside the sky: the baked-mesh material, always lit
        // (flashlight), with the terrain fog rule (see the function note).
        // KF_BAKED_UNLIT=1 (for comparing): plain unlit layers instead.
        let terrain_lit = light.is_some() && !in_sky && black_lightmap.is_some() && std::env::var_os("KF_BAKED_UNLIT").is_none();
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
                        let l = light.as_ref().map_or([1.0; 3], |l| l[v]);
                        // Layer 0 is opaque, scaled by its weight (in the colour for the
                        // plain material, in the alpha for the terrain-lit one). Others:
                        // additive, scaled by alpha.
                        b.colors.push(if li == 0 && !terrain_lit { [l[0] * wgt, l[1] * wgt, l[2] * wgt, 1.0] } else { [l[0], l[1], l[2], wgt] });
                        if light.is_some() {
                            // Lightmap UVs for the black lightmap (render/baked.rs).
                            b.uvs1.push([0.5, 0.5]);
                        }
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
                unlit: in_sky || light.is_some(),
                ..default()
            });
            let mesh = meshes.add(b.build());
            let mut e = commands.spawn((Mesh3d(mesh), Transform::IDENTITY, MapGeometry));
            match black_lightmap.filter(|_| terrain_lit) {
                Some(black) => {
                    let mut lit = crate::render::baked::baked_material(loader.materials.get(&material).expect("just added"));
                    lit.extension.flags = if li == 0 { crate::render::baked::TERRAIN_WEIGHTED } else { crate::render::baked::TERRAIN_FOG_BLACK };
                    e.insert((MeshMaterial3d(loader.baked_materials.add(lit)), crate::render::baked::black_lightmap(black)));
                }
                None => {
                    e.insert(MeshMaterial3d(material));
                }
            }
            if in_sky {
                e.insert(RenderLayers::layer(SKY_LAYER));
            }
            layers_drawn += 1;
        }
        runlog::kv(
            "terrain_loaded",
            &format!(
                "terrain={ti} heightmap={w}x{h} visible_triangles={} layers={n_layers} layers_drawn={layers_drawn} \
                 layer_triangles={layer_tris} in_sky={in_sky} lit_by_stored_light={terrain_lit} inverted={} seconds={:.2}",
                tris.len(),
                t.inverted,
                started.elapsed().as_secs_f64()
            ),
        );
    }
}

/// Effective weight of each terrain layer at each vertex: a_i x product
/// over the layers above (j > i) of (1 - a_j); layer 0 counts as fully
/// covering, so the weights at a vertex sum to 1. Drawing layer 0 opaque
/// and adding the others scaled by these weights gives the same colour as
/// KF's layer-over-layer alpha blending.
fn layer_weights(alphas: &[Vec<f32>], vertices: usize) -> Vec<Vec<f32>> {
    let n_layers = alphas.len();
    let mut weights = vec![vec![0f32; vertices]; n_layers];
    for v in 0..vertices {
        let mut remaining = 1.0f32;
        for i in (0..n_layers).rev() {
            let a = if i == 0 { 1.0 } else { alphas[i][v] };
            weights[i][v] = a * remaining;
            remaining *= 1.0 - a;
        }
    }
    weights
}

/// Log the terrain's stored vertex light: how many colours the map stored,
/// how many vertices there are, and the mean colour over the vertices of
/// drawn triangles (`terrain_light`), or `terrain_light=missing` when the
/// colours could not be read (then the terrain stays sun-lit).
fn log_terrain_light(ti: usize, t: &Terrain, tris: &[[usize; 3]]) {
    let vertices = t.width * t.height;
    if t.vertex_light.is_empty() {
        runlog::kv("terrain_light", &format!("terrain={ti} missing vertices={vertices} fallback=sun"));
        return;
    }
    let mut used = vec![false; vertices];
    for &v in tris.iter().flatten() {
        used[v] = true;
    }
    let (mut sum, mut n, mut nonblack) = ([0u64; 3], 0u64, 0usize);
    for (c, _) in t.vertex_light.iter().zip(&used).filter(|(_, u)| **u) {
        for i in 0..3 {
            sum[i] += c[i] as u64;
        }
        n += 1;
        nonblack += c.iter().any(|&x| x > 0) as usize;
    }
    let mean = sum.map(|s| s as f64 / n.max(1) as f64);
    runlog::kv(
        "terrain_light",
        &format!(
            "terrain={ti} stored={} vertices={vertices} drawn_vertices={n} nonblack={nonblack} mean_rgb={:.1},{:.1},{:.1}",
            t.vertex_light_stored, mean[0], mean[1], mean[2]
        ),
    );
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
    fn terrain_layer_weights_match_blending_and_sum_to_one() {
        // Three layers, alphas (ignored for layer 0) 0.5 and 0.25 at one vertex.
        let alphas = vec![vec![0.0], vec![0.5], vec![0.25]];
        let w = layer_weights(&alphas, 1);
        let sum: f32 = w.iter().map(|l| l[0]).sum();
        assert!((sum - 1.0).abs() < 1e-6);
        // Layer-over-layer blending of colours 10, 20, 40 (layer 0 first).
        let (c0, c1, c2) = (10.0f32, 20.0, 40.0);
        let blended = (c0 * (1.0 - 0.5) + c1 * 0.5) * (1.0 - 0.25) + c2 * 0.25;
        let weighted = c0 * w[0][0] + c1 * w[1][0] + c2 * w[2][0];
        assert!((blended - weighted).abs() < 1e-4);
        // Fog once: each blended layer fogged as mix(c, fog, 1 - f) equals
        // layer 0 fogged normally plus the others faded toward black.
        let (f, fog) = (0.3f32, 100.0f32);
        let fog_each = |c: f32| c * f + fog * (1.0 - f);
        let kf = (fog_each(c0) * 0.5 + fog_each(c1) * 0.5) * 0.75 + fog_each(c2) * 0.25;
        let ours = (c0 * w[0][0] * f + fog * (1.0 - f)) + c1 * w[1][0] * f + c2 * w[2][0] * f;
        assert!((kf - ours).abs() < 1e-3, "{kf} vs {ours}");
    }

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
