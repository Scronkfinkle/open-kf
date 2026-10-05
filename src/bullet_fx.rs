//! Bullet tracers and impacts. See `docs/DESIGN.md`, "Firing effects".
//!
//! - Tracer (KFWeaponAttachment.SpawnTracer; ZombieBoss.AddTraceHitFX): one
//!   KFNewTracer emitter per shooter, spawned once and reused. Per shot it
//!   is moved to the shot's start, its emitter's StartVelocityRange is set
//!   to the shot direction x speed and its LifetimeRange to (distance -
//!   50) / speed (mTracerPullback 50), and SpawnParticle(1) fires one
//!   particle that dies just before the hit point.
//! - Impact (ROBulletHitEffect, a ROHitEffect): a 16-unit trace into the
//!   surface finds the hit point and material; the entry for the material's
//!   SurfaceType gives a bullet-hole decal and an impact emitter (and a
//!   sound, not played: no sound yet). Every surface uses the default entry
//!   for now (EST_Default: BulletHoleDirt, ROBulletHitRockEffect).

use std::collections::HashMap;

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::coords::{self, SCALE};
use crate::decals::{DecalKind, SpawnDecal};
use crate::particles::{self, EffectLibrary, ParticleEffect, SpawnOptions};
use crate::runlog;

/// KFWeaponAttachment mTracerPullback (also ZombieBoss.AddTraceHitFX's -50).
const TRACER_PULLBACK: f32 = 50.0;
/// ROHitEffect: how far it traces into the surface for the material.
const SURFACE_TRACE: f32 = 16.0;

/// Who fired: each has its own tracer emitter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shooter {
    Player,
    /// One of a pool of tracers for the player's pellets and nails (KF gives
    /// each ShotgunBullet its own KFTracer; a shot has up to 20).
    PlayerPellet(u8),
    Zed(usize),
}

/// One shot's effects. Positions and directions in Unreal world units.
#[derive(Message, Clone, Copy, Debug)]
pub struct BulletFx {
    pub shooter: Shooter,
    /// Where the tracer starts (the weapon's tip); None: no tracer.
    pub start: Option<Vec3>,
    /// mHitLocation: where ROBulletHitEffect is spawned.
    pub hit: Vec3,
    /// ROBulletHitEffect's Rotation, pointing into what was hit: the player's
    /// shots rotator(-HitNormal), the Patriarch's the shot direction.
    pub into: Vec3,
    /// Spawn ROBulletHitEffect.
    pub impact: bool,
    /// Tracer speed (mTracerSpeed 7500; the Patriarch's 10000).
    pub tracer_speed: f32,
    /// No tracer unless distance - 50 is over this (mTracerMinDistance 0;
    /// AddTraceHitFX 10).
    pub min_distance: f32,
}

/// The tracer emitters, by shooter, and shots waiting for theirs to exist
/// (an effect spawned this frame can be used from the next).
#[derive(Resource, Default)]
struct Tracers {
    emitters: HashMap<Shooter, Entity>,
    waiting: Vec<BulletFx>,
}

pub struct BulletFxPlugin;

impl Plugin for BulletFxPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<BulletFx>().init_resource::<Tracers>().add_systems(Update, bullet_fx);
    }
}

/// The frame of an effect whose X axis points along `d` (Unreal).
fn axes_along(d: Vec3) -> Mat3 {
    let k = 65536.0 / std::f32::consts::TAU;
    coords::ue_rotation_matrix(ue_assets::properties::Rotator {
        pitch: (d.z.clamp(-1.0, 1.0).asin() * k) as i32,
        yaw: (d.y.atan2(d.x) * k) as i32,
        roll: 0,
    })
}

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn bullet_fx(
    mut commands: Commands,
    mut requests: MessageReader<BulletFx>,
    library: Option<Res<EffectLibrary>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut effects: Query<&mut ParticleEffect>,
    mut tracers: ResMut<Tracers>,
    mut decals: MessageWriter<SpawnDecal>,
    spatial: SpatialQuery,
) {
    let Some(lib) = library.as_deref() else {
        requests.clear();
        return;
    };
    let mut todo: Vec<BulletFx> = std::mem::take(&mut tracers.waiting);
    for r in requests.read() {
        if r.impact {
            impact(&mut commands, lib, &mut meshes, &mut decals, &spatial, r);
        }
        if r.start.is_some() {
            todo.push(*r);
        }
    }
    for r in todo {
        let Some(start) = r.start else {
            continue;
        };
        let entity = match tracers.emitters.get(&r.shooter) {
            Some(e) => *e,
            None => {
                let options = SpawnOptions {
                    persistent: true,
                    ..default()
                };
                let Some(e) = particles::spawn_effect_with(&mut commands, lib, &mut meshes, "KFMod.KFNewTracer", start, Mat3::IDENTITY, 11, options) else {
                    runlog::kv("tracer_missing", "class=KFMod.KFNewTracer");
                    continue;
                };
                tracers.emitters.insert(r.shooter, e);
                tracers.waiting.push(r);
                continue;
            }
        };
        let Ok(mut fx) = effects.get_mut(entity) else {
            tracers.waiting.push(r);
            continue;
        };
        let to_hit = r.hit - start;
        let dist = to_hit.length() - TRACER_PULLBACK;
        if dist <= r.min_distance {
            continue;
        }
        let dir = to_hit.normalize_or_zero();
        fx.frame = (start, Mat3::IDENTITY);
        fx.set_start(0, dir * r.tracer_speed, dist / r.tracer_speed);
        fx.spawn_all(1);
        runlog::kv(
            "tracer",
            &format!(
                "shooter={:?} start_unreal=({:.0}, {:.0}, {:.0}) hit_unreal=({:.0}, {:.0}, {:.0}) lifetime={:.3}",
                r.shooter,
                start.x,
                start.y,
                start.z,
                r.hit.x,
                r.hit.y,
                r.hit.z,
                dist / r.tracer_speed
            ),
        );
    }
}

/// ROHitEffect.PostNetBeginPlay at `r.hit`, facing into the surface.
fn impact(
    commands: &mut Commands,
    lib: &EffectLibrary,
    meshes: &mut Assets<Mesh>,
    decals: &mut MessageWriter<SpawnDecal>,
    spatial: &SpatialQuery,
    r: &BulletFx,
) {
    // Trace(HitLoc, HitNormal, Location + Vector(Rotation) x 16, Location):
    // Rotation is rotator(-HitNormal), into the surface.
    let into = r.into.normalize_or_zero();
    let from = coords::pos(r.hit.to_array());
    let (hit_loc, hit_normal) = match Dir3::new(coords::dir(into.to_array()))
        .ok()
        .and_then(|d| spatial.cast_ray(from, d, SURFACE_TRACE * SCALE, true, &crate::collision::world_filter()))
    {
        Some(h) => {
            let p = from + coords::dir(into.to_array()) * h.distance;
            let n = Vec3::new(-h.normal.z, h.normal.x, h.normal.y);
            let n = if n.dot(into) > 0.0 { -n } else { n };
            (Some(Vec3::new(-p.z, p.x, p.y) / SCALE), n)
        }
        None => (None, -into),
    };
    // EST_Default (the material's SurfaceType is not read yet).
    decals.write(SpawnDecal {
        kind: DecalKind::BulletHole,
        at: r.hit,
        dir: into,
        trace: false,
    });
    // At HitLoc facing out of the surface, or (no trace hit) at Location
    // with Rotation.
    let (at, axes) = match hit_loc {
        Some(p) => (p, axes_along(hit_normal)),
        None => (r.hit, axes_along(into)),
    };
    particles::spawn_effect(commands, lib, meshes, "ROEffects.ROBulletHitRockEffect", at, axes, 13);
    runlog::kv(
        "bullet_impact",
        &format!(
            "shooter={:?} at_unreal=({:.0}, {:.0}, {:.0}) surface_found={} surface=default",
            r.shooter,
            at.x,
            at.y,
            at.z,
            hit_loc.is_some()
        ),
    );
}
