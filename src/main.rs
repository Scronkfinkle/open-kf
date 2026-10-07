mod engine;
mod world;
mod render;
mod player;
mod weapons;
mod zeds;
mod game;
mod audio;

use engine::{camera, record, runlog, screenshot, view_target};
use world::{collision, door, glass, map, nav, zones};
use render::{decals, overlay, particles};
use player::{armour, pain, walk};
use weapons::{bullet_fx, projectile, scope, weapon, zed_beam};
use zeds::{fireball, gore, vomit, zed};
use game::{buy_menu, combat, dosh, hud, shopkeeper, trader, trader_arrow, trader_path, waves, zed_time};
use audio::{map_sound, music, player_sound, trader_voice};
use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use ue_assets::install::Install;

const DEFAULT_MAP: &str = "KF-WestLondon";

/// Command-line options.
#[derive(Resource, Debug, Default)]
struct Args {
    /// Map to load, e.g. `KF-Farm`. Defaults to KF-WestLondon.
    map: Option<String>,
    /// Quit after this many frames. Used for automated test runs.
    frames: Option<u32>,
    /// Start pose, `X,Y,Z,YAW,PITCH` (Unreal units, radians).
    camera: Option<camera::CameraOverride>,
    /// Take screenshots at these frames, then quit.
    screenshot: Vec<u32>,
    /// Scripted input for tests: (frame, action).
    input: Vec<(u32, String)>,
    /// `--fly`: start flying (the map viewer; screenshots from a
    /// `--camera` pose) instead of walking. V switches during play.
    fly: bool,
    /// Hold "forward" for this many seconds (walk test).
    autowalk: Option<f32>,
    /// Spawn a Clot in front of the start position.
    zed: bool,
    /// Spawn a Gorefast in front of the start position.
    gorefast: bool,
    /// Test: killing hits on limbs always sever them.
    always_sever: bool,
    /// Test: where the start zed appears (Unreal X,Y,Z).
    zed_at: Option<[f32; 3]>,
    /// Spawn this kind of zed at the start (e.g. crawler).
    spawn: Option<String>,
    /// Start in god mode: the player takes no damage.
    god: bool,
    /// Test: extra weapons to carry ("all" or class names, comma-separated).
    give: Option<String>,
    /// Cap the frame rate (frames per second).
    fps: Option<f64>,
    /// `--window WxH`: ask for a window of this many pixels (scale factor
    /// 1), e.g. 1280x960 to compare with KF screenshots. The window
    /// manager may still resize it; the screenshot log has the real size.
    window: Option<(u32, u32)>,
    /// `--mute`: sounds are played (and logged) at zero volume.
    mute: bool,
    /// `--no-vsync`: frames do not wait for the display (test runs while
    /// the window cannot be shown, e.g. a locked screen, ran at 1 fps).
    no_vsync: bool,
    /// `--mode waves|debug` and `--length short|normal|long`.
    game: waves::GameOptions,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--map" => args.map = Some(it.next().ok_or("--map needs a name")?),
            "--window" => {
                let n = it.next().ok_or("--window needs WxH")?;
                let (w, h) = n.split_once('x').ok_or(format!("bad --window value: {n}"))?;
                args.window = Some((w.parse().map_err(|_| format!("bad --window width: {n}"))?, h.parse().map_err(|_| format!("bad --window height: {n}"))?));
            }
            "--frames" => {
                let n = it.next().ok_or("--frames needs a number")?;
                args.frames = Some(n.parse().map_err(|_| format!("bad --frames value: {n}"))?);
            }
            "--camera" => {
                args.camera = Some(camera::CameraOverride::parse(&it.next().ok_or("--camera needs X,Y,Z,YAW,PITCH")?)?)
            }
            "--screenshot" => {
                let n = it.next().ok_or("--screenshot needs frame numbers")?;
                args.screenshot = n
                    .split(',')
                    .map(|x| x.trim().parse().map_err(|_| format!("bad --screenshot value: {n}")))
                    .collect::<Result<_, _>>()?;
            }
            "--input" => {
                let n = it.next().ok_or("--input needs FRAME:ACTION,...")?;
                for item in n.split(',') {
                    let (f, a) = item.split_once(':').ok_or(format!("bad --input item: {item}"))?;
                    let f: u32 = f.trim().parse().map_err(|_| format!("bad --input frame: {item}"))?;
                    args.input.push((f, a.trim().to_ascii_lowercase()));
                }
            }
            "--mode" => {
                let n = it.next().ok_or("--mode needs waves or debug")?;
                args.game.mode = match n.as_str() {
                    "waves" => waves::GameMode::Waves,
                    "debug" => waves::GameMode::Debug,
                    _ => return Err(format!("bad --mode value: {n} (waves or debug)")),
                };
            }
            "--length" => {
                let n = it.next().ok_or("--length needs short, normal or long")?;
                args.game.length = waves::GameLength::parse(&n).ok_or(format!("bad --length value: {n}"))?;
            }
            "--wave" => {
                let n = it.next().ok_or("--wave needs a number")?;
                args.game.start_wave = Some(n.parse().map_err(|_| format!("bad --wave value: {n}"))?);
            }
            "--fly" => args.fly = true,
            "--mute" => args.mute = true,
            "--no-vsync" => args.no_vsync = true,
            "--god" => args.god = true,
            "--give" => args.give = Some(it.next().ok_or("--give needs \"all\" or weapon class names")?),
            "--zed" => args.zed = true,
            "--gorefast" => args.gorefast = true,
            "--always-sever" => args.always_sever = true,
            "--spawn" => args.spawn = Some(it.next().ok_or("--spawn needs a zed name")?),
            "--zed-at" => {
                let n = it.next().ok_or("--zed-at needs X,Y,Z")?;
                let v: Vec<f32> = n.split(',').map(|x| x.trim().parse().map_err(|_| format!("bad --zed-at value: {n}"))).collect::<Result<_, _>>()?;
                let [x, y, z] = v[..] else {
                    return Err(format!("--zed-at needs three numbers: {n}"));
                };
                args.zed_at = Some([x, y, z]);
            }
            "--fps" => {
                let n = it.next().ok_or("--fps needs a number")?;
                let v: f64 = n.parse().map_err(|_| format!("bad --fps value: {n}"))?;
                if !(1.0..=1000.0).contains(&v) {
                    return Err(format!("--fps must be between 1 and 1000: {n}"));
                }
                args.fps = Some(v);
            }
            "--autowalk" => {
                let n = it.next().ok_or("--autowalk needs seconds")?;
                args.autowalk = Some(n.parse().map_err(|_| format!("bad --autowalk value: {n}"))?);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(args)
}

fn main() -> AppExit {
    if let Err(e) = runlog::init() {
        eprintln!("warning: could not create {}: {e}", runlog::LOG_PATH);
    }

    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\nusage: open-kf [--map NAME] [--frames N] [--camera X,Y,Z,YAW,PITCH] [--screenshot F1,F2,..] [--input FRAME:ACTION,..] [--fly] [--autowalk SECONDS] [--zed] [--gorefast] [--always-sever] [--zed-at X,Y,Z] [--spawn NAME] [--god] [--give all|CLASS,..] [--fps N] [--window WxH] [--mode waves|debug] [--length short|normal|long] [--wave N] [--mute] [--no-vsync]");
            runlog::kv("error", &format!("reason=\"{e}\""));
            return AppExit::error();
        }
    };

    let install = match Install::discover() {
        Ok(i) => i,
        Err(e) => {
            eprintln!("error: {e}");
            runlog::kv("error", "reason=install_not_found");
            return AppExit::error();
        }
    };
    runlog::kv(
        "startup",
        &format!(
            "install=\"{}\" build=\"{}\" map={} frames_limit={}",
            install.root.display(),
            install.build_label,
            args.map.as_deref().unwrap_or("none"),
            args.frames.map_or("none".into(), |n| n.to_string()),
        ),
    );

    let map_name = args
        .map
        .clone()
        .unwrap_or_else(|| DEFAULT_MAP.to_string())
        .trim_end_matches(".rom")
        .to_string();
    let request = map::MapRequest {
        install_root: install.root.clone(),
        map: map_name,
    };

    let camera_override = args.camera;
    let auto_shot = screenshot::AutoScreenshot {
        at_frames: args.screenshot.clone(),
    };
    let scripted = weapon::ScriptedInput(args.input.clone());
    let loadout = args.give.as_deref().map(weapon::WeaponLoadout::parse).unwrap_or_default();
    let zed_settings = zed::ZedSettings {
        spawn_at_start: args.zed,
        gorefast_at_start: args.gorefast,
        always_sever: args.always_sever,
        spawn_at: args.zed_at,
        spawn_kind: args.spawn.clone(),
    };
    let args_god = args.god;
    if args.god {
        runlog::kv("god_mode", "on=true source=command_line");
    }
    if let Some(fps) = args.fps {
        runlog::kv("frame_limit", &format!("fps={fps}"));
    }
    let game_options = args.game;
    runlog::kv("game_options", &format!("mode={:?} length={:?}", game_options.mode, game_options.length));
    let walk_settings = walk::WalkSettings {
        start_walking: !args.fly,
        autowalk: args.autowalk,
    };

    let window_size = args.window;
    let present_mode = if args.no_vsync { bevy::window::PresentMode::AutoNoVsync } else { bevy::window::PresentMode::default() };
    let mut app = App::new();
    if let Some(c) = camera_override {
        app.insert_resource(c);
    }
    let exit = app
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(match window_size {
                Some((w, h)) => Window {
                    title: "Open KF".into(),
                    resolution: bevy::window::WindowResolution::new(w, h).with_scale_factor_override(1.0),
                    resizable: false,
                    present_mode,
                    ..default()
                },
                None => Window {
                    title: "Open KF".into(),
                    present_mode,
                    ..default()
                },
            }),
            ..default()
        })
        // Our own mixer (audio/mixer.rs) replaces Bevy's player.
        .disable::<bevy::audio::AudioPlugin>())
        .insert_resource(audio::mixer::AudioSettings { muted: args.mute })
        .insert_resource(args)
        .insert_resource(request)
        .insert_resource(ClearColor(Color::srgb(0.32, 0.36, 0.42)))
        .add_plugins((
            map::MapPlugin,
            camera::FlyCameraPlugin,
            screenshot::ScreenshotPlugin,
            record::RecordPlugin,
            collision::CollisionPlugin,
            walk::WalkPlugin,
            weapon::WeaponPlugin,
            zed::ZedPlugin,
            combat::CombatPlugin,
            gore::GorePlugin,
            particles::ParticlePlugin,
            decals::DecalPlugin,
            nav::NavPlugin,
            vomit::VomitPlugin,
            fireball::FireballPlugin,
        ))
        .add_plugins((bullet_fx::BulletFxPlugin, scope::ScopePlugin, projectile::ProjectilePlugin, zed_beam::ZedBeamPlugin, door::DoorPlugin, waves::GamePlugin, dosh::DoshPlugin, trader::TraderPlugin, buy_menu::BuyMenuPlugin, glass::GlassPlugin, zones::ZonesPlugin, pain::PainPlugin))
        .add_plugins((overlay::OverlayPlugin, armour::ArmourPlugin, trader_path::TraderPathPlugin, trader_arrow::TraderArrowPlugin, hud::HudPlugin, zed_time::ZedTimePlugin, view_target::ViewTargetPlugin, audio::mixer::AudioPlugin, player_sound::PlayerSoundPlugin, music::MusicPlugin, map_sound::MapSoundPlugin, trader_voice::TraderVoicePlugin))
        .add_plugins((player::hit_cam::HitCamPlugin, render::hit_blur::HitBlurPlugin, shopkeeper::ShopkeeperPlugin))
        .insert_resource(auto_shot)
        .insert_resource(walk_settings)
        .insert_resource(game_options)
        .insert_resource(scripted)
        .insert_resource(loadout)
        .insert_resource(zed_settings)
        .insert_resource(combat::PlayerHealth {
            god: args_god,
            ..default()
        })
        .add_systems(Startup, setup)
        .add_systems(Update, (log_frame_stats, quit_after_frame_limit))
        .add_systems(Last, limit_frame_rate)
        .run();

    runlog::kv("shutdown", &format!("exit={exit:?}"));
    exit
}

/// `--fps N`: at the end of each frame, wait until the next frame is due
/// (sleep, then spin the last millisecond for precision). With vsync (the
/// default) the monitor's refresh rate is still the upper limit.
fn limit_frame_rate(args: Res<Args>, mut next: Local<Option<std::time::Instant>>) {
    let Some(fps) = args.fps else {
        return;
    };
    let frame = std::time::Duration::from_secs_f64(1.0 / fps);
    let now = std::time::Instant::now();
    let due = next.unwrap_or(now);
    if due > now {
        let wait = due - now;
        if wait > std::time::Duration::from_millis(2) {
            std::thread::sleep(wait - std::time::Duration::from_millis(1));
        }
        while std::time::Instant::now() < due {
            std::hint::spin_loop();
        }
    }
    // Keep a steady pace; after a slow frame, start again from now
    // instead of rushing to catch up.
    let after = std::time::Instant::now();
    *next = Some(if after > due + frame { after + frame } else { due + frame });
}

fn setup() {
    runlog::kv("window_ready", "");
}

/// Logs frame count and average frame time once per second.
fn log_frame_stats(
    // Wall-clock time: Bevy's default clock caps each frame at 250 ms, which
    // hid a 1 s-per-frame stall once.
    time: Res<Time<Real>>,
    frames: Res<FrameCount>,
    mut last: Local<(f64, u32)>,
) {
    let now = time.elapsed_secs_f64();
    let (last_t, last_frames) = *last;
    if now - last_t >= 1.0 {
        let n = frames.0 - last_frames;
        let ms = if n > 0 { (now - last_t) * 1000.0 / n as f64 } else { 0.0 };
        runlog::kv("frame_stats", &format!("frame={} fps={n} frame_ms={ms:.2}", frames.0));
        *last = (now, frames.0);
    }
}

fn quit_after_frame_limit(
    args: Res<Args>,
    frames: Res<FrameCount>,
    mut exit: MessageWriter<AppExit>,
) {
    if args.frames.is_some_and(|limit| frames.0 >= limit) {
        runlog::kv("frame_limit_reached", &format!("frame={}", frames.0));
        exit.write(AppExit::Success);
    }
}
