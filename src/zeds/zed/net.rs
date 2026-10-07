//! Multiplayer (step 3, docs/multiplayer-prototype.md): what a zed sends
//! to the clients, and the client's "puppet" zeds that copy it.
//!
//! The host runs every zed. 20 times a second `net/zeds.rs` packs each
//! one with `Zed::net_state` into a small `ZedNet` record. A client makes
//! a puppet zed (`SpawnPuppet`, the normal spawn code, so meshes, gore and
//! sounds are the same) and `Zed::apply_net` copies the host's state into
//! it; the puppet does not think, walk or attack (`think.rs` skips it).

use serde::{Deserialize, Serialize};

use super::*;

/// Network-only parts of a zed (all default in single player).
#[derive(Debug, Default)]
pub struct ZedNetSide {
    /// Client: a drawn copy of a host zed (moved by net/zeds.rs).
    pub puppet: bool,
    /// The kind's place in `ZED_CLASSES` (the same on every machine).
    pub kind: u8,
    /// Client: hits this game's weapons did to the puppet, to be reported
    /// to the host (combat.rs `damage_zed` fills it, net/zeds.rs sends).
    pub hits: Vec<crate::game::combat::NetHit>,
    /// Host: the player whose reported hit is being applied (set just
    /// before `damage_zed`), and the player who last damaged the zed (KF's
    /// LastDamagedBy). None: this game's own player.
    pub next_hit_by: Option<u64>,
    pub damaged_by: Option<u64>,
    /// Host: the player hunted (KF's Controller.Enemy; None: this game's
    /// own player), seconds to the next sight check, and a player who has
    /// just hurt it (SetEnemy).
    pub enemy: Option<u64>,
    pub enemy_chosen: bool,
    pub enemy_check: f32,
    /// Some(player): that player just hurt it (None inside: this game's own).
    pub provoked_by: Option<Option<u64>>,
    /// Client: the host's last upper-body sequence (a new one starts here).
    pub host_overlay: Option<u16>,
}

/// One zed as the host sends it (packed small: postcard writes whole
/// numbers in as few bytes as they need).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct ZedNet {
    pub id: u32,
    /// Place in `ZED_CLASSES`.
    pub kind: u8,
    /// Cylinder centre in Unreal units x 2 (half-unit steps).
    pub pos: [i32; 3],
    /// Unreal rotation units (0..65535).
    pub yaw: u16,
    pub state: u8,
    /// Main animation: sequence + 1 (0: none), frame x 100.
    pub seq: u16,
    pub frame: u32,
    /// Upper-body layer: sequence, frame x 100, root bone.
    pub overlay: Option<(u16, u32, u16)>,
    /// Health and head health x 10, rounded up.
    pub health: u32,
    pub head_health: u32,
    pub flags: u8,
}

pub const FLAG_LOOPING: u8 = 1;
pub const FLAG_DEAD: u8 = 2;
pub const FLAG_HEADLESS: u8 = 4;
pub const FLAG_CLOAKED: u8 = 8;
pub const FLAG_RAGING: u8 = 16;
pub const FLAG_BURNING: u8 = 32;
pub const FLAG_BRAINDEAD: u8 = 64;

impl ZedNet {
    /// Cylinder centre, Bevy space.
    pub fn centre(&self) -> Vec3 {
        coords::pos([self.pos[0] as f32 / 2.0, self.pos[1] as f32 / 2.0, self.pos[2] as f32 / 2.0])
    }

    pub fn dead(&self) -> bool {
        self.flags & FLAG_DEAD != 0
    }
}

/// Client: make a puppet for a host zed (read by `spawn_zeds`).
#[derive(Message, Clone, Copy, Debug)]
pub struct SpawnPuppet {
    pub id: usize,
    pub kind: u8,
    /// Cylinder centre (Bevy) and yaw (Unreal units).
    pub centre: Vec3,
    pub yaw: f32,
}

fn state_code(s: ZedState) -> u8 {
    match s {
        ZedState::Idle => 0,
        ZedState::Chase => 1,
        ZedState::Melee => 2,
        ZedState::Falling => 3,
        ZedState::KnockedDown => 4,
        ZedState::Landing => 5,
        ZedState::Enraging => 6,
        ZedState::BossBusy => 7,
        ZedState::DoorBashing => 8,
        ZedState::Dead => 9,
    }
}

fn state_of(code: u8) -> ZedState {
    match code {
        1 => ZedState::Chase,
        2 => ZedState::Melee,
        3 => ZedState::Falling,
        4 => ZedState::KnockedDown,
        5 => ZedState::Landing,
        6 => ZedState::Enraging,
        7 => ZedState::BossBusy,
        8 => ZedState::DoorBashing,
        9 => ZedState::Dead,
        _ => ZedState::Idle,
    }
}

/// The place of a kind in `ZED_CLASSES`.
pub(super) fn kind_code(kind: ZedKind) -> u8 {
    ZED_CLASSES.iter().position(|(k, _)| *k == kind).unwrap_or(0) as u8
}

impl Zed {
    /// Host: this zed packed for the clients.
    pub fn net_state(&self) -> ZedNet {
        let u = ue_pos(self.centre);
        let mut flags = 0;
        for (on, f) in [
            (self.looping, FLAG_LOOPING),
            (self.is_dead(), FLAG_DEAD),
            (self.decapitated, FLAG_HEADLESS),
            (self.cloaked, FLAG_CLOAKED),
            (self.fp_rage.is_some() || self.state == ZedState::Enraging, FLAG_RAGING),
            (self.burn_down > 0, FLAG_BURNING),
            (self.braindead, FLAG_BRAINDEAD),
        ] {
            if on {
                flags |= f;
            }
        }
        ZedNet {
            id: self.id as u32,
            kind: self.net.kind,
            pos: [(u.x * 2.0).round() as i32, (u.y * 2.0).round() as i32, (u.z * 2.0).round() as i32],
            yaw: self.yaw.rem_euclid(65536.0) as u16,
            state: state_code(self.state),
            seq: self.sequence.map_or(0, |s| s as u16 + 1),
            frame: (self.frame.max(0.0) * 100.0) as u32,
            overlay: self.overlay.map(|(s, f, r)| (s as u16, (f.max(0.0) * 100.0) as u32, r as u16)),
            health: (self.health.max(0.0) * 10.0).ceil() as u32,
            head_health: (self.head_health.max(0.0) * 10.0).ceil() as u32,
            flags,
        }
    }

    /// Client: copy the host's state into this puppet. `centre`, `yaw`
    /// and `velocity` are the blended values for the moment drawn; the
    /// rest comes from `n`. Returns what changed, for the log.
    pub fn apply_net(&mut self, n: &ZedNet, centre: Vec3, yaw: f32, velocity: Vec3, anim_length: impl Fn(usize) -> f32) -> Vec<String> {
        let mut changed = Vec::new();
        if self.is_dead() {
            return changed;
        }
        let host_health = n.health as f32 / 10.0;
        // The client's own hits lower the puppet's health at once; the
        // host's figure (a moment old) only lowers it further (others'
        // hits). The Patriarch heals, so his comes from the host as is.
        self.health = if self.boss.is_some() { host_health } else { self.health.min(host_health) };
        self.head_health = self.head_health.min(n.head_health as f32 / 10.0);
        if n.flags & FLAG_HEADLESS != 0 && !self.decapitated {
            // Decapitated by someone else's hit: the head comes off here
            // with its gore (KFMonster.DecapFX runs on every client).
            self.remove_head();
            self.gore_hits.push(GoreHit {
                damage: 0.0,
                health_after: self.health,
                melee: false,
                decapitated: true,
                point: self.head.map_or(self.centre, |(h, _)| h),
                dir: Vec3::ZERO,
                attacker: self.centre,
            });
            // The host decides when a headless zed dies.
            self.bleed_out = None;
            changed.push("headless".to_string());
        }
        if n.dead() {
            self.velocity = velocity;
            self.kill();
            changed.push("dead".to_string());
            return changed;
        }
        self.bleed_out = None;
        self.centre = centre;
        self.yaw = yaw;
        self.velocity = velocity;
        self.braindead = n.flags & FLAG_BRAINDEAD != 0;
        let state = state_of(n.state);
        if state != self.state {
            self.state = state;
        }
        // Main animation: the host's sequence; its frame when the sequence
        // changes or the two drift apart by more than a quarter (a restart).
        let seq = (n.seq > 0).then(|| (n.seq - 1) as usize);
        let frame = n.frame as f32 / 100.0;
        let looping = n.flags & FLAG_LOOPING != 0;
        if self.sequence != seq {
            self.sequence = seq;
            self.frame = frame;
            self.looping = looping;
        } else if let Some(s) = seq {
            let len = anim_length(s).max(1.0);
            let mut d = (self.frame - frame).abs();
            if looping {
                d = d.min(len - d);
            }
            if d > 0.25 * len {
                self.frame = frame;
            }
            self.looping = looping;
        }
        // Upper body: start the host's layer when it starts one (the
        // client's own flinches play meanwhile).
        let host_overlay = n.overlay.map(|o| o.0);
        if host_overlay != self.net.host_overlay {
            self.net.host_overlay = host_overlay;
            if let Some((s, f, r)) = n.overlay
                && self.overlay.map(|o| o.0) != Some(s as usize)
            {
                self.overlay = Some((s as usize, f as f32 / 100.0, r as usize));
            }
        }
        let cloaked = n.flags & FLAG_CLOAKED != 0;
        if cloaked != self.cloaked {
            self.cloaked = cloaked;
            self.cloak_dirty = true;
            changed.push(format!("cloaked={cloaked}"));
        }
        let raging = n.flags & FLAG_RAGING != 0;
        if raging != self.fp_rage.is_some() {
            self.fp_rage = raging.then_some(1.0);
            self.cloak_dirty = true;
            changed.push(format!("raging={raging}"));
        }
        let burning = n.flags & FLAG_BURNING != 0;
        self.burn_down = if burning { self.burn_down.max(1) } else { 0 };
        changed
    }
}

/// Client: what net/zeds.rs worked out for each host zed this frame.
#[derive(Resource, Default)]
pub struct PuppetFeed {
    /// Zed id -> the host's state at the moment drawn.
    pub samples: std::collections::HashMap<u32, PuppetSample>,
    /// The ids in the newest snapshot (None: none received yet). A living
    /// puppet missing from it is gone on the host.
    pub latest: Option<std::collections::HashSet<u32>>,
    /// The host's clock at the moment drawn (seconds), for the log.
    pub at: f64,
}

/// One host zed at the moment drawn: the discrete state (animation,
/// health, flags) from the snapshot before it, and the position, yaw and
/// velocity blended between the two snapshots around it.
#[derive(Clone, Debug)]
pub struct PuppetSample {
    pub n: ZedNet,
    /// Cylinder centre (Bevy), yaw (Unreal units), velocity (Bevy m/s).
    pub centre: Vec3,
    pub yaw: f32,
    pub velocity: Vec3,
}

/// Client: makes a puppet for each new host zed, copies the host's state
/// into the puppets, and removes the ones the host no longer has.
pub(super) fn drive_puppets(
    time: Res<Time<Real>>,
    feed: Res<PuppetFeed>,
    classes: Option<Res<ZedClasses>>,
    mut zeds: Query<&mut Zed>,
    mut spawn: MessageWriter<SpawnPuppet>,
    mut requested: Local<std::collections::HashMap<u32, f64>>,
    mut log_at: Local<f64>,
) {
    let Some(classes) = classes else { return };
    if feed.samples.is_empty() && feed.latest.is_none() {
        return;
    }
    let now = time.elapsed_secs_f64();
    let log_now = now - *log_at >= 1.0;
    if log_now {
        *log_at = now;
    }
    let mut have = std::collections::HashSet::new();
    let mut shown = Vec::new();
    for mut z in &mut zeds {
        if !z.net.puppet {
            continue;
        }
        let id = z.id as u32;
        have.insert(id);
        if let Some(s) = feed.samples.get(&id) {
            let c = &classes.0[z.class];
            let changed = z.apply_net(&s.n, s.centre, s.yaw, s.velocity, |seq| c.model.length(seq));
            if !changed.is_empty() {
                runlog::kv("net_zed_puppet_change", &format!("id={id} {}", changed.join(" ")));
            }
        } else if !z.is_dead() && feed.latest.as_ref().is_some_and(|l| !l.contains(&id)) {
            // Gone on the host (its death was missed, or it was removed).
            z.kill();
            runlog::kv("net_zed_vanished", &format!("id={id}"));
        }
        if log_now && !z.is_dead() {
            let u = ue_pos(z.centre);
            shown.push(format!("{id}:{:.0},{:.0},{:.0}", u.x, u.y, u.z));
        }
    }
    for (id, s) in &feed.samples {
        if have.contains(id) || s.n.dead() {
            continue;
        }
        // Asked for and not there yet (spawning waits for the first frames): ask again after 0.5 s.
        if requested.get(id).is_some_and(|t| now - t < 0.5) {
            continue;
        }
        requested.insert(*id, now);
        spawn.write(SpawnPuppet { id: *id as usize, kind: s.n.kind, centre: s.centre, yaw: s.yaw });
        runlog::kv("net_zed_puppet_spawn", &format!("id={id} kind={} centre_unreal=({:.0}, {:.0}, {:.0})", s.n.kind, s.n.pos[0] as f32 / 2.0, s.n.pos[1] as f32 / 2.0, s.n.pos[2] as f32 / 2.0));
    }
    requested.retain(|id, _| !have.contains(id));
    if log_now {
        let wall = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64());
        runlog::kv("net_zed_shown", &format!("wall={wall:.3} at={:.3} puppets={} zeds=[{}]", feed.at, shown.len(), shown.join(";")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn puppet_copies_the_host_zed() {
        let mut host = Zed::test_clot();
        host.centre = coords::pos([1234.0, -567.0, 89.5]);
        host.yaw = 1000.0;
        host.health = 80.0;
        host.sequence = Some(3);
        host.frame = 5.0;
        host.state = ZedState::Chase;
        let n = host.net_state();
        let mut p = Zed::test_clot();
        p.net.puppet = true;
        p.apply_net(&n, n.centre(), n.yaw as f32, Vec3::ZERO, |_| 30.0);
        assert_eq!((p.sequence, p.state), (Some(3), ZedState::Chase));
        assert!((p.frame - 5.0).abs() < 0.01 && (p.health - 80.0).abs() < 0.1);
        // Half-unit steps.
        assert!(ue_pos(p.centre).distance(Vec3::new(1234.0, -567.0, 89.5)) < 0.3);
        // A puppet's own lower health stays (its own hits come first).
        p.health = 50.0;
        p.apply_net(&n, n.centre(), 0.0, Vec3::ZERO, |_| 30.0);
        assert!((p.health - 50.0).abs() < 1e-3);
        // The head comes off and the zed dies on the host: the same here.
        host.remove_head();
        host.kill();
        let changed = p.apply_net(&host.net_state(), n.centre(), 0.0, Vec3::ZERO, |_| 30.0);
        assert!(p.decapitated && p.is_dead(), "{changed:?}");
        assert_eq!(changed, vec!["headless".to_string(), "dead".to_string()]);
    }
}
