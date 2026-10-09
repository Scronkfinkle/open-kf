//! The zed AI: hunting the player over the navigation network, attacking, hit reactions, falling, jump pads, the specimens' special rules.

use super::*;

/// Turns a zed toward `at` (Bevy) at its RotationRate; returns the wanted yaw.
pub(super) fn turn_toward(z: &mut Zed, c: &ZedClass, at: Vec3, dt: f32) -> f32 {
    let to = (at - z.centre).with_y(0.0);
    let want_yaw = yaw_of(to);
    if to.length() / SCALE > 1.0 {
        let mut delta = (want_yaw - z.yaw).rem_euclid(65536.0);
        if delta > 32768.0 {
            delta -= 65536.0;
        }
        let step = c.turn_rate * dt;
        z.yaw = (z.yaw + delta.clamp(-step, step)).rem_euclid(65536.0);
    }
    want_yaw
}

/// Marks the asked zed; `think_and_move` starts the animation.
pub(super) fn boss_actions(mut actions: MessageReader<BossAction>, mut zeds: Query<&mut Zed>) {
    for a in actions.read() {
        let (BossAction::Entrance(id) | BossAction::Laugh(id)) = *a;
        let Some(mut z) = zeds.iter_mut().find(|z| z.id == id) else { continue };
        let Some(mut b) = z.boss else { continue };
        match a {
            BossAction::Entrance(_) => b.pending_entrance = true,
            BossAction::Laugh(_) => b.pending_laugh = true,
        }
        z.boss = Some(b);
    }
}

/// Doors and game messages `think_and_move` uses.
#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct ZedWorld<'w, 's> {
    doors: Res<'w, crate::world::door::Doors>,
    door_colliders: Query<'w, 's, &'static crate::world::door::DoorCollider>,
    door_hits: MessageWriter<'w, crate::world::door::ZedDoorHit>,
    door_blasts: MessageWriter<'w, crate::world::door::DoorBlast>,
    clear_zeds: MessageReader<'w, 's, crate::game::waves::ClearZeds>,
    kill_stuck: MessageReader<'w, 's, crate::game::waves::KillStuckZed>,
    glass: Query<'w, 's, &'static crate::world::glass::GlassCollider>,
    glass_bumps: MessageWriter<'w, crate::world::glass::GlassBump>,
    player_zone: Res<'w, crate::world::zones::PlayerZone>,
    level_damage: MessageReader<'w, 's, crate::player::pain::LevelDamageZed>,
    boss_death: MessageReader<'w, 's, crate::game::waves::BossDied>,
    match_over: Option<Res<'w, crate::game::end_game::MatchOver>>,
    /// Multiplayer host: the other players' pawns, and Clot grabs on them.
    remote: Res<'w, crate::game::combat::RemotePlayers>,
    grabs: MessageWriter<'w, crate::game::combat::RemoteGrab>,
}

/// A player the zeds can hunt: this game's own, or (network host) another
/// player's pawn.
#[derive(Clone, Copy, Debug)]
pub(super) struct Prey {
    /// None: this game's own player.
    pub peer: Option<u64>,
    /// Cylinder centre, Bevy space.
    pub centre: Vec3,
    /// Unreal units per second.
    pub velocity: Vec3,
    pub alive: bool,
    /// A walking pawn (a flying camera does not block or get grabbed).
    pub walking: bool,
}

/// Which player a zed hunts (network host only; single player always has
/// one). KFMonsterController.FindNewEnemy without threat assessment: the
/// nearest living player, chosen when the zed has no enemy, its enemy died,
/// or (WhatToDoNext: `!EnemyVisible()`) it cannot see it, checked here
/// every 0.5 s (KF re-decides at each WhatToDoNext; the interval is ours).
/// SetEnemy when a player hurts it: kept on the old enemy if its
/// Intelligence is BRAINS_Mammal or more and the old one is in sight and
/// closer. Returns the index into `prey`.
pub(super) fn choose_enemy(z: &mut Zed, c: &ZedClass, prey: &[Prey], spatial: &SpatialQuery, dt: f32) -> usize {
    let eye = z.centre + Vec3::Y * c.collision_height * 0.8 * SCALE;
    let centre = z.centre;
    let dist2 = |i: usize| (prey[i].centre - centre).length_squared();
    let nearest = || (0..prey.len()).filter(|&i| prey[i].alive).min_by(|&a, &b| dist2(a).total_cmp(&dist2(b)));
    let current = prey.iter().position(|p| p.peer == z.net.enemy).filter(|&i| prey[i].alive && z.net.enemy_chosen);
    let mut pick = current;
    let mut reason = "";
    if let Some(by) = z.net.provoked_by.take()
        && let Some(i) = prey.iter().position(|p| p.peer == by && p.alive)
        && Some(i) != current
    {
        let keep = c.intelligence >= 2 && current.is_some_and(|cur| dist2(cur) < dist2(i) && sees(spatial, eye, prey[cur].centre));
        if !keep {
            pick = Some(i);
            reason = "hurt_by";
        }
    }
    z.net.enemy_check -= dt;
    if pick.is_none() {
        pick = nearest();
        reason = if z.net.enemy_chosen { "enemy_dead" } else { "first" };
    } else if reason.is_empty() && z.net.enemy_check <= 0.0 {
        z.net.enemy_check = 0.5;
        if let Some(cur) = pick
            && !sees(spatial, eye, prey[cur].centre)
            && let Some(n) = nearest()
            && n != cur
        {
            pick = Some(n);
            reason = "not_visible";
        }
    }
    let i = pick.unwrap_or(0);
    if prey[i].peer != z.net.enemy || !z.net.enemy_chosen {
        runlog::kv(
            "zed_enemy",
            &format!(
                "id={} from={} to={} reason={reason} distance_unreal={:.0}",
                z.id,
                if z.net.enemy_chosen { z.net.enemy.map_or("local".to_string(), |p| p.to_string()) } else { "none".to_string() },
                prey[i].peer.map_or("local".to_string(), |p| p.to_string()),
                dist2(i).sqrt() / SCALE
            ),
        );
        z.net.enemy = prey[i].peer;
        z.net.enemy_chosen = true;
    }
    i
}

/// KFMonsterController MoanTime: the next moan (also for puppets, whose
/// moans are not sent).
fn moan_tick(z: &mut Zed, now: f32) {
    if z.moan_at < 0.0 {
        z.moan_at = (now + 2.0 + 36.0 * (z.random() % 1000) as f32 / 1000.0).floor();
    } else if now > z.moan_at {
        z.moan_at = (now + 12.0 + 8.0 * (z.random() % 1000) as f32 / 1000.0).floor();
        let busy = z.boss.is_some() && (z.attack.is_some() || z.state == ZedState::BossBusy);
        if !z.decapitated && !busy {
            z.sound_events.push(ZedSound::Moan);
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)] // Bevy system parameters
pub(super) fn think_and_move(
    mut commands: Commands,
    time: Res<Time>,
    active: Res<ZedsActive>,
    classes: Option<Res<ZedClasses>>,
    spatial: SpatialQuery,
    player: Query<(&Transform, Option<&Walker>), With<FlyCamera>>,
    mut zeds: Query<(Entity, &mut Zed, &mut Transform, Option<&RagdollState>), Without<FlyCamera>>,
    mut player_damage: MessageWriter<crate::game::combat::PlayerDamaged>,
    (mut vomit, mut push, mut fireball, mut bullet_fx, mut scream_shake): (
        MessageWriter<crate::zeds::vomit::SpawnVomit>,
        MessageWriter<crate::player::walk::PlayerPush>,
        MessageWriter<crate::zeds::fireball::SpawnFireball>,
        MessageWriter<crate::weapons::bullet_fx::BulletFx>,
        MessageWriter<crate::player::hit_cam::SirenScreamShake>,
    ),
    mut kills: ResMut<crate::game::combat::KillCount>,
    (mut pinned, vet, player_health): (ResMut<crate::game::combat::PlayerPinned>, Res<crate::game::perks::Veterancy>, Res<crate::game::combat::PlayerHealth>),
    nav: Res<crate::world::nav::NavNetwork>,
    script: Res<crate::weapons::weapon::ScriptedInput>,
    frames: Res<bevy::diagnostic::FrameCount>,
    mut world: ZedWorld,
    mut log_timer: Local<f32>,
) {
    let ZedWorld { doors, door_colliders, door_hits, door_blasts, clear_zeds, kill_stuck, glass, glass_bumps, player_zone, level_damage, boss_death, match_over, remote, grabs } = &mut world;
    let boss_died = boss_death.read().count() > 0;
    // CheckEndGame: every controller goes to GameEnded (P.GameHasEnded);
    // KFMonster.TurnOff does nothing, so the bodies stay where they are.
    let game_ended = match_over.as_ref().is_some_and(|m| m.active());
    let level_hits: Vec<(usize, f32, &'static str)> = level_damage.read().map(|d| (d.zed, d.amount, d.cause)).collect();
    let player_zone = **player_zone;
    let stuck: Vec<usize> = kill_stuck.read().map(|k| k.0).collect();
    let (doors, door_colliders) = (&*doors, &*door_colliders);
    let Some(classes) = classes else {
        return;
    };
    // Test action "hurt_zeds": 100 damage to every living zed (no hit
    // reaction), to test rules that depend on health.
    let hurt = script.0.iter().any(|(f, a)| *f == frames.0 && a == "hurt_zeds");
    // Test action "zap_zeds": SetZapped(10) on every living zed.
    let zap_all = script.0.iter().any(|(f, a)| *f == frames.0 && a == "zap_zeds");
    // Test action "kill_zeds": every living zed dies (wave tests); also a
    // wave game's restart.
    let kill_all = script.0.iter().any(|(f, a)| *f == frames.0 && a == "kill_zeds") || clear_zeds.read().count() > 0;
    // Test action "kill_near_zeds": zeds within 500 units of the player die
    // (the ones that reached you), others live on (cleanup tests).
    let kill_near = script.0.iter().any(|(f, a)| *f == frames.0 && a == "kill_near_zeds");
    // Test action "kill_boss": the Patriarch dies, others live (G3a tests).
    let kill_boss = script.0.iter().any(|(f, a)| *f == frames.0 && a == "kill_boss");
    let Ok((pt, walker)) = player.single() else {
        return;
    };
    // Player cylinder centre: the walker's, or below the flying camera.
    let local_target = walker.map_or(pt.translation - Vec3::Y * PLAYER_EYE * SCALE, |w| w.center);
    let local_alive = !player_health.dead && player_health.health > 0.0;
    // Who decides whether cloaked zeds are spotted: this machine's player
    // (KF: LocalKFHumanPawn).
    let viewer = CloakViewer { location: local_target, alive: local_alive, vet: vet.vet };
    // The players to hunt: this game's own first, then (network host) the
    // other players' pawns.
    let mut prey = vec![Prey {
        peer: None,
        centre: local_target,
        velocity: walker.map_or(Vec3::ZERO, |w| ue_dir(w.velocity) / SCALE),
        alive: local_alive,
        walking: walker.is_some(),
    }];
    prey.extend(remote.0.iter().map(|r| Prey { peer: Some(r.peer), centre: r.centre, velocity: ue_dir(r.velocity) / SCALE, alive: r.alive, walking: true }));
    let dt = time.delta_secs().min(0.1);
    *log_timer += dt;
    let log_now = *log_timer >= 1.0;
    if log_now {
        *log_timer = 0.0;
    }
    // Blocking cylinders: the player first, then every living zed (kept up
    // to date as each zed moves, so later zeds see the new positions).
    let player_cylinder = Cylinder {
        centre: local_target,
        radius: PLAYER_RADIUS * SCALE,
        half_height: PLAYER_HALF_HEIGHT * SCALE,
    };
    // A flying (no-clip) camera does not block.
    let mut blockers: Vec<(Option<Entity>, Cylinder)> = Vec::new();
    if walker.is_some() {
        blockers.push((None, player_cylinder));
    }
    // The other players' pawns block zeds too (network host).
    for p in prey.iter().skip(1).filter(|p| p.alive) {
        blockers.push((None, Cylinder { centre: p.centre, ..player_cylinder }));
    }
    let zeds_ids: std::collections::HashMap<Entity, usize> = zeds.iter().map(|(e, z, _, _)| (e, z.id)).collect();
    blockers.extend(zeds.iter().filter_map(|(e, z, _, _)| z.blocking_cylinder().map(|c| (Some(e), c))));
    for (entity, mut z, mut t, ragdoll_state) in &mut zeds {
        let c = &classes.0[z.class];
        if z.state == ZedState::Dead {
            // On death: a ragdoll if the class has one, else the death
            // animation. The corpse is removed after CORPSE_SECONDS.
            if z.dead_for == 0.0 {
                if pinned.by == Some(z.id) {
                    pinned.release("grabber_died");
                }
                runlog::kv("zed_died", &format!("id={} decapitated={}", z.id, z.decapitated));
                match &c.ragdoll {
                    Some(def) => {
                        let pose = c.model.pose_collapsed(z.sequence, z.frame, &[]).1;
                        let launch = death_launch(&z);
                        runlog::kv(
                            "ragdoll_started",
                            &format!(
                                "id={} ragdoll={} velocity_unreal={:.0} angular_velocity={:.1} hit={}",
                                z.id,
                                def.name,
                                launch.velocity.length() / SCALE,
                                launch.angular_velocity.length(),
                                z.last_hit.is_some()
                            ),
                        );
                        let state = ragdoll::spawn(&mut commands, def, &pose, mesh_frame(c, &t), &launch, z.id);
                        commands.entity(entity).insert(state);
                    }
                    None => {
                        z.sequence = None;
                        start_anim(&mut z, c.death, false);
                    }
                }
            }
            // Pawn state Dying: Sleep(0.2), then PlayDyingSound.
            let before = z.dead_for;
            z.dead_for += dt;
            if before < 0.2 && z.dead_for >= 0.2 {
                z.sound_events.push(ZedSound::Death);
            }
            if log_now && ragdoll_state.is_none() {
                runlog::kv(
                    "zed_corpse",
                    &format!("id={} dead_for={:.1} sequence={:?} frame={:.1}", z.id, z.dead_for, z.sequence, z.frame),
                );
            }
            if z.dead_for > CORPSE_SECONDS {
                if let Some(r) = ragdoll_state {
                    for &e in r.joints.iter().chain(&r.bodies) {
                        commands.entity(e).despawn();
                    }
                }
                commands.entity(entity).despawn();
                runlog::kv("zed_removed", &format!("id={}", z.id));
            }
            continue;
        }
        // A network client's copy of a host zed: net/zeds.rs moves and
        // animates it; nothing is decided here.
        if z.net.puppet {
            if z.decapitated && pinned.by == Some(z.id) {
                pinned.release("grabber_decapitated");
            }
            moan_tick(&mut z, time.elapsed_secs());
            // The Commando's glow is this game's own look (ZombieStalker /
            // ZombieBoss.Tick run on every client for its local player).
            if c.cloak_material.is_some() {
                z.puppet_stalker_glow_tick(&viewer, dt);
            }
            if c.boss.is_some() && c.spotted_material.is_some() {
                let at = z.centre;
                z.boss_spot_tick(Some(&viewer), || sees(&spatial, at, local_target), dt);
            }
            t.translation = z.centre;
            t.rotation = coords::rotation(Rotator { pitch: 0, yaw: z.yaw as i32, roll: 0 });
            continue;
        }
        if kill_all || (kill_near && (z.centre - local_target).length() / SCALE < 500.0 && !z.is_dead()) || (kill_boss && z.boss.is_some()) {
            z.last_hit = None;
            z.kill();
            kills.0 += 1;
            z.killed_by_player = true;
            runlog::kv("zed_killed_test", &format!("id={}", z.id));
            continue;
        }
        // Damage from the level (pain volumes, KillZ): no instigator, no
        // kill credit.
        let zid = z.id;
        for &(_, amount, cause) in level_hits.iter().filter(|h| h.0 == zid) {
            if z.is_dead() {
                break;
            }
            z.health -= amount;
            runlog::kv("zed_level_damage", &format!("id={} cause={cause} damage={amount} health={:.0}", z.id, z.health));
            if z.health <= 0.0 {
                z.last_hit = None;
                z.kill();
            }
        }
        if z.is_dead() {
            t.translation = z.centre;
            continue;
        }
        if stuck.contains(&z.id) && !z.is_dead() {
            z.last_hit = None;
            z.kill();
            runlog::kv("zed_killed_stuck", &format!("id={}", z.id));
            continue;
        }
        // DoBossDeath: the controller is gone mid-whatever; the body stands
        // (our approximation: the attack is dropped and it idles).
        if (boss_died || (game_ended && !z.is_dead())) && !z.braindead {
            z.braindead = true;
            z.attack = None;
            z.overlay = None;
            z.door_bash = None;
            if pinned.by == Some(z.id) {
                pinned.release("boss_died");
            }
            if z.state != ZedState::Falling {
                z.state = ZedState::Idle;
            }
            runlog::kv("zed_braindead", &format!("id={} reason={}", z.id, if boss_died { "boss_died" } else { "game_ended" }));
        }
        let ai = active.0 && !z.braindead;
        // The player this zed hunts (always this game's own in single player).
        let enemy = if prey.len() > 1 { choose_enemy(&mut z, c, &prey, &spatial, dt) } else { 0 };
        let Prey { peer: target_peer, centre: target, velocity: target_velocity, walking: target_walking, .. } = prey[enemy];
        // KFMonster.Tick (standalone), when CanSpeedAdjust (head on, not
        // zapped): seen within the last 5 s of being drawn, else a sight
        // check from its eyes to the player's every second; unseen zeds
        // move at HiddenGroundSpeed; beyond the fog of the player's zone
        // nothing is seen. LastRenderTime (native: drawn this
        // frame) is approximated as within 60 degrees of the view and in
        // clear sight; the zed's eyes as 0.8 of its half height up.
        let now = time.elapsed_secs();
        if !z.decapitated && !z.zapped() {
            let eye = z.centre + Vec3::Y * c.collision_height * 0.8 * SCALE;
            let to = eye - pt.translation;
            // Beyond the player's zone fog a zed is neither drawn nor seen.
            let in_fog = player_zone.in_fog_range(to.length() / SCALE);
            let in_view = to.normalize_or_zero().dot(*pt.forward()) > 0.5;
            if in_fog && in_view && sees(&spatial, pt.translation, eye) {
                z.last_render = now;
            }
            if now - z.last_render > 5.0 {
                if now - z.last_view_check > 1.0 {
                    z.last_view_check = now;
                    let was = z.hidden;
                    z.hidden = !(in_fog && sees(&spatial, eye, pt.translation));
                    // Network host: another player's view counts too (KF's
                    // LastSeenOrRelevantTime is set by any player's view;
                    // here: that player's eye in clear sight, no fog check).
                    if z.hidden && prey.len() > 1 {
                        z.hidden = !prey.iter().skip(1).any(|p| p.alive && sees(&spatial, eye, p.centre + Vec3::Y * PLAYER_EYE * SCALE));
                    }
                    if !z.hidden {
                        z.last_seen = now;
                    }
                    if was != z.hidden && log_now {
                        runlog::kv("zed_hidden", &format!("id={} hidden={}", z.id, z.hidden));
                    }
                }
            } else {
                z.last_seen = now;
                z.hidden = false;
            }
        } else {
            z.hidden = false;
        }
        // Bleeding out (KFMonster.Tick): dies when the time is up. No hit
        // momentum, so the ragdoll gets no push.
        if let Some(left) = z.bleed_out {
            let left = left - dt;
            if left <= 0.0 {
                z.last_hit = None;
                z.bled_out = true;
                z.kill();
                // Credited to the player, as KF credits LastDamagedBy (another
                // network player: the host credits them, net/zeds.rs).
                if z.net.damaged_by.is_none() {
                    kills.0 += 1;
                }
                z.killed_by_player = true;
                runlog::kv("zed_bled_out", &format!("id={} health_left={:.1}", z.id, z.health));
                continue;
            }
            z.bleed_out = Some(left);
        }
        z.since_pain_anim = (z.since_pain_anim + dt).min(1e6);
        z.since_pain_sound = (z.since_pain_sound + dt).min(1e6);
        z.since_hit = (z.since_hit + dt).min(1e6);
        // KFMonsterController: MoanTime = Level.TimeSeconds + 2 + 36 x
        // FRand() at first, then + 12 + 8 x FRand() after each moan (an int,
        // so whole seconds); headless zeds stay quiet, the Patriarch while
        // busy (bShotAnim).
        moan_tick(&mut z, now);
        // Monster.PlayChallengeSound: when the zed takes the player as its
        // enemy and when it sees the player more than 7 s after the last
        // (MonsterController EnemyChanged, Hunting.SeePlayer). Sight is
        // checked every 0.5 s here (UE2's own interval: a guess).
        z.challenge_check -= dt;
        if z.challenge_check <= 0.0 && matches!(z.state, ZedState::Chase | ZedState::Melee) {
            z.challenge_check = 0.5;
            let eye = z.centre + Vec3::Y * c.collision_height * 0.8 * SCALE;
            if now - z.last_challenge > 7.0 && sees(&spatial, eye, pt.translation) {
                z.last_challenge = now;
                z.sound_events.push(ZedSound::Challenge);
            }
        }
        if zap_all && z.health > 0.0 {
            z.set_zapped(10.0);
        }
        if hurt && z.health > 100.0 {
            z.health -= 100.0;
            z.note_damage(100.0);
            runlog::kv("zed_hurt_test", &format!("id={} health={:.0}", z.id, z.health));
            z.note_boss_health();
        }
        if c.fp_rage_anim.is_some() {
            z.fp_since_damaged = (z.fp_since_damaged + dt).min(1e6);
            // StartCharging: PoundRage (full body, waits), then RageCharging.
            if std::mem::take(&mut z.fp_start_rage)
                && z.fp_rage.is_none()
                && !matches!(z.state, ZedState::Enraging | ZedState::Falling | ZedState::Dead)
            {
                z.attack = None;
                z.overlay = None;
                z.state = ZedState::Enraging;
                z.sequence = None;
                start_anim(&mut z, c.fp_rage_anim, false);
                z.cloak_dirty = true; // device colour (DeviceGoRed)
                runlog::kv(
                    "fleshpound_rage",
                    &format!("id={} start two_sec_damage={:.0} frustrated={}", z.id, z.fp_two_sec_damage, z.fp_frustrated),
                );
            }
            // RageCharging.Tick: the rage ends when its time is up (not while
            // attacking, never when frustrated).
            if let Some(left) = z.fp_rage {
                let left = left - dt;
                if left <= 0.0 && z.attack.is_none() && !z.fp_frustrated {
                    z.fp_rage = None;
                    z.cloak_dirty = true;
                    runlog::kv("fleshpound_rage", &format!("id={} end reason=time", z.id));
                } else {
                    z.fp_rage = Some(left);
                }
            }
        }
        // ZombieStalker.Tick: every 0.5 s, spotted by a Commando or not;
        // cloak again (or glow) 1.2 s after the last uncloak (spotted.rs).
        if c.cloak_material.is_some() {
            z.stalker_cloak_tick(Some(&viewer), dt);
        }
        // ZombieBoss.Tick: the Commando's glow while he is cloaked.
        if c.boss.is_some() && c.spotted_material.is_some() {
            let at = z.centre;
            z.boss_spot_tick(Some(&viewer), || sees(&spatial, at, local_target), dt);
        }
        if let Some(s) = z.since_decap.as_mut() {
            *s += dt;
        }
        // A grab ends when the Clot loses its head (ZombieClot.RemoveHead).
        if z.decapitated && pinned.by == Some(z.id) {
            pinned.release("grabber_decapitated");
        }
        z.stunned = (z.stunned - dt).max(0.0);
        // FlipOver: full-body KnockDown; stand still until it ends.
        if z.pending_reaction == Some(HitReaction::KnockDown) && c.knock_down.is_some() {
            z.pending_reaction = None;
            z.overlay = None;
            z.attack = None;
            z.state = ZedState::KnockedDown;
            z.sequence = None;
            start_anim(&mut z, c.knock_down, false);
            runlog::kv("zed_hit_reaction", &format!("id={} reaction=KnockDown", z.id));
        }
        if matches!(z.state, ZedState::KnockedDown | ZedState::Landing | ZedState::Enraging) {
            if z.sequence.is_some_and(|s| z.frame < c.model.last_frame(s)) {
                t.translation = z.centre;
                continue;
            }
            runlog::kv("zed_state", &format!("id={} from={:?} to=Chase reason=animation_done", z.id, z.state));
            if z.state == ZedState::Enraging {
                // BeginRaging -> RageCharging: 5 + FRand() x 6 s (Normal).
                z.fp_rage = Some(5.0 + 6.0 * (z.random() % 1000) as f32 / 1000.0);
                runlog::kv("fleshpound_rage", &format!("id={} charging=true seconds={:.1}", z.id, z.fp_rage.unwrap_or(0.0)));
            }
            z.state = ZedState::Chase;
        }
        // Patriarch: MakeGrandEntry (Entrance) or SetBossLaught (VictoryLaugh)
        // asked for by the wave game: full body, standing, waits.
        if let (Some(bc), Some(mut b)) = (c.boss.as_ref(), z.boss)
            && (b.pending_entrance || b.pending_laugh)
            && !matches!(z.state, ZedState::Dead | ZedState::Falling)
        {
            let entrance = b.pending_entrance;
            let anim = if entrance { bc.entrance_anim } else { bc.laugh_anim };
            b.pending_entrance = false;
            b.pending_laugh = false;
            if let Some((seq, secs)) = anim {
                if entrance {
                    b.start_entrance(secs);
                } else {
                    b.start_laugh(secs);
                }
                z.attack = None;
                z.overlay = None;
                z.state = ZedState::BossBusy;
                z.sequence = None;
                start_anim(&mut z, Some(seq), false);
                runlog::kv("boss_entrance", &format!("id={} start={} seconds={secs:.2}", z.id, if entrance { "Entrance" } else { "VictoryLaugh" }));
            }
            z.boss = Some(b);
        }
        // Patriarch: TakeDamage asked for a knockdown (full body, waits).
        if let (Some(bc), Some(mut b)) = (c.boss.as_ref(), z.boss)
            && b.pending_knockdown
            && !matches!(z.state, ZedState::Dead | ZedState::Falling)
            && let Some((seq, secs)) = bc.knockdown_anim
        {
            let roll = (z.random() % 1000) as f32 / 1000.0;
            b.start_knockdown(secs, roll);
            z.boss = Some(b);
            z.attack = None;
            z.overlay = None;
            z.state = ZedState::BossBusy;
            z.sequence = None;
            start_anim(&mut z, Some(seq), false);
            runlog::kv("boss_knockdown", &format!("id={} start health={:.0} seconds={secs:.2}", z.id, z.health));
        }
        // Patriarch, a full-body action (chaingun, rocket, knockdown, heal):
        // `boss_busy` runs it.
        if z.state == ZedState::BossBusy {
            if ai {
                let player_velocity = target_velocity;
                // ZombieBoss.DoorAttack: the rocket goes at the door
                // (Controller.Target, its Location), not the player.
                let (aim, aim_velocity) = match z.door_bash.and_then(|b| doors.doors.get(b.door)) {
                    Some(d) => (coords::pos(d_pos(d)), Vec3::ZERO),
                    None => (target, player_velocity),
                };
                boss_busy(&mut z, c, &t, aim, aim_velocity, target_peer, dt, &spatial, &mut player_damage, &mut push, &mut fireball, &mut bullet_fx);
            }
            t.translation = z.centre;
            continue;
        }
        if z.state == ZedState::DoorBashing {
            if ai {
                door_bashing(&mut z, c, dt, doors, &spatial, target, door_hits);
            }
            t.translation = z.centre;
            continue;
        }
        let old_yaw = z.yaw;
        let old_sequence = z.sequence;
        let old_centre = z.centre;
        let mover = Mover::new(&spatial, c.collision_radius, c.collision_height, crate::world::collision::zed_filter());
        let to = (target - z.centre).with_y(0.0);
        let dist = to.length() / SCALE;
        // Attack once within MeleeRange of touching (KF's melee start); the
        // damage check later allows MeleeRange x 1.4.
        let reach = z.melee_range + c.collision_radius + PLAYER_RADIUS;
        let old_state = z.state;

        if ai && z.state != ZedState::Falling {
            // Where to head: the player when in reach or attacking, else the
            // hunting route's target (a navigation point, or the player when
            // it can be walked to directly).
            // Patriarch, state Escaping (BossZombieController SyrRetreat): to a
            // hiding spot instead of the player; BeginHealing when there.
            let escape_goal = boss_escape(&mut z, c, target, dt, &nav, &spatial);
            let (goal, touch) = match escape_goal {
                Some(g) => (g, crate::world::nav::HUNT_RADIUS),
                None => (target, crate::world::nav::HUNT_RADIUS + PLAYER_RADIUS),
            };
            let steer = if (escape_goal.is_none() && (z.attack.is_some() || dist <= reach)) || nav.points.is_empty() {
                goal
            } else {
                let others: Vec<(Vec3, f32)> = blockers
                    .iter()
                    .filter(|(e, _)| *e != Some(entity) && e.is_some())
                    .map(|(_, cyl)| (cyl.centre, cyl.radius))
                    .collect();
                let mut seed = z.random() | 1;
                let mut frand = move || {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    (seed % 10000) as f32 / 10000.0
                };
                let speed = c.ground_speed * z.speed_scale * if z.running { GOREFAST_RUN_SPEED } else { 1.0 };
                let hunt = crate::world::nav::hunt_size(c.collision_radius, c.collision_height);
                let input = crate::world::nav::RouteInput {
                    id: z.id,
                    pos: z.centre,
                    player: goal,
                    touch_player: touch,
                    speed,
                    radius: hunt.0,
                    half_height: hunt.1,
                    others: &others,
                };
                z.router.update(&nav, &spatial, &input, dt, &mut frand)
            };
            // KFMonsterController.FindPath: a zed with bCanDistanceAttackDoors
            // traces to its new MoveTarget; a sealed door in the way means
            // BreakUpDoor(door, true): attack it from here.
            if c.distance_door_attack && !z.decapitated && z.attack.is_none() && steer != goal && z.door_checked != Some(steer) {
                z.door_checked = Some(steer);
                if let Ok(dir) = Dir3::new(steer - z.centre)
                    && let Some(h) = spatial.cast_ray(z.centre, dir, (steer - z.centre).length(), true, &crate::world::collision::world_filter())
                    && let Ok(dc) = door_colliders.get(h.entity)
                    && let Some(d) = doors.doors.get(dc.0)
                    && d.sealed
                    && !d.hidden
                {
                    z.door_bash = Some(DoorBash { door: dc.0, wait: 0.0, in_anim: false, hits: 0, anim_time: 0.0, distance: true, ranged: false });
                    z.state = ZedState::DoorBashing;
                    z.overlay = None;
                    runlog::kv(
                        "zed_door_bash",
                        &format!("id={} door={} start distance=true distance_unreal={:.0} weld={:.0}", z.id, d.info.name, h.distance / SCALE, d.weld),
                    );
                    t.translation = z.centre;
                    continue;
                }
            }
            let to_steer = (steer - z.centre).with_y(0.0);
            // Turn toward it at RotationRate.
            if to_steer.length() / SCALE > 1.0 {
                let want = yaw_of(to_steer);
                let mut delta = (want - z.yaw).rem_euclid(65536.0);
                if delta > 32768.0 {
                    delta -= 65536.0;
                }
                let step = c.turn_rate * dt;
                z.yaw = (z.yaw + delta.clamp(-step, step)).rem_euclid(65536.0);
            }
            // The attack in progress: on the upper-body layer (the grab; ends
            // when its layer ends or is replaced by a flinch) or full body.
            if let Some(a) = z.attack
                && a.layered
                && !z.overlay.is_some_and(|(s, _, _)| s == a.seq)
            {
                z.attack = None;
            }
            let progress = z.attack.map(|a| {
                let frame = if a.layered { z.overlay.map_or(0.0, |(_, f, _)| f) } else { z.frame };
                frame / c.model.length(a.seq).max(1.0)
            });
            // The ranged attack's notifies: its AnimNotify_Effects
            // (KFVomitJet on the head, SirenScream) and each SpawnTwoShots
            // (the Bloat's vomit, the Siren's scream pulses).
            if let (Some(p), Some(a)) = (progress, z.attack)
                && a.ranged
            {
                let mut fired = a.fx_fired;
                for (i, (at, effect)) in c.ranged_effects.iter().enumerate().take(8) {
                    if p >= *at && fired & (1 << i) == 0 {
                        fired |= 1 << i;
                        z.pending_fx.push(effect.clone());
                    }
                }
                let mut shots = a.shots_fired;
                let mut due = 0;
                for (i, at) in c.ranged_shots.iter().enumerate().take(8) {
                    if p >= *at && shots & (1 << i) == 0 {
                        shots |= 1 << i;
                        due += 1;
                    }
                }
                z.attack = Some(Attack {
                    fx_fired: fired,
                    shots_fired: shots,
                    ..a
                });
                for _ in 0..due {
                    if let Some((damage, radius, force)) = c.scream {
                        // ZombieSiren.SpawnTwoShots: nothing while zapped.
                        if !z.zapped() {
                            // DoShakeEffect, hit or not.
                            if let Some(shake) = c.scream_shake {
                                scream_shake.write(crate::player::hit_cam::SirenScreamShake { at: z.centre, radius, shake });
                            }
                            // HurtRadius reaches every player in range (in
                            // single player: the one, alive or not, as before).
                            for p in prey.iter().filter(|p| p.alive || prey.len() == 1) {
                                scream_pulse(&z, damage, radius, force, p.centre, p.peer, &spatial, &mut player_damage, &mut push);
                            }
                            // ZombieSiren.HurtRadius reaches doors too (any
                            // non-zed actor in sight within ScreamRadius).
                            door_blasts.write(crate::world::door::DoorBlast {
                                at: ue_pos(z.centre),
                                radius,
                                damage,
                                zed: Some(z.id),
                                direct: None,
                                line_of_sight: true,
                                frag: false,
                                source: "siren_scream",
                            });
                        }
                    } else if c.kind == ZedKind::Husk {
                        let player_velocity = target_velocity;
                        shoot_fireball(&mut z, c, &t, crate::zeds::fireball::Projectile::HuskFire, c.barrel_bone, target, player_velocity, &spatial, &mut fireball);
                    } else {
                        spawn_two_shots(&z, c, target, &mut vomit);
                    }
                }
            }
            // ZombieBoss.ClawDamageTarget: his hits land at the animation's
            // notifies, with the attack's own reach, and push the player.
            if let (Some(p), Some(a), Some(boss)) = (progress, z.attack, c.boss.as_ref())
                && !a.ranged
                && let Some(m) = boss.melee_for(a.seq)
            {
                let mut shots = a.shots_fired;
                let mut due = 0;
                for (i, at) in m.hits.iter().enumerate().take(8) {
                    if p >= *at && shots & (1 << i) == 0 {
                        shots |= 1 << i;
                        due += 1;
                    }
                }
                z.attack = Some(Attack {
                    shots_fired: shots,
                    hit_done: shots.count_ones() as usize >= m.hits.len(),
                    ..a
                });
                for _ in 0..due {
                    let in_range = dist <= m.range * 1.4 + c.collision_radius + PLAYER_RADIUS;
                    let dz = ((target.y - z.centre.y) / SCALE).abs();
                    let level = dz <= c.collision_height.max(PLAYER_HALF_HEIGHT) + 0.5 * c.collision_height.min(PLAYER_HALF_HEIGHT);
                    // Charging.MeleeDamageTarget: push x 1.5; each check uses
                    // one of the charge's attacks, a landed hit ends it.
                    let charging = z.boss.is_some_and(|b| b.charge.is_some());
                    // Escaping.MeleeDamageTarget (the sneak states): push x 1.5
                    // too; SneakAround.MeleeDamageTarget then ends the sneak,
                    // hit or miss.
                    let sneaking = z.boss.is_some_and(|b| b.sneaking());
                    let escaping = z.boss.is_some_and(|b| b.escaping());
                    let push_scale = if charging || escaping { crate::zeds::boss::CHARGE_PUSH } else { 1.0 };
                    let id = z.id;
                    if sneaking && let Some(b) = z.boss.as_mut() {
                        b.end_sneak();
                        z.cloaked = false;
                        z.cloak_dirty = true;
                        runlog::kv("boss_sneak", &format!("id={id} end reason=melee landed={}", in_range && level));
                    }
                    if let Some(b) = z.boss.as_mut()
                        && charging
                    {
                        let ended = b.charge_hit(in_range && level);
                        runlog::kv(
                            "boss_charge_attack",
                            &format!("id={id} landed={} attacks_left={} ended={ended}", in_range && level, b.charge.map_or(0, |c| c.attacks_left)),
                        );
                        if ended {
                            runlog::kv("boss_charge", &format!("id={id} end reason=hit"));
                        }
                    }
                    if in_range && level {
                        let roll = (z.random() % 1000) as f32 / 1000.0;
                        let amount = z.melee_damage * 0.95 + z.melee_damage * 0.1 * roll;
                        player_damage.write(crate::game::combat::PlayerDamaged {
                            amount,
                            armor_stops: true,
                            zed_id: z.id,
                            kind: crate::game::combat::HurtKind::Plain,
                            dam_type: c.melee_dam_type,
                            source: Some(z.centre),
                            dam: None,
                            to_peer: target_peer,
                        });
                        let impale = z.attack.is_some_and(|a| c.model.sequence_name(a.seq) == Some("MeleeImpale"));
                        z.sound_events.push(if impale { ZedSound::ImpaleHit } else { ZedSound::MeleeHit });
                        let (from, to) = (ue_pos(z.centre), ue_pos(target));
                        let momentum = (to - from).normalize_or_zero() * BOSS_DAMAGE_FORCE * push_scale;
                        push.write(crate::player::walk::PlayerPush { momentum, to_peer: target_peer });
                        runlog::kv(
                            "boss_melee_hit",
                            &format!(
                                "id={} sequence={} hit={} damage={amount:.1} distance_unreal={dist:.0} reach_unreal={:.0}",
                                z.id,
                                c.model.sequence_name(a.seq).unwrap_or("?"),
                                shots.count_ones(),
                                m.range * 1.4 + c.collision_radius + PLAYER_RADIUS
                            ),
                        );
                    } else {
                        runlog::kv("zed_attack_missed", &format!("id={} in_range={in_range} level={level} distance_unreal={dist:.0}", z.id));
                    }
                }
            }
            // The melee hits: one damage check at each ClawDamageTarget notify
            // of the attack animation (claw_times; the Gorefast's double
            // swing hits twice). KFMonster.MeleeDamageTarget: the target
            // still within MeleeRange x 1.4 + both radii, roughly level, the
            // zed not stunned and not in its 2 s after losing its head.
            if let (Some(p), Some(a)) = (progress, z.attack)
                && !a.hit_done
                && !a.ranged
                && c.boss.is_none()
            {
                let times = claw_times(&c.model, a.seq);
                let (fired, due) = due_notifies(&times, p, a.shots_fired);
                z.attack = Some(Attack {
                    shots_fired: fired,
                    hit_done: fired.count_ones() as usize >= times.len().min(8),
                    ..a
                });
                if times.is_empty() {
                    runlog::kv("zed_melee_no_notify", &format!("id={} sequence={}", z.id, c.model.sequence_name(a.seq).unwrap_or("?")));
                }
                for i in due {
                    let in_range = dist <= z.melee_range * 1.4 + c.collision_radius + PLAYER_RADIUS;
                    let dz = ((target.y - z.centre.y) / SCALE).abs();
                    let level = dz <= c.collision_height.max(50.0) + 0.5 * c.collision_height.min(50.0);
                    let dazed = z.since_decap.is_some_and(|s| s < 2.0);
                    let sequence = c.model.sequence_name(a.seq).unwrap_or("?");
                    let seconds = p * c.model.length(a.seq).max(1.0) / c.model.rate(a.seq).max(1.0);
                    if in_range && level && z.stunned <= 0.0 && !dazed {
                        // ClawDamageTarget: MeleeDamage -5% .. +5%.
                        let roll = (z.random() % 1000) as f32 / 1000.0;
                        let mut amount = z.melee_damage * 0.95 + z.melee_damage * 0.1 * roll;
                        // ZombieFleshPound.ClawDamageTarget: repeated-hit attacks do
                        // less per hit (PoundAttack1 x 0.5, PoundAttack2 x 0.25);
                        // raging, MeleeDamageTarget x 1.75 and a landed hit ends
                        // the rage (the next notify hits at normal damage).
                        if c.fp_rage_anim.is_some() {
                            match c.model.sequence_name(a.seq) {
                                Some("PoundAttack1") => amount *= 0.5,
                                Some("PoundAttack2") => amount *= 0.25,
                                _ => {}
                            }
                            if z.fp_rage.is_some() {
                                // MeleeDamageTarget(int hitdamage): the raging
                                // override passes hitdamage x 1.75, cut again
                                // in apply_player_damage.
                                amount = amount.trunc() * 1.75;
                                z.fp_rage = None;
                                z.fp_frustrated = false;
                                z.cloak_dirty = true;
                                runlog::kv("fleshpound_rage", &format!("id={} end reason=hit", z.id));
                            }
                        }
                        player_damage.write(crate::game::combat::PlayerDamaged {
                            amount,
                            armor_stops: true,
                            zed_id: z.id,
                            kind: crate::game::combat::HurtKind::Plain,
                            dam_type: c.melee_dam_type,
                            source: Some(z.centre),
                            dam: None,
                            to_peer: target_peer,
                        });
                        // ClawDamageTarget: MeleeAttackHitSound when the hit lands.
                        z.sound_events.push(ZedSound::MeleeHit);
                        // ZombieClot: a landed grab pins the player (not when headless).
                        // CanBeGrabbed: a Berserker is not grabbed by Clots.
                        if c.grapple_duration > 0.0 && !z.decapitated && target_walking {
                            match target_peer {
                                // Another player's: their game pins them
                                // (and checks their perk).
                                Some(peer) => {
                                    grabs.write(crate::game::combat::RemoteGrab { peer, seconds: c.grapple_duration, zed_id: z.id });
                                }
                                None if vet.vet.can_be_grabbed_by_clot() => pinned.pin(c.grapple_duration, z.id),
                                None => runlog::kv("perk_mod", &format!("kind=no_clot_grab perk={} zed={}", vet.vet.label(), z.id)),
                            }
                        }
                        runlog::kv(
                            "zed_melee_hit",
                            &format!(
                                "id={} sequence={sequence} hit={}/{} notify_at={:.3} progress={p:.3} anim_seconds={seconds:.3} damage={amount:.1} distance_unreal={dist:.0} target={}",
                                z.id,
                                i + 1,
                                times.len(),
                                times[i],
                                target_peer.map_or("local".to_string(), |p| p.to_string())
                            ),
                        );
                    } else {
                        runlog::kv(
                            "zed_attack_missed",
                            &format!(
                                "id={} sequence={sequence} hit={}/{} notify_at={:.3} progress={p:.3} in_range={in_range} level={level} stunned={} dazed={dazed}",
                                z.id,
                                i + 1,
                                times.len(),
                                times[i],
                                z.stunned > 0.0
                            ),
                        );
                    }
                }
            }
            // ZombieClot.Tick: the grab animation stops if the target gets out
            // of reach (MeleeRange + both radii).
            if c.kind == ZedKind::Clot && z.attack.is_some_and(|a| a.layered) && dist > z.melee_range + c.collision_radius + PLAYER_RADIUS {
                z.attack = None;
                z.overlay = None;
                runlog::kv("zed_grab_broken", &format!("id={} distance_unreal={dist:.0}", z.id));
            }
            let full_body_busy = z.attack.is_some_and(|a| !a.layered)
                && z.sequence.is_some_and(|s| z.frame < c.model.last_frame(s));
            if z.attack.is_some_and(|a| !a.layered) && !full_body_busy {
                z.attack = None;
            }
            // CrawlerController.FireWeaponAt / ZombieCrawler.DoPounce: out of
            // reach, roughly facing the target (KF compares the facing with
            // the un-normalised vector to it, so nearly any forward angle
            // passes), after 4.5 - FRand() x 3 s since the last pounce, and
            // IsInPounceDist (within MeleeRange x 5, landing at its height):
            // leap at PounceSpeed with JumpZ upward.
            z.since_pounce = (z.since_pounce + dt).min(1e6);
            // ZombieCrawler.DoPounce: not while zapped.
            if c.pounce_speed > 0.0 && z.attack.is_none() && dist > reach && z.state == ZedState::Chase && !z.decapitated && !z.zapped() {
                let wait = 4.5 - (z.random() % 1000) as f32 / 1000.0 * 3.0;
                let to_ue = (target - z.centre) / SCALE;
                let ahead = dir_of(z.yaw).dot(to_ue) > 0.85;
                let t_air = dist / c.pounce_speed;
                let end_z = z.centre.y / SCALE + c.jump_z * t_air - 0.5 * GRAVITY * t_air * t_air;
                let lands_level = (end_z - target.y / SCALE).abs() < c.collision_height + PLAYER_HALF_HEIGHT;
                if z.since_pounce > wait && ahead && lands_level && to_ue.length() < z.melee_range * 5.0 {
                    let dir3 = to_ue.normalize_or_zero();
                    z.state = ZedState::Falling;
                    z.air_velocity = dir3.with_y(0.0) * c.pounce_speed * SCALE;
                    z.vertical_speed = c.jump_z * SCALE;
                    z.pouncing = true;
                    z.since_pounce = 0.0;
                    z.sequence = None;
                    start_anim(&mut z, c.air_anim, false);
                    runlog::kv("crawler_pounce", &format!("id={} distance_unreal={dist:.0} wait={wait:.1}", z.id));
                }
            }
            // ZombieScrake: SawingLoop ends when the target is out of reach
            // (RangedAttack -> GoToState('')): damage and speed back to normal.
            if c.saw_impale.is_some() && z.sawing && z.attack.is_none() && dist > reach {
                z.sawing = false;
                z.saw_charging = false;
                z.melee_damage = c.melee_damage;
                runlog::kv("scrake_sawing", &format!("id={} sawing=false distance_unreal={dist:.0}", z.id));
            }
            // ZombieScrake.RangedAttack: not attacking, has a head, under
            // half health: RunningState (rage), GroundSpeed x 3.5.
            // RunningState.BeginState: not while zapped.
            if c.saw_impale.is_some()
                && !z.raging
                && !z.zapped()
                && !z.sawing
                && z.attack.is_none()
                && !z.decapitated
                && z.health / z.health_max < 0.5
            {
                z.raging = true;
                runlog::kv("scrake_rage", &format!("id={} health={:.0}", z.id, z.health));
            }
            // FleshpoundZombieController ZombieCharge: chasing without an
            // attack for RageFrustrationThreshhold (10) + FRand() x 5 s
            // makes him rage (frustrated: the rage only ends on a hit).
            if c.fp_rage_anim.is_some() && z.fp_rage.is_none() && z.state == ZedState::Chase {
                if z.attack.is_some() {
                    z.fp_frustration = 0.0;
                } else {
                    z.fp_frustration += dt;
                    if z.fp_frustration >= z.fp_frustration_limit && !z.decapitated && !z.zapped() {
                        z.fp_frustration = 0.0;
                        z.fp_frustrated = true;
                        z.fp_start_rage = true;
                    }
                }
            }
            // RangedAttack (ZombieBloat, ZombieSiren): out of melee reach,
            // in range and with the head on. The Bloat vomits within 250,
            // with ChargeChance 0.4 (Normal) on the upper body while walking,
            // else standing; the Siren screams within ScreamRadius, always on
            // the upper body (her Tick keeps her walking at 0.65 speed).
            // MonsterController.FireWeaponAt: only at an enemy in view
            // (Focus); HuskZombieController: not before NextFireProjectileTime.
            let dist3 = (target - z.centre).length() / SCALE;
            z.ranged_wait = (z.ranged_wait - dt).max(0.0);
            // ZombieSiren.RangedAttack: no scream while zapped.
            if let Some(ranged) = c.ranged_anim
                && !(c.scream.is_some() && z.zapped())
                && z.attack.is_none()
                && z.stunned <= 0.0
                && !z.decapitated
                && z.state == ZedState::Chase
                && dist > reach
                && dist3 <= c.ranged_distance
                && z.ranged_wait <= 0.0
                && sees(&spatial, z.centre, target)
            {
                if c.ranged_interval > 0.0 {
                    z.ranged_wait = c.ranged_interval + 2.0 * (z.random() % 1000) as f32 / 1000.0;
                }
                let moving = c.fire_root_bone.is_some() && (z.random() % 1000) as f32 / 1000.0 < c.ranged_moving_chance;
                if moving {
                    z.overlay = Some((ranged, 0.0, c.fire_root_bone.expect("checked")));
                } else {
                    z.state = ZedState::Melee;
                    z.sequence = None;
                    start_anim(&mut z, Some(ranged), false);
                }
                z.attack = Some(Attack {
                    seq: ranged,
                    layered: moving,
                    hit_done: false,
                    ranged: true,
                    fx_fired: 0,
                    shots_fired: 0,
                });
                runlog::kv(
                    "zed_ranged_attack",
                    &format!(
                        "id={} sequence={} moving={moving} distance_unreal={dist3:.0}",
                        z.id,
                        c.model.sequence_name(ranged).unwrap_or("?")
                    ),
                );
            }
            // ZombieBoss.RangedAttack: close enough (IsCloseEnuf), MeleeImpale
            // or MeleeClaw on the upper body from SpineBone1.
            if let Some(boss) = c.boss.as_ref()
                && z.attack.is_none()
                && z.state == ZedState::Chase
                && let Some(root) = c.fire_root_bone
                && crate::zeds::boss::is_close_enough(dist, (target.y - z.centre.y) / SCALE, c.collision_radius, c.collision_height, PLAYER_RADIUS, PLAYER_HALF_HEIGHT)
            {
                let roll = (z.random() % 1000) as f32 / 1000.0;
                // Escaping.RangedAttack (the sneak states): MeleeClaw only,
                // and UnCloakBoss first.
                let escaping = z.boss.is_some_and(|b| b.escaping());
                let seq = if escaping { boss.claw.seq } else { boss.choose_melee(z.health, roll).seq };
                if escaping && let Some(b) = z.boss.as_mut() {
                    b.cloaked = false;
                    z.cloaked = false;
                    z.cloak_dirty = true;
                    runlog::kv("boss_sneak", &format!("id={} uncloak reason=attack", z.id));
                }
                z.overlay = Some((seq, 0.0, root));
                z.attack = Some(Attack {
                    seq,
                    layered: true,
                    hit_done: false,
                    ranged: false,
                    fx_fired: 0,
                    shots_fired: 0,
                });
                runlog::kv(
                    "zed_attack",
                    &format!(
                        "id={} sequence={} layered=true charge=false length_frames={} rate={} health={:.0} distance_unreal={dist:.0}",
                        z.id,
                        c.model.sequence_name(seq).unwrap_or("?"),
                        c.model.length(seq),
                        c.model.rate(seq),
                        z.health
                    ),
                );
            }
            // ZombieBoss.RangedAttack beyond melee, about every frame while he
            // sees the player (`boss.rs`), and the Charging state's ends.
            if c.boss.is_some()
                && let Some(mut b) = z.boss
            {
                if let Some(why) = b.tick(dt) {
                    runlog::kv("boss_charge", &format!("id={} end reason={why} distance_unreal={dist3:.0}", z.id));
                }
                let initial = b.sneak.is_some_and(|s| s.initial);
                let seen = b.sneaking() && sees(&spatial, z.centre, target);
                match b.sneak_step(dt, seen, z.attack.is_some()) {
                    Some(crate::zeds::boss::SneakChange::Cloaked) => {
                        runlog::kv("boss_sneak", &format!("id={} cloak", z.id));
                    }
                    Some(crate::zeds::boss::SneakChange::Ended(why)) => {
                        runlog::kv("boss_sneak", &format!("id={} end reason={why} initial={initial} distance_unreal={dist3:.0}", z.id));
                    }
                    None => {}
                }
                if z.cloaked != b.cloaked {
                    z.cloaked = b.cloaked;
                    z.cloak_dirty = true;
                }
                // (Only FireChaingun reacts to being shot from close.)
                b.hit_from_close = false;
                if z.state == ZedState::Chase && sees(&spatial, z.centre, target) {
                    let attacking = z.attack.is_some();
                    let decision = {
                        let mut roll = || (z.random() % 10000) as f32 / 10000.0;
                        b.decide(dist3, attacking, &mut roll)
                    };
                    match decision {
                        crate::zeds::boss::Decision::StartCharge { attacks } => {
                            // SetAnimAction('transition'): upper body.
                            if z.attack.is_none()
                                && let (Some(seq), Some(root)) = (c.boss.as_ref().and_then(|bc| bc.transition), c.fire_root_bone)
                            {
                                z.overlay = Some((seq, 0.0, root));
                            }
                            runlog::kv("boss_charge", &format!("id={} start attacks={attacks} distance_unreal={dist3:.0}", z.id));
                        }
                        crate::zeds::boss::Decision::EndCharge(why) => {
                            runlog::kv("boss_charge", &format!("id={} end reason={why} distance_unreal={dist3:.0}", z.id));
                        }
                        crate::zeds::boss::Decision::StartChaingun { shots } => {
                            if let Some(anims) = c.boss.as_ref().and_then(|bc| bc.mg_anims) {
                                // PreFireMG (full body, waits), state FireChaingun.
                                b.chaingun = Some(crate::zeds::boss::Chaingun::start(shots, anims[0].1));
                                b.mg_focal = target;
                                z.attack = None;
                                z.overlay = None;
                                z.state = ZedState::BossBusy;
                                z.sequence = None;
                                start_anim(&mut z, Some(anims[0].0), false);
                                runlog::kv(
                                    "boss_chaingun",
                                    &format!("id={} start shots={shots} next_in={:.1} distance_unreal={dist3:.0}", z.id, b.chaingun_wait),
                                );
                            }
                        }
                        crate::zeds::boss::Decision::StartMissile => {
                            if let Some(anims) = c.boss.as_ref().and_then(|bc| bc.missile_anims) {
                                // PreFireMissile (full body, waits), state FireMissile.
                                b.missile = Some(crate::zeds::boss::Missile::start(anims[0].1));
                                z.attack = None;
                                z.overlay = None;
                                z.state = ZedState::BossBusy;
                                z.sequence = None;
                                start_anim(&mut z, Some(anims[0].0), false);
                                runlog::kv(
                                    "boss_missile",
                                    &format!("id={} start next_in={:.1} distance_unreal={dist3:.0}", z.id, b.missile_wait),
                                );
                            }
                        }
                        crate::zeds::boss::Decision::DelayMissile(wait) => {
                            runlog::kv("boss_missile", &format!("id={} put_off seconds={wait:.1}", z.id));
                        }
                        crate::zeds::boss::Decision::StartSneak => {
                            // SetAnimAction('transition'), GoToState('SneakAround'):
                            // CloakBoss at once.
                            if z.attack.is_none()
                                && let (Some(seq), Some(root)) = (c.boss.as_ref().and_then(|bc| bc.transition), c.fire_root_bone)
                            {
                                z.overlay = Some((seq, 0.0, root));
                            }
                            z.cloaked = !z.zapped();
                            z.cloak_dirty = true;
                            runlog::kv("boss_sneak", &format!("id={} start distance_unreal={dist3:.0}", z.id));
                        }
                        crate::zeds::boss::Decision::DelaySneak => {
                            runlog::kv("boss_sneak", &format!("id={} put_off seconds=20", z.id));
                        }
                        crate::zeds::boss::Decision::DelayChaingun(wait) => {
                            runlog::kv("boss_chaingun", &format!("id={} put_off seconds={wait:.1}", z.id));
                        }
                        crate::zeds::boss::Decision::Nothing => {}
                    }
                }
                z.boss = Some(b);
            }
            let melee = if z.decapitated && !c.headless_melee.is_empty() { &c.headless_melee } else { &c.melee };
            // ZombieSiren.RemoveHead: MeleeRange -500, no more bites.
            let no_melee = c.scream.is_some() && z.decapitated;
            if z.attack.is_none() && dist <= reach && !melee.is_empty() && z.stunned <= 0.0 && !no_melee {
                // (CanAttack is false while stunned.) A random attack (Rand(3)).
                let slot = z.random() as usize % melee.len();
                let mut seq = melee[slot];
                // RemoveHead: "no head so biting is out": Claw3 becomes Claw2
                // (slot 2) or Claw1 (slot 1).
                if z.decapitated && c.model.sequence_name(seq).is_some_and(|n| n.eq_ignore_ascii_case("Claw3")) {
                    let swap = if slot == 2 { "Claw2" } else { "Claw1" };
                    seq = c.model.sequence(swap).unwrap_or(seq);
                }
                // ZombieGoreFast RunningState.RangedAttack: while running, a
                // moving attack with ChargeChance (upper body, keeps running
                // for RunAttackTimeout); otherwise full body, and the run ends.
                let charge = z.running
                    && c.fire_root_bone.is_some()
                    && (z.random() % 1000) as f32 / 1000.0 < GOREFAST_CHARGE_CHANCE;
                if z.running {
                    if charge {
                        z.run_attack_timeout = c.run_attack_seconds;
                    } else {
                        z.running = false;
                        runlog::kv("gorefast_run", &format!("id={} running=false reason=full_body_attack distance_unreal={dist:.0}", z.id));
                    }
                }
                // ZombieFleshPound.PostNetReceive: raging, every attack is
                // FPRageAttack.
                if z.fp_rage.is_some()
                    && let Some(rage_attack) = c.fp_rage_attack
                {
                    seq = rage_attack;
                }
                // ZombieScrake: the first swing enters SawingLoop (charging
                // with ChargeChance 0.5, or 0.7 under half health: Normal
                // difficulty); while sawing the attack is SawImpaleLoop (full
                // body) at MeleeDamage x 0.6, repeated while in reach.
                if let Some(impale) = c.saw_impale {
                    if z.sawing {
                        seq = impale;
                        z.melee_damage = c.melee_damage * 0.6;
                    } else {
                        z.sawing = true;
                        z.raging = false;
                        let roll1 = (z.random() % 1000) as f32 / 1000.0;
                        let roll2 = (z.random() % 1000) as f32 / 1000.0;
                        z.saw_charging = (z.health / z.health_max < 0.5 && roll1 <= 0.7) || roll2 <= 0.5;
                        runlog::kv("scrake_sawing", &format!("id={} sawing=true charging={}", z.id, z.saw_charging));
                    }
                }
                let layered = (charge || c.layered_attacks.contains(&seq)) && c.fire_root_bone.is_some();
                if layered {
                    z.overlay = Some((seq, 0.0, c.fire_root_bone.expect("checked")));
                } else {
                    z.state = ZedState::Melee;
                    z.sequence = None; // force restart even if the same attack
                    start_anim(&mut z, Some(seq), false);
                }
                z.attack = Some(Attack {
                    seq,
                    layered,
                    hit_done: false,
                    ranged: false,
                    fx_fired: 0,
                    shots_fired: 0,
                });
                // ZombieStalker.SetAnimAction: a melee attack uncloaks her.
                if c.cloak_material.is_some() {
                    z.since_uncloak = 0.0;
                    // UncloakStalker sets the normal skin: the glow goes.
                    if z.glow {
                        z.clear_glow();
                        runlog::kv("stalker_glow", &format!("id={} on=false reason=attack", z.id));
                    }
                    if z.cloaked {
                        z.cloaked = false;
                        z.cloak_dirty = true;
                        runlog::kv("stalker_uncloak", &format!("id={} reason=attack", z.id));
                    }
                }
                runlog::kv(
                    "zed_attack",
                    &format!(
                        "id={} sequence={} layered={layered} charge={charge} length_frames={} rate={}",
                        z.id,
                        c.model.sequence_name(seq).unwrap_or("?"),
                        c.model.length(seq),
                        c.model.rate(seq)
                    ),
                );
            }
            let attacking = z.attack.is_some();
            if let Some(what) = z.update_running(dist, attacking, dt) {
                runlog::kv(
                    "gorefast_run",
                    &format!("id={} running={} reason={what} distance_unreal={dist:.0}", z.id, z.running),
                );
            }
            if z.state == ZedState::Falling {
                // Just left the ground (a pounce): no walking this frame.
            } else if z.attack.is_some_and(|a| !a.layered) {
                // Full-body attack: stand and finish it.
            } else if z.state == ZedState::BossBusy {
                // The Patriarch just started a full-body action (PreFireMG).
            } else {
                // Walking, also during a grab (ZombieClot.Tick keeps
                // accelerating toward the target while attacking).
                z.state = ZedState::Chase;
                // RemoveHead: GroundSpeed x 0.8 once headless; a running
                // Gorefast x 1.875.
                let any_run = z.running || z.raging || z.fp_rage.is_some() || z.boss.is_some_and(|b| b.charge.is_some() || b.escaping());
                if !any_run {
                    z.run_speed_lost = false;
                }
                // OriginalGroundSpeed: the class's x the difficulty's
                // MovementSpeedDifficultyScale (KFMonster.PostBeginPlay);
                // HiddenGroundSpeed is not scaled.
                let ground = c.ground_speed * z.speed_scale;
                let speed = if z.zapped() && z.boss.is_some_and(|b| b.charge.is_some() || b.escaping()) {
                    // ZombieBoss Charging / Escaping: "Zapping slows him
                    // down, but doesn't stop him": x 1.5.
                    ground * 1.5
                } else if z.zapped() {
                    // SetZappedBehavior: OriginalGroundSpeed x ZappedSpeedMod.
                    ground * z.zap.speed_mod
                } else if z.decapitated {
                    ground * 0.8
                } else if z.run_speed_lost && any_run {
                    ground
                } else if z.running {
                    ground * GOREFAST_RUN_SPEED
                } else if z.fp_rage.is_some() {
                    ground * FLESHPOUND_RAGE_SPEED
                } else if z.raging {
                    ground * SCRAKE_RAGE_SPEED
                } else if z.saw_charging {
                    ground * SCRAKE_ATTACK_CHARGE_RATE
                } else if c.scream.is_some() && z.attack.is_some() {
                    // ZombieSiren.Tick: GroundSpeed x 0.65 while attacking.
                    ground * 0.65
                } else if z.boss.is_some_and(|b| b.escaping()) {
                    // Escaping.Tick (and the sneak states): x 2.5, normal speed
                    // while attacking.
                    let scale = if z.attack.is_some() { 1.0 } else { crate::zeds::boss::CHARGE_SPEED };
                    ground * scale
                } else if z.boss.is_some_and(|b| b.charge.is_some()) {
                    // ZombieBoss Charging.Tick: x 2.5, x 1.25 while attacking.
                    let scale = if z.attack.is_some() { crate::zeds::boss::CHARGE_ATTACK_SPEED } else { crate::zeds::boss::CHARGE_SPEED };
                    ground * scale
                } else if z.hidden {
                    // KFMonster.Tick: unseen, SetGroundSpeed(HiddenGroundSpeed).
                    c.hidden_speed
                } else {
                    ground
                };
                // KFMonster.TakeDamage on catching fire: GroundSpeed x 0.8
                // (of the current speed, so headless zeds slow down more).
                let speed = if z.burn_down > 0 && !z.zapped() { speed * 0.8 } else { speed };
                let delta = dir_of(z.yaw) * speed * SCALE * dt;
                let others: Vec<(Option<Entity>, Cylinder)> =
                    blockers.iter().filter(|(e, _)| *e != Some(entity)).copied().collect();
                let shapes: Vec<Cylinder> = others.iter().map(|(_, c)| *c).collect();
                let me = z.blocking_cylinder().expect("living zed");
                let (delta, blocked) = clip_move(&me, delta, &shapes);
                if let Some(i) = blocked
                    && log_now
                {
                    let by = others[i].0.and_then(|e| zeds_ids.get(&e).copied()).map_or("player".into(), |id| format!("zed {id}"));
                    runlog::kv("pawn_blocked", &format!("mover=zed {} by={by}", z.id));
                }
                let (moved, wall) = mover.ground_move(z.centre, delta);
                // KFDoorMover.Bump: a zed walking into a closed or sealed
                // door is told to BreakUpDoor (state DoorBashing). Its loop
                // only runs while the door is sealed, visible and not
                // bZombiesIgnore, so only then does the zed stay.
                if let Some(h) = &wall
                    && let Ok(dc) = door_colliders.get(h.entity)
                    && let Some(d) = doors.doors.get(dc.0)
                    && d.sealed
                    && !d.hidden
                    && !d.info.zombies_ignore
                {
                    // ZombieSiren / ZombieBoss have no DoorBash: their
                    // DoorAttack screams or fires a rocket.
                    if c.door_bash.is_some() || c.scream.is_some() || c.boss.is_some() {
                        z.door_bash = Some(DoorBash { door: dc.0, wait: 0.0, in_anim: false, hits: 0, anim_time: 0.0, distance: false, ranged: false });
                        z.state = ZedState::DoorBashing;
                        z.attack = None;
                        z.overlay = None;
                        z.running = false;
                        runlog::kv("zed_door_bash", &format!("id={} door={} start weld={:.0}", z.id, d.info.name, d.weld));
                        t.translation = z.centre;
                        continue;
                    }
                }
                // KFGlassMover.Bump: the pane takes the zed's speed and its
                // MeleeDamage; HandleBumpGlass: the zed stops and plays
                // MeleeAnims[0] (WaitForAnim).
                if let Some(h) = &wall
                    && let Ok(g) = glass.get(h.entity)
                {
                    glass_bumps.write(crate::world::glass::GlassBump { pane: g.0, speed, melee: Some(z.melee_damage) });
                    if let Some(&seq) = c.melee.first() {
                        z.state = ZedState::Melee;
                        z.sequence = None;
                        start_anim(&mut z, Some(seq), false);
                        z.attack = Some(Attack { seq, layered: false, hit_done: true, ranged: false, fx_fired: 0, shots_fired: 0 });
                    }
                    runlog::kv("zed_bump_glass", &format!("id={} speed={speed:.0}", z.id));
                    t.translation = z.centre;
                    continue;
                }
                let progress = (moved - z.centre).with_y(0.0).length();
                // Blocked by the level (not a pawn) while heading somewhere:
                // jump it if a jump clears it (native PickWallAdjust jumps
                // obstacles; rule from memory, not the scripts). The jump
                // carries the zed forward at its ground speed.
                let jump = wall.is_some()
                    && blocked.is_none()
                    && progress < 0.3 * delta.length()
                    && z.jump_cooldown <= 0.0
                    && mover
                        .jump_over(z.centre, dir_of(z.yaw) * 2.0 * c.collision_radius * SCALE, crate::world::nav::JUMP_APEX * SCALE)
                        .is_some();
                if jump {
                    z.state = ZedState::Falling;
                    z.vertical_speed = c.jump_z * SCALE;
                    z.air_velocity = dir_of(z.yaw) * speed * SCALE;
                    z.jump_cooldown = 1.0;
                    let u = z.centre / SCALE;
                    runlog::kv("zed_jump", &format!("id={} at_unreal=({:.0}, {:.0}, {:.0}) jump_z={}", z.id, -u.z, u.x, u.y, c.jump_z));
                } else {
                    match mover.snap_to_floor(moved) {
                        Some(on_floor) => z.centre = on_floor,
                        None => {
                            // Walked off a ledge: fall, keeping the walking speed.
                            z.centre = moved;
                            z.state = ZedState::Falling;
                            z.vertical_speed = 0.0;
                            z.air_velocity = if dt > 0.0 { (moved - old_centre).with_y(0.0) / dt } else { Vec3::ZERO };
                        }
                    }
                }
            }
        } else if !ai && z.state != ZedState::Falling {
            z.state = ZedState::Idle;
        }
        z.jump_cooldown = (z.jump_cooldown - dt).max(0.0);
        // ZombieCrawler.Bump: a pouncing Crawler that touches the player
        // hurts it once (MeleeDamage -5% .. +5%).
        if z.pouncing && z.state == ZedState::Falling && target_walking {
            let d = target - z.centre;
            let touching = d.with_y(0.0).length() / SCALE <= c.collision_radius + PLAYER_RADIUS + 2.0
                && (d.y / SCALE).abs() <= c.collision_height + PLAYER_HALF_HEIGHT;
            if touching {
                let roll = (z.random() % 1000) as f32 / 1000.0;
                let amount = z.melee_damage * 0.95 + z.melee_damage * 0.1 * roll;
                player_damage.write(crate::game::combat::PlayerDamaged {
                        amount,
                        armor_stops: true,
                        zed_id: z.id,
                        kind: crate::game::combat::HurtKind::Plain,
                        // ZombieCrawler.Bump: class'KFmod.ZombieMeleeDamage'.
                        dam_type: crate::game::combat::DamType::ZombieMelee,
                        source: Some(z.centre),
                        dam: None,
                        to_peer: target_peer,
                    });
                z.pouncing = false;
                runlog::kv("crawler_pounce_hit", &format!("id={} damage={amount:.1}", z.id));
            }
        }
        // JumpPad.Touch / PostTouch: a pawn touching a pad is thrown with
        // its JumpVelocity (falling), heading for its JumpTarget.
        let touching_pad = nav.jump_pads.iter().position(|p| {
            let d = nav.points[p.point].pos - z.centre;
            d.with_y(0.0).length() / SCALE < crate::world::nav::JUMP_PAD_RADIUS + c.collision_radius
                && (d.y / SCALE).abs() < crate::world::nav::JUMP_PAD_HALF_HEIGHT + c.collision_height
        });
        if let Some(i) = touching_pad
            && z.on_pad != Some(i)
            && z.health > 0.0
        {
            let pad = nav.jump_pads[i];
            // Launched from the pad's centre: the editor worked JumpVelocity
            // out for a jump from there. KF launches where the zed first
            // touches (up to 66 units off), which on the KF-WestLondon
            // fence pad hits the fence; whatever native detail gets KF's
            // zeds over is not known, so this is an approximation.
            let pc = nav.points[pad.point].pos;
            z.centre = Vec3::new(pc.x, z.centre.y, pc.z);
            z.state = ZedState::Falling;
            z.vertical_speed = pad.velocity.y;
            z.air_velocity = pad.velocity.with_y(0.0);
            z.yaw = yaw_of(pad.velocity.with_y(0.0));
            z.attack = None;
            runlog::kv(
                "zed_jump_pad",
                &format!("id={} pad={} target={} up_unreal={:.0}", z.id, nav.points[pad.point].name, nav.points[pad.target].name, pad.velocity.y / SCALE),
            );
        }
        z.on_pad = touching_pad;
        if z.state == ZedState::Falling {
            // PHYS_Falling: gravity, the horizontal velocity kept (AirControl
            // 0.05 is not applied), sliding along whatever is hit.
            z.vertical_speed -= GRAVITY * SCALE * dt;
            // Pawns block falling zeds too (sideways part), as walking ones.
            let mut air = z.air_velocity * dt;
            if let Some(me) = z.blocking_cylinder() {
                let shapes: Vec<Cylinder> = blockers.iter().filter(|(e, _)| *e != Some(entity)).map(|(_, c)| *c).collect();
                let (clipped, by) = clip_move(&me, air, &shapes);
                if by.is_some() {
                    z.air_velocity = Vec3::ZERO;
                }
                air = clipped;
            }
            let old = z.centre;
            let (moved, hit) = mover.slide(z.centre, air + Vec3::Y * z.vertical_speed * dt);
            z.centre = moved;
            if hit.as_ref().is_some_and(|h| h.normal.y < -0.7) && z.vertical_speed > 0.0 {
                z.vertical_speed = 0.0; // head hit a ceiling
            }
            let normal = hit.as_ref().map(|h| h.normal);
            // KF lands on a floor (normal up 0.7 or more), or in a V between
            // two up-facing slopes facing each other (motion.rs `ditch`).
            let in_ditch = normal.is_some_and(|n| motion::ditch(z.motion.fall_hit, n, old.y - moved.y));
            if in_ditch {
                let u = moved / SCALE;
                runlog::kv("zed_ditch_landed", &format!("id={} at_unreal=({:.0}, {:.0}, {:.0})", z.id, -u.z, u.x, u.y));
            }
            if normal.is_some() {
                z.motion.fall_hit = normal;
            }
            if normal.is_some_and(|n| n.y > 0.7) || in_ditch {
                let impact = -z.vertical_speed / SCALE;
                if z.health > 0.0 && impact > 0.0 {
                    z.sound_events.push(ZedSound::Land((0.3 * impact / c.jump_z).min(1.0)));
                }
                z.pouncing = false; // Landed
                z.vertical_speed = 0.0;
                z.air_velocity = Vec3::ZERO;
                // Landed (LandAnims) only after a real fall: faster than half
                // the jump speed (assumed; the engine decides natively). A
                // drop of a few units goes straight back to walking.
                if impact < 0.5 * c.jump_z {
                    z.state = ZedState::Chase;
                } else if c.land_anim.is_some() {
                    z.state = ZedState::Landing;
                    z.sequence = None;
                    start_anim(&mut z, c.land_anim, false);
                } else {
                    z.state = ZedState::Idle;
                }
            } else if dt > 0.0 {
                // KF recomputes the velocity from the actual move after each
                // falling step, so whatever blocked the fall takes its speed.
                let actual = (moved - old) / dt;
                z.air_velocity = actual.with_y(0.0);
                z.vertical_speed = actual.y;
            }
        }
        // A fall that does not end (wedged between surfaces) is logged once.
        if z.state == ZedState::Falling {
            let before = z.motion.fall_seconds;
            z.motion.fall_seconds += dt;
            if before < motion::LONG_FALL && z.motion.fall_seconds >= motion::LONG_FALL {
                let u = z.centre / SCALE;
                runlog::kv(
                    "zed_fall_long",
                    &format!(
                        "id={} at_unreal=({:.0}, {:.0}, {:.0}) seconds={:.1} air_speed_unreal={:.0} vertical_unreal={:.0}",
                        z.id,
                        -u.z,
                        u.x,
                        u.y,
                        z.motion.fall_seconds,
                        z.air_velocity.length() / SCALE,
                        z.vertical_speed / SCALE
                    ),
                );
            }
        } else {
            z.motion.fall_seconds = 0.0;
            z.motion.fall_hit = None;
        }

        match z.state {
            ZedState::Chase => {
                // Not actually moving (e.g. pressed against the player): idle,
                // or a turn-in-place animation while turning. The engine picks
                // these natively; this is an approximation of that rule.
                let speed = if dt > 0.0 { (z.centre - old_centre).with_y(0.0).length() / SCALE / dt } else { 0.0 };
                let velocity = if dt > 0.0 { (z.centre - old_centre) / SCALE / dt } else { Vec3::ZERO };
                let turn_rate = if dt > 0.0 { (z.yaw - old_yaw) / dt } else { 0.0 };
                let moving = speed >= STANDING_SPEED;
                let anim = if moving {
                    if z.decapitated {
                        c.headless_walk.or(c.walk)
                    } else if z.zapped() || (z.burn_down > 0 && z.burn_down < CRISP_UP_THRESHOLD) {
                        // SetZappedBehavior also sets the burning walk.
                        // ZombieCrispUp (BurnDown below CrispUpThreshhold) ->
                        // SetBurningBehavior: MovementAnims[0] = BurningWalkFAnims.
                        c.burning_walk.or(c.walk)
                    } else if z.running {
                        c.run_anim.or(c.walk)
                    } else if z.fp_rage.is_some() {
                        c.fp_charge_walk.or(c.walk)
                    } else if z.raging || z.saw_charging {
                        c.charge_anim.or(c.walk)
                    } else if z.boss.is_some_and(|b| b.charge.is_some() || b.escaping()) && z.attack.is_none() {
                        // ZombieBoss.PostNetReceive: charging, MovementAnims = ChargingAnim.
                        c.boss.as_ref().and_then(|b| b.charge_walk).or(c.walk)
                    } else {
                        c.walk
                    }
                } else if turn_rate > TURNING_RATE {
                    c.turn_right.or(c.idle)
                } else if turn_rate < -TURNING_RATE {
                    c.turn_left.or(c.idle)
                } else {
                    c.idle
                };
                play_chase_anim(&mut z, c, moving, anim, anim, velocity)
            }
            ZedState::Falling => start_anim(&mut z, c.air_anim.or(c.idle), true),
            ZedState::Idle => start_anim_tween(&mut z, c.idle, true, IDLE_TWEEN),
            ZedState::Melee | ZedState::KnockedDown | ZedState::Landing | ZedState::Enraging | ZedState::BossBusy | ZedState::DoorBashing | ZedState::Dead => {}
        }
        if z.sequence != old_sequence {
            runlog::kv(
                "zed_anim",
                &format!("id={} state={:?} sequence={}", z.id, z.state, z.sequence.and_then(|s| c.model.sequence_name(s)).unwrap_or("none")),
            );
        }
        if z.state != old_state {
            runlog::kv(
                "zed_state",
                &format!("id={} from={old_state:?} to={:?} distance_unreal={dist:.0}", z.id, z.state),
            );
        }
        if dt > 0.0 {
            z.velocity = (z.centre - old_centre) / dt;
        }
        if let Some(b) = blockers.iter_mut().find(|(e, _)| *e == Some(entity)) {
            b.1.centre = z.centre;
        }
        t.translation = z.centre;
        t.rotation = coords::rotation(Rotator { pitch: 0, yaw: z.yaw as i32, roll: 0 });
        if log_now {
            let u = z.centre / SCALE;
            runlog::kv(
                "zed",
                &format!(
                    "id={} state={:?} centre_unreal=({:.0}, {:.0}, {:.0}) distance_to_player={dist:.0} yaw={:.0} sequence={:?} frame={:.1} anim_rate={:.2} speed_unreal={:.0} running={}",
                    z.id,
                    z.state,
                    -u.z,
                    u.x,
                    u.y,
                    z.yaw,
                    z.sequence,
                    z.frame,
                    z.anim.rate,
                    z.velocity.with_y(0.0).length() / SCALE,
                    z.running
                ),
            );
        }
    }
}
