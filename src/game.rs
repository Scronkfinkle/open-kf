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
use ue_assets::properties::{Value, read_export_properties, string_array, struct_array};

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
    pub volumes: Vec<crate::zvolume::ZombieVolume>,
    /// ZombieFlag, size and class names of every zed the waves use.
    pub zeds: std::collections::HashMap<String, crate::zvolume::ZedInfo>,
    /// Spawn points are built once the level's colliders exist.
    pub points_ready: bool,
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
        Some(Value::Array { count, raw }) => raw.as_chunks::<4>().0.iter().take(*count).map(|b| i32::from_le_bytes(*b)).collect(),
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
    // The map: KFLevelRules and the ZombieVolumes.
    let pkg = &map.pkg;
    for i in 0..pkg.exports.len() {
        if pkg.export_class_name(i) != "KFLevelRules" {
            continue;
        }
        if let Ok(props) = read_export_properties(pkg, i)
            && let Some(Value::Float(f)) = props.get(pkg, "WaveSpawnPeriod")
        {
            data.spawn_period = *f;
        }
    }
    let mut classes: Vec<String> = data.squads.iter().chain(data.special.iter()).flatten().cloned().collect();
    classes.push(data.boss_class.clone());
    classes.sort();
    classes.dedup();
    let (volumes, zeds) = crate::zvolume::load(set, defaults, map, &classes);
    data.volumes = volumes;
    data.zeds = zeds;
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
    /// LastSpawningVolume (rated x 0.2 next time).
    last_spawning_volume: Option<usize>,
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
            last_spawning_volume: None,
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

/// A zed for `zed.rs` to spawn: class path, its cylinder centre (Unreal
/// units), yaw (Unreal rotation units).
#[derive(Message, Clone, Debug)]
pub struct SpawnZedAt {
    pub class: String,
    pub centre: Vec3,
    pub yaw: f32,
}

/// Kill every living zed (a restart clears the map).
#[derive(Message, Clone, Copy, Debug)]
pub struct ClearZeds;

/// The stuck-zed cleanup kills this zed (`Pawn.KilledBy(self)`: no kill
/// credit, no dosh).
#[derive(Message, Clone, Copy, Debug)]
pub struct KillStuckZed(pub usize);

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
            .add_message::<KillStuckZed>()
            .add_systems(Update, wave_timer);
    }
}

type PlayerQuery<'w, 's> = Query<'w, 's, (&'static Transform, Option<&'static crate::walk::Walker>), With<crate::camera::FlyCamera>>;

#[allow(clippy::too_many_arguments)]
fn wave_timer(
    time: Res<Time>,
    options: Res<GameOptions>,
    data: Option<ResMut<GameData>>,
    mut game: ResMut<WaveGame>,
    mut hud: ResMut<WaveHud>,
    zeds: Query<&crate::zed::Zed>,
    health: Res<crate::combat::PlayerHealth>,
    spatial: SpatialQuery,
    frames: Res<bevy::diagnostic::FrameCount>,
    mut spawns: MessageWriter<SpawnZedAt>,
    script: Res<crate::weapon::ScriptedInput>,
    (keys, mut clear): (Res<ButtonInput<KeyCode>>, MessageWriter<ClearZeds>),
    (player, doors, mut kill_stuck): (PlayerQuery, Res<crate::door::Doors>, MessageWriter<KillStuckZed>),
) {
    if options.mode != GameMode::Waves || frames.0 < 10 {
        return;
    }
    // Test action "next_wave": the countdown ends at the next tick.
    let skip = script.0.iter().any(|(f, a)| *f == frames.0 && a == "next_wave");
    let Some(mut data) = data else { return };
    let now = time.elapsed_secs();
    let g = &mut *game;
    let data = &mut *data;
    // The player's cylinder centre, Unreal units.
    let Ok((cam, walker)) = player.single() else { return };
    let centre = walker.map_or(cam.translation - Vec3::Y * crate::combat::PLAYER_EYE_HEIGHT * SCALE, |w| w.center);
    let view = crate::zvolume::PlayerView {
        location: Vec3::new(-centre.z, centre.x, centre.y) / SCALE,
    };
    if !data.points_ready {
        data.points_ready = true;
        let mut empty = Vec::new();
        for v in data.volumes.iter_mut() {
            v.init_spawn_points(&spatial);
            if v.spawn_pos.is_empty() {
                empty.push(v.name.clone());
            }
        }
        runlog::kv(
            "zvolume_points",
            &format!(
                "volumes={} points={:?} without_points=[{}]",
                data.volumes.len(),
                data.volumes.iter().map(|v| v.spawn_pos.len()).collect::<Vec<_>>(),
                empty.join(" ")
            ),
        );
    }
    crate::zvolume::touch(&mut data.volumes, &view, now);
    let ctx = SpawnCtx {
        spatial: &spatial,
        player: view,
        doors: &doors,
        now,
    };
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
            // StartWaveBoss / AddBoss: a volume rated for the boss (G3 adds
            // the rest of the boss's own rules).
            if g.total_max_monsters > 0 {
                g.next_squad = vec![data.boss_class.clone()];
                if let Some(v) = find_volume(g, data, &ctx, true) {
                    let n = spawn_in_here(g, data, v, &ctx, num_monsters, &mut spawns);
                    if n > 0 {
                        g.last_volume = Some(v);
                        runlog::kv("boss_spawned", &format!("volume={}", data.volumes[v].name));
                    }
                }
            } else if num_monsters <= 0 {
                do_wave_end(g);
            }
        }
        Phase::Wave => {
            g.wave_time_elapsed += 1.0;
            if g.total_max_monsters <= 0 {
                // All spawned, 5 or fewer left: one zed a tick that
                // CanKillMeYet (unseen for 8 s; any zed from the final wave
                // on) is killed, so a stuck zed cannot stall the wave.
                if num_monsters <= 5
                    && let Some(z) = zeds.iter().find(|z| !z.is_dead() && (g.wave_num >= g.final_wave || z.unseen_for(now) > 8.0))
                {
                    kill_stuck.write(KillStuckZed(z.id));
                    runlog::kv("zed_cleanup", &format!("id={} unseen_seconds={:.0} left={num_monsters}", z.id, z.unseen_for(now).min(9999.0)));
                }
                if num_monsters <= 0 {
                    do_wave_end(g);
                }
            } else if now > g.next_monster_time && num_monsters + g.next_squad.len() as i32 <= g.max_monsters {
                add_squad(g, data, num_monsters, &ctx, &mut spawns);
                g.next_monster_time = if g.next_squad.is_empty() { now + g.next_squad_time(data, options.length) } else { now + 0.2 };
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
                    g.setup_wave(data);
                }
            }
        }
    }
}

/// What spawning needs to know about the world this tick.
struct SpawnCtx<'a, 'w, 's> {
    spatial: &'a SpatialQuery<'w, 's>,
    player: crate::zvolume::PlayerView,
    doors: &'a crate::door::Doors,
    now: f32,
}

/// FindSpawningVolume: the best-rated volume for the squad in
/// `next_squad` (refused volumes skipped).
fn find_volume(g: &mut WaveGame, data: &GameData, ctx: &SpawnCtx, boss: bool) -> Option<usize> {
    let door_state = |name: &str| {
        ctx.doors
            .doors
            .iter()
            .find(|d| d.info.name.eq_ignore_ascii_case(name))
            .map(|d| (d.sealed, d.key_num == 0))
    };
    let mut best: Option<(usize, f32)> = None;
    let mut refused: std::collections::BTreeMap<&str, usize> = Default::default();
    for (i, v) in data.volumes.iter().enumerate() {
        let frand = g.frand();
        match crate::zvolume::rate(ctx.spatial, v, g.last_spawning_volume == Some(i), boss, &g.next_squad, &data.zeds, &door_state, &ctx.player, ctx.now, frand) {
            Ok(score) => {
                if best.is_none_or(|(_, b)| score > b) {
                    best = Some((i, score));
                }
            }
            Err(why) => *refused.entry(why).or_default() += 1,
        }
    }
    if best.is_none() {
        runlog::kv("zvolume_none", &format!("refused={refused:?}"));
    }
    best.map(|(i, _)| i)
}

/// ZombieVolume.SpawnInHere: drop the squad's zeds this volume does not
/// allow (they are lost, as in KF: the squad array is edited in place), then
/// up to TotalMaxMonsters and MaxMonsters - NumMonsters zeds, each at one
/// of 3 random spawn points the player cannot see. Native Spawn's fit test:
/// the zed's own cylinder must not overlap the level (raised so a taller
/// zed stands where the 44-high tester stood). Returns how many spawned.
fn spawn_in_here(g: &mut WaveGame, data: &mut GameData, v: usize, ctx: &SpawnCtx, num_monsters: i32, spawns: &mut MessageWriter<SpawnZedAt>) -> usize {
    let vol = &data.volumes[v];
    let before = g.next_squad.len();
    g.next_squad.retain(|c| vol.allows(&data.zeds, c));
    if g.next_squad.len() < before {
        runlog::kv("squad_filtered", &format!("volume={} removed={}", vol.name, before - g.next_squad.len()));
    }
    // ZombieCountMulti (1 on every KF-Manor volume).
    let multi = vol.zombie_count_multi;
    if multi < 1.0 {
        let n = ((g.next_squad.len() as f32 * multi) as usize).max(1);
        g.next_squad.truncate(n);
    } else if multi > 1.0 {
        let f = g.frand();
        let n = ((g.next_squad.len() as f32 * (multi / 2.0 + multi * f)) as usize).max(g.next_squad.len());
        while g.next_squad.len() < n {
            let k = g.rand(g.next_squad.len());
            let c = g.next_squad[k].clone();
            g.next_squad.push(c);
        }
    }
    if g.next_squad.is_empty() {
        return 0;
    }
    let mut total = g.total_max_monsters;
    let mut at_once_left = g.max_monsters - num_monsters;
    let mut spawned = 0usize;
    let mut names = Vec::new();
    for i in 0..g.next_squad.len() {
        if total <= 0 || at_once_left <= 0 {
            continue;
        }
        let class = g.next_squad[i].clone();
        let info = data.zeds.get(&class.to_ascii_lowercase()).cloned().unwrap_or_default();
        let yaw = g.rand(65536) as f32;
        for _ in 0..3 {
            let k = g.rand(vol.spawn_pos.len());
            let p = vol.spawn_pos[k];
            if crate::zvolume::player_can_see_point(ctx.spatial, vol, p, &info, &ctx.player) {
                continue;
            }
            let centre = p + Vec3::Z * (info.half_height - 44.0).max(0.0);
            let shape = Collider::cylinder(info.radius * SCALE, 2.0 * info.half_height * SCALE);
            if !ctx
                .spatial
                .shape_intersections(&shape, coords::pos(centre.to_array()), Quat::IDENTITY, &crate::collision::zed_filter())
                .is_empty()
            {
                continue;
            }
            spawns.write(SpawnZedAt { class: class.clone(), centre, yaw });
            total -= 1;
            at_once_left -= 1;
            spawned += 1;
            names.push(class.rsplit('.').next().unwrap_or(&class).trim_end_matches("_STANDARD").to_string());
            break;
        }
    }
    g.total_max_monsters = total;
    let vol = &mut data.volumes[v];
    if spawned > 0 {
        vol.last_spawn_time = ctx.now;
        vol.last_failed_spawn_time = f32::MIN;
        let d = (vol.location - ctx.player.location).length();
        runlog::kv(
            "squad_spawned",
            &format!("wave={} volume={} distance={d:.0} zeds=[{}] left_in_wave={}", g.wave_num + 1, vol.name, names.join(" "), g.total_max_monsters),
        );
    } else {
        vol.last_failed_spawn_time = ctx.now;
    }
    spawned
}

/// AddSquad: the special squad on odd passes through the list (once per
/// pass), else the next squad; a volume for it; spawn as many as fit, the
/// rest next time (AddSquad removes the first `numspawned` entries, as in
/// KF, even if a later one was the one that fitted).
fn add_squad(g: &mut WaveGame, data: &mut GameData, num_monsters: i32, ctx: &SpawnCtx, spawns: &mut MessageWriter<SpawnZedAt>) {
    if g.last_volume.is_none() || g.next_squad.is_empty() {
        let special = data.special.get(g.wave_num).filter(|s| !s.is_empty()).cloned();
        if let Some(sq) = special
            && !g.used_special
            && g.special_counter % 2 == 1
        {
            runlog::kv("squad_special", &format!("wave={} zeds={}", g.wave_num + 1, sq.len()));
            g.next_squad = sq;
            g.used_special = true;
        } else {
            g.build_next_squad(data);
        }
        g.last_volume = find_volume(g, data, ctx, false);
        if g.last_volume.is_some() {
            g.last_spawning_volume = g.last_volume;
        }
    }
    let Some(v) = g.last_volume else {
        // No volume: the squad is dropped.
        runlog::kv("squad_dropped", &format!("zeds={}", g.next_squad.len()));
        g.next_squad.clear();
        return;
    };
    let spawned = spawn_in_here(g, data, v, ctx, num_monsters, spawns);
    if spawned > 0 {
        let n = spawned.min(g.next_squad.len());
        g.next_squad.drain(..n);
    } else {
        // TryToSpawnInAnotherVolume.
        runlog::kv("squad_failed", &format!("volume={}", data.volumes[v].name));
        g.last_volume = find_volume(g, data, ctx, false);
        if g.last_volume.is_some() {
            g.last_spawning_volume = g.last_volume;
        }
    }
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
