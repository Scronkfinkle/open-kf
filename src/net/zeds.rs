//! Step 3: one shared match (docs/multiplayer-prototype.md, step 3).
//!
//! Host: shares its wave state (`NetWave`), sends every client a
//! `ZedSnapshot` 20 times a second, tells the zeds where the other players
//! are (`RemotePlayers`), sends each client the hits its player takes
//! (`PlayerEvent`), applies the hits clients report on zeds (`NetHit`,
//! client-trusted) and credits their kills (`KillCredit`).
//!
//! Client: copies the host's wave state into its own (`RemoteWave`),
//! smooths the snapshots into the puppet zeds (`PuppetFeed`, drawn 0.1 s
//! in the past like the remote pawns), reports its hits on puppets, and
//! applies the hits and kill credits the host sends.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::prelude::*;
use lightyear::connection::host::HostClient;
use lightyear::prelude::server::ClientOf;
use lightyear::prelude::*;

use super::NetMode;
use super::pawns::RemotePawn;
use super::protocol::{GameChannel, KillCredit, NetGame, NetPlayer, NetWave, PlayerEvent, ProjectileFx, Stamped, ZedChannel, ZedSnapshot};
use super::server::PlayerSlot;
use crate::engine::coords::SCALE;
use crate::engine::runlog;
use crate::game::combat::NetHit;
use crate::player::body::PawnState;
use crate::zeds::zed::{PuppetFeed, PuppetSample, Zed, ZedNet, ZedSystems};

/// Snapshots per second (as the pawn updates).
const SEND_RATE: f32 = 20.0;
/// How far in the past puppets are drawn (seconds; as the remote pawns).
const INTERP_DELAY: f64 = 0.1;
/// The client's clock-offset estimate may rise by this much per snapshot
/// (see pawns.rs).
const OFFSET_DRIFT_PER_SNAPSHOT: f64 = 0.001;
/// A dead zed is still sent this long, so a client that missed the moment
/// still sees the death (then it is left out; a puppet missing from a
/// snapshot is gone on the host).
const SEND_DEAD_FOR: f64 = 3.0;
/// When snapshots stop coming, puppets keep moving along their last
/// velocity for at most this long (seconds).
const MAX_EXTRAPOLATE: f64 = 0.1;

pub(super) fn build(app: &mut App, mode: &NetMode) {
    match mode {
        NetMode::Host { .. } => {
            app.init_resource::<SendStats>()
                .add_systems(
                    Update,
                    (share_wave, feed_remote_players, receive_hits, credit_kills, forward_player_events, send_snapshots).chain().before(ZedSystems).before(crate::game::waves::wave_timer),
                )
                .add_systems(Update, forward_projectiles.after(ZedSystems));
        }
        NetMode::Client { .. } => {
            app.init_resource::<SnapshotBuffer>()
                .add_systems(crate::world::map_change::MapUnload, forget_old_map_zeds)
                .add_systems(Update, (follow_wave, receive_snapshots, feed_puppets, send_hits, receive_player_events, receive_kill_credits, receive_projectiles).chain().before(ZedSystems).before(crate::game::waves::wave_timer));
        }
        NetMode::Off => {}
    }
}

/// Wall-clock seconds (Unix time), to compare two games' logs.
fn wall() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// The links of connected clients (not the host's own player).
type RemoteLinks = (With<ClientOf>, With<Connected>, Without<HostClient>);

/// The link of a player (server side).
fn link_of(players: &Query<(&NetPlayer, &PlayerSlot)>, peer: u64) -> Option<Entity> {
    players.iter().find(|(p, _)| p.peer == peer).map(|(_, s)| s.link)
}

// ---------------------------------------------------------------- host

/// The wave state onto the `NetGame` entity (replicated when it changes).
fn share_wave(mut commands: Commands, game: Res<crate::game::waves::WaveGame>, shops: Res<crate::game::trader::Shops>, mut records: Query<(Entity, Option<&mut NetWave>), With<NetGame>>) {
    let share = game.share(&shops);
    for (e, wave) in &mut records {
        match wave {
            None => {
                commands.entity(e).insert(NetWave(share.clone()));
            }
            Some(mut w) => {
                if w.0 != share {
                    if w.0.phase != share.phase || w.0.wave_num != share.wave_num || w.0.started != share.started {
                        runlog::kv("net_wave_shared", &format!("{share:?}"));
                    }
                    w.0 = share.clone();
                }
            }
        }
    }
}

/// The other players' pawns for the zeds (where they are drawn on the
/// host: 0.1 s behind their own game, plus the trip).
fn feed_remote_players(pawns: Query<(&RemotePawn, &PawnState)>, mut remote: ResMut<crate::game::combat::RemotePlayers>) {
    remote.0.clear();
    for (r, s) in &pawns {
        if !s.active {
            continue;
        }
        remote.0.push(crate::game::combat::RemotePlayer { peer: r.peer, centre: s.location, velocity: s.velocity, alive: !s.dead });
    }
}

/// Clients' hits on zeds: the same `damage_zed` call on the host's zed,
/// with that client's perk (prototype rule: the client is trusted).
fn receive_hits(
    mut links: Query<(Entity, &mut MessageReceiver<Stamped<NetHit>>), With<ClientOf>>,
    players: Query<(&NetPlayer, &PlayerSlot)>,
    mut zeds: Query<&mut Zed>,
    mut kills: ResMut<crate::game::combat::KillCount>,
    travel: Res<super::NetTravel>,
) {
    for (link, mut rx) in &mut links {
        let Some(peer) = players.iter().find(|(_, s)| s.link == link).map(|(p, _)| p.peer) else {
            rx.receive().for_each(drop);
            continue;
        };
        // A hit on another map's zed (zeds are numbered per map): dropped.
        for hit in rx.receive().filter_map(|m| travel.from_client(m, "zed_hit")) {
            let Some(mut z) = zeds.iter_mut().find(|z| z.id as u32 == hit.zed) else {
                runlog::kv("net_zed_hit_dropped", &format!("peer={peer} zed={} reason=no_such_zed", hit.zed));
                continue;
            };
            if z.health <= 0.0 {
                runlog::kv("net_zed_hit_dropped", &format!("peer={peer} zed={} reason=already_dead weapon={}", hit.zed, hit.weapon));
                continue;
            }
            let before = z.health;
            z.last_hit = Some((Vec3::from_array(hit.point), Vec3::from_array(hit.dir)));
            z.net.next_hit_by = Some(peer);
            crate::game::combat::damage_zed(&mut z, hit.damage, hit.headshot, hit.headshot_mult, &hit.weapon, hit.distance, hit.source(), &mut kills);
            runlog::kv(
                "net_zed_hit_applied",
                &format!(
                    "peer={peer} zed={} weapon={} damage={:.1} headshot={} health={before:.1}->{:.1} killed={} decapitated={}",
                    hit.zed,
                    hit.weapon,
                    hit.damage,
                    hit.headshot,
                    z.health.max(0.0),
                    z.is_dead(),
                    z.decapitated
                ),
            );
        }
    }
}

/// A client's kill: their kill count and dosh (KF's ScoreKill on their
/// PRI), sent to their game; this game's player gets nothing for it.
fn credit_kills(mut zeds: Query<&mut Zed>, players: Query<(&NetPlayer, &PlayerSlot)>, mut senders: Query<&mut MessageSender<Stamped<KillCredit>>, RemoteLinks>, travel: Res<super::NetTravel>) {
    for mut z in &mut zeds {
        let Some(peer) = z.net.damaged_by else { continue };
        if !z.is_dead() || !z.killed_by_player || z.kill_paid {
            continue;
        }
        z.kill_paid = true;
        let credit = KillCredit { zed_id: z.id as u32, scoring_value: z.scoring_value, headshot: z.headshot_kill };
        let sent = link_of(&players, peer).and_then(|l| senders.get_mut(l).ok()).map(|mut tx| tx.send::<GameChannel>(travel.stamp(credit))).is_some();
        runlog::kv("net_kill_credit", &format!("peer={peer} zed={} scoring_value={} sent={sent}", z.id, z.scoring_value));
    }
}

/// Hits the zeds dealt to other players' pawns go to those players' games.
fn forward_player_events(
    mut hits: MessageReader<crate::game::combat::PlayerDamaged>,
    mut pushes: MessageReader<crate::player::walk::PlayerPush>,
    mut grabs: MessageReader<crate::game::combat::RemoteGrab>,
    players: Query<(&NetPlayer, &PlayerSlot)>,
    mut senders: Query<&mut MessageSender<Stamped<PlayerEvent>>, RemoteLinks>,
    travel: Res<super::NetTravel>,
) {
    let mut out: Vec<(u64, PlayerEvent)> = Vec::new();
    for h in hits.read() {
        if let Some(peer) = h.to_peer {
            out.push((
                peer,
                PlayerEvent::Hurt {
                    amount: h.amount,
                    zed_id: h.zed_id as u32,
                    kind: h.kind,
                    armor_stops: h.armor_stops,
                    dam_type: h.dam_type,
                    source: h.source.map(|v| v.to_array()),
                    dam: h.dam.map(|d| (d.chain.0.clone(), d.melee)),
                },
            ));
        }
    }
    for p in pushes.read() {
        if let Some(peer) = p.to_peer {
            out.push((peer, PlayerEvent::Push { momentum: p.momentum.to_array() }));
        }
    }
    for g in grabs.read() {
        out.push((g.peer, PlayerEvent::Grab { seconds: g.seconds, zed_id: g.zed_id as u32 }));
    }
    for (peer, ev) in out {
        let sent = link_of(&players, peer).and_then(|l| senders.get_mut(l).ok()).map(|mut tx| tx.send::<GameChannel>(travel.stamp(ev.clone()))).is_some();
        runlog::kv("net_player_event_sent", &format!("peer={peer} sent={sent} event={ev:?}"));
    }
}

/// The host's zeds' projectiles go to every client as harmless copies (so
/// they see and hear them; the host's copies do the damage).
fn forward_projectiles(
    mut globs: MessageReader<crate::zeds::vomit::SpawnVomit>,
    mut fireballs: MessageReader<crate::zeds::fireball::SpawnFireball>,
    mut senders: Query<&mut MessageSender<Stamped<ProjectileFx>>, RemoteLinks>,
    travel: Res<super::NetTravel>,
) {
    let mut out: Vec<ProjectileFx> = globs.read().map(|g| ProjectileFx::Bile { at: g.at.to_array(), velocity: g.velocity.to_array(), zed_id: g.zed_id as u32 }).collect();
    out.extend(fireballs.read().map(|f| ProjectileFx::Fireball {
        at: f.at.to_array(),
        dir: f.dir.to_array(),
        zed_id: f.zed_id as u32,
        rocket: f.kind == crate::zeds::fireball::Projectile::BossRocket,
    }));
    for p in out {
        let mut clients = 0;
        for mut tx in &mut senders {
            tx.send::<GameChannel>(travel.stamp(p.clone()));
            clients += 1;
        }
        runlog::kv("net_projectile_sent", &format!("clients={clients} {p:?}"));
    }
}

/// Bytes and snapshots sent in the current second (host log).
#[derive(Resource, Default)]
struct SendStats {
    acc: f32,
    seq: u32,
    second: i64,
    snapshots: u32,
    bytes: usize,
    max_bytes: usize,
    max_zeds: usize,
    /// When each zed was first seen dead (host clock).
    dead_since: HashMap<usize, f64>,
}

/// `SEND_RATE` times a second: every living zed (and those dead for
/// under `SEND_DEAD_FOR`) to every client.
fn send_snapshots(time: Res<Time<Real>>, zeds: Query<&Zed>, mut senders: Query<&mut MessageSender<ZedSnapshot>, RemoteLinks>, mut stats: ResMut<SendStats>, travel: Res<super::NetTravel>) {
    let now = time.elapsed_secs_f64();
    stats.acc += time.delta_secs();
    let interval = 1.0 / SEND_RATE;
    if stats.acc < interval {
        return;
    }
    stats.acc = (stats.acc - interval).min(interval);
    let mut list: Vec<ZedNet> = Vec::new();
    let mut alive_ids = HashSet::new();
    for z in &zeds {
        alive_ids.insert(z.id);
        if z.is_dead() {
            let since = *stats.dead_since.entry(z.id).or_insert(now);
            if now - since > SEND_DEAD_FOR {
                continue;
            }
        }
        list.push(z.net_state());
    }
    stats.dead_since.retain(|id, _| alive_ids.contains(id));
    list.sort_by_key(|n| n.id);
    stats.seq += 1;
    let snap = ZedSnapshot { seq: stats.seq, travel: travel.loaded, time: now, zeds: list };
    let bytes = postcard::to_allocvec(&snap).map_or(0, |v| v.len());
    let mut clients = 0;
    for mut tx in &mut senders {
        tx.send::<ZedChannel>(snap.clone());
        clients += 1;
    }
    if clients > 0 {
        stats.snapshots += 1;
        stats.bytes += bytes * clients;
        stats.max_bytes = stats.max_bytes.max(bytes);
        stats.max_zeds = stats.max_zeds.max(snap.zeds.len());
        // One line per snapshot (the analysis compares positions by wall clock).
        let zs: Vec<String> = snap.zeds.iter().map(|n| format!("{}:{:.0},{:.0},{:.0}", n.id, n.pos[0] as f32 / 2.0, n.pos[1] as f32 / 2.0, n.pos[2] as f32 / 2.0)).collect();
        runlog::kv("net_zed_snapshot", &format!("seq={} send_t={now:.3} wall={:.3} zeds={} bytes={bytes} at=[{}]", snap.seq, wall(), snap.zeds.len(), zs.join(";")));
    }
    let second = now as i64;
    if second != stats.second {
        stats.second = second;
        if stats.snapshots > 0 {
            runlog::kv(
                "net_zed_send_rate",
                &format!("snapshots={} clients={clients} payload_bytes_per_s={} max_snapshot_bytes={} max_zeds={} (postcard payload; packet headers not counted)", stats.snapshots, stats.bytes, stats.max_bytes, stats.max_zeds),
            );
        }
        stats.snapshots = 0;
        stats.bytes = 0;
        stats.max_bytes = 0;
        stats.max_zeds = 0;
    }
}

// -------------------------------------------------------------- client

/// This game's own connection to the server.
type MyConnection = (With<Client>, With<Connected>, Without<LinkOf>);

/// The host's wave state, for waves.rs.
fn follow_wave(records: Query<&NetWave>, mut remote: ResMut<crate::game::waves::RemoteWave>) {
    let Some(w) = records.iter().next() else { return };
    if remote.0.as_ref() != Some(&w.0) {
        remote.0 = Some(w.0.clone());
    }
}

/// Received snapshots, oldest first, and the clock offset (as pawns.rs).
#[derive(Resource, Default)]
struct SnapshotBuffer {
    snaps: VecDeque<ZedSnapshot>,
    last_seq: u32,
    offset: Option<f64>,
    count: u32,
    bytes: usize,
    second: i64,
    late_frames: u32,
    /// Smoothness check: each puppet's last drawn centre, and this
    /// half-second's biggest and summed per-frame moves (Unreal units).
    prev: HashMap<u32, Vec3>,
    max_step: f32,
    sum_step: f32,
    steps: u32,
    motion_at: f64,
}

fn receive_snapshots(time: Res<Time<Real>>, mut rx: Query<&mut MessageReceiver<ZedSnapshot>, (With<Client>, Without<LinkOf>)>, mut buf: ResMut<SnapshotBuffer>, travel: Res<super::NetTravel>, mut dropped: Local<(u32, u32)>) {
    let now = time.elapsed_secs_f64();
    for mut r in &mut rx {
        for s in r.receive() {
            if s.seq <= buf.last_seq {
                continue;
            }
            // Another map's zeds (sent before or during a map change):
            // logged once per map.
            if !travel.current(s.travel) {
                if dropped.0 != travel.loaded || dropped.1 == 0 {
                    runlog::kv("net_zed_snapshot_dropped", &format!("travel={} loaded={} seq={}", s.travel, travel.loaded, s.seq));
                }
                *dropped = (travel.loaded, dropped.1 + 1);
                continue;
            }
            buf.last_seq = s.seq;
            let off = now - s.time;
            buf.offset = Some(buf.offset.map_or(off, |o| (o + OFFSET_DRIFT_PER_SNAPSHOT).min(off)));
            buf.count += 1;
            buf.bytes += postcard::to_allocvec(&s).map_or(0, |v| v.len());
            buf.snaps.push_back(s);
        }
    }
    let second = now as i64;
    if second != buf.second {
        buf.second = second;
        if buf.count > 0 || buf.late_frames > 0 {
            runlog::kv(
                "net_zed_receive_rate",
                &format!("snapshots={} payload_bytes_per_s={} buffered={} late_frames={} offset={:.3}", buf.count, buf.bytes, buf.snaps.len(), buf.late_frames, buf.offset.unwrap_or(0.0)),
            );
        }
        buf.count = 0;
        buf.bytes = 0;
        buf.late_frames = 0;
    }
}

/// A map change: the buffered snapshots and the puppet feed are the old
/// map's zeds (their puppets are despawned with the map); the sequence
/// number and the clock offset stay (the host's clock goes on).
fn forget_old_map_zeds(mut buf: ResMut<SnapshotBuffer>, mut feed: ResMut<PuppetFeed>) {
    let dropped = buf.snaps.len();
    buf.snaps.clear();
    buf.prev.clear();
    *feed = PuppetFeed::default();
    runlog::kv("net_zeds_forgotten", &format!("buffered_snapshots={dropped}"));
}

/// The shortest way between two Unreal yaws.
fn yaw_lerp(a: f32, b: f32, t: f32) -> f32 {
    let d = (b - a + 32768.0).rem_euclid(65536.0) - 32768.0;
    (a + d * t).rem_euclid(65536.0)
}

/// Each host zed where the host had it `INTERP_DELAY` ago (host clock),
/// blended between the two snapshots around that moment. Snapshot times
/// are real seconds; `speed` (the shared game speed, 0.2 in zed time)
/// turns the velocity into game units per game second (what the puppet's
/// ragdoll uses) and scales the coast past the newest snapshot.
fn sample(snaps: &VecDeque<ZedSnapshot>, at: f64, speed: f32) -> (HashMap<u32, PuppetSample>, bool) {
    let speed = speed.max(0.05);
    let mut out = HashMap::new();
    let Some(last) = snaps.back() else { return (out, false) };
    // The snapshot pair around `at` (or the newest, moved along).
    let pair = snaps.iter().zip(snaps.iter().skip(1)).find(|(a, b)| at >= a.time && at <= b.time);
    let (a, b, t, late) = match pair {
        Some((a, b)) => (a, Some(b), if b.time > a.time { ((at - a.time) / (b.time - a.time)) as f32 } else { 1.0 }, false),
        None if at < snaps[0].time => (&snaps[0], None, 0.0, false),
        None => (last, None, 0.0, true),
    };
    let next: HashMap<u32, &ZedNet> = b.map(|b| b.zeds.iter().map(|n| (n.id, n)).collect()).unwrap_or_default();
    let dt = b.map_or(0.0, |b| (b.time - a.time) as f32);
    for n in &a.zeds {
        let (centre, yaw, velocity) = match next.get(&n.id) {
            Some(m) if dt > 0.0 => {
                let (ca, cb) = (n.centre(), m.centre());
                // A jump of more than 400 units is not slid across.
                let c = if ca.distance(cb) > 400.0 * SCALE { if t < 0.5 { ca } else { cb } } else { ca.lerp(cb, t) };
                (c, yaw_lerp(n.yaw as f32, m.yaw as f32, t), (cb - ca) / dt / speed)
            }
            _ => (n.centre(), n.yaw as f32, Vec3::ZERO),
        };
        out.insert(n.id, PuppetSample { n: n.clone(), centre, yaw, velocity });
    }
    // Late: keep moving along the last known velocity for a moment.
    if late && snaps.len() >= 2 {
        let prev = &snaps[snaps.len() - 2];
        let dt = (last.time - prev.time) as f32;
        let ahead = (at - last.time).clamp(0.0, MAX_EXTRAPOLATE) as f32;
        let before: HashMap<u32, Vec3> = prev.zeds.iter().map(|n| (n.id, n.centre())).collect();
        for s in out.values_mut() {
            if let Some(p) = before.get(&s.n.id)
                && dt > 0.0
                && !s.n.dead()
            {
                s.velocity = (s.centre - *p) / dt / speed;
                s.centre += s.velocity * speed * ahead;
            }
        }
    }
    (out, late)
}

/// The snapshots -> `PuppetFeed` (zeds/zed/net.rs applies it).
fn feed_puppets(time: Res<Time<Real>>, zed_time: Res<crate::game::zed_time::ZedTime>, mut buf: ResMut<SnapshotBuffer>, mut feed: ResMut<PuppetFeed>) {
    let Some(offset) = buf.offset else { return };
    let at = time.elapsed_secs_f64() - offset - INTERP_DELAY;
    // Keep one snapshot older than the moment drawn.
    while buf.snaps.len() > 2 && buf.snaps[1].time <= at {
        buf.snaps.pop_front();
    }
    let (samples, late) = sample(&buf.snaps, at, zed_time.speed());
    if late {
        buf.late_frames += 1;
    }
    // How far each living puppet moves per frame (a jump shows as a max
    // far above the mean), logged twice a second with the game speed.
    let mut prev = std::mem::take(&mut buf.prev);
    for (id, s) in &samples {
        if s.n.dead() {
            continue;
        }
        if let Some(p) = prev.get(id) {
            let step = p.distance(s.centre) / SCALE;
            buf.max_step = buf.max_step.max(step);
            buf.sum_step += step;
            buf.steps += 1;
        }
        prev.insert(*id, s.centre);
    }
    prev.retain(|id, _| samples.contains_key(id));
    buf.prev = prev;
    let now = time.elapsed_secs_f64();
    if now - buf.motion_at >= 0.5 {
        buf.motion_at = now;
        if buf.steps > 0 {
            runlog::kv(
                "net_puppet_motion",
                &format!("wall={:.3} game_speed={:.2} puppets={} max_step_uu={:.2} mean_step_uu={:.2} steps={}", wall(), zed_time.speed(), samples.len(), buf.max_step, buf.sum_step / buf.steps as f32, buf.steps),
            );
        }
        buf.max_step = 0.0;
        buf.sum_step = 0.0;
        buf.steps = 0;
    }
    feed.samples = samples;
    feed.latest = buf.snaps.back().map(|s| s.zeds.iter().map(|n| n.id).collect());
    feed.at = at;
}

/// My hits on puppets go to the host.
fn send_hits(mut zeds: Query<&mut Zed>, mut tx: Query<&mut MessageSender<Stamped<NetHit>>, MyConnection>, travel: Res<super::NetTravel>) {
    let Ok(mut tx) = tx.single_mut() else { return };
    for mut z in &mut zeds {
        for hit in std::mem::take(&mut z.net.hits) {
            runlog::kv("net_zed_hit_sent", &format!("zed={} weapon={} damage={:.1} headshot={} puppet_health_now={:.1}", hit.zed, hit.weapon, hit.damage, hit.headshot, z.health.max(0.0)));
            tx.send::<GameChannel>(travel.stamp(hit));
        }
    }
}

/// Hits the host's zeds dealt to my player: applied here (health is this
/// game's own).
#[allow(clippy::type_complexity)] // Bevy system parameters
fn receive_player_events(
    mut rx: Query<&mut MessageReceiver<Stamped<PlayerEvent>>, (With<Client>, Without<LinkOf>)>,
    mut damage: MessageWriter<crate::game::combat::PlayerDamaged>,
    mut push: MessageWriter<crate::player::walk::PlayerPush>,
    mut pinned: ResMut<crate::game::combat::PlayerPinned>,
    vet: Res<crate::game::perks::Veterancy>,
    mut healed: MessageWriter<crate::game::healing::HealedByTeammate>,
    travel: Res<super::NetTravel>,
) {
    for mut r in &mut rx {
        // A hit or heal from another map: dropped.
        for ev in r.receive().filter_map(|m| travel.from_host(m, "player_event")) {
            runlog::kv("net_player_event", &format!("{ev:?}"));
            match ev {
                PlayerEvent::Hurt { amount, zed_id, kind, armor_stops, dam_type, source, dam } => {
                    // The damage type travels so this player's own perk can
                    // reduce it (KFGameType.ReduceDamage runs for the hurt pawn).
                    let dam = dam.map(|(chain, melee)| crate::game::perks::intern_dam_type(crate::game::perks::ClassChain(chain), melee));
                    damage.write(crate::game::combat::PlayerDamaged { amount, zed_id: zed_id as usize, kind, armor_stops, dam_type, source: source.map(Vec3::from_array), dam, to_peer: None });
                }
                PlayerEvent::Push { momentum } => {
                    push.write(crate::player::walk::PlayerPush { momentum: Vec3::from_array(momentum), to_peer: None });
                }
                PlayerEvent::Healed { amount, healer, source } => {
                    healed.write(crate::game::healing::HealedByTeammate { amount, healer, source });
                }
                PlayerEvent::Grab { seconds, zed_id } => {
                    // CanBeGrabbed: a Berserker is not grabbed by Clots.
                    if vet.vet.can_be_grabbed_by_clot() {
                        pinned.pin(seconds, zed_id as usize);
                    } else {
                        runlog::kv("perk_mod", &format!("kind=no_clot_grab perk={} zed={zed_id}", vet.vet.label()));
                    }
                }
            }
        }
    }
}

/// The host's zeds' projectiles: harmless copies here (vomit.rs and
/// fireball.rs leave a client's projectiles harmless).
#[allow(clippy::type_complexity)] // Bevy system parameters
fn receive_projectiles(
    mut rx: Query<&mut MessageReceiver<Stamped<ProjectileFx>>, (With<Client>, Without<LinkOf>)>,
    mut globs: MessageWriter<crate::zeds::vomit::SpawnVomit>,
    mut fireballs: MessageWriter<crate::zeds::fireball::SpawnFireball>,
    travel: Res<super::NetTravel>,
) {
    for mut r in &mut rx {
        for p in r.receive().filter_map(|m| travel.from_host(m, "projectile")) {
            runlog::kv("net_projectile", &format!("{p:?}"));
            match p {
                ProjectileFx::Bile { at, velocity, zed_id } => {
                    globs.write(crate::zeds::vomit::SpawnVomit { at: Vec3::from_array(at), velocity: Vec3::from_array(velocity), zed_id: zed_id as usize });
                }
                ProjectileFx::Fireball { at, dir, zed_id, rocket } => {
                    let kind = if rocket { crate::zeds::fireball::Projectile::BossRocket } else { crate::zeds::fireball::Projectile::HuskFire };
                    fireballs.write(crate::zeds::fireball::SpawnFireball { at: Vec3::from_array(at), dir: Vec3::from_array(dir), zed_id: zed_id as usize, kind });
                }
            }
        }
    }
}

/// The host credited me with a kill: kill count and dosh (dosh.rs's
/// ScoreKill, as for my own kills in single player).
fn receive_kill_credits(
    mut rx: Query<&mut MessageReceiver<Stamped<KillCredit>>, (With<Client>, Without<LinkOf>)>,
    mut kills: ResMut<crate::game::combat::KillCount>,
    mut dosh: ResMut<crate::game::dosh::Dosh>,
    options: Res<crate::game::waves::GameOptions>,
    travel: Res<super::NetTravel>,
) {
    for mut r in &mut rx {
        // A kill on the old map (the new map's count starts at 0): dropped.
        for c in r.receive().filter_map(|m| travel.from_host(m, "kill_credit")) {
            kills.0 += 1;
            let paid = dosh.kill(c.scoring_value, options.length);
            runlog::kv("dosh", &format!("reason=kill_credit zed={} amount={paid:.0} total={:.0} team={:.0} kills={} headshot={}", c.zed_id, dosh.score, dosh.team, kills.0, c.headshot));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zed(id: u32, x: i32, yaw: u16) -> ZedNet {
        ZedNet { id, pos: [x * 2, 0, 0], yaw, ..default() }
    }

    #[test]
    fn puppets_blend_between_snapshots() {
        let snaps = VecDeque::from([
            ZedSnapshot { seq: 1, travel: 0, time: 1.0, zeds: vec![zed(7, 0, 65000), zed(8, 50, 0)] },
            ZedSnapshot { seq: 2, travel: 0, time: 1.1, zeds: vec![zed(7, 100, 500)] },
        ]);
        let (s, late) = sample(&snaps, 1.05, 1.0);
        assert!(!late);
        let p = &s[&7];
        // Halfway: x 50 Unreal units; the yaw goes the short way through 0.
        assert!((crate::zeds::zed::ZedNet { pos: [100, 0, 0], ..default() }.centre() - p.centre).length() < 1e-4);
        assert!((p.yaw - 65518.0).abs() < 1.0, "yaw {}", p.yaw);
        // A zed only in the earlier snapshot stays where it was.
        assert_eq!(s[&8].velocity, Vec3::ZERO);
        // After the newest: late.
        assert!(sample(&snaps, 2.0, 1.0).1);
        // In zed time the same movement is five times faster in game time.
        let (z, _) = sample(&snaps, 1.05, 0.2);
        let (n, _) = sample(&snaps, 1.05, 1.0);
        assert!((z[&7].velocity - n[&7].velocity * 5.0).length() < 1e-3);
    }

    #[test]
    fn yaw_takes_the_short_way() {
        assert!((yaw_lerp(65000.0, 500.0, 0.5) - 65518.0).abs() < 1.0);
        assert!((yaw_lerp(500.0, 65000.0, 0.5) - 65518.0).abs() < 1.0);
        assert!((yaw_lerp(100.0, 300.0, 0.5) - 200.0).abs() < 1e-3);
    }
}
