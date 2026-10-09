//! The game loop (`--mode waves`): KFGameType's wave state machine. `--mode
//! debug` (the default) leaves everything as before: no waves, zeds only
//! from the spawn keys and test flags.
//!
//! Rules from KFMod.KFGameType state MatchInProgress (Timer once a
//! second), SetupWave, BuildNextSquad, AddSquad, CalcNextSquadSpawnTime and
//! DoWaveEnd; squads and waves from the KFGameType / KFMonstersCollection
//! class defaults (see DESIGN.md, "Game loop"). One player; the
//! difficulty's rules from difficulty.rs.

use std::rc::Rc;

use avian3d::prelude::*;
use bevy::prelude::*;
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package_set::{LoadedPackage, PackageSet};
use ue_assets::properties::{Value, read_export_properties, string_array, struct_array};

use crate::engine::coords::{self, SCALE};
use crate::engine::runlog;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GameMode {
    /// Today's test setup: no waves.
    #[default]
    Debug,
    /// KF's waves.
    Waves,
}

impl GameMode {
    /// `waves` / `debug`, any case (the host-info query sends `Waves`).
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "waves" => Some(Self::Waves),
            "debug" => Some(Self::Debug),
            _ => None,
        }
    }
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
    /// `--difficulty` (also difficulty::current(), set at startup).
    pub difficulty: crate::game::difficulty::Difficulty,
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
    /// KFMonstersCollection FinalSquads: the Patriarch's helper squads.
    pub final_squads: Vec<Vec<String>>,
    pub boss_class: String,
    /// KFLevelRules.WaveSpawnPeriod (the map's, else the class default 2).
    pub spawn_period: f32,
    pub volumes: Vec<crate::world::zvolume::ZombieVolume>,
    /// ZombieFlag, size and class names of every zed the waves use.
    pub zeds: std::collections::HashMap<String, crate::world::zvolume::ZedInfo>,
    /// Spawn points are built once the level's colliders exist.
    pub points_ready: bool,
}

/// KFGameType defaults (TimeBetweenWaves: difficulty.rs).
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
    // SpecialSquad lists (ZedClass, NumZeds), as the squad's zed classes.
    let special_squads = |prop: &str| -> Vec<Vec<String>> {
        let Some((v, from)) = defaults.get(&coll, prop) else {
            return Vec::new();
        };
        struct_array(&from.pkg, &v)
            .iter()
            .map(|p| {
                let classes = p.get(&from.pkg, "ZedClass").map(string_array).unwrap_or_default();
                let counts = int_array(p.get(&from.pkg, "NumZeds"));
                let mut squad = Vec::new();
                for (c, n) in classes.iter().zip(counts.iter()) {
                    if !c.is_empty() {
                        squad.extend(std::iter::repeat_n(c.clone(), (*n).max(0) as usize));
                    }
                }
                squad
            })
            .collect()
    };
    data.special = special_squads(special_prop);
    data.final_squads = special_squads("FinalSquads");
    data.boss_class = match defaults.get(&coll, "EndGameBossClass") {
        Some((Value::Str(s), _)) => s,
        _ => "KFChar.ZombieBoss_STANDARD".into(),
    };
    // The map: KFLevelRules and the ZombieVolumes.
    let pkg = &map.pkg;
    for i in pkg.level_actor_exports() {
        if pkg.export_class_name(i) != "KFLevelRules" {
            continue;
        }
        if let Ok(props) = read_export_properties(pkg, i)
            && let Some(Value::Float(f)) = props.get(pkg, "WaveSpawnPeriod")
        {
            data.spawn_period = *f;
        }
    }
    let mut classes: Vec<String> = data.squads.iter().chain(data.special.iter()).chain(data.final_squads.iter()).flatten().cloned().collect();
    classes.push(data.boss_class.clone());
    classes.sort();
    classes.dedup();
    let (volumes, zeds) = crate::world::zvolume::load(set, defaults, map, &classes);
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
    let short = |c: &String| c.rsplit('.').next().unwrap_or(c).trim_end_matches("_STANDARD").to_string();
    runlog::kv(
        "game_final_squads",
        &format!("{:?}", data.final_squads.iter().map(|s| s.iter().map(short).collect::<Vec<_>>().join(" ")).collect::<Vec<_>>()),
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

/// The wave timer's sound messages (music.rs, trader_voice.rs) and the
/// kill count they need.
#[derive(bevy::ecs::system::SystemParam)]
pub struct WaveSounds<'w> {
    music: MessageWriter<'w, crate::audio::music::MusicCue>,
    speech: MessageWriter<'w, crate::audio::trader_voice::TraderSpeech>,
    kills: Res<'w, crate::game::combat::KillCount>,
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
    /// FinalSquadNum: the next helper squad (one per syringe).
    final_squad_num: usize,
    /// WaveEndTime: the boss wave stops trying to spawn him after this.
    wave_end_time: f32,
    /// DoBossDeath has run.
    boss_killed: bool,
    /// bHasSetViewYet (MakeGrandEntry asked) and MatchOver's
    /// bBossHasSaidWord (BossLaughtIt done).
    has_set_view: bool,
    boss_laughed: bool,
    /// The boss's finished knockdowns already answered.
    boss_knockdowns_seen: u32,
    /// KFGameType MusicPlaying and CalmMusicPlaying (music.rs).
    music_playing: bool,
    calm_music_playing: bool,
    /// bDidTraderMovingMessage, bDidMoveTowardTraderMessage, and the kill
    /// count when the wave started (for ZombiesKilled).
    did_moving_message: bool,
    did_almost_open_message: bool,
    kills_at_wave_start: u32,
    /// Counters other systems watch (dosh.rs): games restarted, DoWaveEnd
    /// calls. Kept across a restart.
    pub restarts: u32,
    pub waves_ended: u32,
    /// What the HUD's wave circle shows, as last logged (`log_hud`).
    last_hud: Option<(Phase, usize, usize, i32)>,
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
            final_squad_num: 0,
            wave_end_time: 0.0,
            boss_killed: false,
            has_set_view: false,
            boss_laughed: false,
            boss_knockdowns_seen: 0,
            restarts: 0,
            music_playing: false,
            calm_music_playing: false,
            did_moving_message: false,
            did_almost_open_message: false,
            kills_at_wave_start: 0,
            waves_ended: 0,
            last_hud: None,
        }
    }
}

impl WaveGame {
    /// GameInfo.RestartGame's fresh state: everything back to the start,
    /// `restarts` counted up (dosh.rs and the pickups start again when it
    /// changes), `waves_ended` kept. Used by the restart after a match and
    /// by a map change.
    fn for_new_match(&self) -> WaveGame {
        WaveGame {
            restarts: self.restarts + 1,
            waves_ended: self.waves_ended,
            ..default()
        }
    }

    /// One log line whenever the HUD's wave circle changes (phase, wave,
    /// zeds left, countdown), to compare a host's and a client's HUD.
    fn log_hud(&mut self) {
        let now = (self.phase, self.wave_num, self.total_max_monsters.max(0) as usize + self.living, self.countdown);
        if self.last_hud != Some(now) {
            self.last_hud = Some(now);
            runlog::kv("wave_hud", &format!("phase={:?} wave={}/{} zeds_left={} countdown={}", now.0, now.1 + 1, self.final_wave, now.2, now.3));
        }
    }

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

    /// SetupWave: zeds this wave (`wave_total`), at most MaxZombiesOnce
    /// alive, the squad list, the first squad.
    fn setup_wave(&mut self, data: &GameData) {
        let max = data.waves.get(self.wave_num).map_or(5, |w| w.max_monsters);
        let difficulty = crate::game::difficulty::current();
        self.total_max_monsters = wave_total(max, difficulty);
        self.max_monsters = self.total_max_monsters.clamp(5, MAX_ZOMBIES_ONCE);
        self.squads_to_use = self.masked_squads(data);
        self.used_special = false;
        self.special_counter = 1;
        self.build_next_squad(data);
        runlog::kv(
            "wave_start",
            &format!(
                "wave={} of={} wave_max_monsters={max} difficulty={} difficulty_mod={} zeds={} max_at_once={} squads={:?}",
                self.wave_num + 1,
                self.final_wave,
                difficulty.word(),
                difficulty.wave_size_scale(),
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

    /// CalcNextSquadSpawnTime for one player (x 0.85 at Hard and up).
    fn next_squad_time(&self, data: &GameData, length: GameLength) -> f32 {
        self.next_squad_time_at(data, length, crate::game::difficulty::current())
    }

    fn next_squad_time_at(&self, data: &GameData, length: GameLength, difficulty: crate::game::difficulty::Difficulty) -> f32 {
        let sine = 1.0 - (self.wave_time_elapsed * SINE_WAVE_FREQ).sin().abs();
        let late = match length {
            GameLength::Short => self.wave_num >= 2,
            GameLength::Normal => self.wave_num >= 4,
            GameLength::Long => self.wave_num >= 7,
        };
        let t = data.spawn_period * if late { 1.1 } else { 1.0 } * difficulty.squad_time_scale();
        t + sine * t * 2.0
    }
}

/// KFGameType.SetupWave: TotalMaxMonsters = Clamp(WaveMaxMonsters x
/// DifficultyMod x NumPlayersMod (1: one player), 5, 800); Clamp takes
/// ints, so the product is cut to a whole number.
pub fn wave_total(wave_max_monsters: i32, difficulty: crate::game::difficulty::Difficulty) -> i32 {
    ((wave_max_monsters as f32 * difficulty.wave_size_scale()) as i32).clamp(5, 800)
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
/// DoBossDeath: every other zed's controller goes away (zed.rs).
#[derive(Message, Clone, Copy, Debug)]
pub struct BossDied;

#[derive(Message, Clone, Copy, Debug)]
pub struct KillStuckZed(pub usize);

/// The wave line on the HUD (empty in debug mode).
#[derive(Resource, Default)]
pub struct WaveHud(pub String);

/// Multiplayer: the wave state the host shares with the clients (KF's
/// KFGameReplicationInfo: WaveNumber, FinalWave, MaxMonsters,
/// TimeToNextWave, bWaveInProgress, CurrentShop, EndGameType).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Default)]
pub struct WaveShare {
    /// The host's wave game has started (its `game_start`).
    pub started: bool,
    /// `Phase` as a number (Countdown 0, Wave 1, BossWave 2, Won 3, Lost 4).
    pub phase: u8,
    pub wave_num: u16,
    pub final_wave: u16,
    pub countdown: i32,
    pub total_max_monsters: i32,
    pub living: u16,
    pub waves_ended: u32,
    pub restarts: u32,
    /// Shops: the current one, and whether the trader is open.
    pub shop: Option<u8>,
    pub doors_open: bool,
}

const PHASES: [Phase; 5] = [Phase::Countdown, Phase::Wave, Phase::BossWave, Phase::Won, Phase::Lost];

impl WaveGame {
    /// Host: the shared part.
    pub fn share(&self, shops: &crate::game::trader::Shops) -> WaveShare {
        WaveShare {
            started: self.deaths_at_start.is_some(),
            phase: PHASES.iter().position(|p| *p == self.phase).unwrap_or(0) as u8,
            wave_num: self.wave_num as u16,
            final_wave: self.final_wave as u16,
            countdown: self.countdown,
            total_max_monsters: self.total_max_monsters,
            living: self.living as u16,
            waves_ended: self.waves_ended,
            restarts: self.restarts,
            shop: shops.current.map(|c| c as u8),
            doors_open: shops.doors_open,
        }
    }
}

/// Multiplayer client: the host's wave state, as last received (net/).
#[derive(Resource, Default)]
pub struct RemoteWave(pub Option<WaveShare>);

pub struct GamePlugin;

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WaveGame>()
            .init_resource::<WaveHud>()
            .init_resource::<RemoteWave>()
            .add_message::<SpawnZedAt>()
            .add_message::<ClearZeds>()
            .add_message::<KillStuckZed>()
            .add_message::<BossDied>()
            // Per map (world/map_change.rs): the map loader inserts the
            // map's GameData in waves mode; a new map starts a new match.
            .add_systems(crate::world::map_change::MapUnload, |mut commands: Commands| commands.remove_resource::<GameData>())
            .add_systems(crate::world::map_change::PostMapLoad, new_match_on_new_map)
            .add_systems(Update, wave_timer);
    }
}

/// A map change (not the first map): a new match on the new map, as
/// RestartGame does (the zeds went with the old map). The wave timer starts
/// it as at the start of a run (`game_start`, `--wave` honoured).
fn new_match_on_new_map(
    epoch: Res<crate::world::map_change::MapEpoch>,
    options: Res<GameOptions>,
    mut game: ResMut<WaveGame>,
    mut hud: ResMut<WaveHud>,
    mut view_target: ResMut<crate::engine::view_target::ViewTarget>,
) {
    if epoch.first() || options.mode != GameMode::Waves {
        return;
    }
    let before = (game.phase, game.wave_num);
    *game = game.for_new_match();
    hud.0.clear();
    view_target.set(None, "map_change");
    runlog::kv("wave_game_reset", &format!("reason=map_change load={} was_phase={:?} was_wave={} restarts={}", epoch.load, before.0, before.1 + 1, game.restarts));
}

type PlayerQuery<'w, 's> = Query<'w, 's, (&'static Transform, Option<&'static crate::player::walk::Walker>), With<crate::engine::camera::FlyCamera>>;

#[allow(clippy::too_many_arguments)]
pub fn wave_timer(
    time: Res<Time>,
    options: Res<GameOptions>,
    data: Option<ResMut<GameData>>,
    mut game: ResMut<WaveGame>,
    mut hud: ResMut<WaveHud>,
    zeds: Query<&crate::zeds::zed::Zed>,
    health: Res<crate::game::combat::PlayerHealth>,
    spatial: SpatialQuery,
    frames: Res<bevy::diagnostic::FrameCount>,
    mut spawns: MessageWriter<SpawnZedAt>,
    script: Res<crate::weapons::weapon::ScriptedInput>,
    (keys, mut clear, mut hud_messages, mut talk): (Res<ButtonInput<KeyCode>>, MessageWriter<ClearZeds>, MessageWriter<crate::game::hud::LocalMessage>, WaveSounds),
    (player, mut doors, mut kill_stuck, player_zone): (PlayerQuery, ResMut<crate::world::door::Doors>, MessageWriter<KillStuckZed>, Res<crate::world::zones::PlayerZone>),
    (mut boss_died, mut respawn_doors, mut view_target, mut boss_actions): (
        MessageWriter<BossDied>,
        MessageWriter<crate::world::door::RespawnDoors>,
        ResMut<crate::engine::view_target::ViewTarget>,
        MessageWriter<crate::zeds::zed::BossAction>,
    ),
    (mut shops, menus): (ResMut<crate::game::trader::Shops>, Res<crate::game::menus::MenuState>),
    (remote_wave, net_mode, remote_players): (Res<RemoteWave>, Option<Res<crate::net::NetMode>>, Res<crate::game::combat::RemotePlayers>),
) {
    // The lobby: KF only enters MatchInProgress once everyone is ready.
    if options.mode != GameMode::Waves || frames.0 < 10 || menus.lobby_open() {
        return;
    }
    // A network client runs no waves of its own: it follows the host's.
    if net_mode.as_ref().is_some_and(|m| matches!(**m, crate::net::NetMode::Client { .. })) {
        if let Some(w) = remote_wave.0.as_ref() {
            let now = time.elapsed_secs();
            let location = player.single().ok().map(|(cam, walker)| {
                let c = walker.map_or(cam.translation - Vec3::Y * crate::game::combat::PLAYER_EYE_HEIGHT * SCALE, |w| w.center);
                Vec3::new(-c.z, c.x, c.y) / SCALE
            });
            follow_remote_wave(&mut game, w, now, health.deaths, &mut hud, &mut shops, &mut doors, &mut talk, &mut hud_messages, location);
        }
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
    let centre = walker.map_or(cam.translation - Vec3::Y * crate::game::combat::PLAYER_EYE_HEIGHT * SCALE, |w| w.center);
    let view = crate::world::zvolume::PlayerView {
        location: Vec3::new(-centre.z, centre.x, centre.y) / SCALE,
        fog_end: player_zone.fog_end,
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
    crate::world::zvolume::touch(&mut data.volumes, &view, now);
    let ctx = SpawnCtx {
        spatial: &spatial,
        player: view,
        doors: &doors,
        now,
    };
    // After a win or a loss, Enter / test action "restart_game" (ours, a
    // shortcut for testing): the same map again, at once. KF's own restart
    // (end_game.rs: Fire after 5 s, or the MatchOver timer) goes to the
    // next map instead (game/travel.rs).
    let restart = keys.just_pressed(KeyCode::Enter) || script.0.iter().any(|(f, a)| *f == frames.0 && a == "restart_game");
    if restart && matches!(g.phase, Phase::Won | Phase::Lost) {
        *g = WaveGame {
            deaths_at_start: Some(health.deaths),
            final_wave: data.waves.len(),
            next_tick: now + 1.0,
            ..g.for_new_match()
        };
        clear.write(ClearZeds);
        view_target.set(None, "restart");
        runlog::kv("game_start", &format!("mode=waves length={:?} difficulty={:?} final_wave={} countdown={} restart=true", options.length, options.difficulty, g.final_wave, g.countdown));
    }
    if skip && g.phase == Phase::Countdown {
        g.countdown = 1;
        runlog::kv("wave_countdown", "skipped=true");
    }
    // Test action "end_wave" (ours): the running wave ends now (no more
    // spawns, every zed dies), so the trader opens at the next ticks.
    if g.phase == Phase::Wave && script.0.iter().any(|(f, a)| *f == frames.0 && a == "end_wave") {
        g.total_max_monsters = 0;
        g.next_squad.clear();
        clear.write(ClearZeds);
        runlog::kv("wave_end_test", &format!("wave={} living={}", g.wave_num + 1, g.living));
    }
    if g.deaths_at_start.is_none() {
        g.deaths_at_start = Some(health.deaths);
        g.final_wave = data.waves.len();
        g.next_tick = now + 1.0;
        if let Some(w) = options.start_wave {
            g.wave_num = w.saturating_sub(1).min(g.final_wave);
        }
        runlog::kv("game_start", &format!("mode=waves length={:?} difficulty={:?} final_wave={} countdown={}", options.length, options.difficulty, g.final_wave, g.countdown));
    }
    // UpdateMonsterCount counts pawns that still have a controller.
    g.living = zeds.iter().filter(|z| !z.is_dead() && !z.braindead).count();
    if g.phase == Phase::BossWave {
        boss_rules(g, data, &zeds, &ctx, &mut spawns, &mut boss_died);
    }
    // ZombieBoss.Died: the view goes to him at once (behind view).
    if g.boss_killed
        && view_target.reason() != "boss_death"
        && let Some(z) = zeds.iter().find(|z| z.boss_knockdowns().is_some() && z.is_dead())
    {
        view_target.set(Some(z.id), "boss_death");
    }
    // UpdateMonsterCount: no living player ends the game (solo: no respawn;
    // a network host also waits for the other players' pawns to be dead).
    let others_alive = remote_players.0.iter().filter(|p| p.alive).count();
    // Network: players come back at a wave end (net/starts.rs), so it is
    // whether the host's player is dead now (and died in this game: on a
    // restart it is revived just after this, end_game.rs).
    let died = health.deaths > g.deaths_at_start.unwrap_or(0);
    let host_dead = if net_mode.as_ref().is_some_and(|m| m.active()) { died && health.dead } else { died };
    if matches!(g.phase, Phase::Countdown | Phase::Wave | Phase::BossWave) && host_dead && others_alive == 0 {
        g.phase = Phase::Lost;
        runlog::kv("game_end", &format!("result=lost wave={}", g.wave_num + 1));
    }
    g.log_hud();
    hud.0 = match g.phase {
        Phase::Countdown => format!("WAVE {}/{}  NEXT IN {}", g.wave_num + 1, g.final_wave, g.countdown.max(0)),
        Phase::Wave => format!("WAVE {}/{}  ZEDS {}", g.wave_num + 1, g.final_wave, g.total_max_monsters.max(0) as usize + g.living),
        Phase::BossWave => "PATRIARCH".into(),
        Phase::Won => "YOU WON - ENTER TO PLAY AGAIN".into(),
        Phase::Lost => "YOU DIED - ENTER TO PLAY AGAIN".into(),
    };
    if let Some(t) = crate::game::trader::distance_text(&shops, ctx.player.location) {
        hud.0 += &format!("  {t}");
    }
    if now < g.next_tick {
        return;
    }
    // MatchInProgress.Timer, once a second.
    g.next_tick += 1.0;
    let num_monsters = g.living as i32;
    boss_view_tick(g, &zeds, &mut view_target, &mut boss_actions, &mut talk.music);
    // Shop calls are made after the match (`ctx` borrows the doors).
    let mut shop_action = ShopAction::None;
    match g.phase {
        Phase::Won | Phase::Lost => {}
        Phase::BossWave => {
            shop_action = ShopAction::CloseAndBoot;
            if g.total_max_monsters <= 0 || now > g.wave_end_time {
                // Everyone spawned and all dead (or he never found a
                // volume in 60 s: the wave ends without him).
                if num_monsters <= 0 {
                    do_wave_end(g, &mut respawn_doors, &mut hud_messages);
                }
            } else {
                add_boss(g, data, &ctx, &mut spawns);
            }
        }
        Phase::Wave => {
            shop_action = ShopAction::CloseAndBoot;
            g.wave_time_elapsed += 1.0;
            // KFGameType.Killed: the trader's "shop moving" line once 20% of
            // the wave is dead, "almost open" at 80% (solo: only farther
            // than 30 m from the shop); not before the last waves. KF checks
            // at each kill; ours once a second.
            let killed = talk.kills.0.saturating_sub(g.kills_at_wave_start) as f32;
            let share = killed / (killed + (g.total_max_monsters.max(0) as usize + g.living) as f32).max(1.0);
            if g.wave_num < g.final_wave && killed > 0.0 {
                if !g.did_moving_message && share >= 0.2 {
                    g.did_moving_message = true;
                    talk.speech.write(crate::audio::trader_voice::TraderSpeech(0));
                } else if g.did_moving_message && !g.did_almost_open_message && share >= 0.8 {
                    g.did_almost_open_message = true;
                    let far = shops.current.and_then(|i| shops.shops.get(i)).is_none_or(|s| (s.location - ctx.player.location).length_squared() > 2_250_000.0);
                    if far {
                        talk.speech.write(crate::audio::trader_voice::TraderSpeech(1));
                    }
                }
            }
            // if( !MusicPlaying ) StartGameMusic(True).
            if !g.music_playing {
                g.music_playing = true;
                g.calm_music_playing = false;
                talk.music.write(crate::audio::music::MusicCue::Combat(g.wave_num));
            }
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
                    do_wave_end(g, &mut respawn_doors, &mut hud_messages);
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
            // WaveCountDown == 30 / 10: the trader's 30 s and 10 s lines.
            if g.countdown == 30 {
                talk.speech.write(crate::audio::trader_voice::TraderSpeech(4));
            } else if g.countdown == 10 {
                talk.speech.write(crate::audio::trader_voice::TraderSpeech(5));
            }
            // if ( !CalmMusicPlaying ) StartGameMusic(False).
            if !g.calm_music_playing {
                g.calm_music_playing = true;
                g.music_playing = false;
                talk.music.write(crate::audio::music::MusicCue::Calm(g.wave_num));
            }
            // WaitingMessage at 4..1 seconds left (after WaveCountDown--):
            // FINAL WAVE INBOUND before the boss, else NEXT WAVE INBOUND!
            if g.countdown > 0 && g.countdown < 5 {
                let switch = if g.wave_num == g.final_wave { 3 } else { 1 };
                hud_messages.write(crate::game::hud::LocalMessage::new(crate::game::hud::MessageClass::Waiting, switch));
            }
            // Open the trader (not before the first wave); pick a shop if
            // none is picked yet.
            shop_action = if g.wave_num != 0 && !shops.doors_open { ShopAction::Open } else { ShopAction::Select };
            if g.countdown % 10 == 0 && g.countdown > 0 {
                runlog::kv("wave_countdown", &format!("wave={} seconds={}", g.wave_num + 1, g.countdown));
            }
            if g.countdown <= 0 {
                if g.wave_num == g.final_wave {
                    // StartWaveBoss: TotalMaxMonsters 1, MaxMonsters 1, 60 s.
                    g.phase = Phase::BossWave;
                    g.total_max_monsters = 1;
                    g.max_monsters = 1;
                    g.wave_end_time = now + 60.0;
                    runlog::kv("wave_start", &format!("wave=boss class={}", data.boss_class));
                } else {
                    g.phase = Phase::Wave;
                    g.setup_wave(data);
                    g.kills_at_wave_start = talk.kills.0;
                    g.did_moving_message = false;
                    g.did_almost_open_message = false;
                }
            }
        }
    }
    match shop_action {
        ShopAction::None => {}
        ShopAction::Select => {
            if shops.current.is_none() {
                shops.select_shop();
            }
        }
        ShopAction::Open => {
            shops.open_shops(&mut doors);
            // OpenShops: "the shop is open" (or "last wave" before the boss).
            talk.speech.write(crate::audio::trader_voice::TraderSpeech(if g.wave_num < g.final_wave { 2 } else { 3 }));
        }
        ShopAction::CloseAndBoot => {
            if shops.doors_open {
                shops.close_shops(&mut doors);
                // CloseShops: "the shop is closed" if WaveNum < FinalWave - 1.
                if g.wave_num + 1 < g.final_wave {
                    talk.speech.write(crate::audio::trader_voice::TraderSpeech(6));
                }
            }
            // BootShopPlayers (trader.rs moves the player).
            shops.boot_requested = true;
        }
    }
}

/// Multiplayer client: copy the host's wave state (`WaveShare`) into this
/// game's `WaveGame`, so the HUD, trader, music, dosh and end screen
/// follow it; play what the host's wave timer would play here (music,
/// trader lines, the "wave inbound" messages); open and close the shops
/// as the host does. `location`: this player's pawn, Unreal units.
#[allow(clippy::too_many_arguments)]
fn follow_remote_wave(
    g: &mut WaveGame,
    w: &WaveShare,
    now: f32,
    deaths: u32,
    hud: &mut WaveHud,
    shops: &mut crate::game::trader::Shops,
    doors: &mut crate::world::door::Doors,
    talk: &mut WaveSounds,
    hud_messages: &mut MessageWriter<crate::game::hud::LocalMessage>,
    location: Option<Vec3>,
) {
    if !w.started {
        hud.0 = "WAITING FOR THE HOST".into();
        return;
    }
    if g.deaths_at_start.is_none() {
        g.deaths_at_start = Some(deaths);
        g.next_tick = now + 1.0;
        runlog::kv("game_start", &format!("mode=waves follow_host=true final_wave={} wave={} phase={:?}", w.final_wave, w.wave_num + 1, PHASES[w.phase as usize % 5]));
    }
    let (old_phase, old_countdown, old_wave) = (g.phase, g.countdown, g.wave_num);
    g.phase = PHASES[w.phase as usize % 5];
    g.wave_num = w.wave_num as usize;
    g.final_wave = w.final_wave as usize;
    g.countdown = w.countdown;
    g.total_max_monsters = w.total_max_monsters;
    g.living = w.living as usize;
    g.waves_ended = w.waves_ended;
    g.restarts = w.restarts;
    if old_phase != g.phase || old_wave != g.wave_num {
        runlog::kv("wave_follow", &format!("phase={:?} wave={} of={} zeds={} countdown={}", g.phase, g.wave_num + 1, g.final_wave, g.total_max_monsters.max(0) as usize + g.living, g.countdown));
        match g.phase {
            Phase::Wave | Phase::BossWave if !g.music_playing => {
                g.music_playing = true;
                g.calm_music_playing = false;
                talk.music.write(crate::audio::music::MusicCue::Combat(g.wave_num));
            }
            Phase::Countdown if !g.calm_music_playing => {
                g.calm_music_playing = true;
                g.music_playing = false;
                talk.music.write(crate::audio::music::MusicCue::Calm(g.wave_num));
            }
            _ => {}
        }
    }
    if g.phase == Phase::Countdown && g.countdown != old_countdown {
        if g.countdown == 30 {
            talk.speech.write(crate::audio::trader_voice::TraderSpeech(4));
        } else if g.countdown == 10 {
            talk.speech.write(crate::audio::trader_voice::TraderSpeech(5));
        }
        if g.countdown > 0 && g.countdown < 5 {
            let switch = if g.wave_num == g.final_wave { 3 } else { 1 };
            hud_messages.write(crate::game::hud::LocalMessage::new(crate::game::hud::MessageClass::Waiting, switch));
        }
        if g.countdown % 10 == 0 && g.countdown > 0 {
            runlog::kv("wave_countdown", &format!("wave={} seconds={} follow_host=true", g.wave_num + 1, g.countdown));
        }
    }
    // The shops as the host has them.
    let host_shop = w.shop.map(|s| s as usize).filter(|s| *s < shops.shops.len());
    if w.doors_open && !shops.doors_open {
        shops.current = host_shop;
        shops.open_shops(doors);
        talk.speech.write(crate::audio::trader_voice::TraderSpeech(if g.wave_num < g.final_wave { 2 } else { 3 }));
    } else if !w.doors_open && shops.doors_open {
        shops.close_shops(doors);
        if g.wave_num + 1 < g.final_wave {
            talk.speech.write(crate::audio::trader_voice::TraderSpeech(6));
        }
    }
    if shops.current != host_shop && host_shop.is_some() {
        shops.current = host_shop;
    }
    // BootShopPlayers once a second during a wave, as the host's timer.
    if now >= g.next_tick {
        g.next_tick = now + 1.0;
        if matches!(g.phase, Phase::Wave | Phase::BossWave) {
            shops.boot_requested = true;
        }
    }
    g.log_hud();
    hud.0 = match g.phase {
        Phase::Countdown => format!("WAVE {}/{}  NEXT IN {}", g.wave_num + 1, g.final_wave, g.countdown.max(0)),
        Phase::Wave => format!("WAVE {}/{}  ZEDS {}", g.wave_num + 1, g.final_wave, g.total_max_monsters.max(0) as usize + g.living),
        Phase::BossWave => "PATRIARCH".into(),
        Phase::Won => "YOU WON".into(),
        Phase::Lost => "YOUR SQUAD WAS WIPED OUT".into(),
    };
    if let Some(at) = location
        && let Some(t) = crate::game::trader::distance_text(shops, at)
    {
        hud.0 += &format!("  {t}");
    }
}

/// What the wave timer asks of the shops this tick.
enum ShopAction {
    None,
    /// Between waves before the trader opens: pick a shop if none.
    Select,
    /// OpenShops (then, as KF, a shop is picked if none).
    Open,
    /// During a wave: CloseShops if open, then BootShopPlayers.
    CloseAndBoot,
}

/// What spawning needs to know about the world this tick.
struct SpawnCtx<'a, 'w, 's> {
    spatial: &'a SpatialQuery<'w, 's>,
    player: crate::world::zvolume::PlayerView,
    doors: &'a crate::world::door::Doors,
    now: f32,
}

/// FindSpawningVolume: the best-rated volume for the squad in
/// `next_squad` (refused volumes skipped).
fn find_volume(g: &mut WaveGame, data: &GameData, ctx: &SpawnCtx, ignore_failed: bool, boss: bool) -> Option<usize> {
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
        match crate::world::zvolume::rate(ctx.spatial, v, g.last_spawning_volume == Some(i), ignore_failed, boss, &g.next_squad, &data.zeds, &door_state, &ctx.player, ctx.now, frand) {
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
/// up to `limits.total` (TotalMaxMonsters, lowered by each spawn) and
/// `limits.at_once` zeds, each at one of 3 random spawn points the player
/// cannot see (every point with bTryAllSpawns). Native Spawn's fit test:
/// the zed's own cylinder must not overlap the level (raised so a taller
/// zed stands where the 44-high tester stood). Returns how many spawned.
fn spawn_in_here(g: &mut WaveGame, data: &mut GameData, v: usize, ctx: &SpawnCtx, limits: &mut Limits, spawns: &mut MessageWriter<SpawnZedAt>) -> usize {
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
    let mut at_once_left = limits.at_once;
    let tries = if limits.try_all { vol.spawn_pos.len() } else { 3 };
    let mut spawned = 0usize;
    let mut names = Vec::new();
    for i in 0..g.next_squad.len() {
        if limits.total <= 0 || at_once_left <= 0 {
            continue;
        }
        let class = g.next_squad[i].clone();
        let info = data.zeds.get(&class.to_ascii_lowercase()).cloned().unwrap_or_default();
        let yaw = g.rand(65536) as f32;
        for _ in 0..tries {
            let k = g.rand(vol.spawn_pos.len());
            let p = vol.spawn_pos[k];
            if crate::world::zvolume::player_can_see_point(ctx.spatial, vol, p, &info, &ctx.player) {
                continue;
            }
            let centre = p + Vec3::Z * (info.half_height - 44.0).max(0.0);
            let shape = Collider::cylinder(info.radius * SCALE, 2.0 * info.half_height * SCALE);
            if !ctx
                .spatial
                .shape_intersections(&shape, coords::pos(centre.to_array()), Quat::IDENTITY, &crate::world::collision::zed_filter())
                .is_empty()
            {
                continue;
            }
            spawns.write(SpawnZedAt { class: class.clone(), centre, yaw });
            limits.total -= 1;
            at_once_left -= 1;
            spawned += 1;
            names.push(class.rsplit('.').next().unwrap_or(&class).trim_end_matches("_STANDARD").to_string());
            break;
        }
    }
    let vol = &mut data.volumes[v];
    if spawned > 0 {
        vol.last_spawn_time = ctx.now;
        vol.last_failed_spawn_time = f32::MIN;
        let d = (vol.location - ctx.player.location).length();
        runlog::kv(
            "squad_spawned",
            &format!("wave={} volume={} distance={d:.0} zeds=[{}] left_in_wave={}", g.wave_num + 1, vol.name, names.join(" "), limits.total),
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
        g.last_volume = find_volume(g, data, ctx, false, false);
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
    let mut limits = Limits {
        total: g.total_max_monsters,
        at_once: g.max_monsters - num_monsters,
        try_all: false,
    };
    let spawned = spawn_in_here(g, data, v, ctx, &mut limits, spawns);
    g.total_max_monsters = limits.total;
    // The new zeds exist from the next frame on; count them now so "zeds
    // to come + alive" (the HUD, and what clients are sent) does not dip
    // for a frame.
    g.living += spawned;
    if spawned > 0 {
        let n = spawned.min(g.next_squad.len());
        g.next_squad.drain(..n);
    } else {
        // TryToSpawnInAnotherVolume.
        runlog::kv("squad_failed", &format!("volume={}", data.volumes[v].name));
        g.last_volume = find_volume(g, data, ctx, false, false);
        if g.last_volume.is_some() {
            g.last_spawning_volume = g.last_volume;
        }
    }
}

/// SpawnInHere's limits: TotalMaxMonsters (lowered as zeds spawn),
/// MaxMonstersAtOnceLeft, bTryAllSpawns.
struct Limits {
    total: i32,
    at_once: i32,
    try_all: bool,
}

/// AddBoss: FinalSquadNum back to 0; the boss in LastZVol, else the best
/// boss-rated volume, else the same ignoring the 5 s failed-spawn wait.
/// SpawnInHere tries every spawn point, with 32 "at once" (not MaxMonsters
/// - NumMonsters). On failure, TryToSpawnInAnotherVolume(true).
fn add_boss(g: &mut WaveGame, data: &mut GameData, ctx: &SpawnCtx, spawns: &mut MessageWriter<SpawnZedAt>) {
    g.final_squad_num = 0;
    g.next_squad = vec![data.boss_class.clone()];
    if g.last_volume.is_none() {
        g.last_volume = find_volume(g, data, ctx, false, true);
        if g.last_volume.is_none() {
            g.last_volume = find_volume(g, data, ctx, true, true);
        }
        if g.last_volume.is_some() {
            g.last_spawning_volume = g.last_volume;
        }
    }
    let Some(v) = g.last_volume else {
        runlog::kv("boss_no_volume", "retry=next_tick");
        try_another_volume(g, data, ctx, true);
        return;
    };
    let mut limits = Limits {
        total: g.total_max_monsters,
        at_once: MAX_ZOMBIES_ONCE,
        try_all: true,
    };
    let spawned = spawn_in_here(g, data, v, ctx, &mut limits, spawns);
    if spawned > 0 {
        g.total_max_monsters = limits.total;
        g.living += spawned;
        runlog::kv("boss_spawned", &format!("volume={}", data.volumes[v].name));
    } else {
        runlog::kv("boss_spawn_failed", &format!("volume={}", data.volumes[v].name));
        try_another_volume(g, data, ctx, true);
    }
}

/// TryToSpawnInAnotherVolume.
fn try_another_volume(g: &mut WaveGame, data: &GameData, ctx: &SpawnCtx, boss: bool) {
    g.last_volume = find_volume(g, data, ctx, false, boss);
    if g.last_volume.is_some() {
        g.last_spawning_volume = g.last_volume;
    }
}

/// The boss's own calls into the game, checked every frame: a finished
/// KnockDown (AddBossBuddySquad when FinalSquadNum == SyringeCount) and
/// his death (DoBossDeath).
fn boss_rules(
    g: &mut WaveGame,
    data: &mut GameData,
    zeds: &Query<&crate::zeds::zed::Zed>,
    ctx: &SpawnCtx,
    spawns: &mut MessageWriter<SpawnZedAt>,
    boss_died: &mut MessageWriter<BossDied>,
) {
    for z in zeds.iter() {
        let Some((knockdowns, syringes)) = z.boss_knockdowns() else { continue };
        if z.is_dead() {
            if !g.boss_killed {
                g.boss_killed = true;
                boss_died.write(BossDied);
                runlog::kv("boss_killed", &format!("id={} other_zeds={}", z.id, g.living));
            }
            continue;
        }
        while g.boss_knockdowns_seen < knockdowns {
            g.boss_knockdowns_seen += 1;
            if g.final_squad_num == syringes {
                add_boss_buddy_squad(g, data, ctx, spawns);
            } else {
                runlog::kv("boss_helpers", &format!("skipped=true final_squad_num={} syringes={syringes}", g.final_squad_num));
            }
        }
    }
}

/// The boss-wave Timer's view rules and MatchOver's BossLaughtIt, once a
/// game second: the first time he has spawned (TotalMaxMonsters <= 0,
/// NumMonsters > 0), MakeGrandEntry and the view to him; then, once his
/// bShotAnim is off, the view back to the player. After the match, a
/// living Patriarch laughs (SetBossLaught) and the view goes to him.
fn boss_view_tick(
    g: &mut WaveGame,
    zeds: &Query<&crate::zeds::zed::Zed>,
    view: &mut crate::engine::view_target::ViewTarget,
    actions: &mut MessageWriter<crate::zeds::zed::BossAction>,
    music: &mut MessageWriter<crate::audio::music::MusicCue>,
) {
    let living_boss = || zeds.iter().find(|z| z.boss_knockdowns().is_some() && !z.is_dead());
    match g.phase {
        Phase::BossWave => {
            if !g.has_set_view && g.total_max_monsters <= 0 && g.living > 0 {
                g.has_set_view = true;
                if let Some(z) = living_boss() {
                    actions.write(crate::zeds::zed::BossAction::Entrance(z.id));
                    // ClientSetMusic(BossBattleSong, MTRAN_FastFade).
                    music.write(crate::audio::music::MusicCue::Boss);
                    view.set(Some(z.id), "entrance");
                }
            } else if view.reason() == "entrance"
                && let Some(id) = view.zed()
                && !zeds.iter().any(|z| z.id == id && z.boss_shot_anim())
            {
                view.set(None, "entrance_done");
            }
        }
        Phase::Won | Phase::Lost if !g.boss_laughed => {
            g.boss_laughed = true;
            if let Some(z) = living_boss() {
                actions.write(crate::zeds::zed::BossAction::Laugh(z.id));
                view.set(Some(z.id), "victory_laugh");
            }
        }
        _ => {}
    }
}

/// AddBossBuddySquad: 8 helpers for one player, from FinalSquads
/// [FinalSquadNum], up to 10 passes, each in the best normal volume,
/// ignoring MaxMonsters and the wave's TotalMaxMonsters; then the next
/// squad number.
fn add_boss_buddy_squad(g: &mut WaveGame, data: &mut GameData, ctx: &SpawnCtx, spawns: &mut MessageWriter<SpawnZedAt>) {
    const TOTAL_ZEDS: usize = 8; // NumPlayers == 1
    let squad_num = g.final_squad_num;
    let mut total_spawned = 0usize;
    let mut passes = 0;
    let reason = 'passes: {
        for _ in 0..10 {
            passes += 1;
            if total_spawned >= TOTAL_ZEDS {
                break 'passes "enough";
            }
            g.next_squad = data.final_squads.get(squad_num).cloned().unwrap_or_default();
            g.last_volume = find_volume(g, data, ctx, false, false);
            if g.last_volume.is_none() {
                g.last_volume = find_volume(g, data, ctx, false, false);
            }
            if g.last_volume.is_some() {
                g.last_spawning_volume = g.last_volume;
            }
            // Trim from the front so the total stays at TOTAL_ZEDS.
            if g.next_squad.len() + total_spawned > TOTAL_ZEDS {
                let diff = g.next_squad.len() + total_spawned - TOTAL_ZEDS;
                if g.next_squad.len() <= diff {
                    break 'passes "trimmed_empty";
                }
                g.next_squad.drain(..diff);
            }
            // No volume: KF calls SpawnInHere on None, which does nothing.
            let Some(v) = g.last_volume else { continue };
            let mut limits = Limits {
                total: 999,
                at_once: 999,
                try_all: false,
            };
            let n = spawn_in_here(g, data, v, ctx, &mut limits, spawns);
            total_spawned += n;
            let n = n.min(g.next_squad.len());
            g.next_squad.drain(..n);
        }
        "passes_done"
    };
    g.final_squad_num += 1;
    runlog::kv(
        "boss_helpers",
        &format!("squad={squad_num} spawned={total_spawned} passes={passes} end={reason} next_squad_num={}", g.final_squad_num),
    );
}

/// DoWaveEnd: WaveTimeElapsed reset only after the first wave, the
/// countdown to TimeBetweenWaves, WaveNum + 1, every door's RespawnDoor
/// (door.rs). The team's dosh is paid in dosh.rs.
fn do_wave_end(g: &mut WaveGame, respawn: &mut MessageWriter<crate::world::door::RespawnDoors>, hud_messages: &mut MessageWriter<crate::game::hud::LocalMessage>) {
    if g.wave_num < 1 {
        g.wave_time_elapsed = 0.0;
    }
    runlog::kv("wave_end", &format!("wave={}", if g.phase == Phase::BossWave { "boss".to_string() } else { (g.wave_num + 1).to_string() }));
    g.phase = Phase::Countdown;
    // KFGameType.InitGame: TimeBetweenWaves by difficulty (Beginner 90, else 60).
    g.countdown = crate::game::difficulty::current().time_between_waves();
    g.wave_num += 1;
    // RewardSurvivingPlayers (dosh.rs).
    g.waves_ended += 1;
    respawn.write(crate::world::door::RespawnDoors);
    // "WAVE COMPLETED! GET TO THE TRADER!" unless the next is the boss
    // (WaveNum, already raised, < FinalWave).
    if g.wave_num < g.final_wave {
        hud_messages.write(crate::game::hud::LocalMessage::new(crate::game::hud::MessageClass::Waiting, 2));
    }
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
        let n = crate::game::difficulty::Difficulty::Normal;
        assert!((g.next_squad_time_at(&data, GameLength::Short, n) - 7.5).abs() < 1e-4);
        // Hard and up: x 0.85 before the sine term: 2.125 x 3.
        assert!((g.next_squad_time_at(&data, GameLength::Short, crate::game::difficulty::Difficulty::Hard) - 6.375).abs() < 1e-4);
        // |sin| = 1 at elapsed pi / 2 / 0.04: just the period.
        g.wave_time_elapsed = std::f32::consts::FRAC_PI_2 / SINE_WAVE_FREQ;
        assert!((g.next_squad_time_at(&data, GameLength::Short, n) - 2.5).abs() < 1e-3);
    }

    #[test]
    fn wave_sizes_by_difficulty() {
        use crate::game::difficulty::Difficulty::*;
        // Short wave 1 (WaveMaxMonsters 20): 14, 20, 26, 30, 34.
        assert_eq!([Beginner, Normal, Hard, Suicidal, HellOnEarth].map(|d| wave_total(20, d)), [14, 20, 26, 30, 34]);
        // Short wave 2 (32): Beginner int(22.4) = 22, Hard int(41.6) = 41, HoE int(54.4) = 54.
        assert_eq!([Beginner, Hard, HellOnEarth].map(|d| wave_total(32, d)), [22, 41, 54]);
        // At least 5.
        assert_eq!(wave_total(6, Beginner), 5);
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
