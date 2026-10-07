//! Shared zed time (docs/multiplayer-prototype.md, "Shared zed time").
//!
//! KF: only the server runs KFGameType (DramaticEvent, Tick, Killed), so
//! zed time is decided once for everyone; the server's SetGameSpeed
//! reaches the clients as Level.TimeDilation, and ClientEnterZedTime /
//! ClientExitZedTime play the sounds on every player's game.
//!
//! Here: the host's `ZedTime` decides (game/zed_time.rs, role Host). It
//! rolls for every kill, a client's too (the host applies the client's
//! hits, so the zed dies on the host with `damaged_by` = that client), and
//! uses the killer's perk (from the lobby record) and the killer's pawn.
//! Clients do not roll: they send their other rolls (explosions, the
//! debug F2 / `zed_time`) as `ZedTimeRequest`, and follow the host's
//! `ZedTimeCommand`s (start / extend, speed-up, end) with the same
//! countdown and the same sounds.

use bevy::prelude::*;
use lightyear::connection::host::HostClient;
use lightyear::prelude::server::ClientOf;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

use super::NetMode;
use super::pawns::RemotePawn;
use super::protocol::{GameChannel, NetPlayer};
use super::server::PlayerSlot;
use crate::engine::runlog;
use crate::game::perks::{Perk, Vet};
use crate::game::zed_time::{DramaticEvent, ZedTimeCommand, ZedTimeNet, ZedTimeRole, ZedTimeSystems};
use crate::player::body::PawnState;

/// A client's roll for the host (KF: the client's projectiles run their
/// HurtRadius on the server, which calls DramaticEvent there).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ZedTimeRequest {
    pub chance: f32,
    pub duration: f32,
    pub reason: String,
}

pub(super) fn build(app: &mut App, mode: &NetMode) {
    match mode {
        NetMode::Host { .. } => {
            app.insert_resource(ZedTimeNet { role: ZedTimeRole::Host, ..default() })
                .add_systems(Update, (feed_players.before(ZedTimeSystems::Roll), receive_requests.before(ZedTimeSystems::Run), send_commands.after(ZedTimeSystems::Run)));
        }
        NetMode::Client { .. } => {
            app.insert_resource(ZedTimeNet { role: ZedTimeRole::Client, ..default() })
                .add_systems(Update, (receive_commands.before(ZedTimeSystems::Run), send_requests.after(ZedTimeSystems::Run)));
        }
        NetMode::Off => {}
    }
}

/// Connected clients' links (not the host's own player).
type RemoteLinks = (With<ClientOf>, With<Connected>, Without<HostClient>);
/// This game's own connection to the server (a client game).
type MyConnection = (With<Client>, With<Connected>, Without<LinkOf>);

/// Wall-clock seconds (Unix time), to compare two games' logs.
fn wall() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// The perk of a lobby record (KFPRI.ClientVeteranSkill and its level).
fn vet_of(p: &NetPlayer) -> Vet {
    Vet { perk: p.perk.and_then(|i| Perk::ALL.get(i as usize).copied()), level: p.level }
}

// ---------------------------------------------------------------- host

/// Every other player's perk and pawn, for the host's kill rolls.
fn feed_players(players: Query<&NetPlayer>, pawns: Query<(&RemotePawn, &PawnState)>, mut net: ResMut<ZedTimeNet>) {
    net.players.clear();
    for p in &players {
        let at = pawns.iter().find(|(r, s)| r.peer == p.peer && s.active).map(|(_, s)| s.location);
        net.players.insert(p.peer, (vet_of(p), at));
    }
}

/// Clients' rolls go into the host's DramaticEvent.
fn receive_requests(mut links: Query<(Entity, &mut MessageReceiver<ZedTimeRequest>), With<ClientOf>>, players: Query<(&NetPlayer, &PlayerSlot)>, mut events: MessageWriter<DramaticEvent>) {
    for (link, mut rx) in &mut links {
        let peer = players.iter().find(|(_, s)| s.link == link).map(|(p, _)| p.peer);
        for r in rx.receive() {
            runlog::kv("net_zed_time_request", &format!("peer={peer:?} reason={} chance={} duration={} wall={:.3}", r.reason, r.chance, r.duration, wall()));
            if peer.is_none() {
                continue;
            }
            // KF clamps nothing here; a client asking for more than 1 is
            // treated as a forced event.
            let chance = r.chance.clamp(0.0, 1.0);
            events.write(DramaticEvent { chance, duration: r.duration.clamp(0.0, 30.0), reason: crate::game::zed_time::known_reason(&r.reason), peer });
        }
    }
}

/// What the host's zed time did this frame goes to every client.
fn send_commands(mut net: ResMut<ZedTimeNet>, mut senders: Query<&mut MessageSender<ZedTimeCommand>, RemoteLinks>) {
    for c in std::mem::take(&mut net.outgoing) {
        let mut clients = 0;
        for mut tx in &mut senders {
            tx.send::<GameChannel>(c.clone());
            clients += 1;
        }
        runlog::kv("net_zed_time_sent", &format!("clients={clients} wall={:.3} {c:?}", wall()));
    }
}

// -------------------------------------------------------------- client

/// The host's commands, for game/zed_time.rs.
fn receive_commands(mut rx: Query<&mut MessageReceiver<ZedTimeCommand>, (With<Client>, Without<LinkOf>)>, mut net: ResMut<ZedTimeNet>) {
    for mut r in &mut rx {
        for c in r.receive() {
            runlog::kv("net_zed_time", &format!("wall={:.3} {c:?}", wall()));
            net.incoming.push(c);
        }
    }
}

/// My rolls go to the host.
fn send_requests(mut net: ResMut<ZedTimeNet>, mut tx: Query<&mut MessageSender<ZedTimeRequest>, MyConnection>) {
    if net.requests.is_empty() {
        return;
    }
    let Ok(mut tx) = tx.single_mut() else {
        runlog::kv("net_zed_time_request_dropped", &format!("count={} reason=not_connected", net.requests.len()));
        net.requests.clear();
        return;
    };
    for e in std::mem::take(&mut net.requests) {
        tx.send::<GameChannel>(ZedTimeRequest { chance: e.chance, duration: e.duration, reason: e.reason.to_string() });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lobby_perk_gives_the_killers_extensions() {
        let p = NetPlayer { peer: 3, name: "C".into(), perk: Some(3), level: 5, ready: true, character: String::new() };
        let v = vet_of(&p);
        assert_eq!(v.perk, Some(Perk::Commando));
        // KFVetCommando.ZedTimeExtensions: level - 2 from level 3.
        assert_eq!(v.zed_time_extensions(), 3);
        assert_eq!(vet_of(&NetPlayer { perk: None, ..p.clone() }).zed_time_extensions(), 0);
        assert_eq!(vet_of(&NetPlayer { perk: Some(99), ..p }).perk, None);
    }
}
