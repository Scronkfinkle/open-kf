//! 3D weapon scopes, KF's default scope detail (KFWeapon.KFScopeDetail =
//! KF_ModelScope; no ini sets it). While aiming a weapon with bHasScope,
//! Crossbow.RenderTexture draws the world from the eye along the view into a
//! 512 x 512 ScriptedTexture (DrawPortal at scopePortalFOV), combines it with
//! the reticle texture, and puts it on the weapon's lens material slot
//! (Skins[lenseMaterialID]).
//!
//! Here: two cameras (sky zone, then the world) render into an image, a
//! reticle quad in front of the world camera is drawn over it with its alpha,
//! and the weapon code swaps the lens material for one showing the image.
//! The reticle use is my reading of the Combiner (CO_Multiply, AO_Use_Mask):
//! the reticle textures are transparent inside the circle and black at the
//! lines and border, so drawing them over the view by alpha gives the clear
//! circle with black lines; a literal multiply would turn the clear part
//! black. Not checked against the original renderer.

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;

use crate::engine::camera::FlyCamera;
use crate::world::map::{SKY_LAYER, SkyInfo};
use crate::engine::runlog;

/// Render layer seen only by the scope's world camera (the reticle quad).
pub const SCOPE_LAYER: usize = 3;

/// ScriptedTexture size for KF_ModelScope (UpdateScopeMode: SetSize(512, 512)).
const SCOPE_SIZE: u32 = 512;

/// Distance of the reticle quad in front of the scope camera (metres).
const RETICLE_DISTANCE: f32 = 0.02;

pub struct ScopePlugin;

impl Plugin for ScopePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScopeRequest>()
            .add_systems(PostStartup, spawn_scope) // the camera exists: the first map loaded in Startup (world/map_change.rs)
            // A later map: its own sky camera (world/map_change.rs).
            .add_systems(crate::world::map_change::PostMapLoad, rebuild_scope_sky)
            .add_systems(PostUpdate, update_scope.before(bevy::transform::TransformSystems::Propagate));
    }
}

/// What the weapon code wants this frame.
#[derive(Resource, Default)]
pub struct ScopeRequest {
    /// Draw the portal (ShouldDrawPortal: aiming a scoped weapon).
    pub active: bool,
    /// scopePortalFOV, degrees (the texture is square).
    pub fov_deg: f32,
    /// The reticle texture (the Combiner's Material1).
    pub reticle: Option<Handle<Image>>,
}

/// The lens material that shows the scope image.
#[derive(Resource)]
pub struct ScopeView {
    pub lens_material: Handle<StandardMaterial>,
    /// The image the scope cameras draw into (a new map's sky camera too).
    target: RenderTarget,
}

#[derive(Component)]
struct ScopeCamera;

#[derive(Component)]
struct ScopeSkyCamera;

#[derive(Component)]
struct ScopeReticle;

fn scope_projection() -> Projection {
    Projection::from(PerspectiveProjection {
        fov: 12f32.to_radians(),
        aspect_ratio: 1.0,
        near: 0.01,
        ..default()
    })
}

/// The scope's sky camera, if the map has a sky zone (a map thing:
/// `MapScoped`).
fn spawn_scope_sky(commands: &mut Commands, sky: &SkyInfo, target: &RenderTarget) {
    let Some(sky_pos) = sky.camera_position else { return };
    let mut sky_cam = commands.spawn((
        Camera3d::default(),
        Camera {
            order: -3,
            is_active: false,
            ..default()
        },
        target.clone(),
        scope_projection(),
        Transform::from_translation(sky_pos),
        RenderLayers::layer(SKY_LAYER),
        ScopeSkyCamera,
        crate::world::map_change::MapScoped,
    ));
    // The sky zone's own fog, as on the main sky camera.
    if let Some(f) = &sky.fog {
        sky_cam.insert(crate::world::zones::distance_fog(f.start, f.end, f.color));
    }
}

/// A later map (the scope exists already): the new map's sky camera, and
/// the world camera drawing over it or clearing.
fn rebuild_scope_sky(mut commands: Commands, sky: Res<SkyInfo>, view: Option<Res<ScopeView>>, mut cams: Query<&mut Camera, With<ScopeCamera>>) {
    let Some(view) = view else { return };
    spawn_scope_sky(&mut commands, &sky, &view.target);
    for mut c in &mut cams {
        c.clear_color = if sky.camera_position.is_some() { ClearColorConfig::None } else { ClearColorConfig::Default };
    }
    runlog::kv("scope_sky", &format!("sky_camera={}", sky.camera_position.is_some()));
}

fn spawn_scope(
    mut commands: Commands,
    sky: Res<SkyInfo>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let image = images.add(Image::new_target_texture(
        SCOPE_SIZE,
        SCOPE_SIZE,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    let target = RenderTarget::Image(ImageRenderTarget::from(image.clone()));
    spawn_scope_sky(&mut commands, &sky, &target);
    let cam = commands
        .spawn((
            Camera3d::default(),
            Camera {
                order: -2,
                is_active: false,
                // Drawn over the sky camera's image, as the main view is.
                clear_color: if sky.camera_position.is_some() {
                    ClearColorConfig::None
                } else {
                    ClearColorConfig::Default
                },
                ..default()
            },
            target.clone(),
            scope_projection(),
            Transform::default(),
            RenderLayers::from_layers(&[0, SCOPE_LAYER]),
            ScopeCamera,
        ))
        .id();
    // The reticle: a quad filling the scope camera's view, just past its
    // near plane, drawn with the texture's alpha.
    let reticle_material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(1.0, 1.0))),
        MeshMaterial3d(reticle_material),
        Transform::from_xyz(0.0, 0.0, -RETICLE_DISTANCE),
        RenderLayers::layer(SCOPE_LAYER),
        Visibility::Hidden,
        ScopeReticle,
        ChildOf(cam),
    ));
    // The lens: unlit (KF's scope Shader uses the image as SelfIllumination).
    let lens_material = materials.add(StandardMaterial {
        base_color_texture: Some(image.clone()),
        unlit: true,
        cull_mode: None,
        double_sided: true,
        ..default()
    });
    commands.insert_resource(ScopeView { lens_material, target });
    runlog::kv("scope_ready", &format!("size={SCOPE_SIZE} sky_camera={}", sky.camera_position.is_some()));
}

/// Turns the scope cameras on while a scoped weapon is aimed, and keeps them
/// at the eye, along the view (Crossbow.RenderTexture: the view rotation,
/// recoil included).
#[allow(clippy::type_complexity)] // Bevy system parameters
fn update_scope(
    request: Res<ScopeRequest>,
    main: Query<&Transform, (With<FlyCamera>, Without<ScopeCamera>, Without<ScopeSkyCamera>)>,
    mut cams: Query<
        (&mut Camera, &mut Transform, &mut Projection, Has<ScopeSkyCamera>),
        Or<(With<ScopeCamera>, With<ScopeSkyCamera>)>,
    >,
    mut reticle: Query<
        (&mut Transform, &mut Visibility, &MeshMaterial3d<StandardMaterial>),
        (With<ScopeReticle>, Without<ScopeCamera>, Without<ScopeSkyCamera>, Without<FlyCamera>),
    >,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut was_active: Local<bool>,
    sky_info: Res<SkyInfo>,
) {
    let Ok(main) = main.single() else {
        return;
    };
    let fov = request.fov_deg.max(1.0).to_radians();
    for (mut cam, mut t, mut proj, is_sky) in &mut cams {
        cam.is_active = request.active;
        if !request.active {
            continue;
        }
        if is_sky {
            t.rotation = sky_info.rotation * main.rotation;
        } else {
            *t = *main;
        }
        if let Projection::Perspective(p) = proj.as_mut()
            && p.fov != fov
        {
            p.fov = fov;
        }
    }
    if let Ok((mut t, mut vis, mat)) = reticle.single_mut() {
        let has_reticle = request.reticle.is_some();
        *vis = if request.active && has_reticle { Visibility::Inherited } else { Visibility::Hidden };
        // Fill the view: height = 2 d tan(fov / 2).
        let size = 2.0 * RETICLE_DISTANCE * (fov * 0.5).tan();
        t.scale = Vec3::new(size, size, 1.0);
        if let Some(mut m) = materials.get_mut(&mat.0)
            && m.base_color_texture != request.reticle
        {
            m.base_color_texture = request.reticle.clone();
        }
    }
    if request.active != *was_active {
        *was_active = request.active;
        runlog::kv(
            "scope_portal",
            &format!("active={} fov_deg={:.2} reticle={}", request.active, request.fov_deg, request.reticle.is_some()),
        );
    }
}
