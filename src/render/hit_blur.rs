//! E3: the hit blur (KFPlayerController.SetBlur -> PostFX blur pass). The
//! amount (0..1) comes from player/hit_cam.rs; the filter itself is native
//! code in KF (Red Orchestra's PostFX), not in the scripts, so this one is
//! a guess: the frame mixed with a ring-blurred copy of itself, both the
//! mix and the blur radius growing with the amount (at 1: 1.1% of the
//! screen height). See DESIGN.md, "Hit effects", E3.
//!
//! Drawn as a Bevy full-screen pass on the weapon camera, the last 3D
//! camera on the window: it blurs the sky, scene and weapon, before the
//! vision overlay, the trader arrow and the HUD.

use bevy::core_pipeline::fullscreen_material::{FullscreenMaterial, FullscreenMaterialPlugin};
use bevy::prelude::*;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::ShaderType;
use bevy::shader::ShaderRef;

use crate::engine::runlog;

/// Blur radius at amount 1, as a share of the screen height (a guess).
const MAX_RADIUS: f32 = 0.011;

/// The pass's settings, on the weapon camera while the blur is on.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Default)]
pub struct HitBlurPass {
    /// Blur radius in UV units (x, y).
    step: Vec2,
    amount: f32,
}

const HIT_BLUR_SHADER: Handle<bevy::shader::Shader> = bevy::asset::uuid_handle!("5d1f3a7c-2e4b-4c6d-9f8a-0b1c2d3e4f5a");

const HIT_BLUR_WGSL: &str = r#"
#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

struct HitBlur {
    step: vec2<f32>,
    amount: f32,
}

@group(0) @binding(0) var screen: texture_2d<f32>;
@group(0) @binding(1) var screen_sampler: sampler;
@group(0) @binding(2) var<uniform> blur: HitBlur;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let base = textureSample(screen, screen_sampler, in.uv);
    // 12 taps on two rings (full and half radius), plus the centre.
    var sum = base;
    for (var i = 0; i < 12; i++) {
        let a = f32(i) * 0.5235988;
        let r = select(1.0, 0.5, i % 2 == 1);
        sum += textureSample(screen, screen_sampler, in.uv + vec2<f32>(cos(a), sin(a)) * blur.step * r);
    }
    return mix(base, sum / 13.0, clamp(blur.amount, 0.0, 1.0));
}
"#;

impl FullscreenMaterial for HitBlurPass {
    fn fragment_shader() -> ShaderRef {
        HIT_BLUR_SHADER.into()
    }
}

pub struct HitBlurPlugin;

impl Plugin for HitBlurPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FullscreenMaterialPlugin::<HitBlurPass>::default())
            .add_systems(PostUpdate, sync_hit_blur);
        app.world_mut()
            .resource_mut::<Assets<bevy::shader::Shader>>()
            .insert(&HIT_BLUR_SHADER, bevy::shader::Shader::from_wgsl(HIT_BLUR_WGSL, "hit_blur.rs/hit_blur.wgsl"))
            .expect("shader handle");
    }
}

/// Puts the pass on the weapon camera while the blur amount is over 0,
/// takes it off (no pass at all) when it is 0.
fn sync_hit_blur(
    mut commands: Commands,
    cam: Res<crate::player::hit_cam::HitCam>,
    window: Query<&Window>,
    cameras: Query<(Entity, Option<&HitBlurPass>), With<crate::weapons::weapon::WeaponCamera>>,
) {
    let amount = cam.blur.amount.clamp(0.0, 1.0);
    let Ok(win) = window.single() else { return };
    let aspect = win.height() / win.width().max(1.0);
    for (e, pass) in &cameras {
        if amount > 0.0 {
            let radius = amount * MAX_RADIUS;
            commands.entity(e).insert(HitBlurPass { step: Vec2::new(radius * aspect, radius), amount });
            if pass.is_none() {
                runlog::kv("hit_blur_pass", &format!("on=true amount={amount:.3}"));
            }
        } else if pass.is_some() {
            commands.entity(e).remove::<HitBlurPass>();
            runlog::kv("hit_blur_pass", "on=false");
        }
    }
}
