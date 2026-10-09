//! Map voting (KF's xVoting, "majority mode"): at the end of a match the
//! players vote for the next map. The rules are in `rules.rs` (no Bevy);
//! this file runs them in the game: the once-a-second timer, the players,
//! the local player's votes, the log, and the messages in and out. The
//! window is game/menus/map_vote_page.rs; the network messages are
//! net/map_vote.rs.
//!
//! Off by default, as in KF (bMapVote=False); KF never votes in a
//! single-player game, and neither do we unless `MapVoteSettings`
//! says so (`single_player`, set by `--vote-test`).
//!
//! **The map change** (game/travel.rs) uses these hooks:
//! - `MapVoteSettings` (a `VoteConfig`): KF's settings; `enabled` and
//!   `time_limit` come from the launcher / `--map-vote` (main.rs).
//! - Write `StartMapVote { candidates, current_map }` where KF calls
//!   HandleRestartGame (the end screen's 14 s, or the host's Fire after
//!   5 s). It returns nothing; if voting is off it logs
//!   `map_vote_skipped` and does nothing, so check `MapVote::holds_travel`
//!   the next frame (true while a vote runs or has a winner) and go to the
//!   next map as usual when it is false.
//! - Read `MapVoteFinished { map, reason }` (host / single player only):
//!   travel to `map`. Then call `MapVote::clear()` once the new map is
//!   loading (the window closes, the history keeps the winner).
//! - The host's players come from the network records (`NetPlayer`), the
//!   single player is peer 0; clients only show the host's state.
//! - Mid-game voting is in the rules but not wired (nothing opens the
//!   window during a match).

pub mod rules;

use bevy::prelude::*;

use crate::engine::runlog;
use crate::net::NetMode;
use rules::{MapHistory, VoteEvent, VoteSession, VoteView, XorShift};
pub use rules::{EndReason, VoteConfig};

/// The vote settings (KF's defaults; off). The one copy the game reads:
/// main.rs inserts it from the settings file and `--map-vote` /
/// `--vote-time` (`map_rotation::MapVoteConfig::settings`).
pub type MapVoteSettings = VoteConfig;

/// Start a vote (KF: GameInfo.RestartGame -> VotingHandler
/// HandleRestartGame). Only the host or the single player starts one.
#[derive(Message, Clone, Debug)]
pub struct StartMapVote {
    /// The maps to vote for (KF's DefaultMapListLoader: every map with the
    /// game's prefix in the Maps folder).
    pub candidates: Vec<String>,
    /// The map being played.
    pub current_map: String,
}

/// The local player votes for a map (the window's Submit, or a test).
#[derive(Message, Clone, Debug)]
pub struct CastMapVote {
    pub map: String,
}

/// A map won (host / single player). game/travel.rs travels to it.
#[derive(Message, Clone, Debug)]
pub struct MapVoteFinished {
    pub map: String,
    pub reason: EndReason,
}

/// Which side of the vote this game is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum VoteRole {
    /// Single player: runs the vote, the only voter is peer 0.
    #[default]
    Local,
    /// `--host`: runs the vote for everyone.
    Host,
    /// `--join`: shows the host's vote, sends its own votes there.
    Client,
}

/// How long the window stays up after a map has won, showing "<map> has
/// won !" (ours: KF closes the windows at once and prints the line in the
/// message area while the server travels).
pub const WON_SHOW_SECS: f32 = 3.0;

/// The vote as this game knows it.
#[derive(Resource, Default)]
pub struct MapVote {
    pub role: VoteRole,
    /// The running vote (host / single player).
    pub session: Option<VoteSession>,
    /// What the window shows: built from `session`, or received from the
    /// host (client).
    pub view: Option<VoteView>,
    /// The maps won by votes this run (KF's MapVoteHistory).
    pub history: MapHistory,
    /// The view changed since the network last sent it (host).
    pub changed: bool,
    /// Votes from other players, for the host: (peer, map).
    pub incoming: Vec<(u64, String)>,
    /// My votes for the host (client).
    pub outgoing: Vec<String>,
    /// The player closed the window (Close); it opens again with the
    /// next vote, or `reopen`.
    pub closed_by_player: bool,
    /// Real seconds when the winner was known (the window closes
    /// `WON_SHOW_SECS` later).
    pub won_at: Option<f32>,
    rng: Option<XorShift>,
    /// Real seconds since the last timer tick.
    acc: f32,
    /// The winner was announced on this machine.
    announced: bool,
}

impl MapVote {
    /// A vote is running or has a winner: the game must not travel by
    /// itself (KF: HandleRestartGame returned false).
    pub fn holds_travel(&self) -> bool {
        self.session.is_some()
    }

    /// Ends the vote's life (after the map change): the window closes.
    pub fn clear(&mut self) {
        self.session = None;
        self.view = None;
        self.changed = true;
        self.closed_by_player = false;
        self.won_at = None;
        self.announced = false;
    }

    /// Should the vote window be on screen now?
    pub fn window_wanted(&self, now: f32) -> bool {
        let Some(v) = &self.view else { return false };
        if !v.window_open || self.closed_by_player {
            return false;
        }
        self.won_at.is_none_or(|t| now - t < WON_SHOW_SECS)
    }
}

/// `--vote-test N`: N frames in, start a vote with the installed maps
/// (single player allowed), to test the window and the rules headless.
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct MapVoteTest(pub Option<u32>);

pub struct MapVotePlugin;

/// The vote systems (the network ones run around them).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct MapVoteSystems;

impl Plugin for MapVotePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapVote>()
            .init_resource::<MapVoteSettings>()
            .init_resource::<MapVoteTest>()
            .add_message::<StartMapVote>()
            .add_message::<CastMapVote>()
            .add_message::<MapVoteFinished>()
            .add_systems(Startup, setup)
            .add_systems(Update, (vote_test, start_vote, follow_players, cast_votes, tick, publish, announce).chain().in_set(MapVoteSystems));
    }
}

fn setup(mut vote: ResMut<MapVote>, mode: Option<Res<NetMode>>, mut settings: ResMut<MapVoteSettings>, test: Res<MapVoteTest>) {
    vote.role = match mode.as_deref() {
        Some(NetMode::Host { .. }) => VoteRole::Host,
        Some(NetMode::Client { .. }) => VoteRole::Client,
        _ => VoteRole::Local,
    };
    vote.rng = Some(XorShift::from_time());
    if test.0.is_some() {
        settings.enabled = true;
        settings.single_player = true;
    }
    runlog::kv(
        "map_vote_settings",
        &format!(
            "role={:?} enabled={} single_player={} auto_open={} time_limit={} scoreboard_delay={} repeat_limit={} mid_game_percent={} test_frame={:?}",
            vote.role, settings.enabled, settings.single_player, settings.auto_open, settings.time_limit, settings.scoreboard_delay, settings.repeat_limit, settings.mid_game_vote_percent, test.0
        ),
    );
}

/// `--vote-test N`: the vote starts at frame N with the installed maps.
/// The current map is put in the history first (as if a vote had chosen
/// it), so RepeatLimit's disabled map shows.
fn vote_test(test: Res<MapVoteTest>, frames: Res<bevy::diagnostic::FrameCount>, request: Res<crate::world::map::MapRequest>, mut vote: ResMut<MapVote>, mut start: MessageWriter<StartMapVote>) {
    if test.0 != Some(frames.0) || vote.role == VoteRole::Client {
        return;
    }
    vote.history.play_map(&request.map);
    runlog::kv("map_vote_test", &format!("frame={} history_seeded={}", frames.0, request.map));
    start.write(StartMapVote { candidates: crate::game::map_rotation::list_installed_maps(&request.install_root), current_map: request.map.clone() });
}

/// A rule's random number.
fn rand(rng: &mut Option<XorShift>) -> impl FnMut(usize) -> usize + '_ {
    move |n| rng.get_or_insert_with(XorShift::from_time).rand(n)
}

fn start_vote(mut starts: MessageReader<StartMapVote>, settings: Res<MapVoteSettings>, mut vote: ResMut<MapVote>) {
    for s in starts.read() {
        if vote.role == VoteRole::Client {
            runlog::kv("map_vote_skipped", "reason=client (the host runs the vote)");
            continue;
        }
        if vote.role == VoteRole::Local && !settings.single_player {
            // KF: "disable voting in single player mode".
            runlog::kv("map_vote_skipped", "reason=single_player");
            continue;
        }
        let mut session = VoteSession::new(settings.clone(), &s.candidates, &vote.history, &s.current_map);
        if !session.handle_restart_game() {
            runlog::kv("map_vote_skipped", &format!("reason={}", if settings.enabled { "auto_open_off" } else { "disabled" }));
            continue;
        }
        let disabled: Vec<&str> = session.maps.iter().filter(|m| !m.enabled).map(|m| m.name.as_str()).collect();
        runlog::kv(
            "map_vote_start",
            &format!("maps={} disabled=[{}] current={} time_limit={} scoreboard_delay={}", session.maps.len(), disabled.join(","), s.current_map, session.time_left, session.scoreboard_time),
        );
        vote.session = Some(session);
        runlog::kv("map_vote_holds_travel", &format!("holds_travel={}", vote.holds_travel()));
        vote.closed_by_player = false;
        vote.won_at = None;
        vote.announced = false;
        vote.acc = 0.0;
        vote.changed = true;
    }
}

/// The players who vote: every connected player (host), or peer 0.
fn follow_players(mut vote: ResMut<MapVote>, players: Query<&crate::net::protocol::NetPlayer>) {
    let vote = &mut *vote;
    let Some(session) = vote.session.as_mut() else { return };
    let peers: Vec<u64> = match vote.role {
        VoteRole::Host => players.iter().map(|p| p.peer).collect(),
        _ => vec![0],
    };
    let before = session.players().len();
    let events = session.set_players(peers, &mut rand(&mut vote.rng));
    if session.players().len() != before {
        runlog::kv("map_vote_players", &format!("players={}", session.players().len()));
        vote.changed = true;
    }
    handle(events, vote);
}

/// Votes: mine (the window) and, on the host, the other players'.
fn cast_votes(mut casts: MessageReader<CastMapVote>, mut vote: ResMut<MapVote>, lobby: Res<crate::net::lobby::NetLobby>) {
    let vote = &mut *vote;
    let me = lobby.my_peer.unwrap_or(0);
    let mut all: Vec<(u64, String)> = std::mem::take(&mut vote.incoming);
    for c in casts.read() {
        if vote.role == VoteRole::Client {
            runlog::kv("map_vote_request", &format!("peer={me} map={}", c.map));
            vote.outgoing.push(c.map.clone());
        } else {
            all.push((me, c.map.clone()));
        }
    }
    for (peer, map) in all {
        let Some(session) = vote.session.as_mut() else {
            runlog::kv("map_vote_refused", &format!("peer={peer} map={map} reason=no_vote"));
            continue;
        };
        let outcome = session.cast(peer, &map, &mut rand(&mut vote.rng));
        match outcome {
            Ok(events) => {
                let counts = session.counts();
                let line: Vec<String> = session.maps.iter().zip(counts).filter(|(_, c)| *c > 0).map(|(m, c)| format!("{}:{c}", m.name)).collect();
                // lmsgMapVotedFor: "%playername% has voted for %mapname%".
                runlog::kv("map_vote_cast", &format!("peer={peer} map={map} counts=[{}] voted={}/{}", line.join(","), session.votes().count(), session.players().len()));
                vote.changed = true;
                handle(events, vote);
            }
            Err(r) => runlog::kv("map_vote_refused", &format!("peer={peer} map={map} reason={}", r.word())),
        }
    }
}

/// The once-a-second timer (real time: a paused game still counts down,
/// as KF's VotingPage unpauses the game).
fn tick(time: Res<Time<Real>>, mut vote: ResMut<MapVote>) {
    let vote = &mut *vote;
    let Some(session) = vote.session.as_mut() else { return };
    if !session.timer_running {
        vote.acc = 0.0;
        return;
    }
    vote.acc += time.delta_secs();
    while vote.acc >= 1.0 {
        vote.acc -= 1.0;
        let Some(session) = vote.session.as_mut() else { return };
        let events = session.tick(&mut rand(&mut vote.rng));
        vote.changed = true;
        handle(events, vote);
    }
}

fn handle(events: Vec<VoteEvent>, vote: &mut MapVote) {
    for e in events {
        match e {
            VoteEvent::OpenWindows => runlog::kv("map_vote_open", &format!("time_left={}", vote.session.as_ref().map_or(0, |s| s.time_left))),
            VoteEvent::CountDown(n) => runlog::kv("map_vote_countdown", &format!("time_left={n}")),
            VoteEvent::MidGameStarted => runlog::kv("map_vote_mid_game", "event=started"),
            VoteEvent::Finished(r) => {
                // History.PlayMap at the win.
                vote.history.play_map(&r.map);
                runlog::kv("map_vote_end", &format!("winner={} reason={} tie={}", r.map, r.reason.word(), r.tie));
            }
        }
    }
}

/// The session -> the window's view (host / single player).
fn publish(mut vote: ResMut<MapVote>) {
    if vote.role == VoteRole::Client || !vote.changed {
        return;
    }
    let view = vote.session.as_ref().map(|s| s.view());
    if vote.view != view {
        vote.view = view;
    }
}

/// When a map has won: the message on every player's screen, the window
/// closes a moment later, and (host / single player) `MapVoteFinished`.
fn announce(time: Res<Time<Real>>, mut vote: ResMut<MapVote>, mut finished: MessageWriter<MapVoteFinished>, mut messages: MessageWriter<crate::game::hud::LocalMessage>) {
    let Some(result) = vote.view.as_ref().and_then(|v| v.result.clone()) else { return };
    if vote.announced {
        return;
    }
    vote.announced = true;
    vote.won_at = Some(time.elapsed_secs());
    messages.write(crate::game::hud::LocalMessage::text(rules::won_text(&result.map)));
    runlog::kv("map_vote_won", &format!("text=\"{}\" role={:?}", rules::won_text(&result.map), vote.role));
    if vote.role != VoteRole::Client {
        finished.write(MapVoteFinished { map: result.map.clone(), reason: result.reason });
    }
}
