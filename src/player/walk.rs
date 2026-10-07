//! Walking with collision, after Unreal Engine 2's PHYS_Walking / PHYS_Falling.
//!
//! The player is a cylinder (radius 20, half-height 50 Unreal units) swept
//! through the static colliders with shape casts. Values come from the KF
//! player class defaults (see DESIGN.md, "Walking with collision"), except
//! the step height and walkable slope, which are native engine constants.

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::engine::camera::FlyCamera;
use crate::engine::coords::SCALE;
use crate::player::pawn_collision::{Cylinder, clip_move};
use crate::engine::runlog;

/// Pawn.Bob default (clamped to +-0.01 in CheckBob).
const BOB: f32 = 0.006;

/// Movement values in Unreal units (converted with SCALE when used).
pub(crate) mod kf {
    pub const GROUND_SPEED: f32 = 200.0; // KFHumanPawn
    pub const ACCEL_RATE: f32 = 1000.0; // KFHumanPawn
    pub const JUMP_Z: f32 = 325.0; // KFHumanPawn
    pub const AIR_CONTROL: f32 = 0.15; // KFHumanPawn
    pub const GRAVITY: f32 = 950.0; // PhysicsVolume
    pub const GROUND_FRICTION: f32 = 8.0; // PhysicsVolume
    pub const RADIUS: f32 = 20.0; // KFPawn CollisionRadius
    pub const HALF_HEIGHT: f32 = 50.0; // KFHumanPawn CollisionHeight
    pub const EYE_HEIGHT: f32 = 44.0; // KFPawn BaseEyeHeight, above the centre
    pub const MAX_STEP: f32 = 35.0; // engine constant (not verified in data)
    pub const MIN_FLOOR_NORMAL_Y: f32 = 0.7; // engine constant (not verified in data)
    pub const SKIN: f32 = 0.5; // gap kept from surfaces, to avoid starting casts in contact
}

#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug)]
pub enum MoveMode {
    Fly,
    Walk,
}

/// Command-line driven test settings.
#[derive(Resource, Default, Clone, Copy)]
pub struct WalkSettings {
    pub start_walking: bool,
    /// Hold "forward" for this many seconds, starting 1 s after load.
    pub autowalk: Option<f32>,
}

/// Walking bob shared with the weapon (KFPawn.CheckBob's WalkBob), Bevy space,
/// metres. The camera is offset by it; weapons add BobDamping times it.
#[derive(Resource, Default, Debug, Clone, Copy)]
pub struct ViewBob {
    /// Sideways part (along the view's right axis).
    pub side: Vec3,
    /// Vertical part.
    pub up: f32,
}

#[derive(Component, Default, Debug)]
pub struct Walker {
    /// Cylinder centre, Bevy space (metres).
    pub center: Vec3,
    /// Metres per second.
    pub velocity: Vec3,
    pub on_ground: bool,
    pub floor_normal: Vec3,
    /// Time since walking started, for the autowalk test.
    pub time: f32,
    /// KFPawn.CheckBob state.
    pub bob_time: f32,
    pub applied_bob: f32,
}

/// Momentum on the player from damage (Unreal units: mass x velocity),
/// e.g. the Siren's scream pull. Pawn.TakeDamage: on the ground the upward
/// part is at least 0.4 x its size; divided by Mass (KFPawn 400) and added
/// by AddVelocity (leaves the ground; upward halved above 380 up).
#[derive(Message, Clone, Copy, Debug)]
pub struct PlayerPush {
    pub momentum: Vec3,
    /// Multiplayer host: for another player (net/zeds.rs sends it); None:
    /// this game's own player.
    pub to_peer: Option<u64>,
}

/// Pawn.AddVelocity: added to the velocity as is (not divided by mass);
/// the player starts falling, and upward speed is halved when already
/// rising faster than 380. Used for the shotguns' KickMomentum.
#[derive(Message, Clone, Copy, Debug)]
pub struct PlayerAddVelocity {
    /// Unreal units/s, Unreal axes.
    pub velocity: Vec3,
}

/// KFPawn Mass.
const PLAYER_MASS: f32 = 400.0;

/// The walking systems (the pawn's movement for this frame).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct WalkSystems;

pub struct WalkPlugin;

impl Plugin for WalkPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<PlayerPush>()
            .add_message::<PlayerAddVelocity>()
            .init_resource::<WalkSettings>()
            .init_resource::<ViewBob>()
            .insert_resource(MoveMode::Fly)
            .add_systems(PostStartup, apply_start_mode.after(crate::engine::camera::spawn_camera))
            .add_systems(
                Update,
                (toggle_mode, walk)
                    .chain()
                    .in_set(WalkSystems)
                    .after(crate::engine::camera::look)
                    .before(crate::engine::camera::follow_sky),
            );
    }
}

fn apply_start_mode(
    settings: Res<WalkSettings>,
    mut mode: ResMut<MoveMode>,
    mut commands: Commands,
    cams: Query<(Entity, &Transform), With<FlyCamera>>,
) {
    if settings.start_walking {
        *mode = MoveMode::Walk;
        for (e, t) in &cams {
            commands.entity(e).insert(walker_at(t.translation));
        }
        runlog::kv("move_mode", "mode=Walk reason=start");
    }
}

fn walker_at(eye: Vec3) -> Walker {
    Walker {
        center: eye - Vec3::Y * kf::EYE_HEIGHT * SCALE,
        ..default()
    }
}

fn toggle_mode(
    keys: Res<ButtonInput<KeyCode>>,
    mut mode: ResMut<MoveMode>,
    mut commands: Commands,
    cams: Query<(Entity, &Transform), With<FlyCamera>>,
) {
    if !keys.just_pressed(KeyCode::KeyV) {
        return;
    }
    *mode = match *mode {
        MoveMode::Fly => MoveMode::Walk,
        MoveMode::Walk => MoveMode::Fly,
    };
    for (e, t) in &cams {
        match *mode {
            MoveMode::Walk => {
                commands.entity(e).insert(walker_at(t.translation));
            }
            MoveMode::Fly => {
                commands.entity(e).remove::<Walker>();
            }
        }
    }
    runlog::kv("move_mode", &format!("mode={:?} reason=key", *mode));
}

/// Shape-cast helper around an Unreal-style collision cylinder (player or zed).
pub struct Mover<'a, 'w, 's> {
    spatial: &'a SpatialQuery<'w, 's>,
    shape: Collider,
    /// What blocks this cylinder (`player_filter` or `zed_filter`).
    filter: SpatialQueryFilter,
}

pub struct Hit {
    pub distance: f32,
    pub normal: Vec3,
    pub entity: Entity,
}

impl<'a, 'w, 's> Mover<'a, 'w, 's> {
    /// A cylinder of the given radius and half-height, in Unreal units,
    /// blocked by what `filter` lets through.
    pub fn new(spatial: &'a SpatialQuery<'w, 's>, radius: f32, half_height: f32, filter: SpatialQueryFilter) -> Self {
        Mover {
            spatial,
            shape: Collider::cylinder(radius * SCALE, 2.0 * half_height * SCALE),
            filter,
        }
    }

    /// Keeps a walking cylinder on the floor: looks down a step's height for
    /// walkable ground and rests two skins above it. None = no floor (fall).
    pub fn snap_to_floor(&self, centre: Vec3) -> Option<Vec3> {
        let reach = (kf::MAX_STEP + 2.0 * kf::SKIN) * SCALE;
        match self.cast(centre, Vec3::NEG_Y, reach) {
            Some(floor) if floor.normal.y >= kf::MIN_FLOOR_NORMAL_Y => {
                Some(centre - Vec3::Y * (floor.distance - kf::SKIN * SCALE))
            }
            _ => None,
        }
    }

    /// Sweeps the cylinder from `from` along `dir` for up to `max` metres.
    ///
    /// A surface the cylinder is touching (within the skin) is reported at
    /// distance 0 even when the move runs along it or away from it. Such a
    /// hit is not a block: the sweep is repeated from a skin off that
    /// surface. Without this, a pawn pressed against a wall found no floor
    /// below (walking flickered into falling) and a pawn falling down a
    /// wall in a corner stuck in the air.
    pub fn cast(&self, from: Vec3, dir: Vec3, max: f32) -> Option<Hit> {
        let hit = self.cast_once(from, dir, max)?;
        if hit.distance > 1e-5 || hit.normal.dot(dir) < -0.1 {
            return Some(hit);
        }
        // Moving along the surface. The normal's side is not known (two-sided
        // triangles), so push off to whichever side is free.
        let off = hit.normal * kf::SKIN * SCALE;
        for start in [from + off, from - off] {
            if self.spatial.shape_intersections(&self.shape, start, Quat::IDENTITY, &self.filter).is_empty() {
                return self.cast_once(start, dir, max);
            }
        }
        Some(hit)
    }

    fn cast_once(&self, from: Vec3, dir: Vec3, max: f32) -> Option<Hit> {
        let dir3 = Dir3::new(dir).ok()?;
        let config = ShapeCastConfig {
            max_distance: max,
            target_distance: kf::SKIN * SCALE,
            compute_contact_on_penetration: true,
            ignore_origin_penetration: true,
        };
        self.spatial
            .cast_shape(&self.shape, from, Quat::IDENTITY, dir3, &config, &self.filter)
            .map(|h| {
                // Make the normal face against the motion (trimesh triangles are two-sided).
                let n = if h.normal1.dot(dir) > 0.0 { -h.normal1 } else { h.normal1 };
                Hit {
                    distance: h.distance,
                    normal: n.normalize_or_zero(),
                    entity: h.entity,
                }
            })
    }

    /// Moves by `delta`, sliding along anything hit (up to 4 surfaces).
    /// Returns the new position and the last surface normal hit.
    pub fn slide(&self, mut pos: Vec3, mut delta: Vec3) -> (Vec3, Option<Hit>) {
        let mut last = None;
        for _ in 0..4 {
            let len = delta.length();
            if len < 1e-6 {
                break;
            }
            let dir = delta / len;
            match self.cast(pos, dir, len) {
                None => {
                    pos += delta;
                    break;
                }
                Some(hit) => {
                    pos += dir * hit.distance;
                    let remaining = delta * (1.0 - hit.distance / len);
                    delta = remaining - hit.normal * remaining.dot(hit.normal);
                    last = Some(hit);
                }
            }
        }
        (pos, last)
    }

    /// Ground movement: slide, and if a wall blocks, try stepping up onto it.
    /// Also returns the blocking hit (for logging) when movement was blocked.
    pub fn ground_move(&self, pos: Vec3, delta: Vec3) -> (Vec3, Option<Hit>) {
        let (slid, hit) = self.slide(pos, delta);
        let blocked_by_wall = hit.as_ref().is_some_and(|h| h.normal.y < kf::MIN_FLOOR_NORMAL_Y);
        if !blocked_by_wall {
            return (slid, None);
        }
        // Step up: rise, move forward, come back down onto a walkable floor.
        let step = kf::MAX_STEP * SCALE;
        let rise = self.cast(pos, Vec3::Y, step).map_or(step, |h| h.distance);
        let up = pos + Vec3::Y * rise;
        let (forward, _) = self.slide(up, delta);
        match self.cast(forward, Vec3::NEG_Y, rise + step) {
            Some(down) if down.normal.y >= kf::MIN_FLOOR_NORMAL_Y => {
                let stepped = forward - Vec3::Y * down.distance;
                let progress = |p: Vec3| (p - pos).with_y(0.0).length();
                if progress(stepped) > progress(slid) + 1e-4 {
                    (stepped, None)
                } else {
                    (slid, hit)
                }
            }
            _ => (slid, hit),
        }
    }
}

impl Mover<'_, '_, '_> {
    /// Would a jump clear what blocks `delta`? Rises up to `apex` metres
    /// (less under a ceiling), moves `delta` at that height, and comes down
    /// onto a walkable floor. Returns the landing point if that gets
    /// further than half of `delta` (used by zeds, which jump obstacles
    /// they are blocked by).
    pub fn jump_over(&self, pos: Vec3, delta: Vec3, apex: f32) -> Option<Vec3> {
        let rise = self.cast(pos, Vec3::Y, apex).map_or(apex, |h| h.distance);
        let up = pos + Vec3::Y * rise;
        let (forward, _) = self.slide(up, delta);
        if (forward - up).with_y(0.0).length() < 0.5 * delta.with_y(0.0).length() {
            return None;
        }
        let down = self.cast(forward, Vec3::NEG_Y, rise + kf::MAX_STEP * SCALE)?;
        (down.normal.y >= kf::MIN_FLOOR_NORMAL_Y).then(|| forward - Vec3::Y * down.distance)
    }
}

/// Unreal's CalcVelocity: friction turns velocity toward the input
/// direction; with no input it brakes; then accelerate and clamp.
fn calc_velocity(v: Vec3, accel: Vec3, friction: f32, max_speed: f32, dt: f32) -> Vec3 {
    let mut v = v;
    if accel.length_squared() < 1e-8 {
        let old = v;
        v -= 2.0 * v * dt * friction;
        if old.dot(v) <= 0.0 {
            v = Vec3::ZERO;
        }
    } else {
        let speed = v.length();
        v -= (v - accel.normalize() * speed) * (dt * friction).min(1.0);
    }
    v += accel * dt;
    if v.length() > max_speed {
        v = v.normalize() * max_speed;
    }
    v
}

/// Map features the walk touches: glass panes (bumps) and jump pads (the
/// nav network, and the pad being touched).
type WalkMap<'w, 's> = (
    Query<'w, 's, &'static crate::world::glass::GlassCollider>,
    MessageWriter<'w, crate::world::glass::GlassBump>,
    Option<Res<'w, crate::world::nav::NavNetwork>>,
    Local<'s, Option<usize>>,
);

#[allow(clippy::too_many_arguments)]
fn walk(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<MoveMode>,
    settings: Res<WalkSettings>,
    spatial: SpatialQuery,
    mut q: Query<(&mut Transform, &FlyCamera, &mut Walker)>,
    (frames, script, match_over): (Res<bevy::diagnostic::FrameCount>, Res<crate::weapons::weapon::ScriptedInput>, Option<Res<crate::game::end_game::MatchOver>>),
    names: Query<&Name>,
    mut effects: Option<ResMut<crate::weapons::weapon::WeaponEffects>>,
    mut bob: ResMut<ViewBob>,
    zeds: Query<&crate::zeds::zed::Zed>,
    mut pinned: Option<ResMut<crate::game::combat::PlayerPinned>>,
    mut pushes: MessageReader<PlayerPush>,
    mut kicks: MessageReader<PlayerAddVelocity>,
    (mut last_log, mut scripted_walk): (Local<f32>, Local<bool>),
    mut glass: WalkMap,
) {
    let mut last_block: Option<(String, Vec3)> = None;
    // Colliders become queryable a frame or two after they are spawned;
    // simulating earlier could drop the player through the floor.
    if *mode != MoveMode::Walk || frames.0 < 5 {
        return;
    }
    let mover = Mover::new(&spatial, kf::RADIUS, kf::HALF_HEIGHT, crate::world::collision::player_filter());
    // Living zeds block the player (pawn cylinders, see pawn_collision).
    let (zed_ids, zed_cylinders): (Vec<usize>, Vec<Cylinder>) =
        zeds.iter().filter_map(|z| z.blocking_cylinder().map(|c| (z.id, c))).unzip();
    // KFHumanPawn.ModifyVelocity: GroundSpeed x the carried-weight factor,
    // plus the held weapon's bonus (knife +40), x the perk's
    // GetMovementSpeedModifier. The health factor is not done.
    let ground_speed = effects
        .as_ref()
        .map_or(kf::GROUND_SPEED, |e| (kf::GROUND_SPEED * e.weight_speed_mult + e.ground_speed_bonus) * e.perk_speed_mult);
    let dt = time.delta_secs().min(0.1);
    // Paused (the pause menu stops game time): nothing moves. The sub-steps
    // below divide by their length, which would be 0.
    if dt <= 0.0 {
        return;
    }
    // KFFire.ModeDoFire: a shot scales the walking velocity (on the ground).
    let fire_scale = effects.as_mut().and_then(|e| e.fire_velocity_scale.take());
    let steps = (dt * 120.0).ceil().max(1.0) as usize;
    let h = dt / steps as f32;
    let me = |centre: Vec3| Cylinder {
        centre,
        radius: kf::RADIUS * SCALE,
        half_height: kf::HALF_HEIGHT * SCALE,
    };
    for (mut t, cam, mut w) in &mut q {
        if let Some(s) = fire_scale
            && w.on_ground
        {
            w.velocity.x *= s;
            w.velocity.z *= s;
        }
        // First step after entering walk mode: settle onto the floor below
        // (the fly camera can leave the cylinder partly sunk into the
        // ground, e.g. at spawn). Cast down from half a height up.
        if w.time == 0.0 {
            let probe = w.center + Vec3::Y * kf::HALF_HEIGHT * SCALE;
            let reach = (kf::HALF_HEIGHT + kf::MAX_STEP) * SCALE;
            match mover.cast(probe, Vec3::NEG_Y, reach) {
                Some(floor) if floor.distance > 0.0 && floor.normal.y >= kf::MIN_FLOOR_NORMAL_Y => {
                    w.center = probe - Vec3::Y * (floor.distance - kf::SKIN * SCALE);
                    w.on_ground = true;
                    w.floor_normal = floor.normal;
                }
                _ => {}
            }
            let c = w.center / SCALE;
            runlog::kv(
                "walk_start",
                &format!("center_unreal=({:.1}, {:.1}, {:.1}) on_ground={}", -c.z, c.x, c.y, w.on_ground),
            );
        }
        // Input direction on the horizontal plane, from the camera yaw.
        let forward = Vec3::new(-cam.yaw.sin(), 0.0, -cam.yaw.cos());
        let right = Vec3::new(cam.yaw.cos(), 0.0, -cam.yaw.sin());
        let mut wish = Vec3::ZERO;
        // Test actions "walk_on" / "walk_off": hold / release forward.
        for (_, a) in script.0.iter().filter(|(f, _)| *f == frames.0) {
            match a.as_str() {
                "walk_on" | "walk_off" => {
                    *scripted_walk = a == "walk_on";
                    runlog::kv("scripted_walk", &format!("forward={}", *scripted_walk));
                }
                _ => {}
            }
        }
        let auto = *scripted_walk || settings.autowalk.is_some_and(|d| w.time >= 1.0 && w.time < 1.0 + d);
        if keys.pressed(KeyCode::KeyW) || auto {
            wish += forward;
        }
        if keys.pressed(KeyCode::KeyS) {
            wish -= forward;
        }
        if keys.pressed(KeyCode::KeyD) {
            wish += right;
        }
        if keys.pressed(KeyCode::KeyA) {
            wish -= right;
        }
        let mut wish = wish.normalize_or_zero();
        // Test action "jump".
        let mut jump = keys.just_pressed(KeyCode::Space) || script.0.iter().any(|(f, a)| *f == frames.0 && a == "jump");
        // Held by a Clot's grab (KFPawn.DisableMovement / ModifyVelocity):
        // no input, no jumping, and no velocity while on the ground.
        // Also once the match is over (GameEnded: PlayerMove only turns
        // the view; Pawn.TurnOff stops the pawn).
        let held = pinned.as_ref().is_some_and(|p| p.active()) || match_over.as_ref().is_some_and(|m| m.active());
        if held {
            wish = Vec3::ZERO;
            jump = false;
            if w.on_ground {
                w.velocity = Vec3::ZERO;
            }
        }
        if let Some(p) = pinned.as_mut()
            && p.active()
        {
            p.seconds -= dt;
            if !p.active() {
                p.release("time_up");
            }
        }

        // KFShotgunFire.DoFireEffect: AddVelocity(KickMomentum >> view).
        // JumpPad.Touch / PostTouch: the player is thrown with the pad's
        // JumpVelocity (set, not added), from the pad's centre (as zeds:
        // an approximation, see DESIGN "Map fixes" M4).
        if let Some(nav) = glass.2.as_deref() {
            let touching = nav.jump_pads.iter().position(|p| {
                let d = nav.points[p.point].pos - w.center;
                d.with_y(0.0).length() / SCALE < crate::world::nav::JUMP_PAD_RADIUS + kf::RADIUS
                    && (d.y / SCALE).abs() < crate::world::nav::JUMP_PAD_HALF_HEIGHT + kf::HALF_HEIGHT
            });
            if let Some(i) = touching
                && *glass.3 != Some(i)
            {
                let pad = nav.jump_pads[i];
                let pc = nav.points[pad.point].pos;
                w.center = Vec3::new(pc.x, w.center.y, pc.z);
                w.velocity = pad.velocity;
                w.on_ground = false;
                runlog::kv("player_jump_pad", &format!("pad={} up_unreal={:.0}", nav.points[pad.point].name, pad.velocity.y / SCALE));
            }
            *glass.3 = touching;
        }
        for kick in kicks.read() {
            let mut v = kick.velocity;
            if v == Vec3::ZERO {
                continue;
            }
            w.on_ground = false;
            if w.velocity.y > 380.0 * SCALE && v.z > 0.0 {
                v.z *= 0.5;
            }
            w.velocity += Vec3::new(v.y, v.z, -v.x) * SCALE;
            runlog::kv("player_kick", &format!("velocity_add_unreal=({:.0}, {:.0}, {:.0})", v.x, v.y, v.z));
        }
        for push in pushes.read() {
            if push.to_peer.is_some() {
                continue;
            }
            let mut m = push.momentum;
            if w.on_ground {
                m.z = m.z.max(0.4 * m.length());
            }
            let mut v = m / PLAYER_MASS;
            if v == Vec3::ZERO {
                continue;
            }
            w.on_ground = false;
            if w.velocity.y > 380.0 * SCALE && v.z > 0.0 {
                v.z *= 0.5;
            }
            w.velocity += Vec3::new(v.y, v.z, -v.x) * SCALE;
            runlog::kv("player_push", &format!("velocity_add_unreal=({:.0}, {:.0}, {:.0})", v.x, v.y, v.z));
        }
        for _ in 0..steps {
            w.time += h;
            let accel = wish * kf::ACCEL_RATE * SCALE;
            if w.on_ground && jump {
                w.velocity.y = kf::JUMP_Z * SCALE;
                w.on_ground = false;
                jump = false;
                runlog::kv("jump", &format!("center_unreal_z={:.1}", w.center.y / SCALE));
            }
            if w.on_ground {
                let hv = calc_velocity(
                    w.velocity.with_y(0.0),
                    accel,
                    kf::GROUND_FRICTION,
                    ground_speed * SCALE,
                    h,
                );
                w.velocity = hv;
                let (step, by_zed) = clip_move(&me(w.center), hv * h, &zed_cylinders);
                if let Some(i) = by_zed {
                    last_block = Some((format!("zed {}", zed_ids[i]), Vec3::ZERO));
                }
                let (moved, blocked) = mover.ground_move(w.center, step);
                let wanted = (hv * h).length();
                if let Some(b) = &blocked
                    && wanted > 0.0
                    && (moved - w.center).length() < 0.5 * wanted
                {
                    last_block = Some((names.get(b.entity).map_or("unnamed".into(), |n| n.to_string()), b.normal));
                }
                // KFGlassMover.Bump by the player (glass.rs decides).
                if let Some(b) = &blocked
                    && let Ok(g) = glass.0.get(b.entity)
                {
                    glass.1.write(crate::world::glass::GlassBump {
                        pane: g.0,
                        speed: hv.length() / SCALE,
                        melee: None,
                    });
                }
                // Like Unreal: velocity becomes the movement actually made
                // (so walking into a wall shows speed 0, not 200).
                let actual = (moved - w.center).with_y(0.0) / h;
                w.velocity = Vec3::new(actual.x, 0.0, actual.z);
                w.center = moved;
                // Stay on the floor: look down a step's height for walkable ground.
                let reach = (kf::MAX_STEP + 2.0 * kf::SKIN) * SCALE;
                match mover.cast(w.center, Vec3::NEG_Y, reach) {
                    Some(floor) if floor.normal.y >= kf::MIN_FLOOR_NORMAL_Y => {
                        // Rest two skins above the floor. A cast stops one skin
                        // away, and resting at exactly that gap makes every
                        // horizontal sweep "touch" the floor at distance 0.
                        w.center.y -= floor.distance - kf::SKIN * SCALE;
                        w.floor_normal = floor.normal;
                    }
                    _ => w.on_ground = false,
                }
            } else {
                w.velocity.y -= kf::GRAVITY * SCALE * h;
                let air = calc_velocity(
                    w.velocity.with_y(0.0),
                    accel * kf::AIR_CONTROL,
                    0.0,
                    ground_speed.max(w.velocity.with_y(0.0).length() / SCALE) * SCALE,
                    h,
                );
                w.velocity.x = air.x;
                w.velocity.z = air.z;
                let (step, by_zed) = clip_move(&me(w.center), w.velocity * h, &zed_cylinders);
                if let Some(i) = by_zed {
                    last_block = Some((format!("zed {}", zed_ids[i]), Vec3::ZERO));
                    w.velocity.x = step.x / h;
                    w.velocity.z = step.z / h;
                }
                let (pos, hit) = mover.slide(w.center, step);
                w.center = pos;
                if let Some(n) = hit.map(|h| h.normal) {
                    if n.y >= kf::MIN_FLOOR_NORMAL_Y && w.velocity.y <= 0.0 {
                        w.center.y += kf::SKIN * SCALE;
                        w.on_ground = true;
                        w.floor_normal = n;
                        w.velocity.y = 0.0;
                    } else {
                        // Lose the part of the velocity going into the surface.
                        let into = w.velocity.dot(n);
                        if into < 0.0 {
                            w.velocity -= n * into;
                        }
                    }
                }
            }
        }
        // KFPawn.CheckBob: speed scaled by BobSpeedModifier 0.9, Bob 0.006,
        // BobScaleModifier 1. WalkBob = right * Bob * speed * sin(8 t);
        // vertical = AppliedBob + 0.75 * Bob * speed * sin(16 t).
        let speed2d = w.velocity.with_y(0.0).length() / SCALE * 0.9;
        if w.on_ground {
            w.bob_time += if speed2d < 10.0 {
                0.2 * dt
            } else {
                dt * (0.3 + 0.7 * speed2d / ground_speed)
            };
            w.applied_bob *= 1.0 - (16.0 * dt).min(1.0);
            let mut up = w.applied_bob;
            if speed2d > 10.0 {
                up += 0.75 * BOB * speed2d * (16.0 * w.bob_time).sin();
            }
            bob.side = right * (BOB * speed2d * (8.0 * w.bob_time).sin()) * SCALE;
            bob.up = up * SCALE;
        } else {
            w.bob_time = 0.0;
            let k = 1.0 - (8.0 * dt).min(1.0);
            bob.side *= k;
            bob.up *= k;
        }
        // Pawn.EyePosition = EyeHeight + WalkBob.
        t.translation = w.center + Vec3::Y * (kf::EYE_HEIGHT * SCALE + bob.up) + bob.side;

        // Twice a second: position, speed and ground state.
        if w.time - *last_log >= 0.1 {
            *last_log = w.time;
            let c = w.center / SCALE;
            runlog::kv(
                "walk",
                &format!(
                    "t={:.1} center_unreal=({:.0}, {:.0}, {:.0}) speed_unreal={:.0} ground_speed_unreal={ground_speed:.1} vertical_unreal={:.0} on_ground={} floor_normal_y={:.2} input={} held={held} bob_side_unreal={:.2} bob_up_unreal={:.2}",
                    w.time,
                    -c.z,
                    c.x,
                    c.y,
                    w.velocity.with_y(0.0).length() / SCALE,
                    w.velocity.y / SCALE,
                    w.on_ground,
                    w.floor_normal.y,
                    wish != Vec3::ZERO,
                    bob.side.length() / SCALE * (bob.side.dot(right)).signum(),
                    bob.up / SCALE
                ),
            );
            if let Some((name, n)) = &last_block {
                runlog::kv(
                    "walk_blocked",
                    &format!("by={name} normal_bevy=({:.2}, {:.2}, {:.2})", n.x, n.y, n.z),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accelerates_to_ground_speed() {
        let mut v = Vec3::ZERO;
        let accel = Vec3::X * kf::ACCEL_RATE;
        for _ in 0..240 {
            v = calc_velocity(v, accel, kf::GROUND_FRICTION, kf::GROUND_SPEED, 1.0 / 120.0);
        }
        assert!((v.length() - kf::GROUND_SPEED).abs() < 1.0, "speed {}", v.length());
    }

    #[test]
    fn brakes_to_stop() {
        let mut v = Vec3::X * kf::GROUND_SPEED;
        for _ in 0..120 {
            v = calc_velocity(v, Vec3::ZERO, kf::GROUND_FRICTION, kf::GROUND_SPEED, 1.0 / 120.0);
        }
        assert!(v.length() < 1.0, "speed {}", v.length());
    }
}
