//! Dosh (T1): the player's cash (PlayerReplicationInfo.Score) and the team
//! pot (Team.Score), from KFGameType ScoreKill, ScoreKillAssists and
//! RewardSurvivingPlayers. One player, Normal difficulty. See DESIGN.md,
//! "Game loop", T1.

use bevy::prelude::*;

use crate::game::waves::{GameLength, GameOptions, WaveGame};
use crate::engine::runlog;

/// StartingCashNormal.
pub const STARTING_CASH: f32 = 250.0;
/// KillingFloor.ini GameDifficulty (Normal).
pub const GAME_DIFFICULTY: f32 = 2.0;

#[derive(Resource, Debug)]
pub struct Dosh {
    /// PRI.Score: a float in KF, shown as a whole number.
    pub score: f32,
    /// Team.Score: kill rewards paid out at the wave end.
    pub team: f32,
}

impl Default for Dosh {
    fn default() -> Self {
        Dosh { score: STARTING_CASH, team: 0.0 }
    }
}

impl Dosh {
    /// ScoreKill for a zed the player killed: KillScore to the player (all
    /// of it: the only kill assistant) and to the team pot.
    pub fn kill(&mut self, scoring_value: f32, length: GameLength) -> f32 {
        let score = kill_score(scoring_value, length);
        self.score += score;
        self.team += score;
        score
    }

    /// RewardSurvivingPlayers, one living player: the whole pot.
    pub fn wave_reward(&mut self) -> f32 {
        let pot = self.team.max(0.0);
        if pot > 0.0 {
            self.score += pot;
        }
        self.team = 0.0;
        pot
    }

    /// ScoreKill on the player: lose GameDifficulty x 5% of the cash; the
    /// pot loses as much of the new cash.
    pub fn death(&mut self) -> f32 {
        let lost = self.score * GAME_DIFFICULTY * 0.05;
        self.score -= lost;
        self.team -= self.score * GAME_DIFFICULTY * 0.05;
        self.score = self.score.max(0.0);
        self.team = self.team.max(0.0);
        lost
    }
}

/// KillScore = Max(1, int(ScoringValue x 1.0 (Normal) x 1.75 (Short))).
pub fn kill_score(scoring_value: f32, length: GameLength) -> f32 {
    let mut s = scoring_value * 1.0;
    if length == GameLength::Short {
        s *= 1.75;
    }
    (s as i32).max(1) as f32
}

pub struct DoshPlugin;

impl Plugin for DoshPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Dosh>().add_systems(Update, update_dosh.after(crate::game::waves::wave_timer));
    }
}

/// Pays kills, the wave-end pot and the death penalty; a new game starts
/// again from the starting cash.
fn update_dosh(
    mut dosh: ResMut<Dosh>,
    options: Res<GameOptions>,
    game: Res<WaveGame>,
    health: Res<crate::game::combat::PlayerHealth>,
    mut zeds: Query<&mut crate::zeds::zed::Zed>,
    mut seen: Local<Option<(u32, u32, u32)>>,
    (script, frames): (Res<crate::weapons::weapon::ScriptedInput>, Res<bevy::diagnostic::FrameCount>),
) {
    // Test action "add_dosh": + 5000.
    if script.0.iter().any(|(f, a)| *f == frames.0 && a == "add_dosh") {
        dosh.score += 5000.0;
        runlog::kv("dosh", &format!("reason=test amount=5000 total={:.0}", dosh.score));
    }
    let (restarts, waves_ended, deaths) = seen.get_or_insert((game.restarts, game.waves_ended, health.deaths));
    if game.restarts != *restarts {
        *restarts = game.restarts;
        *waves_ended = game.waves_ended;
        *dosh = Dosh::default();
        runlog::kv("dosh", &format!("reason=new_game total={:.0}", dosh.score));
    }
    for mut z in &mut zeds {
        if z.is_dead() && z.killed_by_player && !z.kill_paid {
            z.kill_paid = true;
            let paid = dosh.kill(z.scoring_value, options.length);
            runlog::kv("dosh", &format!("reason=kill zed={} amount={paid:.0} total={:.0} team={:.0}", z.id, dosh.score, dosh.team));
        }
    }
    if health.deaths != *deaths {
        *deaths = health.deaths;
        let lost = dosh.death();
        runlog::kv("dosh", &format!("reason=death amount=-{lost:.1} total={:.0} team={:.0}", dosh.score, dosh.team));
    }
    if game.waves_ended != *waves_ended {
        *waves_ended = game.waves_ended;
        // Only a living player is paid; solo, a dead player has lost.
        if game.phase != crate::game::waves::Phase::Lost {
            let pot = dosh.wave_reward();
            runlog::kv("dosh", &format!("reason=wave_end amount={pot:.0} total={:.0}", dosh.score));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kill_scores_match_kf() {
        // Short x 1.75, truncated: Clot 7 -> 12, Fleshpound 200 -> 350.
        assert_eq!(kill_score(7.0, GameLength::Short), 12.0);
        assert_eq!(kill_score(200.0, GameLength::Short), 350.0);
        assert_eq!(kill_score(7.0, GameLength::Normal), 7.0);
        // Never less than 1.
        assert_eq!(kill_score(0.0, GameLength::Long), 1.0);
    }

    #[test]
    fn kills_pay_twice_solo() {
        let mut d = Dosh::default();
        d.kill(7.0, GameLength::Short);
        d.kill(10.0, GameLength::Short);
        assert_eq!(d.score, 250.0 + 12.0 + 17.0);
        assert_eq!(d.wave_reward(), 29.0);
        assert_eq!(d.score, 250.0 + 2.0 * 29.0);
        assert_eq!(d.team, 0.0);
    }

    #[test]
    fn death_costs_ten_percent() {
        let mut d = Dosh { score: 300.0, team: 100.0 };
        d.death();
        assert_eq!(d.score, 270.0);
        // The pot loses 10% of the new cash.
        assert_eq!(d.team, 73.0);
    }
}
