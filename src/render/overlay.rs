//! KF's vision overlay (HUDKillingFloor.DrawModOverlay): every frame the
//! whole view is multiplied by 2 x tint / 255, the tint following the
//! player zone's fog colour. This is KF-WestLondon's orange look. See
//! DESIGN.md, "Baked lighting", LV. Below 25% health the overlay is
//! NearDeathOverlay instead, a red pulse (E4, DESIGN.md "Hit effects").
//!
//! The brightness setting (`--brightness`, engine/graphics.rs) is a second
//! quad of the same kind: the view times PERCENT / 100, in linear light.

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, ScalingMode};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState, CompareFunction};

use crate::engine::runlog;

/// Render layer of the overlay quad (its own camera, drawn last).
const OVERLAY_LAYER: usize = 2;

pub struct OverlayPlugin;

impl Plugin for OverlayPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<OverlayMaterial>::default())
            .add_systems(Startup, spawn_overlay)
            .add_systems(Update, update_overlay);
        app.world_mut()
            .resource_mut::<Assets<bevy::shader::Shader>>()
            .insert(&OVERLAY_SHADER, bevy::shader::Shader::from_wgsl(OVERLAY_WGSL, "overlay.rs/overlay.wgsl"))
            .expect("shader handle");
    }
}

/// A flat colour blended as 2 x source x screen (UE2's modulate: mid-grey
/// changes nothing, white doubles). Blending happens in linear light, so
/// the colour is the gamma-space factor raised to 2.2, halved.
#[derive(Asset, TypePath, AsBindGroup, Clone)]
pub struct OverlayMaterial {
    #[uniform(0)]
    color: LinearRgba,
}

const OVERLAY_SHADER: Handle<bevy::shader::Shader> = bevy::asset::uuid_handle!("8a2c4e6f-1b3d-4f5a-8c7e-9d0b1a2c3e4f");

const OVERLAY_WGSL: &str = r#"
#import bevy_pbr::forward_io::VertexOutput

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> overlay_color: vec4<f32>;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    return overlay_color;
}
"#;

impl Material for OverlayMaterial {
    fn fragment_shader() -> bevy::shader::ShaderRef {
        OVERLAY_SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    /// result = source x screen + screen x source; always drawn, no depth.
    fn specialize(
        _pipeline: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _layout: &bevy::mesh::MeshVertexBufferLayoutRef,
        _key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(BlendState {
                    color: BlendComponent { src_factor: BlendFactor::Dst, dst_factor: BlendFactor::Src, operation: BlendOperation::Add },
                    alpha: BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
                });
            }
        }
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_compare = Some(CompareFunction::Always);
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

#[derive(Component)]
struct Overlay;

/// HUDKillingFloor's LastR / LastG / LastB (start black: KF's fade-in) and
/// the colour it eases toward.
#[derive(Default)]
struct Tint {
    last: [f32; 3],
    target: Option<[f32; 3]>,
}

/// The brightness quad's colour: the blend doubles, so half the factor
/// (`--brightness` is 50-200 %, so 0.25-1.0).
fn brightness_color(percent: u32) -> LinearRgba {
    let half = (percent as f32 / 100.0 / 2.0).clamp(0.0, 1.0);
    LinearRgba::rgb(half, half, half)
}

fn spawn_overlay(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<OverlayMaterial>>,
    graphics: Option<Res<crate::engine::graphics::GraphicsSettings>>,
) {
    let layer = RenderLayers::layer(OVERLAY_LAYER);
    commands.spawn((
        Camera3d::default(),
        // After the scene (0) and the weapon camera (1): KF tints the weapon too.
        Camera { order: 2, clear_color: ClearColorConfig::None, ..default() },
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::Fixed { width: 2.0, height: 2.0 },
            ..OrthographicProjection::default_3d()
        }),
        Tonemapping::None,
        Transform::IDENTITY,
        layer.clone(),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(4.0, 4.0))),
        MeshMaterial3d(materials.add(OverlayMaterial { color: LinearRgba::rgb(0.5, 0.5, 0.5) })),
        Transform::from_xyz(0.0, 0.0, -1.0),
        layer.clone(),
        Overlay,
    ));
    // Brightness: only drawn when not 100 % (then nothing changes at all).
    let percent = graphics.map_or(crate::engine::graphics::DEFAULT_BRIGHTNESS, |g| g.brightness);
    if percent != crate::engine::graphics::DEFAULT_BRIGHTNESS {
        commands.spawn((
            Mesh3d(meshes.add(Rectangle::new(4.0, 4.0))),
            MeshMaterial3d(materials.add(OverlayMaterial { color: brightness_color(percent) })),
            Transform::from_xyz(0.0, 0.0, -1.0),
            layer,
        ));
    }
    let c = brightness_color(percent);
    runlog::kv("brightness_overlay", &format!("percent={percent} drawn={} quad_colour={:.3} light_factor={:.2}", percent != crate::engine::graphics::DEFAULT_BRIGHTNESS, c.red, c.red * 2.0));
}

/// KFX.NearDeathShader -> DeSat, a looping MaterialSequence (TotalTime
/// 1.5): fade to InjuredGrain over 0.5 s, then to Grain1 over 1.0 s
/// (SequenceItems decoded from `kfpkg raw KFX.utx DeSat`). DrawModOverlay
/// draws it with DrawTileScaled(Mat, SizeX, SizeY): scale factors, so the
/// tile is SizeX times the texture and only its top-left texel shows (KF's
/// own film grain passes ClipX / 1024 to fill the screen). Those texels,
/// read from the decoded textures: InjuredGrain (255, 13, 13), Grain1
/// (174, 172, 174). The sepia look's texel is white (Grain2 x Grain2).
const INJURED_TEXEL: [f32; 3] = [1.0, 13.0 / 255.0, 13.0 / 255.0];
const GRAIN1_TEXEL: [f32; 3] = [174.0 / 255.0, 172.0 / 255.0, 174.0 / 255.0];

/// DeSat's colour at game time `t`. MaterialSequence's fade is native:
/// taken as a straight blend from the previous item (a guess), the
/// sequence running on the game clock.
fn near_death_texel(t: f32) -> [f32; 3] {
    let s = t.rem_euclid(1.5);
    let (from, to, k) = if s < 0.5 { (GRAIN1_TEXEL, INJURED_TEXEL, s / 0.5) } else { (INJURED_TEXEL, GRAIN1_TEXEL, (s - 0.5) / 1.0) };
    [0, 1, 2].map(|i| from[i] + (to[i] - from[i]) * k)
}

#[allow(clippy::too_many_arguments)]
fn update_overlay(
    time: Res<Time>,
    health: Res<crate::game::combat::PlayerHealth>,
    mut near_death_was: Local<bool>,
    zones: Option<Res<crate::world::zones::Zones>>,
    player: Res<crate::world::zones::PlayerZone>,
    overlay: Query<&MeshMaterial3d<OverlayMaterial>, With<Overlay>>,
    mut materials: ResMut<Assets<OverlayMaterial>>,
    mut tint: Local<Tint>,
) {
    let Ok(handle) = overlay.single() else { return };
    let Some(zones) = zones else { return };
    let Some(mut mat) = materials.get_mut(&handle.0) else { return };
    if !zones.vision_overlay {
        // No overlay: 2 x 0.5 = no change.
        mat.color = LinearRgba::rgb(0.5, 0.5, 0.5);
        return;
    }
    // The player zone's colour, if it has fog and takes part.
    if let Some(z) = zones.zones.get(player.zone)
        && z.fog
        && let Some(c) = z.overlay
    {
        let t = c.map(|v| v as f32);
        if tint.target != Some(t) {
            runlog::kv("vision_overlay", &format!("zone={} name={} colour={c:?}", player.zone, z.name));
            tint.target = Some(t);
        }
    }
    // No tint yet (the zone has no fog or opts out with
    // bNoKFColorCorrection): DrawModOverlay returns before drawing, so the
    // view is unchanged (2 x 0.5). Was left black: KF-Farm's start zone.
    let Some(target) = tint.target else {
        mat.color = LinearRgba::rgb(0.5, 0.5, 0.5);
        return;
    };
    // Tick: ease toward the target.
    for (last, goal) in tint.last.iter_mut().zip(target) {
        let step = ((*last - goal).abs() * 0.1).round() + 0.0625;
        if *last < goal {
            *last = (*last + step).min(goal);
        } else if *last > goal {
            *last = (*last - step).max(goal);
        }
    }
    // DrawModOverlay: brighten each channel by round(c (1 - c/255) - 2).
    let mut draw = tint.last.map(|c| (c + (c * (1.0 - c / 255.0) - 2.0).round()).clamp(0.0, 255.0) / 255.0);
    // Alive and under HealthMax x 0.25: NearDeathOverlay, the draw colour
    // times its texel.
    let near_death = health.health > 0.0 && health.health < crate::game::combat::PLAYER_HEALTH_MAX * 0.25;
    if near_death != *near_death_was {
        *near_death_was = near_death;
        runlog::kv("near_death_overlay", &format!("on={near_death} health={:.0}", health.health));
    }
    if near_death {
        let texel = near_death_texel(time.elapsed_secs());
        for (d, t) in draw.iter_mut().zip(texel) {
            *d *= t;
        }
    }
    // Screen x 2 x draw in gamma space = screen x (2 draw)^2.2 in linear;
    // the blend doubles, so the source is half of that (at most 1).
    let lin = draw.map(|d| ((2.0 * d).powf(2.2) / 2.0).min(1.0));
    mat.color = LinearRgba::rgb(lin[0], lin[1], lin[2]);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DeSat: red at 0.5 s into each 1.5 s loop, back to Grain1 at 1.5 s.
    #[test]
    fn brightness_quad_colour() {
        assert_eq!(brightness_color(100).red, 0.5);
        assert_eq!(brightness_color(200).red, 1.0);
        assert_eq!(brightness_color(50).red, 0.25);
    }

    #[test]
    fn near_death_pulse() {
        assert_eq!(near_death_texel(0.5), INJURED_TEXEL);
        let end = near_death_texel(1.4999);
        assert!((end[1] - GRAIN1_TEXEL[1]).abs() < 1e-3);
        assert!((near_death_texel(2.0)[0] - 1.0).abs() < 1e-6);
        // Half way into the red: G between the two.
        let g = near_death_texel(0.25)[1];
        assert!((g - (GRAIN1_TEXEL[1] + INJURED_TEXEL[1]) / 2.0).abs() < 1e-5);
    }
}
