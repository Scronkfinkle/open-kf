//! Healing other players over the network (game/healing.rs). Each game
//! knows the other players from their `NetPawn` (position, health) and
//! `NetPlayer` (name): that list is `Teammates`. A heal goes healer ->
//! host -> the healed player's game, which owns its health (KF: the server
//! calls GiveHealth on the healed pawn; here the healed player's own game
//! does, as for the zeds' hits).

use bevy::prelude::*;
use lightyear::connection::host::HostClient;
use lightyear::prelude::server::ClientOf;
use lightyear::prelude::*;

use super::NetMode;
use super::lobby::NetLobby;
use super::protocol::{GameChannel, HealRequest, NetPawn, NetPlayer, PlayerEvent};
use super::server::PlayerSlot;
use crate::engine::runlog;
use crate::game::healing::{HealTeammate, HealedByTeammate, Teammate, Teammates};

type RemoteLinks = (With<ClientOf>, With<Connected>, Without<HostClient>);
type MyConnection = (With<Client>, With<Connected>, Without<LinkOf>);

pub(super) fn build(app: &mut App, mode: &NetMode) {
    match mode {
        NetMode::Host { .. } => {
            app.add_systems(Update, (fill_teammates, route_heals));
        }
        NetMode::Client { .. } => {
            app.add_systems(Update, (fill_teammates, send_heals));
        }
        NetMode::Off => {}
    }
}

/// Every other player's pawn, as the latest update has it.
fn fill_teammates(lobby: Res<NetLobby>, pawns: Query<&NetPawn>, players: Query<&NetPlayer>, mut mates: ResMut<Teammates>) {
    let me = lobby.my_peer;
    mates.0 = pawns
        .iter()
        .filter(|p| Some(p.peer) != me)
        .map(|p| Teammate {
            peer: p.peer,
            name: players.iter().find(|r| r.peer == p.peer).map_or_else(|| "Player".to_string(), |r| r.name.clone()),
            centre: Vec3::from_array(p.state.location),
            health: p.state.health as f32,
            alive: p.state.active && !p.state.dead,
        })
        .collect();
}

/// Client: my heals go to the host.
fn send_heals(mut heals: MessageReader<HealTeammate>, mut tx: Query<&mut MessageSender<HealRequest>, MyConnection>) {
    for h in heals.read() {
        let sent = tx.single_mut().map(|mut tx| tx.send::<GameChannel>(HealRequest { target: h.peer, amount: h.heal_sum, source: h.source.to_string() })).is_ok();
        runlog::kv("net_heal_sent", &format!("target={} amount={} source={} sent={sent}", h.peer, h.heal_sum, h.source));
    }
}

/// Host: the host's own heals and the clients' requests go to the healed
/// player (the host's own player: applied here).
#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn route_heals(
    mut own: MessageReader<HealTeammate>,
    mut links: Query<(Entity, &mut MessageReceiver<HealRequest>), With<ClientOf>>,
    players: Query<(&NetPlayer, &PlayerSlot)>,
    records: Query<&NetPlayer>,
    lobby: Res<NetLobby>,
    mut senders: Query<&mut MessageSender<PlayerEvent>, RemoteLinks>,
    mut healed: MessageWriter<HealedByTeammate>,
) {
    let name_of = |peer: u64| records.iter().find(|p| p.peer == peer).map_or_else(|| "Player".to_string(), |p| p.name.clone());
    let mut out: Vec<(u64, f32, String, String)> = Vec::new();
    let me = lobby.my_peer.unwrap_or(0);
    for h in own.read() {
        out.push((h.peer, h.heal_sum, name_of(me), h.source.to_string()));
    }
    for (link, mut rx) in &mut links {
        let from = players.iter().find(|(_, s)| s.link == link).map(|(p, _)| p.peer);
        for r in rx.receive() {
            let Some(from) = from else { continue };
            // Trusting, as the zed hits: at most a syringe's 20 x 1.75.
            out.push((r.target, r.amount.clamp(0.0, 100.0), name_of(from), r.source));
        }
    }
    for (target, amount, healer, source) in out {
        if target == me {
            healed.write(HealedByTeammate { amount, healer: healer.clone(), source: source.clone() });
            runlog::kv("net_heal_routed", &format!("target={target} (host) amount={amount} healer=\"{healer}\" source={source}"));
            continue;
        }
        let link = players.iter().find(|(p, _)| p.peer == target).map(|(_, s)| s.link);
        let sent = link
            .and_then(|l| senders.get_mut(l).ok())
            .map(|mut tx| tx.send::<GameChannel>(PlayerEvent::Healed { amount, healer: healer.clone(), source: source.clone() }))
            .is_some();
        runlog::kv("net_heal_routed", &format!("target={target} amount={amount} healer=\"{healer}\" source={source} sent={sent}"));
    }
}
