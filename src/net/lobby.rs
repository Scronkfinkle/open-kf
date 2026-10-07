//! Between the network records and the lobby screen: the lobby's player
//! rows come from the replicated `NetPlayer`s, the local player's choices
//! go out as a `LobbyRequest`, and the match starts here when the server
//! says it has begun and this player is ready.

use bevy::prelude::*;

use super::protocol::{LobbyRequest, NetGame, NetPlayer};
use crate::engine::runlog;
use crate::game::menus::{LobbyPlayer, MenuData};
use crate::game::perks::{Perk, Veterancy};

/// What the menus read and write in a network game.
#[derive(Resource, Default, Debug)]
pub struct NetLobby {
    /// A network game (`--host` or `--join`).
    pub active: bool,
    /// The lobby rows: every connected player, the host first.
    pub players: Vec<LobbyPlayer>,
    /// Ready as the local player asked (the Ready / Unready button).
    pub want_ready: bool,
    /// Ready as the server has it.
    pub local_ready: bool,
    /// GRI.bMatchHasBegun.
    pub match_started: bool,
    /// GRI.LobbyTimeout (above 0: "Game will auto-commence in: N").
    pub lobby_timeout: i32,
    /// My peer id (0 for the host), once known.
    pub my_peer: Option<u64>,
    /// My current choices, rebuilt every frame.
    pub local: Option<LobbyRequest>,
    /// The player asked to leave (Disconnect, Forfeit, Exit Game): leave
    /// cleanly, then quit (lobby.rs `leave`).
    pub quit_requested: bool,
    /// The menus were told to start (once).
    pub(super) start_sent: bool,
}

/// Sent once when this player's match begins (the server started the
/// match and this player is ready): the menus close the lobby, the pawn
/// gets its start items, the wave timer runs.
#[derive(Message, Clone, Copy, Debug)]
pub struct StartLocalMatch;

/// The lobby systems (the server and client systems run before them).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct LobbySystems;

pub(super) fn build(app: &mut App) {
    app.add_systems(PreUpdate, local_request)
        .add_systems(Update, (read_records, leave).chain().in_set(LobbySystems));
}

/// My choices: the name (--name), the perk and level, the character, ready.
fn local_request(mut lobby: ResMut<NetLobby>, data: Res<MenuData>, vet: Res<Veterancy>, character: Res<crate::player::character::CharacterChoice>) {
    if data.player_name.is_empty() {
        return; // the menus are not loaded yet
    }
    let req = LobbyRequest {
        name: data.player_name.clone(),
        perk: vet.selected.map(|p| p.index() as u8),
        level: vet.level,
        ready: lobby.want_ready,
        character: character.0.clone().unwrap_or_else(|| crate::player::character::DEFAULT_CHARACTER.to_string()),
    };
    if lobby.local.as_ref() != Some(&req) {
        lobby.local = Some(req);
    }
}

/// The replicated records -> the lobby rows; starts my match when due.
fn read_records(players: Query<&NetPlayer>, game: Query<&NetGame>, mut lobby: ResMut<NetLobby>, mut start: MessageWriter<StartLocalMatch>, mut last_log: Local<String>) {
    let mut recs: Vec<&NetPlayer> = players.iter().collect();
    // The host (peer 0) first, then by id: the same order on every screen.
    recs.sort_by_key(|p| p.peer);
    lobby.players = recs
        .iter()
        .map(|p| LobbyPlayer { name: p.name.clone(), perk: p.perk.and_then(|i| Perk::ALL.get(i as usize).copied()), level: p.level, ready: p.ready })
        .collect();
    let me = lobby.my_peer.and_then(|id| recs.iter().find(|p| p.peer == id));
    lobby.local_ready = me.is_some_and(|p| p.ready);
    if let Ok(g) = game.single() {
        lobby.match_started = g.match_started;
        lobby.lobby_timeout = g.lobby_timeout;
    }
    // One log line whenever what the lobby shows changes.
    let line = format!(
        "players={} match_started={} lobby_timeout={} my_peer={:?} rows=[{}]",
        recs.len(),
        lobby.match_started,
        lobby.lobby_timeout,
        lobby.my_peer,
        recs.iter()
            .map(|p| format!("{}:\"{}\":{}:L{}:{}", p.peer, p.name, p.perk.and_then(|i| Perk::ALL.get(i as usize)).map_or("none", |k| k.class()), p.level, if p.ready { "ready" } else { "not_ready" }))
            .collect::<Vec<_>>()
            .join(",")
    );
    if *last_log != line {
        runlog::kv("net_lobby", &line);
        *last_log = line;
    }
    if lobby.match_started && lobby.local_ready && !lobby.start_sent {
        lobby.start_sent = true;
        start.write(StartLocalMatch);
        runlog::kv("net_local_match_start", &format!("my_peer={:?}", lobby.my_peer));
    }
}

/// Leaving on purpose: tell the other side (the client disconnects, the
/// host stops its server) and quit a few frames later, so the goodbye
/// packets are sent.
fn leave(mut lobby: ResMut<NetLobby>, mut commands: Commands, clients: Query<Entity, (With<lightyear::prelude::Client>, Without<lightyear::prelude::LinkOf>)>, server: Option<Res<super::server::ServerEntity>>, mut frames: Local<Option<u32>>, mut exit: MessageWriter<AppExit>) {
    if !lobby.quit_requested {
        return;
    }
    match *frames {
        None => {
            if let Some(s) = server {
                commands.trigger(lightyear::prelude::server::Stop { entity: s.0 });
                runlog::kv("net_leave", "side=host action=stop_server");
            }
            for c in &clients {
                commands.trigger(lightyear::prelude::Disconnect { entity: c });
                runlog::kv("net_leave", "side=client action=disconnect");
            }
            *frames = Some(0);
        }
        Some(n) if n >= 10 => {
            exit.write(AppExit::Success);
        }
        Some(n) => *frames = Some(n + 1),
    }
    // Keep the request so the quit is not taken for a lost connection.
    lobby.quit_requested = true;
}
