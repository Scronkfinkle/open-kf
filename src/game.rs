//! The game loop (`--mode waves`): KFGameType's wave state machine. `--mode
//! debug` (the default) leaves everything as before: no waves, zeds only
//! from the spawn keys and test flags.
//!
//! Rules from KFMod.KFGameType state MatchInProgress (Timer once a
//! second), SetupWave, BuildNextSquad, AddSquad, CalcNextSquadSpawnTime and
//! DoWaveEnd; squads and waves from the KFGameType / KFMonstersCollection
//! class defaults (see DESIGN.md, "Game loop"). One player, Normal
//! difficulty.

use std::rc::Rc;

use avian3d::prelude::*;
use bevy::prelude::*;
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package_set::{LoadedPackage, PackageSet};
use ue_assets::properties::{PropertyList, Value, read_export_properties, string_array, struct_array};

use crate::coords::{self, SCALE};
use crate::runlog;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GameMode {
    /// Today's test setup: no waves.
    #[default]
    Debug,
    /// KF's waves.
    Waves,
}

/// KFGameType.KFGameLength: GL_Short 0 (KF's default, KillingFloor.ini),
/// GL_Normal 1, GL_Long 2.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GameLength {
    #[default]
    Short,
    Normal,
    Long,
}

impl GameLength {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "short" => Some(Self::Short),
            "normal" => Some(Self::Normal),
            "long" => Some(Self::Long),
            _ => None,
        }
    }

    /// The KFGameType defaults array of this length, and of the special
    /// squads in KFMonstersCollection.
    fn waves_property(self) -> (&'static str, &'static str) {
        match self {
            Self::Short => ("ShortWaves", "ShortSpecialSquads"),
            Self::Normal => ("NormalWaves", "NormalSpecialSquads"),
            Self::Long => ("LongWaves", "LongSpecialSquads"),
        }
    }
}

#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct GameOptions {
    pub mode: GameMode,
    pub length: GameLength,
    /// Test: start at this wave (1-based; one past the last = the boss).
    pub start_wave: Option<usize>,
}

/// KFGameType.Waves[i] (KFGameType.WaveInfo).
#[derive(Clone, Copy, Debug)]
pub struct WaveConfig {
    pub mask: u32,
    pub max_monsters: i32,
}

/// A ZombieVolume (G1: only where it is).
#[derive(Clone, Debug)]
pub struct SpawnVolume {
    pub name: String,
    /// Unreal units.
    pub location: [f32; 3],
}

/// Everything the wave loop reads from the game's classes and the map.
#[derive(Resource, Clone, Debug, Default)]
pub struct GameData {
    pub waves: Vec<WaveConfig>,
    /// InitSquads: each StandardMonsterSquads entry as zed class paths.
    pub squads: Vec<Vec<String>>,
    /// The collection's special squad per wave (empty = none).
    pub special: Vec<Vec<String>>,
    pub boss_class: String,
    /// KFLevelRules.WaveSpawnPeriod (the map's, else the class default 2).
    pub spawn_period: f32,
    pub volumes: Vec<SpawnVolume>,
}

/// KFGameType defaults for Normal difficulty.
const TIME_BETWEEN_WAVES: i32 = 60; // TimeBetweenWavesNormal
const FIRST_COUNTDOWN: i32 = 10; // MatchInProgress.BeginState
const MAX_ZOMBIES_ONCE: i32 = 32; // StandardMaxZombiesOnce
const SINE_WAVE_FREQ: f32 = 0.04; // SineWaveFreq

fn class_of_letter(letters: &[(String, String)], c: char) -> Option<String> {
    letters.iter().find(|(id, _)| id.eq_ignore_ascii_case(&c.to_string())).map(|(_, class)| class.clone())
}

/// KFGameType.LoadUpMonsterList: "4A1G" = 4 of A, 1 of G.
pub fn parse_squad(s: &str, letters: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    let mut count = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() {
            count.push(c);
            continue;
        }
        let n: usize = count.parse().unwrap_or(1);
        count.clear();
        if let Some(class) = class_of_letter(letters, c) {
            out.extend(std::iter::repeat_n(class, n));
        }
    }
    out
}

fn int_array(value: Option<&Value>) -> Vec<i32> {
    match value {
        Some(Value::Array { count, raw }) => raw.chunks_exact(4).take(*count).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect(),
        _ => Vec::new(),
    }
}

/// Reads the wave tables (class defaults) and the map's ZombieVolumes and
/// KFLevelRules. Called by the map loader.
pub fn load_game_data(set: &PackageSet, defaults: &ClassDefaults, map: &Rc<LoadedPackage>, length: GameLength) -> GameData {
    let mut data = GameData {
        spawn_period: 2.0,
        ..default()
    };
    let (waves_prop, special_prop) = length.waves_property();
    let Some(game_lp) = set.load("KFMod") else {
        runlog::kv("game_data_error", "reason=no_KFMod");
        return data;
    };
    let class = |pkg: &Rc<LoadedPackage>, name: &str| {
        (0..pkg.pkg.exports.len())
            .find(|&i| pkg.pkg.export_class_name(i) == "Class" && pkg.pkg.object_name(ue_assets::package::ObjectRef::Export(i)) == name)
            .map(|export| ue_assets::package_set::ObjectHandle { package: pkg.clone(), export })
    };
    let (Some(game), Some(coll)) = (class(&game_lp, "KFGameType"), class(&game_lp, "KFMonstersCollection")) else {
        runlog::kv("game_data_error", "reason=classes_not_found");
        return data;
    };
    for i in 0..16 {
        let Some((Value::TaggedStruct { props, .. }, from)) = defaults.get_at(&game, waves_prop, i) else {
            break;
        };
        // WaveInfo: int WaveMask, byte WaveMaxMonsters.
        let int = |name: &str| match props.get(&from.pkg, name) {
            Some(Value::Int(n)) => *n,
            Some(Value::Byte(b)) => *b as i32,
            _ => 0,
        };
        data.waves.push(WaveConfig {
            mask: int("WaveMask") as u32,
            max_monsters: int("WaveMaxMonsters"),
        });
    }
    // MonsterClasses: (MID letter, MClassName).
    let mut letters: Vec<(String, String)> = Vec::new();
    if let Some((v, from)) = defaults.get(&coll, "MonsterClasses") {
        for p in struct_array(&from.pkg, &v) {
            let s = |n: &str| match p.get(&from.pkg, n) {
                Some(Value::Str(x)) => x.clone(),
                _ => String::new(),
            };
            letters.push((s("MID"), s("MClassName")));
        }
    }
    if let Some((v, _)) = defaults.get(&game, "StandardMonsterSquads") {
        let strings = string_array(&v);
        runlog::kv("game_squads", &format!("standard=[{}]", strings.join(" ")));
        data.squads = strings.iter().map(|s| parse_squad(s, &letters)).collect();
    }
    if let Some((v, from)) = defaults.get(&coll, special_prop) {
        for p in struct_array(&from.pkg, &v) {
            let classes = p.get(&from.pkg, "ZedClass").map(string_array).unwrap_or_default();
            let counts = int_array(p.get(&from.pkg, "NumZeds"));
            let mut squad = Vec::new();
            for (c, n) in classes.iter().zip(counts.iter()) {
                if !c.is_empty() {
                    squad.extend(std::iter::repeat_n(c.clone(), (*n).max(0) as usize));
                }
            }
            data.special.push(squad);
        }
    }
    data.boss_class = match defaults.get(&coll, "EndGameBossClass") {
        Some((Value::Str(s), _)) => s,
        _ => "KFChar.ZombieBoss_STANDARD".into(),
    };
    // The map: ZombieVolumes (not bObjectiveModeOnly) and KFLevelRules.
    let pkg = &map.pkg;
    for i in 0..pkg.exports.len() {
        let class = pkg.export_class_name(i);
        if class != "ZombieVolume" && class != "KFLevelRules" {
            continue;
        }
        let Ok(props) = read_export_properties(pkg, i) else { continue };
        let get = |p: &PropertyList, n: &str| p.get(pkg, n).cloned();
        if class == "KFLevelRules" {
            if let Some(Value::Float(f)) = get(&props, "WaveSpawnPeriod") {
                data.spawn_period = f;
            }
            continue;
        }
        if matches!(get(&props, "bObjectiveModeOnly"), Some(Value::Bool(true))) {
            continue;
        }
        let location = match get(&props, "Location") {
            Some(Value::Vector(v)) => v,
            _ => continue,
        };
        data.volumes.push(SpawnVolume {
            name: pkg.object_name(ue_assets::package::ObjectRef::Export(i)).to_string(),
            location,
        });
    }
    runlog::kv(
        "game_data",
        &format!(
            "length={length:?} waves={} max_monsters={:?} squads={} letters={} special={:?} boss={} spawn_period={} volumes={}",
            data.waves.len(),
            data.waves.iter().map(|w| w.max_monsters).collect::<Vec<_>>(),
            data.squads.len(),
            letters.iter().map(|(l, c)| format!("{l}={}", c.rsplit('.').next().unwrap_or(c))).collect::<Vec<_>>().join(","),
            data.special.iter().map(|s| s.len()).collect::<Vec<_>>(),
            data.boss_class,
            data.spawn_period,
            data.volumes.len()
        ),
    );
    data
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Between waves (WaveCountDown running; trader time after wave 1).
    Countdown,
    Wave,
    BossWave,
    Won,
    Lost,
}

/// KFGameType's wave state.
#[derive(Resource, Debug)]
pub struct WaveGame {
    pub phase: Phase,
    /// WaveNum: 0-based; the boss wave is `final_wave`.
    pub wave_num: usize,
    pub final_wave: usize,
    pub countdown: i32,
    pub total_max_monsters: i32,
    pub max_monsters: i32,
    squads_to_use: Vec<usize>,
    special_counter: u32,
    used_special: bool,
    next_squad: Vec<String>,
    next_monster_time: f32,
    wave_time_elapsed: f32,
    /// LastZVol: the volume a squad is being spawned in.
    last_volume: Option<usize>,
    /// The Timer's next tick (game seconds).
    next_tick: f32,
    pub living: usize,
    rng: u32,
    deaths_at_start: Option<u32>,
}

impl Default for WaveGame {
    fn default() -> Self {
        WaveGame {
            phase: Phase::Countdown,
            wave_num: 0,
            final_wave: 0,
            countdown: FIRST_COUNTDOWN,
            total_max_monsters: 0,
            max_monsters: 0,
            squads_to_use: Vec::new(),
            special_counter: 1,
            used_special: false,
            next_squad: Vec::new(),
            next_monster_time: 0.0,
            wave_time_elapsed: 0.0,
            last_volume: None,
            next_tick: 1.0,
            living: 0,
            rng: 0x2545_F491,
            deaths_at_start: None,
        }
    }
}

impl WaveGame {
    /// Rand(n).
    fn rand(&mut self, n: usize) -> usize {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        if n == 0 { 0 } else { self.rng as usize % n }
    }

    fn frand(&mut self) -> f32 {
        self.rand(10000) as f32 / 10000.0
    }

    /// The squads this wave's WaveMask enables (inserted at the front, so
    /// the list is in reverse order, as KF builds it).
    fn masked_squads(&self, data: &GameData) -> Vec<usize> {
        let mask = data.waves.get(self.wave_num).map_or(0, |w| w.mask);
        let mut list = Vec::new();
        for i in 0..data.squads.len().min(32) {
            if mask & (1 << i) != 0 {
                list.insert(0, i);
            }
        }
        list
    }

    /// SetupWave: zeds this wave (WaveMaxMonsters x DifficultyMod 1.0
    /// (Normal) x NumPlayersMod 1 (one player), clamped 5..800), at most
    /// MaxZombiesOnce alive, the squad list, the first squad.
    fn setup_wave(&mut self, data: &GameData) {
        let max = data.waves.get(self.wave_num).map_or(5, |w| w.max_monsters);
        self.total_max_monsters = max.clamp(5, 800);
        self.max_monsters = self.total_max_monsters.clamp(5, MAX_ZOMBIES_ONCE);
        self.squads_to_use = self.masked_squads(data);
        self.used_special = false;
        self.special_counter = 1;
        self.build_next_squad(data);
        runlog::kv(
            "wave_start",
            &format!(
                "wave={} of={} zeds={} max_at_once={} squads={:?}",
                self.wave_num + 1,
                self.final_wave,
                self.total_max_monsters,
                self.max_monsters,
                self.squads_to_use
            ),
        );
    }

    /// BuildNextSquad: a random squad from the list, removed from it; an
    /// empty list is refilled from the mask (SpecialListCounter + 1).
    fn build_next_squad(&mut self, data: &GameData) {
        if self.squads_to_use.is_empty() {
            self.squads_to_use = self.masked_squads(data);
            if self.squads_to_use.is_empty() {
                runlog::kv("game_warning", "reason=no_squads");
                return;
            }
            self.special_counter += 1;
            self.used_special = false;
        }
        let i = self.rand(self.squads_to_use.len());
        let squad = self.squads_to_use.remove(i);
        self.next_squad = data.squads[squad].clone();
    }

    /// CalcNextSquadSpawnTime for one player at Normal difficulty.
    fn next_squad_time(&self, data: &GameData, length: GameLength) -> f32 {
        let sine = 1.0 - (self.wave_time_elapsed * SINE_WAVE_FREQ).sin().abs();
        let late = match length {
            GameLength::Short => self.wave_num >= 2,
            GameLength::Normal => self.wave_num >= 4,
            GameLength::Long => self.wave_num >= 7,
        };
        let t = data.spawn_period * if late { 1.1 } else { 1.0 };
        t + sine * t * 2.0
    }
}

/// A zed for `zed.rs` to spawn: class path, the floor point it stands on
/// (Unreal units), yaw (Unreal rotation units).
#[derive(Message, Clone, Debug)]
pub struct SpawnZedAt {
    pub class: String,
    pub floor: Vec3,
    pub yaw: f32,
}

/// Kill every living zed (a restart clears the map).
#[derive(Message, Clone, Copy, Debug)]
pub struct ClearZeds;

/// The wave line on the HUD (empty in debug mode).
#[derive(Resource, Default)]
pub struct WaveHud(pub String);

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WaveGame>()
            .init_resource::<WaveHud>()
            .add_message::<SpawnZedAt>()
            .add_message::<ClearZeds>()
            .add_systems(Update, wave_timer);
    }
}

/// Where a squad member can stand in volume `v` (G1: the volume's pivot,
/// spread on a 60-unit row, dropped to the floor). Unreal units.
fn spawn_spot(spatial: &SpatialQuery, data: &GameData, v: usize, k: usize) -> Option<Vec3> {
    let l = data.volumes[v].location;
    let offset = (k as f32 - 2.0) * 60.0;
    let p = Vec3::new(l[0] + offset, l[1], l[2]);
    let from = coords::pos(p.to_array());
    let hit = spatial.cast_ray(from, Dir3::NEG_Y, 400.0 * SCALE, true, &crate::collision::world_filter())?;
    Some(p - Vec3::Z * (hit.distance / SCALE))
}

#[allow(clippy::too_many_arguments)]
fn wave_timer(
    time: Res<Time>,
    options: Res<GameOptions>,
    data: Option<Res<GameData>>,
    mut game: ResMut<WaveGame>,
    mut hud: ResMut<WaveHud>,
    zeds: Query<&crate::zed::Zed>,
    health: Res<crate::combat::PlayerHealth>,
    spatial: SpatialQuery,
    frames: Res<bevy::diagnostic::FrameCount>,
    mut spawns: MessageWriter<SpawnZedAt>,
    script: Res<crate::weapon::ScriptedInput>,
    (keys, mut clear): (Res<ButtonInput<KeyCode>>, MessageWriter<ClearZeds>),
) {
    if options.mode != GameMode::Waves || frames.0 < 10 {
        return;
    }
    // Test action "next_wave": the countdown ends at the next tick.
    let skip = script.0.iter().any(|(f, a)| *f == frames.0 && a == "next_wave");
    let Some(data) = data else { return };
    let now = time.elapsed_secs();
    let g = &mut *game;
    // After a win or a loss, Enter (test action "restart_game") starts over.
    let restart = keys.just_pressed(KeyCode::Enter) || script.0.iter().any(|(f, a)| *f == frames.0 && a == "restart_game");
    if restart && matches!(g.phase, Phase::Won | Phase::Lost) {
        *g = WaveGame {
            deaths_at_start: Some(health.deaths),
            final_wave: data.waves.len(),
            next_tick: now + 1.0,
            ..default()
        };
        clear.write(ClearZeds);
        runlog::kv("game_start", &format!("mode=waves length={:?} final_wave={} countdown={} restart=true", options.length, g.final_wave, g.countdown));
    }
    if skip && g.phase == Phase::Countdown {
        g.countdown = 1;
        runlog::kv("wave_countdown", "skipped=true");
    }
    if g.deaths_at_start.is_none() {
        g.deaths_at_start = Some(health.deaths);
        g.final_wave = data.waves.len();
        g.next_tick = now + 1.0;
        if let Some(w) = options.start_wave {
            g.wave_num = w.saturating_sub(1).min(g.final_wave);
        }
        runlog::kv("game_start", &format!("mode=waves length={:?} final_wave={} countdown={}", options.length, g.final_wave, g.countdown));
    }
    g.living = zeds.iter().filter(|z| !z.is_dead()).count();
    // UpdateMonsterCount: no living player ends the game (solo: no respawn).
    if matches!(g.phase, Phase::Countdown | Phase::Wave | Phase::BossWave) && health.deaths > g.deaths_at_start.unwrap_or(0) {
        g.phase = Phase::Lost;
        runlog::kv("game_end", &format!("result=lost wave={}", g.wave_num + 1));
    }
    hud.0 = match g.phase {
        Phase::Countdown => format!("WAVE {}/{}  NEXT IN {}", g.wave_num + 1, g.final_wave, g.countdown.max(0)),
        Phase::Wave => format!("WAVE {}/{}  ZEDS {}", g.wave_num + 1, g.final_wave, g.total_max_monsters.max(0) as usize + g.living),
        Phase::BossWave => "PATRIARCH".into(),
        Phase::Won => "YOU WON - ENTER TO PLAY AGAIN".into(),
        Phase::Lost => "YOU DIED - ENTER TO PLAY AGAIN".into(),
    };
    if now < g.next_tick {
        return;
    }
    // MatchInProgress.Timer, once a second.
    g.next_tick += 1.0;
    let num_monsters = g.living as i32;
    match g.phase {
        Phase::Won | Phase::Lost => {}
        Phase::BossWave => {
            // StartWaveBoss / AddBoss (G3 adds the boss's own rules).
            if g.total_max_monsters > 0 {
                if let Some(v) = (!data.volumes.is_empty()).then(|| g.rand(data.volumes.len()))
                    && let Some(at) = spawn_spot(&spatial, &data, v, 2)
                {
                    spawns.write(SpawnZedAt { class: data.boss_class.clone(), floor: at, yaw: 0.0 });
                    g.total_max_monsters = 0;
                    runlog::kv("boss_spawned", &format!("volume={}", data.volumes[v].name));
                }
            } else if num_monsters <= 0 {
                do_wave_end(g);
            }
        }
        Phase::Wave => {
            g.wave_time_elapsed += 1.0;
            if g.total_max_monsters <= 0 {
                if num_monsters <= 0 {
                    do_wave_end(g);
                }
            } else if now > g.next_monster_time && num_monsters + g.next_squad.len() as i32 <= g.max_monsters {
                add_squad(g, &data, num_monsters, &spatial, &mut spawns);
                g.next_monster_time = if g.next_squad.is_empty() { now + g.next_squad_time(&data, options.length) } else { now + 0.2 };
            }
        }
        Phase::Countdown => {
            if num_monsters > 0 {
                return;
            }
            // WaveNum == FinalWave + 1 after the boss wave: the game is won.
            if g.wave_num > g.final_wave {
                g.phase = Phase::Won;
                runlog::kv("game_end", "result=won");
                return;
            }
            g.countdown -= 1;
            if g.countdown % 10 == 0 && g.countdown > 0 {
                runlog::kv("wave_countdown", &format!("wave={} seconds={}", g.wave_num + 1, g.countdown));
            }
            if g.countdown <= 0 {
                if g.wave_num == g.final_wave {
                    // StartWaveBoss: TotalMaxMonsters 1.
                    g.phase = Phase::BossWave;
                    g.total_max_monsters = 1;
                    runlog::kv("wave_start", &format!("wave=boss class={}", data.boss_class));
                } else {
                    g.phase = Phase::Wave;
                    g.setup_wave(&data);
                }
            }
        }
    }
}

/// AddSquad: the special squad on odd passes through the list (once per
/// pass), else the next squad; then spawn as many as fit.
fn add_squad(g: &mut WaveGame, data: &GameData, num_monsters: i32, spatial: &SpatialQuery, spawns: &mut MessageWriter<SpawnZedAt>) {
    if g.last_volume.is_none() || g.next_squad.is_empty() {
        let special = data.special.get(g.wave_num).filter(|s| !s.is_empty());
        if let Some(sq) = special
            && !g.used_special
            && g.special_counter % 2 == 1
        {
            g.next_squad = sq.clone();
            g.used_special = true;
            runlog::kv("squad_special", &format!("wave={} zeds={}", g.wave_num + 1, sq.len()));
        } else {
            g.build_next_squad(data);
        }
        // FindSpawningVolume (G1: any volume, at random).
        g.last_volume = (!data.volumes.is_empty()).then(|| g.rand(data.volumes.len()));
    }
    let Some(v) = g.last_volume else {
        g.next_squad.clear();
        return;
    };
    // SpawnInHere: up to TotalMaxMonsters and MaxMonsters - NumMonsters.
    let room = (g.max_monsters - num_monsters).min(g.total_max_monsters).max(0) as usize;
    let mut spawned = 0usize;
    for k in 0..g.next_squad.len().min(room) {
        let Some(at) = spawn_spot(spatial, data, v, k) else {
            break;
        };
        let yaw = g.frand() * 65536.0;
        spawns.write(SpawnZedAt { class: g.next_squad[k].clone(), floor: at, yaw });
        spawned += 1;
    }
    if spawned == 0 {
        // TryToSpawnInAnotherVolume.
        runlog::kv("squad_failed", &format!("volume={}", data.volumes[v].name));
        g.last_volume = (!data.volumes.is_empty()).then(|| g.rand(data.volumes.len()));
        return;
    }
    let names: Vec<&str> = g.next_squad[..spawned].iter().map(|c| c.rsplit('.').next().unwrap_or(c).trim_end_matches("_STANDARD")).collect();
    runlog::kv(
        "squad_spawned",
        &format!("wave={} volume={} zeds=[{}] left_in_wave={}", g.wave_num + 1, data.volumes[v].name, names.join(" "), g.total_max_monsters - spawned as i32),
    );
    g.total_max_monsters -= spawned as i32;
    g.next_squad.drain(..spawned);
}

/// DoWaveEnd: WaveTimeElapsed reset only after the first wave, the
/// countdown to TimeBetweenWaves, WaveNum + 1. Door respawns (D5) and the
/// team's dosh (T1) come later.
fn do_wave_end(g: &mut WaveGame) {
    if g.wave_num < 1 {
        g.wave_time_elapsed = 0.0;
    }
    runlog::kv("wave_end", &format!("wave={}", if g.phase == Phase::BossWave { "boss".to_string() } else { (g.wave_num + 1).to_string() }));
    g.phase = Phase::Countdown;
    g.countdown = TIME_BETWEEN_WAVES;
    g.wave_num += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn letters() -> Vec<(String, String)> {
        [("A", "Clot"), ("B", "Crawler"), ("G", "Bloat")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn squads_parse_like_load_up_monster_list() {
        assert_eq!(parse_squad("4A1G", &letters()), vec!["Clot", "Clot", "Clot", "Clot", "Bloat"]);
        assert_eq!(parse_squad("2B", &letters()), vec!["Crawler", "Crawler"]);
    }

    #[test]
    fn spawn_time_runs_from_one_to_three_periods() {
        let data = GameData { spawn_period: 2.5, ..default() };
        let mut g = WaveGame::default();
        // sin(0) = 0: the longest wait, 3 x the period.
        assert!((g.next_squad_time(&data, GameLength::Short) - 7.5).abs() < 1e-4);
        // |sin| = 1 at elapsed pi / 2 / 0.04: just the period.
        g.wave_time_elapsed = std::f32::consts::FRAC_PI_2 / SINE_WAVE_FREQ;
        assert!((g.next_squad_time(&data, GameLength::Short) - 2.5).abs() < 1e-3);
    }

    #[test]
    fn masks_pick_squads_by_bit() {
        let data = GameData {
            waves: vec![WaveConfig { mask: 0b101, max_monsters: 20 }],
            squads: vec![vec!["A".into()], vec!["B".into()], vec!["C".into()]],
            ..default()
        };
        let g = WaveGame::default();
        assert_eq!(g.masked_squads(&data), vec![2, 0]);
    }
}
