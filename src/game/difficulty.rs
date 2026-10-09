//! KF's difficulty (GameInfo.GameDifficulty): 1 Beginner, 2 Normal, 4
//! Hard, 5 Suicidal, 7 Hell on Earth (KFMod.int GIPropsExtras[0],
//! KFGui.KFMapPage.GameDifficultyChange). One value for the whole run,
//! like KF's Level.Game.GameDifficulty: set at startup (`set_current`),
//! read with `current()`. The rules below take the difficulty as a
//! parameter and copy KF's thresholds (`>= 7`, `>= 5`, `>= 4`, `>= 2`),
//! so "Suicidal and up" includes Hell on Earth. See DESIGN.md,
//! "Difficulty".

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Difficulty {
    Beginner,
    /// KF's default (KillingFloor.ini GameDifficulty=2), and what every
    /// rule here used before difficulties existed.
    #[default]
    Normal,
    Hard,
    Suicidal,
    HellOnEarth,
}

impl Difficulty {
    pub const ALL: [Difficulty; 5] = [Self::Beginner, Self::Normal, Self::Hard, Self::Suicidal, Self::HellOnEarth];

    /// `--difficulty` words (`hoe` or `hell` for Hell on Earth), KF's
    /// names, the `{:?}` form the host sends (`HellOnEarth`), or the
    /// GameDifficulty number; any case.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase().replace([' ', '_', '-'], "");
        match s.as_str() {
            "beginner" | "1" => Some(Self::Beginner),
            "normal" | "2" => Some(Self::Normal),
            "hard" | "4" => Some(Self::Hard),
            "suicidal" | "5" => Some(Self::Suicidal),
            "hoe" | "hell" | "hellonearth" | "7" => Some(Self::HellOnEarth),
            _ => None,
        }
    }

    /// The `--difficulty` word.
    pub fn word(self) -> &'static str {
        match self {
            Self::Beginner => "beginner",
            Self::Normal => "normal",
            Self::Hard => "hard",
            Self::Suicidal => "suicidal",
            Self::HellOnEarth => "hoe",
        }
    }

    /// The name KF shows (KFMod.int KFScoreBoard.SkillLevel[N], KFGui.int
    /// LobbyMenu BeginnerString .. HellOnEarthString).
    pub fn name(self) -> &'static str {
        match self {
            Self::Beginner => "Beginner",
            Self::Normal => "Normal",
            Self::Hard => "Hard",
            Self::Suicidal => "Suicidal",
            Self::HellOnEarth => "Hell on Earth",
        }
    }

    /// GameInfo.GameDifficulty.
    pub fn game_difficulty(self) -> f32 {
        match self {
            Self::Beginner => 1.0,
            Self::Normal => 2.0,
            Self::Hard => 4.0,
            Self::Suicidal => 5.0,
            Self::HellOnEarth => 7.0,
        }
    }

    fn gd(self) -> f32 {
        self.game_difficulty()
    }

    /// KFMonster.DifficultyHealthModifer: zed Health and HealthMax x this
    /// (PostBeginPlay); DifficultyHeadHealthModifer has the same values.
    pub fn zed_health_scale(self) -> f32 {
        match self.gd() {
            g if g >= 7.0 => 1.75,
            g if g >= 5.0 => 1.55,
            g if g >= 4.0 => 1.35,
            g if g >= 2.0 => 1.0,
            _ => 0.5,
        }
    }

    /// KFMonster.DifficultyDamageModifer: x 0.75 more when NumPlayers is 1.
    pub fn zed_damage_scale(self, solo: bool) -> f32 {
        let m = match self.gd() {
            g if g >= 7.0 => 1.75,
            g if g >= 5.0 => 1.50,
            g if g >= 4.0 => 1.25,
            g if g >= 2.0 => 1.0,
            _ => 0.3,
        };
        if solo { m * 0.75 } else { m }
    }

    /// KFMonster.PostBeginPlay MovementSpeedDifficultyScale: GroundSpeed
    /// (and OriginalGroundSpeed, what the speed changes start from) x
    /// this. HiddenGroundSpeed is not scaled.
    pub fn zed_speed_scale(self) -> f32 {
        match self.gd() {
            g if g < 2.0 => 0.95,
            g if g < 4.0 => 1.0,
            g if g < 5.0 => 1.15,
            g if g < 7.0 => 1.22,
            _ => 1.3,
        }
    }

    /// KFGameType.SetupWave DifficultyMod: WaveMaxMonsters x this.
    pub fn wave_size_scale(self) -> f32 {
        match self.gd() {
            g if g >= 7.0 => 1.7,
            g if g >= 5.0 => 1.5,
            g if g >= 4.0 => 1.3,
            g if g >= 2.0 => 1.0,
            _ => 0.7,
        }
    }

    /// KFGameType.ScoreKill: KillScore = ScoringValue x this.
    pub fn kill_score_scale(self) -> f32 {
        match self.gd() {
            g if g >= 5.0 => 0.65,
            g if g >= 4.0 => 0.85,
            g if g >= 2.0 => 1.0,
            _ => 2.0,
        }
    }

    /// KFGameType.InitGame: StartingCash, MinRespawnCash and
    /// TimeBetweenWaves from the StartingCash* / MinRespawnCash* /
    /// TimeBetweenWaves* class defaults.
    pub fn starting_cash(self) -> f32 {
        match self.gd() {
            g if g >= 7.0 => 100.0,
            g if g >= 5.0 => 200.0,
            g if g >= 4.0 => 250.0,
            g if g >= 2.0 => 250.0,
            _ => 300.0,
        }
    }

    pub fn min_respawn_cash(self) -> f32 {
        match self.gd() {
            g if g >= 7.0 => 100.0,
            g if g >= 5.0 => 150.0,
            g if g >= 4.0 => 200.0,
            g if g >= 2.0 => 200.0,
            _ => 250.0,
        }
    }

    pub fn time_between_waves(self) -> i32 {
        if self.gd() >= 2.0 { 60 } else { 90 }
    }

    /// KFGameType.CalcNextSquadSpawnTime: "the zeds come a little faster
    /// at all times on Hard and above": NextSpawnTime x 0.85.
    pub fn squad_time_scale(self) -> f32 {
        if self.gd() >= 4.0 { 0.85 } else { 1.0 }
    }

    /// KFBloatVomit.DifficultyDamageModifer: BaseDamage and Damage =
    /// Max(int(x d), 1) (no player-count factor).
    pub fn vomit_damage_scale(self) -> f32 {
        match self.gd() {
            g if g >= 7.0 => 2.5,
            g if g >= 5.0 => 2.0,
            g if g >= 4.0 => 1.5,
            g if g >= 2.0 => 1.0,
            _ => 0.3,
        }
    }

    /// HuskFireProjectile.PostBeginPlay: Damage = default.Damage x this.
    pub fn husk_fire_damage_scale(self) -> f32 {
        match self.gd() {
            g if g < 2.0 => 0.75,
            g if g < 4.0 => 1.0,
            g if g < 5.0 => 1.15,
            _ => 1.3,
        }
    }

    /// BossLAWProj.PostBeginPlay: the Patriarch's rocket Damage =
    /// default.Damage x this; with one player Beginner and Normal are lower.
    pub fn boss_rocket_scale(self, solo: bool) -> f32 {
        match self.gd() {
            g if g < 2.0 => if solo { 0.25 } else { 0.375 },
            g if g < 4.0 => if solo { 0.375 } else { 1.0 },
            g if g < 5.0 => 1.15,
            _ => 1.3,
        }
    }

    /// ZombieBoss.PostBeginPlay: MGDamage = default.MGDamage x this.
    pub fn boss_mg_scale(self, solo: bool) -> f32 {
        match self.gd() {
            g if g < 2.0 => 0.375,
            g if g < 4.0 => if solo { 0.75 } else { 1.0 },
            g if g < 5.0 => 1.15,
            _ => 1.3,
        }
    }
}

/// KFMonster.PostBeginPlay: MeleeDamage / ScreamDamage (ints) =
/// Max(DifficultyDamageModifer x damage, 1), one player.
pub fn zed_damage(d: f32, difficulty: Difficulty) -> f32 {
    (d * difficulty.zed_damage_scale(true)).trunc().max(1.0)
}

/// KFMonster.PostBeginPlay: Health is an int (`Health *= modifier` cuts
/// the fraction); HealthMax and HeadHealth are floats.
pub fn zed_health(health: f32, difficulty: Difficulty) -> (f32, f32) {
    let s = difficulty.zed_health_scale();
    ((health * s).trunc(), health * s)
}

static CURRENT: AtomicU8 = AtomicU8::new(1); // Normal

/// Sets the game's difficulty (startup: `--difficulty`, or the host's).
pub fn set_current(d: Difficulty) {
    let i = Difficulty::ALL.iter().position(|x| *x == d).unwrap_or(1);
    CURRENT.store(i as u8, Ordering::Relaxed);
}

/// The game's difficulty (Level.Game.GameDifficulty).
pub fn current() -> Difficulty {
    Difficulty::ALL.get(CURRENT.load(Ordering::Relaxed) as usize).copied().unwrap_or_default()
}

/// The game's GameDifficulty number.
pub fn game_difficulty() -> f32 {
    current().game_difficulty()
}

/// The startup log line, with the scales the rules use.
pub fn log_line(d: Difficulty) -> String {
    format!(
        "level={} name=\"{}\" game_difficulty={} zed_health_scale={} zed_damage_scale_solo={} zed_speed_scale={} wave_size_scale={} kill_score_scale={} starting_cash={} min_respawn_cash={} time_between_waves={} squad_time_scale={}",
        d.word(),
        d.name(),
        d.game_difficulty(),
        d.zed_health_scale(),
        d.zed_damage_scale(true),
        d.zed_speed_scale(),
        d.wave_size_scale(),
        d.kill_score_scale(),
        d.starting_cash(),
        d.min_respawn_cash(),
        d.time_between_waves(),
        d.squad_time_scale(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use Difficulty::*;

    #[test]
    fn parses_words_names_and_numbers() {
        assert_eq!(Difficulty::parse("hoe"), Some(HellOnEarth));
        assert_eq!(Difficulty::parse("Hell on Earth"), Some(HellOnEarth));
        assert_eq!(Difficulty::parse("HellOnEarth"), Some(HellOnEarth));
        assert_eq!(Difficulty::parse("SUICIDAL"), Some(Suicidal));
        assert_eq!(Difficulty::parse("4"), Some(Hard));
        assert_eq!(Difficulty::parse("3"), None);
        assert_eq!(Difficulty::parse("insane"), None);
        for d in Difficulty::ALL {
            assert_eq!(Difficulty::parse(d.word()), Some(d));
            assert_eq!(Difficulty::parse(d.name()), Some(d));
            assert_eq!(Difficulty::parse(&format!("{d:?}")), Some(d));
        }
        assert_eq!(Difficulty::ALL.map(|d| d.game_difficulty()), [1.0, 2.0, 4.0, 5.0, 7.0]);
    }

    #[test]
    fn zed_health_by_difficulty() {
        // ZombieClot Health 130, HeadHealth 25 (class defaults).
        assert_eq!(zed_health(130.0, Beginner), (65.0, 65.0));
        assert_eq!(zed_health(130.0, Normal), (130.0, 130.0));
        assert_eq!(zed_health(130.0, Hard), (175.0, 175.5)); // 175.5 cut to 175
        assert_eq!(zed_health(130.0, HellOnEarth), (227.0, 227.5));
        assert!((25.0 * HellOnEarth.zed_health_scale() - 43.75).abs() < 1e-4);
        // ZombieFleshPound Health 1500: Suicidal 2325.
        assert_eq!(zed_health(1500.0, Suicidal).0, 2325.0);
    }

    #[test]
    fn zed_damage_by_difficulty_solo() {
        // Clot MeleeDamage 6: Beginner int(6 x 0.3 x 0.75) = 1, Normal
        // int(4.5) = 4, Hard int(5.625) = 5, Suicidal int(6.75) = 6, HoE
        // int(7.875) = 7.
        assert_eq!(Difficulty::ALL.map(|d| zed_damage(6.0, d)), [1.0, 4.0, 5.0, 6.0, 7.0]);
        // Siren ScreamDamage 8: 1, 6, 7, 9, 10.
        assert_eq!(Difficulty::ALL.map(|d| zed_damage(8.0, d)), [1.0, 6.0, 7.0, 9.0, 10.0]);
        // Never below 1.
        assert_eq!(zed_damage(1.0, Beginner), 1.0);
        assert_eq!(HellOnEarth.zed_damage_scale(false), 1.75);
    }

    #[test]
    fn game_rules_by_difficulty() {
        assert_eq!(Difficulty::ALL.map(|d| d.zed_speed_scale()), [0.95, 1.0, 1.15, 1.22, 1.3]);
        assert_eq!(Difficulty::ALL.map(|d| d.wave_size_scale()), [0.7, 1.0, 1.3, 1.5, 1.7]);
        assert_eq!(Difficulty::ALL.map(|d| d.kill_score_scale()), [2.0, 1.0, 0.85, 0.65, 0.65]);
        assert_eq!(Difficulty::ALL.map(|d| d.starting_cash()), [300.0, 250.0, 250.0, 200.0, 100.0]);
        assert_eq!(Difficulty::ALL.map(|d| d.min_respawn_cash()), [250.0, 200.0, 200.0, 150.0, 100.0]);
        assert_eq!(Difficulty::ALL.map(|d| d.time_between_waves()), [90, 60, 60, 60, 60]);
        assert_eq!(Difficulty::ALL.map(|d| d.squad_time_scale()), [1.0, 1.0, 0.85, 0.85, 0.85]);
        assert_eq!(Difficulty::ALL.map(|d| d.vomit_damage_scale()), [0.3, 1.0, 1.5, 2.0, 2.5]);
        assert_eq!(Difficulty::ALL.map(|d| d.husk_fire_damage_scale()), [0.75, 1.0, 1.15, 1.3, 1.3]);
        assert_eq!(Difficulty::ALL.map(|d| d.boss_rocket_scale(true)), [0.25, 0.375, 1.15, 1.3, 1.3]);
        assert_eq!(Difficulty::ALL.map(|d| d.boss_mg_scale(true)), [0.375, 0.75, 1.15, 1.3, 1.3]);
        assert_eq!(Difficulty::ALL.map(|d| d.boss_mg_scale(false)), [0.375, 1.0, 1.15, 1.3, 1.3]);
    }
}
