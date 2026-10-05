//! Doors: KFDoorMover actors moved by Unreal's mover rules, opened through
//! KFUseTriggers by the player's USE key (E) and by zeds walking in.
//!
//! The rules come from Engine.Mover, KFMod.KFDoorMover and KFMod.KFUseTrigger
//! (see DESIGN.md, "Doors"). Positions are kept in Unreal units and turned
//! into Bevy transforms when drawn.

use avian3d::prelude::*;
use bevy::prelude::*;
use ue_assets::level::{DoorInfo, UseTriggerInfo};
use ue_assets::properties::Rotator;

use crate::collision::{GameLayer, TriSoup};
use crate::coords::{self, SCALE};
use crate::runlog;

/// Filled by the map loader; turned into `Doors` once colliders can spawn.
#[derive(Resource, Default)]
pub struct DoorSetup {
    pub doors: Vec<DoorSpawn>,
    pub triggers: Vec<UseTriggerInfo>,
}

pub struct DoorSpawn {
    pub info: DoorInfo,
    /// The drawn door (mesh parts are its children).
    pub root: Entity,
    /// Collision triangles in the door's local space, scale applied
    /// (empty if the door does not block).
    pub collision: TriSoup,
}

/// Marks a door's collider; the index is into `Doors::doors` (read by
/// the Welder from D2 on).
#[allow(dead_code)]
#[derive(Component)]
pub struct DoorCollider(pub usize);

/// What a door is doing, standing in for the latent code of Mover's
/// TriggerToggle state (labels Open / Close, OpenToKey / CloseToFirst).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Idle,
    /// Open: `Sleep(DelayTime)` (time left), then open to a key (None =
    /// DoOpen, Some = DoOpenToKey), then wait for the move to finish.
    Opening { delay: f32, to_key: Option<u8>, started: bool },
    /// Close / CloseToFirst: moving back, then FinishedClosing.
    Closing,
}

pub struct Door {
    pub info: DoorInfo,
    root: Entity,
    collider: Option<Entity>,
    /// Index into `Doors::triggers` (KFDoorMover.MyTrigger).
    pub trigger: Option<usize>,
    /// The mover's InitialState; only TriggerToggle is simulated.
    toggles: bool,
    // Mover state, Unreal units / rotation units.
    pub key_num: u8,
    pub prev_key_num: u8,
    pos: [f32; 3],
    rot: [f32; 3],
    old_pos: [f32; 3],
    old_rot: [f32; 3],
    phys_alpha: f32,
    phys_rate: f32,
    interpolating: bool,
    /// Mover.bClosed: finished closing (true at start, even for doors
    /// saved open, as in KF).
    pub closed: bool,
    phase: Phase,
    // KFDoorMover state used from D2 on.
    pub sealed: bool,
    pub hidden: bool,
}

pub struct Trigger {
    pub info: UseTriggerInfo,
    pub doors: Vec<usize>,
    /// KFUseTrigger.LastAttempt: an int, so the time is rounded down.
    last_attempt: i32,
    last_message: f32,
    /// vector(Rotation), for bDirectionalOpen.
    init_dir: [f32; 3],
}

#[derive(Resource, Default)]
pub struct Doors {
    pub doors: Vec<Door>,
    pub triggers: Vec<Trigger>,
    /// Which triggers each pawn touched last frame (pawn key: 0 = player,
    /// zed id + 1), to fire Touch only on entering.
    touching: std::collections::HashMap<usize, Vec<usize>>,
}

pub struct DoorPlugin;

impl Plugin for DoorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DoorSetup>()
            .init_resource::<Doors>()
            .add_systems(PostStartup, spawn_doors)
            .add_systems(Update, (use_and_touch, move_doors).chain());
    }
}

/// `BasePos + KeyPos[k]` and `BaseRot + KeyRot[k]` (rotation as floats).
pub fn key_pose(info: &DoorInfo, k: usize) -> ([f32; 3], [f32; 3]) {
    let p = info.key_pos.get(k).copied().unwrap_or_default();
    let r = info.key_rot.get(k).copied().unwrap_or_default();
    let b = info.base_pos;
    let br = info.base_rot;
    (
        [b[0] + p[0], b[1] + p[1], b[2] + p[2]],
        [(br.pitch + r.pitch) as f32, (br.yaw + r.yaw) as f32, (br.roll + r.roll) as f32],
    )
}

/// An Unreal rotation given as float (pitch, yaw, roll) as a Bevy rotation.
pub fn rotation_of(r: [f32; 3]) -> Quat {
    coords::rotation(Rotator {
        pitch: r[0].round() as i32,
        yaw: r[1].round() as i32,
        roll: r[2].round() as i32,
    })
}

/// Unreal's vector(Rotator): the rotated X axis.
fn rot_dir(r: Rotator) -> [f32; 3] {
    let k = std::f32::consts::TAU / 65536.0;
    let (sp, cp) = (r.pitch as f32 * k).sin_cos();
    let (sy, cy) = (r.yaw as f32 * k).sin_cos();
    [cp * cy, cp * sy, sp]
}

fn ue(p: Vec3) -> [f32; 3] {
    [-p.z / SCALE, p.x / SCALE, p.y / SCALE]
}

/// MV_GlideByTime's curve, smooth at both ends. Native code: taken from
/// the Unreal 1 public source as I remember it, not verified against KF.
fn glide(a: f32) -> f32 {
    3.0 * a * a - 2.0 * a * a * a
}

impl Door {
    fn new(info: DoorInfo, root: Entity, collider: Option<Entity>) -> Self {
        let k = (info.key_num as usize).min(23);
        let (pos, rot) = key_pose(&info, k);
        Door {
            toggles: info.initial_state.eq_ignore_ascii_case("TriggerToggle"),
            key_num: k as u8,
            prev_key_num: k as u8,
            pos,
            rot,
            old_pos: pos,
            old_rot: rot,
            phys_alpha: 0.0,
            phys_rate: 1.0,
            interpolating: false,
            closed: true,
            phase: Phase::Idle,
            sealed: false,
            hidden: false,
            info,
            root,
            collider,
            trigger: None,
        }
    }

    /// Mover.InterpolateTo (native). Going back to the key we came from
    /// reverses smoothly: the progress is mirrored (Unreal 1 source, as I
    /// remember it).
    fn interpolate_to(&mut self, key: u8, seconds: f32) {
        let key = key.min(23);
        if key == self.prev_key_num && self.key_num != self.prev_key_num {
            self.phys_alpha = 1.0 - self.phys_alpha;
            let (p, r) = key_pose(&self.info, self.key_num as usize);
            self.old_pos = p;
            self.old_rot = r;
        } else {
            self.old_pos = self.pos;
            self.old_rot = self.rot;
            self.phys_alpha = 0.0;
        }
        self.interpolating = true;
        self.prev_key_num = self.key_num;
        self.key_num = key;
        self.phys_rate = 1.0 / seconds.max(0.005);
    }

    /// Mover.KeyFrameReached: chain on through the keys, else stop.
    fn key_frame_reached(&mut self) {
        let old = self.prev_key_num;
        self.prev_key_num = self.key_num;
        self.phys_alpha = 0.0;
        if self.key_num > 0 && self.key_num < old {
            self.interpolate_to(self.key_num - 1, self.info.move_time);
        } else if self.key_num + 1 < self.info.num_keys && self.key_num > old {
            self.interpolate_to(self.key_num + 1, self.info.move_time);
        }
    }

    /// PHYS_MovingBrush for one frame.
    fn physics(&mut self, dt: f32) {
        if !self.interpolating {
            return;
        }
        let alpha = (self.phys_alpha + dt * self.phys_rate).min(1.0);
        let a = if self.info.glide_type == 1 { glide(alpha) } else { alpha };
        let (to_pos, to_rot) = key_pose(&self.info, self.key_num as usize);
        for i in 0..3 {
            self.pos[i] = self.old_pos[i] + (to_pos[i] - self.old_pos[i]) * a;
            self.rot[i] = self.old_rot[i] + (to_rot[i] - self.old_rot[i]) * a;
        }
        self.phys_alpha = alpha;
        if alpha >= 1.0 {
            self.interpolating = false;
            self.key_frame_reached();
        }
    }

    /// Trigger in state TriggerToggle: open if closed or still opening,
    /// else close.
    fn trigger(&mut self, by: &str) {
        if !self.toggles {
            runlog::kv("door_not_simulated", &format!("door={} state={} by={by}", self.info.name, self.info.initial_state));
            return;
        }
        if self.key_num == 0 || self.key_num < self.prev_key_num {
            self.goto_open(None, by);
        } else {
            self.goto_close(false, by);
        }
    }

    /// KFDoorMover TriggerToggle.OpenDoorToKey (bDirectionalOpen triggers).
    fn open_door_to_key(&mut self, key: u8, by: &str) {
        if !self.toggles {
            self.trigger(by);
            return;
        }
        if self.key_num == 0 {
            self.goto_open(Some(key), by);
        } else {
            self.goto_close(true, by);
        }
    }

    fn goto_open(&mut self, to_key: Option<u8>, by: &str) {
        self.closed = false;
        self.phase = Phase::Opening {
            delay: self.info.delay_time,
            to_key,
            started: false,
        };
        self.log("open", by);
    }

    /// Close (DoClose: one key back, KeyFrameReached chains down to 0) or
    /// CloseToFirst (DoCloseToFirst: straight to key 0, directional
    /// doors). KFDoorMover skips the move while sealed or hidden.
    fn goto_close(&mut self, to_first: bool, by: &str) {
        if !(self.sealed || self.hidden) {
            let back = if to_first { 0 } else { self.key_num.saturating_sub(1) };
            self.interpolate_to(back, self.info.move_time);
        }
        self.phase = Phase::Closing;
        self.log("close", by);
    }

    fn log(&self, event: &str, by: &str) {
        runlog::kv(
            "door",
            &format!(
                "door={} event={event} by={by} key={} prev_key={} pos=({:.0}, {:.0}, {:.0}) yaw={:.0} closed={}",
                self.info.name, self.key_num, self.prev_key_num, self.pos[0], self.pos[1], self.pos[2], self.rot[1], self.closed
            ),
        );
    }

    /// Runs the latent state code for one frame.
    fn step(&mut self, dt: f32) {
        match self.phase {
            Phase::Idle => {}
            Phase::Opening { delay, to_key, started } => {
                if !started {
                    if delay > 0.0 {
                        self.phase = Phase::Opening { delay: delay - dt, to_key, started };
                        return;
                    }
                    if !(self.sealed || self.hidden) {
                        self.interpolate_to(to_key.unwrap_or(1), self.info.move_time);
                    }
                    self.phase = Phase::Opening { delay: 0.0, to_key, started: true };
                } else if !self.interpolating {
                    self.phase = Phase::Idle;
                    self.log("opened", "move");
                }
            }
            Phase::Closing => {
                if !self.interpolating {
                    self.closed = true;
                    self.phase = Phase::Idle;
                    self.log("closed", "move");
                }
            }
        }
    }
}

fn spawn_doors(mut commands: Commands, mut setup: ResMut<DoorSetup>, mut doors: ResMut<Doors>) {
    let spawns = std::mem::take(&mut setup.doors);
    let mut colliders = 0usize;
    for (i, s) in spawns.into_iter().enumerate() {
        let collider = (!s.collision.triangles.is_empty()).then(|| {
            colliders += 1;
            let mut layers = LayerMask::from(GameLayer::Door);
            if s.info.blocks_traces {
                layers |= GameLayer::DoorTraces;
            }
            let (pos, rot) = key_pose(&s.info, s.info.key_num as usize);
            commands
                .spawn((
                    RigidBody::Kinematic,
                    Collider::trimesh(s.collision.vertices, s.collision.triangles),
                    CollisionLayers::new(layers, LayerMask::ALL),
                    Transform::from_translation(coords::pos(pos)).with_rotation(rotation_of(rot)),
                    DoorCollider(i),
                    Name::new(s.info.name.clone()),
                ))
                .id()
        });
        doors.doors.push(Door::new(s.info, s.root, collider));
    }
    // KFDoorMover.PostBeginPlay: the KFUseTrigger whose Event is our Tag.
    for info in std::mem::take(&mut setup.triggers) {
        let t = doors.triggers.len();
        let members: Vec<usize> = doors
            .doors
            .iter()
            .enumerate()
            .filter(|(_, d)| !d.info.tag.is_empty() && d.info.tag.eq_ignore_ascii_case(&info.event))
            .map(|(i, _)| i)
            .collect();
        for &i in &members {
            if doors.doors[i].trigger.is_some() {
                runlog::kv("door_warning", &format!("door={} reason=multiple_triggers", doors.doors[i].info.name));
            }
            doors.doors[i].trigger = Some(t);
        }
        doors.triggers.push(Trigger {
            init_dir: rot_dir(info.rotation),
            info,
            doors: members,
            last_attempt: i32::MIN / 2,
            last_message: f32::MIN,
        });
    }
    let mut states: std::collections::BTreeMap<String, usize> = Default::default();
    for d in &doors.doors {
        *states.entry(d.info.initial_state.clone()).or_default() += 1;
    }
    let no_trigger: Vec<&str> = doors.doors.iter().filter(|d| d.trigger.is_none()).map(|d| d.info.name.as_str()).collect();
    let not_toggle: Vec<String> = doors
        .doors
        .iter()
        .filter(|d| !d.toggles)
        .map(|d| format!("{}:{}", d.info.name, d.info.tag))
        .collect();
    let empty_triggers = doors.triggers.iter().filter(|t| t.doors.is_empty()).count();
    runlog::kv(
        "doors_loaded",
        &format!(
            "doors={} colliders={colliders} triggers={} triggers_without_doors={empty_triggers} states={states:?} \
             without_trigger={} not_simulated=[{}] without_trigger_names=[{}]",
            doors.doors.len(),
            doors.triggers.len(),
            no_trigger.len(),
            not_toggle.join(" "),
            no_trigger.join(" ")
        ),
    );
}

/// Do two upright cylinders overlap (Unreal units)? Unreal's Touch test.
fn cylinders_touch(a: [f32; 3], ar: f32, ah: f32, b: [f32; 3], br: f32, bh: f32) -> bool {
    let (dx, dy) = (a[0] - b[0], a[1] - b[1]);
    (a[2] - b[2]).abs() < ah + bh && dx * dx + dy * dy < (ar + br) * (ar + br)
}

/// Touched triggers this frame for a pawn at `p` (Unreal units).
fn touched(doors: &Doors, p: [f32; 3], radius: f32, half_height: f32) -> Vec<usize> {
    (0..doors.triggers.len())
        .filter(|&t| {
            let i = &doors.triggers[t].info;
            cylinders_touch(p, radius, half_height, i.location, i.radius, i.height)
        })
        .collect()
}

/// KFUseTrigger bDirectionalOpen: key 1 on the trigger's facing side,
/// key 2 behind it; 0 when not directional.
fn open_key(t: &Trigger, user: [f32; 3]) -> u8 {
    if !t.info.directional_open {
        return 0;
    }
    let d = [user[0] - t.info.location[0], user[1] - t.info.location[1], user[2] - t.info.location[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
    let dot = (d[0] * t.init_dir[0] + d[1] * t.init_dir[1] + d[2] * t.init_dir[2]) / len;
    if dot > 0.0 { 1 } else { 2 }
}

/// KFUseTrigger.UsedBy for the player.
fn used_by_player(doors: &mut Doors, t: usize, user: [f32; 3], now: f32) {
    let trig = &doors.triggers[t];
    if now - (trig.last_attempt as f32) < trig.info.refire_delay as f32 {
        runlog::kv("door_use_refused", &format!("trigger={} reason=refire_delay", trig.info.name));
        return;
    }
    let key = open_key(trig, user);
    let members = trig.doors.clone();
    let mut attempted = false;
    for i in members {
        let d = &mut doors.doors[i];
        if !d.sealed && !d.hidden && !d.info.key_locked {
            if key == 0 {
                d.trigger("player");
            } else {
                d.open_door_to_key(key, "player");
            }
            attempted = true;
        }
        if d.sealed && !d.hidden && d.closed {
            runlog::kv("message", &format!("text=\"This door is welded shut.|Use the Welder's alt-fire to unweld.\" door={}", d.info.name));
            attempted = true;
        }
        if d.info.key_locked && !d.sealed && !d.hidden && d.closed {
            runlog::kv("door_use_refused", &format!("door={} reason=key_locked", d.info.name));
        }
    }
    if attempted {
        doors.triggers[t].last_attempt = now.floor() as i32;
    }
}

/// KFUseTrigger.Touch: a zed walking in opens the doors still at key 0;
/// the player gets the trigger's message.
fn touch(doors: &mut Doors, t: usize, pawn: [f32; 3], zed: Option<usize>, now: f32) {
    let key = open_key(&doors.triggers[t], pawn);
    let members = doors.triggers[t].doors.clone();
    for i in members {
        let d = &mut doors.doors[i];
        match zed {
            Some(id) => {
                if !d.info.key_locked && !d.sealed && !d.hidden && d.key_num == 0 {
                    let by = format!("zed{id}");
                    if key == 0 {
                        d.trigger(&by);
                    } else {
                        d.open_door_to_key(key, &by);
                    }
                }
            }
            None => {
                let trig = &mut doors.triggers[t];
                if trig.last_message >= now || trig.info.message.is_empty() {
                    continue;
                }
                let text = if !d.sealed && !d.hidden {
                    // Messages containing "USE" show WaitingMessage's door hint.
                    if trig.info.message.contains("USE") {
                        "Press '%Use%' to open/close the door.|Use the Welder to seal closed doors."
                    } else {
                        trig.info.message.as_str()
                    }
                } else if !d.hidden && trig.info.always_show_message {
                    trig.info.message.as_str()
                } else {
                    continue;
                };
                trig.last_message = now + 0.6;
                runlog::kv("message", &format!("text=\"{text}\" trigger={}", trig.info.name));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn use_and_touch(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<crate::walk::MoveMode>,
    script: Res<crate::weapon::ScriptedInput>,
    frames: Res<bevy::diagnostic::FrameCount>,
    player: Query<&crate::walk::Walker>,
    zeds: Query<&crate::zed::Zed>,
    mut doors: ResMut<Doors>,
) {
    if doors.triggers.is_empty() {
        return;
    }
    let now = time.elapsed_secs();
    let doors = &mut *doors;
    // Player: KFPawn cylinder 20 x 50 (walk.rs).
    let player_at = (*mode == crate::walk::MoveMode::Walk).then(|| player.iter().next().map(|w| ue(w.center))).flatten();
    let mut pawns: Vec<(usize, [f32; 3], f32, f32, Option<usize>)> = Vec::new();
    if let Some(p) = player_at {
        pawns.push((0, p, 20.0, 50.0, None));
    }
    for z in &zeds {
        if let Some(c) = z.blocking_cylinder() {
            pawns.push((z.id + 1, ue(c.centre), c.radius / SCALE, c.half_height / SCALE, Some(z.id)));
        }
    }
    for (key, at, r, h, zed) in pawns {
        let now_touching = touched(doors, at, r, h);
        let before = doors.touching.remove(&key).unwrap_or_default();
        for &t in now_touching.iter().filter(|t| !before.contains(t)) {
            touch(doors, t, at, zed, now);
        }
        doors.touching.insert(key, now_touching);
    }
    let pressed = keys.just_pressed(KeyCode::KeyE) || script.0.iter().any(|(f, a)| *f == frames.0 && a == "use");
    if pressed && let Some(p) = player_at {
        // PlayerController.ServerUse: UsedBy on every touching actor.
        let list = doors.touching.get(&0).cloned().unwrap_or_default();
        runlog::kv(
            "use_pressed",
            &format!(
                "at=({:.0}, {:.0}, {:.0}) triggers=[{}]",
                p[0],
                p[1],
                p[2],
                list.iter().map(|&t| doors.triggers[t].info.name.as_str()).collect::<Vec<_>>().join(" ")
            ),
        );
        for t in list {
            used_by_player(doors, t, p, now);
        }
    }
}

fn move_doors(time: Res<Time>, mut doors: ResMut<Doors>, mut transforms: Query<&mut Transform>) {
    let dt = time.delta_secs();
    for d in doors.doors.iter_mut() {
        if d.phase == Phase::Idle && !d.interpolating {
            continue;
        }
        d.step(dt);
        d.physics(dt);
        d.step(0.0);
        let translation = coords::pos(d.pos);
        let rotation = rotation_of(d.rot);
        if let Ok(mut t) = transforms.get_mut(d.root) {
            t.translation = translation;
            t.rotation = rotation;
        }
        if let Some(c) = d.collider
            && let Ok(mut t) = transforms.get_mut(c)
        {
            t.translation = translation;
            t.rotation = rotation;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_door(num_keys: u8) -> Door {
        let mut key_rot = vec![Rotator::default(); 24];
        key_rot[1].yaw = 16384;
        key_rot[2].yaw = -16384;
        let info = DoorInfo {
            name: "TestDoor".into(),
            tag: "T".into(),
            base_pos: [0.0; 3],
            base_rot: Rotator::default(),
            key_pos: vec![[0.0; 3]; 24],
            key_rot,
            key_num: 0,
            num_keys,
            move_time: 1.0,
            delay_time: 0.0,
            glide_type: 1,
            initial_state: "TriggerToggle".into(),
            start_sealed: false,
            start_sealed_weld_prc: 0.0,
            disallow_weld: false,
            key_locked: false,
            no_seal: false,
            small_arms_damage: false,
            zombies_ignore: false,
            block_damaging_of_weld: false,
            surface_type: 0,
            is_leader: false,
            return_group: String::new(),
            blocks_traces: true,
        };
        Door::new(info, Entity::PLACEHOLDER, None)
    }

    fn run(d: &mut Door, seconds: f32) {
        let dt = 1.0 / 100.0;
        for _ in 0..(seconds / dt).round() as usize {
            d.step(dt);
            d.physics(dt);
            d.step(0.0);
        }
    }

    #[test]
    fn toggle_opens_glides_and_closes() {
        let mut d = test_door(2);
        d.trigger("test");
        assert!(!d.closed);
        run(&mut d, 0.5);
        // Halfway in time is halfway on the glide curve.
        assert!((d.rot[1] - 8192.0).abs() < 200.0, "yaw {}", d.rot[1]);
        run(&mut d, 0.6);
        assert_eq!(d.rot[1], 16384.0);
        assert_eq!(d.phase, Phase::Idle);
        d.trigger("test");
        run(&mut d, 1.1);
        assert_eq!(d.rot[1], 0.0);
        assert!(d.closed);
    }

    #[test]
    fn retrigger_while_opening_reverses_smoothly() {
        let mut d = test_door(2);
        d.trigger("test");
        run(&mut d, 0.3);
        let yaw = d.rot[1];
        d.trigger("test"); // still opening: KeyNum 1 > PrevKeyNum 0, so close
        assert_eq!(d.phase, Phase::Closing);
        run(&mut d, 0.01);
        assert!((d.rot[1] - yaw).abs() < 400.0, "jumped from {yaw} to {}", d.rot[1]);
        run(&mut d, 1.0);
        assert_eq!(d.rot[1], 0.0);
        assert!(d.closed);
    }

    #[test]
    fn directional_open_to_key_two_and_back() {
        let mut d = test_door(2);
        d.open_door_to_key(2, "test");
        run(&mut d, 1.1);
        assert_eq!(d.key_num, 2);
        assert_eq!(d.rot[1], -16384.0);
        d.open_door_to_key(2, "test");
        run(&mut d, 1.1);
        assert_eq!(d.key_num, 0);
        assert_eq!(d.rot[1], 0.0);
    }
}
