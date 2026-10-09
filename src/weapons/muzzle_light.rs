//! The muzzle-flash light, the world side (DESIGN.md, "Muzzle-flash
//! light").
//!
//! KF (KFWeaponAttachment.ThirdPersonEffects -> WeaponLight, every shot):
//! the first-person weapon (seen in first person) or the third-person
//! attachment (seen by others) gets bDynamicLight for 0.15 s; every shot
//! restarts the 0.15 s. The light's values are that actor's class
//! defaults (KFWeapon / KFWeaponAttachment: steady, hue 30, saturation
//! 150, brightness 255, radius 10).
//!
//! Here every pawn that shoots carries a `MuzzleLight`: whoever runs its
//! weapon fills in the light values, the position, and calls `flash` on
//! each shot (the local player: weapons/weapon/muzzle_light.rs; other
//! players: player/body/fire_fx.rs). `place_muzzle_lights` keeps one
//! Bevy point light per pawn (made once, hidden while off) and, while on,
//! adds the light to `DynamicLights` so baked meshes switch to their lit
//! material (render/baked.rs) and zeds, bodies and the weapon get it as
//! vertex light (render/actor_light.rs).

use bevy::prelude::*;
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package_set::ObjectHandle;
use ue_assets::properties::Value;

use crate::engine::coords::SCALE;
use crate::engine::runlog;
use crate::render::actor_light::{self, Falloff, Kind, Source};

/// xWeaponAttachment.WeaponLight: SetTimer(0.15, false); Timer turns the
/// light off.
pub const ON_SECONDS: f32 = 0.15;
/// **Guess**: Bevy's light falls off as 1 / d^2, UE2's over its radius;
/// the strength is matched at this share of the radius.
const CALIBRATE_AT: f32 = 0.3;
/// The flashlight glow's rule: Bevy's range a little past the UE2 radius
/// (its window function fades the last part).
const RANGE_PAST_RADIUS: f32 = 1.12;
/// LE_NonIncidence and LE_QuadraticNonIncidence (Engine/Actor.uc
/// ELightEffect).
const LE_NON_INCIDENCE: u8 = 13;
const LE_QUADRATIC_NON_INCIDENCE: u8 = 21;

/// An actor's light properties (Actor: LightType, LightEffect, LightHue,
/// LightSaturation, LightBrightness, LightRadius), from class defaults.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightDef {
    /// ELightType: 0 none, 1 steady, 2 pulse, ...
    pub light_type: u8,
    pub effect: u8,
    pub hue: u8,
    pub saturation: u8,
    pub brightness: f32,
    pub radius: f32,
}

impl LightDef {
    /// The class's light, own or inherited; None when it gives no light
    /// (LightType none, or no brightness).
    pub fn read(defaults: &ClassDefaults, class: &ObjectHandle) -> Option<LightDef> {
        let byte = |p: &str, d: u8| match defaults.get(class, p) {
            Some((Value::Byte(b), _)) => b,
            Some((Value::Int(i), _)) => i.clamp(0, 255) as u8,
            _ => d,
        };
        let float = |p: &str, d: f32| match defaults.get(class, p) {
            Some((Value::Float(f), _)) => f,
            Some((Value::Byte(b), _)) => b as f32,
            Some((Value::Int(i), _)) => i as f32,
            _ => d,
        };
        // Engine.Actor sets none of them: all 0 (no light).
        let def = LightDef {
            light_type: byte("LightType", 0),
            effect: byte("LightEffect", 0),
            hue: byte("LightHue", 0),
            saturation: byte("LightSaturation", 0),
            brightness: float("LightBrightness", 0.0),
            radius: float("LightRadius", 0.0),
        };
        (def.light_type != 0 && def.brightness > 0.0).then_some(def)
    }

    /// Light colour in UE2's 0..1 units at full strength: hue colour x
    /// LightBrightness / 255 x the native colour scale actors use (the same
    /// as a map light: `Source::from_map` x `actor_colour_scale`).
    pub fn native_colour(&self) -> Vec3 {
        actor_light::hue_colour(self.hue, self.saturation) * (self.brightness / 255.0) * native_scale()
    }

    /// Reach in Unreal units: 25 x (LightRadius + 1).
    pub fn world_radius(&self) -> f32 {
        actor_light::world_radius(self.radius)
    }

    /// For logs: type, radius, brightness, colour.
    pub fn describe(&self) -> String {
        let c = self.native_colour();
        format!(
            "type={} effect={} hue={} saturation={} radius={} world_radius={:.0} brightness={} color=({:.2},{:.2},{:.2})",
            self.light_type,
            self.effect,
            self.hue,
            self.saturation,
            self.radius,
            self.world_radius(),
            self.brightness,
            c.x,
            c.y,
            c.z
        )
    }
}

/// The native colour scale (0.8213): `actor_colour_scale` x LIGHT_SCALE.
fn native_scale() -> f32 {
    actor_light::actor_colour_scale() * actor_light::LIGHT_SCALE
}

/// What the attachment class lets through to WeaponLight
/// (KFWeaponAttachment.ThirdPersonEffects and its subclasses).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttachmentRule {
    /// bDoFiringEffects (KFMeleeAttachment: false) and WeaponLight not
    /// emptied (PipeBombAttachment, BlowerThrowerAttachment).
    pub lights: bool,
    /// FiringMode 1 returns before any effect (SingleAttachment,
    /// ShotgunAttachment, DualiesAttachment, NailGunAttachment: the
    /// flashlight button).
    pub not_alt: bool,
}

impl AttachmentRule {
    pub fn read(defaults: &ClassDefaults, attachment: &ObjectHandle) -> AttachmentRule {
        let is = |c: &str| defaults.is_a(attachment, c);
        let effects = is("KFWeaponAttachment") && !matches!(defaults.get(attachment, "bDoFiringEffects"), Some((Value::Bool(false), _)));
        AttachmentRule {
            lights: effects && !is("PipeBombAttachment") && !is("BlowerThrowerAttachment"),
            not_alt: ["SingleAttachment", "ShotgunAttachment", "DualiesAttachment", "NailGunAttachment"].iter().any(|c| is(c)),
        }
    }

    /// Does a shot of this fire mode turn the light on?
    pub fn allows(&self, firing_mode: u8) -> bool {
        self.lights && !(self.not_alt && firing_mode == 1)
    }
}

/// A pawn's muzzle light.
#[derive(Component, Debug, Default)]
pub struct MuzzleLight {
    /// The weapon (or attachment) class whose light this is, for logs.
    pub weapon: String,
    /// Its light; None: this weapon gives none.
    pub light: Option<LightDef>,
    /// Where the light is this frame (Bevy world); None: hidden.
    pub pos: Option<Vec3>,
    /// Seconds left on the 0.15 s timer.
    pub left: f32,
    ons: u32,
    offs: u32,
    was_on: bool,
    entity: Option<Entity>,
    /// For the frame-time log: frames and real seconds while the timer
    /// runs (counted with the light switched off too).
    burst_frames: u32,
    burst_secs: f64,
    was_active: bool,
}

impl MuzzleLight {
    /// WeaponLight: on for 0.15 s from now (if this weapon has a light).
    pub fn flash(&mut self) {
        if self.light.is_some() {
            self.left = ON_SECONDS;
        }
    }

    /// A new weapon in hand: its light (logged once per weapon and pawn).
    pub fn set_weapon(&mut self, weapon: &str, light: Option<LightDef>, who: &str) {
        if self.weapon == weapon {
            return;
        }
        self.weapon = weapon.to_string();
        self.light = light;
        self.left = 0.0;
        runlog::kv(
            "muzzle_light_setup",
            &format!("who={who} weapon={weapon} light={}", light.map_or("none".to_string(), |l| l.describe())),
        );
    }
}

/// Systems that call `flash` and set positions run in this set; the
/// lights are placed after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct MuzzleLightTrigger;

/// Bevy lumens and range for the point light: a wall facing it at
/// CALIBRATE_AT x radius gets the UE2 value there (colour x falloff x K,
/// made linear, as the flashlight glow).
pub fn bevy_light(def: &LightDef) -> (f32, f32) {
    let radius = def.world_radius();
    let range = radius * RANGE_PAST_RADIUS * SCALE;
    let at = radius * CALIBRATE_AT;
    let gamma = def.native_colour().max_element() * actor_light::FALLOFF.at(CALIBRATE_AT) * crate::render::lighting::BRIGHTNESS;
    (crate::weapons::flashlight::lumens_for(gamma.powf(2.2), at * SCALE, range), range)
}

/// `KF_MUZZLE_LIGHT=0` switches the light off (to measure its cost).
fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = std::env::var("KF_MUZZLE_LIGHT").map_or(true, |v| v != "0");
        runlog::kv("muzzle_light_enabled", &format!("on={on}"));
        on
    })
}

pub struct MuzzleLightPlugin;

impl Plugin for MuzzleLightPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            place_muzzle_lights.after(MuzzleLightTrigger).after(crate::weapons::flashlight::FlashlightLights),
        );
    }
}

/// Ticks each pawn's timer, shows / hides its point light, and adds the
/// light to `DynamicLights` while on. (`DynamicLights` is cleared by the
/// flashlight's system, which runs first.)
fn place_muzzle_lights(
    mut commands: Commands,
    time: Res<Time>,
    real: Res<Time<Real>>,
    mut holders: Query<&mut MuzzleLight>,
    mut lights: Query<(&mut PointLight, &mut Transform, &mut Visibility)>,
    mut dynamic: ResMut<actor_light::DynamicLights>,
) {
    let dt = time.delta_secs();
    let on_allowed = enabled();
    for mut m in &mut holders {
        let m = &mut *m;
        m.left = (m.left - dt).max(0.0);
        // Frame time while the timer runs (KF_MUZZLE_LIGHT=0 runs log the
        // same, without the light, for comparison).
        let active = m.left > 0.0;
        if active {
            m.burst_frames += 1;
            m.burst_secs += real.delta_secs_f64();
        } else if m.was_active {
            let mean = if m.burst_frames > 0 { m.burst_secs * 1000.0 / m.burst_frames as f64 } else { 0.0 };
            runlog::kv(
                "muzzle_light_burst",
                &format!("weapon={} lit={on_allowed} frames={} seconds={:.2} mean_frame_ms={mean:.2}", m.weapon, m.burst_frames, m.burst_secs),
            );
            m.burst_frames = 0;
            m.burst_secs = 0.0;
        }
        m.was_active = active;
        let on = on_allowed && active;
        let shown = match (on, m.light, m.pos) {
            (true, Some(def), Some(pos)) => Some((def, pos)),
            _ => None,
        };
        let Some(entity) = m.entity else {
            // Made once, hidden; no shadows (UE2 dynamic lights cast none).
            m.entity = Some(
                commands
                    .spawn((PointLight { shadow_maps_enabled: false, ..default() }, Transform::default(), Visibility::Hidden, Name::new("muzzle_light")))
                    .id(),
            );
            continue;
        };
        let Ok((mut light, mut t, mut vis)) = lights.get_mut(entity) else { continue };
        if on != m.was_on {
            m.was_on = on;
            if on {
                m.ons += 1;
            } else {
                m.offs += 1;
            }
            let (lumens, range) = m.light.as_ref().map_or((0.0, 0.0), bevy_light);
            runlog::kv(
                "muzzle_light",
                &format!(
                    "weapon={} on={on} {} lumens={lumens:.0} range_m={range:.2} pos={} ons={} offs={}",
                    m.weapon,
                    m.light.map_or("light=none".to_string(), |l| l.describe()),
                    m.pos.map_or("none".to_string(), |p| format!("({:.2},{:.2},{:.2})", p.x, p.y, p.z)),
                    m.ons,
                    m.offs
                ),
            );
        }
        let Some((def, pos)) = shown else {
            vis.set_if_neq(Visibility::Hidden);
            continue;
        };
        let (lumens, range) = bevy_light(&def);
        let c = actor_light::hue_colour(def.hue, def.saturation);
        let colour = Color::srgb(c.x, c.y, c.z);
        if light.color != colour {
            light.color = colour;
        }
        light.intensity = lumens;
        light.range = range;
        t.translation = pos;
        vis.set_if_neq(Visibility::Visible);
        dynamic.0.push(Source {
            name: "muzzle_light".into(),
            class: m.weapon.clone(),
            pos,
            colour: actor_light::hue_colour(def.hue, def.saturation) * (def.brightness / 255.0) * actor_light::LIGHT_SCALE,
            radius: def.world_radius() * SCALE,
            kind: Kind::Point,
            // As a map light (`Source::from_map`).
            incidence: !matches!(def.effect, LE_NON_INCIDENCE | LE_QUADRATIC_NON_INCIDENCE),
            falloff: if def.effect == LE_QUADRATIC_NON_INCIDENCE { Falloff::Quadratic } else { actor_light::FALLOFF },
            zone: None,
            dynamic: true,
            brightness: def.brightness,
            line_check: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kf_weapon_light_values() {
        // KFWeapon defaults: steady, NonIncidence, hue 30, saturation 150,
        // brightness 255, radius 10.
        let d = LightDef { light_type: 1, effect: 13, hue: 30, saturation: 150, brightness: 255.0, radius: 10.0 };
        assert_eq!(d.world_radius(), 275.0);
        let c = d.native_colour();
        // Warm: red strongest, blue weakest, red at the native 0.82.
        assert!(c.x > c.y && c.y > c.z, "{c:?}");
        assert!((c.x - 0.8213).abs() < 1e-3, "{c:?}");
        let (lumens, range) = bevy_light(&d);
        assert!((range - 275.0 * 1.12 * SCALE).abs() < 1e-4);
        assert!(lumens > 0.0);
    }

    #[test]
    fn attachment_rule_skips_alt_fire_on_torch_attachments() {
        let r = AttachmentRule { lights: true, not_alt: true };
        assert!(r.allows(0));
        assert!(!r.allows(1));
        let melee = AttachmentRule { lights: false, not_alt: false };
        assert!(!melee.allows(0));
    }

    #[test]
    fn flash_only_with_a_light() {
        let mut m = MuzzleLight::default();
        m.flash();
        assert_eq!(m.left, 0.0);
        m.light = Some(LightDef { light_type: 1, effect: 13, hue: 30, saturation: 150, brightness: 255.0, radius: 10.0 });
        m.flash();
        assert_eq!(m.left, ON_SECONDS);
    }
}
