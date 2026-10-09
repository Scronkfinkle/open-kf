//! Going to the next map at the end of a match (KF's GameInfo.RestartGame
//! and ServerTravel), without restarting the program. DESIGN.md, "Map
//! rotation and map voting without restarting", steps 2 and 3.
//!
//! **Who decides** (the single player, or the host): when the end screen
//! asks for the restart (`end_game::RestartGame`: Fire after 5 s, or by
//! itself 14 s after the end), KF's RestartGame runs:
//! - map voting on, and a network game (or our `--vote-test` switch in
//!   single player): the vote starts (`StartMapVote`) and holds the
//!   travel; its winner (`MapVoteFinished`) is the next map. A vote that
//!   did not start (e.g. auto open off) falls back to the map list;
//! - else the map list's next map (`MapRotation::next_map`), and the list
//!   position is saved in our settings file (KF's MapList SaveConfig).
//!
//! **Everyone** (single player, host, and network clients, who are told
//! by the host: net/client.rs) then travels the same way, on a
//! [`BeginTravel`] message: the loading screen is shown; once it is on
//! the display the map change starts (`ChangeMap`, a few seconds' frozen
//! load); when the new map is loaded (`MapLoaded`) the loading screen
//! goes and the vote window closes. KF's clients reconnect on every map
//! change; ours stay connected.
//!
//! **The new map** ([`fresh_player_on_new_map`], any map load after the
//! first): a new pawn as KF's map load gives (full health, no armour
//! unless the perk gives it, the starting inventory, kills and deaths
//! back to 0; the cash goes back to the start through `WaveGame`'s
//! restart counter, dosh.rs); name, perk, perk level and character are
//! kept. The lobby opens again (KF's PlayerController bPendingLobbyDisplay)
//! when the game started in it, and always in a network game.

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use crate::engine::runlog;
use crate::game::end_game::RestartGame;
use crate::game::loading_screen::{HideLoadingScreen, LoadingScreenShown, ShowLoadingScreen};
use crate::game::map_rotation::{MapRotation, list_installed_maps};
use crate::game::map_vote::{MapVote, MapVoteFinished, MapVoteSettings, MapVoteSystems, StartMapVote};
use crate::net::NetMode;
use crate::world::map::MapRequest;
use crate::world::map_change::{ChangeMap, MapEpoch, MapLoaded};

/// Where the map list position is saved after each `next_map` (the
/// settings file, `--settings`), and whether it may be: a list given on
/// the command line (`--map-list`) is for that run only and is not
/// written into the file.
#[derive(Resource, Clone, Debug, Default)]
pub struct RotationSave {
    pub path: std::path::PathBuf,
    pub save: bool,
}

/// Travel to `map` now: loading screen, map change, lobby. Sent by the
/// decision below (single player, host) or by net/client.rs when the
/// host announces a map.
#[derive(Message, Clone, Debug)]
pub struct BeginTravel {
    pub map: String,
    /// For the log: "rotation", "vote", "host", "join".
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum TravelState {
    #[default]
    Idle,
    /// The vote holds the travel; started at this frame.
    Voting { since: u32 },
    /// The loading screen is going up; the load starts when it is shown.
    Showing { map: String },
    /// `ChangeMap` sent at `asked` with the map epoch `epoch` before it.
    Loading { map: String, asked: u32, epoch: u32 },
}

/// The travel in progress, if any.
#[derive(Resource, Default, Debug)]
pub struct Travel {
    pub state: TravelState,
    /// Travels finished this run.
    pub done: u32,
    /// Real seconds when the current travel began, and from which map.
    began: Option<(std::time::Instant, String, String)>,
}

impl Travel {
    /// A vote, a loading screen or a load is in progress.
    pub fn busy(&self) -> bool {
        self.state != TravelState::Idle
    }
}

/// The travel systems (after the vote's, which send `MapVoteFinished`).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct TravelSystems;

pub struct TravelPlugin;

impl Plugin for TravelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Travel>()
            .init_resource::<RotationSave>()
            .add_message::<BeginTravel>()
            .add_systems(crate::world::map_change::PostMapLoad, fresh_player_on_new_map)
            .add_systems(Update, (decide, follow_vote, begin, start_load, finish).chain().in_set(TravelSystems).after(MapVoteSystems));
    }
}

/// The map list's next map (KF: MapList.GetNextMap), with the position
/// saved.
fn next_from_rotation(rotation: &mut MapRotation, request: &MapRequest, save: &RotationSave) -> String {
    let installed = list_installed_maps(&request.install_root);
    let next = rotation.next_map(&request.map, &installed);
    let saved = if !save.save {
        "false reason=list_from_command_line".to_string()
    } else {
        match crate::launcher::save_map_rotation(&save.path, rotation) {
            Ok(()) => "true".to_string(),
            Err(e) => format!("false reason=\"{e}\""),
        }
    };
    runlog::kv("map_rotation_saved", &format!("position={} file={} saved={saved}", rotation.position, save.path.display()));
    next
}

fn is_client(mode: &Option<Res<NetMode>>) -> bool {
    mode.as_deref().is_some_and(|m| matches!(m, NetMode::Client { .. }))
}

fn is_host(mode: &Option<Res<NetMode>>) -> bool {
    mode.as_deref().is_some_and(|m| matches!(m, NetMode::Host { .. }))
}

/// GameInfo.RestartGame: vote or map list (single player and host).
#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn decide(
    mut restarts: MessageReader<RestartGame>,
    mode: Option<Res<NetMode>>,
    settings: Res<MapVoteSettings>,
    mut travel: ResMut<Travel>,
    mut rotation: ResMut<MapRotation>,
    request: Res<MapRequest>,
    save: Res<RotationSave>,
    frames: Res<FrameCount>,
    mut start_vote: MessageWriter<StartMapVote>,
    mut begin: MessageWriter<BeginTravel>,
) {
    if restarts.read().count() == 0 {
        return;
    }
    if is_client(&mode) {
        // A client's end screen also times out; only the host travels.
        runlog::kv("travel_ignored", "reason=client (the host decides the next map)");
        return;
    }
    if travel.busy() {
        runlog::kv("travel_ignored", &format!("reason=busy state={:?}", travel.state));
        return;
    }
    // xVotingHandler: no voting in a standalone game (our --vote-test
    // switch allows it).
    let vote = settings.enabled && (is_host(&mode) || settings.single_player);
    runlog::kv("travel_decide", &format!("current={} vote={vote} vote_enabled={} host={} frame={}", request.map, settings.enabled, is_host(&mode), frames.0));
    if vote {
        travel.state = TravelState::Voting { since: frames.0 };
        start_vote.write(StartMapVote { candidates: list_installed_maps(&request.install_root), current_map: request.map.clone() });
        return;
    }
    let map = next_from_rotation(&mut rotation, &request, &save);
    begin.write(BeginTravel { map, reason: "rotation".into() });
}

/// The vote's winner, or the map list when the vote did not start.
#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn follow_vote(
    mut finished: MessageReader<MapVoteFinished>,
    vote: Res<MapVote>,
    mut travel: ResMut<Travel>,
    mut rotation: ResMut<MapRotation>,
    request: Res<MapRequest>,
    save: Res<RotationSave>,
    frames: Res<FrameCount>,
    mut begin: MessageWriter<BeginTravel>,
) {
    let winner = finished.read().last().cloned();
    let TravelState::Voting { since } = travel.state else {
        if let Some(f) = winner {
            runlog::kv("travel_ignored", &format!("reason=vote_finished_while_not_voting map={} state={:?}", f.map, travel.state));
        }
        return;
    };
    if let Some(f) = winner {
        runlog::kv("map_vote_travel", &format!("map={} reason={} travel=wired", f.map, f.reason.word()));
        begin.write(BeginTravel { map: f.map, reason: "vote".into() });
        travel.state = TravelState::Idle;
        return;
    }
    // StartMapVote is read the frame it is sent; one frame later a vote
    // that did not start holds nothing (HandleRestartGame returned true).
    if frames.0 > since + 1 && !vote.holds_travel() {
        runlog::kv("travel_vote_skipped", "fallback=map_list");
        let map = next_from_rotation(&mut rotation, &request, &save);
        begin.write(BeginTravel { map, reason: "rotation".into() });
        travel.state = TravelState::Idle;
    }
}

/// A travel begins: the loading screen goes up.
fn begin(mut begins: MessageReader<BeginTravel>, mut travel: ResMut<Travel>, mut show: MessageWriter<ShowLoadingScreen>, request: Res<MapRequest>, frames: Res<FrameCount>) {
    let Some(b) = begins.read().last().cloned() else { return };
    if matches!(travel.state, TravelState::Loading { .. }) {
        runlog::kv("travel_ignored", &format!("reason=loading map={} state={:?}", b.map, travel.state));
        return;
    }
    runlog::kv("travel_begin", &format!("from={} to={} reason={} frame={}", request.map, b.map, b.reason, frames.0));
    travel.began = Some((std::time::Instant::now(), request.map.clone(), b.reason.clone()));
    travel.state = TravelState::Showing { map: b.map.clone() };
    show.write(ShowLoadingScreen { map: b.map });
}

/// The loading screen is on the display: load the map now.
fn start_load(mut shown: MessageReader<LoadingScreenShown>, mut travel: ResMut<Travel>, mut change: MessageWriter<ChangeMap>, epoch: Res<MapEpoch>, frames: Res<FrameCount>) {
    let shown: Vec<String> = shown.read().map(|s| s.map.clone()).collect();
    let TravelState::Showing { map } = travel.state.clone() else { return };
    if !shown.contains(&map) {
        return;
    }
    runlog::kv("travel_load", &format!("map={map} frame={}", frames.0));
    change.write(ChangeMap { map: map.clone() });
    travel.state = TravelState::Loading { map, asked: frames.0, epoch: epoch.load };
}

/// The new map is loaded (or was refused): the loading screen goes, the
/// vote is over.
#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn finish(
    mut loaded: MessageReader<MapLoaded>,
    mut travel: ResMut<Travel>,
    epoch: Res<MapEpoch>,
    frames: Res<FrameCount>,
    mut hide: MessageWriter<HideLoadingScreen>,
    mut vote: ResMut<MapVote>,
) {
    let done = loaded.read().last().cloned();
    let TravelState::Loading { map, asked, epoch: before } = travel.state.clone() else { return };
    let (since, from, reason) = travel.began.clone().unwrap_or((std::time::Instant::now(), String::new(), String::new()));
    if let Some(m) = done {
        travel.state = TravelState::Idle;
        travel.done += 1;
        hide.write(HideLoadingScreen);
        vote.clear();
        runlog::kv(
            "travel_done",
            &format!("from={from} to={} reason={reason} load={} load_seconds={:.2} total_seconds={:.2} travels={} frame={}", m.map, m.load, m.seconds, since.elapsed().as_secs_f32(), travel.done, frames.0),
        );
    } else if frames.0 > asked + 2 && epoch.load == before {
        // ChangeMap refused (no such map): stay on this one.
        travel.state = TravelState::Idle;
        hide.write(HideLoadingScreen);
        vote.clear();
        runlog::kv("travel_failed", &format!("map={map} reason=map_change_refused frame={}", frames.0));
    }
}

/// Any map after the first: the player starts the new map as KF's map
/// load makes them (a new pawn and PlayerReplicationInfo values), and the
/// lobby opens again where it applies.
#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn fresh_player_on_new_map(
    epoch: Res<MapEpoch>,
    (mut health, mut armour, mut kills): (ResMut<crate::game::combat::PlayerHealth>, ResMut<crate::player::armour::Armour>, ResMut<crate::game::combat::KillCount>),
    mut respawned: MessageWriter<crate::game::perks::RespawnPawn>,
    mut menus: ResMut<crate::game::menus::MenuState>,
    lobby_settings: Res<crate::game::menus::LobbySettings>,
    options: Res<crate::game::waves::GameOptions>,
    mut net: ResMut<crate::net::lobby::NetLobby>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    vet: Res<crate::game::perks::Veterancy>,
) {
    if epoch.first() {
        return;
    }
    let was = (health.health, health.dead, health.deaths, kills.0);
    health.dead = false;
    health.health = 100.0;
    health.to_give = 0.0;
    health.deaths = 0;
    *armour = crate::player::armour::Armour::default();
    kills.0 = 0;
    respawned.write(crate::game::perks::RespawnPawn);
    // The lobby's Ready gives the pawn from this perk on (menus/mod.rs).
    menus.pawn_vet = None;
    let pages: Vec<String> = menus.stack.iter().map(|p| format!("{p:?}")).collect();
    menus.stack.clear();
    menus.drag = None;
    // Network: nobody is ready on the new map (the host's records were
    // reset when it announced the map, net/server.rs).
    net.want_ready = false;
    net.reset_for_new_map();
    let lobby = options.mode == crate::game::waves::GameMode::Waves && (lobby_settings.open || net.active);
    if lobby {
        menus.stack.push(crate::game::menus::Page::Lobby);
        if let Ok(mut c) = cursor.single_mut() {
            c.grab_mode = CursorGrabMode::None;
            c.visible = true;
        }
    }
    runlog::kv(
        "new_map_player",
        &format!(
            "load={} health={:.0}->{:.0} dead={}->false deaths={}->0 kills={}->0 perk={} closed_pages=[{}] lobby={lobby} lobby_reason={}",
            epoch.load,
            was.0,
            health.health,
            was.1,
            was.2,
            was.3,
            vet.vet.label(),
            pages.join(","),
            if net.active { "net_game" } else { lobby_settings.reason }
        ),
    );
}
