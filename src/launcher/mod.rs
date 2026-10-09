//! The launcher: a KF-style window to choose how to play, opened when the
//! program starts with no arguments (or `--launcher`). PLAY starts the
//! program again with the matching command-line options and waits for
//! it. See DESIGN.md, "The launcher".

pub mod choices;
mod draw;

use std::sync::{Arc, Mutex};

use bevy::clipboard::Clipboard;
use bevy::diagnostic::FrameCount;
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardFocusLost, KeyboardInput};
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk};
use bevy::window::PrimaryWindow;
use ue_assets::install::Install;

use crate::engine::runlog;
use crate::game::menus::gui::{self, Gui, MenuSlot, POOL};
use choices::{Choices, PlayType, command_line};

/// The launcher's log (the game keeps `logs/latest.log`).
const LOG_PATH: &str = "logs/launcher.log";
/// The saved choices, next to `logs/` (gitignored: not in the whitelist).
/// The game reads and writes its volume lines too (`read_volumes`,
/// `save_volumes`), its aim line (`read_aim`, `save_aim`) and its mouse
/// lines (`read_mouse`, `save_mouse`).
pub const SETTINGS_PATH: &str = "settings/launcher.txt";

/// The volumes saved in the settings file, and whether the file had them
/// ("file") or not ("default": no file or no volume lines).
pub fn read_volumes(path: &std::path::Path) -> (choices::Volumes, &'static str) {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let has = text.lines().any(|l| l.split_once('=').is_some_and(|(k, _)| choices::VOLUME_FIELDS.contains(&k.trim())));
            (Choices::from_text(&text).0.volumes, if has { "file" } else { "default" })
        }
        Err(_) => (choices::Volumes::default(), "default"),
    }
}

/// The game's save: rewrites only the volume lines of the settings file
/// (the launcher's other choices stay), creating it if needed.
pub fn save_volumes(path: &std::path::Path, v: &choices::Volumes) -> Result<(), String> {
    let old = std::fs::read_to_string(path).unwrap_or_default();
    let new = choices::with_volume_lines(&old, v);
    path.parent()
        .filter(|d| !d.as_os_str().is_empty())
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|_| std::fs::write(path, new))
        .map_err(|e| e.to_string())
}

/// The mouse settings saved in the settings file (sensitivity, invert),
/// and whether the file had a mouse line ("file") or not ("default":
/// KF's sensitivity 3, not inverted).
pub fn read_mouse(path: &std::path::Path) -> ((f32, bool), &'static str) {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let has = text.lines().any(|l| l.split_once('=').is_some_and(|(k, _)| choices::MOUSE_FIELDS.contains(&k.trim())));
            let c = Choices::from_text(&text).0;
            ((c.mouse_sensitivity, c.invert_mouse), if has { "file" } else { "default" })
        }
        Err(_) => ((choices::SENSITIVITY_DEFAULT, false), "default"),
    }
}

/// The game's save: rewrites only the mouse lines of the settings file.
pub fn save_mouse(path: &std::path::Path, sensitivity: f32, invert: bool) -> Result<(), String> {
    let old = std::fs::read_to_string(path).unwrap_or_default();
    let new = choices::with_mouse_lines(&old, sensitivity, invert);
    path.parent()
        .filter(|d| !d.as_os_str().is_empty())
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|_| std::fs::write(path, new))
        .map_err(|e| e.to_string())
}

/// The aim setting saved in the settings file (true = hold), and whether
/// the file had it ("file") or not ("default": KF's toggle).
pub fn read_aim(path: &std::path::Path) -> (bool, &'static str) {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let has = text.lines().any(|l| l.split_once('=').is_some_and(|(k, _)| k.trim() == choices::AIM_FIELD));
            (Choices::from_text(&text).0.aim_hold, if has { "file" } else { "default" })
        }
        Err(_) => (false, "default"),
    }
}

/// The game's save: rewrites only the `aim=` line of the settings file.
pub fn save_aim(path: &std::path::Path, hold: bool) -> Result<(), String> {
    let old = std::fs::read_to_string(path).unwrap_or_default();
    let new = choices::with_aim_line(&old, hold);
    path.parent()
        .filter(|d| !d.as_os_str().is_empty())
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|_| std::fs::write(path, new))
        .map_err(|e| e.to_string())
}

/// Does this command line open the launcher? Nothing at all, or
/// `--launcher` first. Any other argument runs the game directly.
pub fn wanted(args: &[String]) -> bool {
    args.is_empty() || args[0] == "--launcher"
}

/// The launcher's own options (after `--launcher`), mostly for tests.
#[derive(Resource, Default, Debug, Clone)]
pub struct Options {
    /// `--input FRAME:ACTION,...`: `click:ID`, `set:FIELD=VALUE`,
    /// `type:TEXT`, `key:tab|enter|backspace|escape`, `dump`.
    input: Vec<(u32, String)>,
    /// `--dry-run`: PLAY logs the command and quits without starting it.
    dry_run: bool,
    /// `--mute`: the game started gets `--mute` too.
    mute: bool,
    screenshot: Vec<u32>,
    frames: Option<u32>,
    window: Option<(u32, u32)>,
    log: Option<String>,
    /// `--settings FILE`: where the choices are read and saved.
    settings: Option<String>,
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut o = Options::default();
    let mut it = args.iter().skip(1);
    let num = |v: Option<&String>, what: &str| -> Result<u32, String> { v.ok_or(format!("{what} needs a number"))?.parse().map_err(|_| format!("bad {what} value")) };
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dry-run" => o.dry_run = true,
            "--mute" => o.mute = true,
            "--frames" => o.frames = Some(num(it.next(), "--frames")?),
            "--screenshot" => {
                let n = it.next().ok_or("--screenshot needs frame numbers")?;
                o.screenshot = n.split(',').map(|x| x.trim().parse().map_err(|_| format!("bad --screenshot value: {n}"))).collect::<Result<_, _>>()?;
            }
            "--window" => {
                let n = it.next().ok_or("--window needs WxH")?;
                let (w, h) = n.split_once('x').ok_or(format!("bad --window value: {n}"))?;
                o.window = Some((w.parse().map_err(|_| format!("bad --window value: {n}"))?, h.parse().map_err(|_| format!("bad --window value: {n}"))?));
            }
            "--input" => {
                // Not lower-cased (names and addresses keep their case).
                let n = it.next().ok_or("--input needs FRAME:ACTION,...")?;
                for item in n.split(',') {
                    let (f, act) = item.split_once(':').ok_or(format!("bad --input item: {item}"))?;
                    o.input.push((f.trim().parse().map_err(|_| format!("bad --input frame: {item}"))?, act.trim().to_string()));
                }
            }
            "--log" => o.log = Some(it.next().ok_or("--log needs a file name")?.clone()),
            "--settings" => o.settings = Some(it.next().ok_or("--settings needs a file name")?.clone()),
            other => return Err(format!("unknown launcher option: {other} (the launcher takes --dry-run --mute --frames N --screenshot N,.. --window WxH --input FRAME:ACTION,.. --log FILE --settings FILE)")),
        }
    }
    Ok(o)
}

impl Options {
    fn settings_path(&self) -> std::path::PathBuf {
        self.settings.as_deref().unwrap_or(SETTINGS_PATH).into()
    }
}

/// The saved choices, or the defaults when there are none.
fn load_choices(path: &std::path::Path) -> Choices {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let (c, bad) = Choices::from_text(&text);
            runlog::kv("launcher_settings_loaded", &format!("file={} unusable_lines={} [{}]", path.display(), bad.len(), bad.join(" | ")));
            c
        }
        Err(e) => {
            runlog::kv("launcher_settings_none", &format!("file={} reason=\"{e}\"", path.display()));
            Choices::default()
        }
    }
}

/// Writes the choices (on PLAY).
fn save_choices(path: &std::path::Path, c: &Choices) {
    let r = path.parent().filter(|d| !d.as_os_str().is_empty()).map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(path, c.to_text()));
    match r {
        Ok(()) => runlog::kv("launcher_settings_saved", &format!("file={}", path.display())),
        Err(e) => runlog::kv("launcher_settings_save_failed", &format!("file={} reason=\"{e}\"", path.display())),
    }
}

/// The text fields, in Tab order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Port,
    Address,
    Name,
    Extra,
}

impl Field {
    const ALL: [Field; 4] = [Field::Port, Field::Address, Field::Name, Field::Extra];

    fn key(self) -> &'static str {
        match self {
            Field::Port => "port",
            Field::Address => "address",
            Field::Name => "name",
            Field::Extra => "extra",
        }
    }

    fn parse(s: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|f| f.key() == s)
    }

    /// Is the field on screen for this play type?
    fn shown(self, play: PlayType) -> bool {
        match self {
            Field::Port => play == PlayType::Host,
            Field::Address => play == PlayType::Join,
            _ => true,
        }
    }

    fn text(self, c: &mut Choices) -> &mut String {
        match self {
            Field::Port => &mut c.port,
            Field::Address => &mut c.address,
            Field::Name => &mut c.name,
            Field::Extra => &mut c.extra,
        }
    }

    /// The most characters the field takes (typed or pasted). Name: KF's
    /// name box (ROTab_GameSettings MaxWidth=16); the rest our choice.
    fn max_len(self) -> usize {
        match self {
            Field::Port => 5,
            Field::Address => 100,
            Field::Name => 16,
            Field::Extra => 1000,
        }
    }

    /// Can the field hold this character? Name: KF's settings page removes
    /// quotes. Address: IPv4, host names, `host:port`.
    fn allows(self, c: char) -> bool {
        match self {
            Field::Port => c.is_ascii_digit(),
            Field::Address => c.is_ascii_alphanumeric() || ".:-_[]".contains(c),
            Field::Name => !c.is_control() && c != '"',
            Field::Extra => !c.is_control(),
        }
    }

    /// Typed characters: added at the end while allowed and room is left.
    fn type_text(self, text: &mut String, typed: &str) {
        for c in typed.chars().filter(|&c| self.allows(c)) {
            if text.chars().count() < self.max_len() {
                text.push(c);
            }
        }
    }
}

/// What a paste did to a field.
#[derive(Debug, PartialEq)]
pub struct Pasted {
    /// The field's new text.
    pub text: String,
    /// The field's text was replaced (port, address) or added to.
    pub replaced: bool,
    /// Characters kept, dropped as not allowed, cut by the length limit.
    pub kept: usize,
    pub dropped: usize,
    pub cut: usize,
}

/// The field's text after pasting `clip` into it (DESIGN.md, "Pasting
/// into the launcher's text fields"): one line (extra: all lines joined),
/// trimmed, only allowed characters, up to the length limit. Port and
/// address are replaced, name and extra get the paste at the end.
pub fn paste_text(f: Field, current: &str, clip: &str) -> Pasted {
    let line = match f {
        Field::Extra => clip.split_whitespace().collect::<Vec<_>>().join(" "),
        _ => clip.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string(),
    };
    // `host:port` pasted into the host's port: the port.
    let line = match (f, line.rsplit_once(':')) {
        (Field::Port, Some((_, p))) => p.trim().to_string(),
        _ => line,
    };
    let replaced = matches!(f, Field::Port | Field::Address);
    let mut text = if replaced { String::new() } else { current.to_string() };
    // Extra arguments: a space between the old ones and the pasted ones.
    if f == Field::Extra && !line.is_empty() && !text.is_empty() && !text.ends_with(' ') && text.chars().count() < f.max_len() {
        text.push(' ');
    }
    let (mut kept, mut dropped, mut cut) = (0, 0, 0);
    for c in line.chars() {
        if !f.allows(c) {
            dropped += 1;
        } else if text.chars().count() >= f.max_len() {
            cut += 1;
        } else {
            text.push(c);
            kept += 1;
        }
    }
    Pasted { text, replaced, kept, dropped, cut }
}

/// Pastes the clipboard into a field (and selects it). `via`: what asked
/// (for the log).
fn paste(l: &mut Launcher, clipboard: &mut Clipboard, f: Field, via: &str) {
    l.focus = Some(f);
    let clip = match clipboard.fetch_text().poll_result() {
        Some(Ok(t)) => t,
        Some(Err(e)) => {
            runlog::kv("launcher_paste_failed", &format!("field={} via={via} reason=\"{e}\"", f.key()));
            l.status = format!("Cannot read the clipboard ({e}).");
            return;
        }
        None => {
            runlog::kv("launcher_paste_failed", &format!("field={} via={via} reason=no_answer", f.key()));
            return;
        }
    };
    let text = f.text(&mut l.choices);
    let p = paste_text(f, text, &clip);
    runlog::kv(
        "launcher_paste",
        &format!("field={} via={via} mode={} clipboard_chars={} kept={} dropped={} cut={} value=\"{}\"", f.key(), if p.replaced { "replace" } else { "append" }, clip.chars().count(), p.kept, p.dropped, p.cut, p.text),
    );
    if p.kept == 0 {
        l.status = "Nothing usable to paste in the clipboard.".into();
        return;
    }
    *text = p.text;
}

/// Everything the launcher shows and changes.
#[derive(Resource, Default)]
pub struct Launcher {
    pub choices: Choices,
    /// The install's playable maps, sorted.
    pub maps: Vec<String>,
    /// (name, portrait texture path), KF's Select Character list.
    pub characters: Vec<(String, String)>,
    pub focus: Option<Field>,
    /// The map list's first shown row, and how many rows fit (draw sets it).
    pub map_top: usize,
    pub map_rows: usize,
    /// Scroll the list to the chosen map on the next draw.
    pub reveal_map: bool,
    /// A message under the buttons (why PLAY did nothing, ...).
    pub status: String,
    /// The choices last logged (to log only what changed).
    logged: Option<Choices>,
    /// CHECK HOST: the answer being waited for, and what it said (with
    /// the address it was for; a new address clears it).
    host_wait: Option<HostAnswer>,
    pub host_text: Option<(String, String)>,
    /// The install's KillingFloor.ini SoundVolume and MusicVolume (what
    /// the Effects and Music sliders show while unset).
    pub ini_volumes: (f32, f32),
    /// The volume slider the mouse holds (index into `choices::SLIDERS`).
    pub volume_drag: Option<usize>,
    /// Graphics: the sizes the Resolution row offers (the primary
    /// monitor's, largest first; `WINDOW_SIZES` until it is known or if it
    /// reports none) and the desktop's size.
    pub resolutions: Vec<(u32, u32)>,
    pub desktop: Option<(u32, u32)>,
}

type HostAnswer = Arc<Mutex<Option<Result<(u16, crate::net::query::HostInfo), String>>>>;

/// CHECK HOST: asks the host's query port from a background thread
/// (net/query.rs; up to 3 tries of 0.5 s), so the window keeps drawing.
fn start_host_check(l: &mut Launcher) {
    let addr = l.choices.address.trim().to_string();
    if addr.is_empty() {
        l.status = "Type the host's address first.".into();
        return;
    }
    let answer: HostAnswer = Arc::default();
    let slot = answer.clone();
    runlog::kv("launcher_host_check", &format!("address={addr} state=asking"));
    l.host_text = Some((addr.clone(), format!("Asking {addr}...")));
    std::thread::spawn(move || {
        let r = (|| {
            let crate::net::NetMode::Client { server } = crate::net::NetMode::join(&addr)? else {
                return Err("not an address".to_string());
            };
            let q = crate::net::query::query_port(server.port()).ok_or("the port has no query port after it")?;
            let (info, _) = crate::net::query::ask(std::net::SocketAddr::new(server.ip(), q), 3, std::time::Duration::from_millis(500))?;
            Ok((q, info))
        })();
        if let Ok(mut s) = slot.lock() {
            *s = Some(r);
        }
    });
    l.host_wait = Some(answer);
}

/// Shows the CHECK HOST answer when it arrives.
fn poll_host_check(mut launcher: ResMut<Launcher>) {
    let l = &mut *launcher;
    let addr = l.choices.address.trim().to_string();
    if l.host_text.as_ref().is_some_and(|(a, _)| *a != addr) {
        l.host_text = None;
        l.host_wait = None;
    }
    let Some(r) = l.host_wait.as_ref().and_then(|w| w.lock().ok().and_then(|mut s| s.take())) else { return };
    l.host_wait = None;
    let text = match r {
        Ok((q, info)) => {
            runlog::kv(
                "launcher_host_check",
                &format!("address={addr} state=answer query_port={q} map={} mode={} length={} players={} max_players={} match_started={} protocol={:#x} same_version={}", info.map, info.mode, info.length, info.players, info.max_players, info.match_started, info.protocol, info.protocol == crate::net::PROTOCOL_ID),
            );
            if info.protocol != crate::net::PROTOCOL_ID {
                "The host runs a different version of Open KF.".to_string()
            } else {
                format!("{} - {} {} - {}/{} players{}", info.map, info.mode, info.length, info.players, info.max_players, if info.match_started { " - started" } else { "" })
            }
        }
        Err(e) => {
            runlog::kv("launcher_host_check", &format!("address={addr} state=failed reason=\"{e}\""));
            format!("No answer ({e}).")
        }
    };
    l.host_text = Some((addr, text));
}

/// The boxes that took clicks last frame (id, physical pixels).
#[derive(Resource, Default)]
struct Hits(Vec<(String, Rect)>);

/// The game started by PLAY, handed to `run` after the window closes.
#[derive(Resource, Clone, Default)]
struct Started(Arc<Mutex<Option<std::process::Child>>>);

/// Opens the launcher. `args`: the command line without the program name.
pub fn run(args: &[String]) -> AppExit {
    let opts = match parse_options(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: {e}");
            return AppExit::error();
        }
    };
    let log_path = opts.log.clone().unwrap_or_else(|| LOG_PATH.to_string());
    if let Err(e) = runlog::init(&log_path) {
        eprintln!("warning: could not create {log_path}: {e}");
    }
    let install = match Install::discover() {
        Ok(i) => i,
        Err(e) => {
            eprintln!("error: {e}");
            runlog::kv("error", "reason=install_not_found");
            return AppExit::error();
        }
    };
    let launcher = Launcher {
        choices: load_choices(&opts.settings_path()),
        maps: list_maps(&install.root),
        characters: crate::player::character::model_select_records(&install.root).into_iter().map(|r| (r.name, r.portrait)).collect(),
        reveal_map: true,
        ini_volumes: crate::audio::mixer::ini_volumes(&install.root),
        resolutions: choices::WINDOW_SIZES.iter().flatten().copied().collect(),
        ..default()
    };
    runlog::kv(
        "launcher_open",
        &format!("install=\"{}\" maps={} characters={} dry_run={} mute={} options=\"{}\"", install.root.display(), launcher.maps.len(), launcher.characters.len(), opts.dry_run, opts.mute, command_line(args)),
    );
    let started = Started::default();
    let window = match opts.window {
        Some((w, h)) => Window {
            title: "Open KF".into(),
            resolution: bevy::window::WindowResolution::new(w, h).with_scale_factor_override(1.0),
            ..default()
        },
        None => Window { title: "Open KF".into(), resolution: bevy::window::WindowResolution::new(1280, 800), ..default() },
    };
    let exit = App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin { primary_window: Some(window), ..default() }).disable::<bevy::audio::AudioPlugin>())
        .insert_resource(ClearColor(Color::srgb(0.05, 0.04, 0.04)))
        .insert_resource(InstallRoot(install.root.clone()))
        .insert_resource(launcher)
        .insert_resource(opts)
        .insert_resource(started.clone())
        .init_resource::<Gui>()
        .init_resource::<Hits>()
        .add_systems(Startup, setup)
        .add_systems(Update, (read_monitors, input, poll_host_check, log_changes, test_end).chain())
        .add_systems(PostUpdate, paint)
        .run();
    // The window is closed now (the app is gone); wait for the game.
    let child = started.0.lock().ok().and_then(|mut c| c.take());
    let Some(mut child) = child else {
        runlog::kv("shutdown", &format!("exit={exit:?} launched=false"));
        return exit;
    };
    match child.wait() {
        Ok(status) => {
            runlog::kv("launcher_child_exit", &format!("pid={} code={}", child.id(), status.code().map_or("none".into(), |c| c.to_string())));
            match status.code() {
                Some(0) => AppExit::Success,
                Some(c) => AppExit::from_code(c.clamp(1, 255) as u8),
                None => AppExit::error(),
            }
        }
        Err(e) => {
            runlog::kv("launcher_child_exit", &format!("pid={} error=\"{e}\"", child.id()));
            AppExit::error()
        }
    }
}

#[derive(Resource)]
struct InstallRoot(std::path::PathBuf);

/// The playable maps in the install's `Maps` folder: every `.rom` but
/// KF's start-up, intro and main-menu maps.
fn list_maps(root: &std::path::Path) -> Vec<String> {
    let mut maps: Vec<String> = std::fs::read_dir(root.join("Maps"))
        .map(|d| {
            d.flatten()
                .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".rom").or_else(|| n.strip_suffix(".ROM"))).map(str::to_string))
                .filter(|n| !["entry", "kfintro", "kf-menu"].contains(&n.to_ascii_lowercase().as_str()))
                .collect()
        })
        .unwrap_or_default();
    maps.sort_by_key(|m| m.to_ascii_lowercase());
    maps
}

fn setup(mut commands: Commands, mut gui: ResMut<Gui>, root: Res<InstallRoot>, launcher: Res<Launcher>, mut images: ResMut<Assets<Image>>) {
    let started = std::time::Instant::now();
    commands.spawn(Camera2d);
    let mut extra: Vec<String> = crate::game::perks::Perk::ALL.iter().map(|p| p.icons().0.to_string()).collect();
    extra.extend(launcher.characters.iter().map(|(_, p)| p.clone()).filter(|p| !p.is_empty()));
    gui::load(&mut gui, &root.0, &extra, &mut images);
    for i in 0..POOL {
        commands.spawn((
            Node { position_type: PositionType::Absolute, ..default() },
            ImageNode { image_mode: bevy::ui::widget::NodeImageMode::Stretch, ..default() },
            Visibility::Hidden,
            GlobalZIndex(i as i32),
            MenuSlot(i),
        ));
    }
    runlog::kv("launcher_ready", &format!("fonts={} textures={} seconds={:.2}", gui.fonts.len(), gui.textures.len(), started.elapsed().as_secs_f64()));
}

/// The Ctrl and Shift keys held (left and right), from the key events.
#[derive(Default)]
struct Modifiers([bool; 4]);

impl Modifiers {
    const KEYS: [KeyCode; 4] = [KeyCode::ControlLeft, KeyCode::ControlRight, KeyCode::ShiftLeft, KeyCode::ShiftRight];

    fn update(&mut self, k: &KeyboardInput) {
        if let Some(i) = Self::KEYS.iter().position(|c| *c == k.key_code) {
            self.0[i] = k.state == ButtonState::Pressed;
        }
    }

    fn ctrl(&self) -> bool {
        self.0[0] || self.0[1]
    }

    fn shift(&self) -> bool {
        self.0[2] || self.0[3]
    }
}

/// Keyboard, mouse and test actions.
#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn input(
    mut launcher: ResMut<Launcher>,
    opts: Res<Options>,
    frames: Res<FrameCount>,
    mut keys: MessageReader<KeyboardInput>,
    mouse: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    window: Query<&Window, With<PrimaryWindow>>,
    hits: Res<Hits>,
    started: Res<Started>,
    mut exit: MessageWriter<AppExit>,
    mut focus_lost: MessageReader<KeyboardFocusLost>,
    mut mods: Local<Modifiers>,
    mut clipboard: ResMut<Clipboard>,
) {
    let mut ids: Vec<String> = Vec::new();
    // (field, how), done after the clicks.
    let mut pastes: Vec<(Field, &str)> = Vec::new();
    let l = &mut *launcher;
    // Keys held elsewhere are not ours (Bevy releases them too).
    if focus_lost.read().count() > 0 {
        *mods = Modifiers::default();
    }
    // Typing into the focused field.
    for k in keys.read() {
        // Ctrl and Shift followed in event order: a quick Ctrl+V can press
        // and release both within one frame (seen with xdotool), which
        // ButtonInput<KeyCode> (the state at the frame's end) misses.
        mods.update(k);
        if k.state != ButtonState::Pressed {
            continue;
        }
        let (ctrl, shift) = (mods.ctrl(), mods.shift());
        // Paste: Ctrl+V, Shift+Insert, a keyboard's Paste key.
        let via = match &k.logical_key {
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("v") => Some("ctrl+v"),
            _ if ctrl && k.key_code == KeyCode::KeyV => Some("ctrl+v"),
            Key::Insert if shift => Some("shift+insert"),
            Key::Paste => Some("paste_key"),
            _ => None,
        };
        if let Some(via) = via {
            match l.focus {
                Some(f) => pastes.push((f, via)),
                None => runlog::kv("launcher_paste_failed", &format!("via={via} reason=no_focused_field")),
            }
            continue;
        }
        match (&k.logical_key, l.focus) {
            (Key::Escape, Some(_)) | (Key::Enter, Some(_)) => ids.push("unfocus".into()),
            (Key::Escape, None) => ids.push("quit".into()),
            (Key::Enter, None) => ids.push("launch".into()),
            (Key::Tab, _) => ids.push("tab".into()),
            (Key::Backspace, Some(f)) => {
                f.text(&mut l.choices).pop();
            }
            (_, Some(f)) => {
                if let Some(t) = &k.text {
                    f.type_text(f.text(&mut l.choices), t);
                }
            }
            _ => {}
        }
    }
    for (_, a) in opts.input.iter().filter(|(f, _)| *f == frames.0) {
        runlog::kv("launcher_action", &format!("frame={} action=\"{a}\"", frames.0));
        if let Some(f) = a.strip_prefix("click:paste:").and_then(Field::parse) {
            pastes.push((f, "button"));
        } else if let Some(id) = a.strip_prefix("click:") {
            ids.push(id.to_string());
        } else if let Some(t) = a.strip_prefix("clipboard:") {
            let r = clipboard.set_text(t.to_string());
            runlog::kv("launcher_clipboard_set", &format!("chars={} ok={}", t.chars().count(), r.is_ok()));
        } else if a == "paste" {
            match l.focus {
                Some(f) => pastes.push((f, "test")),
                None => runlog::kv("launcher_paste_failed", "via=test reason=no_focused_field"),
            }
        } else if let Some((field, v)) = a.strip_prefix("set:").and_then(|s| s.split_once('=')) {
            match l.choices.set(field, v) {
                Ok(()) => l.reveal_map |= field == "map",
                Err(e) => runlog::kv("launcher_action_refused", &format!("action=\"{a}\" reason=\"{e}\"")),
            }
        } else if let Some(t) = a.strip_prefix("type:") {
            match l.focus {
                Some(f) => f.type_text(f.text(&mut l.choices), t),
                None => runlog::kv("launcher_action_refused", &format!("action=\"{a}\" reason=no_focused_field")),
            }
        } else if let Some(k) = a.strip_prefix("key:") {
            match k {
                "tab" => ids.push("tab".into()),
                "enter" => ids.push(if l.focus.is_some() { "unfocus" } else { "launch" }.into()),
                "escape" => ids.push(if l.focus.is_some() { "unfocus" } else { "quit" }.into()),
                "backspace" => {
                    if let Some(f) = l.focus {
                        f.text(&mut l.choices).pop();
                    }
                }
                _ => runlog::kv("launcher_action_refused", &format!("action=\"{a}\" reason=unknown_key")),
            }
        } else if let Some((name, f)) = a.strip_prefix("volume_click:").and_then(|s| s.split_once('@')) {
            // A click at that fraction of a volume slider's box (as drawn
            // last frame), through the mouse's code.
            let i = choices::SLIDERS.iter().position(|(n, _)| *n == name);
            let r = hits.0.iter().find(|(id, _)| *id == volume_slider_id(name)).map(|(_, r)| *r);
            match (i, r, f.trim().parse::<f32>()) {
                (Some(i), Some(r), Ok(f)) => slide_to(l, &hits.0, i, r.min.x + f * r.width()),
                _ => runlog::kv("launcher_action_refused", &format!("action=\"{a}\" reason=no_such_slider")),
            }
        } else if a == "dump" {
            let list: Vec<String> = hits.0.iter().map(|(i, r)| format!("{i}:({:.0},{:.0})-({:.0},{:.0})", r.min.x, r.min.y, r.max.x, r.max.y)).collect();
            runlog::kv("launcher_dump", &format!("hits={} [{}]", list.len(), list.join(" ")));
        } else {
            runlog::kv("launcher_action_refused", &format!("action=\"{a}\" reason=unknown_action"));
        }
    }
    let Ok(win) = window.single() else { return };
    if mouse.just_pressed(MouseButton::Left)
        && let Some(pos) = win.physical_cursor_position()
    {
        // A click outside every box ends typing.
        let id = hits.0.iter().rev().find(|(_, r)| r.contains(pos)).map_or("unfocus".into(), |(id, _)| id.clone());
        // A press on a volume slider grabs it (GUISlider, as in the game's
        // Audio window): the value follows the mouse until let go.
        match choices::SLIDERS.iter().position(|(n, _)| volume_slider_id(n) == id) {
            Some(i) => {
                l.focus = None;
                l.volume_drag = Some(i);
            }
            None => match id.strip_prefix("paste:").and_then(Field::parse) {
                Some(f) => pastes.push((f, "button")),
                None => ids.push(id),
            },
        }
    }
    // A right-click on a text field selects it and pastes.
    if mouse.just_pressed(MouseButton::Right)
        && let Some(pos) = win.physical_cursor_position()
        && let Some(f) = hits.0.iter().rev().find(|(_, r)| r.contains(pos)).and_then(|(id, _)| id.strip_prefix("focus:")).and_then(Field::parse)
    {
        pastes.push((f, "right_click"));
    }
    if let Some(i) = l.volume_drag {
        if !mouse.pressed(MouseButton::Left) {
            l.volume_drag = None;
        } else if let Some(pos) = win.physical_cursor_position() {
            slide_to(l, &hits.0, i, pos.x);
        }
    }
    // The mouse wheel over the map list scrolls it.
    if scroll.delta.y != 0.0
        && let Some(pos) = win.physical_cursor_position()
        && hits.0.iter().any(|(id, r)| id.starts_with("map:") && r.contains(pos))
    {
        ids.push(format!("maps.scroll:{}", if scroll.delta.y > 0.0 { -3 } else { 3 }));
    }
    for id in ids {
        apply(&id, l, &opts, &started, &mut exit);
    }
    for (f, via) in pastes {
        paste(l, &mut clipboard, f, via);
    }
}

/// The click id of a volume slider (as in the game's Audio window).
pub fn volume_slider_id(name: &str) -> String {
    format!("volume.slider:{name}")
}

/// A volume slider follows the mouse's x over its box as drawn last frame.
fn slide_to(l: &mut Launcher, hits: &[(String, Rect)], i: usize, x: f32) {
    let Some((name, _)) = choices::SLIDERS.get(i) else { return };
    let Some((_, r)) = hits.iter().find(|(id, _)| *id == volume_slider_id(name)) else { return };
    let (_, max) = l.choices.volumes.slider(name, l.ini_volumes).unwrap_or((0.0, 1.0));
    l.choices.volumes.set_slider(name, gui::slider_fraction(*r, x) * max);
}

fn apply(id: &str, l: &mut Launcher, opts: &Options, started: &Started, exit: &mut MessageWriter<AppExit>) {
    let characters: Vec<String> = l.characters.iter().map(|(n, _)| n.clone()).collect();
    // Clicking a button ends typing in a field.
    if !id.starts_with("focus:") && id != "tab" {
        l.focus = None;
    }
    if let Some(p) = id.strip_prefix("play:") {
        if let Err(e) = l.choices.set("play", p) {
            runlog::kv("launcher_action_refused", &format!("id={id} reason=\"{e}\""));
        }
    } else if let Some(f) = id.strip_prefix("focus:").and_then(Field::parse) {
        l.focus = Some(f);
    } else if let Some(d) = id.strip_prefix("spin:window:") {
        let sizes = l.resolutions.clone();
        l.choices.step_resolution(d.parse().unwrap_or(1), &sizes);
    } else if let Some((field, d)) = id.strip_prefix("spin:").and_then(|s| s.rsplit_once(':')) {
        l.choices.step(field, d.parse().unwrap_or(1), &characters);
    } else if let Some(i) = id.strip_prefix("map:").and_then(|n| n.parse::<usize>().ok()) {
        if let Some(m) = l.maps.get(i) {
            l.choices.map = m.clone();
        }
    } else if let Some(n) = id.strip_prefix("maps.scroll:").and_then(|n| n.parse::<i64>().ok()) {
        let max = l.maps.len().saturating_sub(l.map_rows.max(1)) as i64;
        l.map_top = (l.map_top as i64 + n).clamp(0, max) as usize;
    } else {
        match id {
            "unfocus" => {}
            "tab" => {
                let shown: Vec<Field> = Field::ALL.into_iter().filter(|f| f.shown(l.choices.play)).collect();
                let next = match l.focus.and_then(|f| shown.iter().position(|&s| s == f)) {
                    Some(i) => shown[(i + 1) % shown.len()],
                    None => shown[0],
                };
                l.focus = Some(next);
            }
            "launch" => launch(l, opts, started, exit),
            "check_host" => start_host_check(l),
            "quit" => {
                runlog::kv("launcher_quit", "");
                exit.write(AppExit::Success);
            }
            _ => runlog::kv("launcher_action_ignored", &format!("id={id}")),
        }
    }
    runlog::kv("launcher_click", &format!("id={id} focus={:?}", l.focus));
}

/// PLAY: build the arguments, let the game's own parser check them, then
/// start the game (or only log it with `--dry-run`).
fn launch(l: &mut Launcher, opts: &Options, started: &Started, exit: &mut MessageWriter<AppExit>) {
    let args = match l.choices.to_args(opts.mute) {
        // The game reads its volumes from the same file.
        Ok(mut a) => {
            if let Some(s) = &opts.settings {
                a.extend(["--settings".to_string(), s.clone()]);
            }
            a
        }
        Err(e) => {
            runlog::kv("launcher_refused", &format!("reason=\"{e}\""));
            l.status = e;
            return;
        }
    };
    let line = command_line(&args);
    if let Err(e) = crate::parse_args(args.clone()) {
        runlog::kv("launcher_refused", &format!("reason=\"{e}\" args=\"{line}\""));
        l.status = format!("The game would refuse these options: {e}");
        return;
    }
    runlog::kv("launcher_launch", &format!("args=\"{line}\" dry_run={}", opts.dry_run));
    save_choices(&opts.settings_path(), &l.choices);
    if opts.dry_run {
        exit.write(AppExit::Success);
        return;
    }
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            l.status = format!("Cannot find the game's program file: {e}");
            runlog::kv("launcher_start_failed", &format!("reason=\"{e}\""));
            return;
        }
    };
    match std::process::Command::new(&exe).args(&args).spawn() {
        Ok(child) => {
            runlog::kv("launcher_child_started", &format!("pid={} exe=\"{}\"", child.id(), exe.display()));
            if let Ok(mut s) = started.0.lock() {
                *s = Some(child);
            }
            exit.write(AppExit::Success);
        }
        Err(e) => {
            l.status = format!("Cannot start the game: {e}");
            runlog::kv("launcher_start_failed", &format!("exe=\"{}\" reason=\"{e}\"", exe.display()));
        }
    }
}

/// The Resolution list: the primary monitor's video mode sizes (Bevy's
/// `Monitor` entities, from winit: the same on Linux and Windows), largest
/// first, plus its current size. Read once, when the monitors are known;
/// after 60 frames without one the fixed list stays.
fn read_monitors(mut launcher: ResMut<Launcher>, monitors: Query<(&bevy::window::Monitor, Has<bevy::window::PrimaryMonitor>)>, mut frames: Local<u32>, mut done: Local<bool>) {
    if *done {
        return;
    }
    *frames += 1;
    let primary = monitors.iter().find(|(_, p)| *p).or_else(|| monitors.iter().next());
    let Some((m, is_primary)) = primary else {
        if *frames > 60 {
            *done = true;
            runlog::kv("launcher_display_modes", &format!("monitor=none sizes={} source=fixed_list", launcher.resolutions.len()));
        }
        return;
    };
    *done = true;
    let desktop = (m.physical_width, m.physical_height);
    let sizes = monitor_sizes(m.video_modes.iter().map(|v| (v.physical_size.x, v.physical_size.y)), desktop);
    let list: Vec<String> = sizes.iter().map(|(w, h)| format!("{w}x{h}")).collect();
    runlog::kv(
        "launcher_display_modes",
        &format!("monitor=\"{}\" primary={is_primary} desktop={}x{} refresh_hz={:.2} video_modes={} sizes={} source=monitor [{}]", m.name.as_deref().unwrap_or("?"), desktop.0, desktop.1, m.refresh_rate_millihertz.unwrap_or(0) as f32 / 1000.0, m.video_modes.len(), sizes.len(), list.join(" ")),
    );
    launcher.desktop = Some(desktop);
    launcher.resolutions = sizes;
}

/// The distinct sizes, largest first (by pixels, then width), with the
/// desktop's own size always in.
fn monitor_sizes(modes: impl Iterator<Item = (u32, u32)>, desktop: (u32, u32)) -> Vec<(u32, u32)> {
    let mut sizes: Vec<(u32, u32)> = modes.chain(std::iter::once(desktop)).filter(|(w, h)| *w > 0 && *h > 0).collect();
    sizes.sort_by_key(|&(w, h)| std::cmp::Reverse((w as u64 * h as u64, w)));
    sizes.dedup();
    sizes
}

/// Logs every choice that changed this frame.
fn log_changes(mut launcher: ResMut<Launcher>) {
    if launcher.logged.as_ref() == Some(&launcher.choices) {
        return;
    }
    let before = launcher.logged.take();
    for f in choices::FIELDS {
        let v = launcher.choices.get(f);
        if before.as_ref().is_none_or(|b| b.get(f) != v) {
            runlog::kv(if before.is_none() { "launcher_choice" } else { "launcher_change" }, &format!("field={f} value=\"{v}\""));
        }
    }
    // A new choice clears an old message.
    if before.is_some() {
        launcher.status.clear();
    }
    launcher.logged = Some(launcher.choices.clone());
}

/// `--screenshot` and `--frames` (test runs end by themselves).
fn test_end(mut commands: Commands, opts: Res<Options>, frames: Res<FrameCount>, mut exit: MessageWriter<AppExit>, mut done: Local<Option<u32>>, last_seen: Local<Arc<Mutex<bool>>>) {
    if opts.screenshot.contains(&frames.0) {
        let dir = std::path::PathBuf::from("work/screenshots");
        let _ = std::fs::create_dir_all(&dir);
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let png = dir.join(format!("launcher-{stamp}-{}.png", frames.0));
        runlog::kv("screenshot", &format!("file={} frame={}", png.display(), frames.0));
        let mut shot = commands.spawn(Screenshot::primary_window());
        shot.observe(save_to_disk(png));
        if opts.screenshot.iter().max() == Some(&frames.0) {
            let flag = last_seen.clone();
            shot.observe(move |_: On<ScreenshotCaptured>| {
                if let Ok(mut f) = flag.lock() {
                    *f = true;
                }
            });
        }
    }
    if done.is_none() && last_seen.lock().is_ok_and(|f| *f) {
        *done = Some(frames.0);
    }
    // A few frames for the file to be written.
    if done.is_some_and(|d| frames.0 > d + 5) {
        runlog::kv("screenshot_saved_quitting", "");
        exit.write(AppExit::Success);
    }
    if opts.frames.is_some_and(|n| frames.0 >= n) {
        runlog::kv("frame_limit_reached", &format!("frame={}", frames.0));
        exit.write(AppExit::Success);
    }
}

fn paint(
    gui: Res<Gui>,
    mut launcher: ResMut<Launcher>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut slots: Query<(&MenuSlot, &mut Node, &mut ImageNode, &mut Visibility)>,
    mut hits: ResMut<Hits>,
    mut shown: Local<usize>,
) {
    if !gui.loaded {
        return;
    }
    let Ok(win) = window.single() else { return };
    let mut p = gui::Painter::new(&gui, Vec2::new(win.width(), win.height()), win.scale_factor(), win.physical_cursor_position());
    draw::draw(&mut p, &mut launcher);
    hits.0 = std::mem::take(&mut p.hits);
    gui::flush(&p.canvas.quads, &gui, &mut slots, &mut shown);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_opens_only_without_game_arguments() {
        assert!(wanted(&[]));
        assert!(wanted(&["--launcher".to_string(), "--dry-run".to_string()]));
        assert!(!wanted(&["--mode".to_string(), "debug".to_string()]));
        assert!(!wanted(&["--map".to_string(), "KF-Farm".to_string(), "--launcher".to_string()]));
    }

    #[test]
    fn built_arguments_pass_the_games_parser() {
        let mut c = Choices::default();
        for play in [PlayType::Solo, PlayType::Host, PlayType::Join] {
            c.play = play;
            c.address = "127.0.0.1:7800".into();
            c.perk = Some(crate::game::perks::Perk::Firebug);
            c.perk_level = 6;
            c.name = "Big Al".into();
            c.window = Some((1280, 720));
            c.fps = Some(60);
            c.vsync = false;
            c.trader = crate::game::buy_menu::MenuKind::Nu;
            c.display = crate::engine::graphics::DisplayMode::Borderless;
            c.fov = 110;
            c.brightness = 130;
            c.msaa = 1;
            c.anisotropy = 16;
            let args = c.to_args(true).unwrap();
            let parsed = crate::parse_args(args.clone()).unwrap_or_else(|e| panic!("{play:?}: {e} ({args:?})"));
            assert_eq!(parsed.display, crate::engine::graphics::DisplayMode::Borderless);
            assert_eq!((parsed.fov, parsed.brightness, parsed.msaa, parsed.anisotropy), (Some(110), Some(130), Some(1), Some(16)));
            assert_eq!(parsed.trader_menu, crate::game::buy_menu::MenuKind::Nu);
            assert_eq!(parsed.name.as_deref(), Some("Big Al"));
            assert!(parsed.mute && parsed.no_vsync);
            assert_eq!(parsed.net.active(), play != PlayType::Solo);
        }
        c.extra = "--no-such-option".into();
        assert!(crate::parse_args(c.to_args(false).unwrap()).is_err());
    }

    #[test]
    fn typed_options_override_saved_graphics() {
        // The extra arguments come last; the game keeps the last value.
        let c = Choices { fov: 100, extra: "--fov 85 --display fullscreen".into(), ..Default::default() };
        let parsed = crate::parse_args(c.to_args(false).unwrap()).unwrap();
        assert_eq!(parsed.fov, Some(85));
        assert_eq!(parsed.display, crate::engine::graphics::DisplayMode::Fullscreen);
    }

    #[test]
    fn monitor_sizes_sorted_and_unique() {
        let modes = [(1280, 720), (1920, 1080), (1920, 1080), (800, 600), (1280, 1024)];
        assert_eq!(monitor_sizes(modes.into_iter(), (2560, 1440)), vec![(2560, 1440), (1920, 1080), (1280, 1024), (1280, 720), (800, 600)]);
        // Only the desktop (e.g. a virtual display that lists no modes).
        assert_eq!(monitor_sizes(std::iter::empty(), (1920, 1080)), vec![(1920, 1080)]);
    }

    #[test]
    fn pasted_text_is_cleaned_per_field() {
        // An address with spaces and a newline replaces the old one.
        let p = paste_text(Field::Address, "10.0.0.1", "  1.2.3.4:7707\r\n");
        assert_eq!(p, Pasted { text: "1.2.3.4:7707".into(), replaced: true, kept: 12, dropped: 0, cut: 0 });
        assert!(crate::net::NetMode::join(&p.text).is_ok());
        // The first non-empty line only; other characters dropped.
        let p = paste_text(Field::Address, "", "\n\nhost name.example!\nsecond");
        assert_eq!((p.text.as_str(), p.dropped), ("hostname.example", 2));
        // host:port into the port field: the port.
        assert_eq!(paste_text(Field::Port, "7707", "1.2.3.4:7800").text, "7800");
        assert_eq!(paste_text(Field::Port, "7707", " 9000x ").text, "9000");
        let p = paste_text(Field::Port, "", "1234567");
        assert_eq!((p.text.as_str(), p.cut), ("12345", 2));
        // Names: added at the end, no quotes, 16 characters.
        let p = paste_text(Field::Name, "Big ", "\"Al\" the Great Destroyer");
        assert_eq!(p, Pasted { text: "Big Al the Great".into(), replaced: false, kept: 12, dropped: 2, cut: 10 });
        // Extra: the lines joined.
        assert_eq!(paste_text(Field::Extra, "--god", " --give\nall\t").text, "--god --give all");
        // Nothing usable.
        assert_eq!(paste_text(Field::Address, "a", " \n ").kept, 0);
    }

    #[test]
    fn typing_follows_the_same_rules() {
        let mut s = String::new();
        Field::Port.type_text(&mut s, "77a07999");
        assert_eq!(s, "77079");
        let mut s = "x".repeat(15);
        Field::Name.type_text(&mut s, "\"yz");
        assert_eq!(s, format!("{}y", "x".repeat(15)));
    }

    #[test]
    fn launcher_options_parse() {
        let a: Vec<String> = ["--launcher", "--dry-run", "--mute", "--input", "10:set:name=Big Al,20:click:launch", "--screenshot", "5,9"].iter().map(|s| s.to_string()).collect();
        let o = parse_options(&a).unwrap();
        assert!(o.dry_run && o.mute);
        assert_eq!(o.input, vec![(10, "set:name=Big Al".to_string()), (20, "click:launch".to_string())]);
        assert_eq!(o.screenshot, vec![5, 9]);
        assert!(parse_options(&["--launcher".to_string(), "--map".to_string()]).is_err());
    }
}
