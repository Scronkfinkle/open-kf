//! Special attacks: the Bloat's vomit, the Siren's scream, the Husk's and the Patriarch's fireballs and rockets, the chaingun's shots, bashing welded doors.

use super::*;

/// ZombieBloat.SpawnTwoShots: three KFBloatVomit globs from 30 ahead and 64
/// up (x DrawScale), aimed at the target (AdjustAim; KF also leads a moving
/// target: not done), the side ones half a CollisionRadius out and turned
/// 1200 yaw units (6.6 degrees).
pub(super) fn spawn_two_shots(z: &Zed, c: &ZedClass, target: Vec3, out: &mut MessageWriter<crate::zeds::vomit::SpawnVomit>) {
    let k = std::f32::consts::TAU / 65536.0;
    let a = z.yaw * k;
    let (x, y) = (Vec3::new(a.cos(), a.sin(), 0.0), Vec3::new(-a.sin(), a.cos(), 0.0));
    let start = ue_pos(z.centre) + (x * 30.0 + Vec3::Z * 64.0) * c.draw_scale;
    let to = ue_pos(target) - start;
    let (yaw, pitch) = (to.y.atan2(to.x), to.z.atan2(to.truncate().length()));
    for (side, turn) in [(0.0, 0.0), (-0.5, -1200.0), (0.5, 1200.0)] {
        let yw = yaw + turn * k;
        let dir = Vec3::new(pitch.cos() * yw.cos(), pitch.cos() * yw.sin(), pitch.sin());
        out.write(crate::zeds::vomit::SpawnVomit {
            at: start + y * side * c.collision_radius,
            velocity: dir * crate::zeds::vomit::SPEED,
            zed_id: z.id,
        });
    }
    runlog::kv(
        "bloat_shots",
        &format!("id={} start_unreal=({:.0}, {:.0}, {:.0}) pitch_deg={:.1}", z.id, start.x, start.y, start.z, pitch.to_degrees()),
    );
}

/// One ZombieSiren.SpawnTwoShots: HurtRadius(ScreamDamage, ScreamRadius,
/// ScreamForce) on the player, if within the radius and in sight:
/// damageScale = 1 - (distance - 20) / ScreamRadius, damage x scale (whole
/// points), momentum damageScale x ScreamForce along the line from her to
/// the player (negative: a pull toward her). The screen shake and blur
/// (DoShakeEffect) are player/hit_cam.rs.
#[allow(clippy::too_many_arguments)]
pub(super) fn scream_pulse(
    z: &Zed,
    damage: f32,
    radius: f32,
    force: f32,
    target: Vec3,
    spatial: &SpatialQuery,
    out: &mut MessageWriter<crate::game::combat::PlayerDamaged>,
    push: &mut MessageWriter<crate::player::walk::PlayerPush>,
) {
    let (from, to) = (ue_pos(z.centre), ue_pos(target));
    let dist = (to - from).length().max(1.0);
    if dist - PLAYER_RADIUS > radius {
        return;
    }
    if let Ok(d) = Dir3::new(target - z.centre)
        && spatial
            .cast_ray(z.centre, d, (target - z.centre).length(), true, &crate::world::collision::world_filter())
            .is_some()
    {
        runlog::kv("siren_scream", &format!("id={} distance_unreal={dist:.0} blocked=true", z.id));
        return;
    }
    let scale = 1.0 - ((dist - PLAYER_RADIUS) / radius).max(0.0);
    let amount = (scale * damage).floor();
    if amount > 0.0 {
        out.write(crate::game::combat::PlayerDamaged {
            amount,
            // SirenScreamDamage: bArmorStops false.
            armor_stops: false,
            zed_id: z.id,
            kind: crate::game::combat::HurtKind::Plain,
            dam_type: crate::game::combat::DamType::SirenScream,
            source: Some(z.centre),
            dam: None,
        });
    }
    let momentum = (to - from) / dist * (scale * force);
    push.write(crate::player::walk::PlayerPush { momentum });
    runlog::kv(
        "siren_scream",
        &format!(
            "id={} distance_unreal={dist:.0} scale={scale:.2} damage={amount} momentum_unreal=({:.0}, {:.0}, {:.0})",
            z.id, momentum.x, momentum.y, momentum.z
        ),
    );
}

/// FireChaingun.FireMGShot: from the tip at the aim point (or straight
/// ahead if he still has to turn more than 2000), VRand() x 0.06 spread, a
/// 10000-unit trace; the player takes MGDamage + Rand(3) in whole points
/// and momentum 500 along the shot. `tip` in Unreal units, the rest Bevy.
#[allow(clippy::too_many_arguments)]
pub(super) fn boss_mg_shot(
    z: &mut Zed,
    tip: Vec3,
    aim_at: Vec3,
    want_yaw: f32,
    player: Vec3,
    spatial: &SpatialQuery,
    player_damage: &mut MessageWriter<crate::game::combat::PlayerDamaged>,
    push: &mut MessageWriter<crate::player::walk::PlayerPush>,
    bullet_fx: &mut MessageWriter<crate::weapons::bullet_fx::BulletFx>,
    shots_left: i32,
) {
    let yaw_err = (want_yaw - z.yaw).rem_euclid(65536.0);
    let turning = !(yaw_err < 2000.0 || yaw_err > 63535.0);
    let aim = if turning {
        let a = z.yaw * std::f32::consts::TAU / 65536.0;
        Vec3::new(a.cos(), a.sin(), 0.0)
    } else {
        (ue_pos(aim_at) - tip).normalize_or_zero()
    };
    // VRand(): a random unit vector.
    let spread = loop {
        let mut r = || (z.random() % 20001) as f32 / 10000.0 - 1.0;
        let v = Vec3::new(r(), r(), r());
        if v.length_squared() > 1e-4 && v.length_squared() <= 1.0 {
            break v.normalize();
        }
    };
    let dir = (aim + spread * crate::zeds::boss::MG_SPREAD).normalize_or_zero();
    let origin = coords::pos(tip.to_array());
    let dir_bevy = coords::dir(dir.to_array());
    let max = crate::zeds::boss::MG_RANGE * SCALE;
    let world = Dir3::new(dir_bevy)
        .ok()
        .and_then(|d| spatial.cast_ray(origin, d, max, true, &crate::world::collision::world_filter()))
        .map(|h| h.distance);
    let on_player = crate::game::combat::ray_cylinder(origin, dir_bevy, player, PLAYER_RADIUS * SCALE, PLAYER_HALF_HEIGHT * SCALE)
        .filter(|d| *d <= world.unwrap_or(max));
    let what = if on_player.is_some() {
        let amount = (crate::zeds::boss::MG_DAMAGE + (z.random() % 3) as f32).floor();
        player_damage.write(crate::game::combat::PlayerDamaged {
            amount,
            armor_stops: true,
            zed_id: z.id,
            kind: crate::game::combat::HurtKind::Plain,
            // ZombieBoss chaingun: Class'DamageType'.
            dam_type: crate::game::combat::DamType::Other,
            source: Some(origin),
            dam: None,
        });
        push.write(crate::player::walk::PlayerPush { momentum: dir * crate::zeds::boss::MG_MOMENTUM });
        format!("player damage={amount}")
    } else if world.is_some() {
        "world".to_string()
    } else {
        "nothing".to_string()
    };
    // AddTraceHitFX (only when the trace hit something: A != None): the
    // muzzle flash on `tip`, a tracer at 10000 and ROBulletHitEffect at the
    // hit point, rotated along the shot, whatever was hit (the player too).
    if let Some(d) = on_player.or(world) {
        let hit = ue_pos(origin + dir_bevy * d);
        z.mg_flash_shots += 1;
        bullet_fx.write(crate::weapons::bullet_fx::BulletFx {
            shooter: crate::weapons::bullet_fx::Shooter::Zed(z.id),
            start: Some(tip),
            hit,
            into: (hit - tip).normalize_or_zero(),
            impact: true,
            tracer_speed: crate::zeds::boss::MG_TRACER_SPEED,
            min_distance: 10.0,
        });
    }
    runlog::kv("boss_mg_shot", &format!("id={} left={shots_left} hit={what} turning={turning}", z.id));
}

/// ZombieHusk.SpawnTwoShots: a HuskFireProjectile from the Barrel bone,
/// aimed by HuskZombieController.AdjustAim: lead the target by its velocity
/// x distance / speed x min(1, 0.7 + 0.6 FRand()) (no higher than the target),
/// and, with bTrySplash at Skill 2 (assumed for Normal) half the time when
/// the target is not more than 19 above him (the Husk only: the Patriarch's
/// rocket has bTrySplash off), at the floor under the target;
/// otherwise its middle, else its head, whichever is in sight. Aim error and
/// the wall checks are not done.
#[allow(clippy::too_many_arguments)]
pub(super) fn shoot_fireball(
    z: &mut Zed,
    c: &ZedClass,
    t: &Transform,
    kind: crate::zeds::fireball::Projectile,
    bone: Option<usize>,
    target: Vec3,
    target_velocity: Vec3,
    spatial: &SpatialQuery,
    out: &mut MessageWriter<crate::zeds::fireball::SpawnFireball>,
) {
    // bTrySplash (aim at the feet): the Husk's fireball; the Patriarch's
    // rocket has it off.
    let try_splash = kind == crate::zeds::fireball::Projectile::HuskFire;
    let start = bone
        .and_then(|b| c.model.bone_frame(&z.last_pose, b))
        .map_or(ue_pos(z.centre), |f| world_axes(c, t, f).0);
    let tgt = ue_pos(target);
    let dist = (tgt - ue_pos(z.centre)).length();
    let lead = (0.7 + 0.6 * (z.random() % 1000) as f32 / 1000.0).min(1.0);
    let mut spot = tgt + target_velocity * (lead * dist / kind.speed());
    spot.z = spot.z.min(tgt.z);
    let visible = |p: Vec3| sees(spatial, coords::pos(start.to_array()), coords::pos(p.to_array()));
    let feet = try_splash && (z.random() % 1000) as f32 / 1000.0 > 0.5 && ue_pos(z.centre).z + 19.0 >= tgt.z;
    let mut aim = "middle";
    let mut clean = false;
    if feet {
        let from = coords::pos(spot.to_array());
        if let Some(h) = spatial.cast_ray(from, Dir3::NEG_Y, (PLAYER_HALF_HEIGHT + 10.0) * SCALE, true, &crate::world::collision::world_filter()) {
            let floor = spot - Vec3::Z * (h.distance / SCALE) + Vec3::Z * 3.0;
            if visible(floor) {
                spot = floor;
                clean = true;
                aim = "feet";
            }
        }
    }
    if !clean {
        spot.z = tgt.z;
        if !visible(spot) {
            spot.z = tgt.z + 0.9 * PLAYER_HALF_HEIGHT;
            aim = "head";
        }
    }
    let dir = (spot - start).normalize_or_zero();
    out.write(crate::zeds::fireball::SpawnFireball {
        at: start,
        dir,
        zed_id: z.id,
        kind,
    });
    runlog::kv(
        if try_splash { "husk_shot" } else { "boss_rocket_shot" },
        &format!(
            "id={} aim={aim} start_unreal=({:.0}, {:.0}, {:.0}) spot_unreal=({:.0}, {:.0}, {:.0}){}",
            z.id,
            start.x,
            start.y,
            start.z,
            spot.x,
            spot.y,
            spot.z,
            if try_splash { format!(" next_in={:.1}", z.ranged_wait) } else { String::new() }
        ),
    );
}

/// A door's Location (Unreal units) as an array, for aiming.
pub(super) fn d_pos(d: &crate::world::door::Door) -> [f32; 3] {
    d.location()
}

/// KFMonsterController.DoorBashing for one frame. The loop: while the
/// door is sealed, visible and not bZombiesIgnore, AttackDoor, wait for
/// the animation (polled every 0.25 s), Sleep(0.1); after each, a zed of
/// Intelligence BRAINS_Mammal or more leaves for an enemy it can reach.
/// When the loop ends: WhatToDoNext (back to the chase).
///
/// DoorAttack by class: KFMonster plays the full-body DoorBash, whose
/// ClawDamageTarget notifies each hit the door for MeleeDamage -5% ..
/// +5%. ZombieBloat / ZombieHusk with bDistanceAttackingDoor (and a head):
/// ZombieBarf / ShootBurns, whose SpawnTwoShots each do 22 to the door.
/// ZombieSiren (with a head): Siren_Scream, each SpawnTwoShots
/// ScreamDamage x 0.6 to the door (nothing while zapped). ZombieBoss:
/// PreFireMissile and a rocket at the door (state FireMissile).
pub(super) fn door_bashing(
    z: &mut Zed,
    c: &ZedClass,
    dt: f32,
    doors: &crate::world::door::Doors,
    spatial: &SpatialQuery,
    target: Vec3,
    hits: &mut MessageWriter<crate::world::door::ZedDoorHit>,
) {
    let Some(mut b) = z.door_bash else {
        z.state = ZedState::Chase;
        return;
    };
    let leave = |z: &mut Zed, why: &str| {
        runlog::kv("zed_door_bash", &format!("id={} door={} end reason={why}", z.id, b.door));
        z.door_bash = None;
        z.state = ZedState::Chase;
        z.sequence = None;
    };
    // The loop's ActorReachable(Enemy) check, with doors in the way.
    let reachable = |z: &Zed| {
        let hunt = crate::world::nav::hunt_size(c.collision_radius, c.collision_height);
        let touch = crate::world::nav::HUNT_RADIUS + PLAYER_RADIUS;
        c.intelligence >= 2 && crate::world::nav::probe_with(spatial, crate::world::collision::zed_filter(), z.centre, target, touch, hunt.0, hunt.1).is_ok()
    };
    if b.in_anim {
        b.anim_time += dt;
        let Some(seq) = z.sequence else {
            b.in_anim = false;
            z.door_bash = Some(b);
            return;
        };
        let len = c.model.length(seq).max(1.0);
        let p = z.frame / len;
        let times: Vec<f32> = if b.ranged {
            c.ranged_shots.clone()
        } else {
            c.model.notifies(seq).iter().filter(|n| n.name.eq_ignore_ascii_case("ClawDamageTarget")).map(|n| n.time).collect()
        };
        for (i, at) in times.iter().enumerate().take(8) {
            if p >= *at && b.hits & (1 << i) == 0 {
                b.hits |= 1 << i;
                let (damage, kind) = if !b.ranged {
                    let roll = (z.random() % 1000) as f32 / 1000.0;
                    (if z.melee_damage > 1.0 { z.melee_damage * 0.95 + z.melee_damage * 0.1 * roll } else { z.melee_damage }, "claw")
                } else if let Some((scream, _, _)) = c.scream {
                    if z.zapped() {
                        continue;
                    }
                    (scream * 0.6, "scream")
                } else {
                    // DamTypeVomit for both the Bloat and the Husk.
                    (22.0, "ranged")
                };
                hits.write(crate::world::door::ZedDoorHit { door: b.door, damage, zed: z.id, kind });
            }
        }
        // The ranged animation's own effects (the vomit jet, the scream).
        if b.ranged {
            for (at, effect) in c.ranged_effects.iter().take(8) {
                if p >= *at && p - dt * c.model.rate(seq) / len < *at {
                    z.pending_fx.push(effect.clone());
                }
            }
        }
        if z.frame >= len - 0.5 {
            // While(bShotAnim) Sleep(0.25), then Sleep(0.1).
            b.in_anim = false;
            b.wait = (b.anim_time / 0.25).ceil() * 0.25 - b.anim_time + 0.1;
            z.door_bash = Some(b);
            if reachable(z) {
                return leave(z, "enemy_reachable");
            }
            return;
        }
        z.door_bash = Some(b);
        return;
    }
    if b.wait > 0.0 {
        b.wait -= dt;
        z.door_bash = Some(b);
        return;
    }
    let Some(d) = doors.doors.get(b.door) else {
        return leave(z, "no_door");
    };
    if d.hidden {
        return leave(z, "door_broken");
    }
    if !d.sealed {
        return leave(z, "door_unsealed");
    }
    if d.info.zombies_ignore {
        return leave(z, "zombies_ignore");
    }
    // AttackDoor.
    let (seq, ranged) = if let (Some(bc), Some(mut boss)) = (c.boss.as_ref(), z.boss) {
        let Some(anims) = bc.missile_anims else {
            return leave(z, "no_missile_animation");
        };
        // PreFireMissile (full body), state FireMissile; boss_busy aims
        // at the door and comes back here when it is done.
        boss.missile = Some(crate::zeds::boss::Missile::start(anims[0].1));
        z.boss = Some(boss);
        z.state = ZedState::BossBusy;
        z.sequence = None;
        start_anim(z, Some(anims[0].0), false);
        b.wait = 0.1;
        z.door_bash = Some(b);
        runlog::kv("zed_door_attack", &format!("id={} door={} kind=rocket", z.id, d.info.name));
        return;
    } else if c.scream.is_some() {
        (if z.decapitated { None } else { c.ranged_anim }, true)
    } else if b.distance && !z.decapitated && c.ranged_anim.is_some() {
        (c.ranged_anim, true)
    } else {
        (c.door_bash, false)
    };
    let Some(seq) = seq else {
        // ZombieSiren.DoorAttack does nothing without a head; the loop
        // goes on (Sleep(0.1)) while the door stays sealed.
        b.wait = 0.1;
        z.door_bash = Some(b);
        if reachable(z) {
            leave(z, "enemy_reachable");
        }
        return;
    };
    b.in_anim = true;
    b.ranged = ranged;
    b.hits = 0;
    b.anim_time = 0.0;
    z.door_bash = Some(b);
    z.sequence = None;
    start_anim(z, Some(seq), false);
    runlog::kv(
        "zed_door_attack",
        &format!("id={} door={} kind={} sequence={}", z.id, d.info.name, if ranged { "ranged" } else { "bash" }, c.model.sequence_name(seq).unwrap_or("?")),
    );
}

/// The ClawDamageTarget notify times (0..1) of an attack animation, in
/// order. KF's melee damage comes only from these AnimNotify_Script
/// notifies in the zed's MeshAnimation: each one calls ClawDamageTarget
/// (MeleeDamage -5% .. +5%, then MeleeDamageTarget's reach check), so an
/// attack can hit more than once: the Gorefast's GoreAttack1 at 0.256 and
/// 0.445, the Clot's Claw at 0.354 and 0.650, the Fleshpound's
/// PoundAttack2 four times. An animation without one does no damage.
pub(super) fn claw_times(model: &SkinnedModel, seq: usize) -> Vec<f32> {
    let mut times: Vec<f32> = model
        .notifies(seq)
        .iter()
        .filter(|n| n.name.eq_ignore_ascii_case("ClawDamageTarget"))
        .map(|n| n.time)
        .collect();
    times.sort_by(f32::total_cmp);
    times
}

/// The notifies an animation at `progress` (0..1) has passed and not yet
/// fired (bit i of `fired` set = notify i done; at most 8). Returns the
/// updated bits and the indices due now, in order: a long frame can pass
/// two at once, and each still fires (UE2 runs every notify crossed).
pub(super) fn due_notifies(times: &[f32], progress: f32, fired: u8) -> (u8, Vec<usize>) {
    let mut bits = fired;
    let mut due = Vec::new();
    for (i, at) in times.iter().enumerate().take(8) {
        if progress >= *at && bits & (1 << i) == 0 {
            bits |= 1 << i;
            due.push(i);
        }
    }
    (bits, due)
}
