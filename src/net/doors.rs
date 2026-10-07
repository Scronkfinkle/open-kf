//! Step 4: the doors are the host's (world/door.rs `DoorNet`). Clients
//! send what their player does to a door (USE, the Welder's hits); the host
//! does it and sends every door's state to the clients when a door changes
//! (and every 2 s, for players who join late).

use bevy::prelude::*;
use lightyear::connection::host::HostClient;
use lightyear::prelude::server::ClientOf;
use lightyear::prelude::*;

use super::NetMode;
use super::protocol::{DoorStates, GameChannel, NetPlayer};
use super::server::PlayerSlot;
use crate::engine::runlog;
use crate::world::door::{DoorNet, DoorRequest, DoorRole};

/// Resend the door states this often even without a change (seconds).
const RESEND: f32 = 2.0;

pub(super) fn build(app: &mut App, mode: &NetMode) {
    match mode {
        NetMode::Host { .. } => {
            app.insert_resource(DoorNet::new(DoorRole::Host)).add_systems(Update, (receive_door_requests, send_door_states));
        }
        NetMode::Client { .. } => {
            app.insert_resource(DoorNet::new(DoorRole::Client)).add_systems(Update, (send_door_requests, receive_door_states));
        }
        NetMode::Off => {}
    }
}

fn receive_door_requests(mut links: Query<(Entity, &mut MessageReceiver<DoorRequest>), With<ClientOf>>, players: Query<(&NetPlayer, &PlayerSlot)>, mut net: ResMut<DoorNet>) {
    for (link, mut rx) in &mut links {
        let peer = players.iter().find(|(_, s)| s.link == link).map(|(p, _)| p.peer);
        for r in rx.receive() {
            match peer {
                Some(p) => net.incoming.push((p, r)),
                None => runlog::kv("door_request_dropped", &format!("link={link:?} reason=no_player")),
            }
        }
    }
}

#[allow(clippy::type_complexity)] // Bevy system parameters
fn send_door_states(
    time: Res<Time<Real>>,
    net: Res<DoorNet>,
    mut senders: Query<&mut MessageSender<DoorStates>, (With<ClientOf>, With<Connected>, Without<HostClient>)>,
    mut last: Local<(Vec<crate::world::door::DoorNetState>, f32)>,
) {
    let now = time.elapsed_secs();
    let changed = net.state != last.0;
    if net.state.is_empty() || (!changed && now - last.1 < RESEND) {
        return;
    }
    let mut clients = 0;
    for mut tx in &mut senders {
        tx.send::<GameChannel>(DoorStates(net.state.clone()));
        clients += 1;
    }
    if changed {
        let diff: Vec<String> = net
            .state
            .iter()
            .enumerate()
            .filter(|(i, s)| last.0.get(*i) != Some(s))
            .map(|(i, s)| format!("{i}:open={}:weld={:.0}:sealed={}:dead={}", s.open, s.weld, s.sealed, s.dead))
            .collect();
        runlog::kv("net_doors_sent", &format!("clients={clients} doors={} changed=[{}]", net.state.len(), diff.join(" ")));
    }
    *last = (net.state.clone(), now);
}

#[allow(clippy::type_complexity)] // Bevy system parameters
fn send_door_requests(mut net: ResMut<DoorNet>, mut tx: Query<&mut MessageSender<DoorRequest>, (With<Client>, With<Connected>, Without<LinkOf>)>) {
    if net.outgoing.is_empty() {
        return;
    }
    let Ok(mut tx) = tx.single_mut() else { return };
    for r in std::mem::take(&mut net.outgoing) {
        runlog::kv("net_door_request_sent", &format!("{r:?}"));
        tx.send::<GameChannel>(r);
    }
}

fn receive_door_states(mut rx: Query<&mut MessageReceiver<DoorStates>, (With<Client>, Without<LinkOf>)>, mut net: ResMut<DoorNet>) {
    for mut r in &mut rx {
        for s in r.receive() {
            if net.from_host.as_ref() != Some(&s.0) {
                runlog::kv("net_doors_received", &format!("doors={}", s.0.len()));
            }
            net.from_host = Some(s.0);
        }
    }
}
