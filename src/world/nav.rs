//! Navigation for zeds (DESIGN.md, "Pathfinding"): the map's navigation
//! network (NavigationPoints joined by ReachSpecs, read by
//! `ue_assets::nav`), a "can I walk straight there" test, and route search.
//!
//! Positions are Bevy space unless named `_unreal`.

use avian3d::prelude::*;
use bevy::prelude::*;

use ue_assets::nav::{NavGraph, R_DOOR, R_FORCED, R_JUMP, R_WALK};

use crate::engine::coords::{self, SCALE};
use crate::engine::runlog;
use crate::player::walk::Mover;

/// KFMonsterController ZombieHunt shrinks a hunting zed's collision to this
/// (Unreal units) so big zeds use the same paths.
pub const HUNT_RADIUS: f32 = 24.0;
pub const HUNT_HALF_HEIGHT: f32 = 44.0;

/// Reachability walk: step length and limits (assumed; native walkReachable
/// also simulates the walk in steps).
const PROBE_STEP: f32 = 16.0;
const PROBE_MAX: f32 = 2400.0;
/// How high a zed's jump lifts it: KFMonster JumpZ 320 at gravity 950
/// (v^2 / 2g, about 54 units).
pub const JUMP_Z: f32 = 320.0;
pub const JUMP_APEX: f32 = JUMP_Z * JUMP_Z / (2.0 * 950.0);

/// Deepest drop a zed may fall down safely: KFMonster MaxFallSpeed 2500
/// at gravity 950 (v^2 / 2g, about 3289 units).
const MAX_DROP: f32 = 2500.0 * 2500.0 / (2.0 * 950.0);

pub struct NavPoint {
    pub name: String,
    pub pos: Vec3,
}

/// The network, keeping only links a walking zed may use.
#[derive(Resource, Default)]
pub struct NavNetwork {
    pub points: Vec<NavPoint>,
    /// Per point: (next point, cost = ReachSpec Distance).
    pub links: Vec<Vec<(usize, f32)>>,
    /// NavigationPoint.ExtraCost per point, added when a route enters it
    /// (KFDoorMover raises its DoorPathNode's while welded, door.rs).
    pub extra_cost: Vec<f32>,
    /// JumpPads: touching one launches a pawn at its JumpTarget.
    pub jump_pads: Vec<JumpPad>,
}

/// A JumpPad (UTJumppad): a navigation point that throws pawns touching
/// it (CollisionRadius 40, CollisionHeight 43, NavigationPoint defaults)
/// with the JumpVelocity the editor saved, toward JumpTarget
/// (JumpPad.PostTouch). Its link to the target is a special link the
/// walk check leaves alone.
#[derive(Clone, Copy, Debug)]
pub struct JumpPad {
    pub point: usize,
    pub target: usize,
    /// Bevy space, metres per second.
    pub velocity: Vec3,
}

/// JumpPad touch cylinder (NavigationPoint CollisionRadius / Height).
pub const JUMP_PAD_RADIUS: f32 = 40.0;
pub const JUMP_PAD_HALF_HEIGHT: f32 = 43.0;

impl NavNetwork {
    /// Links a 24 x 44 zed may take: walk / forced / door / jump flags only
    /// (no fly, swim, ladder, special, proscribed or player-only) and wide
    /// and tall enough. Jump links (R_JUMP, e.g. over the KF-WestLondon
    /// fence behind the start) need the zeds' jumps; the start-up check
    /// drops those our zeds cannot make.
    pub fn from_graph(g: &NavGraph) -> Self {
        let points = g
            .nodes
            .iter()
            .map(|n| NavPoint {
                name: n.name.clone(),
                pos: coords::pos(n.location),
            })
            .collect::<Vec<_>>();
        let mut links = vec![Vec::new(); points.len()];
        let mut used = 0;
        for e in &g.edges {
            if e.flags & !(R_WALK | R_FORCED | R_DOOR | R_JUMP) == 0 && e.radius >= HUNT_RADIUS && e.height >= HUNT_HALF_HEIGHT && !e.pruned {
                links[e.from].push((e.to, e.distance.max(1.0)));
                used += 1;
            }
        }
        let (sizes, _) = g.groups();
        runlog::kv(
            "nav_loaded",
            &format!(
                "points={} reachspecs={} usable_links={used} groups={} largest={:?}",
                points.len(),
                g.edges.len(),
                sizes.len(),
                &sizes[..sizes.len().min(3)]
            ),
        );
        let extra_cost = vec![0.0; points.len()];
        NavNetwork {
            points,
            links,
            extra_cost,
            jump_pads: Vec::new(),
        }
    }

    /// Reads the map's JumpPads (any class named *Jumppad) and adds each
    /// pad -> JumpTarget link.
    pub fn add_jump_pads(&mut self, pkg: &ue_assets::package::Package) {
        use ue_assets::properties::{Value, read_export_properties};
        let index = |name: &str| self.points.iter().position(|p| p.name.eq_ignore_ascii_case(name));
        let mut pads = Vec::new();
        for i in 0..pkg.exports.len() {
            if !pkg.export_class_name(i).to_ascii_lowercase().ends_with("jumppad") {
                continue;
            }
            let Ok(props) = read_export_properties(pkg, i) else { continue };
            let name = pkg.object_name(ue_assets::package::ObjectRef::Export(i)).to_string();
            let (Some(Value::Vector(v)), Some(Value::Object(t))) = (props.get(pkg, "JumpVelocity"), props.get(pkg, "JumpTarget")) else {
                continue;
            };
            let (Some(point), Some(target)) = (index(&name), index(pkg.object_name(*t))) else {
                continue;
            };
            pads.push(JumpPad {
                point,
                target,
                velocity: coords::dir(*v) * SCALE,
            });
            if !self.links[point].iter().any(|&(b, _)| b == target) {
                let d = (self.points[target].pos - self.points[point].pos).length() / SCALE;
                self.links[point].push((target, d.max(1.0)));
            }
        }
        runlog::kv(
            "nav_jump_pads",
            &format!(
                "pads=[{}]",
                pads.iter()
                    .map(|p| {
                        let v = p.velocity / SCALE;
                        format!("{}->{} up={:.0}", self.points[p.point].name, self.points[p.target].name, v.y)
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        );
        self.jump_pads = pads;
    }

    /// The pad -> target links skip the walk check.
    fn is_pad_link(&self, a: usize, b: usize) -> bool {
        self.jump_pads.iter().any(|p| p.point == a && p.target == b)
    }

    /// Points within `range` metres of `p`, nearest first.
    pub fn near(&self, p: Vec3, range: f32) -> Vec<(usize, f32)> {
        let mut v: Vec<(usize, f32)> = self
            .points
            .iter()
            .enumerate()
            .map(|(i, n)| (i, n.pos.distance(p)))
            .filter(|(_, d)| *d <= range)
            .collect();
        v.sort_by(|a, b| a.1.total_cmp(&b.1));
        v
    }

    /// Shortest route (Dijkstra over ReachSpec distances, Unreal units)
    /// from any of `starts` (point, cost to get there) to any of `goals`
    /// (point, cost from there to the goal). `extra` adds cost for entering
    /// a point (KF's NavigationPoint.ExtraCost). Returns the points in
    /// order and the total cost.
    pub fn route(
        &self,
        starts: &[(usize, f32)],
        goals: &[(usize, f32)],
        extra: &dyn Fn(usize) -> f32,
        skip_link: &dyn Fn(usize, usize) -> bool,
    ) -> Option<(Vec<usize>, f32)> {
        let n = self.points.len();
        let mut dist = vec![f32::INFINITY; n];
        let mut prev = vec![usize::MAX; n];
        let mut done = vec![false; n];
        for &(s, c) in starts {
            let c = c + extra(s);
            if c < dist[s] {
                dist[s] = c;
            }
        }
        let goal_cost: std::collections::HashMap<usize, f32> = goals.iter().copied().collect();
        let mut best: Option<(usize, f32)> = None;
        // Small graphs (hundreds of points): a linear scan is enough.
        while let Some(u) = (0..n).filter(|&i| !done[i] && dist[i].is_finite()).min_by(|&a, &b| dist[a].total_cmp(&dist[b])) {
            if best.is_some_and(|(_, c)| dist[u] >= c) {
                break;
            }
            done[u] = true;
            if let Some(g) = goal_cost.get(&u) {
                let total = dist[u] + g;
                if best.is_none_or(|(_, c)| total < c) {
                    best = Some((u, total));
                }
            }
            for &(v, cost) in &self.links[u] {
                if skip_link(u, v) {
                    continue;
                }
                let c = dist[u] + cost + extra(v);
                if c < dist[v] {
                    dist[v] = c;
                    prev[v] = u;
                }
            }
        }
        let (end, total) = best?;
        let mut path = vec![end];
        while prev[*path.last().expect("non-empty")] != usize::MAX {
            path.push(prev[*path.last().expect("non-empty")]);
        }
        path.reverse();
        Some((path, total))
    }
}


/// The size a zed hunts with: KFMonsterController.ZombieHunt shrinks zeds
/// over 27 x 46 to 24 x 44; smaller ones keep their own (the Clot's 26 x 44).
pub fn hunt_size(radius: f32, half_height: f32) -> (f32, f32) {
    if radius > 27.0 || half_height > 46.0 { (HUNT_RADIUS, HUNT_HALF_HEIGHT) } else { (radius, half_height) }
}

/// Our stand-in for native ActorReachable / pointReachable: walk a cylinder
/// of `radius` x `half_height` (Unreal units) from `from` toward `to` (both
/// cylinder centres) in short steps with the player's movement rules (slide,
/// step up, snap to floor, fall up to MAX_DROP). Ok if it gets within
/// `touch`; Err says why not and where (Unreal units).
pub fn probe(spatial: &SpatialQuery, from: Vec3, to: Vec3, touch: f32, radius: f32, half_height: f32) -> Result<(), String> {
    probe_with(spatial, crate::world::collision::zed_path_filter(), from, to, touch, radius, half_height)
}

/// `probe` blocked by what `filter` lets through (e.g. `zed_filter`, which
/// counts closed doors).
pub fn probe_with(
    spatial: &SpatialQuery,
    filter: SpatialQueryFilter,
    from: Vec3,
    to: Vec3,
    touch: f32,
    radius: f32,
    half_height: f32,
) -> Result<(), String> {
    let mover = Mover::new(spatial, radius, half_height, filter);
    let ue = |p: Vec3| format!("({:.0}, {:.0}, {:.0})", -p.z / SCALE, p.x / SCALE, p.y / SCALE);
    let mut pos = from;
    let mut jumps = 0;
    let total = (to - from).with_y(0.0).length() / SCALE;
    if total > PROBE_MAX {
        return Err("too_far".into());
    }
    // Slopes and sliding make steps shorter than PROBE_STEP: allow twice as many.
    let steps = 2 * (total / PROBE_STEP).ceil() as usize + 4;
    for _ in 0..steps {
        let flat = (to - pos).with_y(0.0);
        let left = flat.length() / SCALE;
        if left <= touch {
            // Close enough sideways; the heights must overlap too.
            let dz = (to.y - pos.y) / SCALE;
            if dz.abs() > 2.0 * half_height {
                return Err(format!("wrong_height dz={dz:.0} at {}", ue(pos)));
            }
            if jumps > 0 {
                runlog::kv("nav_probe_jumped", &format!("from={} to={} jumps={jumps}", ue(from), ue(to)));
            }
            return Ok(());
        }
        let step = flat.normalize() * left.min(PROBE_STEP) * SCALE;
        let (mut moved, _) = mover.ground_move(pos, step);
        if (moved - pos).with_y(0.0).length() / SCALE < 0.5 * left.min(PROBE_STEP) {
            // Blocked: zeds jump what they can clear (JUMP_APEX).
            match mover.jump_over(pos, step, JUMP_APEX * SCALE) {
                Some(p) => {
                    moved = p;
                    jumps += 1;
                }
                None => return Err(format!("blocked at {} left={left:.0}", ue(pos))),
            }
        }
        pos = match mover.snap_to_floor(moved) {
            Some(p) => p,
            // A ledge: fall, if the drop is not too deep. Landing on an edge
            // (a kerb corner reports a steep normal) slides off it a little
            // further along, as a falling zed would.
            None => {
                let nudge = flat.normalize() * 4.0 * SCALE;
                let mut at = moved;
                let mut landed = None;
                for _ in 0..6 {
                    match mover.cast(at, Vec3::NEG_Y, MAX_DROP * SCALE) {
                        Some(h) if h.normal.y >= 0.7 => {
                            landed = Some(at - Vec3::Y * h.distance);
                            break;
                        }
                        Some(h) => at = at - Vec3::Y * h.distance + nudge,
                        None => break,
                    }
                }
                match landed {
                    Some(p) => p,
                    None => return Err(format!("no_floor at {}", ue(moved))),
                }
            }
        };
    }
    Err("too_many_steps".into())
}

pub struct NavPlugin;

impl Plugin for NavPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NavNetwork>().add_systems(Update, check_links_once);
    }
}

/// Once the colliders exist: walk every usable link with `reachable` and log
/// how many fail. KF's editor built these links as walkable, so failures
/// show where our collision or movement differs from Unreal's.
///
/// Links that fail are dropped from routing: our zeds cannot jump or take
/// the special moves some links need (an approximation until they can).
fn check_links_once(
    frames: Res<bevy::diagnostic::FrameCount>,
    spatial: SpatialQuery,
    names: Query<&Name>,
    mut nav: ResMut<NavNetwork>,
    mut done: Local<bool>,
) {
    if *done || frames.0 < 5 || nav.points.is_empty() {
        return;
    }
    *done = true;
    let started = std::time::Instant::now();
    let (mut ok, mut failed) = (0usize, Vec::new());
    let mut kept = Vec::with_capacity(nav.links.len());
    for (a, links) in nav.links.iter().enumerate() {
        let mut keep = Vec::new();
        for &(b, cost) in links {
            if nav.is_pad_link(a, b) {
                ok += 1;
                keep.push((b, cost));
                continue;
            }
            // Checked at the Clot's and Gorefast's hunting size (26 x 44).
            match probe(&spatial, nav.points[a].pos, nav.points[b].pos, HUNT_RADIUS, 26.0, HUNT_HALF_HEIGHT) {
                Ok(()) => {
                    ok += 1;
                    keep.push((b, cost));
                }
                Err(why) => {
                    // What is in the way: sweep the cylinder from the failure
                    // point toward the target and name the collider hit.
                    let mut what = String::new();
                    if let Some(at) = why.split(" at (").nth(1).and_then(|s| s.split(')').next()) {
                        let v: Vec<f32> = at.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                        if let [x, y, z] = v[..] {
                            let p = coords::pos([x, y, z]);
                            let dir = (nav.points[b].pos - p).with_y(0.0).normalize_or_zero();
                            let mover = Mover::new(&spatial, 26.0, HUNT_HALF_HEIGHT, crate::world::collision::zed_path_filter());
                            if let Some(h) = mover.cast(p, dir, 64.0 * SCALE) {
                                let name = names.get(h.entity).map_or("unnamed".to_string(), |n| n.to_string());
                                what = format!(" hit={name} normal_up={:.2}", h.normal.y);
                            }
                        }
                    }
                    failed.push(format!("{}->{}:{why}{what}", nav.points[a].name, nav.points[b].name));
                }
            }
        }
        kept.push(keep);
    }
    nav.links = kept;
    runlog::kv(
        "nav_link_check",
        &format!(
            "walkable={ok} not_walkable={} seconds={:.2} examples=[{}]",
            failed.len(),
            started.elapsed().as_secs_f64(),
            failed.iter().take(6).cloned().collect::<Vec<_>>().join(" | ")
        ),
    );
    for f in &failed {
        runlog::kv("nav_link_unwalkable", f);
    }
    // Which points can reach each other over the links zeds use (one-way
    // links count both ways here): the groups, largest first.
    let n = nav.points.len();
    let mut group = vec![usize::MAX; n];
    let mut sizes = Vec::new();
    for start in 0..n {
        if group[start] != usize::MAX {
            continue;
        }
        let g = sizes.len();
        let mut stack = vec![start];
        group[start] = g;
        let mut size = 0;
        while let Some(a) = stack.pop() {
            size += 1;
            let next: Vec<usize> = nav.links[a].iter().map(|&(b, _)| b).chain((0..n).filter(|&b| nav.links[b].iter().any(|&(c, _)| c == a))).collect();
            for b in next {
                if group[b] == usize::MAX {
                    group[b] = g;
                    stack.push(b);
                }
            }
        }
        sizes.push(size);
    }
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|&g| std::cmp::Reverse(sizes[g]));
    runlog::kv(
        "nav_groups",
        &format!("groups={} sizes={:?}", sizes.len(), order.iter().map(|&g| sizes[g]).take(12).collect::<Vec<_>>()),
    );
}

/// Where a hunting zed is heading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    /// Straight at the player (ActorReachable).
    Player,
    /// A navigation point on the route (MoveTarget).
    Point(usize),
}

/// KF's hunting route state for one zed (KFMonsterController: MoveTarget,
/// LastResult, NumAttmpts, BlockedWay, ExtraCostWay).
#[derive(Default)]
pub struct Router {
    pub target: Option<Target>,
    /// Time left for the current move before it counts as failed (MoveTimer).
    move_timer: f32,
    /// Time to the next "can I walk straight to the player now" check while
    /// on a route.
    check_timer: f32,
    last_result: Option<usize>,
    attempts: u32,
    blocked_way: Option<usize>,
    route_len: usize,
    /// The last point reached, and links (from, to) this zed failed to walk
    /// with how many times (our addition, not KF's: some links the editor
    /// made are impossible for our zeds, e.g. jumps they cannot clear;
    /// after two failures a link is left out of this zed's routes).
    from_point: Option<usize>,
    failed_links: Vec<((usize, usize), u32)>,
    /// Progress check (our addition): where the zed was a second ago, and
    /// how long walking straight at the player is off after getting stuck.
    progress_from: Option<Vec3>,
    progress_timer: f32,
    no_direct: f32,
    /// The last route search found no way to the goal (and the goal was
    /// not walkable directly).
    pub no_route: bool,
}

/// Progress check: a zed that covers less than this share of its speed in
/// STUCK_WINDOW seconds counts as stuck (our addition: sliding along a
/// corner can stall a straight walk that the walk test passed).
const STUCK_SHARE: f32 = 0.25;
const STUCK_WINDOW: f32 = 1.0;
/// After getting stuck walking straight at the player, use the network for
/// this long.
const NO_DIRECT: f32 = 3.0;

/// What the router needs from the world each think.
pub struct RouteInput<'a> {
    pub id: usize,
    pub pos: Vec3,
    pub player: Vec3,
    /// Touching distance to the player (Unreal units, both radii).
    pub touch_player: f32,
    pub speed: f32,
    /// The zed's hunting size (`hunt_size`), Unreal units.
    pub radius: f32,
    pub half_height: f32,
    /// Other zeds' cylinders (centre, radius in metres).
    pub others: &'a [(Vec3, f32)],
}

impl RouteInput<'_> {
    /// Can this zed walk straight from `from` to `to`?
    /// ActorReachable / pointReachable at run time: closed doors block
    /// (native code; assumed that movers block it, not verified), so a
    /// zed behind a shut door follows the path through the doorway (and
    /// meets the trigger or, for a Bloat or Husk, sees the welded door).
    fn walkable(&self, spatial: &SpatialQuery, from: Vec3, to: Vec3, touch: f32) -> bool {
        probe_with(spatial, crate::world::collision::zed_filter(), from, to, touch, self.radius, self.half_height).is_ok()
    }
}

/// KF's anchor search: nav points within this many Unreal units...
const ANCHOR_RANGE: f32 = 1200.0;
/// ...at most this many, nearest first.
const ANCHOR_CANDIDATES: usize = 32;

/// KF's cheap check before the reach test: a line from `a` to `b`, or, if
/// that is blocked, from `b` raised by the pawn's half height (Unreal
/// units) to `a` raised the same.
fn sees(spatial: &SpatialQuery, a: Vec3, b: Vec3, half_height: f32) -> bool {
    let clear = |a: Vec3, b: Vec3| {
        let Ok(d) = Dir3::new(b - a) else { return true };
        spatial.cast_ray(a, d, (b - a).length(), true, &crate::world::collision::zed_filter()).is_none()
    };
    let up = Vec3::Y * half_height * SCALE;
    clear(a, b) || clear(b + up, a + up)
}

/// KF's anchor search (FindPathToward): nav points within 1200 units of
/// `at`, nearest first, at most 32; the first one in sight that passes
/// `reach` is the anchor. Returns it with its distance (Unreal units) and
/// the number of points tried.
pub fn anchor(nav: &NavNetwork, spatial: &SpatialQuery, at: Vec3, half_height: f32, reach: impl Fn(usize) -> bool) -> (Option<(usize, f32)>, usize) {
    let mut tried = 0;
    for (i, d) in nav.near(at, ANCHOR_RANGE * SCALE).into_iter().take(ANCHOR_CANDIDATES) {
        tried += 1;
        if sees(spatial, at, nav.points[i].pos, half_height) && reach(i) {
            return (Some((i, d / SCALE)), tried);
        }
    }
    (None, tried)
}

/// Re-check interval for walking straight at the player while following a
/// route, and for re-aiming while walking straight at the player (assumed;
/// KF re-runs PickDestination when a move ends).
const RECHECK: f32 = 0.5;

impl Router {
    /// Updates the target if the current move has ended (reached, timed out,
    /// or none yet) and returns the point to walk toward.
    pub fn update(&mut self, nav: &NavNetwork, spatial: &SpatialQuery, inp: &RouteInput, dt: f32, frand: &mut dyn FnMut() -> f32) -> Vec3 {
        self.move_timer -= dt;
        self.check_timer -= dt;
        self.no_direct -= dt;
        // Progress check.
        self.progress_timer += dt;
        let from = *self.progress_from.get_or_insert(inp.pos);
        if self.progress_timer >= STUCK_WINDOW {
            let moved = (inp.pos - from).with_y(0.0).length() / SCALE;
            let expected = inp.speed * self.progress_timer;
            self.progress_from = Some(inp.pos);
            self.progress_timer = 0.0;
            if moved < STUCK_SHARE * expected {
                runlog::kv(
                    "zed_stuck",
                    &format!(
                        "id={} moved_unreal={moved:.0} expected={expected:.0} target={:?} goal_distance_unreal={:.0}",
                        inp.id,
                        self.target,
                        (inp.player - inp.pos).length() / SCALE
                    ),
                );
                if self.target == Some(Target::Player) {
                    self.no_direct = NO_DIRECT;
                }
                self.move_timer = 0.0; // the move has ended
            }
        }
        let point_pos = |t: Option<Target>| match t {
            Some(Target::Point(i)) => nav.points[i].pos,
            _ => inp.player,
        };
        let reached = match self.target {
            Some(Target::Point(i)) => {
                let d = nav.points[i].pos - inp.pos;
                let beside = d.with_y(0.0).length() / SCALE <= HUNT_RADIUS + 8.0;
                let level = (d.y / SCALE).abs() <= 2.0 * HUNT_HALF_HEIGHT;
                // Right above or below it but not on its level (e.g. on a
                // ledge over it): circling would never get there, so the move
                // ends within a second and the stuck rule can take over.
                if beside && !level {
                    self.move_timer = self.move_timer.min(1.0);
                }
                beside && level
            }
            _ => false,
        };
        let on_route = matches!(self.target, Some(Target::Point(_)));
        let timed_out = self.move_timer <= 0.0;
        // While on a route, switch to the player once it is walkable.
        if on_route && !reached && !timed_out && self.check_timer <= 0.0 {
            self.check_timer = RECHECK;
            if self.no_direct <= 0.0 && inp.walkable(spatial, inp.pos, inp.player, inp.touch_player) {
                self.set(Target::Player, inp, nav, "player_walkable");
            }
            return point_pos(self.target);
        }
        if self.target.is_some() && !reached && !timed_out {
            return point_pos(self.target);
        }
        match self.target {
            Some(Target::Point(i)) if reached => self.from_point = Some(i),
            Some(Target::Point(i)) if timed_out => {
                if let Some(f) = self.from_point {
                    let n = match self.failed_links.iter_mut().find(|(l, _)| *l == (f, i)) {
                        Some((_, n)) => {
                            *n += 1;
                            *n
                        }
                        None => {
                            self.failed_links.push(((f, i), 1));
                            1
                        }
                    };
                    if n == 2 {
                        runlog::kv(
                            "zed_link_given_up",
                            &format!("id={} from={} to={}", inp.id, nav.points[f].name, nav.points[i].name),
                        );
                    }
                }
                self.from_point = None;
            }
            _ => {}
        }
        self.pick(nav, spatial, inp, frand, if reached { "reached" } else if timed_out { "move_ended" } else { "start" });
        point_pos(self.target)
    }

    fn set(&mut self, t: Target, inp: &RouteInput, nav: &NavNetwork, reason: &str) {
        let to = match t {
            Target::Point(i) => nav.points[i].pos,
            Target::Player => inp.player,
        };
        let dist = (to - inp.pos).length() / SCALE;
        // MoveTimer for a MoveToward (assumed: 1 s plus 1.3 x the walking time).
        self.move_timer = if t == Target::Player { RECHECK } else { 1.0 + 1.3 * dist / inp.speed.max(1.0) };
        self.check_timer = RECHECK;
        if self.target != Some(t) {
            let name = match t {
                Target::Point(i) => nav.points[i].name.as_str(),
                Target::Player => "player",
            };
            runlog::kv(
                "zed_route",
                &format!(
                    "id={} target={name} reason={reason} distance_unreal={dist:.0} route_points={} attempts={} blocked_way={}",
                    inp.id,
                    self.route_len,
                    self.attempts,
                    self.blocked_way.map_or("none", |b| nav.points[b].name.as_str())
                ),
            );
        }
        self.target = Some(t);
    }

    /// PickDestination + FindBestPathToward.
    fn pick(&mut self, nav: &NavNetwork, spatial: &SpatialQuery, inp: &RouteInput, frand: &mut dyn FnMut() -> f32, reason: &str) {
        self.no_route = false;
        if self.no_direct <= 0.0 && inp.walkable(spatial, inp.pos, inp.player, inp.touch_player) {
            self.route_len = 0;
            self.set(Target::Player, inp, nav, reason);
            return;
        }
        let Some(mut next) = self.find_path(nav, spatial, inp, None) else {
            // KF would hunt the last seen position; we walk straight at the
            // player. The escaping Patriarch heals instead (boss_ai.rs).
            self.no_route = true;
            self.route_len = 0;
            if self.target != Some(Target::Player) {
                runlog::kv("zed_route_failed", &format!("id={} reason=no_route", inp.id));
            }
            self.set(Target::Player, inp, nav, "no_route");
            return;
        };
        // Another zed between us and the next point: +200 there and re-plan.
        if self.zed_in_way(nav, inp, next)
            && let Some(alt) = self.find_path(nav, spatial, inp, Some((next, 200.0)))
        {
            next = alt;
        }
        // Same target again and again (more than 3 times): 60% of the time
        // treat it as blocked (+10000) and re-plan.
        if self.last_result == Some(next) && self.attempts > 3 && frand() < 0.6 {
            self.blocked_way = Some(next);
            self.last_result = None;
            self.attempts = 0;
            runlog::kv("zed_route_blocked", &format!("id={} point={}", inp.id, nav.points[next].name));
            if let Some(alt) = self.find_path(nav, spatial, inp, None) {
                next = alt;
            }
        } else if self.last_result == Some(next) {
            self.attempts += 1;
        } else {
            self.attempts = 0;
        }
        self.last_result = Some(next);
        self.set(Target::Point(next), inp, nav, reason);
    }

    /// FindPathToward: the first point to head for (with the RouteCache[1]
    /// shortcut), avoiding `extra` (point, cost) and the blocked way.
    fn find_path(&mut self, nav: &NavNetwork, spatial: &SpatialQuery, inp: &RouteInput, extra: Option<(usize, f32)>) -> Option<usize> {
        // KF: one start anchor near the zed and one near the player (the
        // first nav point in sight that the zed can reach), not a set.
        let (start, start_tried) = anchor(nav, spatial, inp.pos, inp.half_height, |i| inp.walkable(spatial, inp.pos, nav.points[i].pos, HUNT_RADIUS));
        let (goal, goal_tried) = anchor(nav, spatial, inp.player, inp.half_height, |i| inp.walkable(spatial, nav.points[i].pos, inp.player, inp.touch_player));
        let starts: Vec<(usize, f32)> = start.into_iter().collect();
        let goals: Vec<(usize, f32)> = goal.into_iter().collect();
        let blocked = self.blocked_way;
        let cost = |i: usize| -> f32 {
            let mut c = 0.0;
            if Some(i) == blocked {
                c += 10000.0;
            }
            if let Some((p, x)) = extra
                && p == i
            {
                c += x;
            }
            c + nav.extra_cost.get(i).copied().unwrap_or(0.0)
        };
        let failed = &self.failed_links;
        let skip = |a: usize, b: usize| failed.iter().any(|&(l, n)| l == (a, b) && n >= 2);
        let Some((path, total)) = nav.route(&starts, &goals, &cost, &skip) else {
            runlog::kv(
                "zed_path_none",
                &format!("id={} starts={} goals={} start_tried={start_tried} goal_tried={goal_tried}", inp.id, starts.len(), goals.len()),
            );
            return None;
        };
        self.route_len = path.len();
        runlog::kv(
            "zed_path",
            &format!(
                "id={} starts={} goals={} points={} cost={total:.0} route=[{}]",
                inp.id,
                starts.len(),
                goals.len(),
                path.len(),
                path.iter().map(|&i| nav.points[i].name.as_str()).collect::<Vec<_>>().join(" ")
            ),
        );
        // Skip the first point if we are already on it, and take the
        // RouteCache[1] shortcut when the second is walkable.
        let close = |i: usize| (nav.points[i].pos - inp.pos).with_y(0.0).length() / SCALE <= HUNT_RADIUS + 8.0;
        let mut first = path[0];
        if path.len() > 1 && (close(first) || inp.walkable(spatial, inp.pos, nav.points[path[1]].pos, HUNT_RADIUS)) {
            first = path[1];
        }
        Some(first)
    }

    /// True if another zed's cylinder lies on the line to point `i`
    /// (KF: Trace to MoveTarget hits a KFMonster).
    fn zed_in_way(&self, nav: &NavNetwork, inp: &RouteInput, i: usize) -> bool {
        let (a, b) = (inp.pos.with_y(0.0), nav.points[i].pos.with_y(0.0));
        let ab = b - a;
        inp.others.iter().any(|&(c, r)| {
            let t = ((c.with_y(0.0) - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            (a + ab * t).distance(c.with_y(0.0)) < r + HUNT_RADIUS * SCALE && t > 0.0
        })
    }
}
