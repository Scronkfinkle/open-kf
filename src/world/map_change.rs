//! The map's life cycle: unloading the current map and loading another
//! inside the running game (DESIGN.md, "Map rotation and map voting",
//! step 1).
//!
//! Public API for the other steps (map list, network map change, voting):
//! - write a [`ChangeMap`] message to ask for a map (the change happens at
//!   the start of the next frame, in `First`);
//! - read [`MapLoaded`] to learn that a map has finished loading (sent for
//!   the first map at startup too, with `load == 1`);
//! - put [`MapScoped`] on every entity that belongs to the map: it is
//!   despawned (with its children) when the map is unloaded;
//! - add systems to the [`MapUnload`], [`MapLoad`] and [`PostMapLoad`]
//!   schedules (or use [`MapResourceExt`]) for per-map state;
//! - per-map "done once" flags in Update systems: [`OncePerMap`] with
//!   [`MapEpoch`].
//!
//! One code path: startup sends the first `ChangeMap` (the map given with
//! `--map`) and the same exclusive system loads it. The order inside a
//! change is the order the startup had before this module existed:
//!
//! 1. `MapUnload` (only when a map is loaded): each plugin's per-map
//!    resources are reset or removed; then every `MapScoped` entity is
//!    despawned and freed assets are dropped at once.
//! 2. `MapLoad` (was `Startup`): `load_map` reads the map and fills the
//!    setup resources; map sounds and the music trigger.
//! 3. `PostMapLoad` (was `PostStartup`): colliders, doors, glass, actor
//!    lights, placed decals and emitters, the camera placed at the start.
//!
//! Commands queued in a schedule are applied at its end, so `PostMapLoad`
//! sees everything `MapLoad` inserted, as `PostStartup` saw `Startup`'s.

use std::collections::HashSet;
use std::time::Instant;

use bevy::diagnostic::FrameCount;
use bevy::ecs::message::MessageCursor;
use bevy::ecs::resource::IsResource;
use bevy::ecs::schedule::ScheduleLabel;
use bevy::ecs::system::SystemState;
use bevy::prelude::*;

use crate::engine::runlog;
use crate::world::map::MapRequest;

/// Ask for a map change: `map` is a map name as in `--map` (e.g.
/// `KF-Farm`; a trailing `.rom` is dropped). The newest request in a frame
/// wins. A map file that does not exist is refused (logged
/// `map_change_refused`) and the current map stays.
#[derive(Message, Clone, Debug)]
pub struct ChangeMap {
    pub map: String,
}

/// Sent after a map has finished loading (after `PostMapLoad`).
#[derive(Message, Clone, Debug)]
pub struct MapLoaded {
    pub map: String,
    /// 1 for the first map of the run, then 2, 3...
    pub load: u32,
    /// Seconds the unload and load took together.
    pub seconds: f32,
}

/// On every entity that belongs to the current map (geometry, colliders,
/// doors, glass, decals, emitters, map sounds, traders, pickups, zeds,
/// gibs, projectiles...). Despawned, with its children, when the map is
/// unloaded. Children of a tagged entity need no tag of their own.
#[derive(Component, Default, Clone, Copy, Debug)]
pub struct MapScoped;

/// Runs first in a map change, while the old map is still there: reset or
/// remove per-map resources (the tagged entities are despawned after it).
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct MapUnload;

/// Loads the map (what `Startup` did for it before).
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct MapLoad;

/// Builds on what `MapLoad` inserted (what `PostStartup` did for it before).
#[derive(ScheduleLabel, Clone, Debug, PartialEq, Eq, Hash)]
pub struct PostMapLoad;

/// Which map load the world is on, and when it happened.
#[derive(Resource, Default, Debug, Clone)]
pub struct MapEpoch {
    /// 0 before the first map, 1 for the first map, then 2, 3...
    pub load: u32,
    /// The frame the current map was loaded in (0 for the first map).
    pub frame: u32,
}

impl MapEpoch {
    /// Is this the first map of the run (startup)?
    pub fn first(&self) -> bool {
        self.load <= 1
    }

    /// Frames since the current map was loaded.
    pub fn frames_since(&self, frames: &FrameCount) -> u32 {
        frames.0.saturating_sub(self.frame)
    }
}

/// A "done once per map" flag for Update systems (`Local<OncePerMap>`):
/// `done()` is false again after each map change.
#[derive(Default, Debug)]
pub struct OncePerMap(Option<u32>);

impl OncePerMap {
    pub fn done(&self, epoch: &MapEpoch) -> bool {
        self.0 == Some(epoch.load)
    }

    pub fn set(&mut self, epoch: &MapEpoch) {
        self.0 = Some(epoch.load);
    }
}

/// Registers per-map resources next to their `init_resource`.
pub trait MapResourceExt {
    /// Put the default value back at each unload.
    fn reset_on_map_unload<R: Resource + Default>(&mut self) -> &mut Self;
    /// Remove it at each unload (for resources the map load inserts).
    fn remove_on_map_unload<R: Resource>(&mut self) -> &mut Self;
}

impl MapResourceExt for App {
    fn reset_on_map_unload<R: Resource + Default>(&mut self) -> &mut Self {
        self.add_systems(MapUnload, |mut commands: Commands| commands.insert_resource(R::default()))
    }

    fn remove_on_map_unload<R: Resource>(&mut self) -> &mut Self {
        self.add_systems(MapUnload, |mut commands: Commands| commands.remove_resource::<R>())
    }
}

/// `--map-hop A,B,C --map-hop-every N` (testing): every N frames after a
/// load, ask for the next map of the list (round and round).
#[derive(Resource, Debug, Clone)]
pub struct MapHop {
    pub maps: Vec<String>,
    pub every: u32,
    next: usize,
}

impl MapHop {
    pub fn new(maps: Vec<String>, every: u32) -> Self {
        MapHop { maps, every: every.max(1), next: 0 }
    }
}

/// Where the shared `ChangeMap` reader stands (one cursor for the startup
/// and the per-frame runs of `apply_map_change`, so a request is used once).
#[derive(Resource, Default)]
struct ChangeCursor(MessageCursor<ChangeMap>);

pub struct MapChangePlugin;

impl Plugin for MapChangePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ChangeMap>()
            .add_message::<MapLoaded>()
            .init_resource::<MapEpoch>()
            .init_resource::<ChangeCursor>()
            .init_schedule(MapUnload)
            .init_schedule(MapLoad)
            .init_schedule(PostMapLoad)
            .add_systems(Startup, (request_first_map, apply_map_change).chain())
            .add_systems(First, apply_map_change)
            .add_systems(Update, (map_hop, log_settled_counts));
    }
}

/// Startup: ask for the map from the command line (`MapRequest`).
fn request_first_map(request: Res<MapRequest>, mut change: MessageWriter<ChangeMap>) {
    change.write(ChangeMap { map: request.map.clone() });
}

/// Counts for the logs: entities (resources not counted), tagged map
/// entities, and the asset collections the map fills.
fn counts(world: &mut World) -> String {
    fn assets<A: Asset>(world: &World) -> usize {
        world.get_resource::<Assets<A>>().map_or(0, |a| a.len())
    }
    let entities = world.query_filtered::<(), Without<IsResource>>().iter(world).count();
    let scoped = world.query_filtered::<(), With<MapScoped>>().iter(world).count();
    format!(
        "rss_mb={} entities={entities} map_scoped={scoped} meshes={} images={} materials={} baked_materials={} modulate_materials={} blend_materials={}",
        rss_mb().map_or("?".to_string(), |m| m.to_string()),
        assets::<Mesh>(world),
        assets::<Image>(world),
        assets::<StandardMaterial>(world),
        assets::<crate::render::baked::BakedMaterial>(world),
        assets::<crate::render::particles::ModulateMaterial>(world),
        assets::<crate::render::particles::BlendMaterial>(world),
    )
}

/// Hands the old map's freed memory back to the system. Linux's C
/// allocator keeps freed memory per thread "arena", and the next map is
/// built on other threads, so without this every map change added the old
/// map's size (KF-Farm: about 500 MB) and a run through all maps ran out
/// of memory (measured: `rss_mb` in the logs, see MODLOG). Other systems:
/// nothing to do.
fn release_freed_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        unsafe extern "C" {
            /// glibc: release free memory at the top of the heap and free
            /// pages inside every arena; returns 1 if memory was released.
            fn malloc_trim(pad: usize) -> i32;
        }
        // SAFETY: malloc_trim only looks at the allocator's own free lists;
        // no pointer is passed.
        unsafe {
            malloc_trim(0);
        }
    }
}

/// The game's resident memory in MB (Linux: /proc/self/status VmRSS;
/// elsewhere None), to see whether map changes leak.
fn rss_mb() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024)
}

/// The per-map resources' sizes, to see that each load starts clean
/// ("none": the resource is not there).
fn resource_counts(world: &World) -> String {
    let n = |v: Option<usize>| v.map_or("none".to_string(), |v| v.to_string());
    let doors = world.get_resource::<crate::world::door::Doors>();
    format!(
        "doors={} trader_doors={} triggers={} glass={} nav_points={} pickup_spots={} shops={} player_starts={} actor_lights={} map_emitters={} effects={} zones={} wave_data_waves={}",
        n(doors.map(|d| d.doors.len())),
        n(doors.map(|d| d.trader.len())),
        n(doors.map(|d| d.triggers.len())),
        n(world.get_resource::<crate::world::glass::Glass>().map(|g| g.panes.len())),
        n(world.get_resource::<crate::world::nav::NavNetwork>().map(|g| g.points.len())),
        n(world.get_resource::<crate::game::pickups::Pickups>().map(|p| p.spots.len())),
        n(world.get_resource::<crate::game::trader::Shops>().map(|s| s.shops.len())),
        n(world.get_resource::<crate::world::map::PlayerStarts>().map(|s| s.0.len())),
        n(world.get_resource::<crate::render::actor_light::ActorLights>().map(|l| l.sources.len())),
        n(world.get_resource::<crate::render::particles::MapEmitters>().map(|e| e.0.len())),
        n(world.get_resource::<crate::render::particles::EffectLibrary>().map(|e| e.len())),
        n(world.get_resource::<crate::world::zones::Zones>().map(|z| z.zones.len())),
        n(world.get_resource::<crate::game::waves::GameData>().map(|g| g.waves.len())),
    )
}

/// Drops assets whose last handle went away now (Bevy does it in
/// PreUpdate), so the old map's are gone before the new map loads and the
/// counts are honest.
fn free_dropped_assets(world: &mut World) {
    fn track<A: Asset>(world: &mut World) {
        if world.contains_resource::<Assets<A>>() && world.contains_resource::<AssetServer>() {
            let _ = world.run_system_cached(Assets::<A>::track_assets);
        }
    }
    // Materials first: a freed material lets go of its textures.
    track::<crate::render::baked::BakedMaterial>(world);
    track::<crate::render::particles::ModulateMaterial>(world);
    track::<crate::render::particles::BlendMaterial>(world);
    track::<StandardMaterial>(world);
    track::<Image>(world);
    track::<Mesh>(world);
}

/// The newest `ChangeMap` not handled yet.
fn take_request(world: &mut World) -> Option<ChangeMap> {
    let mut cursor = world.remove_resource::<ChangeCursor>().unwrap_or_default();
    let last = cursor.0.read(world.resource::<Messages<ChangeMap>>()).last().cloned();
    world.insert_resource(cursor);
    last
}

/// The map change itself (Startup for the first map, then `First` each
/// frame when a `ChangeMap` came).
fn apply_map_change(world: &mut World) {
    let Some(request) = take_request(world) else { return };
    let started = Instant::now();
    let map = request.map.trim_end_matches(".rom").to_string();
    let (root, current) = {
        let r = world.resource::<MapRequest>();
        (r.install_root.clone(), r.map.clone())
    };
    let loaded_before = world.resource::<MapEpoch>().load > 0;
    if loaded_before && !root.join("Maps").join(format!("{map}.rom")).is_file() {
        runlog::kv("map_change_refused", &format!("map={map} reason=no_such_map current={current}"));
        return;
    }
    let frame = world.resource::<FrameCount>().0;
    runlog::kv("map_change", &format!("from={} to={map} frame={frame}", if loaded_before { current.as_str() } else { "none" }));
    if loaded_before {
        unload(world, &current);
    }
    world.resource_mut::<MapRequest>().map = map.clone();
    let load = {
        let mut epoch = world.resource_mut::<MapEpoch>();
        epoch.load += 1;
        epoch.frame = frame;
        epoch.load
    };
    let before: HashSet<Entity> = world.query_filtered::<Entity, Without<IsResource>>().iter(world).collect();
    world.run_schedule(MapLoad);
    world.run_schedule(PostMapLoad);
    tag_untagged(world, &before);
    let seconds = started.elapsed().as_secs_f32();
    let c = counts(world);
    let r = resource_counts(world);
    runlog::kv("map_lifecycle_loaded", &format!("map={map} load={load} seconds={seconds:.2} {c} {r}"));
    world.write_message(MapLoaded { map, load, seconds });
}

/// Unload: the per-map resources first (`MapUnload`), then the tagged
/// entities, then the assets nothing uses any more.
fn unload(world: &mut World, map: &str) {
    let started = Instant::now();
    world.run_schedule(MapUnload);
    let tagged: Vec<Entity> = world.query_filtered::<Entity, With<MapScoped>>().iter(world).collect();
    let mut despawned = 0usize;
    for e in &tagged {
        // A tagged child may already be gone with its tagged parent.
        if let Ok(entity) = world.get_entity_mut(*e) {
            entity.despawn();
            despawned += 1;
        }
    }
    world.flush();
    free_dropped_assets(world);
    release_freed_memory();
    let c = counts(world);
    runlog::kv("map_unload", &format!("map={map} despawned={despawned} tagged={} seconds={:.2} {c}", tagged.len(), started.elapsed().as_secs_f32()));
}

/// Entities the load spawned without `MapScoped` (and not below a tagged
/// one): tagged now so they cannot outlive the map, and logged so the
/// spawn site can be fixed. The player's camera is not the map's.
fn tag_untagged(world: &mut World, before: &HashSet<Entity>) {
    let mut q = world.query_filtered::<(Entity, Option<&ChildOf>, Option<&Name>), (Without<IsResource>, Without<MapScoped>, Without<crate::engine::camera::FlyCamera>)>();
    let candidates: Vec<(Entity, Option<Entity>, String)> =
        q.iter(world).filter(|(e, _, _)| !before.contains(e)).map(|(e, p, n)| (e, p.map(|p| p.parent()), n.map_or(String::new(), |n| n.to_string()))).collect();
    let mut tagged = Vec::new();
    for (e, parent, name) in candidates {
        // Below a tagged entity (or below one of these): goes with it.
        let mut up = parent;
        let mut covered = false;
        while let Some(p) = up {
            if world.get::<MapScoped>(p).is_some() {
                covered = true;
                break;
            }
            up = world.get::<ChildOf>(p).map(|c| c.parent());
        }
        if covered {
            continue;
        }
        world.entity_mut(e).insert(MapScoped);
        tagged.push(if name.is_empty() { format!("{e:?}") } else { name });
    }
    if !tagged.is_empty() {
        tagged.sort();
        runlog::kv("map_load_untagged", &format!("count={} first=[{}]", tagged.len(), tagged.iter().take(12).cloned().collect::<Vec<_>>().join(" | ")));
    }
}

/// `--map-hop`: the next map every N frames after a load.
fn map_hop(hop: Option<ResMut<MapHop>>, epoch: Res<MapEpoch>, frames: Res<FrameCount>, mut change: MessageWriter<ChangeMap>, mut asked: Local<u32>) {
    let Some(mut hop) = hop else { return };
    if hop.maps.is_empty() || epoch.load == 0 || *asked == epoch.load || epoch.frames_since(&frames) < hop.every {
        return;
    }
    *asked = epoch.load;
    let map = hop.maps[hop.next % hop.maps.len()].clone();
    hop.next += 1;
    runlog::kv("map_hop", &format!("to={map} after_frames={} hop={}", hop.every, hop.next));
    change.write(ChangeMap { map });
}

/// 60 frames after each `MapLoaded` (Bevy has dropped freed assets and
/// spawned the lazily drawn things by then): the counts again, to compare
/// loads of the same map.
fn log_settled_counts(world: &mut World, loaded: &mut SystemState<MessageReader<MapLoaded>>, mut waiting: Local<Option<(MapLoaded, u32)>>) {
    let frame = world.resource::<FrameCount>().0;
    if let Ok(mut reader) = loaded.get_mut(world)
        && let Some(m) = reader.read().last()
    {
        *waiting = Some((m.clone(), frame));
    }
    let Some((m, at)) = waiting.as_ref() else { return };
    if frame < at + 60 {
        return;
    }
    let (map, load, seconds) = (m.map.clone(), m.load, m.seconds);
    *waiting = None;
    let c = counts(world);
    runlog::kv("map_settled", &format!("map={map} load={load} load_seconds={seconds:.2} frame={frame} {c}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct PerMap(u32);

    #[derive(Resource, Default)]
    struct Loads(Vec<String>);

    /// An install folder with two empty "maps" (only their names matter),
    /// under target/ (gitignored).
    fn install() -> std::path::PathBuf {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target").join("test-map-change");
        std::fs::create_dir_all(root.join("Maps")).unwrap();
        for m in ["KF-A", "KF-B"] {
            std::fs::write(root.join("Maps").join(format!("{m}.rom")), b"").unwrap();
        }
        root
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(MapChangePlugin)
            .insert_resource(MapRequest { install_root: install(), map: "KF-A".into() })
            .init_resource::<PerMap>()
            .reset_on_map_unload::<PerMap>()
            .init_resource::<Loads>()
            .add_systems(MapLoad, |mut commands: Commands, request: Res<MapRequest>, mut per: ResMut<PerMap>, mut loads: ResMut<Loads>| {
                let parent = commands.spawn(MapScoped).id();
                commands.spawn(ChildOf(parent));
                // Forgot its tag: tagged by the safety net.
                commands.spawn(Name::new("untagged"));
                per.0 += 1;
                loads.0.push(request.map.clone());
            });
        app
    }

    fn scoped(app: &mut App) -> usize {
        app.world_mut().query_filtered::<(), With<MapScoped>>().iter(app.world()).count()
    }

    fn loaded(app: &mut App) -> Vec<(String, u32)> {
        let mut cursor = MessageCursor::<MapLoaded>::default();
        cursor.read(app.world().resource::<Messages<MapLoaded>>()).map(|m| (m.map.clone(), m.load)).collect()
    }

    #[test]
    fn startup_loads_the_first_map_then_change_map_swaps_it() {
        let mut app = app();
        app.update();
        assert_eq!(app.world().resource::<Loads>().0, vec!["KF-A"]);
        assert_eq!(app.world().resource::<MapEpoch>().load, 1);
        assert_eq!(scoped(&mut app), 2, "the tagged parent and the untagged one");
        assert_eq!(loaded(&mut app), vec![("KF-A".to_string(), 1)]);

        app.world_mut().write_message(ChangeMap { map: "KF-B.rom".into() });
        app.update();
        let w = app.world();
        assert_eq!(w.resource::<Loads>().0, vec!["KF-A", "KF-B"]);
        assert_eq!(w.resource::<MapRequest>().map, "KF-B");
        assert_eq!(w.resource::<MapEpoch>().load, 2);
        assert_eq!(w.resource::<PerMap>().0, 1, "reset at the unload, then loaded once");
        // The old map's three entities are gone, the new map's are there.
        let all = app.world_mut().query_filtered::<(), Without<IsResource>>().iter(app.world()).count();
        assert_eq!(scoped(&mut app), 2);
        assert_eq!(all, 3);
        assert!(loaded(&mut app).contains(&("KF-B".to_string(), 2)));

        // Nothing asked: nothing happens; a map that is not installed is refused.
        app.update();
        app.world_mut().write_message(ChangeMap { map: "KF-Nowhere".into() });
        app.update();
        assert_eq!(app.world().resource::<Loads>().0, vec!["KF-A", "KF-B"]);
        assert_eq!(app.world().resource::<MapRequest>().map, "KF-B");
    }

    #[test]
    fn once_per_map_comes_back_after_a_change() {
        let mut epoch = MapEpoch { load: 1, frame: 0 };
        let mut once = OncePerMap::default();
        assert!(!once.done(&epoch));
        once.set(&epoch);
        assert!(once.done(&epoch));
        epoch.load = 2;
        epoch.frame = 500;
        assert!(!once.done(&epoch));
        assert_eq!(epoch.frames_since(&FrameCount(505)), 5);
        assert!(!epoch.first());
    }
}
