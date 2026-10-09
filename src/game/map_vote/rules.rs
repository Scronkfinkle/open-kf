//! KF's map vote rules (xVoting.xVotingHandler with KF's settings), as
//! plain data and functions: no Bevy systems, no clock, no randomness of
//! its own. The caller ticks it once a second and hands it a random
//! number function (`Rand(n)`: 0 to n-1), so tests are repeatable.
//!
//! What it copies from KF (KillingFloor.ini `[xVoting.xVotingHandler]`,
//! the handler's script):
//! - Only "majority mode" (bScoreMode, bAccumulationMode and
//!   bEliminationMode are False in the ini): one vote per player.
//! - HandleRestartGame (the end of the match, when the game would go to
//!   the next map): travel is held back; the vote windows open after
//!   ScoreBoardDelay, then VoteTimeLimit counts down once a second.
//! - SubmitMapVote / TallyVotes: votes per player, a vote can be changed;
//!   voting ends early when every player has voted, or (more than 2
//!   players, game over) when one map has more than half the players;
//!   at 0 the leading map wins; nobody voted: GetDefaultMap's random
//!   map; a tie: one of the tied maps at random, not the current map.
//! - The map history (MapVoteHistory_INI): maps played within the last
//!   RepeatLimit votes cannot be voted for.
//! - Mid-game voting (more than 2 players, MidGameVotePercent of them
//!   voted while the game runs): in the rules here, not wired in the game.
//!
//! Not copied: admin votes (an admin's vote changes the map at once),
//! spectators (we have none), kick voting, match setup, the other modes,
//! game types other than KF's (one GameConfig, prefix "KF").

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};

/// KFGameType MapPrefix and Acronym: the only game type in the list
/// (xVotingHandler.LoadMapList's auto-detect mode: GameConfig empty in
/// the ini, so the current game's prefix and acronym are used).
pub const GAME_PREFIX: &str = "KF";
pub const GAME_ACRONYM: &str = "KF";

/// The vote settings, KF's values (KillingFloor.ini
/// `[xVoting.xVotingHandler]`; the class defaults differ for two:
/// VoteTimeLimit 70 and RepeatLimit 4, the ini's 30 and 1 are used).
/// The command line / launcher switch (`--map-vote`) is wired elsewhere.
#[derive(Resource, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VoteConfig {
    /// bMapVote=False: map voting is off unless turned on.
    pub enabled: bool,
    /// Ours: vote in a single-player game too. KF never votes there
    /// (PostBeginPlay / HandleRestartGame return early when NetMode is
    /// NM_StandAlone). Off by default; `--vote-test` turns it on.
    pub single_player: bool,
    /// bAutoOpen=True: the windows open by themselves at the end.
    pub auto_open: bool,
    /// VoteTimeLimit=30 (seconds of voting).
    pub time_limit: u32,
    /// ScoreBoardDelay=5 (seconds before the windows open).
    pub scoreboard_delay: u32,
    /// RepeatLimit=1: maps played within this many votes are disabled.
    pub repeat_limit: u32,
    /// MidGameVotePercent=50.
    pub mid_game_vote_percent: u32,
    /// MinMapCount=2: only used by elimination mode (not built), kept for
    /// the record.
    pub min_map_count: u32,
}

impl Default for VoteConfig {
    fn default() -> Self {
        VoteConfig { enabled: false, single_player: false, auto_open: true, time_limit: 30, scoreboard_delay: 5, repeat_limit: 1, mid_game_vote_percent: 50, min_map_count: 2 }
    }
}

/// One map's record in the history (MapHistoryInfo: M name, P play
/// count, S sequence: 1 = the last map voted in, 2 the one before, 0 =
/// never played).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub name: String,
    pub plays: u32,
    pub seq: u32,
}

/// The maps won by votes (MapVoteHistory_INI). KF keeps it in
/// MapVoteHistory1.ini; ours lives in memory for the run (the game no
/// longer restarts between maps). Not saved to disk yet.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct MapHistory {
    pub entries: Vec<HistoryEntry>,
}

impl MapHistory {
    /// MapVoteHistory_INI.PlayMap: this map becomes sequence 1 and its
    /// play count goes up; every other played map moves one further back.
    pub fn play_map(&mut self, name: &str) {
        if name.is_empty() {
            return;
        }
        let mut found = false;
        for e in &mut self.entries {
            if e.name.eq_ignore_ascii_case(name) {
                e.seq = 1;
                e.plays += 1;
                found = true;
            } else if e.seq > 0 {
                e.seq += 1;
            }
        }
        if !found {
            self.entries.push(HistoryEntry { name: name.to_string(), plays: 1, seq: 1 });
        }
    }

    /// (play count, sequence) of a map, (0, 0) if never played.
    pub fn get(&self, name: &str) -> (u32, u32) {
        self.entries.iter().find(|e| e.name.eq_ignore_ascii_case(name)).map_or((0, 0), |e| (e.plays, e.seq))
    }
}

/// One map in the vote list (MapVoteMapList).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub name: String,
    /// bEnabled: false for a map played within the last RepeatLimit votes.
    pub enabled: bool,
    /// PlayCount ("Played" column).
    pub plays: u32,
    /// Sequence ("Seq" column).
    pub seq: u32,
}

/// xVotingHandler.AddMap for each map: duplicates (any case) dropped,
/// the order kept; a map with history sequence 1..=RepeatLimit is
/// disabled (`MapInfo.S <= RepeatLimit && MapInfo.S != 0`).
pub fn build_candidates(maps: &[String], history: &MapHistory, repeat_limit: u32) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    for m in maps {
        if m.is_empty() || out.iter().any(|c| c.name.eq_ignore_ascii_case(m)) {
            continue;
        }
        let (plays, seq) = history.get(m);
        out.push(Candidate { name: m.clone(), enabled: !(seq <= repeat_limit && seq != 0), plays, seq });
    }
    out
}

/// xVotingHandler.IsValidVote: the map's name starts with the game
/// type's prefix (case-insensitive).
pub fn has_prefix(map: &str) -> bool {
    map.len() >= GAME_PREFIX.len() && map[..GAME_PREFIX.len()].eq_ignore_ascii_case(GAME_PREFIX)
}

/// Why a vote was not taken (SubmitMapVote returns without a message for
/// all of these).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// bLevelSwitchPending: a map has already won.
    Finished,
    /// The voter is not one of the players.
    NotAPlayer,
    /// No such map in the list.
    UnknownMap,
    /// Not the game type's prefix (IsValidVote).
    WrongPrefix,
    /// Disabled by RepeatLimit (the window also says "The selected Map is
    /// disabled.").
    Disabled,
    /// The same map as this player's current vote.
    SameVote,
}

impl Refused {
    pub fn word(self) -> &'static str {
        match self {
            Refused::Finished => "finished",
            Refused::NotAPlayer => "not_a_player",
            Refused::UnknownMap => "unknown_map",
            Refused::WrongPrefix => "wrong_prefix",
            Refused::Disabled => "disabled",
            Refused::SameVote => "same_vote",
        }
    }
}

/// Why voting ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EndReason {
    /// Every player has voted (`NumPlayers == PlayersThatVoted`).
    AllVoted,
    /// More than 2 players, the game is over, and one map has more than
    /// half of them.
    Majority,
    /// TimeLeft reached 0 (TallyVotes(true)): the leading map.
    TimeUp,
    /// TimeLeft reached 0 and nobody voted: GetDefaultMap's random map.
    NoVotes,
}

impl EndReason {
    pub fn word(self) -> &'static str {
        match self {
            EndReason::AllVoted => "all_voted",
            EndReason::Majority => "majority",
            EndReason::TimeUp => "time_up",
            EndReason::NoVotes => "no_votes_random",
        }
    }
}

/// The winner.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoteResult {
    pub map: String,
    pub reason: EndReason,
    /// There was a tie, broken at random.
    pub tie: bool,
}

/// What happened during a call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VoteEvent {
    /// OpenAllVoteWindows (ScoreBoardTime reached 0).
    OpenWindows,
    /// PlayCountDown: the announcer at 60, 30, 20 and 10 seconds left.
    CountDown(i32),
    /// lmsgMidGameVote: "Mid-Game Map Voting has been initiated !!!!".
    MidGameStarted,
    /// lmsgMapWon: a map has won; the windows close and the game travels.
    Finished(VoteResult),
    /// Ours: the time is up and no map can win (every map disabled, e.g.
    /// by RepeatLimit on a small install). KF's vote would wait forever
    /// (TimeLeft goes below 0 and is only checked at 0); ours ends and
    /// the game goes to the map list's next map.
    NoWinner,
}

/// lmsgMapWon with KF's `%mapname%`: the map and the game's acronym.
pub fn won_text(map: &str) -> String {
    format!("{map}({GAME_ACRONYM}) has won !")
}

/// One map vote (an xVotingHandler from HandleRestartGame to the travel).
#[derive(Clone, Debug)]
pub struct VoteSession {
    pub cfg: VoteConfig,
    pub maps: Vec<Candidate>,
    /// GetURLMap: the map being played (a tie avoids it).
    pub current_map: String,
    /// Player -> index into `maps` (MVRI[i].MapVote).
    votes: BTreeMap<u64, usize>,
    /// The players who count (Level.Game.NumPlayers): every connected
    /// player; KF's spectators do not count, we have none.
    players: BTreeSet<u64>,
    /// Level.Game.bGameEnded.
    pub game_ended: bool,
    /// bMidGameVote.
    pub mid_game: bool,
    /// The once-a-second timer runs (SetTimer(1, true)).
    pub timer_running: bool,
    /// ScoreBoardTime: seconds until the windows open; -1 once open.
    pub scoreboard_time: i32,
    /// TimeLeft.
    pub time_left: i32,
    /// The windows have been opened.
    pub windows_open: bool,
    /// bLevelSwitchPending: the winner.
    pub result: Option<VoteResult>,
}

impl VoteSession {
    /// The vote list for this match (LoadMapList at the map's start). The
    /// timer does not run until `handle_restart_game` (or a mid-game vote).
    pub fn new(cfg: VoteConfig, maps: &[String], history: &MapHistory, current_map: &str) -> Self {
        VoteSession {
            maps: build_candidates(maps, history, cfg.repeat_limit),
            cfg,
            current_map: current_map.to_string(),
            votes: BTreeMap::new(),
            players: BTreeSet::new(),
            game_ended: false,
            mid_game: false,
            timer_running: false,
            scoreboard_time: -1,
            time_left: 0,
            windows_open: false,
            result: None,
        }
    }

    /// HandleRestartGame: the match is over and the game would travel.
    /// Returns false when voting does not hold the travel back (map
    /// voting off, or bAutoOpen off: KF then goes to the next map as
    /// usual). Starts the countdown: TimeLeft = VoteTimeLimit,
    /// ScoreBoardTime = ScoreBoardDelay.
    pub fn handle_restart_game(&mut self) -> bool {
        self.game_ended = true;
        if !self.cfg.enabled || !self.cfg.auto_open {
            return false;
        }
        self.time_left = self.cfg.time_limit as i32;
        self.scoreboard_time = self.cfg.scoreboard_delay as i32;
        self.timer_running = true;
        true
    }

    /// The players who can vote. Players who left lose their vote
    /// (PlayerExit: the vote is taken off, then TallyVotes(false)).
    pub fn set_players(&mut self, players: impl IntoIterator<Item = u64>, rng: &mut impl FnMut(usize) -> usize) -> Vec<VoteEvent> {
        let new: BTreeSet<u64> = players.into_iter().collect();
        if new == self.players {
            return Vec::new();
        }
        let left: Vec<u64> = self.players.difference(&new).copied().collect();
        self.players = new;
        let mut events = Vec::new();
        if !left.is_empty() {
            for p in &left {
                self.votes.remove(p);
            }
            events.extend(self.tally(false, rng));
        }
        events
    }

    pub fn players(&self) -> &BTreeSet<u64> {
        &self.players
    }

    /// SubmitMapVote by map name (the window sends an index; ours sends
    /// the name, which is the same on every machine).
    pub fn cast(&mut self, player: u64, map: &str, rng: &mut impl FnMut(usize) -> usize) -> Result<Vec<VoteEvent>, Refused> {
        if self.result.is_some() {
            return Err(Refused::Finished);
        }
        let i = self.maps.iter().position(|m| m.name.eq_ignore_ascii_case(map)).ok_or(Refused::UnknownMap)?;
        if !has_prefix(&self.maps[i].name) {
            return Err(Refused::WrongPrefix);
        }
        if !self.players.contains(&player) {
            return Err(Refused::NotAPlayer);
        }
        if self.votes.get(&player) == Some(&i) {
            return Err(Refused::SameVote);
        }
        if !self.maps[i].enabled {
            return Err(Refused::Disabled);
        }
        // UpdateVoteCount(new, +1) and (old, -1): one vote per player.
        self.votes.insert(player, i);
        Ok(self.tally(false, rng))
    }

    /// Every player's vote (player, map index).
    pub fn votes(&self) -> impl Iterator<Item = (u64, usize)> + '_ {
        self.votes.iter().map(|(p, m)| (*p, *m))
    }

    /// Votes per map (MapVoteCount), by map index.
    pub fn counts(&self) -> Vec<u32> {
        let mut c = vec![0u32; self.maps.len()];
        for (p, m) in &self.votes {
            if self.players.contains(p) {
                c[*m] += 1;
            }
        }
        c
    }

    /// xVotingHandler.Timer, once a second.
    pub fn tick(&mut self, rng: &mut impl FnMut(usize) -> usize) -> Vec<VoteEvent> {
        if self.result.is_some() || !self.timer_running {
            return Vec::new();
        }
        if self.scoreboard_time > -1 {
            let mut events = Vec::new();
            if self.scoreboard_time == 0 {
                self.windows_open = true;
                events.push(VoteEvent::OpenWindows);
            }
            self.scoreboard_time -= 1;
            return events;
        }
        self.time_left -= 1;
        let mut events = Vec::new();
        if matches!(self.time_left, 60 | 30 | 20 | 10) {
            events.push(VoteEvent::CountDown(self.time_left));
        }
        if self.time_left <= 0 {
            // "if no-one has voted a random map will be choosen"
            events.extend(self.tally(true, rng));
            if self.result.is_none() {
                self.timer_running = false;
                events.push(VoteEvent::NoWinner);
            }
        }
        events
    }

    /// TallyVotes. `force`: the time is up (or an admin), a winner is
    /// picked whoever voted.
    pub fn tally(&mut self, force: bool, rng: &mut impl FnMut(usize) -> usize) -> Vec<VoteEvent> {
        let mut events = Vec::new();
        if self.result.is_some() || self.maps.is_empty() {
            return events;
        }
        let num_players = self.players.len();
        let counts = self.counts();
        let voted = self.votes.keys().filter(|p| self.players.contains(p)).count();
        // Majority mode: more than half the players on one map forces a
        // winner, once the game is over and with more than 2 players.
        let majority = num_players > 2 && self.game_ended && counts.iter().any(|&c| c as f32 / num_players as f32 > 0.5);
        // Mid-game vote: MidGameVotePercent of the players voted while
        // the game runs; the countdown starts (ScoreBoardTime 1).
        if num_players > 2 && !self.game_ended && !self.mid_game && (voted as f32 / num_players as f32) * 100.0 >= self.cfg.mid_game_vote_percent as f32 {
            self.mid_game = true;
            self.time_left = self.cfg.time_limit as i32;
            self.scoreboard_time = 1;
            self.timer_running = true;
            events.push(VoteEvent::MidGameStarted);
        }
        let all_voted = num_players > 0 && voted == num_players;
        // KF ranks (and breaks ties at random) on every call but only
        // uses the result when the vote ends; ranking only then draws the
        // same kind of random numbers, fewer of them.
        if !(force || majority || all_voted) {
            return events;
        }
        // The maps with votes, in list order, then sorted by count (KF's
        // exchange sort, kept so ties line up the same way).
        let mut ranking: Vec<usize> = (0..counts.len()).filter(|&i| counts[i] > 0).collect();
        let mut top: Option<usize>;
        let mut tie = false;
        if voted > 1 {
            for x in 0..ranking.len().saturating_sub(1) {
                for y in x + 1..ranking.len() {
                    if counts[ranking[x]] < counts[ranking[y]] {
                        ranking.swap(x, y);
                    }
                }
            }
            top = ranking.first().copied();
            if ranking.len() > 1 && counts[ranking[0]] == counts[ranking[1]] && counts[ranking[0]] != 0 {
                tie = true;
                let tie_count = 1 + (1..ranking.len()).filter(|&x| counts[ranking[x]] == counts[ranking[0]]).count();
                let mut t = ranking[rng(tie_count).min(tie_count - 1)];
                // "Don't allow same map to be choosen" (100 tries).
                let mut r = 0;
                while self.maps[t].name.eq_ignore_ascii_case(&self.current_map) {
                    t = ranking[rng(tie_count).min(tie_count - 1)];
                    if r > 100 {
                        break;
                    }
                    r += 1;
                }
                top = Some(t);
            }
        } else if voted == 0 {
            top = None; // GetDefaultMap, only if the vote ends now
        } else {
            top = ranking.first().copied(); // only one player voted
        }
        let reason = if all_voted {
            EndReason::AllVoted
        } else if majority {
            EndReason::Majority
        } else if voted == 0 {
            EndReason::NoVotes
        } else {
            EndReason::TimeUp
        };
        if top.is_none() {
            top = self.default_map(rng);
        }
        let Some(t) = top else { return events };
        let result = VoteResult { map: self.maps[t].name.clone(), reason, tie };
        self.result = Some(result.clone());
        self.timer_running = false;
        events.push(VoteEvent::Finished(result));
        events
    }

    /// GetDefaultMap: a random enabled map with the game type's prefix
    /// (up to 102 tries), else the first such map, else the first enabled
    /// map. (KF's fallback loops run one past the end of the list, so its
    /// fallback finds no map and the vote never ends; ours takes the
    /// first match.) None: no enabled map at all.
    pub fn default_map(&self, rng: &mut impl FnMut(usize) -> usize) -> Option<usize> {
        let n = self.maps.len();
        if n == 0 {
            return None;
        }
        let ok = |i: usize| self.maps[i].enabled && has_prefix(&self.maps[i].name);
        let mut r = 0;
        loop {
            let i = rng(n).min(n - 1);
            if ok(i) {
                return Some(i);
            }
            if r > 100 {
                break;
            }
            r += 1;
        }
        (0..n).find(|&i| ok(i)).or_else(|| (0..n).find(|&i| self.maps[i].enabled))
    }

    /// What the windows show (sent to every player).
    pub fn view(&self) -> VoteView {
        let counts = self.counts();
        VoteView {
            maps: self.maps.iter().zip(&counts).map(|(m, &votes)| ViewMap { name: m.name.clone(), enabled: m.enabled, plays: m.plays, seq: m.seq, votes }).collect(),
            current_map: self.current_map.clone(),
            voters: self.votes.iter().filter(|(p, _)| self.players.contains(p)).map(|(p, m)| (*p, self.maps[*m].name.clone())).collect(),
            players: self.players.len() as u32,
            window_open: self.windows_open,
            time_left: if self.timer_running || self.result.is_some() { self.time_left.max(0) } else { -1 },
            opens_in: if self.timer_running && self.scoreboard_time > -1 { self.scoreboard_time + 1 } else { 0 },
            result: self.result.clone(),
        }
    }
}

/// One map as the windows show it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewMap {
    pub name: String,
    pub enabled: bool,
    pub plays: u32,
    pub seq: u32,
    pub votes: u32,
}

/// The vote as every player's window shows it (KF replicates the map
/// list once, then each MapVoteCount change, through each player's
/// VotingReplicationInfo). The host sends the whole thing when it changes.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VoteView {
    pub maps: Vec<ViewMap>,
    pub current_map: String,
    /// (player, map voted for).
    pub voters: Vec<(u64, String)>,
    /// Players who can vote.
    pub players: u32,
    /// The windows are open (ScoreBoardDelay is over).
    pub window_open: bool,
    /// Seconds of voting left; -1: no countdown.
    pub time_left: i32,
    /// Seconds until the windows open (0: open or not counting).
    pub opens_in: i32,
    /// The winner, once there is one.
    pub result: Option<VoteResult>,
}

impl VoteView {
    /// The maps with votes, most votes first (MapVoteCountMultiColumnList,
    /// SortColumn Votes, SortDescending; equal counts in list order).
    pub fn ranked(&self) -> Vec<&ViewMap> {
        let mut v: Vec<&ViewMap> = self.maps.iter().filter(|m| m.votes > 0).collect();
        v.sort_by_key(|m| std::cmp::Reverse(m.votes));
        v
    }

    pub fn vote_of(&self, player: u64) -> Option<&str> {
        self.voters.iter().find(|(p, _)| *p == player).map(|(_, m)| m.as_str())
    }
}

/// A small random number source (xorshift) for the game; tests use their
/// own scripted functions.
#[derive(Clone, Debug)]
pub struct XorShift(pub u64);

impl XorShift {
    pub fn from_time() -> Self {
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0x2545_F491_4F6C_DD1D, |d| d.as_nanos() as u64);
        XorShift(n | 1)
    }

    /// Rand(n): 0 to n-1 (0 when n is 0).
    pub fn rand(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        if n == 0 { 0 } else { (self.0 % n as u64) as usize }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn maps(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn on() -> VoteConfig {
        VoteConfig { enabled: true, ..default() }
    }

    fn default() -> VoteConfig {
        VoteConfig::default()
    }

    /// A Rand(n) that returns these numbers in turn (each taken modulo n).
    fn seq(v: &[usize]) -> impl FnMut(usize) -> usize + '_ {
        let mut i = 0;
        move |n| {
            let x = v[i % v.len()];
            i += 1;
            if n == 0 { 0 } else { x % n }
        }
    }

    fn never(_: usize) -> usize {
        panic!("no random number expected")
    }

    const LIST: [&str; 5] = ["KF-BioticsLab", "KF-Farm", "KF-Manor", "KF-Offices", "KF-WestLondon"];

    fn session(players: &[u64]) -> VoteSession {
        let mut s = VoteSession::new(on(), &maps(&LIST), &MapHistory::default(), "KF-WestLondon");
        s.set_players(players.iter().copied(), &mut never);
        s
    }

    #[test]
    fn defaults_are_the_ini_values() {
        let c = VoteConfig::default();
        assert!(!c.enabled && !c.single_player && c.auto_open);
        assert_eq!((c.time_limit, c.scoreboard_delay, c.repeat_limit, c.mid_game_vote_percent, c.min_map_count), (30, 5, 1, 50, 2));
    }

    #[test]
    fn history_sequence_and_repeat_limit() {
        let mut h = MapHistory::default();
        h.play_map("KF-Farm");
        h.play_map("KF-Manor");
        assert_eq!(h.get("KF-Manor"), (1, 1));
        assert_eq!(h.get("kf-farm"), (1, 2));
        assert_eq!(h.get("KF-Offices"), (0, 0));
        h.play_map("KF-Farm");
        assert_eq!(h.get("KF-Farm"), (2, 1));
        assert_eq!(h.get("KF-Manor"), (1, 2));
        // RepeatLimit 1: only the last map is disabled.
        let c = build_candidates(&maps(&LIST), &h, 1);
        let disabled: Vec<&str> = c.iter().filter(|m| !m.enabled).map(|m| m.name.as_str()).collect();
        assert_eq!(disabled, ["KF-Farm"]);
        // RepeatLimit 2: the last two.
        let c = build_candidates(&maps(&LIST), &h, 2);
        assert_eq!(c.iter().filter(|m| !m.enabled).count(), 2);
        // RepeatLimit 0: none.
        assert!(build_candidates(&maps(&LIST), &h, 0).iter().all(|m| m.enabled));
    }

    #[test]
    fn duplicate_maps_are_dropped() {
        let c = build_candidates(&maps(&["KF-Farm", "kf-farm", "KF-Manor", ""]), &MapHistory::default(), 1);
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn disabled_by_config_or_auto_open_does_not_hold_travel() {
        let mut s = VoteSession::new(default(), &maps(&LIST), &MapHistory::default(), "KF-Farm");
        assert!(!s.handle_restart_game());
        let mut s = VoteSession::new(VoteConfig { auto_open: false, ..on() }, &maps(&LIST), &MapHistory::default(), "KF-Farm");
        assert!(!s.handle_restart_game());
        assert!(!s.timer_running);
    }

    #[test]
    fn windows_open_after_scoreboard_delay_then_30_seconds() {
        let mut s = session(&[0, 1]);
        assert!(s.handle_restart_game());
        // ScoreBoardTime 5: ticks 1-5 count it down, tick 6 opens.
        for _ in 0..5 {
            assert!(s.tick(&mut never).is_empty());
        }
        assert_eq!(s.tick(&mut never), vec![VoteEvent::OpenWindows]);
        assert!(s.windows_open);
        let mut countdowns = Vec::new();
        for _ in 0..29 {
            for e in s.tick(&mut never) {
                if let VoteEvent::CountDown(n) = e {
                    countdowns.push(n);
                }
            }
        }
        // 30 limit: 60 is never reached.
        assert_eq!(countdowns, vec![20, 10]);
        assert_eq!(s.time_left, 1);
        // TimeLeft 0: nobody voted -> a random enabled map.
        let ev = s.tick(&mut seq(&[2]));
        assert_eq!(ev, vec![VoteEvent::Finished(VoteResult { map: "KF-Manor".into(), reason: EndReason::NoVotes, tie: false })]);
        // Finished: no more ticks, no more votes.
        assert!(s.tick(&mut never).is_empty());
        assert_eq!(s.cast(0, "KF-Farm", &mut never), Err(Refused::Finished));
    }

    #[test]
    fn thirty_seconds_left_announces_at_30() {
        let mut s = VoteSession::new(VoteConfig { time_limit: 70, scoreboard_delay: 0, ..on() }, &maps(&LIST), &MapHistory::default(), "KF-Farm");
        s.set_players([0, 1], &mut never);
        s.handle_restart_game();
        let mut c = Vec::new();
        for _ in 0..70 {
            for e in s.tick(&mut seq(&[0])) {
                if let VoteEvent::CountDown(n) = e {
                    c.push(n);
                }
            }
        }
        assert_eq!(c, vec![60, 30, 20, 10]);
    }

    #[test]
    fn single_player_vote_ends_at_once() {
        let mut s = session(&[0]);
        s.handle_restart_game();
        let ev = s.cast(0, "KF-Farm", &mut never).unwrap();
        assert_eq!(ev, vec![VoteEvent::Finished(VoteResult { map: "KF-Farm".into(), reason: EndReason::AllVoted, tie: false })]);
    }

    #[test]
    fn all_voted_ends_early_and_votes_can_change() {
        let mut s = session(&[0, 5, 9]);
        s.handle_restart_game();
        assert!(s.cast(0, "KF-Farm", &mut never).unwrap().is_empty());
        assert_eq!(s.cast(0, "KF-Farm", &mut never), Err(Refused::SameVote));
        // Changes his vote: Farm loses it.
        assert!(s.cast(0, "KF-Manor", &mut never).unwrap().is_empty());
        assert_eq!(s.counts(), vec![0, 0, 1, 0, 0]);
        assert!(s.cast(5, "KF-Offices", &mut never).unwrap().is_empty());
        // Third vote: all voted, a 2-1 lead for Manor.
        let ev = s.cast(9, "KF-Manor", &mut never).unwrap();
        assert_eq!(ev, vec![VoteEvent::Finished(VoteResult { map: "KF-Manor".into(), reason: EndReason::AllVoted, tie: false })]);
    }

    #[test]
    fn majority_of_more_than_two_players_ends_it_after_the_game() {
        let mut s = session(&[1, 2, 3, 4, 5]);
        s.handle_restart_game();
        s.cast(1, "KF-Farm", &mut never).unwrap();
        s.cast(2, "KF-Farm", &mut never).unwrap();
        // 3 of 5 > half.
        let ev = s.cast(3, "KF-Farm", &mut never).unwrap();
        assert_eq!(ev, vec![VoteEvent::Finished(VoteResult { map: "KF-Farm".into(), reason: EndReason::Majority, tie: false })]);
        // Two players: 1 of 2 is not "more than half" and NumPlayers > 2 fails anyway.
        let mut s = session(&[1, 2]);
        s.handle_restart_game();
        assert!(s.cast(1, "KF-Farm", &mut never).unwrap().is_empty());
    }

    #[test]
    fn time_up_takes_the_leader_or_the_only_vote() {
        let mut s = session(&[0, 1, 2]);
        s.handle_restart_game();
        s.cast(1, "KF-Offices", &mut never).unwrap();
        s.scoreboard_time = -1;
        s.time_left = 1;
        let ev = s.tick(&mut never);
        assert_eq!(ev, vec![VoteEvent::Finished(VoteResult { map: "KF-Offices".into(), reason: EndReason::TimeUp, tie: false })]);
    }

    #[test]
    fn tie_is_random_among_the_tied_and_avoids_the_current_map() {
        let mut s = session(&[0, 1, 2, 3]);
        s.handle_restart_game();
        s.cast(0, "KF-WestLondon", &mut never).unwrap();
        s.cast(1, "KF-Farm", &mut never).unwrap();
        s.cast(2, "KF-BioticsLab", &mut never).unwrap();
        // Ranking (list order, all 1 vote): BioticsLab, Farm, WestLondon.
        // Time up: Rand(3) picks 2 = WestLondon, the current map; again:
        // 1 = Farm.
        s.scoreboard_time = -1;
        s.time_left = 1;
        let ev = s.tick(&mut seq(&[2, 1]));
        assert_eq!(ev, vec![VoteEvent::Finished(VoteResult { map: "KF-Farm".into(), reason: EndReason::TimeUp, tie: true })]);
    }

    #[test]
    fn tie_with_two_votes_each_after_all_voted() {
        let mut s = session(&[0, 1, 2, 3]);
        s.handle_restart_game();
        s.cast(0, "KF-Manor", &mut never).unwrap();
        s.cast(1, "KF-Farm", &mut never).unwrap();
        s.cast(2, "KF-Manor", &mut never).unwrap();
        let ev = s.cast(3, "KF-Farm", &mut seq(&[0])).unwrap();
        // Ranking after the sort: Farm (index 1) and Manor (2), 2 each;
        // Rand(2) = 0 -> Farm.
        assert_eq!(ev, vec![VoteEvent::Finished(VoteResult { map: "KF-Farm".into(), reason: EndReason::AllVoted, tie: true })]);
    }

    #[test]
    fn tie_gives_up_avoiding_after_100_tries() {
        // Two players, both tied maps... one is current: only the current
        // map can come out if Rand always says 0 and 0 is the current map.
        let mut s = VoteSession::new(on(), &maps(&["KF-A", "KF-B"]), &MapHistory::default(), "KF-A");
        s.set_players([0, 1], &mut never);
        s.handle_restart_game();
        s.cast(0, "KF-A", &mut never).unwrap();
        let mut calls = 0;
        let ev = s.cast(1, "KF-B", &mut |_| {
            calls += 1;
            0
        });
        assert_eq!(ev.unwrap(), vec![VoteEvent::Finished(VoteResult { map: "KF-A".into(), reason: EndReason::AllVoted, tie: true })]);
        assert_eq!(calls, 1 + 102);
    }

    #[test]
    fn refusals() {
        let mut h = MapHistory::default();
        h.play_map("KF-WestLondon");
        let mut s = VoteSession::new(on(), &maps(&["KF-Farm", "KF-WestLondon", "DM-Rankin"]), &h, "KF-WestLondon");
        s.set_players([0], &mut never);
        assert_eq!(s.cast(0, "KF-WestLondon", &mut never), Err(Refused::Disabled));
        assert_eq!(s.cast(0, "KF-Nowhere", &mut never), Err(Refused::UnknownMap));
        assert_eq!(s.cast(0, "DM-Rankin", &mut never), Err(Refused::WrongPrefix));
        assert_eq!(s.cast(7, "KF-Farm", &mut never), Err(Refused::NotAPlayer));
    }

    #[test]
    fn no_votes_random_map_skips_disabled_maps() {
        let mut h = MapHistory::default();
        h.play_map("KF-Farm");
        let mut s = VoteSession::new(on(), &maps(&LIST), &h, "KF-Farm");
        s.set_players([0, 1], &mut never);
        s.handle_restart_game();
        s.scoreboard_time = -1;
        s.time_left = 1;
        // Rand(5) = 1 is KF-Farm (disabled), then 3 = KF-Offices.
        let ev = s.tick(&mut seq(&[1, 3]));
        assert_eq!(ev, vec![VoteEvent::Finished(VoteResult { map: "KF-Offices".into(), reason: EndReason::NoVotes, tie: false })]);
    }

    #[test]
    fn time_up_with_no_map_that_can_win_ends_the_vote() {
        let mut h = MapHistory::default();
        h.play_map("KF-Farm");
        // The only map is disabled by RepeatLimit: no default map.
        let mut s = VoteSession::new(on(), &maps(&["KF-Farm"]), &h, "KF-Farm");
        s.set_players([0], &mut never);
        assert!(s.handle_restart_game());
        let mut events = Vec::new();
        for _ in 0..1000 {
            if !s.timer_running {
                break;
            }
            events.extend(s.tick(&mut |_| 0));
        }
        assert_eq!(events.last(), Some(&VoteEvent::NoWinner));
        assert!(!s.timer_running && s.view().result.is_none());
        // Nothing more happens afterwards.
        assert!(s.tick(&mut |_| 0).is_empty());
    }

    #[test]
    fn default_map_falls_back_to_the_first_match() {
        let mut h = MapHistory::default();
        h.play_map("KF-Farm");
        let s = VoteSession::new(on(), &maps(&["KF-Farm", "KF-Manor"]), &h, "KF-Farm");
        // Rand always lands on the disabled Farm: after 102 tries the
        // first enabled KF map.
        assert_eq!(s.default_map(&mut |_| 0), Some(1));
        let s = VoteSession::new(on(), &maps(&["KF-Farm"]), &h, "KF-Farm");
        assert_eq!(s.default_map(&mut |_| 0), None);
    }

    #[test]
    fn a_player_leaving_takes_the_vote_away_and_can_end_it() {
        let mut s = session(&[0, 1]);
        s.handle_restart_game();
        s.cast(0, "KF-Farm", &mut never).unwrap();
        assert_eq!(s.counts()[1], 1);
        // Player 1 (no vote) leaves: everyone left has voted.
        let ev = s.set_players([0], &mut never);
        assert_eq!(ev, vec![VoteEvent::Finished(VoteResult { map: "KF-Farm".into(), reason: EndReason::AllVoted, tie: false })]);
        let mut s = session(&[0, 1, 2]);
        s.handle_restart_game();
        s.cast(2, "KF-Farm", &mut never).unwrap();
        assert!(s.set_players([0, 1], &mut never).is_empty());
        assert_eq!(s.counts()[1], 0);
    }

    #[test]
    fn mid_game_vote_starts_the_countdown() {
        let mut s = session(&[1, 2, 3, 4]);
        // The game runs (no HandleRestartGame). 1 of 4 = 25% < 50.
        assert!(s.cast(1, "KF-Farm", &mut never).unwrap().is_empty());
        let ev = s.cast(2, "KF-Manor", &mut never).unwrap();
        assert_eq!(ev, vec![VoteEvent::MidGameStarted]);
        assert!(s.timer_running && s.mid_game);
        assert_eq!((s.time_left, s.scoreboard_time), (30, 1));
        // A majority does not end a mid-game vote (bGameEnded false)...
        assert!(s.cast(3, "KF-Farm", &mut never).unwrap().is_empty());
        // ...but everyone voting does.
        let ev = s.cast(4, "KF-Farm", &mut never).unwrap();
        assert_eq!(ev, vec![VoteEvent::Finished(VoteResult { map: "KF-Farm".into(), reason: EndReason::AllVoted, tie: false })]);
    }

    #[test]
    fn view_lists_counts_voters_and_time() {
        let mut s = session(&[0, 1, 2]);
        s.handle_restart_game();
        s.cast(0, "KF-Farm", &mut never).unwrap();
        s.cast(1, "KF-Manor", &mut never).unwrap();
        s.cast(2, "KF-Manor", &mut never).unwrap_or_default();
        let v = s.view();
        // 2 of 3 on Manor is a majority: finished.
        assert_eq!(v.result.as_ref().map(|r| r.map.as_str()), Some("KF-Manor"));
        assert_eq!(v.ranked().iter().map(|m| (m.name.as_str(), m.votes)).collect::<Vec<_>>(), vec![("KF-Manor", 2), ("KF-Farm", 1)]);
        assert_eq!(v.vote_of(0), Some("KF-Farm"));
        assert_eq!(v.players, 3);
        assert_eq!(won_text("KF-Manor"), "KF-Manor(KF) has won !");
        let mut s = session(&[0]);
        s.handle_restart_game();
        assert_eq!((s.view().opens_in, s.view().time_left, s.view().window_open), (6, 30, false));
    }

    #[test]
    fn xorshift_stays_in_range() {
        let mut r = XorShift(12345);
        for n in 1..50 {
            assert!(r.rand(n) < n);
        }
        assert_eq!(r.rand(0), 0);
    }
}
