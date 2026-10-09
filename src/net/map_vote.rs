//! Map voting over the network (game/map_vote). KF: each player's
//! VotingReplicationInfo carries the map list and the vote counts to that
//! player and its SendMapVote to the server (SubmitMapVote).
//!
//! Here the host runs the vote. Clients send `MapVoteRequest` (my vote);
//! the host checks it with the rules as for its own player. The host sends
//! `MapVoteState` (the whole window: maps, counts, who voted, time left,
//! the winner) to every client when it changes, and every 2 s while a
//! vote is on (for players who join during it). `None`: no vote.

use bevy::prelude::*;
use lightyear::connection::host::HostClient;
use lightyear::prelude::server::ClientOf;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

use super::NetMode;
use super::protocol::{GameChannel, NetPlayer};
use super::server::PlayerSlot;
use crate::engine::runlog;
use crate::game::map_vote::rules::VoteView;
use crate::game::map_vote::{MapVote, MapVoteSystems};

/// A client's vote (KF: VotingReplicationInfo.SendMapVote), by map name.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MapVoteRequest {
    pub map: String,
}

/// The host's vote as every window shows it (None: no vote).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MapVoteState(pub Option<VoteView>);

/// Resend the state this often while a vote is on (seconds).
const RESEND: f32 = 2.0;

pub(super) fn build(app: &mut App, mode: &NetMode) {
    match mode {
        NetMode::Host { .. } => {
            app.add_systems(Update, (receive_requests.before(MapVoteSystems), send_state.after(MapVoteSystems)));
        }
        NetMode::Client { .. } => {
            app.add_systems(Update, (receive_state.before(MapVoteSystems), send_requests.after(MapVoteSystems)));
        }
        NetMode::Off => {}
    }
}

/// Connected clients' links (not the host's own player).
type RemoteLinks = (With<ClientOf>, With<Connected>, Without<HostClient>);
/// This game's own connection to the server (a client game).
type MyConnection = (With<Client>, With<Connected>, Without<LinkOf>);

// ---------------------------------------------------------------- host

fn receive_requests(mut links: Query<(Entity, &mut MessageReceiver<MapVoteRequest>), With<ClientOf>>, players: Query<(&NetPlayer, &PlayerSlot)>, mut vote: ResMut<MapVote>) {
    for (link, mut rx) in &mut links {
        let peer = players.iter().find(|(_, s)| s.link == link).map(|(p, _)| p.peer);
        for r in rx.receive() {
            runlog::kv("net_map_vote_request", &format!("peer={peer:?} map={}", r.map));
            match peer {
                Some(p) => vote.incoming.push((p, r.map)),
                None => runlog::kv("map_vote_refused", &format!("link={link:?} map={} reason=no_player", r.map)),
            }
        }
    }
}

fn send_state(time: Res<Time<Real>>, mut vote: ResMut<MapVote>, mut senders: Query<&mut MessageSender<MapVoteState>, RemoteLinks>, mut last: Local<(Option<VoteView>, f32, usize)>) {
    let now = time.elapsed_secs();
    let clients = senders.iter().count();
    let changed = vote.view != last.0;
    // A client that joined since the last send gets it at once.
    let new_client = clients > last.2;
    if !changed && !new_client && (vote.view.is_none() || now - last.1 < RESEND) {
        last.2 = clients;
        return;
    }
    for mut tx in &mut senders {
        tx.send::<GameChannel>(MapVoteState(vote.view.clone()));
    }
    // Log the changes that matter (not every second's countdown).
    let key = |v: Option<&VoteView>| v.map(|v| (v.window_open, v.voters.clone(), v.result.clone(), v.maps.len()));
    if changed && key(vote.view.as_ref()) != key(last.0.as_ref()) {
        let v = vote.view.as_ref();
        runlog::kv(
            "net_map_vote_sent",
            &format!(
                "clients={clients} open={} time_left={} voters={} winner={}",
                v.is_some_and(|v| v.window_open),
                v.map_or(-1, |v| v.time_left),
                v.map_or(0, |v| v.voters.len()),
                v.and_then(|v| v.result.as_ref()).map_or("none", |r| r.map.as_str())
            ),
        );
    }
    vote.changed = false;
    *last = (vote.view.clone(), now, clients);
}

// -------------------------------------------------------------- client

fn receive_state(mut rx: Query<&mut MessageReceiver<MapVoteState>, (With<Client>, Without<LinkOf>)>, mut vote: ResMut<MapVote>) {
    for mut r in &mut rx {
        for s in r.receive() {
            if vote.view == s.0 {
                continue;
            }
            let v = s.0.as_ref();
            // Log the changes that matter (not every second's countdown).
            let key = |v: Option<&VoteView>| v.map(|v| (v.window_open, v.voters.clone(), v.result.clone(), v.maps.len()));
            if key(vote.view.as_ref()) != key(v) {
                runlog::kv(
                    "net_map_vote_received",
                    &format!(
                        "maps={} open={} time_left={} voters=[{}] winner={}",
                        v.map_or(0, |v| v.maps.len()),
                        v.is_some_and(|v| v.window_open),
                        v.map_or(-1, |v| v.time_left),
                        v.map_or(String::new(), |v| v.voters.iter().map(|(p, m)| format!("{p}:{m}")).collect::<Vec<_>>().join(",")),
                        v.and_then(|v| v.result.as_ref()).map_or("none", |r| r.map.as_str())
                    ),
                );
            }
            // A new vote (none before, or a finished one replaced).
            if vote.view.is_none() || (vote.view.as_ref().is_some_and(|o| o.result.is_some()) && v.is_some_and(|n| n.result.is_none())) {
                vote.clear();
            }
            vote.view = s.0;
        }
    }
}

fn send_requests(mut vote: ResMut<MapVote>, mut tx: Query<&mut MessageSender<MapVoteRequest>, MyConnection>) {
    if vote.outgoing.is_empty() {
        return;
    }
    let Ok(mut tx) = tx.single_mut() else {
        runlog::kv("map_vote_refused", &format!("count={} reason=not_connected", vote.outgoing.len()));
        vote.outgoing.clear();
        return;
    };
    for map in std::mem::take(&mut vote.outgoing) {
        runlog::kv("net_map_vote_request_sent", &format!("map={map}"));
        tx.send::<GameChannel>(MapVoteRequest { map });
    }
}
