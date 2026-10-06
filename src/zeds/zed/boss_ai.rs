//! The Patriarch's own AI: his busy states (charge, chaingun, rocket, knockdown, healing), escaping and finding a place to heal.

use super::*;

/// A clear line between two points (Bevy space) through the level.
/// The Patriarch in state FireChaingun, one frame: turn to the player, run
/// the bursts (`boss::Chaingun`), play its animations and fire its shots.
/// Shot from closer than 100: charge instead (FireChaingun.TakeDamage).
#[allow(clippy::too_many_arguments)]
pub(super) fn boss_busy(
    z: &mut Zed,
    c: &ZedClass,
    t: &Transform,
    target: Vec3,
    player_velocity: Vec3,
    dt: f32,
    spatial: &SpatialQuery,
    player_damage: &mut MessageWriter<crate::game::combat::PlayerDamaged>,
    push: &mut MessageWriter<crate::player::walk::PlayerPush>,
    fireball: &mut MessageWriter<crate::zeds::fireball::SpawnFireball>,
    bullet_fx: &mut MessageWriter<crate::weapons::bullet_fx::BulletFx>,
) {
    let (Some(bc), Some(mut b)) = (c.boss.as_ref(), z.boss) else {
        z.state = ZedState::Chase;
        return;
    };
    // MakingEntrance: stand until Entrance is done, then InitialSneak
    // (cloaked). The laugh: stand until VictoryLaugh is done.
    if b.entrance.is_some() || b.laugh.is_some() {
        b.tick(dt);
        if b.entrance.is_some() && b.entrance_step(dt) {
            z.state = ZedState::Chase;
            z.sequence = None;
            // CloakBoss: not while zapped.
            z.cloaked = !z.zapped();
            z.cloak_dirty = true;
            z.router = Default::default();
            runlog::kv("boss_entrance", &format!("id={} end next=InitialSneak", z.id));
        } else if b.laugh.is_some() && b.laugh_step(dt) {
            z.state = ZedState::Chase;
            z.sequence = None;
            runlog::kv("boss_entrance", &format!("id={} end of=VictoryLaugh", z.id));
        }
        z.boss = Some(b);
        return;
    }
    // State KnockDown: when the animation is done, cloak and escape.
    if b.knockdown.is_some() {
        b.tick(dt);
        if b.knockdown_step(dt) {
            // State KnockDown, after the animation and CloakBoss.
            z.sound_events.push(ZedSound::Line(Line {
                sound: "KF_EnemiesFinalSnd.Patriarch.Kev_SaveMe",
                slot: crate::audio::mixer::Slot::Misc,
                volume: 2.0,
                radius: 500.0,
                no_override: false,
            }));
            z.state = ZedState::Chase;
            z.sequence = None;
            // CloakBoss: not while zapped.
            z.cloaked = !z.zapped();
            z.cloak_dirty = true;
            z.router = Default::default();
            runlog::kv("boss_escape", &format!("id={} start health={:.0}", z.id, z.health));
        }
        b.hit_from_close = false;
        z.boss = Some(b);
        return;
    }
    // State Healing: the Heal animation and its syringe notifies.
    if b.heal.is_some()
        && let Some((_, secs)) = bc.heal_anim
    {
        b.tick(dt);
        let (syringe, added, done) = b.heal_step(dt, secs);
        if syringe {
            if let Some(bone) = bc.syringe_bones.get(b.syringes - 1).copied().flatten() {
                z.hidden_bones.push(bone);
            }
            runlog::kv("boss_heal", &format!("id={} syringe={}", z.id, b.syringes));
        }
        if let Some(amount) = added {
            let before = z.health;
            z.health += amount;
            runlog::kv("boss_heal", &format!("id={} health={before:.0}->{:.0}", z.id, z.health));
        }
        if done {
            z.state = ZedState::Chase;
            z.sequence = None;
            runlog::kv("boss_heal", &format!("id={} done health={:.0} syringes={}", z.id, z.health, b.syringes));
        }
        b.hit_from_close = false;
        z.boss = Some(b);
        return;
    }
    // State FireMissile: turn to the player, fire when PreFireMissile ends.
    if let (Some(mut m), Some(anims)) = (b.missile, bc.missile_anims) {
        b.tick(dt);
        turn_toward(z, c, target, dt);
        match m.step(dt, anims[1].1) {
            Some(crate::zeds::boss::MissileEvent::Fire) => {
                z.sound_events.push(ZedSound::Rocket);
                shoot_fireball(z, c, t, crate::zeds::fireball::Projectile::BossRocket, bc.tip_bone, target, player_velocity, spatial, fireball);
                z.sequence = None;
                start_anim(z, Some(anims[1].0), false);
                b.missile = Some(m);
            }
            Some(crate::zeds::boss::MissileEvent::Done) => {
                b.missile = None;
                // Back to the DoorBashing loop if the rocket was at a door.
                z.state = if z.door_bash.is_some() { ZedState::DoorBashing } else { ZedState::Chase };
                z.sequence = None;
                runlog::kv("boss_missile", &format!("id={} done next_in={:.1}", z.id, b.missile_wait));
            }
            None => b.missile = Some(m),
        }
        b.hit_from_close = false;
        z.boss = Some(b);
        return;
    }
    let Some((mut mg, anims)) = b.chaingun.zip(bc.mg_anims) else {
        z.state = ZedState::Chase;
        return;
    };
    b.tick(dt);
    // FireChaingun.TakeDamage: only "shot from closer than 100" works. Its
    // "ChargeDamage > 200 within 500" half never fires (ChargeDamage is
    // always 0, see boss::BossState::decide), and its "a closer attacker
    // than the enemy" half needs a second player.
    if std::mem::take(&mut b.hit_from_close) {
        let roll = (z.random() % 1000) as f32 / 1000.0;
        b.end_chaingun(roll);
        let attacks = if z.random().is_multiple_of(2) { 1 } else { 2 };
        b.charge = Some(crate::zeds::boss::Charge { seconds: 0.0, attacks_left: attacks });
        z.state = ZedState::Chase;
        z.sequence = None;
        if let (Some(seq), Some(root)) = (bc.transition, c.fire_root_bone) {
            z.overlay = Some((seq, 0.0, root));
        }
        runlog::kv("boss_chaingun", &format!("id={} end reason=shot_from_close shots_left={}", z.id, mg.shots_left));
        runlog::kv("boss_charge", &format!("id={} start attacks={attacks} reason=shot_from_close", z.id));
        z.boss = Some(b);
        return;
    }
    // The controller turns him toward Focus (the player) or FocalPoint.
    let aim_at = if mg.has_focus() { target } else { b.mg_focal };
    let want_yaw = turn_toward(z, c, aim_at, dt);
    let tip = bc
        .tip_bone
        .and_then(|bone| c.model.bone_frame(&z.last_pose, bone))
        .map_or(ue_pos(z.centre), |f| world_axes(c, t, f).0);
    let tip_bevy = coords::pos(tip.to_array());
    // AnimEnd: LineOfSightTo and FastTrace from the tip to the enemy.
    let in_sight = sees(spatial, z.centre, target) && sees(spatial, tip_bevy, target);
    let seconds = anims.map(|a| a.1);
    let events = {
        let mut roll = || (z.random() % 10000) as f32 / 10000.0;
        mg.step(dt, in_sight, seconds, &mut roll)
    };
    if mg.has_focus() {
        b.mg_focal = target;
    }
    let mut done = false;
    for e in events {
        match e {
            crate::zeds::boss::MgEvent::Play(a) => {
                let i = match a {
                    crate::zeds::boss::MgAnim::Fire => 1,
                    crate::zeds::boss::MgAnim::End => 2,
                };
                z.sequence = None; // restart even if the same (FireMG again)
                start_anim(z, Some(anims[i].0), false);
                if a == crate::zeds::boss::MgAnim::End {
                    // FireChaingun.EndState: LastChainGunTime = now + 5 + FRand() x 10.
                    b.chaingun_wait = 5.0 + 10.0 * (z.random() % 1000) as f32 / 1000.0;
                    runlog::kv(
                        "boss_chaingun",
                        &format!("id={} end shots_left={} seconds={:.2} next_in={:.1}", z.id, mg.shots_left, mg.clock, b.chaingun_wait),
                    );
                }
            }
            crate::zeds::boss::MgEvent::Shoot => {
                boss_mg_shot(z, tip, aim_at, want_yaw, target, spatial, player_damage, push, bullet_fx, mg.shots_left);
            }
            crate::zeds::boss::MgEvent::Done => done = true,
        }
    }
    if done {
        b.chaingun = None;
        z.state = ZedState::Chase;
        z.sequence = None;
        runlog::kv("boss_chaingun", &format!("id={} done", z.id));
    } else {
        b.chaingun = Some(mg);
    }
    z.boss = Some(b);
}

/// The Patriarch in state Escaping, once per think: Escaping's Begin loop
/// (cloak again), SyrRetreat.FindHideSpot once, and BeginHealing on
/// arriving (or after ESCAPE_GIVE_UP, our addition). Returns where to run
/// (Bevy), or None when not escaping.
pub(super) fn boss_escape(z: &mut Zed, c: &ZedClass, player: Vec3, dt: f32, nav: &crate::world::nav::NavNetwork, spatial: &SpatialQuery) -> Option<Vec3> {
    let (Some(bc), Some(mut b)) = (c.boss.as_ref(), z.boss) else {
        return None;
    };
    b.escape?;
    if z.state != ZedState::Chase {
        return None;
    }
    if b.escape_step(dt, z.attack.is_some()) && !z.zapped() {
        z.cloaked = true;
        z.cloak_dirty = true;
        runlog::kv("boss_sneak", &format!("id={} cloak reason=escaping", z.id));
    }
    let mut e = b.escape.expect("checked");
    if e.goal.is_none() {
        // SyrRetreat.BeginState clears Enemy; SeePlayer sets it again. Seen:
        // FindHideSpot scores points; not: FindRandomDest.
        let enemy = sees(spatial, z.centre, player).then_some(player);
        let mut seed = z.random() | 1;
        e.goal = find_hide_spot(nav, spatial, z.centre, enemy, &mut seed);
        match e.goal {
            Some(g) => runlog::kv(
                "boss_escape",
                &format!(
                    "id={} hide_spot={} distance_unreal={:.0} player_known={} spot_seen_by_player={}",
                    z.id,
                    nav.points[g].name,
                    (nav.points[g].pos - z.centre).length() / SCALE,
                    enemy.is_some(),
                    sees(spatial, nav.points[g].pos, player)
                ),
            ),
            None => runlog::kv("boss_escape", &format!("id={} hide_spot=none", z.id)),
        }
    }
    let goal = e.goal.map(|g| nav.points[g].pos);
    let arrived = goal.is_none_or(|g| {
        let d = g - z.centre;
        d.with_y(0.0).length() / SCALE <= crate::world::nav::HUNT_RADIUS + 8.0 && (d.y / SCALE).abs() <= 2.0 * c.collision_height
    });
    let gave_up = e.seconds > crate::zeds::boss::ESCAPE_GIVE_UP;
    if (arrived || gave_up)
        && z.attack.is_none()
        && let Some((seq, _)) = bc.heal_anim
    {
        b.begin_healing();
        z.boss = Some(b);
        z.cloaked = false;
        z.cloak_dirty = true;
        z.overlay = None;
        z.state = ZedState::BossBusy;
        z.sequence = None;
        start_anim(z, Some(seq), false);
        let why = if gave_up && !arrived { "gave_up" } else { "arrived" };
        runlog::kv(
            "boss_heal",
            &format!("id={} start reason={why} escape_seconds={:.1} health={:.0} player_sees={}", z.id, e.seconds, z.health, sees(spatial, z.centre, player)),
        );
        return None;
    }
    b.escape = Some(e);
    z.boss = Some(b);
    goal
}

/// SyrRetreat.FindHideSpot: of the navigation points within 2500 that the
/// enemy cannot see and that have a path, the best by distance from the
/// enemy / max(distance from him / 800, 1.5), divided by 10 when the
/// direction from the point to the enemy is not within 0.2 (dot) of his own
/// direction to the enemy (a point past or beside the enemy). No enemy:
/// FindRandomDest, a random point with a path. "Has a path": reachable over
/// the network from a point near him that he can see (our stand-in for
/// FindPathToward).
pub(super) fn find_hide_spot(nav: &crate::world::nav::NavNetwork, spatial: &SpatialQuery, pawn: Vec3, enemy: Option<Vec3>, seed: &mut u32) -> Option<usize> {
    let mut reachable = vec![false; nav.points.len()];
    let mut queue: Vec<usize> = nav
        .near(pawn, 800.0 * SCALE)
        .into_iter()
        .filter(|(i, _)| sees(spatial, pawn, nav.points[*i].pos))
        .map(|(i, _)| i)
        .collect();
    for &i in &queue {
        reachable[i] = true;
    }
    while let Some(u) = queue.pop() {
        for &(v, _) in &nav.links[u] {
            if !reachable[v] {
                reachable[v] = true;
                queue.push(v);
            }
        }
    }
    let Some(enemy) = enemy else {
        let all: Vec<usize> = (0..nav.points.len()).filter(|&i| reachable[i]).collect();
        if all.is_empty() {
            return None;
        }
        *seed ^= *seed << 13;
        *seed ^= *seed >> 17;
        *seed ^= *seed << 5;
        return Some(all[*seed as usize % all.len()]);
    };
    let enemy_dir = (enemy - pawn).normalize_or_zero();
    let mut best: Option<(usize, f32)> = None;
    for (i, n) in nav.points.iter().enumerate() {
        let mdist = (n.pos - pawn).length() / SCALE;
        if mdist >= 2500.0 || !reachable[i] || sees(spatial, n.pos, enemy) {
            continue;
        }
        let mut score = (n.pos - enemy).length() / SCALE / (mdist / 800.0).max(1.5);
        if enemy_dir.dot((enemy - n.pos).normalize_or_zero()) < 0.2 {
            score /= 10.0;
        }
        if best.is_none_or(|(_, b)| b < score) {
            best = Some((i, score));
        }
    }
    best.map(|(i, _)| i)
}
