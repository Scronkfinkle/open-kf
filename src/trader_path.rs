//! The trail to the trader (T2b-1): during trader time a red whisp flies
//! from the player along the route to the open shop every 1.1 s
//! (KFPlayerController.SetShowPathToTrader / Timer, KFGameType.ShowPathTo,
//! TraderPathEffect, RedWhisp, WillowWhisp). The whisp's flight is script
//! and copied; its sprites are a native xEmitter, not in the scripts, so
//! they are built from the settings with the guesses marked "assumed".
//! See DESIGN.md, T2b-1. Unreal units and axes inside; Bevy when drawn.

use avian3d::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package::ObjectRef;
use ue_assets::package_set::PackageSet;
use ue_assets::properties::Value;

use crate::camera::FlyCamera;
use crate::coords::{self, SCALE};
use crate::map::MapRequest;
use crate::runlog;

/// KFPlayerController.TraderPathInterval.
const TRADER_PATH_INTERVAL: f32 = 1.1;
/// Controller.RouteCache holds 16 points.
const ROUTE_CACHE: usize = 16;
/// WillowWhisp: StartNextPath acceleration, next point within 80.
const WHISP_ACCEL: f32 = 1200.0;
const WHISP_REACHED: f32 = 80.0;
/// WillowWhisp LifeSpan default; StartNextPath at the end: 1.5 s more.
const WHISP_LIFESPAN: f32 = 10.0;
const WHISP_END_LIFE: f32 = 1.5;
/// Sprites (WillowWhisp / RedWhisp defaults): rate, cap, life, size,
/// growth, spin (assumed degrees a second), mass, air resistance (the
/// xEmitter default), tiles.
const REGEN_PER_SEC: f32 = 90.0;
const MAX_PARTICLES: usize = 150;
const PARTICLE_LIFE: f32 = 1.25;
const SIZE_RANGE: (f32, f32) = (25.0, 30.0);
const GROWTH_RATE: f32 = 13.0;
const SPIN_RANGE: (f32, f32) = (-75.0, 75.0);
const MASS_RANGE: (f32, f32) = (-0.03, -0.01);
const AIR_RESISTANCE: f32 = 0.4;
const TILES: u32 = 4;
/// RedWhisp mColorRange (both ends the same).
const COLOR: [f32; 3] = [1.0, 40.0 / 255.0, 40.0 / 255.0];
/// mAttenKa: the fade-in share of the life (assumed meaning).
const ATTEN_KA: f32 = 0.2;
/// PhysicsVolume gravity (Unreal units / s^2, down).
const GRAVITY: f32 = 950.0;
const WHISP_CLASS: &str = "KFMod.RedWhisp";

struct Sprite {
    pos: Vec3,
    vel_z: f32,
    age: f32,
    size: f32,
    angle: f32,
    spin: f32,
    mass: f32,
    tile: u32,
}

struct Whisp {
    id: u32,
    loc: Vec3,
    vel: Vec3,
    acc: Vec3,
    way_points: Vec<Vec3>,
    /// Index of the next way point to take (WillowWhisp.Position).
    position: usize,
    destination: Vec3,
    headed_right: bool,
    /// LifeLeft: > 0 once the last point is passed.
    life_left: f32,
    age: f32,
    /// mRegen: sprites still being made.
    regen: bool,
    regen_owed: f32,
    sprites: Vec<Sprite>,
    /// The native random numbers are unknown: a small LCG per whisp.
    rng: u32,
}

impl Whisp {
    /// WillowWhisp.StartNextPath.
    fn start_next_path(&mut self) {
        if self.position >= self.way_points.len() {
            self.regen = false;
            self.life_left = WHISP_END_LIFE;
            self.vel = Vec3::ZERO;
            self.acc = Vec3::ZERO;
            runlog::kv(
                "trader_whisp_end",
                &format!("id={} reason=last_point age={:.2} at=({:.0}, {:.0}, {:.0})", self.id, self.age, self.loc.x, self.loc.y, self.loc.z),
            );
            return;
        }
        self.headed_right = false;
        self.destination = self.way_points[self.position];
        self.acc = WHISP_ACCEL * (self.destination - self.loc).normalize_or_zero();
        self.vel *= 0.5;
        self.vel.z = 0.5 * (self.vel.z + self.acc.z);
        self.position += 1;
    }

    /// One frame: Pathing.Tick (script), then PHYS_Projectile. False once
    /// destroyed.
    fn tick(&mut self, dt: f32) -> bool {
        self.age += dt;
        if self.age >= WHISP_LIFESPAN {
            runlog::kv("trader_whisp_end", &format!("id={} reason=lifespan points_left={}", self.id, self.way_points.len() - self.position));
            return false;
        }
        if self.life_left > 0.0 {
            self.life_left -= dt;
            if self.life_left <= 0.0 {
                return false;
            }
        } else {
            self.acc = WHISP_ACCEL * (self.destination - self.loc).normalize_or_zero();
            // "force double acceleration": here and again in the physics.
            self.vel += dt * self.acc;
            if !self.headed_right {
                self.headed_right = self.vel.dot(self.acc) > 0.0;
            } else if self.vel.dot(self.acc) < 0.0 {
                self.start_next_path();
            }
            if (self.destination - self.loc).length() < WHISP_REACHED {
                self.start_next_path();
            }
        }
        // PHYS_Projectile: no collision with the world (bCollideWorld off).
        let from = self.loc;
        self.vel += self.acc * dt;
        self.loc += self.vel * dt;
        self.emit(from, dt);
        true
    }

    /// The xEmitter's sprites: REGEN_PER_SEC along the way the head moved
    /// this frame (assumed spread along the move), aged and drifting.
    fn emit(&mut self, from: Vec3, dt: f32) {
        for s in self.sprites.iter_mut() {
            s.age += dt;
            s.size += GROWTH_RATE * dt;
            s.angle += s.spin * dt;
            // Negative mass under gravity rises (assumed reading).
            s.vel_z += -s.mass * GRAVITY * dt;
            s.vel_z *= (1.0 - AIR_RESISTANCE * dt).max(0.0);
            s.pos.z += s.vel_z * dt;
        }
        self.sprites.retain(|s| s.age < PARTICLE_LIFE);
        if !self.regen {
            return;
        }
        self.regen_owed += REGEN_PER_SEC * dt;
        let n = self.regen_owed.floor() as usize;
        self.regen_owed -= n as f32;
        for k in 0..n {
            if self.sprites.len() >= MAX_PARTICLES {
                break;
            }
            let f = (k as f32 + 1.0) / n as f32;
            let (a, b, c, d, e) = (self.rand(), self.rand(), self.rand(), self.rand(), self.rand());
            self.sprites.push(Sprite {
                pos: from.lerp(self.loc, f),
                vel_z: 0.0,
                age: 0.0,
                size: SIZE_RANGE.0 + (SIZE_RANGE.1 - SIZE_RANGE.0) * a,
                angle: b * std::f32::consts::TAU,
                spin: (SPIN_RANGE.0 + (SPIN_RANGE.1 - SPIN_RANGE.0) * c).to_radians(),
                mass: MASS_RANGE.0 + (MASS_RANGE.1 - MASS_RANGE.0) * d,
                tile: ((e * (TILES * TILES) as f32) as u32).min(TILES * TILES - 1),
            });
        }
    }

    fn rand(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1_103_515_245).wrapping_add(12345);
        ((self.rng >> 8) & 0xFFFF) as f32 / 65536.0
    }
}

#[derive(Resource, Default)]
pub struct TraderPath {
    /// bShowTraderPath.
    pub on: bool,
    next_tick: f32,
    doors_were_open: bool,
    was_inside: Option<usize>,
    whisps: Vec<Whisp>,
    spawned: u32,
}

#[derive(Component)]
struct TrailMesh;

pub struct TraderPathPlugin;

impl Plugin for TraderPathPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TraderPath>()
            .add_systems(PostStartup, spawn_trail_mesh)
            .add_systems(Update, (switch_and_spawn, fly).chain())
            .add_systems(PostUpdate, draw_trail.before(bevy::transform::TransformSystems::Propagate));
    }
}

fn spawn_trail_mesh(
    mut commands: Commands,
    request: Res<MapRequest>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let set = PackageSet::new(&request.install_root);
    let defaults = ClassDefaults::new(&set);
    // RedWhisp.Skins[0], a packed array of object references.
    let skin = crate::gore::find_class(&set, WHISP_CLASS).and_then(|class| match defaults.get(&class, "Skins") {
        Some((Value::Array { count, raw }, pkg)) if count > 0 => {
            let mut r = ue_assets::reader::Reader::new(&raw);
            let first = r.compact_index().ok().map(ObjectRef::from_raw)?;
            let from = ue_assets::package_set::ObjectHandle { package: pkg, export: 0 };
            Some(ue_assets::material::resolve(&set, &from, first))
        }
        _ => None,
    });
    let texture = skin
        .as_ref()
        .and_then(|m| m.texture.as_ref())
        // SmokeAlphab_t is grey with its shape in the alpha: kept, so the
        // additive glow is weighted by it (assumed: with alpha dropped the
        // sprites show as grey squares).
        .and_then(|h| crate::particles::decode(h, false, false, &mut images));
    runlog::kv(
        "trader_whisp_ready",
        &format!(
            "chain={:?} texture={} name={}",
            skin.as_ref().map(|m| m.chain.clone()),
            texture.is_some(),
            skin.as_ref().and_then(|m| m.texture.as_ref()).map_or("-".into(), |h| h.path())
        ),
    );
    let material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: texture,
        unlit: true,
        alpha_mode: AlphaMode::Add,
        cull_mode: None,
        double_sided: true,
        ..default()
    });
    let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    commands.spawn((
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(material),
        Transform::default(),
        Visibility::Hidden,
        NoFrustumCulling,
        bevy::light::NotShadowCaster,
        TrailMesh,
    ));
}

type PlayerQuery<'w, 's> = Query<'w, 's, (&'static Transform, Option<&'static crate::walk::Walker>), With<FlyCamera>>;

/// SetShowPathToTrader on and off, and the Timer's ShowPathTo.
fn switch_and_spawn(
    time: Res<Time>,
    mut path: ResMut<TraderPath>,
    shops: Res<crate::trader::Shops>,
    player: PlayerQuery,
    nav: Option<Res<crate::nav::NavNetwork>>,
    spatial: SpatialQuery,
) {
    let now = time.elapsed_secs();
    let path = &mut *path;
    // OpenShops: on (Timer at once, then every interval). CloseShops: off.
    if shops.doors_open != path.doors_were_open {
        path.doors_were_open = shops.doors_open;
        path.on = shops.doors_open;
        path.next_tick = now;
        runlog::kv("trader_path", &format!("on={} reason={}", path.on, if path.on { "open_shops" } else { "close_shops" }));
    }
    // ShopVolume.Touch on the open shop with no wave: off, and it stays
    // off until the next OpenShops even after leaving (KF quirk, copied).
    let inside = shops.player_inside();
    if inside != path.was_inside {
        path.was_inside = inside;
        if let Some(s) = inside
            && shops.doors_open
            && shops.shops[s].open
            && path.on
        {
            path.on = false;
            runlog::kv("trader_path", &format!("on=false reason=touched_shop shop={}", shops.shops[s].name));
        }
    }
    if !path.on || now < path.next_tick {
        return;
    }
    path.next_tick = now + TRADER_PATH_INTERVAL;
    let (Ok((cam, walker)), Some(nav)) = (player.single(), nav) else { return };
    let centre_b = walker.map_or(cam.translation - Vec3::Y * crate::combat::PLAYER_EYE_HEIGHT * SCALE, |w| w.center);
    let pawn = ue(centre_b);
    let pawn_vel = walker.map_or(Vec3::ZERO, |w| ue(w.velocity));
    let view = ue(cam.forward().as_vec3() * SCALE).normalize_or_zero();
    match show_path_to(&shops, &nav, &spatial, centre_b, pawn, pawn_vel, view) {
        Ok(mut w) => {
            path.spawned += 1;
            w.id = path.spawned;
            w.rng = path.spawned.wrapping_mul(0x9E37_79B9);
            runlog::kv(
                "trader_whisp",
                &format!(
                    "id={} way_points={} [{}]",
                    w.id,
                    w.way_points.len(),
                    w.way_points.iter().map(|p| format!("({:.0}, {:.0}, {:.0})", p.x, p.y, p.z)).collect::<Vec<_>>().join(" ")
                ),
            );
            // PostNetBeginPlay in standalone: StartNextPath at once.
            w.start_next_path();
            path.whisps.push(w);
        }
        Err(why) => runlog::kv("trader_whisp", &format!("spawned=false reason={why}")),
    }
}

/// KFGameType.ShowPathTo and TraderPathEffect.PostBeginPlay: the whisp
/// with its way points, or why there is none.
fn show_path_to(
    shops: &crate::trader::Shops,
    nav: &crate::nav::NavNetwork,
    spatial: &SpatialQuery,
    centre_b: Vec3,
    pawn: Vec3,
    pawn_vel: Vec3,
    view: Vec3,
) -> Result<Whisp, String> {
    let shop = &shops.shops[shops.current.ok_or("no_current_shop")?];
    // CurrentShop.TelList[0].
    let tel = shop.teleporters.first().map(|&t| &shops.teleporters[t]).ok_or("no_teleporter")?;
    let (radius, half) = (20.0, 50.0); // KFPawn CollisionRadius, KFHumanPawn CollisionHeight
    let reach = |from: Vec3, to: Vec3| crate::nav::probe(spatial, from, to, radius + 8.0, radius, half).is_ok();
    // FindPathToward(TelList[0]): from the points the player can walk to,
    // to the teleporter's own point (it is a NavigationPoint), else the
    // points that can walk to it.
    let range = 1500.0 * SCALE;
    let starts: Vec<(usize, f32)> = nav
        .near(centre_b, range)
        .into_iter()
        .take(8)
        .filter(|&(i, _)| reach(centre_b, nav.points[i].pos))
        .map(|(i, d)| (i, d / SCALE))
        .collect();
    let tel_b = coords::pos(tel.location.to_array());
    let goals: Vec<(usize, f32)> = match nav.points.iter().position(|p| p.name.eq_ignore_ascii_case(&tel.name)) {
        Some(i) => vec![(i, 0.0)],
        None => nav.near(tel_b, range).into_iter().take(8).filter(|&(i, _)| reach(nav.points[i].pos, tel_b)).map(|(i, d)| (i, d / SCALE)).collect(),
    };
    let extra = |i: usize| nav.extra_cost.get(i).copied().unwrap_or(0.0);
    let (mut route, _) = nav.route(&starts, &goals, &extra, &|_, _| false).ok_or_else(|| format!("no_route starts={} goals={}", starts.len(), goals.len()))?;
    route.truncate(ROUTE_CACHE);
    let loc = |i: usize| ue(nav.points[i].pos);
    // WayPoints[0]: 200 ahead along the view, cut by a wall.
    let mut wp0 = pawn + 200.0 * view;
    let to_b = coords::pos(wp0.to_array());
    if let Ok(d) = Dir3::new(to_b - centre_b)
        && let Some(hit) = spatial.cast_ray(centre_b, d, (to_b - centre_b).length(), true, &crate::collision::world_filter())
    {
        wp0 = ue(centre_b + *d * hit.distance);
    }
    let mut way_points = vec![wp0];
    // "if ( (C.RouteCache[i] != None) && C.RouteCache[1] != none && ...":
    // i is not set yet, so RouteCache[0] (copied).
    let start = if route.len() > 1 && reach(centre_b, nav.points[route[1]].pos) { 1 } else { 0 };
    for &p in route.iter().skip(start).take(10) {
        way_points.push(loc(p));
    }
    if way_points.len() < start + 10 {
        way_points.push(shop.location);
    }
    let vel = 500.0 * (wp0 - pawn).normalize_or_zero() + pawn_vel;
    Ok(Whisp {
        id: 0,
        loc: pawn,
        vel,
        acc: Vec3::ZERO,
        way_points,
        position: 0,
        destination: pawn,
        headed_right: false,
        life_left: 0.0,
        age: 0.0,
        regen: true,
        regen_owed: 0.0,
        sprites: Vec::new(),
        rng: 1,
    })
}

fn ue(p: Vec3) -> Vec3 {
    Vec3::new(-p.z, p.x, p.y) / SCALE
}

fn fly(time: Res<Time>, mut path: ResMut<TraderPath>, mut log_timer: Local<f32>) {
    let dt = time.delta_secs();
    path.whisps.retain_mut(|w| w.tick(dt));
    *log_timer += dt;
    if *log_timer >= 0.5 && !path.whisps.is_empty() {
        *log_timer = 0.0;
        let heads: Vec<String> = path
            .whisps
            .iter()
            .map(|w| format!("{}:({:.0}, {:.0}, {:.0}) next={} sprites={}", w.id, w.loc.x, w.loc.y, w.loc.z, w.position, w.sprites.len()))
            .collect();
        runlog::kv("trader_whisps", &heads.join(" "));
    }
}

/// Rebuilds the sprite quads, facing the camera.
fn draw_trail(
    path: Res<TraderPath>,
    camera: Query<&Transform, (With<FlyCamera>, Without<TrailMesh>)>,
    mut trail: Query<(&Mesh3d, &mut Visibility), With<TrailMesh>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let (Ok((mesh, mut vis)), Ok(cam)) = (trail.single_mut(), camera.single()) else { return };
    let count: usize = path.whisps.iter().map(|w| w.sprites.len()).sum();
    if count == 0 {
        *vis = Visibility::Hidden;
        return;
    }
    let Some(mut m) = meshes.get_mut(&mesh.0) else { return };
    let (right, up) = (cam.right().as_vec3(), cam.up().as_vec3());
    let (mut positions, mut uvs, mut colors, mut indices) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let tile = 1.0 / TILES as f32;
    for w in &path.whisps {
        for s in &w.sprites {
            let t = (s.age / PARTICLE_LIFE).clamp(0.0, 1.0);
            // mAttenuate (ATF_ExpInOut, Ka 0.2): in over the first fifth,
            // then out (assumed curve). Additive: darker is fainter.
            let k = (t / ATTEN_KA).min(1.0) * (1.0 - t);
            let c = [COLOR[0] * k, COLOR[1] * k, COLOR[2] * k, 1.0];
            let centre = coords::pos(s.pos.to_array());
            let half = 0.5 * s.size * SCALE;
            let (sin, cos) = s.angle.sin_cos();
            let (r, u) = ((right * cos + up * sin) * half, (up * cos - right * sin) * half);
            let base = positions.len() as u32;
            for (corner, uv) in [(-r - u, [0.0, 1.0]), (r - u, [1.0, 1.0]), (r + u, [1.0, 0.0]), (-r + u, [0.0, 0.0])] {
                positions.push((centre + corner).to_array());
                let (tx, ty) = ((s.tile % TILES) as f32, (s.tile / TILES) as f32);
                uvs.push([(tx + uv[0]) * tile, (ty + uv[1]) * tile]);
                colors.push(c);
            }
            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    m.insert_indices(Indices::U32(indices));
    *vis = Visibility::Visible;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn whisp(way_points: Vec<Vec3>, vel: Vec3) -> Whisp {
        Whisp {
            id: 1,
            loc: Vec3::ZERO,
            vel,
            acc: Vec3::ZERO,
            way_points,
            position: 0,
            destination: Vec3::ZERO,
            headed_right: false,
            life_left: 0.0,
            age: 0.0,
            regen: true,
            regen_owed: 0.0,
            sprites: Vec::new(),
            rng: 7,
        }
    }

    #[test]
    fn whisp_visits_every_point_then_fades() {
        let pts = vec![Vec3::new(200.0, 0.0, 0.0), Vec3::new(800.0, 0.0, 0.0), Vec3::new(800.0, 600.0, 0.0)];
        let mut w = whisp(pts.clone(), Vec3::new(500.0, 0.0, 0.0));
        w.start_next_path();
        let dt = 1.0 / 60.0;
        let mut closest = [f32::MAX; 3];
        let mut alive = true;
        for _ in 0..600 {
            alive = w.tick(dt);
            for (k, p) in pts.iter().enumerate() {
                closest[k] = closest[k].min(w.loc.distance(*p));
            }
            if !alive || !w.regen {
                break;
            }
        }
        assert!(alive);
        assert!(!w.regen, "never finished: next={}", w.position);
        // The straight points are reached; the sharp corner is passed wide
        // and left by KF's overshoot rule (velocity turned away), as the
        // hand-worked flight gives (closest about 404).
        assert!(closest[0] < WHISP_REACHED && closest[1] < WHISP_REACHED, "{closest:?}");
        assert!((closest[2] - 404.0).abs() < 5.0, "{closest:?}");
        // Stops and is gone 1.5 s later.
        let mut ticks = 0;
        while w.tick(dt) {
            ticks += 1;
        }
        assert!((ticks as f32 * dt - WHISP_END_LIFE).abs() < 0.05);
    }

    #[test]
    fn sprites_come_at_90_a_second_and_live_one_and_a_quarter() {
        let mut w = whisp(vec![Vec3::new(5000.0, 0.0, 0.0)], Vec3::ZERO);
        w.start_next_path();
        for _ in 0..30 {
            w.tick(1.0 / 60.0);
        }
        // Half a second: 45 sprites.
        assert_eq!(w.sprites.len(), 45);
        for _ in 0..90 {
            w.tick(1.0 / 60.0);
        }
        // After 2 s only the last 1.25 s worth remain (about 112).
        assert!((110..=114).contains(&w.sprites.len()), "{}", w.sprites.len());
        assert!(w.sprites.iter().all(|s| s.tile < 16));
    }

    #[test]
    fn whisp_dies_at_its_lifespan() {
        let mut w = whisp(vec![Vec3::new(1.0e6, 0.0, 0.0)], Vec3::ZERO);
        w.start_next_path();
        let mut t = 0.0;
        while w.tick(0.1) {
            t += 0.1;
        }
        assert!((t - 9.9_f32).abs() < 0.15, "{t}");
    }
}
