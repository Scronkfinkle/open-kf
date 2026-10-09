//! Reflective map materials: a texture blended with an environment map
//! (a cubemap: six pictures of the surroundings, one per side of a cube)
//! by a mask's alpha, the way KF's Combiner CO_AlphaBlend_With_Mask draws
//! it (ue-assets `material::EnvBlend`). Used by KF-Manor's lake
//! (`ManorWaterFB`), which before this was drawn as its ripple texture
//! alone, see-through, without any reflection.
//!
//! Per pixel:
//! - the view direction is reflected about the surface normal and turned
//!   into Unreal's axes (world space; camera space for EM_CameraSpace) to
//!   pick the cubemap colour;
//! - colour = Material2 x a + Material1 x (1 - a), a = the mask texture's
//!   alpha (1 - alpha with InvertMask);
//! - times the baked vertex colour (KF lights the whole combined colour, as
//!   any diffuse colour), alpha from the material's own texture (a Shader's
//!   Opacity), then fog.
//!
//! Drawn as Bevy's StandardMaterial (unlit, so its texture sampling,
//! alpha mode and fog apply) extended by the fragment shader below.

use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

pub type EnvMaterial = ExtendedMaterial<StandardMaterial, EnvExt>;

/// The environment map is Material2 (else Material1).
pub const ENV_IS_MATERIAL2: u32 = 1;
/// Combiner InvertMask: weight by 1 - alpha.
pub const ENV_INVERT_MASK: u32 = 2;
/// TexEnvMap EM_CameraSpace: look up in camera space instead of world space.
pub const ENV_CAMERA_SPACE: u32 = 4;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct EnvExt {
    #[uniform(100)]
    pub flags: u32,
    /// Six layers (+X, -X, +Y, -Y, +Z, -Z in Unreal's axes), viewed as a cube.
    #[texture(101, dimension = "cube")]
    #[sampler(102)]
    pub cubemap: Handle<Image>,
    /// The texture whose alpha weights the blend, on the mesh's first UVs.
    #[texture(103)]
    #[sampler(104)]
    pub mask: Handle<Image>,
}

const ENV_SHADER: Handle<bevy::shader::Shader> = bevy::asset::uuid_handle!("8f3d2b6e-1c4a-4e7f-9a25-6d0b3c8e5f14");

const ENV_WGSL: &str = r#"
#import bevy_pbr::{
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::alpha_discard,
    forward_io::{VertexOutput, FragmentOutput},
    pbr_functions::main_pass_post_lighting_processing,
    mesh_view_bindings as view_bindings,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(100) var<uniform> env_flags: u32;
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var env_cube: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var env_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var mask_texture: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var mask_sampler: sampler;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var plain = in;
#ifdef VERTEX_COLORS
    plain.color = vec4<f32>(1.0);
#endif
    var pbr_input = pbr_input_from_standard_material(plain, is_front);
    pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    let base = pbr_input.material.base_color;

    // Reflection vector, Bevy world space.
    var n = normalize(in.world_normal);
    if !is_front {
        n = -n;
    }
    let view_dir = normalize(in.world_position.xyz - view_bindings::view.world_position);
    let r = reflect(view_dir, n);
    // Bevy (x, y, z) = (Unreal Y, Unreal Z, -Unreal X) (engine/coords.rs).
    var lookup = vec3<f32>(-r.z, r.x, r.y);
    if (env_flags & 4u) != 0u {
        // Camera space as Direct3D has it: x right, y up, z forward.
        let v = (view_bindings::view.view_from_world * vec4<f32>(r, 0.0)).xyz;
        lookup = vec3<f32>(v.x, v.y, -v.z);
    }
    let env = textureSample(env_cube, env_sampler, lookup).rgb;

#ifdef VERTEX_UVS_A
    var a = textureSample(mask_texture, mask_sampler, in.uv).a;
#else
    var a = 1.0;
#endif
    if (env_flags & 2u) != 0u {
        a = 1.0 - a;
    }
    var m1 = env;
    var m2 = base.rgb;
    if (env_flags & 1u) != 0u {
        m1 = base.rgb;
        m2 = env;
    }
    var colour = m2 * a + m1 * (1.0 - a);
#ifdef VERTEX_COLORS
    // Baked light (already scaled by the map brightness).
    colour = colour * in.color.rgb;
#endif
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(pbr_input, vec4<f32>(colour, base.a));
    return out;
}
"#;

impl MaterialExtension for EnvExt {
    fn fragment_shader() -> ShaderRef {
        ENV_SHADER.into()
    }
}

pub struct EnvMapPlugin;

impl Plugin for EnvMapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<EnvMaterial>::default());
        app.world_mut()
            .resource_mut::<Assets<bevy::shader::Shader>>()
            .insert(&ENV_SHADER, bevy::shader::Shader::from_wgsl(ENV_WGSL, "env_map.rs/env_map.wgsl"))
            .expect("shader handle");
    }
}

/// The flags for `EnvExt` from the material's reflection blend.
pub fn flags(env: &ue_assets::material::EnvBlend) -> u32 {
    (if env.env_is_material2 { ENV_IS_MATERIAL2 } else { 0 })
        | (if env.invert_mask { ENV_INVERT_MASK } else { 0 })
        | (if env.camera_space { ENV_CAMERA_SPACE } else { 0 })
}
