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
//! Pawns and zeds are not shared yet: after the start every game plays on
//! its own.
//!
//! For the next steps: every player has a peer id (`NetPlayer::peer`, 0 is
//! the host) and the server keeps `NetPlayers` (peer id -> the player's
//! record entity, whose `PlayerSlot` has room for the pawn entity).

mod client;
pub mod lobby;
pub mod protocol;
mod server;

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
pub const PROTOCOL_ID: u64 = 0x4F4B_4600_0001;
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
        crate::engine::runlog::kv("net_mode", &self.mode.label());
    }
}

/// lightyear sets the speed of Bevy's game clock (`Time<Virtual>`) at the
/// end of every frame (its clock synchronisation), which undid zed time
/// (game/zed_time.rs slows that same clock to 0.2). Just before the clock
/// advances, multiply lightyear's speed by zed time's. Step 1 only: every
/// game has its own zed time; in KF the server decides it for everyone
/// (Level.TimeDilation), which is step 3's job.
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
