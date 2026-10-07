//! What the launcher's player chose, the game arguments that makes, and
//! the `key=value` text the choices are saved as. No Bevy here, so it is
//! all unit-tested. See DESIGN.md, "The launcher".

use crate::game::buy_menu::MenuKind;
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

/// The window sizes offered (None: the system's default window).
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
pub const FIELDS: [&str; 17] = [
    "play", "port", "address", "map", "mode", "length", "wave", "name", "perk", "level", "character", "window", "fps", "vsync", "sound", "trader", "extra",
];

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
    /// None: from the first wave.
    pub start_wave: Option<u32>,
    /// Empty: the game's default (KF's defuser.ini name).
    pub name: String,
    pub perk: Option<Perk>,
    pub perk_level: u8,
    pub character: String,
    pub window: Option<(u32, u32)>,
    pub fps: Option<u32>,
    pub vsync: bool,
    pub sound: bool,
    /// The trader's menu (`--trader-menu`): NuMenu (default) or KF's.
    pub trader: MenuKind,
    /// More options typed by hand (e.g. `--god`).
    pub extra: String,
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
            start_wave: None,
            name: String::new(),
            perk: None,
            perk_level: 0,
            character: DEFAULT_CHARACTER.to_string(),
            window: None,
            fps: None,
            vsync: true,
            sound: true,
            trader: MenuKind::Nu,
            extra: String::new(),
        }
    }
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
            "wave" => self.start_wave.map_or("start".into(), |w| w.to_string()),
            "name" => self.name.clone(),
            "perk" => self.perk.map_or("none", perk_word).into(),
            "level" => self.perk_level.to_string(),
            "character" => self.character.clone(),
            "window" => self.window.map_or("default".into(), |(w, h)| format!("{w}x{h}")),
            "fps" => self.fps.map_or("none".into(), |f| f.to_string()),
            "vsync" => on_off(self.vsync).into(),
            "sound" => on_off(self.sound).into(),
            "trader" => self.trader.word().into(),
            "extra" => self.extra.clone(),
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
            "sound" => self.sound = parse_bool(v)?,
            "trader" => self.trader = MenuKind::parse(v).ok_or(format!("not nu/kf: {v}"))?,
            "extra" => self.extra = v.trim().into(),
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
            "sound" => self.sound = !self.sound,
            "trader" => self.trader = if self.trader == MenuKind::Nu { MenuKind::Kf } else { MenuKind::Nu },
            _ => {}
        }
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
        // A joiner takes the map, mode and length from the host.
        if self.play != PlayType::Join {
            push(&["--map", self.map.trim()]);
            if self.waves {
                push(&["--mode", "waves", "--length", LENGTHS[self.length.min(2)]]);
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
        if !self.sound || mute {
            push(&["--mute"]);
        }
        // NuMenu is the game's default: only the KF menu needs the option.
        if self.trader == MenuKind::Kf {
            push(&["--trader-menu", "kf"]);
        }
        a.extend(split_words(&self.extra)?);
        Ok(a)
    }

    /// The saved file's text: one `key=value` line per choice.
    pub fn to_text(&self) -> String {
        let mut s = String::from("# Open KF launcher choices (written when PLAY is pressed)\n");
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
        (c, bad)
    }
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
        assert_eq!(c.to_args(false).unwrap(), s(&["--map", "KF-WestLondon", "--mode", "waves", "--length", "short", "--character", "Corporal_Lewis"]));
    }

    #[test]
    fn host_and_join_args() {
        let mut c = Choices { play: PlayType::Host, port: "7800".into(), map: "KF-Farm".into(), length: 2, start_wave: Some(3), ..Default::default() };
        c.name = "Big Al".into();
        c.perk = Some(Perk::Support);
        c.perk_level = 6;
        c.sound = false;
        assert_eq!(
            c.to_args(false).unwrap(),
            s(&["--host", "7800", "--map", "KF-Farm", "--mode", "waves", "--length", "long", "--wave", "3", "--name", "Big Al", "--perk", "support", "--perk-level", "6", "--character", "Corporal_Lewis", "--mute"])
        );
        // Joining: no map, mode, length or wave (the host decides).
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
        assert!(!a.contains(&"--length".to_string()) && !a.contains(&"--wave".to_string()));
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
        let mut c = Choices { play: PlayType::Join, address: "10.0.0.5".into(), map: "KF-Farm".into(), waves: false, length: 1, start_wave: Some(11), ..Default::default() };
        c.name = "Jesse R".into();
        c.perk = Some(Perk::Demolitions);
        c.perk_level = 5;
        c.character = "Mr_Foster".into();
        c.window = Some((1600, 900));
        c.fps = Some(144);
        c.vsync = false;
        c.sound = false;
        c.trader = MenuKind::Kf;
        c.extra = "--god --give all".into();
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
        assert_eq!(c.get("trader"), "nu");
        assert!(!c.to_args(false).unwrap().contains(&"--trader-menu".to_string()));
        c.step("trader", 1, &[]);
        assert_eq!(c.trader, MenuKind::Kf);
        let a = c.to_args(false).unwrap();
        assert!(a.windows(2).any(|w| w == ["--trader-menu", "kf"]));
        assert!(c.set("trader", "wizard").is_err());
        c.set("trader", "nu").unwrap();
        assert_eq!(c.trader, MenuKind::Nu);
    }

    #[test]
    fn words_split_with_quotes() {
        assert_eq!(split_words("  --god  --name \"Big Al\" \"\" ").unwrap(), s(&["--god", "--name", "Big Al", ""]));
        assert_eq!(command_line(&s(&["--name", "Big Al", "--god"])), "--name \"Big Al\" --god");
    }
}
