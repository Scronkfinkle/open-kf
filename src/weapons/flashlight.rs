//! Weapon flashlights, the holder's side (DESIGN.md, "Weapon flashlights").
//!
//! KF keeps the battery on the pawn (KFHumanPawn.TorchBatteryLife) and the
//! light on the weapon in hand (KFWeapon.FlashLight, an
//! Effect_TacLightProjector plus its Effect_TacLightGlow). Here both live in
//! a `Flashlight` component on whoever holds the weapon. Whoever places that
//! holder's weapon fills in `Flashlight::beam` each frame (the local player:
//! weapon/torch.rs, from the first-person weapon's LightBone); the systems
//! here turn every holder's beam into lights in the world. Nothing here is
//! specific to the local camera, so another pawn's light would use the same
//! code. No networking (single player only).

use avian3d::prelude::*;
use bevy::camera::Exposure;
use bevy::prelude::*;

use crate::engine::coords::{self, SCALE};
use crate::engine::runlog;

/// KFHumanPawn TorchBatteryLife (class default).
pub const BATTERY_MAX: i32 = 500;
/// KFHumanPawn.PreBeginPlay: SetTimer(1.5, true); its Timer drains or
/// recharges the battery.
const BATTERY_TICK: f32 = 1.5;
/// Timer: -10 while the light is on, +20 otherwise (up to the maximum).
const BATTERY_DRAIN: i32 = 10;
const BATTERY_RECHARGE: i32 = 20;

/// Effect_TacLightProjector.Tick: EndTrace = StartTrace + 1800 x X.
const TRACE: f32 = 1800.0;
/// Effect_TacLightProjector default FOV (degrees) and MaxTraceDistance.
const PROJECTOR_FOV: f32 = 50.0;
const PROJECTOR_MAX_TRACE: f32 = 1600.0;
/// Effect_TacLightGlow defaults: LightBrightness 100, LightRadius 3.
const GLOW_BRIGHTNESS: f32 = 100.0;
const GLOW_RADIUS: f32 = 3.0;

/// The LightCircle texture's average where it is bright (radius under 0.55
/// of the circle): about 150 of 255 (measured on the decoded texture).
const CIRCLE_CORE: f32 = 0.58;
/// **Guess**: Bevy's spot cone fades from the inner to the outer angle;
/// 0.45 x outer roughly matches the circle (bright to half its radius,
/// nothing at the edge).
const INNER_ANGLE_FRACTION: f32 = 0.45;
/// **Guess**: the glow sits this share of its radius in front of the
/// surface (a Bevy light on the surface itself would be infinitely bright
/// there).
const GLOW_STANDOFF: f32 = 0.5;
/// Brightness is calibrated at the hit point, but at most this far
/// (Unreal units), so a beam into the open still lights what is near.
const CALIBRATE_MAX: f32 = 1200.0;

/// Where a holder's beam starts and points this frame (Unreal units and
/// axes).
#[derive(Clone, Copy, Debug)]
pub struct Beam {
    pub start: Vec3,
    pub dir: Vec3,
}

/// A holder's flashlight: battery (pawn) and light (weapon in hand).
#[derive(Component, Debug)]
pub struct Flashlight {
    /// TorchBatteryLife, 0 to BATTERY_MAX.
    pub battery: i32,
    /// FlashLight.bHasLight.
    pub on: bool,
    /// The weapon class whose light this is (the projector's ValidWeapon);
    /// None when that weapon has no light actor (KF: FlashLight == None).
    pub weapon: Option<String>,
    /// This frame's beam, filled in by whoever places the holder's weapon;
    /// None: nothing to shine from (the lights are hidden).
    pub beam: Option<Beam>,
    /// Seconds into the 1.5 s battery timer.
    pub battery_timer: f32,
}

impl Default for Flashlight {
    fn default() -> Self {
        Flashlight { battery: BATTERY_MAX, on: false, weapon: None, beam: None, battery_timer: 0.0 }
    }
}

impl Flashlight {
    /// KFHumanPawn.Timer's battery step: on: -10 (while above 0); off or no
    /// light: +20, up to the maximum.
    pub fn battery_step(&mut self) {
        if self.on && self.weapon.is_some() {
            if self.battery > 0 {
                self.battery -= BATTERY_DRAIN;
            }
        } else if self.battery < BATTERY_MAX {
            self.battery = (self.battery + BATTERY_RECHARGE).min(BATTERY_MAX);
        }
    }

    /// The HUD's FlashlightDigits: 100 x battery / maximum.
    pub fn percent(&self) -> i32 {
        (100.0 * self.battery as f32 / BATTERY_MAX as f32) as i32
    }
}

/// Systems that fill `Flashlight::beam` run in this set; the lights are
/// placed after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct FlashlightBeamSet;

/// `place_lights` (it clears `DynamicLights` each frame; other dynamic
/// lights, such as the muzzle light, are added after it).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct FlashlightLights;

/// The world lights made for one holder.
#[derive(Component)]
struct FlashlightRig {
    spot: Entity,
    glow: Entity,
}

pub struct FlashlightPlugin;

impl Plugin for FlashlightPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (battery_timer, place_lights.after(FlashlightBeamSet).in_set(FlashlightLights)));
    }
}

fn battery_timer(time: Res<Time>, mut lights: Query<&mut Flashlight>) {
    for mut f in &mut lights {
        f.battery_timer += time.delta_secs();
        while f.battery_timer >= BATTERY_TICK {
            f.battery_timer -= BATTERY_TICK;
            let before = f.battery;
            f.battery_step();
            if f.battery != before {
                runlog::kv("flashlight_battery", &format!("battery={} percent={} on={}", f.battery, f.percent(), f.on));
            }
        }
    }
}

/// KF's per-frame light values for a beam that hits after `beam` units
/// (Effect_TacLightProjector.Tick).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightValues {
    /// Projector FOV, degrees: Lerp(Beam / 1800, 0.6 x FOV, FOV).
    pub fov: f32,
    /// TacLightGlow LightBrightness and LightRadius (UE2 units).
    pub glow_brightness: f32,
    pub glow_radius: f32,
}

pub fn light_values(beam: f32) -> LightValues {
    let lerp = |t: f32, a: f32, b: f32| a + (b - a) * t;
    let (glow_brightness, glow_radius) = if beam <= 100.0 {
        (GLOW_BRIGHTNESS, lerp(beam / 100.0, 0.0, GLOW_RADIUS * 1.25))
    } else {
        (GLOW_BRIGHTNESS * (1.0 - beam / TRACE), (GLOW_RADIUS * 4.0).min(lerp(beam / 900.0, GLOW_RADIUS, GLOW_RADIUS * 4.0)))
    };
    LightValues { fov: lerp(beam / TRACE, PROJECTOR_FOV * 0.6, PROJECTOR_FOV), glow_brightness, glow_radius }
}

/// UE2's light radius in world units: 25 x (LightRadius + 1).
fn world_radius(r: f32) -> f32 {
    25.0 * (r + 1.0)
}

/// Bevy lumens that light a surface facing the light, `metres` away,
/// to `linear` x its texture (Bevy: albedo x lumens / 4 pi / pi / d^2 x
/// exposure x the range window).
pub fn lumens_for(linear: f32, metres: f32, range: f32) -> f32 {
    let window = (1.0 - (metres / range).powi(4)).clamp(0.0, 1.0).powi(2).max(0.05);
    let pi = std::f32::consts::PI;
    linear * 4.0 * pi * pi * metres * metres / Exposure::default().exposure() / window
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)] // Bevy system parameters
fn place_lights(
    mut commands: Commands,
    time: Res<Time>,
    spatial: SpatialQuery,
    holders: Query<(Entity, &Flashlight, Option<&FlashlightRig>)>,
    zeds: Query<&crate::zeds::zed::Zed>,
    mut spots: Query<(&mut SpotLight, &mut Transform, &mut Visibility), Without<PointLight>>,
    mut glows: Query<(&mut PointLight, &mut Transform, &mut Visibility), Without<SpotLight>>,
    mut actor_lights: ResMut<crate::render::actor_light::DynamicLights>,
    mut log_timer: Local<f32>,
) {
    actor_lights.0.clear();
    *log_timer += time.delta_secs();
    let log_now = *log_timer >= 1.0;
    if log_now {
        *log_timer = 0.0;
    }
    for (holder, f, rig) in &holders {
        let Some(rig) = rig else {
            // Hidden until the light is on. Shadows on the spot stand in for
            // the projector's bClipBSP (no light through walls).
            let spot = commands
                .spawn((
                    SpotLight { shadow_maps_enabled: true, range: TRACE * SCALE, ..default() },
                    Transform::default(),
                    Visibility::Hidden,
                    Name::new("flashlight_spot"),
                ))
                .id();
            let glow = commands
                .spawn((PointLight { shadow_maps_enabled: false, ..default() }, Transform::default(), Visibility::Hidden, Name::new("flashlight_glow")))
                .id();
            commands.entity(holder).insert(FlashlightRig { spot, glow });
            continue;
        };
        let (Ok((mut spot, mut spot_t, mut spot_vis)), Ok((mut glow, mut glow_t, mut glow_vis))) = (spots.get_mut(rig.spot), glows.get_mut(rig.glow))
        else {
            continue;
        };
        let beam = f.beam.filter(|_| f.on && f.weapon.is_some()).and_then(|b| b.dir.try_normalize().map(|dir| Beam { start: b.start, dir }));
        let Some(beam) = beam else {
            spot_vis.set_if_neq(Visibility::Hidden);
            glow_vis.set_if_neq(Visibility::Hidden);
            continue;
        };
        // Trace 1800 units (actors too): the first hit of the world, doors
        // or a zed's collision cylinder.
        let origin = coords::pos(beam.start.to_array());
        let dir = coords::dir(beam.dir.to_array()).normalize();
        let world_hit = Dir3::new(dir).ok().and_then(|d| spatial.cast_ray(origin, d, TRACE * SCALE, true, &crate::world::collision::world_filter())).map(|h| h.distance);
        let zed_hit = zeds.iter().filter_map(|z| crate::game::combat::zed_hit(z, origin, dir)).filter(|&t| t <= TRACE * SCALE).reduce(f32::min);
        let hit_what = match (world_hit, zed_hit) {
            (_, Some(z)) if world_hit.is_none_or(|w| z < w) => "zed",
            (Some(_), _) => "world",
            _ => "none",
        };
        let hit = match (world_hit, zed_hit) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let length = hit.map_or(TRACE, |h| h / SCALE);
        let v = light_values(length);

        // The projector as a spot light from the beam start. Brightness at
        // the hit (or at CALIBRATE_MAX): texture x circle x the depth fade,
        // made linear.
        let at = length.clamp(20.0, CALIBRATE_MAX);
        let fade = (1.0 - at / PROJECTOR_MAX_TRACE).max(0.0);
        let spot_gamma = CIRCLE_CORE * fade;
        let half = (v.fov * 0.5).to_radians();
        spot.outer_angle = half;
        spot.inner_angle = half * INNER_ANGLE_FRACTION;
        spot.intensity = lumens_for(spot_gamma.powf(2.2), at * SCALE, spot.range);
        let up = if dir.y.abs() > 0.99 { Vec3::X } else { Vec3::Y };
        *spot_t = Transform::from_translation(origin).looking_to(dir, up);
        spot_vis.set_if_neq(Visibility::Visible);

        // The glow: KF puts it on the hit point; ours stands off by half its
        // radius. Brightness: LightBrightness / 255 x K, made linear.
        let radius = world_radius(v.glow_radius);
        let standoff = (radius * GLOW_STANDOFF).min(length).max(1.0);
        let glow_at = origin + dir * (length - standoff) * SCALE;
        let k = crate::render::lighting::BRIGHTNESS;
        let glow_gamma = (v.glow_brightness / 255.0 * k).max(0.0);
        glow.range = (radius * 1.12).max(standoff * 1.12) * SCALE;
        glow.intensity = lumens_for(glow_gamma.powf(2.2), standoff * SCALE, glow.range);
        glow_t.translation = glow_at;
        glow_vis.set_if_neq(if glow.intensity > 0.0 { Visibility::Visible } else { Visibility::Hidden });

        // The same two lights for actor lighting (zeds, bodies and weapons
        // are drawn with vertex light: render/actor_light.rs). The
        // projector: the circle's core, fading to nothing at
        // MaxTraceDistance (bGradient) inside the cone. **Guess**: on
        // actors it counts like a light facing them (backfaces unlit).
        {
            use crate::render::actor_light::{Falloff, Kind, Source};
            actor_lights.0.push(Source {
                name: "flashlight_projector".into(),
                class: "Effect_TacLightProjector".into(),
                pos: origin,
                colour: Vec3::splat(CIRCLE_CORE),
                radius: PROJECTOR_MAX_TRACE * SCALE,
                kind: Kind::Spot { dir, cos_outer: half.cos(), cos_inner: (half * INNER_ANGLE_FRACTION).cos() },
                incidence: true,
                falloff: Falloff::Linear,
                zone: None,
                dynamic: true,
                brightness: 255.0,
                line_check: false,
            });
            if v.glow_brightness > 0.0 {
                actor_lights.0.push(Source {
                    name: "flashlight_glow".into(),
                    class: "Effect_TacLightGlow".into(),
                    pos: glow_at,
                    colour: Vec3::splat(v.glow_brightness / 255.0),
                    radius: radius * SCALE,
                    kind: Kind::Point,
                    incidence: true,
                    falloff: crate::render::actor_light::FALLOFF,
                    zone: None,
                    dynamic: true,
                    brightness: v.glow_brightness,
                    line_check: false,
                });
            }
        }

        if log_now {
            runlog::kv(
                "flashlight_beam",
                &format!(
                    "weapon={} start=({:.0},{:.0},{:.0}) dir=({:.2},{:.2},{:.2}) hit={hit_what} beam_length={length:.0} fov={:.1} spot_gamma={spot_gamma:.2} spot_lumens={:.0} glow_brightness={:.1} glow_radius={:.2} glow_world_radius={radius:.0} glow_lumens={:.0} battery={}",
                    f.weapon.as_deref().unwrap_or("none"),
                    beam.start.x,
                    beam.start.y,
                    beam.start.z,
                    beam.dir.x,
                    beam.dir.y,
                    beam.dir.z,
                    v.fov,
                    spot.intensity,
                    v.glow_brightness,
                    v.glow_radius,
                    glow.intensity,
                    f.battery
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_drains_75_seconds_and_recharges_in_37() {
        let mut f = Flashlight { on: true, weapon: Some("KFMod.Single".into()), ..default() };
        let mut ticks = 0;
        while f.battery > 0 {
            f.battery_step();
            ticks += 1;
        }
        assert_eq!(ticks, 50); // 50 x 1.5 s = 75 s
        f.battery_step();
        assert_eq!(f.battery, 0, "an empty battery stays at 0 while on");
        f.on = false;
        let mut ticks = 0;
        while f.battery < BATTERY_MAX {
            f.battery_step();
            ticks += 1;
        }
        assert_eq!(ticks, 25); // 25 x 1.5 s = 37.5 s
        assert_eq!(f.percent(), 100);
    }

    #[test]
    fn light_values_follow_the_projector_tick() {
        // Close to a wall: full brightness, the glow shrinks to nothing.
        let near = light_values(50.0);
        assert_eq!(near.glow_brightness, 100.0);
        assert!((near.glow_radius - 1.875).abs() < 1e-4);
        assert!((near.fov - (30.0 + 20.0 * 50.0 / 1800.0)).abs() < 1e-4);
        // 900 units: half brightness, radius 12 (the cap), FOV 40.
        let mid = light_values(900.0);
        assert!((mid.glow_brightness - 50.0).abs() < 1e-4);
        assert!((mid.glow_radius - 12.0).abs() < 1e-4);
        assert!((mid.fov - 40.0).abs() < 1e-4);
        // Nothing hit: the glow is out, FOV 50.
        let far = light_values(1800.0);
        assert_eq!(far.glow_brightness, 0.0);
        assert_eq!(far.fov, 50.0);
    }
}
