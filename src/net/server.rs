//! The host's side: a UDP server plus the host's own player (lightyear's
//! host-client mode: the host is a client of its own server, inside the
//! same game, without sending anything over the network to itself).

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};

use bevy::prelude::*;
use lightyear::connection::host::HostClient;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

use super::lobby::NetLobby;
use super::protocol::{LobbyRequest, NetGame, NetPlayer};
use super::{MAX_PLAYERS, NETCODE_KEY, PROTOCOL_ID};
use crate::engine::runlog;

/// KillingFloor.ini `[UnrealGame.DeathMatch]` / `[KFmod.KFGameType]`.
const NET_WAIT: u32 = 5;
const MIN_NET_PLAYERS: usize = 1;
const LOBBY_TIMEOUT: i32 = 20;

/// Server-only part of a player's record: which network link it belongs
/// to, and (step 2) its pawn.
#[derive(Component, Debug)]
pub struct PlayerSlot {
    pub link: Entity,
    pub host: bool,
    /// The player's `NetPawn` entity (made with their first pawn update,
    /// net/pawns.rs).
    pub pawn: Option<Entity>,
}

/// Peer id -> the player's `NetPlayer` entity (server only).
#[derive(Resource, Default, Debug)]
pub struct NetPlayers(pub HashMap<u64, Entity>);

/// The server entity (for stopping it on quit).
#[derive(Resource)]
pub(super) struct ServerEntity(pub Entity);

pub(super) fn build(app: &mut App, port: u16) {
    app.init_resource::<NetPlayers>()
        .insert_resource(PendingMatchTimer { port, ..default() })
        .add_systems(Startup, start_server)
        .add_observer(on_new_link)
        .add_observer(on_connected)
        .add_observer(on_server_started)
        .add_systems(Update, (receive_requests, apply_host_request, drop_left_players, pending_match).chain().before(super::lobby::LobbySystems));
}

fn start_server(mut commands: Commands, mode: Res<super::NetMode>, map: Res<crate::world::map::MapRequest>, options: Res<crate::game::waves::GameOptions>) {
    let super::NetMode::Host { port } = *mode else { return };
    let addr = SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), port);
    let server = commands
        .spawn((
            Name::new("Server"),
            Server::default(),
            NetcodeServer::new(NetcodeConfig {
                protocol_id: PROTOCOL_ID,
                private_key: NETCODE_KEY,
                // The host is the sixth player.
                max_clients: MAX_PLAYERS - 1,
                // Clients name the server by whatever address reaches it
                // (127.0.0.1, a LAN address); the socket listens on all.
                server_addr_check: false,
                ..default()
            }),
            LocalAddr(addr),
            ServerUdpIo::default(),
        ))
        .id();
    commands.trigger(Start { entity: server });
    commands.insert_resource(ServerEntity(server));
    // The host's own player: a client linked to the server in the same game.
    let host = commands.spawn((Name::new("HostClient"), Client, LinkOf { server })).id();
    commands.trigger(Connect { entity: host });
    // KF's GameReplicationInfo.
    commands.spawn((
        Name::new("NetGame"),
        NetGame { map: map.map.clone(), mode: format!("{:?}", options.mode), length: format!("{:?}", options.length), match_started: false, lobby_timeout: -1 },
        Replicate::to_clients(NetworkTarget::All),
    ));
    runlog::kv("net_server_starting", &format!("addr={addr} map={} max_players={MAX_PLAYERS} protocol={PROTOCOL_ID:#x}", map.map));
}

fn on_server_started(trigger: On<Add, Started>, timer: Res<PendingMatchTimer>) {
    runlog::kv("net_server_started", &format!("entity={:?} port={}", trigger.entity, timer.port));
}

/// A new connection attempt (also the host's own client): it may receive
/// replicated records.
fn on_new_link(trigger: On<Add, LinkOf>, mut commands: Commands) {
    commands.entity(trigger.entity).insert(ReplicationSender);
}

/// A player is connected: make their record (KF's Login / PostLogin
/// spawn the PlayerReplicationInfo).
fn on_connected(trigger: On<Add, Connected>, links: Query<(&RemoteId, Has<HostClient>), With<ClientOf>>, mut commands: Commands, mut players: ResMut<NetPlayers>) {
    let Ok((remote, host)) = links.get(trigger.entity) else { return };
    let peer = remote.0.to_bits();
    if players.0.len() >= MAX_PLAYERS {
        runlog::kv("net_player_refused", &format!("peer={peer} reason=server_full"));
        commands.trigger(Disconnect { entity: trigger.entity });
        return;
    }
    let e = commands
        .spawn((
            Name::new(format!("NetPlayer {peer}")),
            NetPlayer { peer, name: "Player".into(), perk: None, level: 0, ready: false, character: String::new() },
            PlayerSlot { link: trigger.entity, host, pawn: None },
            Replicate::to_clients(NetworkTarget::All),
        ))
        .id();
    players.0.insert(peer, e);
    runlog::kv("net_player_joined", &format!("peer={peer} host={host} link={:?} players={}", trigger.entity, players.0.len()));
}

/// Clients' lobby requests (KF: the server side of
/// SendSelectedVeterancyToServer, ServerRestartPlayer, ServerUnreadyPlayer).
fn receive_requests(mut links: Query<(Entity, &mut MessageReceiver<LobbyRequest>), With<ClientOf>>, mut players: Query<(&mut NetPlayer, &PlayerSlot)>, game: Query<&NetGame>) {
    let started = game.iter().any(|g| g.match_started);
    for (link, mut rx) in &mut links {
        for req in rx.receive() {
            let Some((mut p, _)) = players.iter_mut().find(|(_, s)| s.link == link) else {
                runlog::kv("net_request_dropped", &format!("link={link:?} reason=no_player"));
                continue;
            };
            apply_logged(&mut p, &req, started, "client");
        }
    }
}

fn apply_logged(p: &mut Mut<NetPlayer>, req: &LobbyRequest, started: bool, from: &str) {
    // Only touch the record when something changed (a write would send it again).
    let mut copy = (**p).clone();
    let changed = copy.apply(req, started);
    if !changed.is_empty() {
        **p = copy;
        runlog::kv("net_lobby_update", &format!("peer={} from={from} {}", p.peer, changed.join(" ")));
    }
}

/// The host's own choices go straight into its record (no message needed:
/// the server is in this game).
fn apply_host_request(lobby: Res<NetLobby>, mut players: Query<(&mut NetPlayer, &PlayerSlot)>, game: Query<&NetGame>) {
    let Some(req) = lobby.local.as_ref() else { return };
    let started = game.iter().any(|g| g.match_started);
    if let Some((mut p, _)) = players.iter_mut().find(|(_, s)| s.host) {
        apply_logged(&mut p, req, started, "host");
    }
}

/// A player left (their link reports Disconnected, or the link entity is
/// gone): remove their record (KF: Logout destroys the PRI). Checked every
/// frame rather than by an observer so it also catches links that are
/// removed without a Disconnected state.
fn drop_left_players(mut commands: Commands, players_q: Query<(Entity, &NetPlayer, &PlayerSlot)>, links: Query<(Has<Connected>, Option<&Disconnected>)>, mut players: ResMut<NetPlayers>) {
    for (e, p, slot) in &players_q {
        let reason = match links.get(slot.link) {
            Err(_) => "link_removed".to_string(),
            Ok((false, Some(d))) => format!("{:?}", d.reason).replace(' ', "_"),
            Ok(_) => continue,
        };
        commands.entity(e).despawn();
        // KF: Logout destroys the pawn too; every game removes its body.
        if let Some(pawn) = slot.pawn {
            commands.entity(pawn).despawn();
        }
        players.0.remove(&p.peer);
        runlog::kv("net_player_left", &format!("peer={} name=\"{}\" reason={reason} players={}", p.peer, p.name, players.0.len()));
    }
}

/// KFGameType state PendingMatch, Timer() (once a second): when the match
/// starts. Values from KillingFloor.ini (NetWait 5, MinNetPlayers 1,
/// LobbyTimeout 20, MaxPlayers 6).
#[derive(Debug, Clone, PartialEq)]
pub struct PendingMatch {
    pub elapsed: u32,
    pub wait_for_net_players: bool,
    pub lobby_timeout: i32,
}

impl Default for PendingMatch {
    fn default() -> Self {
        // bWaitForNetPlayers=True in the ini; LobbyTimeout=20.
        PendingMatch { elapsed: 0, wait_for_net_players: true, lobby_timeout: LOBBY_TIMEOUT }
    }
}

/// What one Timer() call decided.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TimerResult {
    /// StartMatch().
    pub start: bool,
    /// The countdown ran out: every player is made ready.
    pub force_ready: bool,
    /// New KFGRI.LobbyTimeout (None: left as it was, KF returns early).
    pub gri_lobby_timeout: Option<i32>,
}

impl PendingMatch {
    /// One Timer() call; `ready` has one entry per player (bReadyToPlay).
    pub fn timer(&mut self, ready: &[bool]) -> TimerResult {
        let num = ready.len();
        let mut out = TimerResult::default();
        if num == 0 {
            self.wait_for_net_players = true;
        }
        if self.wait_for_net_players {
            if num >= MIN_NET_PLAYERS {
                self.elapsed += 1;
            } else {
                self.elapsed = 0;
            }
            if num == MAX_PLAYERS || self.elapsed > NET_WAIT {
                self.wait_for_net_players = false;
            }
        }
        if self.wait_for_net_players {
            return out;
        }
        let ready_count = ready.iter().filter(|r| **r).count();
        if num > 0 && ready_count == num {
            out.start = true;
        }
        if num > 2 {
            self.elapsed += 1;
        }
        if (ready_count as f32 >= num as f32 * 0.65 || self.elapsed > 300) && num > 2 && self.lobby_timeout > 0 {
            if self.lobby_timeout <= 1 {
                out.force_ready = true;
                self.lobby_timeout = 0;
            } else {
                self.lobby_timeout -= 1;
            }
            out.gri_lobby_timeout = Some(self.lobby_timeout);
        } else {
            out.gri_lobby_timeout = Some(-1);
        }
        out
    }
}

#[derive(Resource, Default)]
struct PendingMatchTimer {
    port: u16,
    state: PendingMatch,
    /// Real seconds since the last Timer() call.
    acc: f32,
}

fn pending_match(time: Res<Time<Real>>, mut timer: ResMut<PendingMatchTimer>, mut players: Query<&mut NetPlayer>, mut game: Query<&mut NetGame>) {
    let Ok(mut g) = game.single_mut() else { return };
    if g.match_started {
        return;
    }
    timer.acc += time.delta_secs();
    if timer.acc < 1.0 {
        return;
    }
    timer.acc -= 1.0;
    let ready: Vec<bool> = players.iter().map(|p| p.ready).collect();
    let before = timer.state.clone();
    let r = timer.state.timer(&ready);
    if before.wait_for_net_players && !timer.state.wait_for_net_players {
        runlog::kv("net_wait_over", &format!("elapsed={} players={}", timer.state.elapsed, ready.len()));
    }
    if r.force_ready {
        for mut p in &mut players {
            if !p.ready {
                p.ready = true;
            }
        }
        runlog::kv("net_lobby_timeout", "event=everyone_made_ready");
    }
    if let Some(t) = r.gri_lobby_timeout
        && g.lobby_timeout != t
    {
        g.lobby_timeout = t;
        runlog::kv("net_lobby_countdown", &format!("lobby_timeout={t}"));
    }
    if r.start {
        // StartMatch; EndState sets GRI.LobbyTimeout = -1.
        g.match_started = true;
        g.lobby_timeout = -1;
        runlog::kv("net_match_start", &format!("players={} elapsed={}", ready.len(), timer.state.elapsed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_net_wait_seconds_then_starts_when_all_ready() {
        let mut m = PendingMatch::default();
        // Ticks 1-5: elapsed 1..5, still waiting (KF: ElapsedTime > NetWait).
        for _ in 0..5 {
            assert_eq!(m.timer(&[true, true]), TimerResult::default());
        }
        // Tick 6: elapsed 6 > 5: waiting is over, all ready -> start.
        let r = m.timer(&[true, true]);
        assert!(r.start);
        assert_eq!(r.gri_lobby_timeout, Some(-1));
    }

    #[test]
    fn two_players_no_countdown_one_not_ready() {
        let mut m = PendingMatch { wait_for_net_players: false, ..default() };
        for _ in 0..400 {
            let r = m.timer(&[true, false]);
            assert!(!r.start && !r.force_ready);
            assert_eq!(r.gri_lobby_timeout, Some(-1));
        }
    }

    #[test]
    fn three_players_two_ready_counts_down_from_20() {
        let mut m = PendingMatch { wait_for_net_players: false, ..default() };
        // 2 of 3 ready: 2 >= 3 x 0.65 = 1.95.
        let r = m.timer(&[true, true, false]);
        assert_eq!(r.gri_lobby_timeout, Some(19));
        for left in (1..19).rev() {
            assert_eq!(m.timer(&[true, true, false]).gri_lobby_timeout, Some(left));
        }
        let r = m.timer(&[true, true, false]);
        assert!(r.force_ready && !r.start);
        assert_eq!(r.gri_lobby_timeout, Some(0));
        // Next second everyone is ready: start.
        assert!(m.timer(&[true, true, true]).start);
    }

    #[test]
    fn full_server_skips_the_net_wait() {
        let mut m = PendingMatch::default();
        assert!(m.timer(&[true; 6]).start);
    }
}
