//! Experimental multiplayer (branch `multiplayer-lightyear`; plan and
//! status in docs/multiplayer-prototype.md).
//!
//! `--host [PORT]` makes this game a listen server: it is the server and a
//! local player at once (lightyear's "host-client" mode). `--join
//! ADDR[:PORT]` connects to such a host. Without either option nothing in
//! this module runs and lightyear's plugins are not added, so a
//! single-player game is unchanged.
//!
//! Step 1 (this code): the pre-game lobby is shared. The server keeps one
//! `NetPlayer` record per connected player (KF's PlayerReplicationInfo)
//! and one `NetGame` record (KF's GameReplicationInfo), and copies them to
//! every client. Clients send their own choices (`LobbyRequest`). The
//! server starts the match with KF's rule (`server::PendingMatch`).
//! Step 2 (`pawns.rs`): every player's pawn is shared, so each game draws
//! the other players' bodies.
//! Step 3 (`zeds.rs`): one shared match. The host runs the zeds and the
//! waves; clients draw puppets of the host's zeds, follow its wave state,
//! report their hits on zeds, and take the hits the host's zeds deal them.
//! Shared zed time (`zedtime.rs`): the host decides it for everyone.
//!
//! For the next steps: every player has a peer id (`NetPlayer::peer`, 0 is
//! the host) and the server keeps `NetPlayers` (peer id -> the player's
//! record entity, whose `PlayerSlot` has room for the pawn entity).

mod client;
mod doors;
mod heals;
mod map_vote;
mod pickups;
pub mod query;
pub mod lobby;
pub mod pawns;
pub mod protocol;
mod server;
mod scoreboard;
pub mod starts;
mod zeds;
mod zedtime;

use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;

use bevy::prelude::*;

/// KF's game port (KillingFloor.ini `[URL] Port=7707`).
pub const DEFAULT_PORT: u16 = 7707;
/// KF's MaxPlayers (KillingFloor.ini `[Engine.GameInfo] MaxPlayers=6`).
pub const MAX_PLAYERS: usize = 6;
/// The netcode protocol number. Games built with a different number (a
/// different version of our network code) refuse to connect to each other.
/// Raise it whenever `protocol.rs` changes.
pub const PROTOCOL_ID: u64 = 0x4F4B_4600_000A;
/// netcode.io's 32-byte connection key. All zeros on purpose: this is a
/// LAN / direct-IP prototype with no access control (anyone who can reach
/// the port can join). It is not a secret and not a credential.
pub const NETCODE_KEY: [u8; 32] = [0; 32];
/// lightyear's tick. lightyear sets Bevy's fixed-step clock to it; 1/64 s
/// is Bevy's own default, so the physics (ragdolls) step as before.
pub const TICK: Duration = Duration::from_micros(15_625);

/// Is this a network game, and which side are we.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub enum NetMode {
    /// Single player (no network code runs).
    #[default]
    Off,
    /// `--host`: listen server on this UDP port, plus the local player.
    Host { port: u16 },
    /// `--join`: a client of this server.
    Client { server: SocketAddr },
}

impl NetMode {
    pub fn active(&self) -> bool {
        *self != NetMode::Off
    }

    /// `--host` takes an optional port.
    pub fn host(port: Option<u16>) -> Self {
        NetMode::Host { port: port.unwrap_or(DEFAULT_PORT) }
    }

    /// `--join ADDR[:PORT]`: an IP address or a host name, with KF's port
    /// when none is given.
    pub fn join(addr: &str) -> Result<Self, String> {
        let with_port = if addr.parse::<SocketAddr>().is_ok() || (addr.contains(':') && !addr.contains("::") && addr.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok())) {
            addr.to_string()
        } else {
            format!("{addr}:{DEFAULT_PORT}")
        };
        let server = with_port
            .to_socket_addrs()
            .map_err(|e| format!("cannot resolve --join address {addr}: {e}"))?
            .find(|a| a.is_ipv4())
            .ok_or(format!("--join address {addr} has no IPv4 address"))?;
        Ok(NetMode::Client { server })
    }

    pub fn label(&self) -> String {
        match self {
            NetMode::Off => "off".into(),
            NetMode::Host { port } => format!("host port={port}"),
            NetMode::Client { server } => format!("client server={server}"),
        }
    }
}

pub struct NetPlugin {
    pub mode: NetMode,
    /// What a host tells joining games on its query port (query.rs):
    /// the map, mode and length; the rest is filled in here. Unused
    /// unless hosting.
    pub info: query::HostInfo,
}

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        // Always there, so the menus can read them in single player too.
        // The host's own player is lightyear's peer Local(0): id 0.
        let my_peer = matches!(self.mode, NetMode::Host { .. }).then_some(0);
        app.insert_resource(self.mode.clone())
            .insert_resource(lobby::NetLobby { active: self.mode.active(), lobby_timeout: -1, my_peer, ..default() })
            .add_message::<lobby::StartLocalMatch>();
        match &self.mode {
            NetMode::Off => return,
            NetMode::Host { port } => {
                // Both halves in one game (lightyear's host-client mode).
                // The order matters: plugins, then the protocol, then
                // (at Startup) the Server / Client entities.
                app.add_plugins((lightyear::prelude::server::ServerPlugins { tick_duration: TICK }, lightyear::prelude::client::ClientPlugins { tick_duration: TICK }));
                protocol::register(app);
                server::build(app, *port);
                start_query(app, *port, &self.info);
            }
            NetMode::Client { server } => {
                app.add_plugins(lightyear::prelude::client::ClientPlugins { tick_duration: TICK });
                protocol::register(app);
                client::build(app, *server);
            }
        }
        // lightyear's prediction systems need this even before anything is
        // predicted (its examples insert it the same way); without it a
        // system fails with "LastConfirmedInput does not exist".
        app.add_systems(Startup, |mut commands: Commands| commands.insert_resource(lightyear::prelude::PredictionManager::default()));
        app.add_systems(First, keep_zed_time_speed.before(bevy::time::TimeSystems));
        lobby::build(app);
        pawns::build(app, &self.mode);
        zeds::build(app, &self.mode);
        starts::build(app, &self.mode);
        scoreboard::build(app);
        doors::build(app, &self.mode);
        zedtime::build(app, &self.mode);
        pickups::build(app, &self.mode);
        heals::build(app, &self.mode);
        map_vote::build(app, &self.mode);
        crate::engine::runlog::kv("net_mode", &self.mode.label());
    }
}

/// `--host`: answer host-info queries on the game port + 1 (query.rs),
/// from now on, so joiners get an answer while this game still loads.
fn start_query(app: &mut App, port: u16, info: &query::HostInfo) {
    let Some(qport) = query::query_port(port) else {
        crate::engine::runlog::kv("net_query_failed", &format!("side=host reason=no_query_port game_port={port}"));
        return;
    };
    let info = query::HostInfo { protocol: PROTOCOL_ID, game_port: port, max_players: MAX_PLAYERS as u32, players: 0, match_started: false, ..info.clone() };
    let bind = SocketAddr::new(std::net::Ipv4Addr::UNSPECIFIED.into(), qport);
    match query::start_responder(bind, info.clone()) {
        Ok((shared, local)) => {
            crate::engine::runlog::kv("net_query_listening", &format!("addr={local} map={} mode={} length={} difficulty={}", info.map, info.mode, info.length, info.difficulty));
            app.insert_resource(QueryInfo(shared)).add_systems(Update, update_query_info);
        }
        Err(e) => {
            // The game still works; joiners then have to give --map.
            crate::engine::runlog::kv("net_query_failed", &format!("side=host addr={bind} reason=\"{e}\""));
            eprintln!("warning: cannot answer join queries on UDP port {qport} ({e}); players joining must add --map {} --mode {}", info.map, info.mode.to_ascii_lowercase());
        }
    }
}

/// The host's answer to queries, shared with the answering thread.
#[derive(Resource)]
struct QueryInfo(query::SharedInfo);

/// Keeps the player count and "match started" in the query answer current.
fn update_query_info(q: Res<QueryInfo>, players: Res<server::NetPlayers>, game: Query<&protocol::NetGame>) {
    let n = players.0.len() as u32;
    let started = game.single().is_ok_and(|g| g.match_started);
    let mut info = q.0.lock().unwrap_or_else(|e| e.into_inner());
    if info.players != n || info.match_started != started {
        info.players = n;
        info.match_started = started;
    }
}

/// lightyear sets the speed of Bevy's game clock (`Time<Virtual>`) at the
/// end of every frame (its clock synchronisation), which undid zed time
/// (game/zed_time.rs slows that same clock to 0.2). Just before the clock
/// advances, multiply lightyear's speed by zed time's. The host decides zed
/// time for everyone (zedtime.rs), so every game's clock slows together.
fn keep_zed_time_speed(zt: Res<crate::game::zed_time::ZedTime>, mut virt: ResMut<Time<Virtual>>) {
    if zt.speed() != 1.0 {
        let s = virt.relative_speed() * zt.speed();
        virt.set_relative_speed(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_address_gets_kf_port_by_default() {
        assert_eq!(NetMode::join("127.0.0.1").unwrap(), NetMode::Client { server: "127.0.0.1:7707".parse().unwrap() });
        assert_eq!(NetMode::join("127.0.0.1:9000").unwrap(), NetMode::Client { server: "127.0.0.1:9000".parse().unwrap() });
        assert!(matches!(NetMode::join("localhost").unwrap(), NetMode::Client { server } if server.port() == 7707));
        assert_eq!(NetMode::host(None), NetMode::Host { port: 7707 });
    }
}
