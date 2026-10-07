mod engine;
mod world;
mod render;
mod player;
mod weapons;
mod zeds;
mod game;
mod audio;
mod net;
mod launcher;

use engine::{camera, graphics, record, runlog, screenshot, view_target};
use world::{collision, door, glass, map, nav, zones};
use render::{decals, overlay, particles};
use player::{armour, pain, walk};
use weapons::{bullet_fx, projectile, scope, weapon, zed_beam};
use zeds::{fireball, gore, vomit, zed};
use game::{buy_menu, combat, dosh, end_game, hud, perks, shopkeeper, trader, trader_arrow, trader_path, waves, zed_time};
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
    /// `--display windowed|borderless|fullscreen` (engine/graphics.rs).
    display: graphics::DisplayMode,
    /// `--fov DEG`: the player's field of view (KF's degrees at 4:3).
    fov: Option<u32>,
    /// `--brightness PERCENT`: the 3D view's light (100: unchanged).
    brightness: Option<u32>,
    /// `--msaa 0|2|4|8`: anti-aliasing samples (0: off).
    msaa: Option<u32>,
    /// `--anisotropy 1|2|4|8|16`: map texture filtering.
    anisotropy: Option<u16>,
    /// `--mode waves|debug` and `--length short|normal|long`.
    game: waves::GameOptions,
    /// Were `--mode` / `--length` typed (a joiner takes the host's and
    /// says so when they differ from typed ones).
    mode_given: bool,
    length_given: bool,
    /// `--character NAME`: a KF character (System/*.upl); default Corporal_Lewis.
    character: Option<String>,
    /// `--behind-view`: start in behind view (KF's BehindView command; F4
    /// toggles), to see the player's own body.
    behind_view: bool,
    /// `--behind-yaw DEG`: turn the behind-view camera around the pawn
    /// (KF's free camera, CameraDeltaRotation.Yaw; 180 = from the front).
    behind_yaw: f32,
    /// `--perk NAME` and `--perk-level N` (0-6).
    perk: perks::PerkOptions,
    /// `--lobby` (Some(true)) / `--no-lobby` (Some(false)); else the rule
    /// in `lobby_settings`.
    lobby: Option<bool>,
    /// `--name NAME`: the player's name in the lobby.
    name: Option<String>,
    /// `--host [PORT]` / `--join ADDR[:PORT]` (experimental multiplayer).
    net: net::NetMode,
    /// `--log FILE` (read before parsing, see runlog::path_from_args).
    log: Option<String>,
    /// `--trader-menu nu|kf`: our NuMenu (default) or the KF-style list.
    trader_menu: buy_menu::MenuKind,
}

/// When the game opens in KF's lobby (DESIGN.md, "Menus"): `--lobby` /
/// `--no-lobby` decide; otherwise only in waves mode and only when no test
/// flag (`--frames`, `--screenshot`, `--input`) is given, so test runs
/// still start straight in the game.
fn lobby_settings(args: &Args) -> game::menus::LobbySettings {
    let (open, reason) = match args.lobby {
        // A network game always starts in the lobby (parse_args refuses
        // --no-lobby with --host / --join).
        _ if args.net.active() => (true, "net_game"),
        Some(true) => (true, "flag_lobby"),
        Some(false) => (false, "flag_no_lobby"),
        None if args.game.mode != waves::GameMode::Waves => (false, "not_waves_mode"),
        None if args.frames.is_some() || !args.screenshot.is_empty() || !args.input.is_empty() => (false, "test_run"),
        None => (true, "waves_mode"),
    };
    game::menus::LobbySettings { open, reason, name: args.name.clone() }
}

/// Reads the command-line options (without the program name). Also used
/// by the launcher to check the options it built before starting the game.
fn parse_args(list: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut args = Args::default();
    let mut it = list.into_iter().peekable();
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
                args.game.mode = waves::GameMode::parse(&n).ok_or(format!("bad --mode value: {n} (waves or debug)"))?;
                args.mode_given = true;
            }
            "--length" => {
                let n = it.next().ok_or("--length needs short, normal or long")?;
                args.game.length = waves::GameLength::parse(&n).ok_or(format!("bad --length value: {n}"))?;
                args.length_given = true;
            }
            "--wave" => {
                let n = it.next().ok_or("--wave needs a number")?;
                args.game.start_wave = Some(n.parse().map_err(|_| format!("bad --wave value: {n}"))?);
            }
            "--fly" => args.fly = true,
            "--behind-view" => args.behind_view = true,
            "--behind-yaw" => {
                let n = it.next().ok_or("--behind-yaw needs degrees")?;
                args.behind_yaw = n.parse().map_err(|_| format!("bad --behind-yaw value: {n}"))?;
            }
            "--character" => args.character = Some(it.next().ok_or("--character needs a name, e.g. Mr_Foster")?),
            "--perk" => {
                let n = it.next().ok_or("--perk needs a name (medic, support, sharpshooter, commando, berserker, firebug, demolitions)")?;
                args.perk.perk = if n.eq_ignore_ascii_case("none") {
                    None
                } else {
                    Some(perks::Perk::parse(&n).ok_or(format!("bad --perk value: {n} (medic, support, sharpshooter, commando, berserker, firebug, demolitions or none)"))?)
                };
            }
            "--perk-level" => {
                let n = it.next().ok_or("--perk-level needs a number 0-6")?;
                let l: u8 = n.parse().map_err(|_| format!("bad --perk-level value: {n}"))?;
                if l > 6 {
                    return Err(format!("--perk-level must be 0 to 6: {n}"));
                }
                args.perk.level = l;
            }
            "--lobby" => args.lobby = Some(true),
            "--no-lobby" => args.lobby = Some(false),
            "--name" => args.name = Some(it.next().ok_or("--name needs a player name")?),
            "--host" => {
                // The port is optional: take the next argument only if it is a number.
                let port = match it.peek().map(|n| n.parse::<u16>()) {
                    Some(Ok(p)) => {
                        it.next();
                        Some(p)
                    }
                    _ => None,
                };
                if args.net.active() {
                    return Err("--host and --join cannot be used together".into());
                }
                args.net = net::NetMode::host(port);
            }
            "--join" => {
                let a = it.next().ok_or("--join needs an address, e.g. 127.0.0.1 or 192.168.1.20:7707")?;
                if args.net.active() {
                    return Err("--host and --join cannot be used together".into());
                }
                args.net = net::NetMode::join(&a)?;
            }
            "--log" => args.log = Some(it.next().ok_or("--log needs a file name")?),
            "--mute" => args.mute = true,
            "--trader-menu" => {
                let n = it.next().ok_or("--trader-menu needs nu or kf")?;
                args.trader_menu = buy_menu::MenuKind::parse(&n).ok_or(format!("bad --trader-menu value: {n} (nu or kf)"))?;
            }
            "--no-vsync" => args.no_vsync = true,
            "--display" => {
                let n = it.next().ok_or("--display needs windowed, borderless or fullscreen")?;
                args.display = graphics::DisplayMode::parse(&n).ok_or(format!("bad --display value: {n} (windowed, borderless or fullscreen)"))?;
            }
            "--fov" => args.fov = Some(graphics::parse_fov(&it.next().ok_or("--fov needs degrees")?)?),
            "--brightness" => args.brightness = Some(graphics::parse_brightness(&it.next().ok_or("--brightness needs a percentage")?)?),
            "--msaa" => args.msaa = Some(graphics::parse_msaa(&it.next().ok_or("--msaa needs 0, 2, 4 or 8")?)?),
            "--anisotropy" => args.anisotropy = Some(graphics::parse_anisotropy(&it.next().ok_or("--anisotropy needs 1, 2, 4, 8 or 16")?)?),
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
    if args.net.active() && args.lobby == Some(false) {
        return Err("--no-lobby cannot be used with --host / --join (a network game starts in the lobby)".into());
    }
    Ok(args)
}

fn main() -> AppExit {
    // No arguments (or `--launcher` first): the launcher window, which
    // starts the game again with the options picked (launcher/mod.rs).
    let raw: Vec<String> = std::env::args().skip(1).collect();
    if launcher::wanted(&raw) {
        return launcher::run(&raw);
    }
    let log_path = runlog::path_from_args(&std::env::args().collect::<Vec<_>>());
    if let Err(e) = runlog::init(&log_path) {
        eprintln!("warning: could not create {log_path}: {e}");
    }

    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\nusage: open-kf [--map NAME] [--frames N] [--camera X,Y,Z,YAW,PITCH] [--screenshot F1,F2,..] [--input FRAME:ACTION,..] [--fly] [--autowalk SECONDS] [--zed] [--gorefast] [--always-sever] [--zed-at X,Y,Z] [--spawn NAME] [--god] [--give all|CLASS,..] [--fps N] [--window WxH] [--display windowed|borderless|fullscreen] [--fov DEG] [--brightness PERCENT] [--msaa 0|2|4|8] [--anisotropy 1|2|4|8|16] [--mode waves|debug] [--length short|normal|long] [--wave N] [--mute] [--no-vsync] [--character NAME] [--behind-view] [--behind-yaw DEG] [--perk NAME] [--perk-level 0-6] [--lobby | --no-lobby] [--name NAME] [--host [PORT] | --join ADDR[:PORT]] [--trader-menu nu|kf] [--log FILE]");
            runlog::kv("error", &format!("reason=\"{e}\""));
            return AppExit::error();
        }
    };

    // `--join`: ask the host what it plays before anything is loaded.
    let mut args = args;
    if let Err(e) = ask_host(&mut args) {
        eprintln!("error: {e}");
        return AppExit::error();
    }

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
    let character = player::character::CharacterChoice(args.character.clone());
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
    let trader_menu = args.trader_menu;
    runlog::kv("trader_menu", &format!("kind={}", trader_menu.word()));
    let lobby = lobby_settings(&args);
    let veterancy = perks::Veterancy::from_options(args.perk);
    let net_plugin = net::NetPlugin {
        mode: args.net.clone(),
        info: net::query::HostInfo { map: request.map.clone(), mode: format!("{:?}", game_options.mode), length: format!("{:?}", game_options.length), ..default() },
    };
    runlog::kv("game_options", &format!("mode={:?} length={:?}", game_options.mode, game_options.length));
    let walk_settings = walk::WalkSettings {
        start_walking: !args.fly,
        autowalk: args.autowalk,
    };

    let (behind_view, behind_yaw) = (args.behind_view, args.behind_yaw);
    let graphics_settings = graphics::GraphicsSettings {
        display: args.display,
        resolution: args.window,
        vsync: !args.no_vsync,
        fov: args.fov.unwrap_or(graphics::DEFAULT_FOV) as f32,
        brightness: args.brightness.unwrap_or(graphics::DEFAULT_BRIGHTNESS),
        msaa: args.msaa.unwrap_or(graphics::DEFAULT_MSAA),
        anisotropy: args.anisotropy.unwrap_or(graphics::DEFAULT_ANISOTROPY),
    };
    graphics_settings.log(args.fps);
    let mut app = App::new();
    if let Some(c) = camera_override {
        app.insert_resource(c);
    }
    let exit = app
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(graphics_settings.window()),
            ..default()
        })
        // Our own mixer (audio/mixer.rs) replaces Bevy's player.
        .disable::<bevy::audio::AudioPlugin>())
        .insert_resource(graphics_settings)
        .add_plugins(graphics::GraphicsPlugin)
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
        .add_plugins((player::hit_cam::HitCamPlugin, render::hit_blur::HitBlurPlugin, shopkeeper::ShopkeeperPlugin, end_game::EndGamePlugin))
        .add_plugins((perks::PerksPlugin, game::healing::HealingPlugin, player::body::BodyPlugin, game::menus::MenusPlugin, game::pickups::PickupPlugin, net_plugin))
        .insert_resource(view_target::ViewTarget::starting_behind(behind_view, behind_yaw))
        .insert_resource(lobby)
        .insert_resource(auto_shot)
        .insert_resource(walk_settings)
        .insert_resource(game_options)
        .insert_resource(veterancy)
        .insert_resource(scripted)
        .insert_resource(buy_menu::BuyMenu::with_kind(trader_menu))
        .insert_resource(loadout)
        .insert_resource(character)
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

/// How long a joiner waits for the host's query answer: tries, and
/// seconds per try (3 s in all).
const QUERY_TRIES: u32 = 5;
const QUERY_WAIT: std::time::Duration = std::time::Duration::from_millis(600);

/// `--join`: asks the host's query port (game port + 1, net/query.rs)
/// for its map, mode and length and uses them, so `--join ADDR` alone is
/// enough. The host wins over typed `--map` / `--mode` / `--length` (a
/// note is printed when they differ). No answer: an error, unless `--map`
/// was typed (then the game goes on with the typed options, as before the
/// query existed, e.g. when only the game port is open in a firewall).
fn ask_host(args: &mut Args) -> Result<(), String> {
    let net::NetMode::Client { server } = args.net else { return Ok(()) };
    let Some(qport) = net::query::query_port(server.port()) else {
        return Err(format!("--join port {} has no query port after it", server.port()));
    };
    let qaddr = std::net::SocketAddr::new(server.ip(), qport);
    let host = match net::query::ask(qaddr, QUERY_TRIES, QUERY_WAIT) {
        Ok((info, took)) => {
            runlog::kv(
                "net_query_answer",
                &format!(
                    "from={qaddr} after_ms={} host_map={} mode={} length={} players={} max_players={} match_started={} protocol={:#x} game_port={}",
                    took.as_millis(),
                    info.map,
                    info.mode,
                    info.length,
                    info.players,
                    info.max_players,
                    info.match_started,
                    info.protocol,
                    info.game_port
                ),
            );
            info
        }
        Err(e) => {
            let secs = QUERY_WAIT.as_secs_f32() * QUERY_TRIES as f32;
            runlog::kv("net_query_failed", &format!("side=client to={qaddr} tries={QUERY_TRIES} seconds={secs:.1} reason=\"{e}\" map_given={}", args.map.is_some()));
            if let Some(m) = &args.map {
                eprintln!("warning: no answer from a host at {qaddr} (query port) after {secs:.0} s ({e}); trying to join anyway with --map {m}");
                return Ok(());
            }
            return Err(format!(
                "no answer from a host at {} (asked its query port {qport} for {secs:.0} s: {e}).\n  Is the host started with --host, and is the address / port right? A host on port P answers on UDP port P+1, so both must be reachable.\n  (To join without asking, add --map NAME --mode waves.)",
                server.ip()
            ));
        }
    };
    if host.protocol != net::PROTOCOL_ID {
        runlog::kv("net_query_failed", &format!("side=client reason=protocol_mismatch host_protocol={:#x} my_protocol={:#x}", host.protocol, net::PROTOCOL_ID));
        return Err(format!("the host at {} runs a different version of Open KF's network code (protocol {:#x}, this game {:#x}); both need the same version", server.ip(), host.protocol, net::PROTOCOL_ID));
    }
    if host.players as usize >= net::MAX_PLAYERS {
        println!("note: the host at {} has {} of {} players; it may refuse you", server.ip(), host.players, host.max_players);
    }
    // The host's choices win; say so where the player typed others.
    let note = |what: &str, typed: Option<String>, hosts: &str| {
        let differs = typed.as_ref().is_some_and(|t| !t.eq_ignore_ascii_case(hosts));
        if differs {
            println!("note: the host plays {what} {hosts}; using that instead of your --{what} {}", typed.as_deref().unwrap_or(""));
        }
        runlog::kv("net_query_override", &format!("what={what} host={hosts} typed={} differs={differs}", typed.as_deref().unwrap_or("none")));
    };
    note("map", args.map.clone(), &host.map);
    args.map = Some(host.map.clone());
    match waves::GameMode::parse(&host.mode) {
        Some(m) => {
            note("mode", args.mode_given.then(|| format!("{:?}", args.game.mode)), &host.mode);
            args.game.mode = m;
        }
        None => runlog::kv("net_query_override", &format!("what=mode host={} reason=unknown_kept_mine mine={:?}", host.mode, args.game.mode)),
    }
    match waves::GameLength::parse(&host.length) {
        Some(l) => {
            note("length", args.length_given.then(|| format!("{:?}", args.game.length)), &host.length);
            args.game.length = l;
        }
        None => runlog::kv("net_query_override", &format!("what=length host={} reason=unknown_kept_mine mine={:?}", host.length, args.game.length)),
    }
    if host.match_started {
        println!("note: the match at {} has already started", server.ip());
    }
    Ok(())
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
