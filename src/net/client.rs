//! A joining player's side: connect to the host over UDP, send my lobby
//! choices, load the map the host plays (at the join, and whenever the
//! host goes to another map: game/travel.rs), leave.

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
/// netcode's timeout (seconds without a packet, both ways): longer than a
/// map load (game/travel.rs freezes the game while it loads).
const CONNECTION_TIMEOUT: i32 = 20;

pub(super) fn build(app: &mut App, server: SocketAddr) {
    app.insert_resource(ClientLink { server, ..default() })
        .add_systems(Startup, connect)
        .add_systems(Update, (watch_connection, check_game, send_request).chain().before(super::lobby::LobbySystems))
        .add_systems(Update, take_travel_number.after(crate::game::travel::TravelSystems));
}

/// A travel to the host's map has started: its number is the one this
/// game is loading (state stamped with it is taken once it is loaded).
fn take_travel_number(mut started: MessageReader<crate::game::travel::TravelStarted>, mut travel: ResMut<super::NetTravel>) {
    for s in started.read() {
        if let Some(t) = s.net_travel {
            runlog::kv("net_travel_started", &format!("travel={t} map={} loaded={} was_pending={:?}", s.map, travel.loaded, travel.pending));
            travel.pending = Some(t);
        }
    }
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
    // Either side drops the other after this many silent seconds; the
    // token never expires. A map change freezes both games for the load
    // (KF-Farm: about 1 s here, more on slower machines), so the old 3 s
    // was too short.
    let config = NetcodeConfig { client_timeout_secs: CONNECTION_TIMEOUT, token_expire_secs: -1, ..default() };
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

/// The host's game record. The first time: is it the map this game
/// loaded? If not (the host changed map between the query and the join)
/// this game loads the host's map in place, or, when it is not installed
/// here, says which `--map` to use and quits. Afterwards: a new travel
/// number means the host went to another map: load it too (we stay
/// connected; KF's clients reconnect).
#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn check_game(
    mut link: ResMut<ClientLink>,
    game: Query<&NetGame>,
    map: Res<crate::world::map::MapRequest>,
    options: Res<crate::game::waves::GameOptions>,
    mut lobby: ResMut<NetLobby>,
    mut exit: MessageWriter<AppExit>,
    mut travel: ResMut<super::NetTravel>,
    mut begin: MessageWriter<crate::game::travel::BeginTravel>,
) {
    let Ok(g) = game.single() else { return };
    let installed = || crate::game::map_rotation::installed_name(&g.map, &crate::game::map_rotation::list_installed_maps(&map.install_root));
    if link.game_checked {
        if g.travel != travel.seen {
            runlog::kv("net_travel_received", &format!("travel={} was={} map={} my_map={} my_peer={:?}", g.travel, travel.seen, g.map, map.map, lobby.my_peer));
            travel.seen = g.travel;
            // Not ready for the new map (the host has made everyone not
            // ready); my next request says so at once.
            lobby.reset_for_new_map();
            if installed().is_none() {
                // As at the join: the host's map is not here; staying would
                // leave this game on the old map with every state dropped.
                runlog::kv("net_map_missing", &format!("host_map={} my_map={} when=travel action=quit", g.map, map.map));
                eprintln!("error: the host went to {}, which is not installed here (this game is on {}).", g.map, map.map);
                lobby.quit_requested = true;
                exit.write(AppExit::error());
                return;
            }
            // `pending` is set when the travel really starts (`take_travel_number`):
            // one asked for during a load waits for it.
            begin.write(crate::game::travel::BeginTravel { map: g.map.clone(), reason: "host".into(), net_travel: Some(g.travel) });
        }
        return;
    }
    link.game_checked = true;
    let same_map = g.map.eq_ignore_ascii_case(&map.map);
    let ours = (format!("{:?}", options.mode), format!("{:?}", options.length), format!("{:?}", options.difficulty));
    runlog::kv(
        "net_game_info",
        &format!(
            "host_map={} my_map={} same_map={same_map} host_travel={} host_mode={} host_length={} host_difficulty={} my_mode={} my_length={} my_difficulty={} match_started={}",
            g.map, map.map, g.travel, g.mode, g.length, g.difficulty, ours.0, ours.1, ours.2, g.match_started
        ),
    );
    travel.known = true;
    travel.seen = g.travel;
    if same_map {
        travel.loaded = g.travel;
        return;
    }
    let installed = installed().is_some();
    runlog::kv("net_map_mismatch", &format!("host_map={} my_map={} installed={installed} action={}", g.map, map.map, if installed { "travel" } else { "quit" }));
    if installed {
        // The map loaded here is none of the host's: take no map state
        // (stamped with the host's number) until the host's map is loaded.
        travel.loaded = super::NOT_A_HOST_MAP;
        begin.write(crate::game::travel::BeginTravel { map: g.map.clone(), reason: "join".into(), net_travel: Some(g.travel) });
    } else {
        eprintln!("error: the host plays {}, which is not installed here (this game loaded {}).", g.map, map.map);
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
