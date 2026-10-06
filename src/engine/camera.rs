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
            .add_systems(Update, (grab_cursor, look, fly, follow_sky, log_camera).chain());
    }
}

/// The player's field of view as KF defines it: horizontal degrees at 4:3
/// (DefaultFOV 90; iron sights lower it). Applied to the main and sky cameras.
#[derive(Resource)]
pub struct ViewFov(pub f32);

impl Default for ViewFov {
    fn default() -> Self {
        ViewFov(DEFAULT_FOV)
    }
}

/// KFPlayerController DefaultFOV.
pub const DEFAULT_FOV: f32 = 90.0;

/// Vertical field of view for a KF horizontal FOV. KF uses
/// bUseTrueWideScreenFOV, so widescreen keeps the 4:3 vertical angle:
/// 90 -> 2 * atan(tan(45 deg) * 3/4) = 73.74 deg vertical.
pub fn vertical_fov(horizontal_deg: f32) -> f32 {
    2.0 * ((horizontal_deg / 2.0).to_radians().tan() * 0.75).atan()
}

fn kf_projection() -> Projection {
    Projection::from(PerspectiveProjection {
        fov: vertical_fov(DEFAULT_FOV),
        ..default()
    })
}

pub fn spawn_camera(
    mut commands: Commands,
    spawn: Res<SpawnPoint>,
    sky: Res<SkyInfo>,
    over: Option<Res<CameraOverride>>,
) {
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
        kf_projection(),
        // UE2 had no tonemapping: colours go to the screen as computed
        // (Bevy's default film curve darkened and greyed KF's lighting).
        bevy::core_pipeline::tonemapping::Tonemapping::None,
        Transform::from_translation(position).with_rotation(rotation),
        FlyCamera { yaw, pitch, speed: 8.0 },
    ));
    if let Some(sky_pos) = sky.camera_position {
        commands.spawn((
            Camera3d::default(),
            Camera { order: -1, ..default() },
            kf_projection(),
            bevy::core_pipeline::tonemapping::Tonemapping::None,
            Transform::from_translation(sky_pos).with_rotation(rotation),
            RenderLayers::layer(SKY_LAYER),
            SkyCamera,
        ));
    }
    runlog::kv(
        "camera_spawned",
        &format!(
            "position={position} yaw={yaw:.3} pitch={pitch:.3} vertical_fov_deg={:.2} sky_camera={:?}",
            vertical_fov(DEFAULT_FOV).to_degrees(),
            sky.camera_position
        ),
    );
}

#[allow(clippy::type_complexity)] // Bevy system parameters
/// Keeps the sky camera's rotation equal to the main camera's, and both
/// cameras at the current view FOV.
pub fn follow_sky(
    fov: Res<ViewFov>,
    main: Query<&Transform, (With<FlyCamera>, Without<SkyCamera>)>,
    mut sky: Query<&mut Transform, With<SkyCamera>>,
    mut projections: Query<&mut Projection, Or<(With<FlyCamera>, With<SkyCamera>)>>,
) {
    let Ok(main) = main.single() else {
        return;
    };
    for mut t in &mut sky {
        t.rotation = main.rotation;
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
