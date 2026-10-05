//! ZombieVolumes: where wave zeds come from. KFMod.ZombieVolume
//! (InitSpawnPoints, RateZombieVolume, CanSpawnInHere, SpawnInHere,
//! PlayerCanSeePoint, Touch) and KFGameType.FindSpawningVolume. Positions
//! are Unreal units; spatial queries convert to Bevy space.

use std::collections::HashMap;
use std::rc::Rc;

use avian3d::prelude::*;
use bevy::prelude::*;
use ue_assets::bsp::{read_model, read_polys};
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package::ObjectRef;
use ue_assets::package_set::{LoadedPackage, ObjectHandle, PackageSet};
use ue_assets::properties::{PropertyList, Value, read_export_properties, struct_array};

use crate::coords::{self, SCALE};
use crate::runlog;

/// VolumeColTester: CollisionRadius 26, CollisionHeight 44.
const TESTER_RADIUS: f32 = 26.0;
const TESTER_HALF_HEIGHT: f32 = 44.0;
/// KFPawn BaseEyeHeight above the cylinder centre.
const PLAYER_EYE: f32 = 44.0;

/// What a volume knows about a zed class: KFMonster.ZombieFlag (0 normal,
/// 1 ranged, 2 leaping, 3 massive), its collision cylinder, and which of
/// the volume lists' class names it is (ClassIsChildOf).
#[derive(Clone, Debug, Default)]
pub struct ZedInfo {
    pub flag: u8,
    pub radius: f32,
    pub half_height: f32,
    pub is_a: Vec<String>,
}

/// One RoomDoorsList entry (KFDoorSelType): the door's object name.
#[derive(Clone, Debug)]
pub struct RoomDoor {
    pub door: String,
    pub only_when_welded: bool,
}

#[derive(Clone, Debug)]
pub struct ZombieVolume {
    pub name: String,
    pub location: Vec3,
    /// The brush's polygons in world space (for Encompasses).
    pub polys: Vec<Vec<Vec3>>,
    pub spawn_desirability: f32,
    pub min_distance_to_player: f32,
    pub no_z_axis_penalty: bool,
    pub allow_plain_sight: bool,
    /// bNormalZeds, bRangedZeds, bLeapingZeds, bMassiveZeds by ZombieFlag.
    pub allowed_flags: [bool; 4],
    pub disallowed: Vec<String>,
    pub only_allowed: Vec<String>,
    pub zombie_count_multi: f32,
    pub touch_disable_time: f32,
    pub enabled: bool,
    pub room_doors: Vec<RoomDoor>,
    // Run-time state.
    pub spawn_pos: Vec<Vec3>,
    pub last_spawn_time: f32,
    pub last_failed_spawn_time: f32,
    pub last_check_time: f32,
    player_inside: bool,
}

/// Point inside a closed brush: odd number of crossings along a ray (the
/// direction is skewed so it does not run along edges).
pub fn encompasses(polys: &[Vec<Vec3>], p: Vec3) -> bool {
    let dir = Vec3::new(0.5773, 0.6131, 0.5393);
    let mut crossings = 0;
    for poly in polys {
        for k in 1..poly.len().saturating_sub(1) {
            let (a, b, c) = (poly[0], poly[k], poly[k + 1]);
            // Moller-Trumbore.
            let (e1, e2) = (b - a, c - a);
            let h = dir.cross(e2);
            let det = e1.dot(h);
            if det.abs() < 1e-6 {
                continue;
            }
            let s = p - a;
            let u = s.dot(h) / det;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let q = s.cross(e1);
            let v = dir.dot(q) / det;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            if e2.dot(q) / det > 0.0 {
                crossings += 1;
            }
        }
    }
    crossings % 2 == 1
}

fn class_handle(set: &PackageSet, path: &str) -> Option<ObjectHandle> {
    let (pkg, name) = path.split_once('.')?;
    let lp = set.load(pkg)?;
    (0..lp.pkg.exports.len())
        .find(|&i| lp.pkg.export_class_name(i) == "Class" && lp.pkg.object_name(ObjectRef::Export(i)).eq_ignore_ascii_case(name))
        .map(|export| ObjectHandle { package: lp.clone(), export })
}

/// A brush actor's polygons in world space (Unreal units): subtract the
/// pivot, rotate, add Location (as for blocking volumes in map.rs).
pub fn brush_polys(pkg: &ue_assets::package::Package, props: &PropertyList) -> Vec<Vec<Vec3>> {
    let vector = |n: &str| match props.get(pkg, n) {
        Some(Value::Vector(v)) => Vec3::from_array(*v),
        _ => Vec3::ZERO,
    };
    let (location, pre_pivot) = (vector("Location"), vector("PrePivot"));
    let rotation = match props.get(pkg, "Rotation") {
        Some(Value::Rotator(r)) => *r,
        _ => Default::default(),
    };
    let rot = coords::ue_rotation_matrix(rotation);
    match props.get(pkg, "Brush") {
        Some(Value::Object(ObjectRef::Export(m))) => read_model(pkg, *m)
            .ok()
            .and_then(|model| match model.polys {
                ObjectRef::Export(p) => read_polys(pkg, p).ok(),
                _ => None,
            })
            .unwrap_or_default()
            .iter()
            .map(|poly| poly.vertices.iter().map(|v| rot * (Vec3::from_array(*v) - pre_pivot) + location).collect())
            .collect(),
        _ => Vec::new(),
    }
}

/// Reads the map's ZombieVolumes (not bObjectiveModeOnly) and what the
/// volumes need to know about `zed_classes`.
pub fn load(set: &PackageSet, defaults: &ClassDefaults, map: &Rc<LoadedPackage>, zed_classes: &[String]) -> (Vec<ZombieVolume>, HashMap<String, ZedInfo>) {
    let pkg = &map.pkg;
    let mut volumes = Vec::new();
    for i in 0..pkg.exports.len() {
        if pkg.export_class_name(i) != "ZombieVolume" {
            continue;
        }
        let Ok(props) = read_export_properties(pkg, i) else { continue };
        let value = |n: &str| defaults.actor_value(map, i, &props, n);
        let float = |n: &str, d: f32| match value(n) {
            Some(Value::Float(f)) => f,
            _ => d,
        };
        let flag = |n: &str| matches!(value(n), Some(Value::Bool(true)));
        if flag("bObjectiveModeOnly") {
            continue;
        }
        let location = match props.get(pkg, "Location") {
            Some(Value::Vector(v)) => Vec3::from_array(*v),
            _ => Vec3::ZERO,
        };
        let polys = brush_polys(pkg, &props);
        let class_names = |n: &str| -> Vec<String> {
            match props.get(pkg, n) {
                Some(Value::Array { count, raw }) => {
                    let mut r = ue_assets::reader::Reader::new(raw);
                    (0..*count)
                        .map_while(|_| r.compact_index().ok().map(ObjectRef::from_raw))
                        .map(|o| pkg.object_name(o).to_string())
                        .collect()
                }
                _ => Vec::new(),
            }
        };
        let room_doors = props
            .get(pkg, "RoomDoorsList")
            .map(|v| {
                struct_array(pkg, v)
                    .iter()
                    .filter_map(|e| {
                        let door = match e.get(pkg, "DoorActor") {
                            Some(Value::Object(o)) if *o != ObjectRef::Null => pkg.object_name(*o).to_string(),
                            _ => return None,
                        };
                        Some(RoomDoor {
                            door,
                            only_when_welded: matches!(e.get(pkg, "bOnlyWhenWelded"), Some(Value::Bool(true))),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        volumes.push(ZombieVolume {
            name: pkg.object_name(ObjectRef::Export(i)).to_string(),
            location,
            polys,
            spawn_desirability: float("SpawnDesirability", 3000.0),
            min_distance_to_player: float("MinDistanceToPlayer", 600.0),
            no_z_axis_penalty: flag("bNoZAxisDistPenalty"),
            allow_plain_sight: flag("bAllowPlainSightSpawns"),
            allowed_flags: [flag("bNormalZeds"), flag("bRangedZeds"), flag("bLeapingZeds"), flag("bMassiveZeds")],
            disallowed: class_names("DisallowedZeds"),
            only_allowed: class_names("OnlyAllowedZeds"),
            zombie_count_multi: float("ZombieCountMulti", 1.0),
            touch_disable_time: float("TouchDisableTime", 10.0),
            enabled: flag("bVolumeIsEnabled"),
            room_doors,
            spawn_pos: Vec::new(),
            last_spawn_time: f32::MIN,
            last_failed_spawn_time: f32::MIN,
            last_check_time: f32::MIN,
            player_inside: false,
        });
    }
    // Zed classes: ZombieFlag, size, and which listed classes they are.
    let mut listed: Vec<String> = volumes.iter().flat_map(|v| v.disallowed.iter().chain(v.only_allowed.iter()).cloned()).collect();
    listed.sort();
    listed.dedup();
    let mut zeds = HashMap::new();
    for path in zed_classes {
        let Some(h) = class_handle(set, path) else {
            runlog::kv("zvolume_warning", &format!("reason=class_not_found class={path}"));
            continue;
        };
        let f = |n: &str, d: f32| match defaults.get(&h, n) {
            Some((Value::Float(x), _)) => x,
            _ => d,
        };
        zeds.insert(
            path.to_ascii_lowercase(),
            ZedInfo {
                flag: match defaults.get(&h, "ZombieFlag") {
                    Some((Value::Byte(b), _)) => b,
                    _ => 0,
                },
                radius: f("CollisionRadius", 26.0),
                half_height: f("CollisionHeight", 44.0),
                is_a: listed.iter().filter(|n| defaults.is_a(&h, n)).cloned().collect(),
            },
        );
    }
    (volumes, zeds)
}

impl ZombieVolume {
    /// SpawnInHere's type filter: ZombieFlag against b*Zeds, then
    /// DisallowedZeds, then OnlyAllowedZeds.
    pub fn allows(&self, zeds: &HashMap<String, ZedInfo>, class: &str) -> bool {
        let Some(z) = zeds.get(&class.to_ascii_lowercase()) else {
            return true;
        };
        if !self.allowed_flags.get(z.flag as usize).copied().unwrap_or(true) {
            return false;
        }
        let is = |list: &[String]| list.iter().any(|n| z.is_a.iter().any(|a| a.eq_ignore_ascii_case(n)));
        if is(&self.disallowed) {
            return false;
        }
        self.only_allowed.is_empty() || is(&self.only_allowed)
    }

    /// InitSpawnPoints: the volume's extent along +-X and +-Y from its
    /// pivot (100-unit steps while a 26 x 44 tester fits and is inside),
    /// then an 11 x 11 grid (steps of a tenth of the extent, at least 56)
    /// of points where the tester fits, is inside, and a 44-unit vertical
    /// trace through it is clear. SetLocation's native nudge to a free spot
    /// is not copied: a point that does not fit is dropped.
    pub fn init_spawn_points(&mut self, spatial: &SpatialQuery) {
        let tester = Collider::cylinder(TESTER_RADIUS * SCALE, 2.0 * TESTER_HALF_HEIGHT * SCALE);
        let filter = crate::collision::zed_path_filter();
        // Tst.SetLocation with bCollideWhenPlacing, then Encompasses(Tst).
        // The native placement moves the tester to a nearby free spot; we
        // only lift it (up to 44 units in 2-unit steps; assumed, not the
        // native search), which is what volumes whose pivot sits less than
        // 44 above the floor need. Returns where the tester ended up.
        let place = |p: Vec3| -> Option<Vec3> {
            let free = |q: Vec3| spatial.shape_intersections(&tester, coords::pos(q.to_array()), Quat::IDENTITY, &filter).is_empty();
            let at = (0..=22).map(|k| p + Vec3::Z * (2.0 * k as f32)).find(|&q| free(q))?;
            encompasses(&self.polys, at).then_some(at)
        };
        let fits = |p: Vec3| place(p).is_some();
        let extent = |axis: Vec3| {
            let mut best = 0.0;
            for i in 0..=10 {
                let d = 100.0 * i as f32;
                if !fits(self.location + axis * d) {
                    break;
                }
                best = d;
            }
            best
        };
        let (px, py, nx, ny) = (extent(Vec3::X), extent(Vec3::Y), extent(-Vec3::X), extent(-Vec3::Y));
        let step_x = ((nx + px) / 10.0).trunc().max(56.0);
        let step_y = ((ny + py) / 10.0).trunc().max(56.0);
        let world = crate::collision::world_filter();
        let mut points = Vec::new();
        for x in -5..=5 {
            for y in -5..=5 {
                let (ox, oy) = (step_x * x as f32, step_y * y as f32);
                if (ox > 0.0 && ox > px) || (ox < 0.0 && ox < -nx) || (oy > 0.0 && oy > py) || (oy < 0.0 && oy < -ny) {
                    continue;
                }
                let Some(p) = place(self.location + Vec3::new(ox, oy, 0.0)) else {
                    continue;
                };
                let (top, bottom) = (coords::pos((p + Vec3::Z * 22.0).to_array()), coords::pos((p - Vec3::Z * 22.0).to_array()));
                if spatial.cast_ray(top, Dir3::NEG_Y, (top - bottom).length(), true, &world).is_some() {
                    continue;
                }
                points.push(p);
            }
        }
        self.spawn_pos = points;
    }
}

/// FastTrace: nothing of the level between two points (Unreal units).
fn fast_trace(spatial: &SpatialQuery, a: Vec3, b: Vec3) -> bool {
    let (from, to) = (coords::pos(a.to_array()), coords::pos(b.to_array()));
    let Ok(dir) = Dir3::new(to - from) else { return true };
    spatial.cast_ray(from, dir, (to - from).length(), true, &crate::collision::world_filter()).is_none()
}

/// The player as the volume rules see them (Unreal units).
#[derive(Clone, Copy, Debug)]
pub struct PlayerView {
    pub location: Vec3,
    /// DistanceFogEnd of the player's zone (None: no fog).
    pub fog_end: Option<f32>,
}

impl PlayerView {
    fn eye(&self) -> Vec3 {
        self.location + Vec3::Z * PLAYER_EYE
    }
}

/// ZombieVolume.PlayerCanSeePoint: the player sees the point, and both
/// sides of the zed's top (1.25 x its height up, 1.1 x its radius out).
/// Beyond the DistanceFogEnd of the player's zone it is never seen.
pub fn player_can_see_point(spatial: &SpatialQuery, v: &ZombieVolume, p: Vec3, zed: &ZedInfo, player: &PlayerView) -> bool {
    if v.allow_plain_sight {
        return false;
    }
    // Beyond the player's zone fog nothing is seen.
    if player.fog_end.is_some_and(|e| (p - player.location).length() >= e) {
        return false;
    }
    let eye = player.eye();
    let right = (p - player.location).cross(Vec3::Z).normalize_or_zero() * zed.radius * 1.1;
    let up = Vec3::Z * zed.half_height * 1.25;
    fast_trace(spatial, p, eye) && fast_trace(spatial, p + up + right, eye) && fast_trace(spatial, p + up - right, eye)
}

/// RateZombieVolume for one player. `doors`: (name, sealed, at key 0) of
/// every door. -1 = refused (with the reason, for the log).
#[allow(clippy::too_many_arguments)]
pub fn rate(
    spatial: &SpatialQuery,
    v: &ZombieVolume,
    is_last: bool,
    ignore_failed: bool,
    boss: bool,
    squad: &[String],
    zeds: &HashMap<String, ZedInfo>,
    doors: &dyn Fn(&str) -> Option<(bool, bool)>,
    player: &PlayerView,
    now: f32,
    frand: f32,
) -> Result<f32, &'static str> {
    if !ignore_failed && now - v.last_failed_spawn_time < 5.0 {
        return Err("failed_recently");
    }
    for d in &v.room_doors {
        if let Some((sealed, closed_key0)) = doors(&d.door)
            && ((!d.only_when_welded && closed_key0) || sealed)
        {
            return Err("room_door");
        }
    }
    // CanSpawnInHere.
    if v.last_check_time >= now {
        return Err("touched");
    }
    if !v.enabled {
        return Err("disabled");
    }
    if v.spawn_pos.is_empty() {
        return Err("no_spawn_points");
    }
    if !squad.iter().any(|c| v.allows(zeds, c)) {
        return Err("zed_types");
    }
    let mut score = v.spawn_desirability;
    let usage = (now - v.last_spawn_time).min(30.0) / 30.0;
    if encompasses(&v.polys, player.location) {
        return Err("player_inside");
    }
    let dz = (player.location.z - v.location.z).abs();
    let dxy = (player.location - v.location).truncate().length();
    let score_z = (250.0 - dz).clamp(1.0, 250.0) / 250.0;
    let score_xy = (2000.0 - dxy).clamp(1.0, 2000.0) / 2000.0;
    let dist_score = if v.no_z_axis_penalty { score_xy } else { score_z * 0.3 + score_xy * 0.7 };
    let dist = (v.location - player.location).length();
    let in_fog_range = player.fog_end.is_none_or(|e| dist < e);
    if !v.allow_plain_sight && in_fog_range && fast_trace(spatial, v.location, player.eye()) {
        return Err("in_sight");
    }
    if dist < v.min_distance_to_player {
        return Err("too_close");
    }
    score = score * 0.3 + dist_score * score * 0.3 + usage * score * 0.3 + frand * score * 0.1;
    if boss && dist < 1000.0 {
        score *= 0.2;
    }
    if is_last {
        score *= 0.2;
    }
    Ok(score.max(1.0))
}

/// ZombieVolume.Touch by the player: the volume is off for
/// TouchDisableTime from when the player walks in.
pub fn touch(volumes: &mut [ZombieVolume], player: &PlayerView, now: f32) {
    for v in volumes {
        let inside = encompasses(&v.polys, player.location);
        if inside && !v.player_inside {
            v.last_check_time = now + v.touch_disable_time;
            runlog::kv("zvolume_touched", &format!("volume={} until={:.0}", v.name, v.last_check_time));
        }
        v.player_inside = inside;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 200-unit cube as six quads.
    fn cube() -> Vec<Vec<Vec3>> {
        let c = |x: f32, y: f32, z: f32| Vec3::new(x, y, z) * 100.0;
        vec![
            vec![c(-1., -1., -1.), c(1., -1., -1.), c(1., 1., -1.), c(-1., 1., -1.)],
            vec![c(-1., -1., 1.), c(-1., 1., 1.), c(1., 1., 1.), c(1., -1., 1.)],
            vec![c(-1., -1., -1.), c(-1., -1., 1.), c(1., -1., 1.), c(1., -1., -1.)],
            vec![c(-1., 1., -1.), c(1., 1., -1.), c(1., 1., 1.), c(-1., 1., 1.)],
            vec![c(-1., -1., -1.), c(-1., 1., -1.), c(-1., 1., 1.), c(-1., -1., 1.)],
            vec![c(1., -1., -1.), c(1., -1., 1.), c(1., 1., 1.), c(1., 1., -1.)],
        ]
    }

    #[test]
    fn inside_a_brush_by_ray_parity() {
        let b = cube();
        assert!(encompasses(&b, Vec3::ZERO));
        assert!(encompasses(&b, Vec3::new(90.0, -90.0, 50.0)));
        assert!(!encompasses(&b, Vec3::new(150.0, 0.0, 0.0)));
        assert!(!encompasses(&b, Vec3::new(0.0, 0.0, -101.0)));
    }

    #[test]
    fn zed_type_filters() {
        let mut zeds = HashMap::new();
        zeds.insert("kfchar.zombieboss_standard".into(), ZedInfo { flag: 3, is_a: vec!["ZombieBoss".into()], ..default() });
        zeds.insert("kfchar.zombiecrawler_standard".into(), ZedInfo { flag: 2, is_a: vec!["ZombieCrawler".into()], ..default() });
        zeds.insert("kfchar.zombieclot_standard".into(), ZedInfo { flag: 0, ..default() });
        let mut v = ZombieVolume {
            name: "V".into(),
            location: Vec3::ZERO,
            polys: cube(),
            spawn_desirability: 3000.0,
            min_distance_to_player: 600.0,
            no_z_axis_penalty: false,
            allow_plain_sight: false,
            allowed_flags: [true; 4],
            disallowed: vec!["ZombieBoss".into()],
            only_allowed: Vec::new(),
            zombie_count_multi: 1.0,
            touch_disable_time: 10.0,
            enabled: true,
            room_doors: Vec::new(),
            spawn_pos: Vec::new(),
            last_spawn_time: 0.0,
            last_failed_spawn_time: 0.0,
            last_check_time: 0.0,
            player_inside: false,
        };
        assert!(!v.allows(&zeds, "KFChar.ZombieBoss_STANDARD"));
        assert!(v.allows(&zeds, "KFChar.ZombieClot_STANDARD"));
        v.only_allowed = vec!["ZombieCrawler".into()];
        assert!(!v.allows(&zeds, "KFChar.ZombieClot_STANDARD"));
        assert!(v.allows(&zeds, "KFChar.ZombieCrawler_STANDARD"));
        v.allowed_flags[2] = false; // bLeapingZeds off
        assert!(!v.allows(&zeds, "KFChar.ZombieCrawler_STANDARD"));
    }
}
