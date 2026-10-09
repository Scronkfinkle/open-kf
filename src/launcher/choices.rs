//! What the launcher's player chose, the game arguments that makes, and
//! the `key=value` text the choices are saved as. No Bevy here, so it is
//! all unit-tested. See DESIGN.md, "The launcher".

use crate::engine::graphics::{self, DisplayMode};
use crate::game::buy_menu::MenuKind;
use crate::game::difficulty::Difficulty;
use crate::game::map_rotation::{self, MapRotation, MapVoteConfig};
use crate::game::perks::Perk;
use crate::player::character::DEFAULT_CHARACTER;

/// Solo, host a network game, or join one.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PlayType {
    #[default]
    Solo,
    Host,
    Join,
}

impl PlayType {
    pub fn word(self) -> &'static str {
        match self {
            PlayType::Solo => "solo",
            PlayType::Host => "host",
            PlayType::Join => "join",
        }
    }
}

/// The window sizes offered when the monitor reports none (None: the
/// system's default window). Normally the launcher offers the monitor's
/// own sizes (`step_resolution`).
pub const WINDOW_SIZES: &[Option<(u32, u32)>] = &[None, Some((1280, 720)), Some((1280, 960)), Some((1600, 900)), Some((1920, 1080)), Some((2560, 1440))];
/// The frame limits offered (None: no limit).
pub const FPS_LIMITS: &[Option<u32>] = &[None, Some(30), Some(60), Some(120), Some(144), Some(240)];
/// `--length` values, in the order the button steps through them.
pub const LENGTHS: [&str; 3] = ["short", "normal", "long"];
/// The highest starting wave offered: a long game's 10 waves + the boss.
pub const MAX_START_WAVE: u32 = 11;

/// The `--perk` word of a perk.
pub fn perk_word(p: Perk) -> &'static str {
    match p {
        Perk::Medic => "medic",
        Perk::Support => "support",
        Perk::Sharpshooter => "sharpshooter",
        Perk::Commando => "commando",
        Perk::Berserker => "berserker",
        Perk::Firebug => "firebug",
        Perk::Demolitions => "demolitions",
    }
}

/// Every choice by name, in the order they are saved and logged.
pub const FIELDS: [&str; 33] = [
    "play", "port", "address", "map", "mode", "length", "difficulty", "wave", "name", "perk", "level", "character", "window", "fps", "vsync", "display", "fov", "brightness", "msaa", "anisotropy", "sound", "trader", "extra",
    "volume", "effects_volume", "music_volume", "aim", "mouse_sensitivity", "invert_mouse", "map_list", "map_position", "map_vote", "vote_time_limit",
];

/// The map rotation lines of the saved file (game/map_rotation.rs). The
/// game rewrites only these (after a map change, `save_map_rotation`).
#[allow(dead_code)] // used by save_map_rotation (not wired yet)
pub const ROTATION_FIELDS: [&str; 2] = ["map_list", "map_position"];

/// The aim line of the saved file (the game rewrites only it).
pub const AIM_FIELD: &str = "aim";

/// The mouse lines of the saved file (the game rewrites only these).
pub const MOUSE_FIELDS: [&str; 2] = ["mouse_sensitivity", "invert_mouse"];

/// KF's mouse sensitivity (Engine.PlayerInput MouseSensitivity = 3) and
/// its options box (UT2K4Tab_IForceSettings InputMouseSensitivity:
/// MinValue 0.25, MaxValue 25, Step 0.25). See DESIGN.md, "Mouse
/// sensitivity and invert mouse".
pub const SENSITIVITY_DEFAULT: f32 = 3.0;
pub const SENSITIVITY_MIN: f32 = 0.25;
pub const SENSITIVITY_MAX: f32 = 25.0;
pub const SENSITIVITY_STEP: f32 = 0.25;

/// The volume lines of the saved file (the game rewrites only these).
pub const VOLUME_FIELDS: [&str; 3] = ["volume", "effects_volume", "music_volume"];

/// The top of the master volume slider (ours: KF has none).
pub const MASTER_MAX: f32 = 1.0;
/// The top of KF's Music Volume and Effects Volume sliders
/// (KFAudioSettingsTab.AudioMusicVolume / AudioEffectsVolumeSlider
/// MaxValue 0.5; MinValue 0).
pub const KF_VOLUME_MAX: f32 = 0.5;

/// The slider names in screen order, with their captions (KFGui.int
/// [KFAudioSettingsTab]; "Master Volume" is ours).
pub const SLIDERS: [(&str, &str); 3] = [("master", "Master Volume"), ("effects", "Effects Volume"), ("music", "Music Volume")];

/// The volume settings (DESIGN.md, "Volume control"). Heard volume:
/// sounds = master x effects, music = master x music.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Volumes {
    /// Ours (KF has no master volume): 0 to 1.
    pub master: f32,
    /// KF's SoundVolume, 0 to 0.5; None: the install's KillingFloor.ini.
    pub effects: Option<f32>,
    /// KF's MusicVolume, 0 to 0.5; None: the install's KillingFloor.ini.
    pub music: Option<f32>,
}

impl Default for Volumes {
    fn default() -> Self {
        Volumes { master: 1.0, effects: None, music: None }
    }
}

/// A slider's value as kept and saved: inside its range, to 0.001.
pub fn clamp_volume(v: f32, max: f32) -> f32 {
    if v.is_finite() { (v.clamp(0.0, max) * 1000.0).round() / 1000.0 } else { 0.0 }
}

impl Volumes {
    /// (effects, music), the ini's values where not set.
    pub fn resolved(&self, ini: (f32, f32)) -> (f32, f32) {
        (self.effects.unwrap_or(ini.0), self.music.unwrap_or(ini.1))
    }

    /// What the mixer uses: (sounds, music) = master x each.
    pub fn heard(&self, ini: (f32, f32)) -> (f32, f32) {
        let (e, m) = self.resolved(ini);
        (self.master * e, self.master * m)
    }

    /// A slider by name (`master`, `effects`, `music`): its value now and
    /// its top.
    pub fn slider(&self, name: &str, ini: (f32, f32)) -> Option<(f32, f32)> {
        let (e, m) = self.resolved(ini);
        match name {
            "master" => Some((self.master, MASTER_MAX)),
            "effects" => Some((e, KF_VOLUME_MAX)),
            "music" => Some((m, KF_VOLUME_MAX)),
            _ => None,
        }
    }

    /// Sets a slider by name (clamped). False for an unknown name.
    pub fn set_slider(&mut self, name: &str, v: f32) -> bool {
        match name {
            "master" => self.master = clamp_volume(v, MASTER_MAX),
            "effects" => self.effects = Some(clamp_volume(v, KF_VOLUME_MAX)),
            "music" => self.music = Some(clamp_volume(v, KF_VOLUME_MAX)),
            _ => return false,
        }
        true
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Choices {
    pub play: PlayType,
    /// Host: the game port (text as typed).
    pub port: String,
    /// Join: `ADDR` or `ADDR:PORT` (text as typed).
    pub address: String,
    pub map: String,
    /// Waves (true) or debug mode.
    pub waves: bool,
    /// Index into `LENGTHS`.
    pub length: usize,
    /// `--difficulty` (default Normal, KF's).
    pub difficulty: Difficulty,
    /// None: from the first wave.
    pub start_wave: Option<u32>,
    /// Empty: the game's default (KF's defuser.ini name).
    pub name: String,
    pub perk: Option<Perk>,
    pub perk_level: u8,
    pub character: String,
    /// The resolution (`--window`): the window size, or the video mode in
    /// fullscreen. None: the default window / the desktop's mode.
    pub window: Option<(u32, u32)>,
    pub fps: Option<u32>,
    pub vsync: bool,
    /// Graphics (engine/graphics.rs): `--display`, `--fov`, `--brightness`
    /// (percent), `--msaa` (samples, 1 = off), `--anisotropy`.
    pub display: DisplayMode,
    pub fov: u32,
    pub brightness: u32,
    pub msaa: u32,
    pub anisotropy: u16,
    pub sound: bool,
    /// The trader's menu (`--trader-menu`): KF's (default) or NuMenu.
    pub trader: MenuKind,
    /// More options typed by hand (e.g. `--god`).
    pub extra: String,
    /// The volumes (the game's pause menu changes them too).
    pub volumes: Volumes,
    /// Aim down sights while the button is held (else a press toggles,
    /// KF's default). The game's pause menu changes it too.
    pub aim_hold: bool,
    /// KF's MouseSensitivity (0.25 to 25; 3 is KF's default). The game's
    /// pause menu changes it too.
    pub mouse_sensitivity: f32,
    /// KF's bInvertMouse: moving the mouse forward looks down.
    pub invert_mouse: bool,
    /// The map list and its position (`map_list=`, `map_position=`); Solo
    /// and Host only (a joiner gets the host's maps).
    pub rotation: MapRotation,
    /// Map voting on/off and its time (`map_vote=`, `vote_time_limit=`).
    pub vote: MapVoteConfig,
}

impl Default for Choices {
    fn default() -> Self {
        Choices {
            play: PlayType::Solo,
            port: crate::net::DEFAULT_PORT.to_string(),
            address: String::new(),
            map: crate::DEFAULT_MAP.to_string(),
            waves: true,
            length: 0,
            difficulty: Difficulty::Normal,
            start_wave: None,
            name: String::new(),
            perk: None,
            perk_level: 0,
            character: DEFAULT_CHARACTER.to_string(),
            window: None,
            fps: None,
            vsync: true,
            display: DisplayMode::Windowed,
            fov: graphics::DEFAULT_FOV,
            brightness: graphics::DEFAULT_BRIGHTNESS,
            msaa: graphics::DEFAULT_MSAA,
            anisotropy: graphics::DEFAULT_ANISOTROPY,
            sound: true,
            trader: MenuKind::Kf,
            extra: String::new(),
            volumes: Volumes::default(),
            aim_hold: false,
            mouse_sensitivity: SENSITIVITY_DEFAULT,
            invert_mouse: false,
            rotation: MapRotation::default(),
            vote: MapVoteConfig::default(),
        }
    }
}

/// A KF volume as saved: "default" (the ini's) or the number.
fn kf_volume_text(v: Option<f32>) -> String {
    v.map_or("default".into(), |v| format!("{v:.3}"))
}

fn parse_volume(v: &str) -> Result<f32, String> {
    v.trim().parse::<f32>().ok().filter(|x| x.is_finite()).ok_or(format!("not a volume: {v}"))
}

/// The aim setting as saved: "hold" or "toggle".
pub fn aim_word(hold: bool) -> &'static str {
    if hold { "hold" } else { "toggle" }
}

/// "hold" / "toggle" (the saved file, `aim_mode:` test actions).
pub fn parse_aim(v: &str) -> Result<bool, String> {
    match v.trim().to_ascii_lowercase().as_str() {
        "hold" => Ok(true),
        "toggle" => Ok(false),
        _ => Err(format!("not toggle/hold: {v}")),
    }
}

/// A sensitivity as kept and saved: inside KF's range, to 0.01.
pub fn clamp_sensitivity(v: f32) -> f32 {
    if v.is_finite() { (v.clamp(SENSITIVITY_MIN, SENSITIVITY_MAX) * 100.0).round() / 100.0 } else { SENSITIVITY_DEFAULT }
}

/// A typed sensitivity (the saved file, `--sensitivity`, test actions):
/// a number from 0.25 to 25 (KF's options box), kept to 0.01.
pub fn parse_sensitivity(v: &str) -> Result<f32, String> {
    let x = v.trim().parse::<f32>().ok().filter(|x| x.is_finite()).ok_or(format!("not a number: {v}"))?;
    if !(SENSITIVITY_MIN..=SENSITIVITY_MAX).contains(&x) {
        return Err(format!("mouse sensitivity must be {SENSITIVITY_MIN} to {SENSITIVITY_MAX}: {v}"));
    }
    Ok(clamp_sensitivity(x))
}

/// One step of KF's box (0.25) up or down, stopping at the ends.
pub fn step_sensitivity(v: f32, d: i32) -> f32 {
    clamp_sensitivity(v + d as f32 * SENSITIVITY_STEP)
}

/// Where a sensitivity sits on the pause menu's slider (0 to 1).
pub fn sensitivity_fraction(v: f32) -> f32 {
    ((v - SENSITIVITY_MIN) / (SENSITIVITY_MAX - SENSITIVITY_MIN)).clamp(0.0, 1.0)
}

/// The sensitivity at a fraction of the slider, on KF's 0.25 steps.
pub fn sensitivity_at_fraction(f: f32) -> f32 {
    let v = SENSITIVITY_MIN + f.clamp(0.0, 1.0) * (SENSITIVITY_MAX - SENSITIVITY_MIN);
    clamp_sensitivity((v / SENSITIVITY_STEP).round() * SENSITIVITY_STEP)
}

/// "on" / "off" (the saved file's switches).
pub fn on_off_word(b: bool) -> &'static str {
    on_off(b)
}

/// "on" / "off" (also yes / no, true / false, 1 / 0).
pub fn parse_on_off(v: &str) -> Result<bool, String> {
    parse_bool(v.trim())
}

fn on_off(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

fn parse_bool(v: &str) -> Result<bool, String> {
    match v.to_ascii_lowercase().as_str() {
        "on" | "yes" | "true" | "1" => Ok(true),
        "off" | "no" | "false" | "0" => Ok(false),
        _ => Err(format!("not on/off: {v}")),
    }
}

impl Choices {
    /// A choice as text (what is saved and logged).
    pub fn get(&self, field: &str) -> String {
        match field {
            "play" => self.play.word().into(),
            "port" => self.port.clone(),
            "address" => self.address.clone(),
            "map" => self.map.clone(),
            "mode" => if self.waves { "waves" } else { "debug" }.into(),
            "length" => LENGTHS[self.length.min(2)].into(),
            "difficulty" => self.difficulty.word().into(),
            "wave" => self.start_wave.map_or("start".into(), |w| w.to_string()),
            "name" => self.name.clone(),
            "perk" => self.perk.map_or("none", perk_word).into(),
            "level" => self.perk_level.to_string(),
            "character" => self.character.clone(),
            "window" => self.window.map_or("default".into(), |(w, h)| format!("{w}x{h}")),
            "fps" => self.fps.map_or("none".into(), |f| f.to_string()),
            "vsync" => on_off(self.vsync).into(),
            "display" => self.display.word().into(),
            "fov" => self.fov.to_string(),
            "brightness" => self.brightness.to_string(),
            "msaa" => if self.msaa <= 1 { "off".into() } else { self.msaa.to_string() },
            "anisotropy" => self.anisotropy.to_string(),
            "sound" => on_off(self.sound).into(),
            "trader" => self.trader.word().into(),
            "extra" => self.extra.clone(),
            "volume" => format!("{:.3}", self.volumes.master),
            "effects_volume" => kf_volume_text(self.volumes.effects),
            "music_volume" => kf_volume_text(self.volumes.music),
            "aim" => aim_word(self.aim_hold).into(),
            "mouse_sensitivity" => format!("{:.2}", self.mouse_sensitivity),
            "invert_mouse" => on_off(self.invert_mouse).into(),
            "map_list" => map_rotation::map_list_text(&self.rotation.maps),
            "map_position" => self.rotation.position.to_string(),
            "map_vote" => on_off(self.vote.enabled).into(),
            "vote_time_limit" => self.vote.time_limit.to_string(),
            _ => String::new(),
        }
    }

    /// Sets a choice from text (the saved file, the `set:` test action).
    pub fn set(&mut self, field: &str, v: &str) -> Result<(), String> {
        let num = |v: &str| v.trim().parse::<u32>().map_err(|_| format!("not a number: {v}"));
        match field {
            "play" => {
                self.play = match v.to_ascii_lowercase().as_str() {
                    "solo" => PlayType::Solo,
                    "host" => PlayType::Host,
                    "join" => PlayType::Join,
                    _ => return Err(format!("not solo/host/join: {v}")),
                }
            }
            "port" => self.port = v.trim().into(),
            "address" => self.address = v.trim().into(),
            "map" => self.map = v.trim().trim_end_matches(".rom").into(),
            "mode" => self.waves = crate::game::waves::GameMode::parse(v).ok_or(format!("not waves/debug: {v}"))? == crate::game::waves::GameMode::Waves,
            "length" => self.length = LENGTHS.iter().position(|l| l.eq_ignore_ascii_case(v.trim())).ok_or(format!("not short/normal/long: {v}"))?,
            "difficulty" => self.difficulty = Difficulty::parse(v).ok_or(format!("not beginner/normal/hard/suicidal/hoe: {v}"))?,
            "wave" => {
                self.start_wave = if v.eq_ignore_ascii_case("start") || v.trim().is_empty() {
                    None
                } else {
                    let w = num(v)?;
                    if !(1..=MAX_START_WAVE).contains(&w) {
                        return Err(format!("wave must be 1 to {MAX_START_WAVE}: {v}"));
                    }
                    Some(w)
                }
            }
            "name" => self.name = v.trim().into(),
            "perk" => self.perk = if v.eq_ignore_ascii_case("none") { None } else { Some(Perk::parse(v).ok_or(format!("unknown perk: {v}"))?) },
            "level" => {
                let l = num(v)?;
                if l > 6 {
                    return Err(format!("level must be 0 to 6: {v}"));
                }
                self.perk_level = l as u8;
            }
            "character" => self.character = v.trim().into(),
            "window" => {
                self.window = if v.eq_ignore_ascii_case("default") {
                    None
                } else {
                    let (w, h) = v.split_once('x').ok_or(format!("not WxH: {v}"))?;
                    Some((num(w)?, num(h)?))
                }
            }
            "fps" => self.fps = if v.eq_ignore_ascii_case("none") { None } else { Some(num(v)?.clamp(1, 1000)) },
            "vsync" => self.vsync = parse_bool(v)?,
            "display" => self.display = DisplayMode::parse(v).ok_or(format!("not windowed/borderless/fullscreen: {v}"))?,
            "fov" => self.fov = graphics::parse_fov(v)?,
            "brightness" => self.brightness = graphics::parse_brightness(v)?,
            "msaa" => self.msaa = graphics::parse_msaa(v)?,
            "anisotropy" => self.anisotropy = graphics::parse_anisotropy(v)?,
            "sound" => self.sound = parse_bool(v)?,
            "trader" => self.trader = MenuKind::parse(v).ok_or(format!("not nu/kf: {v}"))?,
            "extra" => self.extra = v.trim().into(),
            "volume" => self.volumes.master = clamp_volume(parse_volume(v)?, MASTER_MAX),
            "effects_volume" | "music_volume" => {
                let x = if v.trim().eq_ignore_ascii_case("default") { None } else { Some(clamp_volume(parse_volume(v)?, KF_VOLUME_MAX)) };
                if field == "effects_volume" {
                    self.volumes.effects = x;
                } else {
                    self.volumes.music = x;
                }
            }
            "aim" => self.aim_hold = parse_aim(v)?,
            "mouse_sensitivity" => self.mouse_sensitivity = parse_sensitivity(v)?,
            "invert_mouse" => self.invert_mouse = parse_on_off(v)?,
            // A position past the list's end goes back to 0 once the whole
            // file is read (`from_text`), whatever the line order.
            "map_list" => self.rotation.maps = map_rotation::parse_map_list(v),
            "map_position" => self.rotation.position = num(v)? as usize,
            "map_vote" => self.vote.enabled = parse_on_off(v)?,
            "vote_time_limit" => self.vote.time_limit = map_rotation::parse_vote_time(v)?,
            _ => return Err(format!("unknown choice: {field}")),
        }
        Ok(())
    }

    /// The `<` / `>` buttons: one step through a list of values, wrapping.
    /// `characters`: the character names in list order.
    pub fn step(&mut self, field: &str, d: i32, characters: &[String]) {
        fn cycle(i: usize, n: usize, d: i32) -> usize {
            (i as i64 + d as i64).rem_euclid(n as i64) as usize
        }
        match field {
            "mode" => self.waves = !self.waves,
            "length" => self.length = cycle(self.length, LENGTHS.len(), d),
            "difficulty" => {
                let i = Difficulty::ALL.iter().position(|x| *x == self.difficulty).unwrap_or(1);
                self.difficulty = Difficulty::ALL[cycle(i, Difficulty::ALL.len(), d)];
            }
            "wave" => {
                // 0 = from the start.
                let i = self.start_wave.unwrap_or(0) as usize;
                let n = cycle(i, MAX_START_WAVE as usize + 1, d);
                self.start_wave = (n > 0).then_some(n as u32);
            }
            "perk" => {
                // 0 = none, then the seven perks.
                let i = self.perk.map_or(0, |p| p.index() + 1);
                let n = cycle(i, Perk::ALL.len() + 1, d);
                self.perk = (n > 0).then(|| Perk::ALL[n - 1]);
            }
            "level" => self.perk_level = cycle(self.perk_level as usize, 7, d) as u8,
            "character" if !characters.is_empty() => {
                let i = characters.iter().position(|c| c.eq_ignore_ascii_case(&self.character)).unwrap_or(0);
                self.character = characters[cycle(i, characters.len(), d)].clone();
            }
            "window" => {
                let i = WINDOW_SIZES.iter().position(|w| *w == self.window).unwrap_or(0);
                self.window = WINDOW_SIZES[cycle(i, WINDOW_SIZES.len(), d)];
            }
            "fps" => {
                let i = FPS_LIMITS.iter().position(|f| *f == self.fps).unwrap_or(0);
                self.fps = FPS_LIMITS[cycle(i, FPS_LIMITS.len(), d)];
            }
            "vsync" => self.vsync = !self.vsync,
            "display" => {
                let i = DisplayMode::ALL.iter().position(|m| *m == self.display).unwrap_or(0);
                self.display = DisplayMode::ALL[cycle(i, DisplayMode::ALL.len(), d)];
            }
            // Numbers stop at their ends instead of wrapping.
            "fov" => self.fov = (self.fov as i64 + d as i64 * graphics::FOV_STEP as i64).clamp(graphics::FOV_MIN as i64, graphics::FOV_MAX as i64) as u32,
            "brightness" => self.brightness = (self.brightness as i64 + d as i64 * graphics::BRIGHTNESS_STEP as i64).clamp(graphics::BRIGHTNESS_MIN as i64, graphics::BRIGHTNESS_MAX as i64) as u32,
            "msaa" => {
                let i = graphics::MSAA_SAMPLES.iter().position(|m| *m == self.msaa).unwrap_or(0);
                self.msaa = graphics::MSAA_SAMPLES[cycle(i, graphics::MSAA_SAMPLES.len(), d)];
            }
            "anisotropy" => {
                let i = graphics::ANISOTROPY_LEVELS.iter().position(|a| *a == self.anisotropy).unwrap_or(0);
                self.anisotropy = graphics::ANISOTROPY_LEVELS[cycle(i, graphics::ANISOTROPY_LEVELS.len(), d)];
            }
            "sound" => self.sound = !self.sound,
            "trader" => self.trader = if self.trader == MenuKind::Nu { MenuKind::Kf } else { MenuKind::Nu },
            "aim" => self.aim_hold = !self.aim_hold,
            "mouse_sensitivity" => self.mouse_sensitivity = step_sensitivity(self.mouse_sensitivity, d),
            "invert_mouse" => self.invert_mouse = !self.invert_mouse,
            "map_vote" => self.vote.enabled = !self.vote.enabled,
            _ => {}
        }
    }

    /// The Resolution `<` / `>` buttons: one step through Default and
    /// `sizes` (the monitor's, largest first), wrapping. A saved size the
    /// monitor does not list steps to the list's first entry.
    pub fn step_resolution(&mut self, d: i32, sizes: &[(u32, u32)]) {
        let list: Vec<Option<(u32, u32)>> = std::iter::once(None).chain(sizes.iter().copied().map(Some)).collect();
        let i = list.iter().position(|w| *w == self.window).unwrap_or(0);
        self.window = list[(i as i64 + d as i64).rem_euclid(list.len() as i64) as usize];
    }

    /// Why PLAY cannot be used now, if it cannot.
    pub fn problem(&self) -> Option<String> {
        match self.play {
            PlayType::Host => match self.port.trim().parse::<u16>() {
                // The query port is the game port + 1 (net/query.rs).
                Ok(p) if (1..u16::MAX).contains(&p) => {}
                _ => return Some(format!("The port must be a number from 1 to 65534 (it is \"{}\").", self.port)),
            },
            PlayType::Join if self.address.trim().is_empty() => return Some("Type the host's address to join.".into()),
            _ => {}
        }
        if self.play != PlayType::Join && self.map.trim().is_empty() {
            return Some("Pick a map.".into());
        }
        split_words(&self.extra).err()
    }

    /// The game's command-line arguments for these choices. `mute`: add
    /// `--mute` whatever the sound choice (the launcher's own `--mute`).
    pub fn to_args(&self, mute: bool) -> Result<Vec<String>, String> {
        if let Some(p) = self.problem() {
            return Err(p);
        }
        let mut a: Vec<String> = Vec::new();
        let mut push = |xs: &[&str]| a.extend(xs.iter().map(|s| s.to_string()));
        match self.play {
            PlayType::Solo => {}
            PlayType::Host => push(&["--host", self.port.trim()]),
            PlayType::Join => push(&["--join", self.address.trim()]),
        }
        // A joiner takes the map, mode, length and difficulty from the host.
        if self.play != PlayType::Join {
            push(&["--map", self.map.trim()]);
            // The map list and voting: only what differs from KF's
            // defaults (the game reads the same settings file too).
            if self.rotation.maps != MapRotation::default().maps {
                push(&["--map-list", &map_rotation::map_list_text(&self.rotation.maps)]);
            }
            if self.vote.enabled {
                push(&["--map-vote"]);
            }
            if self.vote.time_limit != map_rotation::KF_VOTE_TIME_LIMIT {
                push(&["--vote-time", &self.vote.time_limit.to_string()]);
            }
            if self.waves {
                push(&["--mode", "waves", "--length", LENGTHS[self.length.min(2)], "--difficulty", self.difficulty.word()]);
                if let Some(w) = self.start_wave {
                    push(&["--wave", &w.to_string()]);
                }
            } else {
                push(&["--mode", "debug"]);
            }
        }
        if !self.name.trim().is_empty() {
            push(&["--name", self.name.trim()]);
        }
        if let Some(p) = self.perk {
            push(&["--perk", perk_word(p), "--perk-level", &self.perk_level.to_string()]);
        }
        if !self.character.trim().is_empty() {
            push(&["--character", self.character.trim()]);
        }
        if let Some((w, h)) = self.window {
            push(&["--window", &format!("{w}x{h}")]);
        }
        if let Some(f) = self.fps {
            push(&["--fps", &f.to_string()]);
        }
        if !self.vsync {
            push(&["--no-vsync"]);
        }
        // Graphics: only what differs from the game's defaults.
        if self.display != DisplayMode::Windowed {
            push(&["--display", self.display.word()]);
        }
        if self.fov != graphics::DEFAULT_FOV {
            push(&["--fov", &self.fov.to_string()]);
        }
        if self.brightness != graphics::DEFAULT_BRIGHTNESS {
            push(&["--brightness", &self.brightness.to_string()]);
        }
        if self.msaa != graphics::DEFAULT_MSAA {
            push(&["--msaa", &if self.msaa <= 1 { 0 } else { self.msaa }.to_string()]);
        }
        if self.anisotropy != graphics::DEFAULT_ANISOTROPY {
            push(&["--anisotropy", &self.anisotropy.to_string()]);
        }
        if !self.sound || mute {
            push(&["--mute"]);
        }
        // The KF menu is the game's default: only NuMenu needs the option.
        if self.trader == MenuKind::Nu {
            push(&["--trader-menu", "nu"]);
        }
        a.extend(split_words(&self.extra)?);
        Ok(a)
    }

    /// The saved file's text: one `key=value` line per choice.
    pub fn to_text(&self) -> String {
        let mut s = String::from("# Open KF launcher choices (written when PLAY is pressed; the game rewrites the volume, aim and mouse lines)\n");
        for f in FIELDS {
            s.push_str(&format!("{f}={}\n", self.get(f)));
        }
        s
    }

    /// Reads a saved file's text over the defaults. Returns the lines that
    /// could not be used (kept out, the default stays).
    pub fn from_text(text: &str) -> (Choices, Vec<String>) {
        let mut c = Choices::default();
        let mut bad = Vec::new();
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let Some((k, v)) = line.split_once('=') else {
                bad.push(line.to_string());
                continue;
            };
            if let Err(e) = c.set(k.trim(), v) {
                bad.push(format!("{line} ({e})"));
            }
        }
        let maps = std::mem::take(&mut c.rotation.maps);
        c.rotation = c.rotation.with_maps(maps);
        (c, bad)
    }
}

/// The saved file's text with only the volume lines replaced (or added at
/// the end); every other line stays as it was. An empty text gets a
/// header line first.
pub fn with_volume_lines(text: &str, v: &Volumes) -> String {
    let c = Choices { volumes: *v, ..Default::default() };
    let lines: Vec<(&str, String)> = VOLUME_FIELDS.iter().map(|f| (*f, c.get(f))).collect();
    with_lines(text, &lines)
}

/// The saved file's text with only the `aim=` line replaced (or added).
pub fn with_aim_line(text: &str, hold: bool) -> String {
    with_lines(text, &[(AIM_FIELD, aim_word(hold).to_string())])
}

/// The saved file's text with only the mouse lines replaced (or added).
pub fn with_mouse_lines(text: &str, sensitivity: f32, invert: bool) -> String {
    let c = Choices { mouse_sensitivity: clamp_sensitivity(sensitivity), invert_mouse: invert, ..Default::default() };
    let lines: Vec<(&str, String)> = MOUSE_FIELDS.iter().map(|f| (*f, c.get(f))).collect();
    with_lines(text, &lines)
}

/// The saved file's text with only the `key=` lines of `lines` replaced
/// (or added at the end); every other line stays as it was. An empty
/// text gets a header line first.
pub fn with_lines(text: &str, lines: &[(&str, String)]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut done = vec![false; lines.len()];
    for line in text.lines() {
        let key = line.split_once('=').map(|(k, _)| k.trim());
        match key.and_then(|k| lines.iter().position(|(f, _)| *f == k)) {
            // A repeated line is dropped (the first one is replaced).
            Some(i) if done[i] => {}
            Some(i) => {
                out.push(format!("{}={}", lines[i].0, lines[i].1));
                done[i] = true;
            }
            None => out.push(line.to_string()),
        }
    }
    if out.is_empty() {
        out.push("# Open KF settings (the launcher writes its other choices here on PLAY)".into());
    }
    for (i, (f, v)) in lines.iter().enumerate() {
        if !done[i] {
            out.push(format!("{f}={v}"));
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// Splits typed options into arguments: spaces separate them, double
/// quotes keep spaces inside one ("--name \"Big Al\"").
pub fn split_words(s: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for ch in s.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if quoted {
        return Err("The extra arguments have an unclosed \" quote.".into());
    }
    if any {
        out.push(cur);
    }
    Ok(out)
}

/// Arguments as one line to show and log (quoted where they have spaces).
pub fn command_line(args: &[String]) -> String {
    args.iter().map(|a| if a.is_empty() || a.contains(char::is_whitespace) { format!("\"{a}\"") } else { a.clone() }).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn solo_default_args() {
        let c = Choices::default();
        assert_eq!(c.to_args(false).unwrap(), s(&["--map", "KF-WestLondon", "--mode", "waves", "--length", "short", "--difficulty", "normal", "--character", "Corporal_Lewis"]));
    }

    #[test]
    fn host_and_join_args() {
        let mut c = Choices { play: PlayType::Host, port: "7800".into(), map: "KF-Farm".into(), length: 2, difficulty: Difficulty::HellOnEarth, start_wave: Some(3), ..Default::default() };
        c.name = "Big Al".into();
        c.perk = Some(Perk::Support);
        c.perk_level = 6;
        c.sound = false;
        assert_eq!(
            c.to_args(false).unwrap(),
            s(&["--host", "7800", "--map", "KF-Farm", "--mode", "waves", "--length", "long", "--difficulty", "hoe", "--wave", "3", "--name", "Big Al", "--perk", "support", "--perk-level", "6", "--character", "Corporal_Lewis", "--mute"])
        );
        // Joining: no map, mode, length, difficulty or wave (the host decides).
        c.play = PlayType::Join;
        c.address = " 192.168.1.20:7707 ".into();
        c.window = Some((1280, 720));
        c.fps = Some(60);
        c.vsync = false;
        c.extra = "--god --give \"all\"".into();
        assert_eq!(
            c.to_args(true).unwrap(),
            s(&["--join", "192.168.1.20:7707", "--name", "Big Al", "--perk", "support", "--perk-level", "6", "--character", "Corporal_Lewis", "--window", "1280x720", "--fps", "60", "--no-vsync", "--mute", "--god", "--give", "all"])
        );
    }

    #[test]
    fn debug_mode_drops_length_and_wave() {
        let c = Choices { waves: false, start_wave: Some(4), ..Default::default() };
        let a = c.to_args(false).unwrap();
        assert!(a.windows(2).any(|w| w == ["--mode", "debug"]));
        assert!(!a.contains(&"--length".to_string()) && !a.contains(&"--wave".to_string()) && !a.contains(&"--difficulty".to_string()));
    }

    #[test]
    fn problems_block_play() {
        let c = Choices { play: PlayType::Host, port: "99999".into(), ..Default::default() };
        assert!(c.problem().is_some() && c.to_args(false).is_err());
        let c = Choices { play: PlayType::Join, ..Default::default() };
        assert!(c.problem().is_some());
        let c = Choices { extra: "--name \"oops".into(), ..Default::default() };
        assert!(c.problem().is_some());
    }

    #[test]
    fn saved_text_round_trips() {
        let mut c = Choices { play: PlayType::Join, address: "10.0.0.5".into(), map: "KF-Farm".into(), waves: false, length: 1, difficulty: Difficulty::Suicidal, start_wave: Some(11), ..Default::default() };
        c.name = "Jesse R".into();
        c.perk = Some(Perk::Demolitions);
        c.perk_level = 5;
        c.character = "Mr_Foster".into();
        c.window = Some((1600, 900));
        c.fps = Some(144);
        c.vsync = false;
        c.sound = false;
        c.trader = MenuKind::Kf;
        c.display = DisplayMode::Fullscreen;
        c.fov = 105;
        c.brightness = 140;
        c.msaa = 1;
        c.anisotropy = 16;
        c.extra = "--god --give all".into();
        c.volumes = Volumes { master: 0.75, effects: Some(0.123), music: None };
        let (back, bad) = Choices::from_text(&c.to_text());
        assert!(bad.is_empty(), "{bad:?}");
        assert_eq!(back, c);
        // Unknown and broken lines are reported; the rest is used.
        let (back, bad) = Choices::from_text("map=KF-Farm\nperk=wizard\nnonsense\ncolour=blue\n");
        assert_eq!(back.map, "KF-Farm");
        assert_eq!(back.perk, None);
        assert_eq!(bad.len(), 3);
    }

    #[test]
    fn steps_wrap() {
        let mut c = Choices::default();
        c.step("perk", -1, &[]);
        assert_eq!(c.perk, Some(Perk::Demolitions));
        c.step("perk", 1, &[]);
        assert_eq!(c.perk, None);
        c.step("wave", -1, &[]);
        assert_eq!(c.start_wave, Some(MAX_START_WAVE));
        c.step("wave", 1, &[]);
        assert_eq!(c.start_wave, None);
        // Normal, back to Beginner, back round to Hell on Earth.
        c.step("difficulty", -1, &[]);
        assert_eq!(c.difficulty, Difficulty::Beginner);
        c.step("difficulty", -1, &[]);
        assert_eq!(c.difficulty, Difficulty::HellOnEarth);
        let chars = s(&["A", "Corporal_Lewis", "Z"]);
        c.step("character", 1, &chars);
        assert_eq!(c.character, "Z");
        c.step("character", 1, &chars);
        assert_eq!(c.character, "A");
        c.step("window", 1, &[]);
        assert_eq!(c.window, WINDOW_SIZES[1]);
    }

    #[test]
    fn trader_menu_choice() {
        let mut c = Choices::default();
        assert_eq!(c.get("trader"), "kf");
        assert!(!c.to_args(false).unwrap().contains(&"--trader-menu".to_string()));
        c.step("trader", 1, &[]);
        assert_eq!(c.trader, MenuKind::Nu);
        let a = c.to_args(false).unwrap();
        assert!(a.windows(2).any(|w| w == ["--trader-menu", "nu"]));
        assert!(c.set("trader", "wizard").is_err());
        c.set("trader", "kf").unwrap();
        assert_eq!(c.trader, MenuKind::Kf);
    }

    #[test]
    fn volumes_parse_clamp_and_resolve() {
        let (c, bad) = Choices::from_text("volume=2\neffects_volume=0.25\nmusic_volume=default\n");
        assert!(bad.is_empty(), "{bad:?}");
        assert_eq!(c.volumes, Volumes { master: 1.0, effects: Some(0.25), music: None });
        let (c, bad) = Choices::from_text("volume=loud\neffects_volume=-1\nmusic_volume=0.9\n");
        assert_eq!(bad.len(), 1);
        assert_eq!(c.volumes, Volumes { master: 1.0, effects: Some(0.0), music: Some(KF_VOLUME_MAX) });
        // KillingFloor.ini's values where nothing is set; master scales both.
        let v = Volumes { master: 0.5, effects: None, music: Some(0.2) };
        assert_eq!(v.resolved((0.3, 0.1)), (0.3, 0.2));
        assert_eq!(v.heard((0.3, 0.1)), (0.15, 0.1));
        let mut v = Volumes::default();
        assert!(v.set_slider("music", 0.33333) && v.music == Some(0.333));
        assert!(v.set_slider("master", 7.0) && v.master == 1.0);
        assert!(!v.set_slider("voice", 0.1));
        assert_eq!(v.slider("effects", (0.3, 0.1)), Some((0.3, KF_VOLUME_MAX)));
    }

    #[test]
    fn volume_lines_replace_only_themselves() {
        let v = Volumes { master: 0.8, effects: Some(0.2), music: None };
        // Other lines (even unknown ones) stay; the volume lines change in place.
        let old = "# head\nmap=KF-Farm\nvolume=1.000\nfuture_option=x\nvolume=0.1\n";
        let new = with_volume_lines(old, &v);
        assert_eq!(new, "# head\nmap=KF-Farm\nvolume=0.800\nfuture_option=x\neffects_volume=0.200\nmusic_volume=default\n");
        let (c, _) = Choices::from_text(&new);
        assert_eq!((c.map.as_str(), c.volumes), ("KF-Farm", v));
        // No file yet: a header and the three lines.
        let fresh = with_volume_lines("", &v);
        assert!(fresh.starts_with('#') && fresh.lines().count() == 4);
        assert_eq!(Choices::from_text(&fresh).0.volumes, v);
    }

    #[test]
    fn graphics_choices() {
        let mut c = Choices::default();
        // Defaults add nothing to the command line.
        let a = c.to_args(false).unwrap();
        for o in ["--display", "--fov", "--brightness", "--msaa", "--anisotropy"] {
            assert!(!a.contains(&o.to_string()), "{o}");
        }
        c.step("display", 1, &[]);
        assert_eq!(c.display, DisplayMode::Borderless);
        c.step("display", -2, &[]);
        assert_eq!(c.display, DisplayMode::Fullscreen);
        c.step("fov", 1, &[]);
        assert_eq!(c.fov, 95);
        for _ in 0..20 {
            c.step("fov", 1, &[]);
            c.step("brightness", -1, &[]);
        }
        assert_eq!((c.fov, c.brightness), (graphics::FOV_MAX, graphics::BRIGHTNESS_MIN));
        c.step("msaa", 1, &[]);
        assert_eq!(c.msaa, 8);
        c.step("msaa", 1, &[]);
        assert_eq!(c.msaa, 1);
        assert_eq!(c.get("msaa"), "off");
        c.step("anisotropy", -1, &[]);
        assert_eq!(c.anisotropy, 4);
        let a = c.to_args(false).unwrap();
        assert!(a.windows(2).any(|w| w == ["--display", "fullscreen"]));
        assert!(a.windows(2).any(|w| w == ["--fov", "120"]));
        assert!(a.windows(2).any(|w| w == ["--brightness", "50"]));
        assert!(a.windows(2).any(|w| w == ["--msaa", "0"]));
        assert!(a.windows(2).any(|w| w == ["--anisotropy", "4"]));
        assert!(c.set("fov", "200").is_err() && c.set("msaa", "3").is_err() && c.set("display", "huge").is_err());
        c.set("msaa", "off").unwrap();
        assert_eq!(c.msaa, 1);
    }

    #[test]
    fn resolution_steps_through_the_monitor_sizes() {
        let mut c = Choices::default();
        let sizes = [(2560, 1440), (1920, 1080), (1280, 720)];
        c.step_resolution(1, &sizes);
        assert_eq!(c.window, Some((2560, 1440)));
        c.step_resolution(-2, &sizes);
        assert_eq!(c.window, Some((1280, 720)));
        c.step_resolution(1, &sizes);
        assert_eq!(c.window, None);
        // A saved size the monitor does not have: the next step starts over.
        c.window = Some((1600, 900));
        c.step_resolution(1, &sizes);
        assert_eq!(c.window, Some((2560, 1440)));
    }

    #[test]
    fn aim_line_parses_and_replaces_only_itself() {
        // Default: KF's toggle (RightMouse=ToggleAiming).
        assert!(!Choices::default().aim_hold);
        let (c, bad) = Choices::from_text("aim=HOLD\n");
        assert!(c.aim_hold && bad.is_empty());
        let (c, bad) = Choices::from_text("aim=sometimes\n");
        assert!(!c.aim_hold && bad.len() == 1);
        let old = "# head\nvolume=0.500\naim=toggle\nmap=KF-Farm\n";
        assert_eq!(with_aim_line(old, true), "# head\nvolume=0.500\naim=hold\nmap=KF-Farm\n");
        let added = with_aim_line("map=KF-Farm\n", true);
        assert_eq!(added, "map=KF-Farm\naim=hold\n");
        // The volume save keeps the aim line, and the other way round.
        let both = with_volume_lines(&added, &Volumes::default());
        assert!(Choices::from_text(&both).0.aim_hold);
        let mut c = Choices::default();
        c.step("aim", 1, &[]);
        assert_eq!(c.get("aim"), "hold");
    }

    #[test]
    fn mouse_lines_parse_step_and_replace_only_themselves() {
        // Defaults: KF's MouseSensitivity 3, bInvertMouse False.
        let c = Choices::default();
        assert_eq!((c.mouse_sensitivity, c.invert_mouse), (3.0, false));
        assert_eq!((c.get("mouse_sensitivity").as_str(), c.get("invert_mouse").as_str()), ("3.00", "off"));
        let (c, bad) = Choices::from_text("mouse_sensitivity=1.626\ninvert_mouse=ON\n");
        assert!(bad.is_empty(), "{bad:?}");
        assert_eq!((c.mouse_sensitivity, c.invert_mouse), (1.63, true));
        // Outside KF's box (0.25 to 25) or not a number: refused, default kept.
        for t in ["mouse_sensitivity=0.1", "mouse_sensitivity=30", "mouse_sensitivity=fast", "mouse_sensitivity=NaN", "invert_mouse=maybe"] {
            let (c, bad) = Choices::from_text(t);
            assert_eq!(bad.len(), 1, "{t}");
            assert_eq!((c.mouse_sensitivity, c.invert_mouse), (3.0, false), "{t}");
        }
        // Steps of 0.25, stopping at the ends.
        let mut c = Choices::default();
        c.step("mouse_sensitivity", 1, &[]);
        assert_eq!(c.mouse_sensitivity, 3.25);
        c.step("mouse_sensitivity", -2, &[]);
        assert_eq!(c.mouse_sensitivity, 2.75);
        for _ in 0..200 {
            c.step("mouse_sensitivity", -1, &[]);
        }
        assert_eq!(c.mouse_sensitivity, SENSITIVITY_MIN);
        for _ in 0..200 {
            c.step("mouse_sensitivity", 1, &[]);
        }
        assert_eq!(c.mouse_sensitivity, SENSITIVITY_MAX);
        c.step("invert_mouse", 1, &[]);
        assert!(c.invert_mouse);
        // The pause menu's slider: the ends, and drags land on 0.25 steps.
        assert_eq!((sensitivity_at_fraction(0.0), sensitivity_at_fraction(1.0)), (SENSITIVITY_MIN, SENSITIVITY_MAX));
        assert_eq!(sensitivity_at_fraction(sensitivity_fraction(3.0)), 3.0);
        assert_eq!(sensitivity_at_fraction(0.1), 2.75);
        assert_eq!(sensitivity_fraction(30.0), 1.0);
        // The game's save rewrites only the mouse lines.
        let old = "# head\nvolume=0.500\nmouse_sensitivity=3.00\naim=hold\n";
        assert_eq!(with_mouse_lines(old, 1.5, true), "# head\nvolume=0.500\nmouse_sensitivity=1.50\naim=hold\ninvert_mouse=on\n");
        let all = with_aim_line(&with_volume_lines(&with_mouse_lines("", 7.25, true), &Volumes::default()), true);
        let (c, bad) = Choices::from_text(&all);
        assert!(bad.is_empty() && c.aim_hold && c.invert_mouse && c.mouse_sensitivity == 7.25, "{all}");
        // The launcher's whole file keeps them too.
        let mut c = Choices::default();
        c.set("mouse_sensitivity", "12.5").unwrap();
        c.set("invert_mouse", "yes").unwrap();
        assert_eq!(Choices::from_text(&c.to_text()).0, c);
    }

    #[test]
    fn words_split_with_quotes() {
        assert_eq!(split_words("  --god  --name \"Big Al\" \"\" ").unwrap(), s(&["--god", "--name", "Big Al", ""]));
        assert_eq!(command_line(&s(&["--name", "Big Al", "--god"])), "--name \"Big Al\" --god");
    }

    #[test]
    fn map_rotation_saved_and_passed() {
        // An old file without the lines: KF's defaults.
        let (c, bad) = Choices::from_text("map=KF-Farm\naim=hold\n");
        assert!(bad.is_empty());
        assert_eq!((c.rotation.clone(), c.vote), (MapRotation::default(), MapVoteConfig::default()));
        assert!(!c.to_args(false).unwrap().iter().any(|a| a.starts_with("--map-") || a == "--vote-time"));
        // Round trip.
        let mut c = Choices::default();
        c.rotation = MapRotation::new(s(&["KF-Farm", "KF-Manor"]), 1);
        c.vote = MapVoteConfig { enabled: true, time_limit: 45 };
        let back = Choices::from_text(&c.to_text()).0;
        assert_eq!(back, c);
        assert!(c.to_text().contains("map_list=KF-Farm,KF-Manor\nmap_position=1\nmap_vote=on\nvote_time_limit=45\n"));
        let a = c.to_args(false).unwrap();
        assert!(a.windows(2).any(|w| w == ["--map-list", "KF-Farm,KF-Manor"]));
        assert!(a.contains(&"--map-vote".to_string()) && a.windows(2).any(|w| w == ["--vote-time", "45"]));
        // A joiner takes the host's.
        c.play = PlayType::Join;
        c.address = "10.0.0.1".into();
        assert!(!c.to_args(false).unwrap().iter().any(|a| a.starts_with("--map-") || a == "--vote-time"));
        // A position past the end goes back to 0, whatever the line order.
        let (c, _) = Choices::from_text("map_position=7\nmap_list=KF-Farm,KF-Manor\n");
        assert_eq!(c.rotation.position, 0);
        // An empty list is kept empty (the game then falls back).
        let (c, bad) = Choices::from_text("map_list=\n");
        assert!(bad.is_empty() && c.rotation.maps.is_empty());
        // Bad values are refused, the default stays.
        let (c, bad) = Choices::from_text("vote_time_limit=1\nmap_vote=maybe\n");
        assert_eq!((bad.len(), c.vote), (2, MapVoteConfig::default()));
        // The voting switch.
        let mut c = Choices::default();
        c.step("map_vote", 1, &[]);
        assert!(c.vote.enabled);
    }
}
