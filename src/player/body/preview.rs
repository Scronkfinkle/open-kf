//! Character previews for the menus: KF's SpinnyWeap actors drawn with
//! `Canvas.DrawActorClipped` into a GUI box (the perk page's "3D View",
//! KFTab_Profile; the character select window's model, KFModelSelect).
//! See DESIGN.md, "The 3D View and Change Character".
//!
//! Each preview slot has its own camera rendering into an image, on its
//! own render layer (the world does not see the model, the camera does
//! not see the world), cleared to transparent so the menu's box shows
//! behind the model, as DrawActorClipped draws over the menu. The menus
//! say every frame what each slot should show (`CharacterPreview`); a slot
//! without a request is switched off.

use std::collections::HashMap;

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use ue_assets::properties::Rotator;

use super::load::{BodyModels, BodyPackages, find_or_load};
use crate::engine::coords;
use crate::engine::runlog;
use crate::world::map::MapRequest;

/// The perk page's 3D View (KFTab_Profile.SpinnyDude).
pub const PREVIEW_PROFILE: usize = 0;
/// The character select window's model (KFModelSelect.SpinnyDude).
pub const PREVIEW_MODEL_SELECT: usize = 1;
pub const PREVIEW_SLOTS: usize = 2;
/// Render layers of the slots (6, 7: unused elsewhere).
const FIRST_LAYER: usize = 6;
/// KFTab_Profile.UpdateSpinnyDude / KFModelSelect.UpdateSpinnyDude:
/// `SpinnyDude.LoopAnim('Profile_idle')`.
const IDLE_ANIM: &str = "Profile_idle";

/// What one slot shows. Unreal units and rotation units.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewRequest {
    /// The character record's name.
    pub character: String,
    /// The GUI box (the image), physical pixels.
    pub size: UVec2,
    /// DrawActorClipped's DisplayFOV (degrees, across the box's width:
    /// UE2's FOV is horizontal).
    pub fov_deg: f32,
    /// Where the actor is from the camera: forward (X), right (Y), up (Z).
    pub offset: Vec3,
    /// The actor's yaw relative to the camera's (32768: facing it).
    pub yaw: i32,
    /// SpinnyDude.SetDrawScale.
    pub draw_scale: f32,
}

/// The requests (the menus fill them each frame) and the images the
/// slots draw into (an image is replaced when its size changes).
#[derive(Resource, Default)]
pub struct CharacterPreview {
    pub requests: [Option<PreviewRequest>; PREVIEW_SLOTS],
    pub images: [Option<Handle<Image>>; PREVIEW_SLOTS],
}

/// The preview systems (the menus copy the images after them).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct PreviewSystems;

#[derive(Component)]
pub(super) struct PreviewCamera(usize);

/// One slot's drawn model.
#[derive(Default)]
struct Slot {
    /// Index into `BodyModels::characters`.
    character: Option<usize>,
    root: Option<Entity>,
    meshes: Vec<Handle<Mesh>>,
    frame: f32,
    size: UVec2,
    log_second: i64,
    shown: bool,
    /// The last character asked for that could not be loaded (logged once).
    failed: Option<String>,
}

#[derive(Default)]
pub(super) struct PreviewState {
    slots: [Slot; PREVIEW_SLOTS],
    /// Unlit copies of a character's part materials (SpinnyWeap bUnlit).
    unlit: HashMap<usize, Vec<Handle<StandardMaterial>>>,
}

pub(super) fn spawn_preview_cameras(mut commands: Commands, mut preview: ResMut<CharacterPreview>, mut images: ResMut<Assets<Image>>) {
    for slot in 0..PREVIEW_SLOTS {
        let image = images.add(Image::new_target_texture(16, 16, TextureFormat::Rgba8UnormSrgb, None));
        commands.spawn((
            Camera3d::default(),
            Camera {
                // Before the main view (order 0) and the scope (-3, -2).
                order: -10 + slot as isize,
                is_active: false,
                clear_color: ClearColorConfig::Custom(Color::NONE),
                ..default()
            },
            RenderTarget::Image(ImageRenderTarget::from(image.clone())),
            // Unlit textures shown as they are (as the HUD's).
            bevy::core_pipeline::tonemapping::Tonemapping::None,
            Projection::from(PerspectiveProjection { fov: 15f32.to_radians(), aspect_ratio: 1.0, near: 0.05, ..default() }),
            Transform::from_translation(slot_base(slot)),
            RenderLayers::layer(FIRST_LAYER + slot),
            PreviewCamera(slot),
        ));
        preview.images[slot] = Some(image);
    }
}

/// Where a slot's camera stands (Bevy space): far below the maps, so
/// nothing else is near (the layers already keep them apart).
fn slot_base(slot: usize) -> Vec3 {
    Vec3::new(slot as f32 * 100.0, -5000.0, 0.0)
}

/// Loads, poses and frames each requested preview; switches off the rest.
#[allow(clippy::too_many_arguments)] // Bevy system parameters
pub(super) fn update_previews(
    mut commands: Commands,
    mut preview: ResMut<CharacterPreview>,
    mut state: NonSendMut<PreviewState>,
    kept: NonSend<BodyPackages>,
    mut models: ResMut<BodyModels>,
    map: Res<MapRequest>,
    mut cams: Query<(&PreviewCamera, &mut Camera, &mut Projection, &mut RenderTarget)>,
    mut roots: Query<(&mut Transform, &mut Visibility)>,
    (mut meshes, mut images, mut materials): super::load::RenderAssets,
    time: Res<Time<Real>>,
) {
    let state = &mut *state;
    // SpinnyWeap has bAlwaysTick: it animates even while the game is paused.
    let dt = time.delta_secs();
    for (cam_slot, mut cam, mut proj, mut target) in &mut cams {
        let slot = cam_slot.0;
        let st = &mut state.slots[slot];
        let Some(req) = preview.requests[slot].clone() else {
            cam.is_active = false;
            if let Some(root) = st.root
                && let Ok((_, mut vis)) = roots.get_mut(root)
            {
                *vis = Visibility::Hidden;
            }
            if st.shown {
                st.shown = false;
                runlog::kv("preview_hidden", &format!("slot={slot}"));
            }
            continue;
        };
        let size = req.size.max(UVec2::ONE);
        // The image follows the box's size.
        if size != st.size {
            let image = images.add(Image::new_target_texture(size.x, size.y, TextureFormat::Rgba8UnormSrgb, None));
            *target = RenderTarget::Image(ImageRenderTarget::from(image.clone()));
            preview.images[slot] = Some(image);
            st.size = size;
        }
        // DrawActorClipped's FOV is across the box's width; Bevy's is vertical.
        let aspect = size.x as f32 / size.y as f32;
        let half_h = (req.fov_deg.to_radians() * 0.5).tan();
        if let Projection::Perspective(p) = proj.as_mut() {
            p.fov = 2.0 * (half_h / aspect).atan();
            p.aspect_ratio = aspect;
        }
        // The character (loaded once; later requests reuse it).
        let Some(set) = kept.0.as_ref() else { continue };
        let wanted = find_or_load(set, &mut models, &map, &req.character, (&mut meshes, &mut images, &mut materials));
        let Some(index) = wanted else {
            cam.is_active = false;
            if st.failed.as_deref() != Some(req.character.as_str()) {
                runlog::kv("preview_error", &format!("slot={slot} character={} reason=not_loaded", req.character));
                st.failed = Some(req.character.clone());
            }
            continue;
        };
        let b = &models.characters[index];
        if st.character != Some(index) {
            if let Some(root) = st.root.take() {
                commands.entity(root).despawn();
            }
            let unlit = state.unlit.entry(index).or_insert_with(|| {
                b.model
                    .parts
                    .iter()
                    .map(|p| {
                        let mut m = materials.get(&p.material).cloned().unwrap_or_default();
                        m.unlit = true;
                        materials.add(m)
                    })
                    .collect()
            });
            let handles = b.model.new_instance(&mut meshes);
            let layer = RenderLayers::layer(FIRST_LAYER + slot);
            let root = commands.spawn((Transform::default(), Visibility::Hidden, layer.clone())).id();
            for (h, m) in handles.iter().zip(unlit.iter()) {
                commands.spawn((
                    Mesh3d(h.clone()),
                    MeshMaterial3d(m.clone()),
                    Transform::IDENTITY,
                    layer.clone(),
                    NoFrustumCulling,
                    bevy::light::NotShadowCaster,
                    bevy::light::NotShadowReceiver,
                    ChildOf(root),
                ));
            }
            let st = &mut state.slots[slot];
            st.root = Some(root);
            st.meshes = handles;
            st.character = Some(index);
            st.frame = 0.0;
            st.failed = None;
            let seq = b.model.sequence(IDLE_ANIM);
            runlog::kv(
                "preview_character",
                &format!(
                    "slot={slot} character={} wanted={} anim={IDLE_ANIM} anim_found={} anim_frames={:.0} anim_rate={:.1} parts={} draw_scale={} fov_deg={} offset={:?} box={}x{}",
                    b.name,
                    req.character,
                    seq.is_some(),
                    seq.map_or(0.0, |s| b.model.length(s)),
                    seq.map_or(0.0, |s| b.model.rate(s)),
                    b.model.parts.len(),
                    req.draw_scale,
                    req.fov_deg,
                    req.offset.to_array(),
                    size.x,
                    size.y
                ),
            );
        }
        let st = &mut state.slots[slot];
        cam.is_active = true;
        // LoopAnim('Profile_idle'): the sequence's own rate, looping.
        let seq = b.model.sequence(IDLE_ANIM);
        if let Some(s) = seq {
            let len = b.model.length(s).max(1.0);
            st.frame = (st.frame + dt * b.model.rate(s)).rem_euclid(len);
        }
        let pose = b.model.pose_from_locals(&b.model.sample_locals(seq, st.frame), &[]);
        let skinned = b.model.skin(&pose, &[]);
        let to_actor = b.model.mesh_to_actor(Vec3::ZERO, req.draw_scale);
        b.model.upload_to(&st.meshes, &skinned, |p| coords::pos(to_actor(p).to_array()), &mut meshes);
        if let Some(root) = st.root
            && let Ok((mut t, mut vis)) = roots.get_mut(root)
        {
            t.translation = slot_base(slot) + coords::pos(req.offset.to_array());
            t.rotation = coords::rotation(Rotator { pitch: 0, yaw: req.yaw, roll: 0 });
            *vis = Visibility::Inherited;
        }
        if !st.shown {
            st.shown = true;
            st.log_second = -1;
            runlog::kv("preview_shown", &format!("slot={slot} character={} box={}x{}", b.name, size.x, size.y));
        }
        // Every 5 seconds: where the model lands in the box (pixels), from
        // the same projection (Unreal camera space: X forward, Y right, Z up).
        let second = time.elapsed_secs() as i64 / 5;
        if second != st.log_second {
            st.log_second = second;
            let rot = coords::ue_rotation_matrix(Rotator { pitch: 0, yaw: req.yaw, roll: 0 });
            let focal = size.x as f32 * 0.5 / half_h;
            let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
            let (mut zlo, mut zhi) = (f32::MAX, f32::MIN);
            for &p in &skinned {
                let a = rot * to_actor(p);
                let c = req.offset + a;
                if c.x <= 1.0 {
                    continue;
                }
                let px = Vec2::new(size.x as f32 * 0.5 + c.y / c.x * focal, size.y as f32 * 0.5 - c.z / c.x * focal);
                lo = lo.min(px);
                hi = hi.max(px);
                zlo = zlo.min(a.z);
                zhi = zhi.max(a.z);
            }
            runlog::kv(
                "preview_state",
                &format!(
                    "slot={slot} character={} frame={:.1} yaw={} box={}x{} model_px=({:.0},{:.0})-({:.0},{:.0}) model_height_units={:.1} model_z=({:.1},{:.1})",
                    b.name,
                    st.frame,
                    req.yaw,
                    size.x,
                    size.y,
                    lo.x,
                    lo.y,
                    hi.x,
                    hi.y,
                    zhi - zlo,
                    zlo,
                    zhi
                ),
            );
        }
    }
}
