//! Screenshots to `work/screenshots/` (gitignored).
//!
//! - F12 saves one at any time.
//! - `--screenshot N[,M,...]` saves one at each listed frame, then quits once
//!   the last is on disk.
//!
//! Next to each PNG a `.txt` file holds the command that reproduces the exact
//! view (`--map ... --camera X,Y,Z,YAW,PITCH`), so a shot can be retaken after
//! a change.

use std::path::PathBuf;

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};

use crate::camera::FlyCamera;
use crate::coords::SCALE;
use crate::map::MapRequest;
use crate::runlog;

/// Frames at which to take automatic screenshots; quits after the last.
#[derive(Resource, Default)]
pub struct AutoScreenshot {
    pub at_frames: Vec<u32>,
}

/// Set by the capture observer once the last automatic screenshot is saved.
#[derive(Resource, Default)]
struct AutoScreenshotDone(bool);

pub struct ScreenshotPlugin;

impl Plugin for ScreenshotPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AutoScreenshot>()
            .init_resource::<AutoScreenshotDone>()
            .add_systems(Update, (take_screenshots, quit_after_auto_screenshot));
    }
}

/// The current view as command-line arguments for `--camera`.
pub fn camera_args(t: &Transform, cam: &FlyCamera) -> String {
    let p = t.translation / SCALE;
    // Inverse of coords::pos: unreal = (-z, x, y)
    format!("{:.0},{:.0},{:.0},{:.4},{:.4}", -p.z, p.x, p.y, cam.yaw, cam.pitch)
}

fn take_screenshots(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    frames: Res<FrameCount>,
    auto: Res<AutoScreenshot>,
    request: Res<MapRequest>,
    cams: Query<(&Transform, &FlyCamera)>,
    mut counter: Local<u32>,
) {
    let automatic = auto.at_frames.contains(&frames.0);
    let last_automatic = automatic && auto.at_frames.iter().max() == Some(&frames.0);
    if !keys.just_pressed(KeyCode::F12) && !automatic {
        return;
    }
    let Ok((t, cam)) = cams.single() else {
        return;
    };
    let dir = PathBuf::from("work/screenshots");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        error!("cannot create {}: {e}", dir.display());
        return;
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    *counter += 1;
    let name = format!("{}-{stamp}-{}", request.map, *counter);
    let png = dir.join(format!("{name}.png"));
    let args = camera_args(t, cam);
    let repro = format!("cargo run --release -- --map {} --camera {args} --screenshot 60", request.map);
    let _ = std::fs::write(dir.join(format!("{name}.txt")), format!("{repro}\n"));
    runlog::kv(
        "screenshot",
        &format!("file={} automatic={automatic} camera={args}", png.display()),
    );
    let mut shot = commands.spawn(Screenshot::primary_window());
    shot.observe(save_to_disk(png));
    if last_automatic {
        shot.observe(|_: On<ScreenshotCaptured>, mut done: ResMut<AutoScreenshotDone>| {
            done.0 = true;
        });
    }
}

fn quit_after_auto_screenshot(
    done: Res<AutoScreenshotDone>,
    mut exit: MessageWriter<AppExit>,
    mut waited: Local<u32>,
) {
    if done.0 {
        // Give the file write a few frames to finish before exiting.
        *waited += 1;
        if *waited > 5 {
            runlog::kv("screenshot_saved_quitting", "");
            exit.write(AppExit::Success);
        }
    }
}
