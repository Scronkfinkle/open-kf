//! Actor lighting (DESIGN.md, "Actor lighting (L4)"): zeds, players'
//! bodies, the trader, gore and the first-person weapon lit by the map's
//! own lights and zone ambient, the way UE2 lit mesh actors: a few lights
//! per actor (Actor.MaxLights), each line-checked (bLightingVisibility),
//! summed per vertex with the zone's ambient light.
//!
//! Each lit actor carries an `ActorLight` (its probe point, MaxLights,
//! AmbientGlow, and its light cache); each of its drawn parts a `LitPart`
//! pointing at it. `update_rigs` keeps the cache as UE2 does (below);
//! `apply_vertex_light` lights every vertex and writes the result as vertex
//! colours into the parts' meshes, drawn unlit (texture x colour x K, as
//! the map's baked meshes).
//!
//! Taken from KF's engine (2026-10-08; DESIGN.md, "Actor lighting (L4)",
//! describes the rules; engine-level details in the local RE.md): the light cache
//! and its priority, the 0.35 s line checks and fades, the trace flags,
//! the zone ambient, the hardware light's strength. What is still a guess
//! is marked **Guess**.

use std::collections::HashMap;

use avian3d::parry::query::{Ray, RayCast};
use avian3d::parry::shape::TriMesh;
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use ue_assets::level::MapLight;

use crate::engine::coords::{self, SCALE};
use crate::engine::runlog;

/// The map's light actors, as read at load (map.rs).
#[derive(Resource, Default, Clone)]
pub struct MapLightList(pub Vec<MapLight>);

/// ELightEffect values used here (Engine/Actor.uc).
const LE_STATIC_SPOT: u8 = 8;
const LE_SPOTLIGHT: u8 = 12;
const LE_NON_INCIDENCE: u8 = 13;
const LE_SUNLIGHT: u8 = 20;
const LE_QUADRATIC_NON_INCIDENCE: u8 = 21;

/// How a light fades from its centre (t = 0) to its radius (t = 1).
/// **Guess** which one UE2 uses: fitted against the maps' baked vertex
/// colours (`KF_LIGHT_CALIBRATE=1`, log `light_calibrate`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Falloff {
    /// 1 - t
    Linear,
    /// 1 - t^2
    Quadratic,
    /// (1 - t)^2
    Squared,
    /// (1 - t)^3
    Cubed,
    /// 1 - smoothstep: 2t^3 - 3t^2 + 1
    Smooth,
}

impl Falloff {
    pub const ALL: [Falloff; 5] = [Falloff::Linear, Falloff::Quadratic, Falloff::Squared, Falloff::Cubed, Falloff::Smooth];

    pub fn at(self, t: f32) -> f32 {
        if t >= 1.0 {
            return 0.0;
        }
        let t = t.max(0.0);
        match self {
            Falloff::Linear => 1.0 - t,
            Falloff::Quadratic => 1.0 - t * t,
            Falloff::Squared => (1.0 - t) * (1.0 - t),
            Falloff::Cubed => (1.0 - t).powi(3),
            Falloff::Smooth => 2.0 * t * t * t - 3.0 * t * t + 1.0,
        }
    }
}

/// The falloff used for point and spot lights (see `Falloff`). Fitted
/// on all 33 maps (2026-10-07, KF_LIGHT_CALIBRATE, 30000 baked vertices
/// each): Smooth median correlation 0.855, Linear 0.845, Quadratic 0.833,
/// Squared 0.839, Cubed 0.802. **Guess**: UE2's own formula is native.
pub const FALLOFF: Falloff = Falloff::Smooth;

/// A light's colour: Unreal hue colour x LightBrightness / 255 x this, in
/// the units of the baked mesh colours. Fitted with `FALLOFF` against the
/// baked colours: median best scale over the maps 1.205 (most maps 1.1 to
/// 1.4). (The native colour is FGetHSV(hue, saturation, 255) x
/// LightBrightness / 255, about 0.82 at most, so the fit is 1.5 times
/// that; not explained.)
pub const LIGHT_SCALE: f32 = 1.2;

/// Actors' hardware point and spot lights: colour x 2 x smoothstep x cos
/// (KF's renderer fits the hardware attenuation over the actor's bounding
/// sphere to 2 x smoothstep), with the
/// native colour (`actor_colour_scale`), drawn with the same x2 modulate
/// as the baked meshes. (The baked colours are colour x 0.5 x
/// SampleIntensity.)
pub const ACTOR_POINT_GAIN: f32 = 2.0;
/// Actors' light colour against `LIGHT_SCALE`: the native colour is
/// FGetHSV(hue, saturation, 255) x LightBrightness / 255, 0.8213 at most, while the baked fit
/// gave 1.2. Checked against your KF-WestLondon screenshot (precise.jpg,
/// matched view): the hands and gun measure 30.6 (ours, sunlit) against
/// 30.8 (KF); with the fitted 1.2 they were 44. `KF_LIGHT_ACTOR_SCALE`
/// overrides (testing).
pub fn actor_colour_scale() -> f32 {
    static S: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *S.get_or_init(|| std::env::var("KF_LIGHT_ACTOR_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(0.8213) / LIGHT_SCALE)
}

/// Sunlight on actors: KF gives a directional light colour x 1.75
/// (baked: colour x cos, i.e. 2 x 0.5).
pub const ACTOR_SUN_GAIN: f32 = 1.75;
/// The light cache re-checks a light's line every 0.35 s and fades its
/// share linearly over 0.35 s (UE2's relevant-light cache).
const RECHECK_SECONDS: f64 = 0.35;
/// Lights kept in an actor's cache (the same function: 16 slots).
const CACHE_SLOTS: usize = 16;
/// Hardware lights per actor at most (8; 4 with bDramaticLighting).
const HARDWARE_LIGHTS: usize = 8;
/// An actor's bounding-sphere radius when not given, Unreal units (it
/// widens a light's reach in the priority).
const DEFAULT_RADIUS: f32 = 50.0;
/// Grid cell for sorting lights (Bevy metres; 800 Unreal units).
const CELL: f32 = 16.0;
/// Sunlight: a line this long toward the sun must leave the level (metres).
const SUN_TRACE: f32 = 1000.0;

/// How far short of a light its line check stops (metres): hits closer
/// than this to the light are ignored. `KF_LIGHT_STOP` (Unreal units)
/// overrides it, for testing.
fn stop_short() -> f32 {
    static STOP: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *STOP.get_or_init(|| std::env::var("KF_LIGHT_STOP").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(STOP_SHORT) * SCALE)
}

/// Default for `stop_short`, Unreal units.
const STOP_SHORT: f32 = 5.0;

/// A spot's strength at cos `c` from its axis: ((c - edge) / (inner -
/// edge))^2, 0 outside (SampleIntensity squares the ramp).
fn spot_factor(c: f32, cos_outer: f32, cos_inner: f32) -> f32 {
    let r = ((c - cos_outer) / (cos_inner - cos_outer).max(1e-4)).clamp(0.0, 1.0);
    r * r
}

/// UE2's light radius in world units: 25 x (LightRadius + 1) (as UE1;
/// also used by the flashlight glow).
pub fn world_radius(r: f32) -> f32 {
    25.0 * (r + 1.0)
}

/// Unreal's hue / saturation colour at full value (UE1 FGetHSV without its
/// brightness curve): hue 0 red, 85 green, 170 blue; saturation runs
/// backwards (255 = white, 0 = the pure hue). Largest channel 1.
pub fn hue_colour(hue: u8, saturation: u8) -> Vec3 {
    let h = hue as f32;
    let base = if h < 86.0 {
        Vec3::new((85.0 - h) / 85.0, h / 85.0, 0.0)
    } else if h < 171.0 {
        Vec3::new(0.0, (170.0 - h) / 85.0, (h - 85.0) / 85.0)
    } else {
        Vec3::new((h - 170.0) / 85.0, 0.0, (255.0 - h) / 84.0)
    };
    let c = base + saturation as f32 / 255.0 * (Vec3::ONE - base);
    c / c.max_element().max(1e-6)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Point,
    /// Cone around `dir` (Bevy, unit); full inside `cos_inner`, nothing
    /// outside `cos_outer`.
    Spot { dir: Vec3, cos_outer: f32, cos_inner: f32 },
    /// Direction toward the sun (Bevy, unit); no reach limit.
    Sun { to_sun: Vec3 },
}

/// A light as actor lighting sees it (Bevy space).
#[derive(Clone, Debug)]
pub struct Source {
    pub name: String,
    pub class: String,
    pub pos: Vec3,
    /// Colour x strength, in UE2's 0..1 light units (1 = a fully lit
    /// surface before the x2 overbright).
    pub colour: Vec3,
    /// Reach, metres.
    pub radius: f32,
    pub kind: Kind,
    /// False for LE_NonIncidence / LE_QuadraticNonIncidence: the surface
    /// angle does not matter.
    pub incidence: bool,
    pub falloff: Falloff,
    /// The zone the light stands in (map lights; set at load).
    pub zone: Option<usize>,
    /// bDynamicLight: not in the baked lighting (calibration skips it).
    pub dynamic: bool,
    /// LightBrightness (the cache priority uses it).
    pub brightness: f32,
    /// bStatic: only static lights are line-checked for actors (the
    /// relevant-light cache); others (the flashlight's glow, TriggerLights) light
    /// through walls.
    pub line_check: bool,
}

impl Source {
    pub fn from_map(l: &MapLight) -> Option<Source> {
        if l.brightness <= 0.0 || l.special_lit {
            return None;
        }
        let colour = hue_colour(l.hue, l.saturation) * (l.brightness / 255.0) * LIGHT_SCALE;
        let forward = coords::dir((coords::ue_rotation_matrix(l.rotation) * Vec3::X).to_array()).normalize_or_zero();
        let kind = match l.effect {
            LE_SUNLIGHT => Kind::Sun { to_sun: -forward },
            // LE_Spotlight and LE_StaticSpot (UE2's light sampling): edge at cos = 1 -
            // LightCone / 256, strength ((cos - edge) / (1 - edge))^2.
            LE_SPOTLIGHT | LE_STATIC_SPOT => {
                let cos_outer = 1.0 - l.cone as f32 / 256.0;
                Kind::Spot { dir: forward, cos_outer, cos_inner: 1.0 }
            }
            _ => Kind::Point,
        };
        Some(Source {
            name: l.name.clone(),
            class: l.class.clone(),
            pos: coords::pos(l.location),
            colour,
            radius: world_radius(l.radius) * SCALE,
            kind,
            incidence: !matches!(l.effect, LE_NON_INCIDENCE | LE_QUADRATIC_NON_INCIDENCE),
            falloff: if l.effect == LE_QUADRATIC_NON_INCIDENCE { Falloff::Quadratic } else { FALLOFF },
            zone: None,
            dynamic: l.dynamic,
            brightness: l.brightness,
            line_check: l.is_static,
        })
    }

    /// Strength (largest colour channel) and direction toward the light at
    /// `p`, before any line check; None when out of reach.
    pub fn reach(&self, p: Vec3) -> Option<(f32, Vec3)> {
        match self.kind {
            Kind::Sun { to_sun } => Some((self.colour.max_element(), to_sun)),
            _ => {
                let d = self.pos - p;
                let dist = d.length();
                if dist >= self.radius {
                    return None;
                }
                let to = d / dist.max(1e-4);
                let mut f = self.falloff.at(dist / self.radius);
                if let Kind::Spot { dir, cos_outer, cos_inner } = self.kind {
                    f *= spot_factor(dir.dot(-to), cos_outer, cos_inner);
                }
                (f > 1e-3).then_some((f * self.colour.max_element(), to))
            }
        }
    }
}

/// Map lights sorted for lookup, and the walls that block them.
#[derive(Resource, Default)]
pub struct ActorLights {
    pub sources: Vec<Source>,
    grid: HashMap<(i32, i32), Vec<u32>>,
    suns: Vec<u32>,
    /// What blocks light in actors' line checks: the BSP walls (no sky
    /// backdrop).
    occluder: Option<TriMesh>,
    /// For KF_LIGHT_CALIBRATE: the BSP walls alone, and with the
    /// shadow-casting static meshes (as the baked lighting saw them).
    bsp_only: Option<TriMesh>,
    with_meshes: Option<TriMesh>,
    /// For logs: BSP triangle count, and which mesh actor owns the
    /// triangles after it (first triangle, name).
    bsp_triangles: u32,
    mesh_owners: Vec<(u32, String)>,
}

impl ActorLights {
    pub fn new(sources: Vec<Source>, occluder: Option<TriMesh>) -> Self {
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        let mut suns = Vec::new();
        for (i, s) in sources.iter().enumerate() {
            if matches!(s.kind, Kind::Sun { .. }) {
                suns.push(i as u32);
                continue;
            }
            let (lo, hi) = (s.pos - Vec3::splat(s.radius), s.pos + Vec3::splat(s.radius));
            for cx in (lo.x / CELL).floor() as i32..=(hi.x / CELL).floor() as i32 {
                for cz in (lo.z / CELL).floor() as i32..=(hi.z / CELL).floor() as i32 {
                    grid.entry((cx, cz)).or_default().push(i as u32);
                }
            }
        }
        ActorLights { sources, grid, suns, occluder, bsp_only: None, with_meshes: None, bsp_triangles: 0, mesh_owners: Vec::new() }
    }

    /// Lights that may reach a sphere of radius `r` around `p` (cells
    /// touched by the sphere; duplicates removed).
    fn candidates_within(&self, p: Vec3, r: f32) -> Vec<u32> {
        let (lo, hi) = (p - Vec3::splat(r), p + Vec3::splat(r));
        let mut out: Vec<u32> = Vec::new();
        for cx in (lo.x / CELL).floor() as i32..=(hi.x / CELL).floor() as i32 {
            for cz in (lo.z / CELL).floor() as i32..=(hi.z / CELL).floor() as i32 {
                out.extend(self.grid.get(&(cx, cz)).into_iter().flatten().copied());
            }
        }
        out.sort_unstable();
        out.dedup();
        out.extend(self.suns.iter().copied());
        out
    }

    /// Lights that may reach `p`.
    fn candidates(&self, p: Vec3) -> impl Iterator<Item = u32> + '_ {
        let cell = ((p.x / CELL).floor() as i32, (p.z / CELL).floor() as i32);
        self.grid.get(&cell).into_iter().flatten().copied().chain(self.suns.iter().copied())
    }

    fn clear_in(mesh: Option<&TriMesh>, a: Vec3, b: Vec3) -> bool {
        let Some(mesh) = mesh else { return true };
        let d = b - a;
        let len = d.length();
        if len < 1e-4 {
            return true;
        }
        let dir = d / len;
        let ray = Ray::new(a.to_array().into(), dir.to_array().into());
        // Stop short of the light: lamps sit against walls and inside
        // their fixtures.
        !mesh.intersects_local_ray(&ray, (len - stop_short()).max(0.0))
    }

    /// Whether the light reaches `p` past the walls.
    fn visible(&self, s: &Source, p: Vec3) -> bool {
        self.visible_in(self.occluder.as_ref(), s, p)
    }

    fn visible_in(&self, mesh: Option<&TriMesh>, s: &Source, p: Vec3) -> bool {
        match s.kind {
            Kind::Sun { to_sun } => Self::clear_in(mesh, p, p + to_sun * SUN_TRACE),
            _ => Self::clear_in(mesh, p, s.pos),
        }
    }
}

/// Lights that are not map actors, refreshed every frame by their owners
/// (the flashlight: weapons/flashlight.rs). Not line-checked (not bStatic)
/// and not counted against MaxLights (**guess**: the projector is a
/// projector in KF, not a light).
#[derive(Resource, Default)]
pub struct DynamicLights(pub Vec<Source>);

/// One light in an actor's cache.
#[derive(Clone, Debug)]
pub struct Slot {
    /// Index into `ActorLights::sources`.
    pub src: u32,
    /// Priority (see `priority`).
    pub key: f32,
    /// The line check now and at the one before; the share fades from the
    /// old to the new over `RECHECK_SECONDS`.
    pub visible: bool,
    pub was_visible: bool,
    /// When the line was last checked (seconds of game time).
    pub checked_at: f64,
    /// Current share, 0..1.
    pub alpha: f32,
}

/// A light as used this frame: the source (map light or dynamic) and its
/// share.
#[derive(Clone, Debug)]
pub struct Used {
    pub src: Source,
    pub alpha: f32,
}

/// A lit actor: where it samples light, how many lights it takes, and its
/// light cache.
#[derive(Component, Debug)]
pub struct ActorLight {
    /// For logs ("zed 3", "player body", ...).
    pub label: String,
    /// Offset from the entity's position to its centre (Bevy world).
    pub offset: Vec3,
    /// Actor.MaxLights (capped at `HARDWARE_LIGHTS`).
    pub max_lights: usize,
    /// Actor.AmbientGlow (0..254).
    pub glow: u8,
    /// Bounding-sphere radius, Unreal units.
    pub radius: f32,
    pub slots: Vec<Slot>,
    /// The lights in use this frame (map lights, then dynamic ones).
    pub used: Vec<Used>,
    pub ambient: Vec3,
    pub zone: usize,
    started: bool,
    /// Bumped when the light set changes (static parts relight then).
    pub version: u32,
    /// Light sum at the centre last frame (to bump `version`).
    last_sum: Vec3,
}

impl ActorLight {
    pub fn new(label: impl Into<String>, offset: Vec3, max_lights: usize, glow: u8) -> Self {
        ActorLight {
            label: label.into(),
            offset,
            max_lights,
            glow,
            radius: DEFAULT_RADIUS,
            slots: Vec::new(),
            used: Vec::new(),
            ambient: Vec3::ZERO,
            zone: 0,
            started: false,
            version: 0,
            last_sum: Vec3::ZERO,
        }
    }

    /// With a bounding-sphere radius (Unreal units).
    pub fn with_radius(mut self, r: f32) -> Self {
        self.radius = r;
        self
    }

    /// The light (UE2 units, before clamping) at `p` on a surface facing
    /// `normal` (Bevy world).
    pub fn light_at(&self, p: Vec3, normal: Vec3) -> Vec3 {
        let mut c = self.ambient;
        for u in &self.used {
            c += actor_light_from(&u.src, p, normal) * u.alpha;
        }
        c
    }

    /// Largest channel of ambient plus every light at full incidence at
    /// the centre `p`: how bright the brightest side can be.
    pub fn peak(&self, p: Vec3) -> f32 {
        let mut c = self.ambient;
        for u in &self.used {
            if let Some((_, to)) = u.src.reach(p) {
                c += actor_light_from(&u.src, p, to) * u.alpha;
            }
        }
        c.max_element()
    }
}

/// One hardware light on an actor's vertex at `p` with normal `n` (Bevy
/// world): point and spot lights colour x 2 x falloff x cos, sunlight
/// colour x 1.75 x cos (see `ACTOR_POINT_GAIN`). D3D lights always use the
/// surface angle, so LE_NonIncidence makes no difference on actors.
pub fn actor_light_from(s: &Source, p: Vec3, n: Vec3) -> Vec3 {
    let colour = s.colour * actor_colour_scale();
    match s.kind {
        Kind::Sun { to_sun } => colour * (ACTOR_SUN_GAIN * n.dot(to_sun).max(0.0)),
        _ => {
            let d = s.pos - p;
            let dist = d.length();
            if dist >= s.radius {
                return Vec3::ZERO;
            }
            let to = d / dist.max(1e-4);
            let mut f = s.falloff.at(dist / s.radius);
            if let Kind::Spot { dir, cos_outer, cos_inner } = s.kind {
                f *= spot_factor(dir.dot(-to), cos_outer, cos_inner);
            }
            colour * (ACTOR_POINT_GAIN * f * n.dot(to).max(0.0))
        }
    }
}

/// The cache priority of a light for an actor at `p` with bounding radius
/// `r`: sunlight first; point lights
/// LightBrightness x (1 - d^2 / (R + r)^2); spotlights the same if the
/// actor is inside the cone, else 0. Lights at 0 or below are not used.
pub fn priority(s: &Source, p: Vec3, r: f32) -> f32 {
    match s.kind {
        Kind::Sun { .. } => f32::MAX,
        _ => {
            let d = s.pos - p;
            if let Kind::Spot { dir, cos_outer, .. } = s.kind
                && dir.dot(-d.normalize_or_zero()) <= cos_outer
            {
                return 0.0;
            }
            let reach = s.radius + r;
            (1.0 - d.length_squared() / (reach * reach)) * s.brightness
        }
    }
}

/// A drawn part of a lit actor (its mesh gets the vertex colours).
#[derive(Component, Debug, Clone, Copy)]
pub struct LitPart {
    pub owner: Entity,
    /// Posed or moved every frame (vertex colours redone every frame);
    /// else only when the light set changes.
    pub animated: bool,
    /// The part's own material. When set and the part shows another one
    /// (the scope lens view, the welder screen: self-lit displays; a
    /// Stalker's cloak), it is drawn as before actor lighting (texture x 1,
    /// no light).
    pub own: Option<AssetId<StandardMaterial>>,
}

/// Version of the owner's light set last written into a static part.
#[derive(Component, Default)]
struct LitVersion(Option<u32>);

/// Lit-actor statistics for the once-a-second log.
#[derive(Resource, Default)]
struct LightStats {
    checks: usize,
    pick_us: f64,
    apply_us: f64,
    vertices: usize,
    parts: usize,
    frames: usize,
    /// Per actor: sum of each drawn vertex's light (largest channel,
    /// clamped) and the vertex count.
    drawn: HashMap<Entity, (f64, usize)>,
}

pub struct ActorLightPlugin;

impl Plugin for ActorLightPlugin {
    fn build(&self, app: &mut App) {
        use crate::world::map_change::MapResourceExt;
        app.init_resource::<DynamicLights>()
            .init_resource::<LightStats>()
            // Per map (world/map_change.rs): the map's lights, and the
            // light caches of actors that outlive the map (the player's
            // body and weapon), which index the old map's lights.
            .remove_on_map_unload::<MapLightList>()
            .remove_on_map_unload::<ActorLights>()
            .reset_on_map_unload::<LightStats>()
            .add_systems(crate::world::map_change::MapUnload, forget_map_lights)
            .add_systems(crate::world::map_change::PostMapLoad, build_actor_lights)
            .add_systems(Update, (calibrate, survey))
            .add_systems(
                PostUpdate,
                (update_rigs, apply_vertex_light, log_actor_light)
                    .chain()
                    .after(bevy::transform::TransformSystems::Propagate)
                    // After the cameras' visibility checks: only parts on
                    // screen get their vertex colours.
                    .after(bevy::camera::visibility::VisibilitySystems::CheckVisibility),
            );
    }
}

/// A map change: actors kept across it start their light cache again.
fn forget_map_lights(mut actors: Query<&mut ActorLight>) {
    for mut a in &mut actors {
        a.slots.clear();
        a.used.clear();
        a.started = false;
        a.version = a.version.wrapping_add(1);
    }
}

/// Builds `ActorLights` from the map's lights and the light-blocking walls
/// (after the map loader has run).
fn build_actor_lights(
    mut commands: Commands,
    list: Option<Res<MapLightList>>,
    zones: Option<Res<crate::world::zones::Zones>>,
    mut geo: ResMut<crate::world::collision::CollisionGeometry>,
) {
    let Some(list) = list else { return };
    let soup = std::mem::take(&mut geo.light_bsp);
    let triangles = soup.triangles.len() + geo.light_meshes.triangles.len();
    let occluder = if soup.triangles.is_empty() {
        None
    } else {
        TriMesh::new(soup.vertices.iter().map(|v| v.to_array().into()).collect(), soup.triangles.clone()).ok()
    };
    let meshes = std::mem::take(&mut geo.light_meshes);
    let mesh_owners = std::mem::take(&mut geo.light_mesh_owners);
    let bsp_triangles = soup.triangles.len() as u32;
    let mesh_occluder = (!meshes.triangles.is_empty()).then(|| {
        let base = soup.vertices.len() as u32;
        let vertices: Vec<_> = soup.vertices.iter().chain(&meshes.vertices).map(|v| v.to_array().into()).collect();
        let tris: Vec<[u32; 3]> = soup.triangles.iter().copied().chain(meshes.triangles.iter().map(|t| t.map(|i| i + base))).collect();
        TriMesh::new(vertices, tris).ok()
    }).flatten();
    let mut sources: Vec<Source> = list.0.iter().filter_map(Source::from_map).collect();
    if let Some(z) = zones.as_deref() {
        for s in &mut sources {
            let u = s.pos / SCALE;
            s.zone = Some(z.bsp.point_zone([-u.z, u.x, u.y]));
        }
    }
    let mut by_class: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut by_effect: std::collections::BTreeMap<u8, usize> = std::collections::BTreeMap::new();
    for l in &list.0 {
        *by_class.entry(l.class.clone()).or_default() += 1;
        *by_effect.entry(l.effect).or_default() += 1;
    }
    let skipped = list.0.len() - sources.len();
    let mut by_type: std::collections::BTreeMap<u8, usize> = std::collections::BTreeMap::new();
    for l in &list.0 {
        *by_type.entry(l.light_type).or_default() += 1;
    }
    let dynamic = list.0.iter().filter(|l| l.dynamic).count();
    let not_static = list.0.iter().filter(|l| !l.is_static).count();
    let mut strongest: Vec<&MapLight> = list.0.iter().collect();
    strongest.sort_by(|a, b| (b.brightness * b.radius).total_cmp(&(a.brightness * a.radius)));
    let strongest: Vec<String> = strongest
        .iter()
        .take(6)
        .map(|l| format!("{}:{}:b{:.0}:r{:.0}:t{}:e{}:h{}:s{}{}", l.name, l.class, l.brightness, l.radius, l.light_type, l.effect, l.hue, l.saturation, if l.dynamic { ":dyn" } else { "" }))
        .collect();
    let suns = sources.iter().filter(|s| matches!(s.kind, Kind::Sun { .. })).count();
    let max_radius = sources.iter().filter(|s| !matches!(s.kind, Kind::Sun { .. })).map(|s| s.radius / SCALE).fold(0.0, f32::max);
    runlog::kv(
        "map_lights",
        &format!(
            "lights={} used={} skipped_dark_or_special={skipped} suns={suns} max_radius_unreal={max_radius:.0} by_class={by_class:?} by_effect={by_effect:?} occluder_triangles={triangles} occluder={} falloff={FALLOFF:?} scale={LIGHT_SCALE} by_type={by_type:?} dynamic={dynamic} not_static={not_static} strongest=[{}]",
            list.0.len(),
            sources.len(),
            occluder.is_some(),
            strongest.join(" ")
        ),
    );
    // Actors' line checks see the BSP and bShadowCast actors (the
    // shadow-casting static meshes): TRACE_Level | TRACE_ShadowCast (see
    // `update_cache`). KF_LIGHT_BSP_ONLY=1 (for comparing): the BSP only.
    let blocking = if std::env::var_os("KF_LIGHT_BSP_ONLY").is_some() { occluder.clone() } else { mesh_occluder.clone().or(occluder.clone()) };
    let mut lights = ActorLights::new(sources, blocking);
    lights.bsp_only = occluder.clone();
    lights.bsp_triangles = bsp_triangles;
    lights.mesh_owners = mesh_owners;
    lights.with_meshes = mesh_occluder.or(occluder);
    commands.insert_resource(lights);
}


/// Unreal's FGetHSV: hue colour at the given
/// saturation, times a brightness curve of the value V: x = V x 1.4 / 255,
/// x / (sqrt(x) + 0.01) x 0.7, clamped to 0..1 (V 255: 0.82; V 2: 0.067).
pub fn fget_hsv(hue: u8, saturation: u8, value: u8) -> Vec3 {
    let x = value as f32 * 1.4 / 255.0;
    let b = (x / (x.sqrt() + 0.01) * 0.7).clamp(0.0, 1.0);
    let h = hue as f32;
    let base = if h < 86.0 {
        Vec3::new((85.0 - h) / 85.0, h / 85.0, 0.0)
    } else if h < 171.0 {
        Vec3::new(0.0, (170.0 - h) / 85.0, (h - 85.0) / 85.0)
    } else {
        Vec3::new((h - 170.0) / 85.0, 0.0, (255.0 - h) / 84.0)
    };
    (base + saturation as f32 / 255.0 * (Vec3::ONE - base)) * b
}

/// An actor's ambient light: the zone's AmbientVector (FGetHSV(AmbientHue,
/// AmbientSaturation, AmbientBrightness), set in AZoneInfo::PostEditChange)
/// plus AmbientGlow / 255 as grey. UE2 takes the largest per channel over
/// all zones the actor's box touches; we take the zone at its centre.
pub fn ambient_of(zones: Option<&crate::world::zones::Zones>, p: Vec3, glow: u8) -> (usize, Vec3) {
    let g = Vec3::splat(glow as f32 / 255.0);
    let Some(z) = zones else { return (0, g) };
    let u = p / SCALE;
    let zone = z.bsp.point_zone([-u.z, u.x, u.y]);
    let ambient = match z.zones.get(zone) {
        // The saved AmbientVector is what the game uses (KF-WestLondon:
        // equal to our FGetHSV where both exist; some zones keep a vector
        // with AmbientBrightness back at 0).
        Some(zf) => zf.ambient_vector.map_or_else(|| fget_hsv(zf.ambient[1], zf.ambient[2], zf.ambient[0]), Vec3::from_array),
        None => Vec3::ZERO,
    };
    (zone, (ambient + g).min(Vec3::ONE))
}

/// UE2's relevant-light cache for one actor at `p`, at game time `now`:
/// - every light reaching the actor gets a priority (`priority`); the
///   cache keeps the 16 best (a new light replaces the lowest);
/// - each cached light's line is checked when 0.35 s have passed since its
///   last check (a new light at once), only for bStatic lights, with
///   a zero-extent line check that stops at the first hit against the BSP
///   and actors with bShadowCast, i.e. the shadow-casting static meshes;
/// - its share fades linearly from the old result to the new over 0.35 s
///   (so a new light fades in);
/// - in priority order, lights that are or were visible are used, up to
///   MaxLights (at most 8).
///
/// A Sunlight lights only its own zone: **found by fitting** the baked
/// colours (KF_LIGHT_CALIBRATE; KF-SirensBelch correlation 0.87 against
/// 0.46), not seen in the code.
pub fn update_cache(a: &mut ActorLight, lights: &ActorLights, p: Vec3, zone: usize, now: f64, checks: &mut usize) {
    let r = a.radius * SCALE;
    // New priorities for cached lights; drop those no longer relevant.
    for slot in &mut a.slots {
        slot.key = priority(&lights.sources[slot.src as usize], p, r);
    }
    a.slots.retain(|s| s.key > 0.0);
    for i in lights.candidates_within(p, r) {
        let s = &lights.sources[i as usize];
        if matches!(s.kind, Kind::Sun { .. }) && s.zone.is_some_and(|z| z != zone) {
            continue;
        }
        if a.slots.iter().any(|sl| sl.src == i) {
            continue;
        }
        let key = priority(s, p, r);
        if key <= 0.0 {
            continue;
        }
        let slot = Slot { src: i, key, visible: false, was_visible: false, checked_at: now - RECHECK_SECONDS, alpha: 0.0 };
        if a.slots.len() < CACHE_SLOTS {
            a.slots.push(slot);
        } else if let Some(low) = a.slots.iter_mut().min_by(|x, y| x.key.total_cmp(&y.key))
            && low.key < key
        {
            *low = slot;
        }
    }
    a.slots.sort_by(|x, y| y.key.total_cmp(&x.key));
    for slot in &mut a.slots {
        if now - slot.checked_at >= RECHECK_SECONDS - 1e-6 {
            let s = &lights.sources[slot.src as usize];
            slot.was_visible = slot.visible;
            slot.visible = if s.line_check {
                *checks += 1;
                lights.visible(s, p)
            } else {
                true
            };
            // As UE2: the next check is due 0.35 s after this frame.
            slot.checked_at = now;
        }
        let (from, to) = (slot.was_visible as u8 as f32, slot.visible as u8 as f32);
        slot.alpha = from + (to - from) * ((now - slot.checked_at) / RECHECK_SECONDS).clamp(0.0, 1.0) as f32;
    }
    let max = a.max_lights.min(HARDWARE_LIGHTS);
    a.used = a
        .slots
        .iter()
        .filter(|s| s.visible || s.was_visible)
        .take(max)
        .map(|s| Used { src: lights.sources[s.src as usize].clone(), alpha: s.alpha })
        .collect();
}

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn update_rigs(
    time: Res<Time>,
    lights: Option<Res<ActorLights>>,
    zones: Option<Res<crate::world::zones::Zones>>,
    dynamic: Res<DynamicLights>,
    mut stats: ResMut<LightStats>,
    mut actors: Query<(&GlobalTransform, &mut ActorLight)>,
) {
    let Some(lights) = lights else { return };
    let started = std::time::Instant::now();
    let now = time.elapsed_secs_f64();
    for (gt, mut a) in &mut actors {
        let a = &mut *a;
        let p = gt.translation() + a.offset;
        let (zone, ambient) = ambient_of(zones.as_deref(), p, a.glow);
        a.zone = zone;
        a.ambient = ambient;
        update_cache(a, &lights, p, zone, now, &mut stats.checks);
        for s in &dynamic.0 {
            if s.reach(p).is_some() {
                a.used.push(Used { src: s.clone(), alpha: 1.0 });
            }
        }
        // Static parts relight when the light at the centre changes.
        let sum = a.ambient + a.used.iter().map(|u| u.src.reach(p).map_or(Vec3::ZERO, |(k, _)| Vec3::splat(k * u.alpha))).sum::<Vec3>();
        if !a.started || (sum - a.last_sum).abs().max_element() > 1.0 / 512.0 {
            a.version = a.version.wrapping_add(1);
            a.last_sum = sum;
        }
        a.started = true;
    }
    stats.pick_us += started.elapsed().as_secs_f64() * 1e6;
}

/// Steps in the gamma-to-linear table.
const LUT_STEPS: usize = 1024;

/// UE2 light 0..1 (gamma) to linear x K, tabulated (a power per channel
/// per vertex cost about 2 ms a frame).
fn linear_table(k_lin: f32) -> Vec<f32> {
    (0..=LUT_STEPS).map(|i| Color::srgb(i as f32 / LUT_STEPS as f32, 0.0, 0.0).to_linear().red * k_lin).collect()
}

/// UE2 light (clamped to 0..1) to the linear vertex colour that gives
/// texture x light x K on an unlit material (as the baked meshes).
fn vertex_colour(c: Vec3, table: &[f32]) -> [f32; 4] {
    let at = |v: f32| table[(v.clamp(0.0, 1.0) * LUT_STEPS as f32 + 0.5) as usize];
    [at(c.x), at(c.y), at(c.z), 1.0]
}

#[allow(clippy::type_complexity)] // Bevy query
fn apply_vertex_light(
    actors: Query<&ActorLight>,
    mut parts: Query<(&LitPart, &Mesh3d, &GlobalTransform, &ViewVisibility, Option<&mut LitVersion>, Entity, &MeshMaterial3d<StandardMaterial>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut stats: ResMut<LightStats>,
    mut commands: Commands,
    mut table: Local<Option<Vec<f32>>>,
) {
    let started = std::time::Instant::now();
    let table = table.get_or_insert_with(|| linear_table(crate::render::lighting::brightness_linear()));
    // KF_ACTOR_MASK=1: lit actors drawn saturated magenta, so a screenshot
    // shows which pixels are actors (for measuring them against the
    // world behind).
    let mask = std::env::var_os("KF_ACTOR_MASK").is_some();
    for (part, mesh3d, gt, vis, version, entity, material) in &mut parts {
        if !vis.get() {
            continue;
        }
        if part.own.is_some_and(|own| own != material.0.id()) {
            let Some(n) = meshes.get(&mesh3d.0).map(|m| m.count_vertices()) else { continue };
            if let Some(mut mesh) = meshes.get_mut(&mesh3d.0) {
                mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32, 1.0, 1.0, 1.0]; n]);
            }
            continue;
        }
        let Ok(a) = actors.get(part.owner) else { continue };
        if !part.animated {
            match version {
                Some(mut v) => {
                    if v.0 == Some(a.version) {
                        continue;
                    }
                    v.0 = Some(a.version);
                }
                None => {
                    commands.entity(entity).insert(LitVersion(Some(a.version)));
                }
            }
        }
        // Per vertex, in world space: D3D's lights fall off per vertex.
        let affine = gt.affine();
        let rot = gt.rotation();
        let Some(mesh) = meshes.get(&mesh3d.0) else { continue };
        let (Some(VertexAttributeValues::Float32x3(normals)), Some(VertexAttributeValues::Float32x3(positions))) =
            (mesh.attribute(Mesh::ATTRIBUTE_NORMAL), mesh.attribute(Mesh::ATTRIBUTE_POSITION))
        else {
            continue;
        };
        let mut sum = 0.0f64;
        let colours: Vec<[f32; 4]> = normals
            .iter()
            .zip(positions)
            .map(|(n, v)| {
                let n = rot * Vec3::from_array(*n);
                let p = affine.transform_point3(Vec3::from_array(*v));
                let c = a.light_at(p, n);
                sum += c.clamp(Vec3::ZERO, Vec3::ONE).max_element() as f64;
                if mask { [64.0, 0.0, 64.0, 1.0] } else { vertex_colour(c, table) }
            })
            .collect();
        let d = stats.drawn.entry(part.owner).or_default();
        d.0 += sum;
        d.1 += colours.len();
        stats.vertices += colours.len();
        stats.parts += 1;
        if let Some(mut mesh) = meshes.get_mut(&mesh3d.0) {
            mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
        }
    }
    stats.apply_us += started.elapsed().as_secs_f64() * 1e6;
    stats.frames += 1;
}

/// Once a second: each lit actor's zone, ambient and lights, and timings.
fn log_actor_light(
    time: Res<Time>,
    lights: Option<Res<ActorLights>>,
    mut stats: ResMut<LightStats>,
    actors: Query<(Entity, &GlobalTransform, &ActorLight)>,
    mut timer: Local<f32>,
) {
    *timer += time.delta_secs();
    if *timer < 1.0 {
        return;
    }
    *timer = 0.0;
    let Some(lights) = lights else { return };
    for (e, gt, a) in &actors {
        let drawn = stats.drawn.get(&e).filter(|d| d.1 > 0).map_or("drawn=none".to_string(), |d| format!("drawn_mean={:.3}", d.0 / d.1 as f64));
        let p = gt.translation() + a.offset;
        let u = p / SCALE;
        let list: Vec<String> = a
            .used
            .iter()
            .map(|l| {
                let s = &l.src;
                let dist = if matches!(s.kind, Kind::Sun { .. }) { -1.0 } else { (s.pos - p).length() / SCALE };
                let at = s.reach(p).map_or(Vec3::ZERO, |(_, to)| actor_light_from(s, p, to) * l.alpha);
                format!("{}:{}:d{:.0}:a{:.2}:c({:.2},{:.2},{:.2})", s.name, s.class, dist, l.alpha, at.x, at.y, at.z)
            })
            .collect();
        // Cached lights left out by their line check: where the line is hit
        // (units from the actor / of the whole line), and by the BSP alone.
        let blocked: Vec<String> = a
            .slots
            .iter()
            .filter(|s| !s.visible && !s.was_visible)
            .map(|sl| {
                let s = &lights.sources[sl.src as usize];
                let to = match s.kind {
                    Kind::Sun { to_sun } => p + to_sun * SUN_TRACE,
                    _ => s.pos,
                };
                let hit = |m: Option<&TriMesh>| {
                    let (d, len) = ((to - p).normalize_or_zero(), (to - p).length());
                    m.and_then(|m| m.cast_local_ray(&Ray::new(p.to_array().into(), d.to_array().into()), len, true)).map_or("-".to_string(), |t| format!("{:.0}", t / SCALE))
                };
                let hit_at = lights.occluder.as_ref().and_then(|m| {
                    let d = (to - p).normalize_or_zero();
                    m.cast_local_ray(&Ray::new(p.to_array().into(), d.to_array().into()), (to - p).length(), true).map(|t| (p + d * t) / SCALE)
                });
                let at = hit_at.map_or(String::new(), |h| format!("@({:.0},{:.0},{:.0})", -h.z, h.x, h.y));
                let what = lights.occluder.as_ref().and_then(|m| {
                    let d = (to - p).normalize_or_zero();
                    let hit = m.cast_local_ray_and_get_normal(&Ray::new(p.to_array().into(), d.to_array().into()), (to - p).length(), true)?;
                    let avian3d::parry::shape::FeatureId::Face(f) = hit.feature else { return None };
                    let f = f % m.indices().len().max(1) as u32;
                    if f < lights.bsp_triangles {
                        return Some("bsp".to_string());
                    }
                    let t = f - lights.bsp_triangles;
                    lights.mesh_owners.iter().rev().find(|(first, _)| *first <= t).map(|(_, n)| n.clone())
                });
                let at = format!("{at}[{}]", what.unwrap_or_default());
                format!("{}@{}/{:.0}(bsp:{}){at}", s.name, hit(lights.occluder.as_ref()), (to - p).length() / SCALE, hit(lights.bsp_only.as_ref()))
            })
            .collect();
        runlog::kv(
            "actor_light",
            &format!(
                "actor={} {drawn} at_unreal=({:.0},{:.0},{:.0}) zone={} ambient=({:.3},{:.3},{:.3}) cached={} used={} peak={:.3} top={:.3} chosen=[{}] blocked=[{}]",
                a.label,
                -u.z,
                u.x,
                u.y,
                a.zone,
                a.ambient.x,
                a.ambient.y,
                a.ambient.z,
                a.slots.len(),
                a.used.len(),
                a.peak(p),
                a.light_at(p, Vec3::Y).max_element(),
                list.join(" "),
                blocked.join(" ")
            ),
        );
    }
    if stats.frames > 0 {
        let f = stats.frames as f64;
        runlog::kv(
            "actor_light_cost",
            &format!(
                "actors={} line_checks_per_s={} cache_ms_per_frame={:.3} vertex_ms_per_frame={:.3} parts_per_frame={:.1} vertices_per_frame={:.0}",
                actors.iter().count(),
                stats.checks,
                stats.pick_us / f / 1000.0,
                stats.apply_us / f / 1000.0,
                stats.parts as f64 / f,
                stats.vertices as f64 / f
            ),
        );
    }
    *stats = LightStats::default();
}

/// `KF_LIGHT_SURVEY=1`: once, the light a zed would get (MaxLights 5,
/// lights settled) at every PathNode (about a zed's centre height above
/// the floor): how bright its surface is on average (six directions,
/// clamped as drawn) and at its brightest side, as percentiles, plus the
/// darkest and brightest spots. Log `light_survey`.
fn survey(
    lights: Option<Res<ActorLights>>,
    zones: Option<Res<crate::world::zones::Zones>>,
    geo: Res<crate::world::collision::CollisionGeometry>,
    mut done: Local<bool>,
) {
    let Some(lights) = lights else { return };
    if *done || std::env::var_os("KF_LIGHT_SURVEY").is_none() {
        return;
    }
    *done = true;
    let started = std::time::Instant::now();
    let dirs = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z];
    let mut rows: Vec<(f32, f32, Vec3, usize, usize)> = Vec::new();
    let mut checks = 0;
    for &p in geo.nav_points.iter().take(4000) {
        let (zone, ambient) = ambient_of(zones.as_deref(), p, 0);
        let mut a = ActorLight::new("survey", Vec3::ZERO, 5, 0);
        a.ambient = ambient;
        // Two passes 0.35 s apart: every light checked and fully faded.
        update_cache(&mut a, &lights, p, zone, 0.0, &mut checks);
        update_cache(&mut a, &lights, p, zone, RECHECK_SECONDS, &mut checks);
        update_cache(&mut a, &lights, p, zone, 2.0 * RECHECK_SECONDS, &mut checks);
        let avg = dirs.iter().map(|&d| a.light_at(p, d).clamp(Vec3::ZERO, Vec3::ONE).max_element()).sum::<f32>() / 6.0;
        rows.push((avg, a.peak(p), p, zone, a.used.len()));
    }
    if rows.is_empty() {
        runlog::kv("light_survey", "points=0");
        return;
    }
    let pct = |v: &mut Vec<f32>, q: f32| {
        v.sort_by(f32::total_cmp);
        v[((v.len() - 1) as f32 * q) as usize]
    };
    let mut avgs: Vec<f32> = rows.iter().map(|r| r.0).collect();
    let mut peaks: Vec<f32> = rows.iter().map(|r| r.1).collect();
    let q = |v: &mut Vec<f32>| format!("{:.3}/{:.3}/{:.3}/{:.3}/{:.3}", pct(v, 0.0), pct(v, 0.1), pct(v, 0.5), pct(v, 0.9), pct(v, 1.0));
    let unreal = |p: Vec3| {
        let u = p / SCALE;
        format!("({:.0},{:.0},{:.0})", -u.z, u.x, u.y)
    };
    let darkest = rows.iter().min_by(|a, b| a.0.total_cmp(&b.0)).expect("rows");
    let brightest = rows.iter().max_by(|a, b| a.0.total_cmp(&b.0)).expect("rows");
    let black = rows.iter().filter(|r| r.0 < 0.02).count();
    runlog::kv(
        "light_survey",
        &format!(
            "points={} avg_min/p10/median/p90/max={} peak_min/p10/median/p90/max={} near_black={black} darkest={} avg={:.3} zone={} lights={} brightest={} avg={:.3} zone={} lights={} line_checks={checks} ms={:.1}",
            rows.len(),
            q(&mut avgs),
            q(&mut peaks),
            unreal(darkest.2),
            darkest.0,
            darkest.3,
            darkest.4,
            unreal(brightest.2),
            brightest.0,
            brightest.3,
            brightest.4,
            started.elapsed().as_secs_f64() * 1000.0
        ),
    );
}

/// Baked static-mesh vertices (world position, normal, UE2's stored
/// colour 0..1) for checking our light formula against UE2's own lighting
/// (`KF_LIGHT_CALIBRATE=1`; filled by the map loader).
#[derive(Resource, Default)]
pub struct CalibrationSamples {
    samples: Vec<(Vec3, Vec3, Vec3)>,
    seen: usize,
}

impl CalibrationSamples {
    /// Every 7th vertex of a baked actor's meshes (colours are linear x
    /// `k_lin` there; turned back into UE2's 0..1).
    pub fn add_actor(&mut self, t: &Transform, parts: &[Handle<Mesh>], meshes: &Assets<Mesh>, k_lin: f32) {
        let m = t.compute_affine();
        let normal_m = m.matrix3.inverse().transpose();
        for h in parts {
            let Some(mesh) = meshes.get(h) else { continue };
            let (Some(VertexAttributeValues::Float32x3(pos)), Some(VertexAttributeValues::Float32x3(nor)), Some(VertexAttributeValues::Float32x4(col))) =
                (mesh.attribute(Mesh::ATTRIBUTE_POSITION), mesh.attribute(Mesh::ATTRIBUTE_NORMAL), mesh.attribute(Mesh::ATTRIBUTE_COLOR))
            else {
                continue;
            };
            for ((p, n), c) in pos.iter().zip(nor).zip(col) {
                self.seen += 1;
                if !self.seen.is_multiple_of(7) || self.samples.len() >= 30_000 {
                    continue;
                }
                let gamma = |v: f32| Color::linear_rgb(v / k_lin, 0.0, 0.0).to_srgba().red;
                let n = (normal_m * Vec3::from_array(*n)).normalize_or_zero();
                self.samples.push((m.transform_point3(Vec3::from_array(*p)), n, Vec3::new(gamma(c[0]), gamma(c[1]), gamma(c[2]))));
            }
        }
    }
}

/// Once: our formula (every light, no MaxLights limit, as the editor's
/// static lighting) at each sampled vertex, for each falloff and each way
/// of checking walls (none, BSP, BSP and static meshes); the best scale
/// (least squares over unsaturated vertices) and how well it fits, for all
/// vertices, those the sun reaches and those it does not. Log
/// `light_calibrate`.
fn calibrate(mut commands: Commands, samples: Option<Res<CalibrationSamples>>, lights: Option<Res<ActorLights>>, zones: Option<Res<crate::world::zones::Zones>>) {
    let (Some(samples), Some(lights)) = (samples, lights) else { return };
    commands.remove_resource::<CalibrationSamples>();
    const N: usize = Falloff::ALL.len();
    const MODES: [&str; 5] = ["no_checks", "bsp", "bsp_and_meshes", "bsp_meshes_sun_own_zone", "bsp_meshes_all_own_zone"];
    const SUBSETS: [&str; 3] = ["all", "sun", "no_sun"];
    // (falloff, mode) -> prediction per sample; subset flags per sample.
    let mut rows: Vec<(Vec<[Vec3; 5]>, Vec3, bool)> = Vec::new();
    let mut ambient_sum = 0.0f64;
    for &(pos, n, baked) in &samples.samples {
        if baked.max_element() >= 0.97 {
            continue; // saturated: the true light is unknown
        }
        let p = pos + n * (2.0 * SCALE);
        let (zone, ambient) = ambient_of(zones.as_deref(), p, 0);
        ambient_sum += ambient.max_element() as f64;
        // The baked colours are the lights only (KF's baking adds
        // no ambient).
        let mut pred = vec![[Vec3::ZERO; 5]; N];
        let mut sunlit = false;
        for i in lights.candidates(p) {
            let s = &lights.sources[i as usize];
            if s.dynamic {
                continue; // not baked
            }
            let (to, base, sun) = match s.kind {
                Kind::Sun { to_sun } => (to_sun, [1.0f32; N], true),
                _ => {
                    let d = s.pos - p;
                    let dist = d.length();
                    if dist >= s.radius {
                        continue;
                    }
                    let to = d / dist.max(1e-4);
                    let mut cone = 1.0;
                    if let Kind::Spot { dir, cos_outer, cos_inner } = s.kind {
                        cone = spot_factor(dir.dot(-to), cos_outer, cos_inner);
                    }
                    let t = dist / s.radius;
                    let mut b = [0.0; N];
                    for (k, f) in Falloff::ALL.iter().enumerate() {
                        b[k] = if s.falloff == Falloff::Quadratic { Falloff::Quadratic.at(t) } else { f.at(t) } * cone;
                    }
                    (to, b, false)
                }
            };
            let inc = if s.incidence { n.dot(to).max(0.0) } else { 1.0 };
            if inc <= 0.0 || base.iter().all(|&b| b <= 0.0) {
                continue;
            }
            let meshes = lights.visible_in(lights.with_meshes.as_ref(), s, p);
            let seen = [true, lights.visible_in(lights.bsp_only.as_ref(), s, p), meshes, meshes && (!sun || s.zone == Some(zone)), meshes && s.zone == Some(zone)];
            if sun && seen[1] {
                sunlit = true;
            }
            for k in 0..N {
                let c = s.colour * base[k] * inc;
                for m in 0..5 {
                    if seen[m] {
                        pred[k][m] += c;
                    }
                }
            }
        }
        rows.push((pred, baked, sunlit));
    }
    for (k, f) in Falloff::ALL.iter().enumerate() {
        for (m, mode) in MODES.iter().enumerate() {
            let mut parts = Vec::new();
            for (si, subset) in SUBSETS.iter().enumerate() {
                let pick = |r: &&(Vec<[Vec3; 5]>, Vec3, bool)| si == 0 || (si == 1) == r.2;
                let (mut xy, mut xx, mut yy, mut sx, mut sy, mut cnt) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64, 0usize);
                for r in rows.iter().filter(pick) {
                    cnt += 1;
                    for ch in 0..3 {
                        let (x, y) = (r.0[k][m][ch] as f64, r.1[ch] as f64);
                        xy += x * y;
                        xx += x * x;
                        yy += y * y;
                        sx += x;
                        sy += y;
                    }
                }
                let n = (cnt * 3).max(1) as f64;
                let scale = if xx > 0.0 { xy / xx } else { 0.0 };
                let mut sq = 0.0f64;
                for r in rows.iter().filter(pick) {
                    for ch in 0..3 {
                        let e = r.1[ch] as f64 - (scale * r.0[k][m][ch] as f64).min(1.0);
                        sq += e * e;
                    }
                }
                let cov = xy / n - (sx / n) * (sy / n);
                let corr = cov / ((xx / n - (sx / n).powi(2)) * (yy / n - (sy / n).powi(2))).sqrt().max(1e-12);
                // Median of baked / predicted where both are clearly lit
                // (less pulled down by shadows we miss than the fit).
                let mut ratios: Vec<f32> = rows
                    .iter()
                    .filter(pick)
                    .filter_map(|r| {
                        let (p, b) = (r.0[k][m].max_element(), r.1.max_element());
                        (p > 0.05 && b > 0.03 && b < 0.9).then_some(b / p)
                    })
                    .collect();
                ratios.sort_by(f32::total_cmp);
                let median = ratios.get(ratios.len() / 2).copied().unwrap_or(f32::NAN);
                parts.push(format!("{subset}:n={cnt},scale={scale:.3},rms={:.4},corr={corr:.3},mean_baked={:.4},median_ratio={median:.3}", (sq / n).sqrt(), sy / n));
            }
            runlog::kv("light_calibrate", &format!("falloff={f:?} walls={mode} {}", parts.join(" ")));
        }
    }
    runlog::kv(
        "light_calibrate_summary",
        &format!(
            "samples_total={} unsaturated={} sunlit={} mean_zone_ambient={:.4} mesh_walls={} current_falloff={FALLOFF:?} current_scale={LIGHT_SCALE}",
            samples.samples.len(),
            rows.len(),
            rows.iter().filter(|r| r.2).count(),
            ambient_sum / rows.len().max(1) as f64,
            lights.with_meshes.is_some()
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hue_colour_follows_unreal() {
        // Saturation 255 is white whatever the hue.
        assert!((hue_colour(40, 255) - Vec3::ONE).abs().max_element() < 1e-5);
        // Hue 0, saturation 0: pure red; 170: pure blue.
        assert!((hue_colour(0, 0) - Vec3::X).abs().max_element() < 1e-5);
        assert!((hue_colour(170, 0) - Vec3::Z).abs().max_element() < 1e-5);
    }

    #[test]
    fn vertex_colour_matches_the_baked_meshes() {
        // As map.rs draws baked colours: sRGB to linear, times K linear.
        let k = crate::render::lighting::brightness_linear();
        let table = linear_table(k);
        let c = vertex_colour(Vec3::new(0.5, 1.0, 2.0), &table);
        let want = Color::srgb(0.5, 0.0, 0.0).to_linear().red * k;
        assert!((c[0] - want).abs() < 2e-3 * k);
        // Clamped at 1 before the x K (UE2's overbright).
        assert!((c[1] - k).abs() < 1e-5 && (c[2] - k).abs() < 1e-5);
    }

    #[test]
    fn falloffs_end_at_the_radius() {
        for f in Falloff::ALL {
            assert_eq!(f.at(0.0), 1.0);
            assert_eq!(f.at(1.0), 0.0);
            assert!(f.at(0.5) > 0.0 && f.at(0.5) < 1.0);
        }
    }

    #[test]
    fn point_light_reach_and_walls() {
        let l = MapLight {
            name: "Light1".into(),
            class: "Light".into(),
            location: [0.0, 0.0, 0.0],
            rotation: Default::default(),
            light_type: 1,
            effect: 0,
            hue: 0,
            saturation: 255,
            brightness: 255.0,
            radius: 7.0, // 200 units
            cone: 128,
            dynamic: false,
            is_static: true,
            special_lit: false,
        };
        let s = Source::from_map(&l).unwrap();
        assert!((s.radius / SCALE - 200.0).abs() < 1e-3);
        // 100 units away (Unreal X): half way.
        let p = coords::pos([100.0, 0.0, 0.0]);
        let (strength, to) = s.reach(p).unwrap();
        assert!((strength - FALLOFF.at(0.5) * LIGHT_SCALE).abs() < 1e-4);
        assert!((to - coords::dir([-1.0, 0.0, 0.0])).length() < 1e-4);
        assert!(s.reach(coords::pos([250.0, 0.0, 0.0])).is_none());
        // A wall half way blocks it.
        let wall = 50.0 * SCALE;
        let x = -wall; // Unreal X 50 -> Bevy -Z
        let tri = TriMesh::new(
            vec![[-10.0, -10.0, x].into(), [10.0, -10.0, x].into(), [0.0, 10.0, x].into()],
            vec![[0, 1, 2]],
        )
        .unwrap();
        let lights = ActorLights::new(vec![s], Some(tri));
        let mut checks = 0;
        // Behind the wall: cached but never used.
        let mut a = ActorLight::new("test", Vec3::ZERO, 4, 0);
        for k in 0..4 {
            update_cache(&mut a, &lights, p, 0, k as f64 * RECHECK_SECONDS, &mut checks);
        }
        assert_eq!(a.slots.len(), 1);
        assert!(a.used.is_empty());
        // In front: fades in over 0.35 s, then full.
        let q = coords::pos([-100.0, 0.0, 0.0]);
        let mut b = ActorLight::new("test", Vec3::ZERO, 4, 0);
        update_cache(&mut b, &lights, q, 0, 10.0, &mut checks);
        assert_eq!(b.used.len(), 1);
        assert!(b.used[0].alpha < 0.01);
        update_cache(&mut b, &lights, q, 0, 10.0 + 0.5 * RECHECK_SECONDS, &mut checks);
        assert!((b.used[0].alpha - 0.5).abs() < 0.01);
        update_cache(&mut b, &lights, q, 0, 10.0 + 2.0 * RECHECK_SECONDS, &mut checks);
        assert!((b.used[0].alpha - 1.0).abs() < 1e-4);
        // An actor: 2 x the native colour (0.8213 for white at 255) x falloff.
        let lit = b.light_at(q, coords::dir([1.0, 0.0, 0.0]));
        assert!((lit.x - ACTOR_POINT_GAIN * FALLOFF.at(0.5) * 0.8213).abs() < 1e-3);
    }

    #[test]
    fn fget_hsv_brightness_curve() {
        // White at V 255: 0.82; V 2 (KF-WestLondon zones): about 0.067.
        assert!((fget_hsv(0, 255, 255).x - 0.8213).abs() < 1e-3);
        assert!((fget_hsv(0, 255, 2).x - 0.0669).abs() < 1e-3);
        assert_eq!(fget_hsv(0, 255, 0), Vec3::ZERO);
    }
}
