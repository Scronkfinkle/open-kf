//! The player's flying projectiles: shotgun pellets (ShotgunBullet and its
//! subclasses) and nails (NailGunProjectile). They fly in a straight line at
//! Speed (nails fall after their first bounce), pass through zeds losing
//! damage, and stop at walls (nails bounce first). Grenades and rockets come
//! in W6 and will reuse this.
//!
//! Positions and velocities are Unreal units; collision tests convert to
//! Bevy space.

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::coords::{self, SCALE};
use crate::runlog;
use crate::zed::Zed;

/// PhysicsVolume gravity (Unreal units/s^2), for nails after a bounce.
const GRAVITY: f32 = 950.0;

/// A projectile class's values (from its defaults).
#[derive(Clone, Copy, Debug, Default)]
pub struct ProjectileStats {
    /// Speed (Unreal units/s).
    pub speed: f32,
    pub damage: f32,
    /// ShotgunBullet.ProcessTouch: damage x PenDamageReduction per zed; gone
    /// once damage / default damage <= PenDamageReduction / MaxPenetrations.
    pub max_penetrations: f32,
    pub pen_damage_reduction: f32,
    /// The projectile's HeadShotDamageMult (ProcessTouch).
    pub headshot_mult: f32,
    /// The damage type's HeadShotDamageMult (KFMonster.TakeDamage applies it
    /// again on headshots: a KF quirk, kept).
    pub damage_type_headshot_mult: f32,
    pub life_span: f32,
    /// NailGunProjectile Bounces (0 for pellets).
    pub bounces: u32,
}

/// Fire a projectile (KFShotgunFire.SpawnProjectile).
#[derive(Message, Clone, Copy, Debug)]
pub struct SpawnPlayerProjectile {
    /// Unreal world: StartProj.
    pub origin: Vec3,
    /// StartTrace (the eye): if a wall is between it and `origin`, the
    /// projectile starts at the wall (KFShotgunFire.DoFireEffect).
    pub trace_from: Vec3,
    pub dir: Vec3,
    pub stats: ProjectileStats,
    pub weapon: &'static str,
    /// Where its tracer starts (the weapon's tip); None: no tracer.
    pub tracer_start: Option<Vec3>,
}

#[derive(Component)]
struct PlayerProjectile {
    pos: Vec3,
    vel: Vec3,
    damage: f32,
    stats: ProjectileStats,
    weapon: &'static str,
    /// Zeds already hit (each once).
    hit: Vec<usize>,
    bounces_left: u32,
    falling: bool,
    age: f32,
    id: u32,
}

pub struct ProjectilePlugin;

impl Plugin for ProjectilePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<SpawnPlayerProjectile>()
            .add_systems(Update, (spawn_projectiles, move_projectiles).chain());
    }
}

/// Pellet tracers, cycled (KF spawns a KFTracer per pellet).
const PELLET_TRACERS: u32 = 32;

fn spawn_projectiles(
    mut commands: Commands,
    mut spawns: MessageReader<SpawnPlayerProjectile>,
    mut next_id: Local<u32>,
    spatial: SpatialQuery,
    zeds: Query<&Zed>,
    mut bullet_fx: MessageWriter<crate::bullet_fx::BulletFx>,
) {
    for s in spawns.read() {
        *next_id += 1;
        let to_bevy = |v: Vec3| coords::pos(v.to_array());
        let mut origin = s.origin;
        let (a, b) = (to_bevy(s.trace_from), to_bevy(s.origin));
        if let Ok(d) = Dir3::new(b - a)
            && let Some(h) = spatial.cast_ray(a, d, (b - a).length(), true, &crate::collision::world_filter())
        {
            origin = s.trace_from + (s.origin - s.trace_from).normalize_or_zero() * (h.distance / SCALE);
        }
        // The tracer flies along the first straight path, to the wall or to
        // the zed where the projectile will stop (zeds as they are now;
        // nails' bounces not drawn).
        if let Some(start) = s.tracer_start {
            let from = to_bevy(origin);
            let dir = coords::dir(s.dir.normalize_or_zero().to_array()).normalize_or_zero();
            if let Ok(d) = Dir3::new(dir) {
                let max = s.stats.speed * s.stats.life_span * SCALE;
                let wall = spatial.cast_ray(from, d, max, true, &crate::collision::world_filter()).map_or(max, |h| h.distance);
                let mut zed_t: Vec<f32> = zeds
                    .iter()
                    .filter(|z| z.health > 0.0)
                    .filter_map(|z| crate::combat::zed_hit(z, from, dir))
                    .filter(|&t| t < wall)
                    .collect();
                zed_t.sort_by(f32::total_cmp);
                let end_t = zed_t.get(penetration_limit(&s.stats) - 1).copied().unwrap_or(wall);
                bullet_fx.write(crate::bullet_fx::BulletFx {
                    shooter: crate::bullet_fx::Shooter::PlayerPellet((*next_id % PELLET_TRACERS) as u8),
                    start: Some(start),
                    hit: origin + s.dir.normalize_or_zero() * (end_t / SCALE),
                    into: s.dir,
                    impact: false,
                    tracer_speed: s.stats.speed,
                    min_distance: 0.0,
                });
            }
        }
        commands.spawn(PlayerProjectile {
            pos: origin,
            vel: s.dir.normalize_or_zero() * s.stats.speed,
            damage: s.stats.damage,
            stats: s.stats,
            weapon: s.weapon,
            hit: Vec::new(),
            bounces_left: s.stats.bounces,
            falling: false,
            age: 0.0,
            id: *next_id,
        });
    }
}

/// How many zeds a projectile passes before it stops (ProcessTouch's rule),
/// for drawing its tracer only as far as it goes.
pub fn penetration_limit(stats: &ProjectileStats) -> usize {
    let r = stats.pen_damage_reduction;
    if !(0.0..1.0).contains(&r) || stats.max_penetrations <= 0.0 {
        return 1;
    }
    let stop = r / stats.max_penetrations;
    let mut ratio = 1.0;
    for n in 1..=16 {
        ratio *= r;
        if ratio <= stop {
            return n;
        }
    }
    16
}

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn move_projectiles(
    mut commands: Commands,
    time: Res<Time>,
    spatial: SpatialQuery,
    mut projectiles: Query<(Entity, &mut PlayerProjectile)>,
    mut zeds: Query<&mut Zed>,
    mut kills: ResMut<crate::combat::KillCount>,
    mut bullet_fx: MessageWriter<crate::bullet_fx::BulletFx>,
    player: Query<&Transform, With<crate::camera::FlyCamera>>,
) {
    let dt = time.delta_secs();
    let attacker = player.single().map_or(Vec3::ZERO, |t| t.translation - Vec3::Y * crate::combat::PLAYER_EYE_HEIGHT * SCALE);
    for (entity, mut p) in &mut projectiles {
        p.age += dt;
        if p.age >= p.stats.life_span {
            runlog::kv("projectile_expired", &format!("id={} weapon={} age={:.2}", p.id, p.weapon, p.age));
            commands.entity(entity).despawn();
            continue;
        }
        if p.falling {
            p.vel.z -= GRAVITY * dt;
        }
        let step = p.vel * dt;
        let len = step.length();
        if len <= 0.0 {
            continue;
        }
        let dir_ue = step / len;
        let from = coords::pos(p.pos.to_array());
        let dir = coords::dir(dir_ue.to_array()).normalize_or_zero();
        let Ok(dir3) = Dir3::new(dir) else { continue };
        let world = spatial.cast_ray(from, dir3, len * SCALE, true, &crate::collision::world_filter());
        let world_t = world.map_or(len * SCALE, |h| h.distance);
        // Zeds along this step, before the wall, nearest first.
        let mut hits: Vec<(f32, Mut<Zed>)> = Vec::new();
        for z in &mut zeds {
            if z.health <= 0.0 || p.hit.contains(&z.id) {
                continue;
            }
            if let Some(t) = crate::combat::zed_hit(&z, from, dir)
                && t <= world_t
            {
                hits.push((t, z));
            }
        }
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut stopped = false;
        for (t, mut z) in hits {
            let point = from + dir * t;
            // ProcessTouch: x HeadShotDamageMult on a headshot; then
            // KFMonster.TakeDamage checks again and applies the damage
            // type's multiplier too.
            let head = crate::combat::is_headshot(&z, point, dir, 1.0);
            let damage = if head { p.damage * p.stats.headshot_mult } else { p.damage };
            z.last_hit = Some((point, dir));
            p.hit.push(z.id);
            runlog::kv(
                "projectile_hit",
                &format!(
                    "id={} weapon={} zed={} hit_number={} damage={damage:.1} headshot={head} flight_unreal={:.0}",
                    p.id,
                    p.weapon,
                    z.id,
                    p.hit.len(),
                    p.age * p.stats.speed
                ),
            );
            let source = crate::combat::HitSource { point, attacker, melee: false };
            crate::combat::damage_zed(&mut z, damage, head, p.stats.damage_type_headshot_mult, p.weapon, t, source, &mut kills);
            p.damage *= p.stats.pen_damage_reduction;
            if p.damage / p.stats.damage <= p.stats.pen_damage_reduction / p.stats.max_penetrations.max(1e-3) {
                stopped = true;
                break;
            }
        }
        if stopped {
            commands.entity(entity).despawn();
            continue;
        }
        match world {
            Some(h) => {
                let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
                let n = if h.normal.dot(dir) > 0.0 { -h.normal } else { h.normal };
                let n_ue = to_ue(n).normalize_or_zero();
                let hit_ue = p.pos + dir_ue * (h.distance / SCALE);
                if p.bounces_left > 0 {
                    // NailGunProjectile.HitWall: reflect at 0.65 speed, fall.
                    p.bounces_left -= 1;
                    p.vel = 0.65 * (p.vel - 2.0 * n_ue * p.vel.dot(n_ue));
                    p.pos = hit_ue + n_ue;
                    p.falling = true;
                    runlog::kv("projectile_bounce", &format!("id={} weapon={} bounces_left={}", p.id, p.weapon, p.bounces_left));
                    continue;
                }
                // HitWall: ImpactEffect (ROBulletHitEffect) at the wall.
                bullet_fx.write(crate::bullet_fx::BulletFx {
                    shooter: crate::bullet_fx::Shooter::Player,
                    start: None,
                    hit: hit_ue + 2.0 * n_ue,
                    into: -n_ue,
                    impact: true,
                    tracer_speed: p.stats.speed,
                    min_distance: 0.0,
                });
                runlog::kv(
                    "projectile_wall",
                    &format!("id={} weapon={} flight_unreal={:.0} zeds_hit={}", p.id, p.weapon, p.age * p.stats.speed, p.hit.len()),
                );
                commands.entity(entity).despawn();
            }
            None => p.pos += step,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(r: f32, max: f32) -> ProjectileStats {
        ProjectileStats {
            pen_damage_reduction: r,
            max_penetrations: max,
            ..default()
        }
    }

    #[test]
    fn pellets_stop_after_kf_penetration_rule() {
        // Shotgun / Benelli / Trenchgun (0.5, 2): 1 -> 0.5 -> 0.25 <= 0.25.
        assert_eq!(penetration_limit(&stats(0.5, 2.0)), 2);
        // Hunting shotgun (0.65): 0.65, 0.42, 0.27 <= 0.325.
        assert_eq!(penetration_limit(&stats(0.65, 2.0)), 3);
        // AA12, KSG, nails (0.75): 0.75, 0.56, 0.42, 0.32 <= 0.375.
        assert_eq!(penetration_limit(&stats(0.75, 2.0)), 4);
    }
}
