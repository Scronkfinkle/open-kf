//! Pickups: the weapons, ammo boxes and vests lying in a map (KF's Pickup
//! actors and KFRandomItemSpawn), and the core that dropped weapons and
//! tossed dosh can reuse. Plan and KF's rules: docs/DESIGN.md, "Pickups".
//!
//! Every pickup has a network id: the map's spawn points first, then its
//! ammo boxes, then pickups placed directly, in map order (the same on
//! every game that loaded the map); dropped items get ids from
//! `DYNAMIC_ID_BASE`. In single player and on a network host this game
//! runs KF's rules (`rules.rs`); a network client draws the host's list
//! and asks the host for each item it touches (net/pickups.rs).

pub mod classes;
pub mod drop;
pub mod rules;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use avian3d::prelude::*;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::level::PlacedPickup;
use ue_assets::package::ObjectRef;
use ue_assets::package_set::{LoadedPackage, ObjectHandle, PackageSet};
use ue_assets::properties::{Rotator, Value, read_export_properties};
use ue_assets::static_mesh::read_static_mesh;

pub use classes::{CarriedWeapon, PickupClass, PickupGives};
use rules::{AmmoBox, AmmoPhase, Placed, PlacedPhase, RuleEvent, Rules, Senses, SpawnPoint};

use crate::audio::mixer::{Emitter, PlaySound, Slot as SoundSlot};
use crate::engine::camera::FlyCamera;
use crate::engine::coords::{self, SCALE};
use crate::engine::runlog;
use crate::player::walk::{Walker, kf};

/// Ids of pickups made during the game (dropped weapons, tossed dosh).
pub const DYNAMIC_ID_BASE: u32 = 100_000;
/// KFRandomSpawn.PlayersCanSeeMe's distance.
const SEE_DISTANCE: f32 = 2000.0;
/// Network host: extra reach allowed between where it draws a client's
/// pawn (0.1 s behind) and the pickup (Unreal units).
const REMOTE_REACH_SLACK: f32 = 150.0;
/// A touch waiting for an answer is forgotten after this long (seconds).
const PENDING_TIMEOUT: f64 = 2.0;
/// Client: a drop the host has not answered after this long is given up
/// (its cash or weapon stays with the player).
const DROP_TIMEOUT: f64 = 3.0;
/// Network host: how far from where it draws a client's pawn the client's
/// drop may start (Unreal units).
const DROP_REACH: f32 = 300.0;

/// A map place for a pickup (spawn point, ammo box or placed pickup).
#[derive(Clone, Debug)]
pub struct Spot {
    pub name: String,
    /// Unreal units (a spawn point already 1 lower, as KFRandomSpawn.PostBeginPlay).
    pub location: Vec3,
    pub rotation: Rotator,
    /// The map's own StaticMesh, DrawScale, DrawScale3D, CullDistance
    /// (the Christmas maps' gift-box ammo boxes).
    pub mesh: Option<String>,
    pub draw_scale: Option<f32>,
    pub draw_scale_3d: Option<[f32; 3]>,
    pub cull_distance: Option<f32>,
}

/// One pickup shown in the world, as every game draws it (sent by the
/// host to clients).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ShownPickup {
    pub id: u32,
    /// The pickup class's full path.
    pub class: String,
    /// Resting place (on the floor), Unreal units.
    pub location: [f32; 3],
    /// Unreal rotator (pitch, yaw, roll).
    pub rotation: [i32; 3],
    pub gives: PickupGives,
    /// A dropped item still flying: its arc (cleared by the host when it
    /// lands; `location` is where it lands).
    #[serde(default)]
    pub flight: Option<drop::Flight>,
    /// A dropped item in its 1 s fade-out (Pickup state FadeOut).
    #[serde(default)]
    pub fading: bool,
}

/// A pickup made during the game (host / single player): game seconds of
/// its landing, fade-out and end (None: never).
struct Dynamic {
    land: Option<f64>,
    fade: Option<f64>,
    expires: Option<f64>,
    /// It never lands (fell out of the level): gone when `expires`.
    fell_out: bool,
}

#[derive(Resource, Default)]
pub struct Pickups {
    /// The map's pickup places; index = id.
    pub spots: Vec<Spot>,
    /// Pickup classes by lowercase path.
    pub classes: HashMap<String, PickupClass>,
    /// KF's rules (single player and host; a client does not run them).
    rules: Option<Rules>,
    /// What is shown now, by id.
    pub shown: BTreeMap<u32, ShownPickup>,
    dynamic: BTreeMap<u32, Dynamic>,
    next_dynamic: u32,
    started: bool,
    restarts_seen: u32,
    last_phase: Option<crate::game::waves::Phase>,
    /// The 30 s summary line last written (game time / 30).
    last_summary: Option<u64>,
    /// The last pickup this game saw taken (for the `warp_pickup:taken`
    /// test input).
    last_taken: Option<ShownPickup>,
    /// This game's clock when it began following a dropped item's flight
    /// (host: the drop; client: when the item first arrived), and when its
    /// fade-out began here.
    flight_clock: HashMap<u32, f64>,
    fade_clock: HashMap<u32, f64>,
}

impl Pickups {
    pub fn class(&self, path: &str) -> Option<&PickupClass> {
        self.classes.get(&path.to_ascii_lowercase())
    }

    /// A pickup made during the game that lies still (the `spawn_pickup`
    /// test input): shown at `location` (Unreal units, already where it
    /// rests) until taken or until `lifetime` seconds pass. Host / single
    /// player only; the class must be loaded (`classes`). Returns its id.
    pub fn spawn_dynamic(&mut self, class: &str, location: Vec3, rotation: [i32; 3], gives: PickupGives, lifetime: Option<f64>, now: f64) -> u32 {
        let id = DYNAMIC_ID_BASE + self.next_dynamic;
        self.next_dynamic += 1;
        self.dynamic.insert(id, Dynamic { land: None, fade: None, expires: lifetime.map(|l| now + l), fell_out: false });
        self.shown.insert(id, ShownPickup { id, class: class.to_string(), location: location.to_array(), rotation, gives: gives.clone(), flight: None, fading: false });
        runlog::kv("pickup_spawned", &format!("id={id} class={class} why=dynamic gives={} at=({:.0}, {:.0}, {:.0})", gives.label(), location.x, location.y, location.z));
        id
    }

    /// A dropped item (tossed dosh, a thrown or dropped weapon): it flies
    /// along `flight` from `now` and then follows KF's timers for `life`
    /// (drop.rs). Host / single player only. Returns its id.
    pub fn spawn_dropped(&mut self, class: &str, flight: drop::Flight, rotation: [i32; 3], gives: PickupGives, life: drop::Life, now: f64) -> u32 {
        let id = DYNAMIC_ID_BASE + self.next_dynamic;
        self.next_dynamic += 1;
        let secs = flight.seconds() as f64;
        let t = drop::drop_times(life, secs, flight.landed);
        let rest = flight.path.last().copied().unwrap_or([0.0; 3]);
        let start = flight.path.first().copied().unwrap_or(rest);
        self.dynamic.insert(id, Dynamic { land: Some(now + secs), fade: t.fade.map(|f| now + f), expires: t.gone.map(|g| now + g), fell_out: !flight.landed });
        runlog::kv(
            "pickup_spawned",
            &format!(
                "id={id} class={class} why=dropped gives={} from=({:.0}, {:.0}, {:.0}) rest=({:.0}, {:.0}, {:.0}) flight={secs:.2}s landed={} fade_at={} gone_at={}",
                gives.label(),
                start[0],
                start[1],
                start[2],
                rest[0],
                rest[1],
                rest[2],
                flight.landed,
                t.fade.map_or("never".into(), |f| format!("+{f:.2}s")),
                t.gone.map_or("trader_close".into(), |g| format!("+{g:.2}s"))
            ),
        );
        self.flight_clock.insert(id, now);
        self.shown.insert(id, ShownPickup { id, class: class.to_string(), location: rest, rotation, gives, flight: Some(flight), fading: false });
        id
    }

    /// Where a shown pickup is now (on its arc while flying).
    pub fn position(&self, s: &ShownPickup, now: f64) -> Vec3 {
        match (&s.flight, self.flight_clock.get(&s.id)) {
            (Some(f), Some(t0)) => f.at((now - t0) as f32),
            _ => Vec3::from_array(s.location),
        }
    }

    /// Still flying here.
    pub fn flying(&self, s: &ShownPickup, now: f64) -> bool {
        match (&s.flight, self.flight_clock.get(&s.id)) {
            (Some(f), Some(t0)) => ((now - t0) as f32) < f.seconds(),
            _ => false,
        }
    }

    /// Every dropped item goes (KFGameType.CloseShops; a restart).
    fn clear_dropped(&mut self, why: &str) {
        let ids: Vec<u32> = self.dynamic.keys().copied().collect();
        for id in ids {
            self.dynamic.remove(&id);
            self.flight_clock.remove(&id);
            self.fade_clock.remove(&id);
            if let Some(s) = self.shown.remove(&id) {
                runlog::kv("pickup_hidden", &format!("id={id} class={} why={why}", s.class));
            }
        }
    }
}

/// Network games: what the host and clients send about pickups
/// (net/pickups.rs carries it). In single player nothing uses it.
#[derive(Resource, Default)]
pub struct PickupNet {
    pub role: PickupRole,
    /// Client: requests to send.
    pub outgoing: Vec<PickupRequest>,
    /// Host: requests from clients (peer, request).
    pub incoming: Vec<(u64, PickupRequest)>,
    /// Host: notices to send.
    pub notices: Vec<HostNotice>,
    /// Client: the host's newest list, and notices received.
    pub from_host: Option<Vec<ShownPickup>>,
    pub received: Vec<PickupNotice>,
    /// Client: drops to send, and those waiting for the host (token ->
    /// when asked).
    pub drop_outgoing: Vec<DropRequest>,
    drop_waiting: HashMap<u32, f64>,
    /// Host: drops asked for by clients (peer, request).
    pub drop_incoming: Vec<(u64, DropRequest)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PickupRole {
    #[default]
    Off,
    Host,
    Client,
}

/// "My player takes pickup `id` (of class `class`)" (client to host).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PickupRequest {
    pub id: u32,
    pub class: String,
}

/// What the host tells a client.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum PickupNotice {
    /// Pickup `id` was taken; `yours`: by this client's player (its game
    /// gives the item).
    Taken { id: u32, class: String, location: [f32; 3], gives: PickupGives, yours: bool },
    /// This client's request was refused.
    Denied { id: u32, reason: String },
    /// The answer to this client's drop `token`: the pickup made (`id`),
    /// or None and the reason it was refused.
    Dropped { token: u32, id: Option<u32>, reason: String },
}

/// Host side, before it is addressed per client.
#[derive(Clone, Debug)]
pub enum HostNotice {
    /// `taker`: the peer who took it (None: the host's own player).
    Taken { taker: Option<u64>, id: u32, class: String, location: [f32; 3], gives: PickupGives },
    Denied { peer: u64, id: u32, reason: String },
    Dropped { peer: u64, token: u32, id: Option<u32>, reason: String },
}

/// Why something is dropped (logs; the host allows a dead player only
/// the death drop).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropWhy {
    /// KFPawn.TossCash (B).
    Toss,
    /// ThrowWeapon (Backslash).
    Throw,
    /// KFHumanPawn.VeterancyChanged: over the new carry weight.
    Perk,
    /// Pawn.Died: the weapon in hand.
    Death,
}

/// "Drop this as a pickup": from this game's inventory (`DropItem`), and
/// from a client to the host. Unreal units; `origin` is the pawn's centre
/// (the start is moved back towards it if a wall is in between).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DropRequest {
    /// The asker's own number for it (in the answer).
    pub token: u32,
    /// The pickup class (e.g. KFMod.CashPickup, KFMod.ShotgunPickup).
    pub class: String,
    pub gives: PickupGives,
    pub origin: [f32; 3],
    pub start: [f32; 3],
    pub velocity: [f32; 3],
    /// The pickup's yaw (Unreal units).
    pub yaw: i32,
    pub why: DropWhy,
}

/// This game's inventory asks for a drop (weapons/weapon/drop.rs).
#[derive(Message, Clone, Debug)]
pub struct DropItem(pub DropRequest);

/// The answer: the pickup made (`id`; the cash or weapon now leaves the
/// inventory), or None with the reason.
#[derive(Message, Clone, Debug)]
pub struct DropDone {
    pub token: u32,
    pub id: Option<u32>,
    pub reason: String,
}

/// Ask the inventory (weapons/weapon/pickup.rs) whether this game's
/// player can take a pickup (`apply` false: a dry run, nothing changes) or
/// to give it (`apply` true).
#[derive(Message, Clone, Debug)]
pub struct PickupUse {
    pub id: u32,
    pub class: String,
    pub gives: PickupGives,
    pub apply: bool,
}

/// The inventory's answer.
#[derive(Message, Clone, Debug)]
pub struct PickupUsed {
    pub id: u32,
    pub class: String,
    pub apply: bool,
    pub ok: bool,
    pub detail: String,
    /// The message to show instead of the class's PickupMessage.
    pub message: Option<String>,
}

/// Pickup systems, in this order each frame; the inventory answers
/// between `Touch` and `Answer`.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum PickupSystems {
    Rules,
    Touch,
    Answer,
}

pub struct PickupPlugin;

impl Plugin for PickupPlugin {
    fn build(&self, app: &mut App) {
        use crate::world::map_change::MapResourceExt;
        app.init_resource::<Pickups>()
            .init_resource::<PickupNet>()
            .init_resource::<LocalTouch>()
            // Per map (world/map_change.rs): the map loader inserts the
            // map's `Pickups`; the drawn ones are `MapScoped`.
            .reset_on_map_unload::<Pickups>()
            .reset_on_map_unload::<LocalTouch>()
            .add_systems(crate::world::map_change::MapUnload, |mut models: NonSendMut<PickupModels>| *models = PickupModels::default())
            .insert_non_send(PickupModels::default())
            .add_message::<PickupUse>()
            .add_message::<PickupUsed>()
            .add_message::<DropItem>()
            .add_message::<DropDone>()
            .configure_sets(Update, (PickupSystems::Rules, PickupSystems::Touch, PickupSystems::Answer).chain())
            .add_systems(Update, (run_rules, follow_host, drops).chain().in_set(PickupSystems::Rules))
            .add_systems(Update, (scripted_pickup_input, touch).chain().in_set(PickupSystems::Touch).after(crate::player::walk::WalkSystems))
            .add_systems(Update, (answers, host_requests, sync_visuals).chain().in_set(PickupSystems::Answer));
    }
}

// ------------------------------------------------------------ map load

fn class_of(set: &PackageSet, path: &str) -> Option<ObjectHandle> {
    classes::find_class(set, path)
}

/// Loads a class into `classes` (once); false if it cannot be used.
fn ensure_class(classes: &mut HashMap<String, PickupClass>, set: &PackageSet, defaults: &ClassDefaults, path: &str) -> bool {
    let key = path.to_ascii_lowercase();
    if classes.contains_key(&key) {
        return true;
    }
    match classes::read_class(set, defaults, path) {
        Ok(c) => {
            runlog::kv(
                "pickup_class",
                &format!(
                    "class={} gives={} mesh={} draw_scale={} scale3d={:?} pre_pivot={:?} collision={}x{} sound={} volume={} radius={} message=\"{}\" cull={} respawn={}",
                    c.path,
                    c.gives.label(),
                    c.mesh.as_deref().unwrap_or("none"),
                    c.draw_scale,
                    c.draw_scale_3d,
                    c.pre_pivot,
                    c.radius,
                    c.height,
                    c.sound.as_deref().unwrap_or("none"),
                    c.sound_volume,
                    c.sound_radius,
                    c.message,
                    c.cull_distance,
                    c.respawn_time
                ),
            );
            classes.insert(key, c);
            true
        }
        Err(e) => {
            runlog::kv("pickup_class_error", &format!("class={path} error=\"{e}\""));
            false
        }
    }
}

/// A map export's own object property as a full path.
fn own_object(set: &PackageSet, lp: &Rc<LoadedPackage>, v: Option<&Value>) -> Option<Option<String>> {
    match v {
        Some(Value::Object(r)) if *r == ObjectRef::Null => Some(None),
        Some(Value::Object(r)) => Some(set.resolve(lp, *r).map(|h| h.path())),
        _ => None,
    }
}

/// Reads the map's pickups (map.rs calls this while loading the map).
pub fn load(set: &PackageSet, defaults: &ClassDefaults, lp: &Rc<LoadedPackage>, placed: &[PlacedPickup]) -> Pickups {
    let pkg = &lp.pkg;
    let mut out = Pickups::default();
    let mut spawn_spots = Vec::new();
    let mut ammo_spots = Vec::new();
    let mut placed_spots = Vec::new();
    let (mut spawns, mut ammo, mut direct) = (Vec::new(), Vec::new(), Vec::new());
    let mut custom_lists = 0;
    for p in placed {
        let Some(class) = class_of(set, &p.class) else {
            runlog::kv("pickup_class_error", &format!("class={} error=\"class not found\" actor={}", p.class, p.name));
            continue;
        };
        let props = match read_export_properties(pkg, p.export) {
            Ok(props) => props,
            Err(e) => {
                runlog::kv("pickup_class_error", &format!("actor={} error=\"{e}\"", p.name));
                continue;
            }
        };
        let own_float = |name: &str| match props.get(pkg, name) {
            Some(Value::Float(f)) => Some(*f),
            _ => None,
        };
        let mut spot = Spot {
            name: p.name.clone(),
            location: Vec3::from_array(p.location),
            rotation: p.rotation,
            mesh: own_object(set, lp, props.get(pkg, "StaticMesh")).flatten(),
            draw_scale: own_float("DrawScale"),
            draw_scale_3d: match props.get(pkg, "DrawScale3D") {
                Some(Value::Vector(v)) => Some(*v),
                _ => None,
            },
            cull_distance: own_float("CullDistance"),
        };
        if defaults.is_a(&class, "KFRandomSpawn") {
            // bForceDefault: the class's own list, else the map's (an
            // element the map did not save keeps the class default).
            let force = match props.get(pkg, "bForceDefault") {
                Some(Value::Bool(b)) => *b,
                _ => matches!(defaults.get(&class, "bForceDefault"), Some((Value::Bool(true), _))),
            };
            custom_lists += usize::from(!force);
            let mut list = Vec::new();
            for k in 0..11u32 {
                let own = if force { None } else { own_object(set, lp, props.get_at(pkg, "PickupClasses", k)) };
                let path = match own {
                    Some(p) => p,
                    None => match defaults.get_at(&class, "PickupClasses", k) {
                        Some((Value::Object(r), from)) if r != ObjectRef::Null => set.resolve(&from, r).map(|h| h.path()),
                        _ => None,
                    },
                };
                // NumClasses: up to the first None.
                let Some(path) = path else { break };
                let weight = match (if force { None } else { props.get_at(pkg, "PickupWeight", k).cloned() }).or_else(|| defaults.get_at(&class, "PickupWeight", k).map(|v| v.0)) {
                    Some(Value::Int(w)) => w,
                    _ => 0,
                };
                if ensure_class(&mut out.classes, set, defaults, &path) {
                    let weapon = out.class(&path).is_some_and(|c| c.weapon_pickup);
                    list.push((path, weight, weapon));
                }
            }
            if list.is_empty() {
                runlog::kv("pickup_class_error", &format!("actor={} error=\"no usable PickupClasses\"", p.name));
                continue;
            }
            spot.location.z -= 1.0;
            spawns.push(SpawnPoint::new(list));
            spawn_spots.push(spot);
        } else if defaults.is_a(&class, "KFAmmoPickup") {
            let path = class.path();
            if !ensure_class(&mut out.classes, set, defaults, &path) {
                continue;
            }
            let respawn_time = own_float("RespawnTime").unwrap_or_else(|| out.class(&path).map_or(30.0, |c| c.respawn_time));
            ammo.push(AmmoBox { class: path, respawn_time, phase: AmmoPhase::Asleep });
            ammo_spots.push(spot);
        } else {
            let path = class.path();
            if !ensure_class(&mut out.classes, set, defaults, &path) {
                continue;
            }
            let c = out.class(&path).cloned();
            let respawn_time = own_float("RespawnTime").unwrap_or_else(|| c.as_ref().map_or(0.0, |c| c.respawn_time));
            direct.push(Placed { class: path, respawn_time, weapon_pickup: c.is_some_and(|c| c.weapon_pickup), phase: PlacedPhase::Shown });
            placed_spots.push(spot);
        }
    }
    runlog::kv(
        "pickups_loaded",
        &format!(
            "spawn_points={} custom_lists={custom_lists} ammo_boxes={} placed={} classes={} ammo_respawn={:?} placed_classes=[{}]",
            spawns.len(),
            ammo.len(),
            direct.len(),
            out.classes.len(),
            ammo.iter().map(|a| a.respawn_time as i64).collect::<std::collections::BTreeSet<_>>(),
            direct.iter().map(|d| d.class.as_str()).collect::<Vec<_>>().join(" ")
        ),
    );
    out.spots = spawn_spots.into_iter().chain(ammo_spots).chain(placed_spots).collect();
    out.rules = Some(Rules::new(spawns, ammo, direct));
    out
}

// ------------------------------------------------------------ helpers

/// FastTrace: nothing of the level between two points (Unreal units).
fn fast_trace(spatial: &SpatialQuery, a: Vec3, b: Vec3) -> bool {
    let (from, to) = (coords::pos(a.to_array()), coords::pos(b.to_array()));
    let Ok(dir) = Dir3::new(to - from) else { return true };
    spatial.cast_ray(from, dir, (to - from).length(), true, &crate::world::collision::world_filter()).is_none()
}

/// Bevy -> Unreal units.
fn to_unreal(v: Vec3) -> Vec3 {
    Vec3::new(-v.z, v.x, v.y) / SCALE
}

/// Where a pickup comes to rest (Physics PHYS_Falling): straight down to
/// the floor, its collision cylinder standing on it. Unreal units.
fn rest_on_floor(spatial: &SpatialQuery, at: Vec3, height: f32) -> Vec3 {
    let from = coords::pos(at.to_array());
    let max = 1024.0 * SCALE;
    match spatial.cast_ray(from, Dir3::NEG_Y, max, true, &crate::world::collision::world_filter()) {
        Some(hit) => Vec3::new(at.x, at.y, at.z - hit.distance / SCALE + height),
        None => at,
    }
}

/// The player pawns the rules look at (Unreal centres): this game's player
/// (the camera in fly mode) if alive, and on a host the other living players.
struct HostSenses<'a, 'w, 's> {
    spatial: &'a SpatialQuery<'w, 's>,
    players: Vec<Vec3>,
    connected: usize,
    spots: &'a [Spot],
}

impl HostSenses<'_, '_, '_> {
    fn at(&self, id: u32) -> Option<Vec3> {
        self.spots.get(id as usize).map(|s| s.location)
    }
}

impl Senses for HostSenses<'_, '_, '_> {
    fn players_can_see(&self, id: u32) -> bool {
        let Some(at) = self.at(id) else { return false };
        self.players.iter().any(|p| p.distance(at) < SEE_DISTANCE && fast_trace(self.spatial, *p + Vec3::Z * kf::EYE_HEIGHT, at))
    }
    fn line_to_player(&self, id: u32) -> bool {
        let Some(at) = self.at(id) else { return false };
        self.players.iter().any(|p| fast_trace(self.spatial, at, *p))
    }
    fn player_sees(&self, id: u32) -> bool {
        let Some(at) = self.at(id) else { return false };
        self.players.iter().any(|p| fast_trace(self.spatial, *p + Vec3::Z * kf::EYE_HEIGHT, at))
    }
    fn living_players(&self) -> usize {
        self.players.len().max(1)
    }
    fn num_players(&self) -> usize {
        self.connected.max(1)
    }
}

type PlayerQuery<'w, 's> = Query<'w, 's, (&'static Transform, Option<&'static Walker>), With<FlyCamera>>;

/// This game's player's cylinder centre (Unreal units) and whether it has a
/// walking pawn.
fn local_player(player: &PlayerQuery) -> Option<(Vec3, bool)> {
    let (t, walker) = player.single().ok()?;
    let c = walker.map_or(t.translation - Vec3::Y * kf::EYE_HEIGHT * SCALE, |w| w.center);
    Some((to_unreal(c), walker.is_some()))
}

fn role_of(mode: Option<&crate::net::NetMode>) -> PickupRole {
    match mode {
        Some(crate::net::NetMode::Host { .. }) => PickupRole::Host,
        Some(crate::net::NetMode::Client { .. }) => PickupRole::Client,
        _ => PickupRole::Off,
    }
}

/// The rules' changes into the shown list.
fn apply_events(p: &mut Pickups, spatial: &SpatialQuery) {
    let Some(rules) = p.rules.as_mut() else { return };
    let events = std::mem::take(&mut rules.events);
    for ev in events {
        match ev {
            RuleEvent::Shown { id, class, why } => {
                let Some(spot) = p.spots.get(id as usize) else { continue };
                let Some(c) = p.classes.get(&class.to_ascii_lowercase()) else { continue };
                let rest = rest_on_floor(spatial, spot.location, c.height);
                // Spawn points: the spot's yaw, lying flat (DESIGN.md:
                // guess); placed pickups and ammo boxes: their rotation.
                let r = spot.rotation;
                let rotation = if (id as usize) < p.rules.as_ref().map_or(0, |r| r.spawns.len()) { [0, r.yaw, 0] } else { [r.pitch, r.yaw, r.roll] };
                runlog::kv(
                    if why == "respawned" { "pickup_respawned" } else { "pickup_spawned" },
                    &format!("id={id} class={class} why={why} spot={} at=({:.0}, {:.0}, {:.0}) dropped={:.0}", spot.name, rest.x, rest.y, rest.z, spot.location.z - rest.z),
                );
                p.shown.insert(id, ShownPickup { id, class, location: rest.to_array(), rotation, gives: c.gives.clone(), flight: None, fading: false });
            }
            RuleEvent::Hidden { id, why } => {
                if let Some(s) = p.shown.remove(&id) {
                    runlog::kv("pickup_hidden", &format!("id={id} class={} why={why}", s.class));
                }
            }
        }
    }
}

// ------------------------------------------------------------ rules (single player, host)

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn run_rules(
    time: Res<Time>,
    frames: Res<bevy::diagnostic::FrameCount>,
    menus: Res<crate::game::menus::MenuState>,
    options: Res<crate::game::waves::GameOptions>,
    game: Res<crate::game::waves::WaveGame>,
    mode: Option<Res<crate::net::NetMode>>,
    mut pickups: ResMut<Pickups>,
    mut net: ResMut<PickupNet>,
    spatial: SpatialQuery,
    player: PlayerQuery,
    health: Res<crate::game::combat::PlayerHealth>,
    remote: Res<crate::game::combat::RemotePlayers>,
) {
    net.role = role_of(mode.as_deref());
    if net.role == PickupRole::Client || pickups.rules.is_none() {
        return;
    }
    // MatchInProgress (as the wave timer: after the lobby).
    if frames.0 < 10 || menus.lobby_open() {
        return;
    }
    let now = time.elapsed_secs_f64();
    let p = &mut *pickups;
    let mut players: Vec<Vec3> = Vec::new();
    if !health.dead
        && let Some((c, _)) = local_player(&player)
    {
        players.push(c);
    }
    players.extend(remote.0.iter().filter(|r| r.alive).map(|r| to_unreal(r.centre)));
    let spots = std::mem::take(&mut p.spots);
    let senses = HostSenses { spatial: &spatial, players, connected: 1 + remote.0.len(), spots: &spots };
    let waves = options.mode == crate::game::waves::GameMode::Waves;
    let mut setup = None;
    if !p.started {
        p.started = true;
        p.restarts_seen = game.restarts;
        setup = Some("match_start");
    } else if waves && game.restarts != p.restarts_seen {
        p.restarts_seen = game.restarts;
        if let Some(r) = p.rules.as_mut() {
            r.reset();
        }
        p.clear_dropped("restart");
        setup = Some("restart");
    }
    if waves {
        use crate::game::waves::Phase;
        let phase = game.phase;
        if p.last_phase == Some(Phase::Countdown) && matches!(phase, Phase::Wave | Phase::BossWave) && game.wave_num > 0 && setup.is_none() {
            setup = Some("wave_start");
            // KFGameType.CloseShops: every dropped pickup is destroyed.
            p.clear_dropped("close_shops");
        }
        p.last_phase = Some(phase);
    }
    let rules = p.rules.as_mut().expect("checked");
    if let Some(why) = setup {
        let (on_w, on_a) = rules.setup_pickups(now, crate::game::difficulty::game_difficulty(), &senses);
        runlog::kv(
            "pickup_setup",
            &format!(
                "reason={why} wave={} difficulty={} spawn_points_on={}/{} {:?} ammo_on={}/{} {:?} placed={}",
                game.wave_num + 1,
                crate::game::difficulty::game_difficulty(),
                on_w.len(),
                rules.spawns.len(),
                on_w,
                on_a.len(),
                rules.ammo.len(),
                on_a,
                rules.placed.len()
            ),
        );
    }
    rules.tick(now, &senses);
    drop(senses);
    p.spots = spots;
    apply_events(p, &spatial);
    // Dropped items: landing (the arc is no longer sent), fade-out, end.
    let mut landed = Vec::new();
    let mut fading = Vec::new();
    let mut expired = Vec::new();
    for (id, d) in p.dynamic.iter_mut() {
        if d.land.is_some_and(|t| now >= t) {
            d.land = None;
            landed.push((*id, d.fell_out));
        }
        if d.fade.is_some_and(|t| now >= t) {
            d.fade = None;
            fading.push(*id);
        }
        if d.expires.is_some_and(|t| now >= t) {
            expired.push((*id, d.fell_out));
        }
    }
    for (id, fell_out) in landed {
        p.flight_clock.remove(&id);
        if let Some(s) = p.shown.get_mut(&id) {
            s.flight = None;
            if !fell_out {
                runlog::kv("pickup_landed", &format!("id={id} class={} at=({:.0}, {:.0}, {:.0})", s.class, s.location[0], s.location[1], s.location[2]));
            }
        }
    }
    for id in fading {
        if let Some(s) = p.shown.get_mut(&id) {
            s.fading = true;
            runlog::kv("pickup_fading", &format!("id={id} class={} seconds={}", s.class, drop::FADE_SECONDS));
        }
        p.fade_clock.insert(id, now);
    }
    for (id, fell_out) in expired {
        p.dynamic.remove(&id);
        p.flight_clock.remove(&id);
        p.fade_clock.remove(&id);
        if let Some(s) = p.shown.remove(&id) {
            runlog::kv("pickup_hidden", &format!("id={id} class={} why={}", s.class, if fell_out { "fell_out" } else { "expired" }));
        }
    }
    // A summary line every 30 s of game time.
    let tick = (now / 30.0) as u64;
    if p.last_summary != Some(tick)
        && let Some(r) = p.rules.as_ref()
    {
        p.last_summary = Some(tick);
        let (on, out, ammo, coming) = r.counts();
        runlog::kv("pickup_summary", &format!("spawn_points_on={on} items_out={out} ammo_shown={ammo} ammo_coming_back={coming} shown_total={}", p.shown.len()));
    }
}

/// The take itself (single player / host), after the checks: the rules
/// (or the dropped item) let go of it, the sound plays at it, the clients
/// hear of it, this game's player gets it if it was theirs.
#[allow(clippy::too_many_arguments)]
fn take(
    p: &mut Pickups,
    net: &mut PickupNet,
    id: u32,
    class: &str,
    taker: Option<u64>,
    now: f64,
    senses: &dyn Senses,
    spatial: &SpatialQuery,
    sounds: &mut MessageWriter<PlaySound>,
    uses: &mut MessageWriter<PickupUse>,
) -> Result<(), &'static str> {
    let Some(mut shown) = p.shown.get(&id).cloned() else { return Err("not_shown") };
    if !shown.class.eq_ignore_ascii_case(class) {
        return Err("class_changed");
    }
    // Taken in the air: where it is now.
    shown.location = p.position(&shown, now).to_array();
    if id >= DYNAMIC_ID_BASE {
        p.dynamic.remove(&id);
        p.shown.remove(&id);
        p.flight_clock.remove(&id);
        p.fade_clock.remove(&id);
    } else {
        let ok = p.rules.as_mut().is_some_and(|r| r.take(id, now, senses));
        if !ok {
            return Err("not_shown");
        }
        apply_events(p, spatial);
        p.shown.remove(&id);
    }
    runlog::kv(
        "pickup_collected",
        &format!("id={id} class={} by={} gives={} at=({:.0}, {:.0}, {:.0})", shown.class, taker.map_or("local".to_string(), |t| format!("peer:{t}")), shown.gives.label(), shown.location[0], shown.location[1], shown.location[2]),
    );
    play_pickup_sound(p, &shown, sounds);
    p.last_taken = Some(shown.clone());
    if net.role == PickupRole::Host {
        net.notices.push(HostNotice::Taken { taker, id, class: shown.class.clone(), location: shown.location, gives: shown.gives.clone() });
    }
    if taker.is_none() {
        uses.write(PickupUse { id, class: shown.class, gives: shown.gives, apply: true });
    }
    Ok(())
}

/// Pickup.AnnouncePickup: PlaySound(PickupSound, SLOT_Interact) at the
/// pickup, with its TransientSoundVolume / Radius.
fn play_pickup_sound(p: &Pickups, shown: &ShownPickup, sounds: &mut MessageWriter<PlaySound>) {
    let Some(c) = p.class(&shown.class) else { return };
    let Some(sound) = c.sound.clone() else { return };
    let at = coords::pos(shown.location);
    sounds.write(PlaySound::new(sound, Emitter::Point(at)).slot(SoundSlot::Interact).volume(c.sound_volume).radius(c.sound_radius));
}

// ------------------------------------------------------------ client

/// A network client: the host's list is what is shown; the host's notices.
#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn follow_host(
    time: Res<Time>,
    mut pickups: ResMut<Pickups>,
    mut net: ResMut<PickupNet>,
    mut touch: ResMut<LocalTouch>,
    mut sounds: MessageWriter<PlaySound>,
    mut uses: MessageWriter<PickupUse>,
    mut dones: MessageWriter<DropDone>,
) {
    if net.role != PickupRole::Client {
        return;
    }
    let now = time.elapsed_secs_f64();
    let p = &mut *pickups;
    if let Some(list) = net.from_host.take() {
        let new: BTreeMap<u32, ShownPickup> = list.into_iter().map(|s| (s.id, s)).collect();
        for (id, s) in &new {
            match p.shown.get(id) {
                None => {
                    runlog::kv(
                        "pickup_spawned",
                        &format!(
                            "id={id} class={} why=host gives={} at=({:.0}, {:.0}, {:.0}) flight={}",
                            s.class,
                            s.gives.label(),
                            s.location[0],
                            s.location[1],
                            s.location[2],
                            s.flight.as_ref().map_or("none".into(), |f| format!("{:.2}s", f.seconds()))
                        ),
                    );
                    // A dropped item's arc is replayed from now.
                    if s.flight.is_some() {
                        p.flight_clock.insert(*id, now);
                    }
                    if s.fading {
                        p.fade_clock.insert(*id, now);
                    }
                }
                Some(old) if old != s => {
                    let what = if s.class != old.class {
                        "class"
                    } else if s.fading && !old.fading {
                        "fading"
                    } else if s.flight.is_none() && old.flight.is_some() {
                        "landed"
                    } else {
                        "other"
                    };
                    runlog::kv("pickup_changed", &format!("id={id} class={} what={what} at=({:.0}, {:.0}, {:.0})", s.class, s.location[0], s.location[1], s.location[2]));
                    if s.fading && !old.fading {
                        p.fade_clock.insert(*id, now);
                    }
                    if s.flight.is_none() {
                        p.flight_clock.remove(id);
                    }
                }
                _ => {}
            }
        }
        for (id, s) in &p.shown {
            if !new.contains_key(id) {
                runlog::kv("pickup_hidden", &format!("id={id} class={} why=host", s.class));
            }
        }
        p.flight_clock.retain(|id, _| new.contains_key(id));
        p.fade_clock.retain(|id, _| new.contains_key(id));
        p.shown = new;
    }
    // Drops the host did not answer.
    let late: Vec<u32> = net.drop_waiting.iter().filter(|(_, t)| now - **t > DROP_TIMEOUT).map(|(k, _)| *k).collect();
    for token in late {
        net.drop_waiting.remove(&token);
        runlog::kv("net_drop_answer", &format!("token={token} id=none reason=timeout"));
        dones.write(DropDone { token, id: None, reason: "timeout".into() });
    }
    for n in std::mem::take(&mut net.received) {
        match n {
            PickupNotice::Taken { id, class, location, gives, yours } => {
                runlog::kv("net_pickup_taken", &format!("id={id} class={class} yours={yours}"));
                let mut shown = p.shown.remove(&id).unwrap_or(ShownPickup { id, class: class.clone(), location, rotation: [0; 3], gives: gives.clone(), flight: None, fading: false });
                shown.location = location;
                p.flight_clock.remove(&id);
                p.fade_clock.remove(&id);
                if p.class(&class).is_some() {
                    play_pickup_sound(p, &shown, &mut sounds);
                }
                p.last_taken = Some(shown);
                if yours {
                    touch.pending.remove(&id);
                    uses.write(PickupUse { id, class, gives, apply: true });
                }
            }
            PickupNotice::Denied { id, reason } => {
                touch.pending.remove(&id);
                runlog::kv("pickup_denied", &format!("id={id} reason={reason} by=host"));
            }
            PickupNotice::Dropped { token, id, reason } => {
                if net.drop_waiting.remove(&token).is_none() {
                    runlog::kv("net_drop_answer", &format!("token={token} id={id:?} reason={reason} late=true"));
                    continue;
                }
                runlog::kv("net_drop_answer", &format!("token={token} id={} reason={reason}", id.map_or("none".into(), |i| i.to_string())));
                dones.write(DropDone { token, id, reason });
            }
        }
    }
}

// ------------------------------------------------------------ drops

/// Rays into the level for the falling arc (Unreal units in and out).
struct LevelTracer<'a, 'w, 's>(&'a SpatialQuery<'w, 's>);

impl drop::Tracer for LevelTracer<'_, '_, '_> {
    fn trace(&self, from: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3)> {
        let d = Dir3::new(coords::dir(dir.to_array())).ok()?;
        let hit = self.0.cast_ray(coords::pos(from.to_array()), d, max * SCALE, true, &crate::world::collision::world_filter())?;
        let n = hit.normal;
        Some((hit.distance / SCALE, Vec3::new(-n.z, n.x, n.y).normalize_or_zero()))
    }
}

/// Makes the pickup for a drop (single player / host): the class loaded,
/// the start kept on the pawn's side of a wall, the arc worked out.
fn make_drop(p: &mut Pickups, models: &mut PickupModels, install_root: &std::path::Path, spatial: &SpatialQuery, r: &DropRequest, now: f64) -> Result<u32, String> {
    if p.class(&r.class).is_none() {
        if models.set.is_none() {
            models.set = Some(PackageSet::new(install_root));
        }
        let set = models.set.as_ref().expect("just set");
        let defaults = ClassDefaults::new(set);
        ensure_class(&mut p.classes, set, &defaults, &r.class);
    }
    let c = p.class(&r.class).cloned().ok_or_else(|| "class_not_loaded".to_string())?;
    let tracer = LevelTracer(spatial);
    let origin = Vec3::from_array(r.origin);
    let mut start = Vec3::from_array(r.start);
    let d = start - origin;
    if d.length() > 1e-3
        && let Some((dist, _)) = drop::Tracer::trace(&tracer, origin, d.normalize(), d.length())
    {
        start = origin + d.normalize() * (dist - 2.0).max(0.0);
    }
    let flight = drop::fly(&tracer, start, Vec3::from_array(r.velocity), c.height);
    let life = if matches!(r.gives, PickupGives::Cash { .. }) { drop::Life::Cash } else { drop::Life::Weapon };
    // A weapon pickup gives its class's InventoryType with that Weight
    // (the dual handcannons drop a DeaglePickup), with the dropped state.
    let gives = match (&r.gives, &c.gives) {
        (PickupGives::Weapon { carried, .. }, PickupGives::Weapon { weapon, weight, .. }) => PickupGives::Weapon { weapon: weapon.clone(), weight: *weight, carried: *carried },
        (g, _) => g.clone(),
    };
    Ok(p.spawn_dropped(&c.path, flight, [0, r.yaw, 0], gives, life, now))
}

/// Drops from this game's inventory and (host) from clients: made into
/// pickups here (single player / host) or sent to the host (client).
#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn drops(
    time: Res<Time>,
    mut items: MessageReader<DropItem>,
    mut pickups: ResMut<Pickups>,
    mut net: ResMut<PickupNet>,
    mut models: NonSendMut<PickupModels>,
    request: Res<crate::world::map::MapRequest>,
    spatial: SpatialQuery,
    remote: Res<crate::game::combat::RemotePlayers>,
    mut dones: MessageWriter<DropDone>,
) {
    let now = time.elapsed_secs_f64();
    let p = &mut *pickups;
    for DropItem(r) in items.read() {
        let v = Vec3::from_array(r.velocity);
        runlog::kv(
            "drop_request",
            &format!(
                "token={} why={:?} class={} gives={} start=({:.0}, {:.0}, {:.0}) velocity=({:.0}, {:.0}, {:.0}) role={:?}",
                r.token,
                r.why,
                r.class,
                r.gives.label(),
                r.start[0],
                r.start[1],
                r.start[2],
                v.x,
                v.y,
                v.z,
                net.role
            ),
        );
        if net.role == PickupRole::Client {
            net.drop_waiting.insert(r.token, now);
            net.drop_outgoing.push(r.clone());
            continue;
        }
        let (id, reason) = match make_drop(p, &mut models, &request.install_root, &spatial, r, now) {
            Ok(id) => (Some(id), "ok".to_string()),
            Err(e) => (None, e),
        };
        dones.write(DropDone { token: r.token, id, reason });
    }
    if net.role != PickupRole::Host {
        return;
    }
    for (peer, r) in std::mem::take(&mut net.drop_incoming) {
        runlog::kv("net_drop_request_received", &format!("peer={peer} token={} why={:?} class={} gives={}", r.token, r.why, r.class, r.gives.label()));
        let check = match remote.0.iter().find(|x| x.peer == peer) {
            None => Err("no_pawn".to_string()),
            Some(x) if !x.alive && r.why != DropWhy::Death => Err("dead".to_string()),
            Some(x) => {
                let off = to_unreal(x.centre).distance(Vec3::from_array(r.origin));
                if off > DROP_REACH {
                    runlog::kv("drop_reach", &format!("peer={peer} token={} distance={off:.0}", r.token));
                    Err("too_far".to_string())
                } else {
                    Ok(())
                }
            }
        };
        let result = check.and_then(|_| make_drop(p, &mut models, &request.install_root, &spatial, &r, now));
        let (id, reason) = match result {
            Ok(id) => (Some(id), "ok".to_string()),
            Err(e) => {
                runlog::kv("drop_denied", &format!("peer={peer} token={} reason={e}", r.token));
                (None, e)
            }
        };
        net.notices.push(HostNotice::Dropped { peer, token: r.token, id, reason });
    }
}

// ------------------------------------------------------------ touching

/// This game's player's touches: what it overlapped last frame, and touches
/// waiting for an answer (dry run, then on a client the host's).
#[derive(Resource, Default)]
pub struct LocalTouch {
    overlapping: HashSet<(u32, String)>,
    pending: HashMap<u32, f64>,
    /// Pickups this game has looked at, and dropped ones it saw flying
    /// (FallingPickup: no CheckTouching until it lands).
    seen: HashSet<u32>,
    flying: HashSet<u32>,
}

/// Touch: the player's cylinder overlaps the pickup's and a clear line
/// joins their centres (Pickup.ValidTouch), checked when the overlap
/// begins (or the pickup appears on the player: CheckTouching).
#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn touch(
    time: Res<Time>,
    pickups: Res<Pickups>,
    mut state: ResMut<LocalTouch>,
    player: PlayerQuery,
    health: Res<crate::game::combat::PlayerHealth>,
    menus: Res<crate::game::menus::MenuState>,
    spatial: SpatialQuery,
    mut uses: MessageWriter<PickupUse>,
) {
    let now = time.elapsed_secs_f64();
    state.pending.retain(|_, t| now - *t < PENDING_TIMEOUT);
    let Some((me, walking)) = local_player(&player) else { return };
    if !walking || health.dead || menus.lobby_open() {
        state.overlapping.clear();
        return;
    }
    let mut now_over = HashSet::new();
    let state = &mut *state;
    state.seen.retain(|id| pickups.shown.contains_key(id));
    state.flying.retain(|id| pickups.shown.contains_key(id));
    for s in pickups.shown.values() {
        let Some(c) = pickups.class(&s.class) else { continue };
        let first = state.seen.insert(s.id);
        let flying = pickups.flying(s, now);
        if flying {
            state.flying.insert(s.id);
        } else if state.flying.remove(&s.id) {
            // Landed: state Pickup's Begin, CheckTouching (whoever stands on it).
            state.overlapping.retain(|(id, _)| *id != s.id);
        }
        let at = pickups.position(s, now);
        let d = at - me;
        if d.truncate().length() > kf::RADIUS + c.radius || d.z.abs() > kf::HALF_HEIGHT + c.height {
            continue;
        }
        let key = (s.id, s.class.clone());
        let entering = !state.overlapping.contains(&key);
        now_over.insert(key);
        // A dropped item appearing inside this player (the thrower) is no
        // touch: a touch is the start of an overlap.
        if first && flying {
            runlog::kv("pickup_touch_skipped", &format!("id={} class={} reason=inside_when_dropped", s.id, s.class));
            continue;
        }
        if !entering || state.pending.contains_key(&s.id) {
            continue;
        }
        if !fast_trace(&spatial, me, at) {
            runlog::kv("pickup_touch", &format!("id={} class={} blocked=true", s.id, s.class));
            continue;
        }
        runlog::kv("pickup_touch", &format!("id={} class={} gives={} player=({:.0}, {:.0}, {:.0}) distance={:.0} flying={flying}", s.id, s.class, s.gives.label(), me.x, me.y, me.z, d.length()));
        state.pending.insert(s.id, now);
        uses.write(PickupUse { id: s.id, class: s.class.clone(), gives: s.gives.clone(), apply: false });
    }
    state.overlapping = now_over;
}

/// The inventory's answers: a dry run that fits is taken (single player /
/// host) or asked for (client); a given item shows its message.
#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn answers(
    time: Res<Time>,
    mut used: MessageReader<PickupUsed>,
    mut pickups: ResMut<Pickups>,
    mut net: ResMut<PickupNet>,
    mut state: ResMut<LocalTouch>,
    spatial: SpatialQuery,
    player: PlayerQuery,
    remote: Res<crate::game::combat::RemotePlayers>,
    mut sounds: MessageWriter<PlaySound>,
    mut uses: MessageWriter<PickupUse>,
    mut messages: MessageWriter<crate::game::hud::LocalMessage>,
) {
    let now = time.elapsed_secs_f64();
    let answers: Vec<PickupUsed> = used.read().cloned().collect();
    for a in answers {
        if a.apply {
            if a.ok {
                let text = a.message.clone().unwrap_or_else(|| pickups.class(&a.class).map(|c| c.message.clone()).unwrap_or_default());
                runlog::kv("pickup_given", &format!("id={} class={} {} message=\"{text}\"", a.id, a.class, a.detail));
                if !text.is_empty() {
                    messages.write(crate::game::hud::LocalMessage::pickup(text));
                }
            } else {
                runlog::kv("pickup_apply_failed", &format!("id={} class={} reason={}", a.id, a.class, a.detail));
            }
            continue;
        }
        if !a.ok {
            state.pending.remove(&a.id);
            runlog::kv("pickup_refused", &format!("id={} class={} reason={}", a.id, a.class, a.detail));
            continue;
        }
        match net.role {
            PickupRole::Client => {
                runlog::kv("net_pickup_request", &format!("id={} class={}", a.id, a.class));
                net.outgoing.push(PickupRequest { id: a.id, class: a.class.clone() });
            }
            PickupRole::Off | PickupRole::Host => {
                state.pending.remove(&a.id);
                let p = &mut *pickups;
                let spots = std::mem::take(&mut p.spots);
                let mut players = Vec::new();
                if let Some((c, _)) = local_player(&player) {
                    players.push(c);
                }
                players.extend(remote.0.iter().filter(|r| r.alive).map(|r| to_unreal(r.centre)));
                let senses = HostSenses { spatial: &spatial, players, connected: 1 + remote.0.len(), spots: &spots };
                let r = take(p, &mut net, a.id, &a.class, None, now, &senses, &spatial, &mut sounds, &mut uses);
                drop(senses);
                p.spots = spots;
                if let Err(why) = r {
                    runlog::kv("pickup_denied", &format!("id={} class={} reason={why} by=local", a.id, a.class));
                }
            }
        }
    }
}

/// Host: clients' requests, checked and taken as for its own player.
#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn host_requests(
    time: Res<Time>,
    mut pickups: ResMut<Pickups>,
    mut net: ResMut<PickupNet>,
    spatial: SpatialQuery,
    player: PlayerQuery,
    remote: Res<crate::game::combat::RemotePlayers>,
    mut sounds: MessageWriter<PlaySound>,
    mut uses: MessageWriter<PickupUse>,
) {
    if net.role != PickupRole::Host || net.incoming.is_empty() {
        return;
    }
    let now = time.elapsed_secs_f64();
    let p = &mut *pickups;
    let spots = std::mem::take(&mut p.spots);
    let mut players = Vec::new();
    if let Some((c, _)) = local_player(&player) {
        players.push(c);
    }
    players.extend(remote.0.iter().filter(|r| r.alive).map(|r| to_unreal(r.centre)));
    let senses = HostSenses { spatial: &spatial, players, connected: 1 + remote.0.len(), spots: &spots };
    for (peer, req) in std::mem::take(&mut net.incoming) {
        runlog::kv("net_pickup_request_received", &format!("peer={peer} id={} class={}", req.id, req.class));
        let check = match (remote.0.iter().find(|r| r.peer == peer), p.shown.get(&req.id)) {
            (_, None) => Err("not_shown"),
            (None, _) => Err("no_pawn"),
            (Some(r), _) if !r.alive => Err("dead"),
            (Some(r), Some(s)) => {
                let c = p.class(&s.class);
                let (radius, height) = c.map_or((30.0, 30.0), |c| (c.radius, c.height));
                let d = p.position(s, now) - to_unreal(r.centre);
                if d.truncate().length() > kf::RADIUS + radius + REMOTE_REACH_SLACK || d.z.abs() > kf::HALF_HEIGHT + height + REMOTE_REACH_SLACK {
                    runlog::kv("pickup_reach", &format!("peer={peer} id={} horizontal={:.0} vertical={:.0}", req.id, d.truncate().length(), d.z.abs()));
                    Err("too_far")
                } else {
                    Ok(())
                }
            }
        };
        let result = check.and_then(|_| take(p, &mut net, req.id, &req.class, Some(peer), now, &senses, &spatial, &mut sounds, &mut uses));
        if let Err(reason) = result {
            runlog::kv("pickup_denied", &format!("id={} class={} reason={reason} by=peer:{peer}", req.id, req.class));
            net.notices.push(HostNotice::Denied { peer, id: req.id, reason: reason.to_string() });
        }
    }
    drop(senses);
    p.spots = spots;
}

// ------------------------------------------------------------ test inputs

/// Test actions: `warp_pickup:KIND[:DIST]` puts the player on (or DIST
/// units from, facing) the lowest-id shown pickup of KIND (weapon, ammo,
/// vest, any; `dynamic`: the newest dropped-style one; `taken`: where the
/// last pickup this game saw taken was); `look_pickup:KIND`
/// turns the view to it;
/// `spawn_pickup:CLASS[:DIST]` (single player / host) makes a dropped-style
/// pickup of CLASS (e.g. KFMod.ShotgunPickup) DIST units (default 150)
/// ahead of the player, resting on the floor, for 60 s.
#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn scripted_pickup_input(
    time: Res<Time>,
    script: Res<crate::weapons::weapon::ScriptedInput>,
    frames: Res<bevy::diagnostic::FrameCount>,
    mut pickups: ResMut<Pickups>,
    net: Res<PickupNet>,
    request: Res<crate::world::map::MapRequest>,
    spatial: SpatialQuery,
    mut cams: Query<(&mut Transform, &mut FlyCamera, Option<&mut Walker>)>,
) {
    for (_, a) in script.0.iter().filter(|(f, _)| *f == frames.0) {
        if let Some(rest) = a.strip_prefix("spawn_pickup:") {
            let mut parts = rest.split(':');
            let class = parts.next().unwrap_or("");
            let dist: f32 = parts.next().and_then(|d| d.parse().ok()).unwrap_or(150.0);
            if net.role == PickupRole::Client {
                runlog::kv("scripted_pickup", &format!("action={a} refused=client"));
                continue;
            }
            if pickups.class(class).is_none() {
                let set = PackageSet::new(&request.install_root);
                let defaults = ClassDefaults::new(&set);
                ensure_class(&mut pickups.classes, &set, &defaults, class);
            }
            let Some(c) = pickups.class(class).cloned() else { continue };
            let Some((t, cam, walker)) = cams.iter().next() else { continue };
            let me = to_unreal(walker.map_or(t.translation - Vec3::Y * kf::EYE_HEIGHT * SCALE, |w| w.center));
            // The view's forward (Bevy), flattened, in Unreal axes.
            let f = Vec3::new(-cam.yaw.sin(), 0.0, -cam.yaw.cos());
            let dir = Vec3::new(-f.z, f.x, 0.0).normalize_or_zero();
            let at = rest_on_floor(&spatial, me + dir * dist, c.height);
            let id = pickups.spawn_dynamic(&c.path, at, [0, 0, 0], c.gives.clone(), Some(60.0), time.elapsed_secs_f64());
            runlog::kv("scripted_pickup", &format!("action={a} id={id}"));
            continue;
        }
        let (warp, rest) = if let Some(r) = a.strip_prefix("warp_pickup:") {
            (true, r)
        } else if let Some(r) = a.strip_prefix("look_pickup:") {
            (false, r)
        } else {
            continue;
        };
        let mut parts = rest.split(':');
        let kind = parts.next().unwrap_or("any");
        let dist: f32 = parts.next().and_then(|d| d.parse().ok()).unwrap_or(0.0);
        let found = if kind == "taken" {
            pickups.last_taken.as_ref()
        } else if kind == "dynamic" { pickups.shown.values().rev().find(|s| s.id >= DYNAMIC_ID_BASE) } else { pickups.shown.values().find(|s| kind == "any" || s.gives.kind() == kind) };
        let Some(s) = found else {
            runlog::kv("scripted_pickup", &format!("action={a} found=false shown={}", pickups.shown.len()));
            continue;
        };
        let at = Vec3::from_array(s.location);
        for (mut t, mut cam, walker) in &mut cams {
            let me = to_unreal(walker.as_ref().map_or(t.translation - Vec3::Y * kf::EYE_HEIGHT * SCALE, |w| w.center));
            let centre = if warp {
                // Standing on the floor where the pickup rests.
                let c = pickups.class(&s.class).map_or(5.0, |c| c.height);
                let floor = at.z - c;
                let mut away = (me - at).truncate().normalize_or_zero();
                if away == Vec2::ZERO {
                    away = Vec2::X;
                }
                let xy = at.truncate() + away * dist;
                // The floor where the player will stand (it may be a kerb
                // higher or lower than the pickup's).
                let top = Vec3::new(xy.x, xy.y, floor + 120.0);
                // 25 units up: it drops onto the floor (starting inside a
                // kerb's edge made the player fall through the world).
                let stand = rest_on_floor(&spatial, top, kf::HALF_HEIGHT + 25.0);
                if stand == top { Vec3::new(xy.x, xy.y, floor + kf::HALF_HEIGHT + 1.0) } else { stand }
            } else {
                me
            };
            let eye = centre + Vec3::Z * kf::EYE_HEIGHT;
            let d = coords::pos(at.to_array()) - coords::pos(eye.to_array());
            let d = d.normalize_or_zero();
            if dist > 0.0 || !warp {
                cam.yaw = (-d.x).atan2(-d.z);
                cam.pitch = d.y.clamp(-1.0, 1.0).asin();
            }
            if warp {
                t.translation = coords::pos(eye.to_array());
                if let Some(mut w) = walker {
                    w.center = coords::pos(centre.to_array());
                    w.velocity = Vec3::ZERO;
                }
            }
            t.rotation = Quat::from_euler(EulerRot::YXZ, cam.yaw, cam.pitch, 0.0);
            runlog::kv("scripted_pickup", &format!("action={a} id={} class={} at=({:.0}, {:.0}, {:.0}) player=({:.0}, {:.0}, {:.0})", s.id, s.class, at.x, at.y, at.z, centre.x, centre.y, centre.z));
        }
    }
}

// ------------------------------------------------------------ drawing

/// A static mesh's drawn parts.
type MeshParts = Vec<(Handle<Mesh>, Handle<StandardMaterial>)>;

/// Meshes per pickup class (non-send: the package set holds `Rc`s).
#[derive(Default)]
pub struct PickupModels {
    set: Option<PackageSet>,
    /// By lowercase mesh path: the parts, or None if it could not load.
    cache: HashMap<String, Option<MeshParts>>,
    /// Drawn pickups by id.
    drawn: HashMap<u32, Drawn>,
}

/// A drawn pickup: its root entity, class, CullDistance, and the scale
/// and rotation it was drawn with (dropped items move, spin and shrink).
struct Drawn {
    entity: Entity,
    class: String,
    cull: f32,
    scale: Vec3,
    rotation: Rotator,
}

fn load_mesh(set: &PackageSet, path: &str, meshes: &mut Assets<Mesh>, images: &mut Assets<Image>, materials: &mut Assets<StandardMaterial>) -> Option<MeshParts> {
    let h = set.find_object(path, Some("StaticMesh"))?;
    let sm = read_static_mesh(&h.package.pkg, h.export).ok()?;
    let mut parts = Vec::new();
    for (si, section) in sm.sections.iter().enumerate() {
        let tris = &sm.indices[section.first_index..section.first_index + section.num_triangles * 3];
        if tris.is_empty() {
            continue;
        }
        let positions: Vec<[f32; 3]> = sm.positions.iter().map(|p| coords::pos(*p).to_array()).collect();
        let normals: Vec<[f32; 3]> = sm.normals.iter().map(|n| coords::dir(*n).normalize_or_zero().to_array()).collect();
        let uvs: Vec<[f32; 2]> = sm.uvs.first().cloned().unwrap_or_else(|| vec![[0.0, 0.0]; sm.positions.len()]);
        let mesh = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
            .with_inserted_indices(bevy::mesh::Indices::U32(tris.iter().map(|&i| i as u32).collect()));
        let rf = sm.materials.get(si).copied().unwrap_or(ObjectRef::Null);
        let simple = ue_assets::material::resolve(set, &ObjectHandle { package: h.package.clone(), export: 0 }, rf);
        let image = simple.texture.as_ref().and_then(|t| crate::render::skinned::decode_image(t, images));
        // Lit by the map through vertex colours (render/actor_light.rs).
        let material = materials.add(StandardMaterial { base_color_texture: image, unlit: true, cull_mode: None, double_sided: true, ..default() });
        parts.push((meshes.add(mesh), material));
    }
    runlog::kv(
        "pickup_model",
        &format!(
            "mesh={path} parts={} bounds_unreal=({:.1}, {:.1}, {:.1})..({:.1}, {:.1}, {:.1})",
            parts.len(),
            sm.bounds.min[0],
            sm.bounds.min[1],
            sm.bounds.min[2],
            sm.bounds.max[0],
            sm.bounds.max[1],
            sm.bounds.max[2]
        ),
    );
    (!parts.is_empty()).then_some(parts)
}

/// Draws what is shown and removes what is not; hides pickups beyond
/// their CullDistance.
#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn sync_visuals(
    time: Res<Time>,
    mut commands: Commands,
    mut pickups: ResMut<Pickups>,
    mut models: NonSendMut<PickupModels>,
    request: Res<crate::world::map::MapRequest>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    camera: Query<&Transform, With<FlyCamera>>,
    mut visibility: Query<&mut Visibility>,
    mut transforms: Query<&mut Transform, Without<FlyCamera>>,
) {
    let models = &mut *models;
    // Gone or changed.
    let gone: Vec<u32> = models.drawn.iter().filter(|(id, d)| pickups.shown.get(id).is_none_or(|s| !s.class.eq_ignore_ascii_case(&d.class))).map(|(id, _)| *id).collect();
    for id in gone {
        if let Some(d) = models.drawn.remove(&id) {
            commands.entity(d.entity).despawn();
        }
    }
    let new: Vec<ShownPickup> = pickups.shown.values().filter(|s| !models.drawn.contains_key(&s.id)).cloned().collect();
    for s in new {
        if models.set.is_none() {
            models.set = Some(PackageSet::new(&request.install_root));
        }
        let set = models.set.as_ref().expect("just set");
        if pickups.class(&s.class).is_none() {
            let defaults = ClassDefaults::new(set);
            ensure_class(&mut pickups.classes, set, &defaults, &s.class);
        }
        let Some(c) = pickups.class(&s.class).cloned() else {
            models.drawn.insert(s.id, Drawn { entity: commands.spawn((Transform::default(), crate::world::map_change::MapScoped)).id(), class: s.class.clone(), cull: 0.0, scale: Vec3::ONE, rotation: Rotator::default() });
            continue;
        };
        let spot = (s.id < DYNAMIC_ID_BASE).then(|| pickups.spots.get(s.id as usize)).flatten();
        // An ammo box's own mesh and scale (the Christmas maps).
        let mesh_path = spot.and_then(|sp| sp.mesh.clone()).or(c.mesh.clone());
        let draw_scale = spot.and_then(|sp| sp.draw_scale).unwrap_or(c.draw_scale);
        let scale3d = spot.and_then(|sp| sp.draw_scale_3d).unwrap_or(c.draw_scale_3d);
        let cull = spot.and_then(|sp| sp.cull_distance).unwrap_or(c.cull_distance);
        let parts = mesh_path.as_ref().and_then(|m| models.cache.entry(m.to_ascii_lowercase()).or_insert_with(|| load_mesh(set, m, &mut meshes, &mut images, &mut materials)).clone());
        let rot = Rotator { pitch: s.rotation[0], yaw: s.rotation[1], roll: s.rotation[2] };
        let scale = coords::scale(scale3d) * draw_scale;
        let at = pickups.position(&s, time.elapsed_secs_f64());
        // UE2: Location + Rotation x (DrawScale x DrawScale3D x (v - PrePivot)).
        let root = commands
            .spawn((
                Name::new(format!("Pickup {} {}", s.id, s.class)),
                Transform { translation: coords::pos(at.to_array()), rotation: coords::rotation(rot), scale },
                Visibility::Inherited,
                // Lit by the map (render/actor_light.rs), with the class's
                // AmbientGlow (KFWeaponPickup 40) and MaxLights.
                crate::render::actor_light::ActorLight::new(format!("pickup_{}", s.id), Vec3::ZERO, c.max_lights.max(1) as usize, c.ambient_glow),
                crate::world::map_change::MapScoped,
            ))
            .id();
        let offset = coords::pos([-c.pre_pivot[0], -c.pre_pivot[1], -c.pre_pivot[2]]);
        // Dropped items move and spin: their light is redone every frame.
        let moving = s.id >= DYNAMIC_ID_BASE;
        for (mesh, material) in parts.iter().flatten() {
            // Each pickup its own copy (its own vertex colours).
            let own = meshes.get(mesh).cloned().map(|m| meshes.add(m)).unwrap_or_else(|| mesh.clone());
            commands.spawn((
                Mesh3d(own),
                MeshMaterial3d(material.clone()),
                Transform::from_translation(offset),
                ChildOf(root),
                crate::render::actor_light::LitPart { owner: root, animated: moving, own: None },
            ));
        }
        runlog::kv(
            "pickup_drawn",
            &format!("id={} class={} mesh={} parts={} draw_scale={draw_scale} at=({:.0}, {:.0}, {:.0})", s.id, s.class, mesh_path.as_deref().unwrap_or("none"), parts.as_ref().map_or(0, |p| p.len()), s.location[0], s.location[1], s.location[2]),
        );
        models.drawn.insert(s.id, Drawn { entity: root, class: s.class.clone(), cull, scale, rotation: rot });
    }
    // Dropped items: along the arc; FadeOut spins (RotationRate.Yaw 60000)
    // and shrinks them (DrawScale - default DrawScale x seconds, at least
    // 0.01 of it).
    let now = time.elapsed_secs_f64();
    for (id, d) in &models.drawn {
        if *id < DYNAMIC_ID_BASE {
            continue;
        }
        let Some(s) = pickups.shown.get(id) else { continue };
        let Ok(mut t) = transforms.get_mut(d.entity) else { continue };
        t.translation = coords::pos(pickups.position(s, now).to_array());
        if let Some(t0) = pickups.fade_clock.get(id) {
            let secs = (now - t0) as f32;
            let k = (1.0 - secs / drop::FADE_SECONDS as f32).max(0.01);
            t.scale = d.scale * k;
            let yaw = d.rotation.yaw + (drop::FADE_YAW_RATE * secs) as i32;
            t.rotation = coords::rotation(Rotator { yaw, ..d.rotation });
        }
    }
    // CullDistance.
    let Ok(cam) = camera.single() else { return };
    let eye = to_unreal(cam.translation);
    for (id, d) in &models.drawn {
        let Some(s) = pickups.shown.get(id) else { continue };
        let want = if d.cull > 0.0 && eye.distance(Vec3::from_array(s.location)) > d.cull { Visibility::Hidden } else { Visibility::Inherited };
        if let Ok(mut v) = visibility.get_mut(d.entity)
            && *v != want
        {
            *v = want;
        }
    }
}
