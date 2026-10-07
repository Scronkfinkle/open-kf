//! Step 2: every player sees the other players' pawns (their third-person
//! bodies), prototype level.
//!
//! Client-authoritative: each game sends where its own pawn is (a
//! `PawnUpdate`, `SEND_RATE` times a second); the server believes it and
//! copies it into that player's `NetPawn`, which lightyear copies to every
//! game. KF's real design is server-authoritative (the client sends its
//! moves, `ServerMove`; the server moves the pawn itself and corrects the
//! client, `ClientAdjustPosition`); that is later work.
//!
//! On every game, each other player's `NetPawn` gets a local "remote pawn"
//! entity with a `PawnState` (local: false) and a `PawnCharacter`; the body
//! code (`player::body`) draws it like the local player's. Updates arrive
//! a few times a second, so the remote pawn is drawn a little in the past
//! (`INTERP_DELAY`) and moved smoothly between the two updates around that
//! moment (interpolation).

use std::collections::{HashMap, VecDeque};

use bevy::prelude::*;
use lightyear::prelude::*;

use super::lobby::NetLobby;
use super::protocol::{NetPawn, NetPlayer, PawnChannel, PawnUpdate};
use super::server::PlayerSlot;
use super::NetMode;
use crate::engine::coords::SCALE;
use crate::engine::runlog;
use crate::player::body::{BodySystems, PawnBody, PawnCharacter, PawnState};

/// Updates per second each game sends about its own pawn (guess for a
/// prototype; KF's pawns have NetUpdateFrequency 100 but send only what
/// changed, and the server sends at most NetServerMaxTickRate 30 times a
/// second in KillingFloor.ini... a LAN host often more).
const SEND_RATE: f32 = 20.0;
/// How far in the past remote pawns are drawn (seconds): two send
/// intervals, so there is nearly always an update on each side of the
/// moment drawn even when one is late or lost.
const INTERP_DELAY: f64 = 0.1;
/// The receiver's estimate of "their clock minus ours" may drift up by
/// this much per update (clocks run at slightly different speeds; the
/// estimate is the smallest delay seen, which would otherwise never rise).
const OFFSET_DRIFT_PER_UPDATE: f64 = 0.001;
/// When the next update is late, a remote pawn keeps moving along its
/// last velocity for at most this long (seconds), then stops.
const MAX_EXTRAPOLATE: f64 = 0.1;
/// Remote-pawn log lines per second.
const LOG_RATE: f64 = 10.0;

pub(super) fn build(app: &mut App, mode: &NetMode) {
    app.init_resource::<RemotePawns>().init_resource::<HostOutbox>();
    let server = matches!(mode, NetMode::Host { .. });
    if server {
        app.init_resource::<RelayStats>();
        app.add_systems(Update, (send_own_pawn, receive_pawn_updates, track_remote_pawns, drive_remote_pawns).chain().before(BodySystems));
    } else {
        app.add_systems(Update, (send_own_pawn, track_remote_pawns, drive_remote_pawns).chain().before(BodySystems));
    }
}

/// The host's own update (it is the server: no message, stored directly).
#[derive(Resource, Default)]
struct HostOutbox(Option<PawnUpdate>);

/// Wall-clock seconds (Unix time), for comparing the logs of two games on
/// one machine (each game's own clock starts at its own launch).
fn wall() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// Bevy metres -> Unreal units (X forward, Y right, Z up), for the log.
fn unreal(v: [f32; 3]) -> String {
    let u = Vec3::from_array(v) / SCALE;
    format!("({:.0},{:.0},{:.0})", -u.z, u.x, u.y)
}

/// This game's own connection to the server (a client game only).
type MyConnection = (With<Client>, With<Connected>, Without<LinkOf>);

/// Sends my pawn's state `SEND_RATE` times a second once my match has
/// started (the host keeps it for `receive_pawn_updates`).
#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn send_own_pawn(
    time: Res<Time<Real>>,
    mode: Res<NetMode>,
    lobby: Res<NetLobby>,
    pawns: Query<&PawnState>,
    mut senders: Query<&mut MessageSender<PawnUpdate>, MyConnection>,
    mut outbox: ResMut<HostOutbox>,
    mut acc: Local<f32>,
    mut seq: Local<u32>,
) {
    if !lobby.start_sent {
        return;
    }
    let Some(s) = pawns.iter().find(|s| s.local) else { return };
    *acc += time.delta_secs();
    let interval = 1.0 / SEND_RATE;
    if *acc < interval {
        return;
    }
    *acc = (*acc - interval).min(interval);
    *seq += 1;
    let u = PawnUpdate {
        seq: *seq,
        time: time.elapsed_secs_f64(),
        active: s.active,
        location: s.location.to_array(),
        velocity: s.velocity.to_array(),
        on_ground: s.on_ground,
        yaw: s.yaw,
        pitch: s.pitch,
        weapon_class: s.weapon_class.clone(),
        flash_count: s.flash_count,
        firing: s.firing,
        firing_mode: s.firing_mode,
        reloads: s.reloads,
        hits: s.hits,
        hit_from: s.hit_from.map(|v| v.to_array()),
        dead: s.dead,
    };
    let line = format!(
        "seq={} send_t={:.3} wall={:.3} active={} loc_unreal={} speed={:.0} yaw_deg={:.1} pitch_deg={:.1} on_ground={} weapon={} flash={} reloads={} dead={}",
        u.seq,
        u.time,
        wall(),
        u.active,
        unreal(u.location),
        s.velocity.length() / SCALE,
        u.yaw.to_degrees(),
        u.pitch.to_degrees(),
        u.on_ground,
        u.weapon_class.as_deref().unwrap_or("none"),
        u.flash_count,
        u.reloads,
        u.dead
    );
    match *mode {
        NetMode::Host { .. } => outbox.0 = Some(u),
        _ => {
            let Ok(mut tx) = senders.single_mut() else { return };
            tx.send::<PawnChannel>(u);
        }
    }
    runlog::kv("net_pawn_sent", &line);
}

/// Updates received per player in the current second (server log).
#[derive(Resource, Default)]
struct RelayStats {
    counts: HashMap<u64, u32>,
    second: i64,
}

/// Server: the clients' pawn updates and the host's own go into each
/// player's `NetPawn` (made with the first update), which lightyear copies
/// to every game.
#[allow(clippy::type_complexity)] // Bevy system parameters
fn receive_pawn_updates(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut links: Query<(Entity, &mut MessageReceiver<PawnUpdate>), With<lightyear::prelude::server::ClientOf>>,
    mut outbox: ResMut<HostOutbox>,
    mut players: Query<(&NetPlayer, &mut PlayerSlot)>,
    mut pawns: Query<&mut NetPawn>,
    mut stats: ResMut<RelayStats>,
) {
    let mut got: Vec<(Option<Entity>, PawnUpdate)> = Vec::new();
    for (link, mut rx) in &mut links {
        // Several may arrive in one frame: only the newest matters.
        if let Some(u) = rx.receive().max_by_key(|u| u.seq) {
            got.push((Some(link), u));
        }
    }
    if let Some(u) = outbox.0.take() {
        got.push((None, u));
    }
    for (link, u) in got {
        let Some((p, mut slot)) = players.iter_mut().find(|(_, s)| match link {
            Some(l) => s.link == l,
            None => s.host,
        }) else {
            runlog::kv("net_pawn_dropped", &format!("link={link:?} reason=no_player"));
            continue;
        };
        *stats.counts.entry(p.peer).or_default() += 1;
        match slot.pawn.and_then(|e| pawns.get_mut(e).ok()) {
            Some(mut np) => {
                if u.seq > np.state.seq {
                    np.state = u;
                }
            }
            None => {
                let e = commands.spawn((Name::new(format!("NetPawn {}", p.peer)), NetPawn { peer: p.peer, state: u.clone() }, Replicate::to_clients(NetworkTarget::All))).id();
                slot.pawn = Some(e);
                runlog::kv("net_pawn_created", &format!("peer={} name=\"{}\" entity={e:?} loc_unreal={}", p.peer, p.name, unreal(u.location)));
            }
        }
    }
    let second = time.elapsed_secs() as i64;
    if second != stats.second {
        stats.second = second;
        if !stats.counts.is_empty() {
            let mut v: Vec<(u64, u32)> = stats.counts.drain().collect();
            v.sort_unstable();
            runlog::kv("net_pawn_relay", &v.iter().map(|(p, n)| format!("peer={p}:updates={n}")).collect::<Vec<_>>().join(" "));
        }
    }
}

/// One received update and when it arrived (our real clock).
#[derive(Clone, Debug)]
struct Snapshot {
    received: f64,
    u: PawnUpdate,
}

/// Another player's pawn on this game (fed from their `NetPawn`).
#[derive(Component, Debug)]
pub struct RemotePawn {
    pub peer: u64,
    snaps: VecDeque<Snapshot>,
    last_seq: u32,
    /// The smallest "arrival time minus their send time" seen (their
    /// clock -> ours, plus the shortest trip), drifting up slowly.
    offset: Option<f64>,
    /// Updates received this second, and the last second's count.
    count: u32,
    rate: u32,
    count_second: i64,
    /// Frames drawn past the newest update (it was late) since the last log.
    starved: u32,
    last_log: f64,
}

/// Peer id -> the remote pawn entity on this game.
#[derive(Resource, Default)]
struct RemotePawns(HashMap<u64, Entity>);

/// Every other player's `NetPawn` gets a remote pawn here (removed when
/// the `NetPawn` goes: the player left); new updates are queued.
#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn track_remote_pawns(
    mut commands: Commands,
    time: Res<Time<Real>>,
    lobby: Res<NetLobby>,
    net_pawns: Query<&NetPawn>,
    players: Query<&NetPlayer>,
    mut remotes: ResMut<RemotePawns>,
    mut pawns: Query<(&mut RemotePawn, &mut PawnCharacter, Option<&PawnBody>)>,
) {
    let Some(me) = lobby.my_peer else { return };
    let now = time.elapsed_secs_f64();
    let second = now as i64;
    let character_of = |peer: u64| {
        players.iter().find(|p| p.peer == peer).map_or(crate::player::character::DEFAULT_CHARACTER.to_string(), |p| {
            if p.character.is_empty() { crate::player::character::DEFAULT_CHARACTER.to_string() } else { p.character.clone() }
        })
    };
    for np in &net_pawns {
        if np.peer == me {
            continue;
        }
        let Some(&e) = remotes.0.get(&np.peer) else {
            let character = character_of(np.peer);
            let name = players.iter().find(|p| p.peer == np.peer).map_or("?".to_string(), |p| p.name.clone());
            let e = commands
                .spawn((
                    Name::new(format!("RemotePawn {}", np.peer)),
                    PawnState { local: false, ..default() },
                    PawnCharacter(character.clone()),
                    RemotePawn {
                        peer: np.peer,
                        snaps: VecDeque::from([Snapshot { received: now, u: np.state.clone() }]),
                        last_seq: np.state.seq,
                        offset: Some(now - np.state.time),
                        count: 1,
                        rate: 0,
                        count_second: second,
                        starved: 0,
                        last_log: 0.0,
                    },
                ))
                .id();
            remotes.0.insert(np.peer, e);
            runlog::kv("net_remote_pawn_spawned", &format!("peer={} name=\"{name}\" character={character} entity={e:?} loc_unreal={}", np.peer, unreal(np.state.location)));
            continue;
        };
        let Ok((mut r, mut ch, _)) = pawns.get_mut(e) else { continue };
        if np.state.seq > r.last_seq {
            r.last_seq = np.state.seq;
            let off = now - np.state.time;
            r.offset = Some(r.offset.map_or(off, |o| (o + OFFSET_DRIFT_PER_UPDATE).min(off)));
            r.snaps.push_back(Snapshot { received: now, u: np.state.clone() });
            if r.count_second != second {
                r.rate = r.count;
                r.count = 0;
                r.count_second = second;
            }
            r.count += 1;
        }
        let character = character_of(np.peer);
        if ch.0 != character {
            runlog::kv("net_remote_pawn_character", &format!("peer={} from={} to={character}", np.peer, ch.0));
            ch.0 = character;
        }
    }
    // Players whose NetPawn is gone (they left): remove their pawn and body.
    let present: Vec<u64> = net_pawns.iter().map(|n| n.peer).collect();
    remotes.0.retain(|peer, e| {
        if present.contains(peer) {
            return true;
        }
        if let Ok((_, _, body)) = pawns.get(*e)
            && let Some(b) = body
        {
            b.despawn(&mut commands, *e);
        }
        commands.entity(*e).despawn();
        runlog::kv("net_remote_pawn_removed", &format!("peer={peer} entity={e:?}"));
        false
    });
}

/// The shortest way from angle a to b (radians).
fn angle_lerp(a: f32, b: f32, t: f32) -> f32 {
    let d = (b - a + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
    a + d * t
}

/// Where a remote pawn is drawn at sender time `at`: between the two
/// updates around it (positions, velocity and view angles blended; the
/// rest, like the weapon and the shot counter, from the earlier one).
/// Before the first: the first; after the last: the last moved along its
/// velocity for up to `MAX_EXTRAPOLATE` (`true`: late).
fn sample(snaps: &VecDeque<Snapshot>, at: f64) -> Option<(PawnUpdate, bool)> {
    let first = snaps.front()?;
    if at <= first.u.time {
        return Some((first.u.clone(), false));
    }
    for w in snaps.iter().collect::<Vec<_>>().windows(2) {
        let (a, b) = (&w[0].u, &w[1].u);
        if at >= a.time && at <= b.time {
            let t = if b.time > a.time { ((at - a.time) / (b.time - a.time)) as f32 } else { 1.0 };
            let lerp3 = |x: [f32; 3], y: [f32; 3]| Vec3::from_array(x).lerp(Vec3::from_array(y), t).to_array();
            let mut u = a.clone();
            u.location = lerp3(a.location, b.location);
            u.velocity = lerp3(a.velocity, b.velocity);
            u.yaw = angle_lerp(a.yaw, b.yaw, t);
            u.pitch = a.pitch + (b.pitch - a.pitch) * t;
            // A pawn that (re)appears or teleports is not slid across the map.
            if !a.active || !b.active || Vec3::from_array(a.location).distance(Vec3::from_array(b.location)) > 400.0 * SCALE {
                u = if t < 0.5 { a.clone() } else { b.clone() };
            }
            return Some((u, false));
        }
    }
    snaps.back().map(|s| {
        let mut u = s.u.clone();
        if u.active && !u.dead {
            let ahead = (at - u.time).clamp(0.0, MAX_EXTRAPOLATE) as f32;
            u.location = (Vec3::from_array(u.location) + Vec3::from_array(u.velocity) * ahead).to_array();
        }
        (u, true)
    })
}

/// Moves each remote pawn's `PawnState` to where its player was
/// `INTERP_DELAY` seconds ago (on their clock), for the body code.
fn drive_remote_pawns(time: Res<Time<Real>>, mut pawns: Query<(&mut PawnState, &mut RemotePawn)>) {
    let now = time.elapsed_secs_f64();
    for (mut s, mut r) in &mut pawns {
        let Some(offset) = r.offset else { continue };
        let at = now - offset - INTERP_DELAY;
        // Keep one update older than the moment drawn.
        while r.snaps.len() > 2 && r.snaps[1].u.time <= at {
            r.snaps.pop_front();
        }
        let Some((u, late)) = sample(&r.snaps, at) else { continue };
        if late {
            r.starved += 1;
        }
        s.active = u.active;
        s.location = Vec3::from_array(u.location);
        s.velocity = Vec3::from_array(u.velocity);
        s.on_ground = u.on_ground;
        s.yaw = u.yaw;
        s.pitch = u.pitch;
        if s.weapon_class != u.weapon_class {
            s.weapon_class = u.weapon_class.clone();
        }
        s.flash_count = u.flash_count;
        s.firing = u.firing;
        s.firing_mode = u.firing_mode;
        s.reloads = u.reloads;
        s.hits = u.hits;
        s.hit_from = u.hit_from.map(Vec3::from_array);
        s.dead = u.dead;
        if now - r.last_log >= 1.0 / LOG_RATE {
            r.last_log = now;
            let newest = r.snaps.back().map_or(0, |n| n.u.seq);
            let newest_age = r.snaps.back().map_or(0.0, |n| now - n.received);
            runlog::kv(
                "net_remote_pawn",
                &format!(
                    "peer={} wall={:.3} at={at:.3} shown_unreal={} speed={:.0} yaw_deg={:.1} pitch_deg={:.1} active={} on_ground={} weapon={} flash={} reloads={} dead={} newest_seq={newest} newest_age_ms={:.0} buffered={} updates_per_s={} late_frames={}",
                    r.peer,
                    wall(),
                    unreal(u.location),
                    s.velocity.length() / SCALE,
                    u.yaw.to_degrees(),
                    u.pitch.to_degrees(),
                    u.active,
                    u.on_ground,
                    u.weapon_class.as_deref().unwrap_or("none"),
                    u.flash_count,
                    u.reloads,
                    u.dead,
                    newest_age * 1000.0,
                    r.snaps.len(),
                    r.rate,
                    r.starved,
                ),
            );
            r.starved = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(seq: u32, time: f64, x: f32, yaw: f32) -> Snapshot {
        Snapshot { received: time, u: PawnUpdate { seq, time, active: true, location: [x, 0.0, 0.0], yaw, ..default() } }
    }

    #[test]
    fn sample_blends_between_updates() {
        let snaps = VecDeque::from([snap(1, 1.0, 0.0, 3.0), snap(2, 1.1, 1.0, -3.0)]);
        let (u, late) = sample(&snaps, 1.05).unwrap();
        assert!(!late);
        assert!((u.location[0] - 0.5).abs() < 1e-5);
        // 3.0 -> -3.0 rad goes the short way, through pi.
        let d = (u.yaw.abs() - std::f32::consts::PI).abs();
        assert!(d < 1e-4, "yaw {}", u.yaw);
        let (u, late) = sample(&snaps, 2.0).unwrap();
        assert!(late);
        assert_eq!(u.seq, 2);
        // Late: coasts along the velocity for at most MAX_EXTRAPOLATE.
        let mut moving = snaps.clone();
        moving[1].u.velocity = [10.0, 0.0, 0.0];
        assert!((sample(&moving, 1.15).unwrap().0.location[0] - 1.5).abs() < 1e-4);
        assert!((sample(&moving, 9.0).unwrap().0.location[0] - 2.0).abs() < 1e-4);
        assert_eq!(sample(&snaps, 0.0).unwrap().0.seq, 1);
    }

    #[test]
    fn sample_does_not_slide_across_a_teleport() {
        let snaps = VecDeque::from([snap(1, 1.0, 0.0, 0.0), snap(2, 1.1, 100.0, 0.0)]);
        assert_eq!(sample(&snaps, 1.02).unwrap().0.location[0], 0.0);
        assert_eq!(sample(&snaps, 1.08).unwrap().0.location[0], 100.0);
    }
}
