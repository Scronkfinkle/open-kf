//! The camera, and free flying: click to capture the mouse, Escape to
//! release. The game starts walking (player/walk.rs); `--fly` starts
//! flying, and V switches. Flying: WASD to move, mouse to look, Space/E up,
//! Ctrl/Q down, Shift for speed.

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use bevy::camera::visibility::RenderLayers;

use crate::world::map::{SKY_LAYER, SkyInfo, SpawnPoint};
use crate::engine::runlog;

#[derive(Component)]
pub struct FlyCamera {
    /// Radians; 0 looks along -Z.
    pub yaw: f32,
    pub pitch: f32,
    /// Metres per second.
    pub speed: f32,
}

/// Start pose from `--camera X,Y,Z,YAW,PITCH`: position in Unreal units (as
/// printed in the log's `unreal=(...)`), yaw and pitch in radians as logged.
#[derive(Resource, Clone, Copy, Debug)]
pub struct CameraOverride {
    pub unreal_position: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
}

impl CameraOverride {
    pub fn parse(s: &str) -> Result<Self, String> {
        let v: Vec<f32> = s
            .split(',')
            .map(|x| x.trim().parse::<f32>())
            .collect::<Result<_, _>>()
            .map_err(|_| format!("bad --camera value: {s}"))?;
        match v.as_slice() {
            [x, y, z, yaw, pitch] => Ok(CameraOverride {
                unreal_position: [*x, *y, *z],
                yaw: *yaw,
                pitch: *pitch,
            }),
            _ => Err(format!("--camera needs X,Y,Z,YAW,PITCH, got: {s}")),
        }
    }
}

/// Camera inside the sky zone. It turns with the main camera but stays put,
/// like Unreal's skybox rendering.
#[derive(Component)]
pub struct SkyCamera;

pub struct FlyCameraPlugin;

impl Plugin for FlyCameraPlugin {
    fn build(&self, app: &mut App) {
        // Runs after map loading so the spawn point is known.
        app.init_resource::<ViewFov>()
            .add_systems(PostStartup, spawn_camera)
            .add_systems(Update, (grab_cursor, look, scripted_turn, fly, follow_sky, log_camera).chain());
    }
}

/// Test action `turn:DEG`: turns the view by DEG degrees over one second
/// (positive = right, as Unreal yaw), as a mouse would; e.g. to see the
/// body's turning-in-place animation.
fn scripted_turn(
    time: Res<Time>,
    script: Res<crate::weapons::weapon::ScriptedInput>,
    frames: Res<bevy::diagnostic::FrameCount>,
    mut cams: Query<(&mut Transform, &mut FlyCamera, Option<&mut crate::player::walk::Walker>)>,
    mut turning: Local<Option<(f32, f32)>>,
    zeds: Query<&crate::zeds::zed::Zed>,
    pawns: Query<&crate::player::body::PawnState>,
) {
    for (_, a) in script.0.iter().filter(|(f, _)| *f == frames.0) {
        // Test action `aim_player` (network games): look at the nearest
        // other player's pawn (its chest: 20 units above the centre).
        if a == "aim_player" {
            for (mut t, mut cam, _) in &mut cams {
                let eye = t.translation;
                let Some(p) = pawns.iter().filter(|p| !p.local && p.active).min_by(|a, b| (a.location - eye).length_squared().total_cmp(&(b.location - eye).length_squared())) else {
                    runlog::kv("scripted_aim", "player=none");
                    continue;
                };
                let at = p.location + Vec3::Y * 20.0 * crate::engine::coords::SCALE;
                let d = (at - eye).normalize_or_zero();
                cam.yaw = (-d.x).atan2(-d.z);
                cam.pitch = d.y.clamp(-1.0, 1.0).asin();
                t.rotation = Quat::from_euler(EulerRot::YXZ, cam.yaw, cam.pitch, 0.0);
                runlog::kv("scripted_aim", &format!("player=nearest distance_unreal={:.0}", (at - eye).length() / crate::engine::coords::SCALE));
            }
        }
        // Test action `warp:X;Y;Z;YAW`: the player's cylinder centre to
        // (X, Y, Z) Unreal units, facing Unreal yaw YAW degrees.
        if let Some(v) = a.strip_prefix("warp:") {
            let n: Vec<f32> = v.split(';').filter_map(|x| x.parse().ok()).collect();
            if n.len() == 4 {
                for (mut t, mut cam, walker) in &mut cams {
                    let centre = crate::engine::coords::pos([n[0], n[1], n[2]]);
                    t.translation = centre + Vec3::Y * crate::player::walk::kf::EYE_HEIGHT * crate::engine::coords::SCALE;
                    let y = n[3].to_radians();
                    let f = crate::engine::coords::dir([y.cos(), y.sin(), 0.0]);
                    cam.yaw = (-f.x).atan2(-f.z);
                    cam.pitch = 0.0;
                    t.rotation = Quat::from_euler(EulerRot::YXZ, cam.yaw, 0.0, 0.0);
                    if let Some(mut w) = walker {
                        w.center = centre;
                        w.velocity = Vec3::ZERO;
                    }
                }
                runlog::kv("scripted_warp", &format!("at_unreal=({}, {}, {}) yaw_deg={}", n[0], n[1], n[2], n[3]));
            }
        }
        if let Some(deg) = a.strip_prefix("turn:").and_then(|d| d.parse::<f32>().ok()) {
            *turning = Some((deg.to_radians(), 1.0));
            runlog::kv("scripted_turn", &format!("degrees={deg} seconds=1"));
        }
        // Test action `aim_zed`: look straight at the nearest living zed's
        // head (its body centre if the head is not placed yet).
        if a == "aim_zed" {
            for (mut t, mut cam, _) in &mut cams {
                let eye = t.translation;
                let Some(z) = zeds.iter().filter(|z| !z.is_dead()).min_by(|a, b| (a.centre - eye).length_squared().total_cmp(&(b.centre - eye).length_squared())) else {
                    runlog::kv("scripted_aim", "zed=none");
                    continue;
                };
                let at = z.head.map_or(z.centre, |(h, _)| h);
                let d = (at - eye).normalize_or_zero();
                cam.yaw = (-d.x).atan2(-d.z);
                cam.pitch = d.y.clamp(-1.0, 1.0).asin();
                t.rotation = Quat::from_euler(EulerRot::YXZ, cam.yaw, cam.pitch, 0.0);
                runlog::kv("scripted_aim", &format!("zed={} distance_unreal={:.0} head={}", z.id, (at - eye).length() / crate::engine::coords::SCALE, z.head.is_some()));
            }
        }
    }
    let Some((rate, left)) = *turning else { return };
    let dt = time.delta_secs().min(left);
    for (mut t, mut cam, _) in &mut cams {
        cam.yaw -= rate * dt;
        t.rotation = Quat::from_euler(EulerRot::YXZ, cam.yaw, cam.pitch, 0.0);
    }
    *turning = (left - dt > 0.0).then_some((rate, left - dt));
}

/// The player's field of view as KF defines it: horizontal degrees at 4:3
/// (DefaultFOV 90, or `--fov`; iron sights lower it). Applied to the main
/// and sky cameras.
#[derive(Resource)]
pub struct ViewFov(pub f32);

impl Default for ViewFov {
    fn default() -> Self {
        ViewFov(DEFAULT_FOV)
    }
}

/// KFPlayerController DefaultFOV (at 4:3). The player's own value is
/// `GraphicsSettings::fov` (`--fov`, engine/graphics.rs).
pub const DEFAULT_FOV: f32 = 90.0;

/// Vertical field of view for a KF horizontal FOV. KF uses
/// bUseTrueWideScreenFOV, so widescreen keeps the 4:3 vertical angle:
/// 90 -> 2 * atan(tan(45 deg) * 3/4) = 73.74 deg vertical.
pub fn vertical_fov(horizontal_deg: f32) -> f32 {
    2.0 * ((horizontal_deg / 2.0).to_radians().tan() * 0.75).atan()
}

fn kf_projection(fov: f32) -> Projection {
    Projection::from(PerspectiveProjection {
        fov: vertical_fov(fov),
        ..default()
    })
}

pub fn spawn_camera(
    mut commands: Commands,
    spawn: Res<SpawnPoint>,
    sky: Res<SkyInfo>,
    over: Option<Res<CameraOverride>>,
    settings: Res<crate::engine::graphics::GraphicsSettings>,
    mut view_fov: ResMut<ViewFov>,
) {
    // The player's field of view (`--fov`); iron sights zoom from it.
    view_fov.0 = settings.fov;
    let f = spawn.forward.normalize_or(Vec3::NEG_Z);
    let (mut yaw, mut pitch) = ((-f.x).atan2(-f.z), f.y.clamp(-1.0, 1.0).asin());
    let mut position = spawn.position;
    if let Some(o) = over {
        position = crate::engine::coords::pos(o.unreal_position);
        yaw = o.yaw;
        pitch = o.pitch;
    }
    let rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: 0,
            // With a sky zone, the sky camera has already filled the screen;
            // drawing on top of it leaves the sky visible through gaps.
            clear_color: if sky.camera_position.is_some() {
                ClearColorConfig::None
            } else {
                ClearColorConfig::Default
            },
            ..default()
        },
        kf_projection(settings.fov),
        // UE2 had no tonemapping: colours go to the screen as computed
        // (Bevy's default film curve darkened and greyed KF's lighting).
        bevy::core_pipeline::tonemapping::Tonemapping::None,
        Transform::from_translation(position).with_rotation(rotation),
        FlyCamera { yaw, pitch, speed: 8.0 },
    ));
    if let Some(sky_pos) = sky.camera_position {
        let mut cam = commands.spawn((
            Camera3d::default(),
            Camera { order: -1, ..default() },
            kf_projection(settings.fov),
            bevy::core_pipeline::tonemapping::Tonemapping::None,
            Transform::from_translation(sky_pos).with_rotation(rotation),
            RenderLayers::layer(SKY_LAYER),
            SkyCamera,
        ));
        // KF fogs the sky view with the sky zone's own fog.
        if let Some(f) = &sky.fog {
            cam.insert(crate::world::zones::distance_fog(f.start, f.end, f.color));
        }
    }
    runlog::kv(
        "camera_spawned",
        &format!(
            "position={position} yaw={yaw:.3} pitch={pitch:.3} fov_deg={} vertical_fov_deg={:.2} sky_camera={:?}",
            settings.fov,
            vertical_fov(settings.fov).to_degrees(),
            sky.camera_position
        ),
    );
}

#[allow(clippy::type_complexity)] // Bevy system parameters
/// Keeps the sky camera turned like the main camera (times the sky zone's
/// own rotation, as KF does), and both cameras at the current view FOV.
pub fn follow_sky(
    fov: Res<ViewFov>,
    sky_info: Res<SkyInfo>,
    main: Query<&Transform, (With<FlyCamera>, Without<SkyCamera>)>,
    mut sky: Query<&mut Transform, With<SkyCamera>>,
    mut projections: Query<&mut Projection, Or<(With<FlyCamera>, With<SkyCamera>)>>,
) {
    let Ok(main) = main.single() else {
        return;
    };
    for mut t in &mut sky {
        t.rotation = sky_info.rotation * main.rotation;
    }
    let vertical = vertical_fov(fov.0);
    for mut proj in &mut projections {
        if let Projection::Perspective(p) = proj.as_mut()
            && p.fov != vertical
        {
            p.fov = vertical;
        }
    }
}

fn grab_cursor(
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    let Ok(mut cursor) = cursor.single_mut() else {
        return;
    };
    if mouse.just_pressed(MouseButton::Left) {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
}

pub fn look(
    motion: Res<AccumulatedMouseMotion>,
    cursor: Query<&CursorOptions, With<PrimaryWindow>>,
    mut cams: Query<(&mut Transform, &mut FlyCamera)>,
) {
    let grabbed = cursor.single().is_ok_and(|c| c.grab_mode != CursorGrabMode::None);
    if !grabbed || motion.delta == Vec2::ZERO {
        return;
    }
    for (mut t, mut cam) in &mut cams {
        let sensitivity = 0.002;
        cam.yaw -= motion.delta.x * sensitivity;
        cam.pitch = (cam.pitch - motion.delta.y * sensitivity).clamp(-1.54, 1.54);
        t.rotation = Quat::from_euler(EulerRot::YXZ, cam.yaw, cam.pitch, 0.0);
    }
}

fn fly(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<crate::player::walk::MoveMode>,
    mut cams: Query<(&mut Transform, &FlyCamera)>,
) {
    if *mode != crate::player::walk::MoveMode::Fly {
        return;
    }
    for (mut t, cam) in &mut cams {
        let mut dir = Vec3::ZERO;
        let (fwd, right) = (*t.forward(), *t.right());
        if keys.pressed(KeyCode::KeyW) {
            dir += fwd;
        }
        if keys.pressed(KeyCode::KeyS) {
            dir -= fwd;
        }
        if keys.pressed(KeyCode::KeyD) {
            dir += right;
        }
        if keys.pressed(KeyCode::KeyA) {
            dir -= right;
        }
        if keys.any_pressed([KeyCode::Space, KeyCode::KeyE]) {
            dir += Vec3::Y;
        }
        if keys.any_pressed([KeyCode::ControlLeft, KeyCode::KeyQ]) {
            dir -= Vec3::Y;
        }
        let boost = if keys.pressed(KeyCode::ShiftLeft) { 4.0 } else { 1.0 };
        t.translation += dir.normalize_or_zero() * cam.speed * boost * time.delta_secs();
    }
}

/// Camera position once per second, in Bevy metres and in Unreal units.
fn log_camera(time: Res<Time>, cams: Query<(&Transform, &FlyCamera)>, mut last: Local<f64>) {
    let now = time.elapsed_secs_f64();
    if now - *last < 1.0 {
        return;
    }
    *last = now;
    for (t, cam) in &cams {
        let p = t.translation / crate::engine::coords::SCALE;
        // Inverse of coords::pos: ue = (-z, x, y)
        runlog::kv(
            "camera",
            &format!(
                "bevy=({:.2}, {:.2}, {:.2}) unreal=({:.0}, {:.0}, {:.0}) yaw={:.3} pitch={:.3}",
                t.translation.x, t.translation.y, t.translation.z, -p.z, p.x, p.y, cam.yaw, cam.pitch
            ),
        );
    }
}
