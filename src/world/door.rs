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

use crate::world::collision::{GameLayer, TriSoup};
use crate::engine::coords::{self, SCALE};
use crate::engine::runlog;

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

/// Marks a door's collider; the index is into `Doors::doors`.
#[derive(Component)]
pub struct DoorCollider(pub usize);

/// Marks a trader door's collider. Not a
/// `DoorCollider`: zeds, the welder and damage treat it as a wall.
#[derive(Component)]
pub struct TraderDoorCollider;

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
    // KFDoorMover welding state (D2).
    /// bSealed: welded (WeldStrength > 0, or bStartSealed).
    pub sealed: bool,
    pub hidden: bool,
    /// WeldStrength, kept in step with the trigger's (SetWeldStrength).
    pub weld: f32,
    /// MaxWeld: the trigger's MaxWeldStrength (0 without a trigger).
    pub max_weld: f32,
    /// Health when not welded (MaxWeld at the start); damage to it is D3/D4.
    pub health: f32,
    /// bZedHittingDoor: welding counts less while set. Set by zed hits;
    /// cleared by KFDoorMover.Timer, but Mover only starts that timer in
    /// net games, so in single player it stays set (copied).
    pub zed_hitting: bool,
    /// DoorPathNode (a nav point index) and its starting ExtraCost.
    path_node: Option<usize>,
    /// Broken (GoBang): bDoorIsDead.
    pub dead: bool,
    /// bShouldBeOpen: what an open or close asked for while the door was
    /// sealed or hidden (and so did not move). RespawnDoor uses it.
    should_be_open: bool,
    /// Sound events of this frame, played by `door_sounds`.
    sounds: Vec<DoorSound>,
    /// Seconds since the last zed-hit sound (LastZombieHitSoundTime).
    since_zed_hit_sound: f32,
    /// MoveAmbientSound is on (while moving).
    ambient_on: bool,
}

/// What a door plays (Mover and KFDoorMover).
#[derive(Clone, Copy, Debug, PartialEq)]
enum DoorSound {
    /// OpeningSound, OpenedSound, ClosingSound, ClosedSound (index into
    /// DoorInfo.sounds).
    Mover(usize),
    /// KFDoorMover.PlayZombieHitSound.
    ZedHit,
    /// GoBang: MetalBreakSound / WoodBreakSound.
    Break,
}

pub struct Trigger {
    pub info: UseTriggerInfo,
    pub doors: Vec<usize>,
    /// KFUseTrigger.LastAttempt: an int, so the time is rounded down.
    last_attempt: i32,
    last_message: f32,
    /// vector(Rotation), for bDirectionalOpen.
    init_dir: [f32; 3],
    /// KFUseTrigger.WeldStrength, shared by all its doors.
    pub weld_strength: f32,
}

#[derive(Resource, Default)]
pub struct Doors {
    pub doors: Vec<Door>,
    /// KFTraderDoors: moved only by their shop (trader.rs).
    pub trader: Vec<Door>,
    pub triggers: Vec<Trigger>,
    /// Which triggers each pawn touched last frame (pawn key: 0 = player,
    /// zed id + 1), to fire Touch only on entering.
    touching: std::collections::HashMap<usize, Vec<usize>>,
    /// Messages for the player this frame (sent on by `use_and_touch`).
    outbox: Vec<crate::game::hud::LocalMessage>,
}

pub struct DoorPlugin;

impl Plugin for DoorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DoorSetup>()
            .init_resource::<Doors>()
            .init_resource::<WeldView>()
            .add_message::<WeldHit>()
            .add_message::<ZedDoorHit>()
            .add_message::<DoorBlast>()
            .add_message::<RespawnDoors>()
            .add_systems(PostStartup, spawn_doors)
            .add_systems(Update, (use_and_touch, aim_at_door, weld_hits, zed_door_hits, respawn_doors, move_doors, door_path_costs, door_sounds).chain());
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
    /// The mover's Location now (Unreal units).
    pub fn location(&self) -> [f32; 3] {
        self.pos
    }

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
            sounds: Vec::new(),
            since_zed_hit_sound: f32::MAX,
            ambient_on: false,
            hidden: false,
            weld: 0.0,
            max_weld: 0.0,
            health: 0.0,
            zed_hitting: false,
            path_node: None,
            dead: false,
            should_be_open: false,
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

    /// Trigger from a shop (ShopVolume.OpenShop / CloseShop).
    pub fn shop_trigger(&mut self, shop: &str) {
        self.trigger(shop);
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
            // DoClose / DoCloseToFirst: ClosingSound.
            self.sounds.push(DoorSound::Mover(2));
        } else {
            self.should_be_open = false;
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
                        // DoOpen / DoOpenToKey: OpeningSound, AmbientSound = MoveAmbientSound.
                        self.sounds.push(DoorSound::Mover(0));
                    } else {
                        self.should_be_open = true;
                    }
                    self.phase = Phase::Opening { delay: 0.0, to_key, started: true };
                } else if !self.interpolating {
                    self.phase = Phase::Idle;
                    self.log("opened", "move");
                    // FinishedOpening: OpenedSound (KFDoorMover: not while sealed or hidden).
                    if !(self.sealed || self.hidden) {
                        self.sounds.push(DoorSound::Mover(1));
                    }
                }
            }
            Phase::Closing => {
                if !self.interpolating {
                    self.closed = true;
                    self.phase = Phase::Idle;
                    self.log("closed", "move");
                    // FinishedClosing: ClosedSound.
                    if !(self.sealed || self.hidden) {
                        self.sounds.push(DoorSound::Mover(3));
                    }
                }
            }
        }
    }
}

/// Plays the doors' sound events and keeps MoveAmbientSound on while a
/// door moves (Mover: DoOpen / DoClose set it, the end of the move clears
/// it). Mover sounds: PlaySound(X, SLOT_None, SoundVolume / 255, false,
/// SoundRadius, SoundPitch / 64). KFDoorMover: ZombieHitSound (metal or
/// wood by SurfaceType) at 2.0, radius 200; the break sound at 2.0,
/// radius 5000.
fn door_sounds(time: Res<Time>, mut doors: ResMut<Doors>, mut commands: Commands, mut out: MessageWriter<crate::audio::mixer::PlaySound>) {
    use crate::audio::mixer::{Emitter, PlaySound};
    let dt = time.delta_secs();
    let doors = &mut *doors;
    for d in doors.doors.iter_mut().chain(doors.trader.iter_mut()) {
        d.since_zed_hit_sound = (d.since_zed_hit_sound + dt).min(1e6);
        let at = Emitter::Entity(d.root);
        let metal = d.info.surface_type == 3;
        for ev in std::mem::take(&mut d.sounds) {
            let play = match ev {
                DoorSound::Mover(k) => d.info.sounds[k].clone().map(|snd| {
                    PlaySound::new(snd, at).volume(d.info.sound_volume as f32 / 255.0).radius(d.info.sound_radius).pitch(d.info.sound_pitch as f32 / 64.0)
                }),
                DoorSound::ZedHit => {
                    let snd = if metal { "KF_EnemyGlobalSnd.Zomb_HitDoor_Metal" } else { "KF_EnemyGlobalSnd.Zomb_HitDoor_Wood" };
                    Some(PlaySound::new(snd, at).volume(2.0).radius(200.0))
                }
                DoorSound::Break => {
                    let snd = if metal { "KF_EnvAmbientSnd2.DoorBreak.Door_Break_Metal" } else { "KF_EnvAmbientSnd2.DoorBreak.Door_Break_Wood" };
                    Some(PlaySound::new(snd, at).volume(2.0).radius(5000.0))
                }
            };
            if let Some(p) = play {
                out.write(p);
            }
        }
        let want = d.interpolating && !(d.sealed || d.hidden) && d.info.sounds[4].is_some();
        if want != d.ambient_on {
            d.ambient_on = want;
            if want {
                commands.entity(d.root).insert(crate::audio::mixer::AmbientSound {
                    sound: d.info.sounds[4].clone().unwrap_or_default(),
                    volume: d.info.sound_volume,
                    radius: d.info.sound_radius,
                    pitch: d.info.sound_pitch,
                    ..default()
                });
            } else {
                commands.entity(d.root).remove::<crate::audio::mixer::AmbientSound>();
            }
        }
    }
}

/// A door's collision layers when it blocks (at the start, and when
/// RespawnDoor turns collision back on).
fn door_layers(info: &DoorInfo) -> CollisionLayers {
    let mut layers = LayerMask::from(GameLayer::Door);
    if info.blocks_traces {
        layers |= GameLayer::DoorTraces;
    }
    CollisionLayers::new(layers, LayerMask::ALL)
}

fn spawn_doors(mut commands: Commands, mut setup: ResMut<DoorSetup>, mut doors: ResMut<Doors>) {
    let spawns = std::mem::take(&mut setup.doors);
    let mut colliders = 0usize;
    for s in spawns.into_iter() {
        let trader = s.info.trader;
        let i = if trader { doors.trader.len() } else { doors.doors.len() };
        let collider = (!s.collision.triangles.is_empty()).then(|| {
            colliders += 1;
            let (pos, rot) = key_pose(&s.info, s.info.key_num as usize);
            let mut e = commands.spawn((
                RigidBody::Kinematic,
                Collider::trimesh(s.collision.vertices, s.collision.triangles),
                door_layers(&s.info),
                Transform::from_translation(coords::pos(pos)).with_rotation(rotation_of(rot)),
                Name::new(s.info.name.clone()),
            ));
            if trader {
                e.insert(TraderDoorCollider);
            } else {
                e.insert(DoorCollider(i));
            }
            e.id()
        });
        let door = Door::new(s.info, s.root, collider);
        if trader {
            doors.trader.push(door);
        } else {
            doors.doors.push(door);
        }
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
            weld_strength: 0.0,
        });
    }
    // KFDoorMover.PostBeginPlay: MaxWeld and Health from the trigger;
    // bStartSealed doors start welded to StartSealedWeldPrc percent.
    for i in 0..doors.doors.len() {
        if let Some(t) = doors.doors[i].trigger {
            let max = doors.triggers[t].info.max_weld_strength;
            doors.doors[i].max_weld = max;
            doors.doors[i].health = max;
        }
    }
    let mut start_sealed = Vec::new();
    for i in 0..doors.doors.len() {
        if !doors.doors[i].info.start_sealed {
            continue;
        }
        doors.doors[i].sealed = true;
        if let Some(t) = doors.doors[i].trigger {
            doors.triggers[t].weld_strength = 0.0;
            let amount = doors.doors[i].max_weld * doors.doors[i].info.start_sealed_weld_prc / 100.0;
            doors.add_weld(t, amount, false);
        }
        start_sealed.push(format!("{}:{:.0}", doors.doors[i].info.name, doors.doors[i].weld));
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
            "doors={} trader_doors={} colliders={colliders} triggers={} triggers_without_doors={empty_triggers} states={states:?} \
             without_trigger={} start_sealed=[{}] not_simulated=[{}] without_trigger_names=[{}]",
            doors.doors.len(),
            doors.trader.len(),
            doors.triggers.len(),
            no_trigger.len(),
            start_sealed.join(" "),
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
    let mut welded = false;
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
            // KFUseTrigger.UsedBy: WaitingMessage 4.
            welded = true;
            attempted = true;
        }
        if d.info.key_locked && !d.sealed && !d.hidden && d.closed {
            runlog::kv("door_use_refused", &format!("door={} reason=key_locked", d.info.name));
        }
    }
    if attempted {
        doors.triggers[t].last_attempt = now.floor() as i32;
    }
    // One per welded door in KF; the class is unique, so one shows.
    if welded {
        doors.outbox.push(crate::game::hud::LocalMessage::new(crate::game::hud::MessageClass::Waiting, 4));
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
                let (text, message) = if !d.sealed && !d.hidden {
                    // Messages containing "USE" show WaitingMessage's door
                    // hint (6); others go out as ClientMessage 'CriticalEvent'
                    // (KFCriticalEventPlus).
                    if trig.info.message.contains("USE") {
                        let m = crate::game::hud::LocalMessage::new(crate::game::hud::MessageClass::Waiting, 6);
                        ("Press '%Use%' to open/close the door.|Use the Welder to seal closed doors.".to_string(), m)
                    } else {
                        let m = crate::game::hud::LocalMessage { class: crate::game::hud::MessageClass::Critical, switch: 0, text: Some(trig.info.message.clone()) };
                        (trig.info.message.clone(), m)
                    }
                } else if !d.hidden && trig.info.always_show_message {
                    let m = crate::game::hud::LocalMessage { class: crate::game::hud::MessageClass::Critical, switch: 0, text: Some(trig.info.message.clone()) };
                    (trig.info.message.clone(), m)
                } else {
                    continue;
                };
                trig.last_message = now + 0.6;
                runlog::kv("message", &format!("text=\"{text}\" trigger={}", trig.info.name));
                doors.outbox.push(message);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn use_and_touch(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<crate::player::walk::MoveMode>,
    script: Res<crate::weapons::weapon::ScriptedInput>,
    frames: Res<bevy::diagnostic::FrameCount>,
    player: Query<&crate::player::walk::Walker>,
    zeds: Query<&crate::zeds::zed::Zed>,
    mut doors: ResMut<Doors>,
    mut hud_messages: MessageWriter<crate::game::hud::LocalMessage>,
) {
    for m in std::mem::take(&mut doors.outbox) {
        hud_messages.write(m);
    }
    if doors.triggers.is_empty() {
        return;
    }
    let now = time.elapsed_secs();
    let doors = &mut *doors;
    // Player: KFPawn cylinder 20 x 50 (walk.rs).
    let player_at = (*mode == crate::player::walk::MoveMode::Walk).then(|| player.iter().next().map(|w| ue(w.center))).flatten();
    // (touch key, position, radius, half-height, zed id), Unreal units.
    type Pawn = (usize, [f32; 3], f32, f32, Option<usize>);
    let mut pawns: Vec<Pawn> = Vec::new();
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
    let doors = &mut *doors;
    for d in doors.doors.iter_mut().chain(doors.trader.iter_mut()) {
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

/// The door the Welder points at (WeldFire.GetDoor): what a trace from
/// the eye along the view hits first, if that is a door. The Welder
/// checks `distance` against its weaponRange.
#[derive(Resource, Default)]
pub struct WeldView {
    pub door: Option<WeldDoor>,
    /// WeldFire.LastHitActor of each Welder fire mode (0 weld, 1 unweld):
    /// the door the last weld or unweld trace hit, None if it hit no door.
    /// The Welder's screen reads it (welder_screen.rs).
    pub last_hit: [Option<usize>; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct WeldDoor {
    pub index: usize,
    /// Unreal units from the eye.
    pub distance: f32,
    pub weld: f32,
    pub max_weld: f32,
    pub disallow_weld: bool,
}

/// WeldFire / UnWeldFire.Timer: DamagedelayMin after the fire, trace
/// `range` from the eye and damage what is hit with DamTypeWelder or
/// DamTypeUnWeld. Positions in Bevy space.
#[derive(Message, Clone, Copy, Debug)]
pub struct WeldHit {
    pub origin: Vec3,
    pub dir: Vec3,
    /// Unreal units.
    pub range: f32,
    pub damage: f32,
    pub unweld: bool,
}

/// KFDoorMover DamageThreshold (class default; no map changes it).
const DAMAGE_THRESHOLD: i32 = 50;

/// How far the aim trace looks (Unreal units); the Welder's own range
/// (70) is checked by the weapon.
const AIM_TRACE: f32 = 200.0;

impl Doors {
    /// KFDoorMover.SetWeldStrength on every door of trigger `t`.
    fn set_weld(&mut self, t: usize) {
        let w = self.triggers[t].weld_strength;
        for &i in &self.triggers[t].doors {
            let d = &mut self.doors[i];
            d.weld = w;
            d.sealed = w > 0.0;
        }
    }

    /// KFUseTrigger.AddWeld: x CombatSealReduction while zeds hit the
    /// door, capped at MaxWeldStrength.
    fn add_weld(&mut self, t: usize, extra: f32, zombie_attacking: bool) -> f32 {
        let trig = &mut self.triggers[t];
        let mut extra = extra;
        if zombie_attacking {
            extra *= trig.info.combat_seal_reduction;
        }
        if trig.weld_strength + extra > trig.info.max_weld_strength {
            extra = trig.info.max_weld_strength - trig.weld_strength;
        }
        if extra == 0.0 {
            return 0.0;
        }
        trig.weld_strength += extra;
        self.set_weld(t);
        extra
    }

    /// KFUseTrigger.UnWeld: no floor at 0, so the strength can go below
    /// zero (copied; the clamp is commented out in the script).
    fn unweld(&mut self, t: usize, amount: f32, zombie_attacking: bool) -> f32 {
        let mut amount = amount;
        if zombie_attacking {
            amount *= self.triggers[t].info.combat_seal_reduction;
        }
        if amount == 0.0 {
            return 0.0;
        }
        self.triggers[t].weld_strength -= amount;
        self.set_weld(t);
        amount
    }

    /// KFDoorMover.TakeDamage from the player's Welder (DamTypeWelder or
    /// DamTypeUnWeld). Returns what happened, for the log.
    fn welder_damage(&mut self, i: usize, damage: f32, unweld: bool) -> String {
        let d = &self.doors[i];
        let Some(t) = d.trigger else {
            return "ignored reason=no_trigger".into();
        };
        if d.hidden {
            return "ignored reason=hidden".into();
        }
        let mut what = String::new();
        // Unsealed doors lose Health to anything but the welder (D3/D4 break them).
        if !d.sealed && unweld {
            self.doors[i].health -= damage * 0.5;
            what = format!("health={:.0} ", self.doors[i].health);
        }
        let d = &self.doors[i];
        if d.closed && !unweld && !d.info.disallow_weld {
            let zed = d.zed_hitting;
            self.doors[i].sealed = true;
            let added = self.add_weld(t, damage, zed);
            what += &format!("welded added={added:.1}");
        } else if d.sealed {
            let zed = d.zed_hitting;
            if unweld {
                let removed = self.unweld(t, damage, zed);
                what += &format!("unwelded removed={removed:.1}");
            } else if !d.info.block_damaging_of_weld {
                // DamageWeld (a sealed door that is not bClosed); breaking is D3.
                self.triggers[t].weld_strength -= damage;
                self.set_weld(t);
                what += "weld_damaged";
            }
        } else {
            what += if d.closed { "nothing reason=disallowed" } else { "nothing reason=not_closed" };
        }
        what
    }
}

fn ray_door(spatial: &SpatialQuery, colliders: &Query<&DoorCollider>, origin: Vec3, dir: Vec3, range: f32) -> Option<(usize, f32, Vec3, Vec3)> {
    let dir3 = Dir3::new(dir).ok()?;
    let hit = spatial.cast_ray(origin, dir3, range * SCALE, true, &crate::world::collision::world_filter())?;
    let c = colliders.get(hit.entity).ok()?;
    let n = if hit.normal.dot(dir) > 0.0 { -hit.normal } else { hit.normal };
    Some((c.0, hit.distance / SCALE, origin + dir * hit.distance, n))
}

fn aim_at_door(
    spatial: SpatialQuery,
    colliders: Query<&DoorCollider>,
    cam: Query<&Transform, With<crate::engine::camera::FlyCamera>>,
    doors: Res<Doors>,
    mut view: ResMut<WeldView>,
) {
    view.door = cam.single().ok().and_then(|t| {
        let (i, distance, _, _) = ray_door(&spatial, &colliders, t.translation, *t.forward(), AIM_TRACE)?;
        let d = &doors.doors[i];
        Some(WeldDoor {
            index: i,
            distance,
            weld: d.weld,
            max_weld: d.max_weld,
            disallow_weld: d.info.disallow_weld,
        })
    });
}

#[allow(clippy::too_many_arguments)]
fn weld_hits(
    mut hits: MessageReader<WeldHit>,
    spatial: SpatialQuery,
    colliders: Query<&DoorCollider>,
    mut doors: ResMut<Doors>,
    mut view: ResMut<WeldView>,
    mut commands: Commands,
    library: Option<Res<crate::render::particles::EffectLibrary>>,
    (mut meshes, mut sounds): (ResMut<Assets<Mesh>>, MessageWriter<crate::audio::mixer::PlaySound>),
    mut seed: Local<u32>,
) {
    for h in hits.read() {
        let mode = if h.unweld { "unweld" } else { "weld" };
        let hit = ray_door(&spatial, &colliders, h.origin, h.dir, h.range);
        // WeldFire.Timer: LastHitActor = HitActor. KF keeps any actor the
        // trace hits (a wall gives the LevelInfo); only doors are kept here.
        view.last_hit[usize::from(h.unweld)] = hit.map(|(i, ..)| i);
        let Some((i, distance, at, normal)) = hit else {
            runlog::kv("weld_hit", &format!("mode={mode} door=none"));
            continue;
        };
        let what = doors.welder_damage(i, h.damage, h.unweld);
        let d = &doors.doors[i];
        let pct = if d.max_weld > 0.0 { d.weld / d.max_weld * 100.0 } else { 0.0 };
        runlog::kv(
            "weld_hit",
            &format!(
                "mode={mode} door={} distance={distance:.0} damage={} {what} weld={:.1} max={:.0} percent={pct:.0} sealed={}",
                d.info.name, h.damage, d.weld, d.max_weld, d.sealed
            ),
        );
        // WelderHitEmitter (a KFHitEmitter): PlaySound(ImpactSounds[Rand]),
        // PatchSounds.WelderFire, its TransientSoundVolume 150 (capped) and
        // TransientSoundRadius 80.
        sounds.write(crate::audio::mixer::PlaySound::new("PatchSounds.WelderFire", crate::audio::mixer::Emitter::Point(at)).volume(150.0).radius(80.0));
        // KFWelderHitEffect: a WelderHitEmitter on the door, 0.15 x the
        // player's CollisionHeight (50) below the hit, 4 units out.
        if let Some(lib) = library.as_deref() {
            let n = Vec3::new(-normal.z, normal.x, normal.y).normalize_or_zero();
            let p = Vec3::from_array(ue(at)) - Vec3::Z * 0.15 * 50.0 + n * 4.0;
            *seed = seed.wrapping_add(1);
            crate::render::particles::spawn_effect(&mut commands, lib, &mut meshes, "KFMod.WelderHitEmitter", p, crate::zeds::fireball::axes_along(n), *seed);
        }
    }
}

/// A zed's door hit: KFMonster.MeleeDamageTarget with a KFDoorMover as
/// Controller.Target calls its TakeDamage with the claw damage.
#[derive(Message, Clone, Copy, Debug)]
pub struct ZedDoorHit {
    pub door: usize,
    /// UsedMeleeDamage (MeleeDamage -5% .. +5%).
    pub damage: f32,
    pub zed: usize,
    /// What hit, for the log (claw, vomit, scream).
    pub kind: &'static str,
}

impl Doors {
    /// KFUseTrigger.DamageWeld: weld off every door of trigger `t`; at 0
    /// they all break (returned).
    fn damage_weld(&mut self, t: usize, dmg: i32) -> Vec<usize> {
        self.triggers[t].weld_strength -= dmg as f32;
        if self.triggers[t].weld_strength <= 0.0 {
            self.triggers[t].weld_strength = 0.0;
            self.set_weld(t);
            return self.triggers[t].doors.clone();
        }
        self.set_weld(t);
        Vec::new()
    }

    /// KFDoorMover.TakeDamage from the player (not the welder): nothing
    /// unless the damage type is DamTypeFrag (or the door has
    /// bSmallArmsDamage) and the damage (an int) reaches DamageThreshold
    /// 50. Then an unsealed door loses half from Health, a sealed one the
    /// whole from the weld (unless bBlockDamagingOfWeld).
    fn player_damage(&mut self, i: usize, damage: f32, frag: bool) -> (Vec<usize>, String) {
        let d = &self.doors[i];
        let Some(t) = d.trigger else {
            return (Vec::new(), "ignored reason=no_trigger".into());
        };
        if d.hidden {
            return (Vec::new(), "ignored reason=hidden".into());
        }
        let dmg = damage as i32;
        if (!d.info.small_arms_damage && !frag) || dmg < DAMAGE_THRESHOLD {
            return (Vec::new(), format!("ignored damage={dmg} frag={frag}"));
        }
        if !d.sealed {
            let half = dmg / 2;
            let d = &mut self.doors[i];
            d.health -= half as f32;
            let text = format!("damage={dmg} health_damage={half} health={:.0}", d.health);
            return (if d.health <= 0.0 { vec![i] } else { Vec::new() }, text);
        }
        if d.info.block_damaging_of_weld {
            return (Vec::new(), format!("damage={dmg} blocked=bBlockDamagingOfWeld"));
        }
        let broken = self.damage_weld(t, dmg);
        (broken, format!("damage={dmg} weld={:.0}", self.triggers[t].weld_strength))
    }

    /// KFDoorMover.TakeDamage from a zed. Damage is an int parameter, so
    /// the claw damage is truncated; then Max(5, Damage x
    /// ZombieDamageReductionFactor 0.85), again whole. Returns the doors
    /// broken (GoBang) and a log text.
    fn zed_damage(&mut self, i: usize, damage: f32) -> (Vec<usize>, String) {
        let d = &self.doors[i];
        let Some(t) = d.trigger else {
            return (Vec::new(), "ignored reason=no_trigger".into());
        };
        if d.hidden {
            return (Vec::new(), "ignored reason=hidden".into());
        }
        let dmg = ((damage as i32) as f32 * 0.85).max(5.0).trunc() as i32;
        let d = &mut self.doors[i];
        d.zed_hitting = true;
        // KFDoorMover.TakeDamage: welded, at most every 0.5 s.
        if d.sealed && d.since_zed_hit_sound >= 0.5 {
            d.since_zed_hit_sound = 0.0;
            d.sounds.push(DoorSound::ZedHit);
        }
        if !d.sealed {
            // Unsealed: Damage *= 0.5 (int), from Health.
            let half = dmg / 2;
            d.health -= half as f32;
            let text = format!("damage={dmg} health_damage={half} health={:.0}", d.health);
            if d.health <= 0.0 {
                return (vec![i], text);
            }
            return (Vec::new(), text);
        }
        if d.info.block_damaging_of_weld {
            return (Vec::new(), format!("damage={dmg} blocked=bBlockDamagingOfWeld"));
        }
        let broken = self.damage_weld(t, dmg);
        (broken, format!("damage={dmg} weld={:.0}", self.triggers[t].weld_strength))
    }
}

/// Radius damage reaching doors (Actor.HurtRadius and its KF versions).
/// Each door within `radius` of `at`, measured to its Location (the
/// pivot; KFDoorMover CollisionRadius is 0), takes damage x (1 -
/// distance / radius). `direct`: the door a projectile hit (Projectile
/// .HitWall: full damage, and it is left out of the radius part).
/// `line_of_sight`: VisibleCollidingActors (the Siren) rather than
/// CollidingActors (LAWProj: the Husk's fireball, the Patriarch's rocket).
/// Unreal units.
#[derive(Message, Clone, Copy, Debug)]
pub struct DoorBlast {
    pub at: Vec3,
    pub radius: f32,
    pub damage: f32,
    /// The instigating zed (KFMonster damage rules); None = the player.
    pub zed: Option<usize>,
    pub direct: Option<usize>,
    pub line_of_sight: bool,
    /// The damage type is DamTypeFrag (the player's hand grenade; the
    /// only player damage doors take, KFDoorMover.TakeDamage).
    pub frag: bool,
    pub source: &'static str,
}

/// KFDoorMover.GoBang: no collision, hidden, the wood or metal break
/// emitter at the door's Location facing up.
#[allow(clippy::too_many_arguments)]
fn go_bang(
    doors: &mut Doors,
    i: usize,
    by: &str,
    commands: &mut Commands,
    library: Option<&crate::render::particles::EffectLibrary>,
    meshes: &mut Assets<Mesh>,
    visibility: &mut Query<&mut Visibility>,
    seed: &mut u32,
) {
    let d = &mut doors.doors[i];
    if d.hidden {
        return;
    }
    d.hidden = true;
    d.dead = true;
    d.sealed = false;
    if let Some(c) = d.collider {
        commands.entity(c).insert(CollisionLayers::NONE);
    }
    if let Ok(mut v) = visibility.get_mut(d.root) {
        *v = Visibility::Hidden;
    }
    // EST_Metal is 3 (Actor.ESurfaceTypes).
    let class = if d.info.surface_type == 3 { "KFMod.KFDoorExplodeMetal" } else { "KFMod.KFDoorExplodeWood" };
    if let Some(lib) = library {
        *seed = seed.wrapping_add(1);
        crate::render::particles::spawn_effect(commands, lib, meshes, class, Vec3::from_array(d.pos), crate::zeds::fireball::axes_along(Vec3::Z), *seed);
    }
    d.sounds.push(DoorSound::Break);
    d.log("broken", by);
    runlog::kv("door_broken", &format!("door={} effect={class}", d.info.name));
}

#[allow(clippy::too_many_arguments)]
fn zed_door_hits(
    mut hits: MessageReader<ZedDoorHit>,
    mut blasts: MessageReader<DoorBlast>,
    spatial: SpatialQuery,
    mut doors: ResMut<Doors>,
    mut commands: Commands,
    library: Option<Res<crate::render::particles::EffectLibrary>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut visibility: Query<&mut Visibility>,
    mut seed: Local<u32>,
    (script, frames): (Res<crate::weapons::weapon::ScriptedInput>, Res<bevy::diagnostic::FrameCount>),
) {
    // Test action "break_doors": every door with a trigger goes bang.
    if script.0.iter().any(|(f, a)| *f == frames.0 && a == "break_doors") {
        for i in 0..doors.doors.len() {
            if doors.doors[i].trigger.is_some() {
                go_bang(&mut doors, i, "test", &mut commands, library.as_deref(), &mut meshes, &mut visibility, &mut seed);
            }
        }
    }
    // (door, damage, zed, frag, what) for every door hurt this frame.
    let mut damage: Vec<(usize, f32, Option<usize>, bool, String)> = Vec::new();
    for h in hits.read() {
        damage.push((h.door, h.damage, Some(h.zed), false, h.kind.into()));
    }
    // FastTrace for VisibleCollidingActors: level geometry only.
    let level = SpatialQueryFilter::from_mask([GameLayer::World, GameLayer::TraceBlocking]);
    for b in blasts.read() {
        if let Some(i) = b.direct
            && i < doors.doors.len()
        {
            damage.push((i, b.damage, b.zed, b.frag, format!("{} direct", b.source)));
        }
        for (i, d) in doors.doors.iter().enumerate() {
            if Some(i) == b.direct || d.hidden || d.trigger.is_none() {
                continue;
            }
            let loc = Vec3::from_array(d.pos);
            let dist = (loc - b.at).length().max(1.0);
            if dist > b.radius {
                continue;
            }
            if b.line_of_sight {
                let (from, to) = (coords::pos(b.at.to_array()), coords::pos(d.pos));
                if let Ok(dir) = Dir3::new(to - from)
                    && spatial.cast_ray(from, dir, (to - from).length(), true, &level).is_some()
                {
                    runlog::kv("door_blast_blocked", &format!("door={} source={} distance={dist:.0}", d.info.name, b.source));
                    continue;
                }
            }
            let scale = 1.0 - (dist / b.radius).max(0.0);
            damage.push((i, scale * b.damage, b.zed, b.frag, format!("{} radius distance={dist:.0} scale={scale:.2}", b.source)));
        }
    }
    for (i, amount, zed, frag, what) in damage {
        let (broken, text, by) = match zed {
            Some(id) => {
                let (b, t) = doors.zed_damage(i, amount);
                (b, t, format!("zed{id}"))
            }
            None => {
                let (b, t) = doors.player_damage(i, amount, frag);
                (b, t, "player".to_string())
            }
        };
        let d = &doors.doors[i];
        let pct = if d.max_weld > 0.0 { d.weld / d.max_weld * 100.0 } else { 0.0 };
        runlog::kv(
            "door_damage",
            &format!("door={} by={by} source={what} amount={amount:.1} {text} percent={pct:.0} sealed={}", d.info.name, d.sealed),
        );
        for j in broken {
            go_bang(&mut doors, j, &by, &mut commands, library.as_deref(), &mut meshes, &mut visibility, &mut seed);
        }
    }
}

/// KFGameType.DoWaveEnd: every KFDoorMover runs RespawnDoor.
#[derive(Message, Clone, Copy, Debug)]
pub struct RespawnDoors;

impl Doors {
    /// KFDoorMover.RespawnDoor for door `i`; true if it was broken and has
    /// come back. Showing it and its collision is left to the caller.
    fn respawn_door(&mut self, i: usize) -> bool {
        let was_dead = self.doors[i].dead;
        if was_dead {
            let d = &mut self.doors[i];
            d.hidden = false;
            d.dead = false;
            // Reset() in TriggerToggle: Mover.Reset's DoClose (KFDoorMover:
            // skipped while sealed or hidden; neither is true here) and
            // GotoState(InitialState, ''), which stops the latent code. The
            // TriggerToggle part (instant close if bOpening or bDelaying)
            // never runs: DoClose has just cleared both. bClosed is only set
            // by the state code, so it is left as it was: a door broken
            // while open comes back shut but not bClosed, and cannot be
            // welded until it is opened and shut again (KF quirk, copied).
            if d.sealed || d.hidden {
                d.should_be_open = false;
            } else {
                d.interpolate_to(d.key_num.saturating_sub(1), d.info.move_time);
            }
            d.phase = Phase::Idle;
            let last = d.info.num_keys.saturating_sub(1);
            if d.should_be_open {
                if d.key_num != last {
                    d.interpolate_to(last, 0.001);
                }
            } else if d.key_num != 0 {
                d.interpolate_to(0, 0.001);
            }
            if d.info.start_sealed {
                d.sealed = true;
                let amount = d.max_weld * d.info.start_sealed_weld_prc / 100.0;
                if let Some(t) = d.trigger {
                    self.triggers[t].weld_strength = 0.0;
                    self.add_weld(t, amount, false);
                }
            }
        }
        // Every door, broken or not: Health = MaxWeld.
        let d = &mut self.doors[i];
        d.health = d.max_weld;
        was_dead
    }
}

/// DoWaveEnd's door respawn: broken doors come back (shown, solid).
fn respawn_doors(
    mut requests: MessageReader<RespawnDoors>,
    mut doors: ResMut<Doors>,
    mut commands: Commands,
    mut visibility: Query<&mut Visibility>,
) {
    if requests.read().count() == 0 {
        return;
    }
    let mut back = 0;
    for i in 0..doors.doors.len() {
        if !doors.respawn_door(i) {
            continue;
        }
        back += 1;
        let d = &doors.doors[i];
        if let Some(c) = d.collider {
            commands.entity(c).insert(door_layers(&d.info));
        }
        if let Ok(mut v) = visibility.get_mut(d.root) {
            *v = Visibility::Inherited;
        }
        runlog::kv(
            "door_respawned",
            &format!(
                "door={} key={} moving_to_key={} closed={} sealed={} weld={:.0} should_be_open={}",
                d.info.name, d.prev_key_num, d.key_num, d.closed, d.sealed, d.weld, d.should_be_open
            ),
        );
    }
    runlog::kv("doors_respawn", &format!("broken_back={back} health_reset={}", doors.doors.len()));
}

/// KFDoorMover.PostBeginPlay finds its DoorPathNode: the first navigation
/// point within 800 units with a path through the door (TraceThisActor
/// from the point to the path's end hits it), or that path's end if that is
/// closer to the door. KFDoorMover.Tick then sets the node's ExtraCost to
/// its start + 500 + WeldStrength x 6 while sealed, every 0.5 s (first
/// update at a random time within 1 s).
fn door_path_costs(
    time: Res<Time>,
    frames: Res<bevy::diagnostic::FrameCount>,
    spatial: SpatialQuery,
    mut doors: ResMut<Doors>,
    nav: Option<ResMut<crate::world::nav::NavNetwork>>,
    mut next: Local<Vec<f32>>,
    mut done: Local<bool>,
) {
    let Some(mut nav) = nav else {
        return;
    };
    if nav.points.is_empty() || doors.doors.is_empty() || frames.0 < 5 {
        return;
    }
    let now = time.elapsed_secs();
    if !*done {
        *done = true;
        let mut found = 0usize;
        let mut names = Vec::new();
        let mask = SpatialQueryFilter::from_mask([GameLayer::Door, GameLayer::DoorTraces]);
        for i in 0..doors.doors.len() {
            let Some(collider) = doors.doors[i].collider else { continue };
            let at = coords::pos(doors.doors[i].pos);
            'points: for (n, p) in nav.points.iter().enumerate() {
                let dist = p.pos.distance(at) / SCALE;
                if dist >= 800.0 {
                    continue;
                }
                for &(end, _) in &nav.links[n] {
                    let to = nav.points[end].pos;
                    let Ok(dir) = Dir3::new(to - p.pos) else { continue };
                    let hits = spatial.ray_hits(p.pos, dir, (to - p.pos).length(), 8, true, &mask);
                    if !hits.iter().any(|h| h.entity == collider) {
                        continue;
                    }
                    let node = if dist < to.distance(at) / SCALE { n } else { end };
                    doors.doors[i].path_node = Some(node);
                    found += 1;
                    names.push(format!("{}:{}", doors.doors[i].info.name, nav.points[node].name));
                    break 'points;
                }
            }
        }
        *next = (0..doors.doors.len()).map(|i| now + (i as f32 * 0.618).fract()).collect();
        runlog::kv("door_path_nodes", &format!("found={found} of={} nodes=[{}]", doors.doors.len(), names.join(" ")));
    }
    for (i, d) in doors.doors.iter().enumerate() {
        let Some(node) = d.path_node else { continue };
        if next[i] > now {
            continue;
        }
        next[i] = now + 0.5;
        // ExtraCost starts at 0 for every point here (the map's own
        // ExtraCost values are not read).
        let cost = match d.trigger {
            Some(t) if d.sealed => 500.0 + doors.triggers[t].weld_strength * 6.0,
            _ => 0.0,
        };
        if nav.extra_cost[node] != cost {
            runlog::kv("door_path_cost", &format!("door={} node={} extra_cost={cost:.0}", d.info.name, nav.points[node].name));
            nav.extra_cost[node] = cost;
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
            trader: false,
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
            sounds: Default::default(),
            sound_volume: 228,
            sound_radius: 64.0,
            sound_pitch: 64,
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

    fn test_doors(max_weld: f32) -> Doors {
        let mut doors = Doors::default();
        let mut d = test_door(2);
        d.trigger = Some(0);
        d.max_weld = max_weld;
        d.health = max_weld;
        doors.doors.push(d);
        doors.triggers.push(Trigger {
            info: UseTriggerInfo {
                name: "TestTrigger".into(),
                event: "T".into(),
                location: [0.0; 3],
                rotation: Rotator::default(),
                radius: 128.0,
                height: 80.0,
                refire_delay: 1,
                max_weld_strength: max_weld,
                combat_seal_reduction: 0.5,
                directional_open: false,
                message: String::new(),
                always_show_message: false,
            },
            doors: vec![0],
            last_attempt: 0,
            last_message: 0.0,
            init_dir: [1.0, 0.0, 0.0],
            weld_strength: 0.0,
        });
        doors
    }

    #[test]
    fn welding_seals_caps_and_halves_under_attack() {
        let mut doors = test_doors(25.0);
        doors.welder_damage(0, 10.0, false);
        assert!(doors.doors[0].sealed);
        assert_eq!(doors.doors[0].weld, 10.0);
        doors.doors[0].zed_hitting = true;
        doors.welder_damage(0, 10.0, false);
        assert_eq!(doors.doors[0].weld, 15.0); // x CombatSealReduction 0.5
        doors.doors[0].zed_hitting = false;
        doors.welder_damage(0, 10.0, false);
        doors.welder_damage(0, 10.0, false);
        assert_eq!(doors.doors[0].weld, 25.0); // MaxWeldStrength
        // Sealed doors do not open.
        doors.doors[0].trigger("test");
        run(&mut doors.doors[0], 1.5);
        assert_eq!(doors.doors[0].rot[1], 0.0);
    }

    #[test]
    fn open_doors_cannot_be_welded() {
        let mut doors = test_doors(400.0);
        doors.doors[0].trigger("test");
        run(&mut doors.doors[0], 1.5);
        doors.welder_damage(0, 10.0, false);
        assert!(!doors.doors[0].sealed);
        assert_eq!(doors.doors[0].weld, 0.0);
    }

    #[test]
    fn unwelding_goes_below_zero_as_in_kf() {
        let mut doors = test_doors(400.0);
        doors.welder_damage(0, 10.0, false);
        doors.welder_damage(0, 15.0, true);
        assert_eq!(doors.doors[0].weld, -5.0);
        assert!(!doors.doors[0].sealed);
        // The next weld starts from -5.
        doors.welder_damage(0, 10.0, false);
        assert_eq!(doors.doors[0].weld, 5.0);
    }

    #[test]
    fn zed_hits_round_down_floor_at_five_and_break_at_zero() {
        let mut doors = test_doors(400.0);
        doors.welder_damage(0, 10.0, false);
        doors.welder_damage(0, 10.0, false); // weld 20
        // Clot claw 6.3: int 6 x 0.85 = 5.1 -> 5.
        let (broken, _) = doors.zed_damage(0, 6.3);
        assert!(broken.is_empty());
        assert_eq!(doors.doors[0].weld, 15.0);
        // A tiny hit still does 5.
        doors.zed_damage(0, 1.0);
        assert_eq!(doors.doors[0].weld, 10.0);
        // Fleshpound claw 35.9: int 35 x 0.85 = 29.75 -> 29; breaks it.
        let (broken, _) = doors.zed_damage(0, 35.9);
        assert_eq!(broken, vec![0]);
        assert_eq!(doors.doors[0].weld, 0.0);
    }

    #[test]
    fn once_hit_by_a_zed_welding_stays_halved() {
        let mut doors = test_doors(400.0);
        doors.welder_damage(0, 10.0, false);
        doors.zed_damage(0, 6.0);
        assert_eq!(doors.doors[0].weld, 5.0);
        // bZedHittingDoor is never cleared in single player.
        doors.welder_damage(0, 10.0, false);
        assert_eq!(doors.doors[0].weld, 10.0);
    }

    #[test]
    fn only_frag_damage_of_fifty_hurts_doors_from_the_player() {
        let mut doors = test_doors(400.0);
        // Not DamTypeFrag (an M79, a bullet): nothing.
        doors.player_damage(0, 260.0, false);
        assert_eq!(doors.doors[0].health, 400.0);
        // Under DamageThreshold 50: nothing.
        doors.player_damage(0, 49.9, true);
        assert_eq!(doors.doors[0].health, 400.0);
        // Unsealed: half off Health (int).
        doors.player_damage(0, 211.0, true);
        assert_eq!(doors.doors[0].health, 295.0);
        // Sealed: the whole off the weld; at 0 it breaks.
        doors.welder_damage(0, 10.0, false);
        let (broken, _) = doors.player_damage(0, 60.0, true);
        assert_eq!(broken, vec![0]);
    }

    /// What GoBang leaves behind (the drawing and collision are not here).
    fn break_door(doors: &mut Doors, i: usize) {
        let d = &mut doors.doors[i];
        d.hidden = true;
        d.dead = true;
        d.sealed = false;
    }

    #[test]
    fn respawn_brings_a_door_broken_shut_back_shut() {
        let mut doors = test_doors(400.0);
        doors.doors[0].health = 120.0;
        break_door(&mut doors, 0);
        assert!(doors.respawn_door(0));
        let d = &doors.doors[0];
        assert!(!d.hidden && !d.dead && d.closed && !d.sealed);
        assert_eq!((d.key_num, d.health), (0, 400.0));
        // Not broken: nothing but Health.
        assert!(!doors.respawn_door(0));
    }

    #[test]
    fn door_broken_open_glides_shut_but_is_not_closed() {
        let mut doors = test_doors(400.0);
        doors.doors[0].trigger("test");
        run(&mut doors.doors[0], 1.1);
        assert_eq!((doors.doors[0].key_num, doors.doors[0].closed), (1, false));
        break_door(&mut doors, 0);
        doors.respawn_door(0);
        // DoClose: back to key 0 over MoveTime (1 s), not at once.
        run(&mut doors.doors[0], 0.5);
        assert!((doors.doors[0].rot[1] - 8192.0).abs() < 200.0, "yaw {}", doors.doors[0].rot[1]);
        run(&mut doors.doors[0], 0.6);
        assert_eq!(doors.doors[0].rot[1], 0.0);
        // KF quirk: bClosed is still false, so the welder does nothing.
        assert!(!doors.doors[0].closed);
        assert_eq!(doors.welder_damage(0, 10.0, false), "nothing reason=not_closed");
    }

    #[test]
    fn start_sealed_door_comes_back_welded() {
        let mut doors = test_doors(400.0);
        doors.doors[0].info.start_sealed = true;
        doors.doors[0].info.start_sealed_weld_prc = 50.0;
        break_door(&mut doors, 0);
        doors.respawn_door(0);
        let d = &doors.doors[0];
        assert!(d.sealed);
        assert_eq!((d.weld, doors.triggers[0].weld_strength), (200.0, 200.0));
    }

    #[test]
    fn unbroken_door_keeps_its_weld_and_heals() {
        let mut doors = test_doors(400.0);
        doors.add_weld(0, 150.0, false);
        doors.doors[0].health = 10.0;
        assert!(!doors.respawn_door(0));
        assert_eq!((doors.doors[0].weld, doors.doors[0].health), (150.0, 400.0));
    }

    #[test]
    fn opened_while_hidden_comes_back_open_at_once() {
        let mut doors = test_doors(400.0);
        break_door(&mut doors, 0);
        // An open that reaches a hidden door does not move it but sets
        // bShouldBeOpen.
        doors.doors[0].trigger("test");
        run(&mut doors.doors[0], 0.1);
        assert_eq!(doors.doors[0].rot[1], 0.0);
        assert!(doors.doors[0].should_be_open);
        doors.respawn_door(0);
        run(&mut doors.doors[0], 0.02);
        assert_eq!((doors.doors[0].key_num, doors.doors[0].rot[1]), (1, 16384.0));
    }
}
