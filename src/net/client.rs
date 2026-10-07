//! A joining player's side: connect to the host over UDP, send my lobby
//! choices, check the host plays the map this game loaded, leave.

use std::net::{Ipv4Addr, SocketAddr};

use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;

use super::lobby::NetLobby;
use super::protocol::{LobbyChannel, LobbyRequest, NetGame};
use super::{NETCODE_KEY, PROTOCOL_ID};
use crate::engine::runlog;

/// Give up when not connected after this many seconds.
const CONNECT_TIMEOUT: f32 = 15.0;

pub(super) fn build(app: &mut App, server: SocketAddr) {
    app.insert_resource(ClientLink { server, ..default() })
        .add_systems(Startup, connect)
        .add_systems(Update, (watch_connection, check_game, send_request).chain().before(super::lobby::LobbySystems));
}

#[derive(Resource)]
struct ClientLink {
    server: SocketAddr,
    id: u64,
    connected: bool,
    was_connected: bool,
    waited: f32,
    game_checked: bool,
}

impl Default for ClientLink {
    fn default() -> Self {
        ClientLink { server: SocketAddr::new(Ipv4Addr::LOCALHOST.into(), super::DEFAULT_PORT), id: 0, connected: false, was_connected: false, waited: 0.0, game_checked: false }
    }
}

/// A client id for netcode: any number that is not 0 (0 is the host) and
/// unlikely to be taken by another player (time and process id mixed).
fn new_client_id() -> u64 {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let mut x = nanos ^ ((std::process::id() as u64) << 32);
    // splitmix64 finaliser, to spread the bits.
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^= x >> 31;
    x.max(1)
}

fn connect(mut commands: Commands, mut link: ResMut<ClientLink>, mut exit: MessageWriter<AppExit>) {
    link.id = new_client_id();
    let auth = Authentication::Manual { server_addr: link.server, client_id: link.id, private_key: NETCODE_KEY, protocol_id: PROTOCOL_ID };
    // The server drops a silent client after 3 s; the token never expires.
    let config = NetcodeConfig { client_timeout_secs: 3, token_expire_secs: -1, ..default() };
    let netcode = match NetcodeClient::new(auth, config) {
        Ok(n) => n,
        Err(e) => {
            runlog::kv("net_connect_failed", &format!("server={} reason=\"{e:?}\"", link.server));
            eprintln!("error: cannot set up the connection to {}: {e:?}", link.server);
            exit.write(AppExit::error());
            return;
        }
    };
    let e = commands
        .spawn((
            Name::new("Client"),
            Client,
            ReplicationReceiver,
            LocalAddr(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0)),
            PeerAddr(link.server),
            netcode,
            UdpIo::default(),
        ))
        .id();
    commands.trigger(Connect { entity: e });
    runlog::kv("net_connecting", &format!("server={} client_id={} protocol={PROTOCOL_ID:#x}", link.server, link.id));
}

/// The client entity's connection state and its own id.
type ClientStatus = (Has<Connected>, Option<&'static Disconnected>, Option<&'static LocalId>);

/// Logs connection changes; quits when the connection cannot be made or
/// is lost (KF goes back to the main menu; we have none).
fn watch_connection(time: Res<Time<Real>>, mut link: ResMut<ClientLink>, client: Query<ClientStatus, With<Client>>, mut lobby: ResMut<NetLobby>, mut exit: MessageWriter<AppExit>) {
    let Ok((connected, disconnected, local)) = client.single() else { return };
    if connected && !link.connected {
        link.connected = true;
        link.was_connected = true;
        lobby.my_peer = local.map(|l| l.0.to_bits());
        runlog::kv("net_connected", &format!("server={} my_peer={:?} after_s={:.2}", link.server, lobby.my_peer, link.waited));
    }
    if !connected && link.connected {
        link.connected = false;
        let reason = disconnected.map_or("unknown".to_string(), |d| format!("{:?}", d.reason).replace(' ', "_"));
        runlog::kv("net_disconnected", &format!("server={} reason={reason} we_left={}", link.server, lobby.quit_requested));
        if !lobby.quit_requested {
            eprintln!("Lost the connection to the host {} ({reason}).", link.server);
            exit.write(AppExit::Success);
        }
    }
    if !link.was_connected {
        link.waited += time.delta_secs();
        if link.waited > CONNECT_TIMEOUT {
            runlog::kv("net_connect_failed", &format!("server={} reason=timeout seconds={CONNECT_TIMEOUT}", link.server));
            eprintln!("error: no answer from a host at {} after {CONNECT_TIMEOUT} s (is it started with --host? same port?)", link.server);
            exit.write(AppExit::error());
            link.was_connected = true; // report once
        }
    }
}

/// The first time the host's game record arrives: is it the map this game
/// loaded? Each game loads the map from its own install at startup, so a
/// different map cannot be played: say which `--map` to use and quit.
fn check_game(mut link: ResMut<ClientLink>, game: Query<&NetGame>, map: Res<crate::world::map::MapRequest>, options: Res<crate::game::waves::GameOptions>, mut lobby: ResMut<NetLobby>, mut exit: MessageWriter<AppExit>) {
    if link.game_checked {
        return;
    }
    let Ok(g) = game.single() else { return };
    link.game_checked = true;
    let same_map = g.map.eq_ignore_ascii_case(&map.map);
    let ours = (format!("{:?}", options.mode), format!("{:?}", options.length));
    runlog::kv(
        "net_game_info",
        &format!("host_map={} my_map={} same_map={same_map} host_mode={} host_length={} my_mode={} my_length={} match_started={}", g.map, map.map, g.mode, g.length, ours.0, ours.1, g.match_started),
    );
    if !same_map {
        eprintln!("error: the host plays {}; this game loaded {}. Start again with --map {}", g.map, map.map, g.map);
        runlog::kv("net_map_mismatch", &format!("host_map={} my_map={}", g.map, map.map));
        lobby.quit_requested = true;
        exit.write(AppExit::error());
    }
}

/// Sends my lobby choices whenever they change.
fn send_request(lobby: Res<NetLobby>, mut sender: Query<&mut MessageSender<LobbyRequest>, (With<Client>, With<Connected>)>, mut last: Local<Option<LobbyRequest>>, link: Res<ClientLink>) {
    let Some(req) = lobby.local.as_ref() else { return };
    if !link.connected {
        // Send everything again after a (re)connection.
        *last = None;
        return;
    }
    if last.as_ref() == Some(req) {
        return;
    }
    let Ok(mut s) = sender.single_mut() else { return };
    s.send::<LobbyChannel>(req.clone());
    runlog::kv("net_request_sent", &format!("name=\"{}\" perk={:?} level={} ready={} character={}", req.name, req.perk, req.level, req.ready, req.character));
    *last = Some(req.clone());
}
