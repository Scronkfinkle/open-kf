//! Step 4: the host picks every player's start spot (KF's FindPlayerStart)
//! and brings dead players back at the end of a wave (KF's respawn rule).
//!
//! Host: `assign_starts` picks a PlayerStart for each player that needs one
//! (the match began, the player became ready late, the match restarted, or
//! the player is dead when a wave ends or the boss wave begins) and sends
//! it: a `PlayerStartMsg` to a client's game, a `MoveToStart` message to
//! its own player.
//!
//! Every game: `move_to_start` moves its player there once its own match
//! has started; with `respawn` a dead player comes back as a new pawn
//! (KFGameType wave end: Score = Max(MinRespawnCash, Score), then
//! ServerReStartPlayer: full health, the starting inventory).

use std::collections::{HashMap, HashSet};

use avian3d::prelude::*;
use bevy::prelude::*;
use lightyear::prelude::*;

use super::lobby::NetLobby;
use super::protocol::{GameChannel, NetGame, NetPawn, NetPlayer, PlayerStartMsg};
use super::server::PlayerSlot;
use super::NetMode;
use crate::engine::camera::FlyCamera;
use crate::engine::coords::SCALE;
use crate::engine::runlog;
use crate::game::waves::{Phase, WaveGame};
use crate::player::walk::Walker;
use crate::world::map::{PlayerStarts, SpawnPoint, StartSpot};

/// KFGameType.InitGame MinRespawnCash for the game's difficulty (Normal 200).
fn min_respawn_cash() -> f32 {
    crate::game::difficulty::current().min_respawn_cash()
}

/// The player's collision cylinder (KFPawn CollisionRadius, KFHumanPawn
/// CollisionHeight), Unreal units: RatePlayerStart's "inside a pawn" test.
const PAWN_RADIUS: f32 = crate::player::walk::kf::RADIUS;
const PAWN_HEIGHT: f32 = crate::player::walk::kf::HALF_HEIGHT;
/// The camera above a start spot (as the map loader's camera start).
const START_EYE_UP: f32 = 0.6;

/// Move this game's player to a start spot (from the host).
#[derive(Message, Clone, Debug)]
pub struct MoveToStart {
    pub msg: PlayerStartMsg,
}

pub(super) fn build(app: &mut App, mode: &NetMode) {
    use crate::world::map_change::MapResourceExt;
    // Per map (world/map_change.rs): a start not yet applied is the old
    // map's.
    app.add_message::<MoveToStart>().init_resource::<PendingStart>().reset_on_map_unload::<PendingStart>().add_systems(Update, dead_view);
    match mode {
        NetMode::Host { .. } => {
            // Who was placed, and each player's last start spot (an index
            // into the old map's starts), start again with the map.
            app.init_resource::<StartState>().reset_on_map_unload::<StartState>().add_systems(
                Update,
                (assign_starts.after(crate::game::waves::wave_timer).after(crate::game::dosh::DoshSystems), move_to_start).chain(),
            );
        }
        NetMode::Client { .. } => {
            app.add_systems(Update, (receive_starts, move_to_start.after(crate::game::waves::wave_timer).after(crate::game::dosh::DoshSystems)).chain());
        }
        NetMode::Off => {}
    }
}

/// The host's memory for FindPlayerStart and the respawns.
#[derive(Resource, Default)]
struct StartState {
    /// Players given a start in this match (cleared on a restart).
    placed: HashSet<u64>,
    /// Players to bring back (dead at a wave end or the boss wave start).
    respawn: HashSet<u64>,
    /// GameInfo LastStartSpot per player (index into the map's starts).
    last_start: HashMap<u64, usize>,
    rng: u32,
    seen_restarts: Option<u32>,
    seen_waves_ended: u32,
    seen_phase: Option<Phase>,
}

impl StartState {
    fn frand(&mut self) -> f32 {
        if self.rng == 0 {
            self.rng = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.subsec_nanos())) | 1;
        }
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// DeathMatch.RatePlayerStart (KFGameType reaches it through Invasion and
/// TeamGame for players: Team 0, bSpawnInTeamArea off). `others`: the
/// other players' pawns (Unreal units); `last`: this player's
/// LastStartSpot; `frand`: FRand; `trace`: FastTrace between two points;
/// `num_players`: NumPlayers + NumBots. Not done: the water-volume test and
/// the same-zone -1500 (no zone lookup for a start spot here).
fn rate_start(i: usize, s: &StartSpot, others: &[Vec3], last: Option<usize>, frand: f32, num_players: usize, trace: &dyn Fn(Vec3, Vec3) -> bool) -> f32 {
    if !s.enabled {
        return -10_000_000.0;
    }
    let mut score = if s.primary { 10_000_000.0 } else { 5_000_000.0 };
    if last == Some(i) {
        score -= 10_000.0;
    } else {
        score += 3000.0 * frand;
    }
    for &o in others {
        let d = (o - s.location).length();
        if d < PAWN_RADIUS + PAWN_HEIGHT {
            score -= 1_000_000.0;
        } else if d < 3000.0 && trace(s.location, o) {
            score -= 10_000.0 - d;
        } else if num_players == 2 {
            score += 2.0 * d;
            if trace(s.location, o) {
                score -= 10_000.0;
            }
        }
    }
    score.max(5.0)
}

/// Bevy metres -> Unreal units.
fn to_ue(v: Vec3) -> Vec3 {
    Vec3::new(-v.z, v.x, v.y) / SCALE
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)] // Bevy system parameters
fn assign_starts(
    mut state: ResMut<StartState>,
    starts: Res<PlayerStarts>,
    game: Res<WaveGame>,
    net_game: Query<&NetGame>,
    players: Query<(&NetPlayer, &PlayerSlot)>,
    pawns: Query<&NetPawn>,
    me: Query<(&Transform, Option<&Walker>), With<FlyCamera>>,
    health: Res<crate::game::combat::PlayerHealth>,
    spatial: SpatialQuery,
    mut local: MessageWriter<MoveToStart>,
    mut senders: Query<&mut MessageSender<PlayerStartMsg>, (With<lightyear::prelude::server::ClientOf>, With<Connected>)>,
) {
    if !net_game.iter().any(|g| g.match_started) || starts.0.is_empty() {
        return;
    }
    // Who is dead, and where everyone is (Unreal units).
    let mut dead: HashMap<u64, bool> = HashMap::new();
    let mut at: HashMap<u64, Vec3> = HashMap::new();
    for (p, slot) in &players {
        if slot.host {
            dead.insert(p.peer, health.dead);
            if let Ok((t, w)) = me.single() {
                at.insert(p.peer, to_ue(w.map_or(t.translation, |w| w.center)));
            }
        } else if let Some(np) = pawns.iter().find(|n| n.peer == p.peer) {
            dead.insert(p.peer, np.state.dead);
            if np.state.active {
                at.insert(p.peer, to_ue(Vec3::from_array(np.state.location)));
            }
        }
    }
    let mut reason = "match_start";
    // A new game (RestartGame reloads the map in KF: everyone starts again).
    if state.seen_restarts != Some(game.restarts) {
        if state.seen_restarts.is_some() {
            state.placed.clear();
            state.respawn.clear();
            reason = "restart";
        }
        state.seen_restarts = Some(game.restarts);
        state.seen_waves_ended = game.waves_ended;
    }
    // KFGameType wave end (DoWaveEnd): players without a pawn come back;
    // bRespawnOnBoss: also when the boss wave begins.
    let boss_start = game.phase == Phase::BossWave && state.seen_phase.is_some_and(|p| p != Phase::BossWave);
    if game.waves_ended != state.seen_waves_ended || boss_start {
        state.seen_waves_ended = game.waves_ended;
        let back: Vec<u64> = dead.iter().filter(|(_, d)| **d).map(|(p, _)| *p).collect();
        runlog::kv("net_respawn_check", &format!("event={} waves_ended={} dead={back:?}", if boss_start { "boss_wave_start" } else { "wave_end" }, game.waves_ended));
        for p in back {
            state.respawn.insert(p);
            state.placed.remove(&p);
        }
    }
    state.seen_phase = Some(game.phase);
    if matches!(game.phase, Phase::Won | Phase::Lost) {
        return;
    }
    let mut todo: Vec<(u64, String, Entity, bool)> = players.iter().filter(|(p, _)| p.ready && !state.placed.contains(&p.peer)).map(|(p, s)| (p.peer, p.name.clone(), s.link, s.host)).collect();
    if todo.is_empty() {
        return;
    }
    todo.sort_by_key(|t| t.0);
    let num_players = players.iter().count();
    let trace = |a: Vec3, b: Vec3| {
        let (from, to) = (crate::engine::coords::pos(a.to_array()), crate::engine::coords::pos(b.to_array()));
        let Ok(d) = Dir3::new(to - from) else { return true };
        spatial.cast_ray(from, d, (to - from).length(), true, &crate::world::collision::world_filter()).is_none()
    };
    for (peer, name, link, host) in todo {
        // The other pawns: players already placed (alive), at their spots.
        let others: Vec<Vec3> = at.iter().filter(|(p, _)| **p != peer && state.placed.contains(p) && !dead.get(p).copied().unwrap_or(false)).map(|(_, v)| *v).collect();
        let last = state.last_start.get(&peer).copied();
        let mut best: Option<(usize, f32)> = None;
        for (i, s) in starts.0.iter().enumerate() {
            let f = state.frand();
            let r = rate_start(i, s, &others, last, f, num_players, &trace);
            if best.is_none_or(|(_, b)| r > b) {
                best = Some((i, r));
            }
        }
        let Some((i, score)) = best else { continue };
        let s = starts.0[i];
        let respawn = state.respawn.remove(&peer);
        let msg = PlayerStartMsg { location: s.location.to_array(), yaw: s.yaw, respawn, reason: if respawn { "respawn".into() } else { reason.into() } };
        let sent = if host {
            local.write(MoveToStart { msg: msg.clone() });
            true
        } else {
            senders.get_mut(link).map(|mut tx| tx.send::<GameChannel>(msg.clone())).is_ok()
        };
        state.placed.insert(peer);
        state.last_start.insert(peer, i);
        at.insert(peer, s.location);
        dead.insert(peer, false);
        runlog::kv(
            "net_start_assigned",
            &format!(
                "peer={peer} name=\"{name}\" start={i} of={} at_unreal=({:.0},{:.0},{:.0}) yaw={} score={score:.0} respawn={respawn} reason={} others={} sent={sent}",
                starts.0.len(),
                s.location.x,
                s.location.y,
                s.location.z,
                s.yaw,
                msg.reason,
                others.len()
            ),
        );
    }
}

/// Dead in a network game: the view goes behind (the body is hidden, as in
/// single player) until the wave-end respawn. KF: the controller
/// spectates (state Spectating / Dead); our player just waits.
fn dead_view(health: Res<crate::game::combat::PlayerHealth>, mut view: ResMut<crate::engine::view_target::ViewTarget>, mut was: Local<bool>) {
    if health.dead != *was {
        *was = health.dead;
        if health.dead {
            view.set_behind_self(true);
        }
        runlog::kv("net_dead_state", &format!("dead={} deaths={} (no moving or firing until the wave ends)", health.dead, health.deaths));
    }
}

/// Client: the host's start spots for my player.
fn receive_starts(mut rx: Query<&mut MessageReceiver<PlayerStartMsg>, (With<Client>, Without<LinkOf>)>, mut out: MessageWriter<MoveToStart>) {
    for mut r in &mut rx {
        for msg in r.receive() {
            runlog::kv("net_start_received", &format!("{msg:?}"));
            out.write(MoveToStart { msg });
        }
    }
}

/// The newest start not yet applied (my match may not have started yet).
#[derive(Resource, Default)]
struct PendingStart(Option<PlayerStartMsg>);

/// Moves my player to the start spot (and with `respawn`, a dead player
/// comes back as a new pawn), once my own match has started.
#[allow(clippy::too_many_arguments, clippy::type_complexity)] // Bevy system parameters
fn move_to_start(
    mut incoming: MessageReader<MoveToStart>,
    mut pending: ResMut<PendingStart>,
    lobby: Res<NetLobby>,
    mut spawn: ResMut<SpawnPoint>,
    mut player: Query<(&mut Transform, &mut FlyCamera, Option<&mut Walker>)>,
    (mut health, mut dosh, mut armour, vet): (ResMut<crate::game::combat::PlayerHealth>, ResMut<crate::game::dosh::Dosh>, ResMut<crate::player::armour::Armour>, Res<crate::game::perks::Veterancy>),
    mut respawned: MessageWriter<crate::game::perks::RespawnPawn>,
    mut view: ResMut<crate::engine::view_target::ViewTarget>,
    game: Res<WaveGame>,
) {
    for m in incoming.read() {
        // A respawn is never replaced by a plain move before it is applied.
        let keep_respawn = pending.0.as_ref().is_some_and(|p| p.respawn);
        let mut msg = m.msg.clone();
        msg.respawn |= keep_respawn;
        pending.0 = Some(msg);
    }
    if !lobby.start_sent {
        return;
    }
    // A respawn waits until this game has seen the wave end too (a
    // client's wave state comes on another channel and can be a frame
    // late): a dead player gets no share of the wave's pot (dosh.rs).
    if pending.0.as_ref().is_some_and(|m| m.respawn) && game.phase == Phase::Wave {
        return;
    }
    let Some(msg) = pending.0.take() else { return };
    let Ok((mut t, mut cam, walker)) = player.single_mut() else {
        pending.0 = Some(msg);
        return;
    };
    let start = Vec3::from_array(msg.location);
    let eye = crate::engine::coords::pos(start.to_array()) + Vec3::Y * START_EYE_UP;
    let yaw_rad = msg.yaw as f32 * std::f32::consts::TAU / 65536.0;
    let forward = crate::engine::coords::dir([yaw_rad.cos(), yaw_rad.sin(), 0.0]);
    spawn.position = eye;
    spawn.forward = forward;
    t.translation = eye;
    // FlyCamera yaw: 0 faces -Z (Unreal +X); see camera.rs spawn_camera.
    cam.yaw = (-forward.x).atan2(-forward.z);
    cam.pitch = 0.0;
    t.rotation = Quat::from_euler(EulerRot::YXZ, cam.yaw, 0.0, 0.0);
    if let Some(mut w) = walker {
        w.center = eye - Vec3::Y * crate::player::walk::kf::EYE_HEIGHT * SCALE;
        w.velocity = Vec3::ZERO;
    }
    let mut revived = false;
    if msg.respawn && health.dead {
        // ServerReStartPlayer: a new pawn (full health, no armour unless
        // the perk gives it, the starting inventory) and at least
        // MinRespawnCash.
        health.dead = false;
        health.health = 100.0;
        health.to_give = 0.0;
        *armour = crate::player::armour::Armour::default();
        let before = dosh.score;
        dosh.score = dosh.score.max(min_respawn_cash());
        respawned.write(crate::game::perks::RespawnPawn);
        view.set_behind_self(false);
        revived = true;
        runlog::kv("net_respawned", &format!("health={} dosh={before:.0}->{:.0} perk={}", health.health, dosh.score, vet.vet.label()));
    }
    runlog::kv(
        "net_moved_to_start",
        &format!("reason={} respawn={} revived={revived} at_unreal=({:.0},{:.0},{:.0}) yaw={} eye_bevy={eye}", msg.reason, msg.respawn, start.x, start.y, start.z, msg.yaw),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(x: f32) -> StartSpot {
        StartSpot { location: Vec3::new(x, 0.0, 0.0), yaw: 0, enabled: true, primary: true }
    }

    #[test]
    fn a_start_inside_another_pawn_loses() {
        let clear = |_: Vec3, _: Vec3| true;
        let a = rate_start(0, &spot(0.0), &[Vec3::ZERO], None, 0.0, 2, &clear);
        let b = rate_start(1, &spot(100.0), &[Vec3::ZERO], None, 0.0, 2, &clear);
        assert!(b > a, "{a} {b}");
        // Disabled starts are never picked over enabled ones.
        let off = StartSpot { enabled: false, ..spot(5000.0) };
        assert!(rate_start(2, &off, &[], None, 1.0, 1, &clear) < rate_start(0, &spot(0.0), &[], None, 0.0, 1, &clear));
        // LastStartSpot: -10000 instead of the random bonus.
        assert_eq!(rate_start(0, &spot(0.0), &[], Some(0), 1.0, 1, &clear), 10_000_000.0 - 10_000.0);
    }
}
