//! The HUD's arrow to the trader (T2b-2): KFShopDirectionPointer, a 3D
//! arrow drawn in the top-left corner over everything, turned toward the
//! current shop (HUDKillingFloor.DrawKFHUDTextElements). See DESIGN.md,
//! T2b-2.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use ue_assets::package::ObjectRef;
use ue_assets::package_set::{ObjectHandle, PackageSet};
use ue_assets::properties::Rotator;
use ue_assets::static_mesh::read_static_mesh;

use crate::engine::camera::FlyCamera;
use crate::engine::coords::{self, SCALE};
use crate::world::map::MapRequest;
use crate::engine::runlog;

/// Its own layer, drawn by its own camera after the others.
const ARROW_LAYER: usize = 5;
/// KFShopDirectionPointer StaticMesh and DrawScale.
const ARROW_MESH: &str = "DebugObjects.Arrows.debugarrow1";
const DRAW_SCALE: f32 = 0.25;
/// NOT from KF's code: fitted to a real-game screenshot. With KF's own
/// numbers (10 units along a unit ray, DrawScale 0.25) the arrow came out
/// about 7.75 times too big. ScreenToWorld is native (not in the scripts),
/// so its true scale is unknown; perhaps its result is not a unit vector.
/// Measured at 1280 x 960 from KF-WestLondon's spawn, view matched to the
/// screenshot (lamppost at the same pixel): the real arrow covers x 13-122,
/// y 50-89 (1920 red pixels); ours at 7.5x: x 12-123, y 47-91 (2052), at
/// 8x: x 16-120, y 48-90 (1814). The centre matched at every distance.
const ARROW_DISTANCE_FIT: f32 = 7.75;

#[derive(Component)]
struct ArrowCamera;

#[derive(Component)]
struct Arrow;

pub struct TraderArrowPlugin;

impl Plugin for TraderArrowPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, spawn_arrow)
            .add_systems(PostUpdate, place_arrow.before(bevy::transform::TransformSystems::Propagate));
    }
}

fn spawn_arrow(
    mut commands: Commands,
    request: Res<MapRequest>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let layer = RenderLayers::layer(ARROW_LAYER);
    // HUD drawing comes after the scene, the weapon (1) and the vision
    // overlay (2); C.DrawActor(None, False, True) clears Z first, which
    // its own camera's depth buffer does.
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: 3,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        Projection::from(PerspectiveProjection {
            fov: crate::engine::camera::vertical_fov(crate::engine::camera::DEFAULT_FOV),
            near: 0.01,
            ..default()
        }),
        bevy::core_pipeline::tonemapping::Tonemapping::None,
        Transform::default(),
        layer.clone(),
        ArrowCamera,
    ));
    let set = PackageSet::new(&request.install_root);
    let Some(h) = set.find_object(ARROW_MESH, Some("StaticMesh")) else {
        runlog::kv("trader_arrow_ready", &format!("mesh={ARROW_MESH} found=false"));
        return;
    };
    let sm = match read_static_mesh(&h.package.pkg, h.export) {
        Ok(sm) => sm,
        Err(e) => {
            runlog::kv("trader_arrow_ready", &format!("mesh={ARROW_MESH} error={e}"));
            return;
        }
    };
    let parent = commands
        .spawn((Transform::default(), Visibility::Hidden, layer.clone(), Arrow))
        .id();
    let mut textured = 0;
    for (si, section) in sm.sections.iter().enumerate() {
        let tris = &sm.indices[section.first_index..section.first_index + section.num_triangles * 3];
        if tris.is_empty() {
            continue;
        }
        let positions: Vec<[f32; 3]> = sm.positions.iter().map(|p| coords::pos(*p).to_array()).collect();
        let normals: Vec<[f32; 3]> = sm.normals.iter().map(|n| coords::dir(*n).normalize_or_zero().to_array()).collect();
        let uvs: Vec<[f32; 2]> = sm.uvs.first().cloned().unwrap_or_else(|| vec![[0.0, 0.0]; sm.positions.len()]);
        let mesh = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
            .with_inserted_indices(bevy::mesh::Indices::U32(tris.iter().map(|&i| i as u32).collect()));
        let rf = sm.materials.get(si).copied().unwrap_or(ObjectRef::Null);
        let simple = ue_assets::material::resolve(&set, &ObjectHandle { package: h.package.clone(), export: 0 }, rf);
        let image = simple.texture.as_ref().and_then(|t| crate::render::particles::decode(t, true, false, &mut images));
        textured += usize::from(image.is_some());
        // Effects: bUnlit.
        let material = materials.add(StandardMaterial {
            base_color_texture: image,
            unlit: true,
            ..default()
        });
        commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material), Transform::IDENTITY, layer.clone(), ChildOf(parent)));
    }
    runlog::kv(
        "trader_arrow_ready",
        &format!(
            "mesh={ARROW_MESH} sections={} textured={textured} bounds_unreal=({:.1}, {:.1}, {:.1})..({:.1}, {:.1}, {:.1})",
            sm.sections.len(),
            sm.bounds.min[0],
            sm.bounds.min[1],
            sm.bounds.min[2],
            sm.bounds.max[0],
            sm.bounds.max[1],
            sm.bounds.max[2]
        ),
    );
}

/// The arrow's turn (HUDKillingFloor): toward the shop, level unless the
/// shop is more than 50 units above or below the pawn. Unreal rotator.
fn pointer_rotation(shop: Vec3, pawn: Vec3) -> Rotator {
    let mut to = shop;
    if (shop.z - pawn.z).abs() <= 50.0 {
        to.z = pawn.z;
    }
    let d = (to - pawn).normalize_or_zero();
    let k = 65536.0 / std::f32::consts::TAU;
    Rotator {
        pitch: (d.z.clamp(-1.0, 1.0).asin() * k) as i32,
        yaw: (d.y.atan2(d.x) * k) as i32,
        roll: 0,
    }
}

/// ScreenToWorld(SizeX / 18, SizeX / 18) in camera space (Bevy: -Z ahead):
/// the unit ray through that pixel for a perspective camera with vertical
/// FOV `fov_y` and the window's `size`.
fn corner_ray(size: Vec2, fov_y: f32) -> Vec3 {
    // KF's code uses SizeX for both coordinates.
    let px = Vec2::splat(size.x / 18.0);
    let ndc = Vec2::new(2.0 * px.x / size.x - 1.0, 1.0 - 2.0 * px.y / size.y);
    let t = (fov_y / 2.0).tan();
    Vec3::new(ndc.x * t * size.x / size.y, ndc.y * t, -1.0).normalize()
}

type MainView<'w, 's> = Query<'w, 's, (&'static Transform, &'static Projection, Option<&'static crate::player::walk::Walker>), (With<FlyCamera>, Without<ArrowCamera>, Without<Arrow>)>;
type ArrowView<'w, 's> = Query<'w, 's, (&'static mut Transform, &'static mut Projection), (With<ArrowCamera>, Without<Arrow>, Without<FlyCamera>)>;
type ArrowModel<'w, 's> = Query<'w, 's, (&'static mut Transform, &'static mut Visibility), (With<Arrow>, Without<ArrowCamera>, Without<FlyCamera>)>;

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn place_arrow(
    time: Res<Time>,
    main: MainView,
    mut cam: ArrowView,
    mut arrow: ArrowModel,
    window: Query<&Window>,
    (shops, menu, options, fov): (Res<crate::game::trader::Shops>, Res<crate::game::buy_menu::BuyMenu>, Res<crate::game::waves::GameOptions>, Res<crate::engine::camera::ViewFov>),
    game: Res<crate::game::waves::WaveGame>,
    mut log_timer: Local<f32>,
) {
    let (Ok((main_t, main_p, walker)), Ok((mut cam_t, mut cam_p)), Ok((mut t, mut vis))) = (main.single(), cam.single_mut(), arrow.single_mut()) else {
        return;
    };
    // The arrow camera sees what the main camera sees.
    *cam_t = *main_t;
    *cam_p = main_p.clone();
    let shop = shops.current.map(|c| shops.shops[c].location);
    // Won: DrawEndGameHUD(True) returns before DrawKFHUDTextElements.
    let won = game.phase == crate::game::waves::Phase::Won;
    let shown = options.mode == crate::game::waves::GameMode::Waves && !menu.open && !won && shop.is_some();
    *vis = if shown { Visibility::Visible } else { Visibility::Hidden };
    let (Some(shop), Ok(win), Projection::Perspective(p)) = (shop, window.single(), main_p) else { return };
    let ray = corner_ray(Vec2::new(win.width(), win.height()), p.fov);
    // x 10 x (DefaultFOV / FovAngle): further when zoomed, the same size.
    // ARROW_DISTANCE_FIT: see its comment.
    let dist = 10.0 * ARROW_DISTANCE_FIT * (crate::engine::camera::DEFAULT_FOV / fov.0) * SCALE;
    t.translation = main_t.translation + main_t.rotation * (ray * dist);
    let centre = walker.map_or(main_t.translation - Vec3::Y * crate::game::combat::PLAYER_EYE_HEIGHT * SCALE, |w| w.center);
    let pawn = Vec3::new(-centre.z, centre.x, centre.y) / SCALE;
    let rot = pointer_rotation(shop, pawn);
    t.rotation = coords::rotation(rot);
    t.scale = Vec3::splat(DRAW_SCALE);
    *log_timer += time.delta_secs();
    if *log_timer >= 2.0 {
        *log_timer = 0.0;
        runlog::kv(
            "trader_arrow",
            &format!("shown={shown} yaw={} pitch={} distance_m={}", rot.yaw, rot.pitch, ((shop - pawn).length() / 50.0) as i32),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrow_stays_level_within_fifty_units() {
        let pawn = Vec3::ZERO;
        let r = pointer_rotation(Vec3::new(0.0, 1000.0, 40.0), pawn);
        assert_eq!((r.yaw, r.pitch), (16384, 0));
        // 60 above: tilted up.
        let r = pointer_rotation(Vec3::new(1000.0, 0.0, 60.0), pawn);
        assert_eq!(r.yaw, 0);
        assert!(r.pitch > 600 && r.pitch < 650, "{}", r.pitch);
    }

    #[test]
    fn corner_ray_points_up_and_left() {
        let r = corner_ray(Vec2::new(1600.0, 900.0), crate::engine::camera::vertical_fov(90.0));
        assert!(r.x < 0.0 && r.y > 0.0 && r.z < 0.0, "{r}");
        assert!((r.length() - 1.0).abs() < 1e-5);
    }
}
