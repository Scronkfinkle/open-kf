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
//!   SurfaceType gives a bullet-hole decal, an impact emitter and a sound
//!   (PlaySound(HitSound, SLOT_None, 1.0, false, 100)): `HIT_EFFECTS`, by
//!   the hit triangle's material SurfaceType (collision.rs SurfaceMap).

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

/// ROBulletHitEffect.HitEffects[ESurfaceTypes]: (HitSound, HitEffect,
/// HitDecal), from the class defaults (decoded in work/s5-research.md).
const HIT_EFFECTS: [(&str, &str, Option<DecalKind>); 20] = [
    ("Impact_Dirt", "ROBulletHitRockEffect", Some(DecalKind::BulletHole)),
    ("Impact_Asphalt", "ROBulletHitRockEffect", Some(DecalKind::BulletHoleConcrete)),
    ("Impact_Dirt", "ROBulletHitDirtEffect", Some(DecalKind::BulletHole)),
    ("Impact_Metal", "ROBulletHitMetalEffect", Some(DecalKind::BulletHoleMetal)),
    ("Impact_Wood", "ROBulletHitWoodEffect", Some(DecalKind::BulletHoleWood)),
    ("Impact_Grass", "ROBulletHitGrassEffect", Some(DecalKind::BulletHole)),
    ("Impact_Mud", "ROBulletHitFleshEffect", Some(DecalKind::BulletHoleFlesh)),
    ("Impact_Glass", "ROBulletHitIceEffect", Some(DecalKind::BulletHoleIce)),
    ("Impact_Snow", "ROBulletHitSnowEffect", Some(DecalKind::BulletHoleSnow)),
    ("Impact_Snow", "ROBulletHitWaterEffect", None),
    ("Impact_Glass", "ROBreakingGlass", Some(DecalKind::BulletHoleIce)),
    ("Impact_Gravel", "ROBulletHitGravelEffect", Some(DecalKind::BulletHoleConcrete)),
    ("Impact_Asphalt", "ROBulletHitConcreteEffect", Some(DecalKind::BulletHoleConcrete)),
    ("Impact_Wood", "ROBulletHitWoodEffect", Some(DecalKind::BulletHoleWood)),
    ("Impact_Mud", "ROBulletHitMudEffect", Some(DecalKind::BulletHoleSnow)),
    ("Impact_Metal", "ROBulletHitMetalArmorEffect", Some(DecalKind::BulletHoleMetalArmor)),
    ("Impact_Wood", "ROBulletHitPaperEffect", Some(DecalKind::BulletHoleConcrete)),
    ("Impact_Dirt", "ROBulletHitClothEffect", Some(DecalKind::BulletHoleCloth)),
    ("Impact_Dirt", "ROBulletHitRubberEffect", Some(DecalKind::BulletHoleMetal)),
    ("Impact_Mud", "ROBulletHitMudEffect", Some(DecalKind::BulletHole)),
];

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
    (mut decals, mut sounds): (MessageWriter<SpawnDecal>, MessageWriter<crate::audio::PlaySound>),
    (spatial, surfaces): (SpatialQuery, Query<(&Collider, &GlobalTransform, &crate::collision::SurfaceMap)>),
) {
    let Some(lib) = library.as_deref() else {
        requests.clear();
        return;
    };
    let mut todo: Vec<BulletFx> = std::mem::take(&mut tracers.waiting);
    for r in requests.read() {
        if r.impact {
            let (at, sound) = impact(&mut commands, lib, &mut meshes, &mut decals, &spatial, &surfaces, r);
            sounds.write(
                crate::audio::PlaySound::new(format!("ProjectileSounds.Bullets.{sound}"), crate::audio::Emitter::Point(coords::pos(at.to_array())))
                    .volume(1.0)
                    .radius(100.0),
            );
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
    surfaces: &Query<(&Collider, &GlobalTransform, &crate::collision::SurfaceMap)>,
    r: &BulletFx,
) -> (Vec3, &'static str) {
    // Trace(HitLoc, HitNormal, Location + Vector(Rotation) x 16, Location):
    // Rotation is rotator(-HitNormal), into the surface.
    let into = r.into.normalize_or_zero();
    let from = coords::pos(r.hit.to_array());
    let dir_bevy = coords::dir(into.to_array());
    let (hit_loc, hit_normal, surface) = match Dir3::new(dir_bevy)
        .ok()
        .and_then(|d| spatial.cast_ray(from, d, SURFACE_TRACE * SCALE, true, &crate::collision::world_filter()))
    {
        Some(h) => {
            let p = from + dir_bevy * h.distance;
            let n = Vec3::new(-h.normal.z, h.normal.x, h.normal.y);
            let n = if n.dot(into) > 0.0 { -n } else { n };
            // HitMat.SurfaceType (no material: EST_Default).
            let st = crate::collision::surface_of_hit(surfaces, h.entity, from, dir_bevy, SURFACE_TRACE * SCALE).map_or(0, |s| s[0]);
            (Some(Vec3::new(-p.z, p.x, p.y) / SCALE), n, st)
        }
        None => (None, -into, 0),
    };
    let (sound, effect, decal) = HIT_EFFECTS.get(surface as usize).copied().unwrap_or(HIT_EFFECTS[0]);
    if let Some(kind) = decal {
        decals.write(SpawnDecal {
            kind,
            at: r.hit,
            dir: into,
            trace: false,
        });
    }
    // At HitLoc facing out of the surface, or (no trace hit) at Location
    // with Rotation.
    let (at, axes) = match hit_loc {
        Some(p) => (p, axes_along(hit_normal)),
        None => (r.hit, axes_along(into)),
    };
    particles::spawn_effect(commands, lib, meshes, &format!("ROEffects.{effect}"), at, axes, 13);
    runlog::kv(
        "bullet_impact",
        &format!(
            "shooter={:?} at_unreal=({:.0}, {:.0}, {:.0}) surface_found={} surface={}",
            r.shooter,
            at.x,
            at.y,
            at.z,
            hit_loc.is_some(),
            crate::collision::surface_name(surface)
        ),
    );
    (at, sound)
}
