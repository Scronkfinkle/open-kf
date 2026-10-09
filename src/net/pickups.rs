//! The pickups are the host's (game/pickups, docs/DESIGN.md "Pickups").
//! The host sends the list of shown pickups when it changes and every 2 s
//! (for players who join late), and tells everyone when one is taken
//! (`yours` for the taker). A client asks for each pickup its player
//! touches; the host checks and answers.

use bevy::prelude::*;
use lightyear::connection::host::HostClient;
use lightyear::prelude::server::ClientOf;
use lightyear::prelude::*;

use super::NetMode;
use super::protocol::{GameChannel, NetPlayer, PickupStates, Stamped};
use super::server::PlayerSlot;
use crate::engine::runlog;
use crate::game::pickups::{DropRequest, HostNotice, PickupNet, PickupNotice, PickupRequest, PickupSystems, Pickups, ShownPickup};

/// Resend the list this often even without a change (seconds).
const RESEND: f32 = 2.0;

type RemoteLinks = (With<ClientOf>, With<Connected>, Without<HostClient>);
type MyConnection = (With<Client>, With<Connected>, Without<LinkOf>);

pub(super) fn build(app: &mut App, mode: &NetMode) {
    match mode {
        NetMode::Host { .. } => {
            app.add_systems(Update, receive_requests.before(PickupSystems::Answer).after(PickupSystems::Touch))
                .add_systems(Update, receive_drops.before(PickupSystems::Rules))
                .add_systems(Update, (send_notices, send_states).chain().after(PickupSystems::Answer));
        }
        NetMode::Client { .. } => {
            app.add_systems(Update, (receive_states, receive_notices).before(PickupSystems::Rules)).add_systems(Update, (send_requests, send_drops).after(PickupSystems::Answer));
        }
        NetMode::Off => {}
    }
}

fn receive_requests(mut links: Query<(Entity, &mut MessageReceiver<Stamped<PickupRequest>>), With<ClientOf>>, players: Query<(&NetPlayer, &PlayerSlot)>, mut net: ResMut<PickupNet>, travel: Res<super::NetTravel>) {
    for (link, mut rx) in &mut links {
        let peer = players.iter().find(|(_, s)| s.link == link).map(|(p, _)| p.peer);
        for r in rx.receive().filter_map(|m| travel.from_client(m, "pickup_request")) {
            match peer {
                Some(p) => net.incoming.push((p, r)),
                None => runlog::kv("pickup_request_dropped", &format!("link={link:?} reason=no_player")),
            }
        }
    }
}

fn receive_drops(mut links: Query<(Entity, &mut MessageReceiver<Stamped<DropRequest>>), With<ClientOf>>, players: Query<(&NetPlayer, &PlayerSlot)>, mut net: ResMut<PickupNet>, travel: Res<super::NetTravel>) {
    for (link, mut rx) in &mut links {
        let peer = players.iter().find(|(_, s)| s.link == link).map(|(p, _)| p.peer);
        for r in rx.receive().filter_map(|m| travel.from_client(m, "drop_request")) {
            match peer {
                Some(p) => net.drop_incoming.push((p, r)),
                None => runlog::kv("drop_request_dropped", &format!("link={link:?} reason=no_player")),
            }
        }
    }
}

fn send_drops(mut net: ResMut<PickupNet>, mut tx: Query<&mut MessageSender<Stamped<DropRequest>>, MyConnection>, travel: Res<super::NetTravel>) {
    if net.drop_outgoing.is_empty() {
        return;
    }
    let Ok(mut tx) = tx.single_mut() else { return };
    for r in std::mem::take(&mut net.drop_outgoing) {
        runlog::kv("net_drop_request_sent", &format!("token={} why={:?} class={} gives={}", r.token, r.why, r.class, r.gives.label()));
        tx.send::<GameChannel>(travel.stamp(r));
    }
}

fn send_notices(mut net: ResMut<PickupNet>, players: Query<(&NetPlayer, &PlayerSlot)>, mut senders: Query<(Entity, &mut MessageSender<Stamped<PickupNotice>>), RemoteLinks>, travel: Res<super::NetTravel>) {
    // Each notice is about the map loaded now.
    let stamp = |notice: PickupNotice| travel.stamp(notice);
    for n in std::mem::take(&mut net.notices) {
        match n {
            HostNotice::Taken { taker, id, class, location, gives } => {
                let mut sent = Vec::new();
                for (link, mut tx) in &mut senders {
                    let peer = players.iter().find(|(_, s)| s.link == link).map(|(p, _)| p.peer);
                    let yours = peer.is_some() && peer == taker;
                    tx.send::<GameChannel>(stamp(PickupNotice::Taken { id, class: class.clone(), location, gives: gives.clone(), yours }));
                    sent.push(format!("{}:{yours}", peer.map_or("?".into(), |p| p.to_string())));
                }
                runlog::kv("net_pickup_notice_sent", &format!("id={id} class={class} travel={} taker={} to=[{}]", travel.loaded, taker.map_or("host".into(), |t| t.to_string()), sent.join(" ")));
            }
            HostNotice::Dropped { peer, token, id, reason } => {
                let link = players.iter().find(|(p, _)| p.peer == peer).map(|(_, s)| s.link);
                let sent = link.and_then(|l| senders.get_mut(l).ok()).map(|(_, mut tx)| tx.send::<GameChannel>(stamp(PickupNotice::Dropped { token, id, reason: reason.clone() }))).is_some();
                runlog::kv("net_drop_answer_sent", &format!("peer={peer} token={token} id={} reason={reason} sent={sent}", id.map_or("none".into(), |i| i.to_string())));
            }
            HostNotice::Denied { peer, id, reason } => {
                let link = players.iter().find(|(p, _)| p.peer == peer).map(|(_, s)| s.link);
                let sent = link.and_then(|l| senders.get_mut(l).ok()).map(|(_, mut tx)| tx.send::<GameChannel>(stamp(PickupNotice::Denied { id, reason: reason.clone() }))).is_some();
                runlog::kv("net_pickup_denied_sent", &format!("peer={peer} id={id} reason={reason} sent={sent}"));
            }
        }
    }
}

#[allow(clippy::type_complexity)] // Bevy system parameters
fn send_states(time: Res<Time<Real>>, pickups: Res<Pickups>, travel: Res<super::NetTravel>, mut senders: Query<&mut MessageSender<PickupStates>, RemoteLinks>, mut last: Local<(Vec<ShownPickup>, f32, usize)>) {
    let now = time.elapsed_secs();
    let list: Vec<ShownPickup> = pickups.shown.values().cloned().collect();
    let clients = senders.iter().count();
    let changed = list != last.0;
    // A new client gets the list at once.
    if !changed && clients <= last.2 && now - last.1 < RESEND {
        last.2 = clients;
        return;
    }
    for mut tx in &mut senders {
        tx.send::<GameChannel>(PickupStates { travel: travel.loaded, shown: list.clone() });
    }
    if changed {
        let ids: Vec<String> = list.iter().map(|s| format!("{}:{}", s.id, s.class.rsplit('.').next().unwrap_or(""))).collect();
        runlog::kv("net_pickups_sent", &format!("clients={clients} shown={} travel={} [{}]", list.len(), travel.loaded, ids.join(" ")));
    }
    *last = (list, now, clients);
}

fn send_requests(mut net: ResMut<PickupNet>, mut tx: Query<&mut MessageSender<Stamped<PickupRequest>>, MyConnection>, travel: Res<super::NetTravel>) {
    if net.outgoing.is_empty() {
        return;
    }
    let Ok(mut tx) = tx.single_mut() else { return };
    for r in std::mem::take(&mut net.outgoing) {
        runlog::kv("net_pickup_request_sent", &format!("id={} class={} travel={}", r.id, r.class, travel.loaded));
        tx.send::<GameChannel>(travel.stamp(r));
    }
}

fn receive_states(mut rx: Query<&mut MessageReceiver<PickupStates>, (With<Client>, Without<LinkOf>)>, mut net: ResMut<PickupNet>, mut last: Local<Vec<ShownPickup>>, travel: Res<super::NetTravel>, mut dropped: Local<u32>) {
    for mut r in &mut rx {
        for s in r.receive() {
            // Another map's pickups (sent before or during a map change).
            if !travel.current(s.travel) {
                *dropped += 1;
                runlog::kv("net_pickups_dropped", &format!("travel={} loaded={} dropped={}", s.travel, travel.loaded, *dropped));
                continue;
            }
            if *last != s.shown {
                runlog::kv("net_pickups_received", &format!("shown={} travel={}", s.shown.len(), s.travel));
                *last = s.shown.clone();
            }
            net.from_host = Some(s.shown);
        }
    }
}

fn receive_notices(mut rx: Query<&mut MessageReceiver<Stamped<PickupNotice>>, (With<Client>, Without<LinkOf>)>, mut net: ResMut<PickupNet>, travel: Res<super::NetTravel>) {
    for mut r in &mut rx {
        // About another map (sent before or during a map change): dropped.
        for n in r.receive().filter_map(|m| travel.from_host(m, "pickup_notice")) {
            net.received.push(n);
        }
    }
}
