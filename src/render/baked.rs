//! The material for placed meshes with baked vertex colours (DESIGN.md,
//! "Weapon flashlights", FL2): they look exactly as before (texture x
//! baked colour x K, the colour carried as vertex colours) and on top Bevy's
//! dynamic lights (the flashlight's spot and glow) light the texture, as
//! they already light the lightmapped BSP.
//!
//! The lit material costs about 4 ms a frame when every baked mesh uses
//! it (KF-WestLondon, headless), so a baked mesh only switches to it
//! while a dynamic light (the flashlight's spot or glow) can reach it, and
//! back to the plain unlit material afterwards (`swap_baked`).
//!
//! How: Bevy's StandardMaterial, lit, extended by a small fragment shader.
//! The mesh gets a `Lightmap` with a black 1 x 1 image, which tells the sun
//! and the ambient light to leave it alone (as for the BSP); the shader
//! computes the standard lighting on the plain texture (so only point and
//! spot lights count) and adds the baked colour on top.

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::MeshAabb;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;

pub type BakedMaterial = ExtendedMaterial<StandardMaterial, BakedExt>;

/// No settings of its own (the uniform keeps the bind group non-empty).
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub struct BakedExt {
    #[uniform(100)]
    pub unused: u32,
}

const BAKED_SHADER: Handle<bevy::shader::Shader> = bevy::asset::uuid_handle!("2c7e5a91-4f0b-4d6a-8e13-9b5c0d7f3a21");

/// `pbr_input_from_standard_material` multiplies the base colour by the
/// vertex colour, so it is called with the vertex colour set to white to get
/// the plain texture, and the baked colour is applied by hand.
const BAKED_WGSL: &str = r#"
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::{apply_pbr_lighting, main_pass_post_lighting_processing},
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var plain = in;
#ifdef VERTEX_COLORS
    plain.color = vec4<f32>(1.0);
#endif
    var pbr_input = pbr_input_from_standard_material(plain, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
#ifdef VERTEX_COLORS
    let baked = pbr_input.material.base_color.rgb * in.color.rgb;
    // Vertex alpha: 1 on baked meshes; a layer's weight on terrain.
    let alpha = pbr_input.material.base_color.a * in.color.a;
#else
    let baked = pbr_input.material.base_color.rgb;
    let alpha = pbr_input.material.base_color.a;
#endif
    var out: FragmentOutput;
    let dynamic = apply_pbr_lighting(pbr_input);
    out.color = vec4<f32>(baked + dynamic.rgb, alpha);
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
"#;

impl MaterialExtension for BakedExt {
    fn fragment_shader() -> ShaderRef {
        BAKED_SHADER.into()
    }
}

/// A black 1 x 1 lightmap for baked meshes (see the module note).
#[derive(Resource, Clone)]
pub struct BlackLightmap(pub Handle<Image>);

/// A baked mesh's two materials and its bounds (local space), for
/// switching to the lit one while a dynamic light can reach it.
#[derive(Component, Clone)]
pub struct BakedSwap {
    pub unlit: Handle<StandardMaterial>,
    pub lit: Handle<BakedMaterial>,
    centre: Vec3,
    radius: f32,
    lit_now: bool,
}

impl BakedSwap {
    pub fn new(unlit: Handle<StandardMaterial>, lit: Handle<BakedMaterial>, mesh: Option<&Mesh>) -> Self {
        let (centre, radius) = mesh
            .and_then(|m| m.compute_aabb())
            .map_or((Vec3::ZERO, 1.0e6), |b| (Vec3::from(b.center), Vec3::from(b.half_extents).length()));
        BakedSwap { unlit, lit, centre, radius, lit_now: false }
    }
}

/// Every frame: baked meshes within reach of a dynamic light (their
/// bounding sphere touches the light's) get the lit material, the others
/// the unlit one. Log `baked_swap` when the count changes.
fn swap_baked(
    mut commands: Commands,
    dynamic: Res<crate::render::actor_light::DynamicLights>,
    mut meshes: Query<(Entity, &GlobalTransform, &mut BakedSwap)>,
    black: Res<BlackLightmap>,
    mut last: Local<usize>,
) {
    let lights: Vec<(Vec3, f32)> = dynamic.0.iter().map(|s| (s.pos, s.radius)).collect();
    let mut lit_count = 0;
    for (e, gt, mut swap) in &mut meshes {
        let scale = gt.compute_transform().scale.abs().max_element();
        let centre = gt.transform_point(swap.centre);
        let r = swap.radius * scale;
        let want = lights.iter().any(|(p, lr)| p.distance(centre) < lr + r);
        lit_count += want as usize;
        if want == swap.lit_now {
            continue;
        }
        swap.lit_now = want;
        // The black lightmap only while lit (on every mesh it cost about
        // 5 ms a frame even with the unlit material).
        if want {
            commands
                .entity(e)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert((MeshMaterial3d(swap.lit.clone()), black_lightmap(&black)));
        } else {
            commands
                .entity(e)
                .remove::<(MeshMaterial3d<BakedMaterial>, bevy::pbr::Lightmap)>()
                .insert(MeshMaterial3d(swap.unlit.clone()));
        }
    }
    if lit_count != *last {
        *last = lit_count;
        crate::engine::runlog::kv("baked_swap", &format!("lit_meshes={lit_count} dynamic_lights={}", lights.len()));
    }
}

pub struct BakedPlugin;

impl Plugin for BakedPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<BakedMaterial>::default()).add_systems(PostUpdate, swap_baked);
        app.world_mut()
            .resource_mut::<Assets<bevy::shader::Shader>>()
            .insert(&BAKED_SHADER, bevy::shader::Shader::from_wgsl(BAKED_WGSL, "baked.rs/baked.wgsl"))
            .expect("shader handle");
        let image = Image::new(
            Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            TextureDimension::D2,
            vec![0, 0, 0, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        let handle = app.world_mut().resource_mut::<Assets<Image>>().add(image);
        app.insert_resource(BlackLightmap(handle));
    }
}

/// The baked-mesh material for an unlit-style material: the same texture,
/// alpha and sides, lit (so dynamic lights apply) with no specular shine.
pub fn baked_material(m: &StandardMaterial) -> BakedMaterial {
    ExtendedMaterial {
        base: StandardMaterial {
            unlit: false,
            reflectance: 0.0,
            metallic: 0.0,
            perceptual_roughness: 1.0,
            ..m.clone()
        },
        extension: BakedExt::default(),
    }
}

/// The black lightmap component for a baked mesh.
pub fn black_lightmap(image: &BlackLightmap) -> bevy::pbr::Lightmap {
    bevy::pbr::Lightmap { image: image.0.clone(), uv_rect: Rect::new(0.0, 0.0, 1.0, 1.0), bicubic_sampling: false }
}
