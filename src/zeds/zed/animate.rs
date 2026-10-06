//! Animation each frame: playing sequences and the upper-body layer, sound and script notifies, posing the mesh, head and extended collision.

use super::*;

/// The notifies of `seq` passed on the way from frame `prev` to
/// `frame`: a sequence just started counts from before frame 0; a lower
/// frame than before means it wrapped (looping) or restarted.
pub(super) fn passed_notifies(model: &SkinnedModel, seq: usize, prev: Option<f32>, frame: f32, looping: bool) -> Vec<ue_assets::skeletal::Notify> {
    let len = model.length(seq);
    let spans: &[(f32, f32)] = &match prev {
        None => [(-1.0, frame), (0.0, 0.0)],
        Some(p) if frame >= p => [(p, frame), (0.0, 0.0)],
        Some(p) if looping => [(p, len), (-1.0, frame)],
        Some(_) => [(-1.0, frame), (0.0, 0.0)],
    };
    model
        .notifies(seq)
        .iter()
        .filter(|n| spans.iter().any(|&(a, b)| n.time * len > a && n.time * len <= b))
        .cloned()
        .collect()
}

pub(super) fn start_anim(z: &mut Zed, seq: Option<usize>, looping: bool) {
    if z.sequence != seq {
        z.sequence = seq;
        z.frame = 0.0;
    }
    z.looping = looping;
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)] // Bevy system parameters
pub(super) fn animate_zeds(
    mut commands: Commands,
    time: Res<Time>,
    classes: Option<Res<ZedClasses>>,
    gore: Option<Res<GoreAssets>>,
    library: Option<Res<EffectLibrary>>,
    mut effects: Query<&mut ParticleEffect>,
    settings: Res<ZedSettings>,
    mut decals: MessageWriter<SpawnDecal>,
    mut vomit: MessageWriter<crate::zeds::vomit::SpawnVomit>,
    mut zeds: Query<(Entity, &mut Zed, &Transform, Option<&mut RagdollState>)>,
    bodies: Query<(&Transform, &LinearVelocity, Has<Sleeping>, &AngularVelocity), With<RagdollBody>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut log_timer: Local<f32>,
    (mut sounds, mut preload, mut preloaded): (MessageWriter<crate::audio::mixer::PlaySound>, MessageWriter<crate::audio::mixer::PreloadSounds>, Local<bool>),
) {
    let Some(classes) = classes else {
        return;
    };
    if !*preloaded {
        *preloaded = true;
        for c in &classes.0 {
            let mut list = c.model.all_notify_sounds();
            let v = &c.sounds;
            list.extend(
                [&v.moan, &v.pain, &v.death, &v.headless_death, &v.decapitation, &v.melee_hit, &v.saw_loop, &v.chainsaw_off, &v.rocket_fire, &v.impale_hit, &v.mg_fire, &v.mg_spin]
                    .into_iter()
                    .flatten()
                    .cloned(),
            );
            if c.boss.is_some() {
                list.extend(["PatriarchKnockDown", "PatriarchEntrance", "PatriarchVictory", "PatriarchMGPreFire", "PatriarchMisslePreFire"].iter().filter_map(|f| boss_speech(f)).map(|l| l.sound.to_string()));
                list.push("KF_EnemiesFinalSnd.Patriarch.Kev_SaveMe".into());
            }
            list.extend(v.challenge.iter().cloned());
            list.extend(v.ambient.as_ref().map(|a| a.sound.clone()));
            list.sort();
            list.dedup();
            preload.write(crate::audio::mixer::PreloadSounds { what: c.name.clone(), sounds: list });
        }
    }
    let dt = time.delta_secs();
    *log_timer += dt;
    let log_now = *log_timer >= 1.0;
    if log_now {
        *log_timer = 0.0;
    }
    for (entity, mut z, t, ragdoll_state) in &mut zeds {
        let c = &classes.0[z.class];
        // Before the ragdoll branch below, which skips the rest: the death
        // sound comes 0.2 s after death, when the zed is usually a ragdoll.
        play_zed_sounds(&mut commands, entity, c, &mut z, &mut sounds);
        if let Some(gore) = gore.as_deref() {
            let clock = (time.elapsed_secs_f64() * 1000.0) as u32 ^ std::process::id();
            apply_gore(&mut commands, &mut meshes, gore, library.as_deref(), c, &mut z, entity, t, clock, settings.always_sever, &mut decals);
        }
        let seed = z.random();
        for effect in std::mem::take(&mut z.pending_fx) {
            notify_effect(&mut commands, &mut meshes, library.as_deref(), c, &mut z, t, &effect, seed);
        }
        // ZombieBoss.AddTraceHitFX: mMuzzleFlash. KF quirk, kept: the first
        // call only spawns and attaches it ("if( mMuzzleFlash==None )
        // Spawn... else SpawnParticle(1)"), so the first shot of his life
        // shows no flash. Later shots: Emitter.SpawnParticle(1), one
        // particle on each of its 8 emitters.
        if z.mg_flash_shots > 0
            && let (Some(bc), Some(lib)) = (c.boss.as_ref(), library.as_deref())
        {
            match z.mg_flash {
                None => {
                    z.mg_flash_shots -= 1;
                    if let Some(bone) = bc.tip_bone {
                        let anchor = EffectAnchor::Bone {
                            bone,
                            offset: Vec3::ZERO,
                            rotation: Mat3::IDENTITY,
                        };
                        let frame = anchor_frame(c, &z, t, &anchor).unwrap_or((ue_pos(z.centre), Mat3::IDENTITY));
                        let options = crate::render::particles::SpawnOptions {
                            persistent: true,
                            ..default()
                        };
                        if let Some(e) = crate::render::particles::spawn_effect_with(&mut commands, lib, &mut meshes, "ROEffects.MuzzleFlash3rdMG", frame.0, frame.1, seed, options) {
                            z.mg_flash = Some(e);
                            z.effects.push((e, anchor));
                        }
                    }
                }
                Some(e) => {
                    if let Ok(mut fx) = effects.get_mut(e) {
                        for _ in 0..std::mem::take(&mut z.mg_flash_shots) {
                            fx.spawn_all(1);
                        }
                    }
                }
            }
        }
        // Dead: the flash goes once its particles are gone.
        if z.state == ZedState::Dead
            && let Some(e) = z.mg_flash.take()
            && let Ok(mut fx) = effects.get_mut(e)
        {
            fx.kill();
        }
        // ZombieBloat.PlayDyingAnimation and Tick: unless he bled out
        // headless he bursts: BileExplosion (BileExplosionHeadless without a
        // head) half his height up, SpineBone2 hidden, and BileBomb's BileJet
        // throws 4 globs up (Rotator(-Gravity) turned by Pitch 2000 and a
        // random yaw).
        if z.state == ZedState::Dead && c.burst_bone.is_some() && !z.burst_done {
            z.burst_done = true;
            if !z.bled_out {
                if let Some(b) = c.burst_bone {
                    z.hidden_bones.push(b);
                }
                let at = ue_pos(z.centre) + Vec3::Z * (0.5 * c.collision_height);
                let class = if z.decapitated { "KFMod.BileExplosionHeadless" } else { "KFMod.BileExplosion" };
                let k = std::f32::consts::TAU / 65536.0;
                let axes = coords::ue_rotation_matrix(Rotator { pitch: 0, yaw: z.yaw as i32, roll: 0 });
                let spawned = library
                    .as_deref()
                    .and_then(|lib| particles::spawn_effect(&mut commands, lib, &mut meshes, class, at, axes, seed));
                let tilt = 2000.0 * k;
                for _ in 0..4 {
                    let yaw = (z.random() % 65536) as f32 * k;
                    let dir = Vec3::new(tilt.sin() * yaw.cos(), tilt.sin() * yaw.sin(), tilt.cos());
                    vomit.write(crate::zeds::vomit::SpawnVomit {
                        at: ue_pos(z.centre),
                        velocity: dir * crate::zeds::vomit::SPEED,
                        zed_id: z.id,
                    });
                }
                runlog::kv(
                    "bloat_burst",
                    &format!("id={} effect={class} spawned={} at_unreal=({:.0}, {:.0}, {:.0})", z.id, spawned.is_some(), at.x, at.y, at.z),
                );
            }
        }
        // Hidden bones: the head once decapitated, and severed limbs.
        let mut collapse = z.hidden_bones.clone();
        if z.decapitated
            && let Some(h) = c.head_bone
        {
            collapse.push(h);
        }
        // Ragdoll: bones come from the physics bodies.
        if let (Some(mut r), Some(def)) = (ragdoll_state, c.ragdoll.as_ref()) {
            r.age += dt;
            let Some(pose) = r.pose(def, |e| bodies.get(e).ok().map(|(bt, _, _, _)| *bt)) else {
                continue; // bodies not spawned yet (first frame)
            };
            let first = r.ball_joints.iter().all(|w| w.max_swing == 0.0 && w.max_twist == 0.0);
            r.watch_limits(|e| bodies.get(e).ok().map(|(bt, _, _, _)| *bt));
            if first {
                runlog::kv("ragdoll_limits_at_death", &format!("id={} deg {}", z.id, r.limit_report(def)));
            }
            if r.age >= 5.0 && r.age - dt < 5.0 {
                runlog::kv("ragdoll_limits", &format!("id={} largest_seen_deg {}", z.id, r.limit_report(def)));
            }
            let skinned = c.model.skin(&pose, &collapse);
            let to_actor = mesh_to_actor(c);
            c.model.upload_to(&z.meshes, &skinned, |p| coords::pos(to_actor(p).to_array()), &mut meshes);
            if let Some(gore) = gore.as_deref() {
                for (kind, tag, scale, handles) in &z.stumps {
                    gore::place_stump(gore, *kind, &c.model, &pose, &to_actor, tag, *scale, handles, &mut meshes);
                }
            }
            z.last_pose = pose.clone();
            follow_tags(c, &z, t, &mut effects);
            let (fastest, max_speed) = r
                .bodies
                .iter()
                .enumerate()
                .filter_map(|(i, &e)| bodies.get(e).ok().map(|(_, v, _, _)| (i, v.0.length() / SCALE)))
                .fold((0, 0.0), |a, b| if b.1 > a.1 { b } else { a });
            let root = bodies.get(r.bodies[def.root]).ok().map(|(bt, _, _, _)| bt.translation / SCALE);
            let (spinning, max_spin) = r
                .bodies
                .iter()
                .enumerate()
                .filter_map(|(i, &e)| bodies.get(e).ok().map(|(_, _, _, w)| (i, w.0.length())))
                .fold((0, 0.0), |a, b| if b.1 > a.1 { b } else { a });
            let sleeping = r.bodies.iter().filter(|&&e| bodies.get(e).is_ok_and(|(_, _, s, _)| s)).count();
            if (r.age < 0.3 || log_now)
                && let Some((name, gap)) = r.worst_joint_gap(def, |e| bodies.get(e).ok().map(|(bt, _, _, _)| *bt))
            {
                runlog::kv("ragdoll_joint_gap", &format!("id={} age={:.2} worst_joint={name} gap_unreal={gap:.1}", z.id, r.age));
            }
            if r.age >= 2.0 && r.age - dt < 2.0 {
                // One-off snapshot: every part's offset from the root part.
                let parts: Vec<String> = r
                    .bodies
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &e)| {
                        let (bt, v, _, _) = bodies.get(e).ok()?;
                        let rel = (bt.translation - root? * SCALE) / SCALE;
                        Some(format!("{}:dist={:.0},up={:.0},speed={:.0}", def.part_name(i), rel.length(), rel.y, v.0.length() / SCALE))
                    })
                    .collect();
                runlog::kv("ragdoll_parts", &format!("id={} {}", z.id, parts.join(" ")));
                let gaps = r.joint_gaps(def, |e| bodies.get(e).ok().map(|(bt, _, _, _)| *bt));
                runlog::kv("ragdoll_joint_gaps", &format!("id={} {}", z.id, gaps.join(" ")));
            }
            if r.rested_at.is_none() && r.age > 0.5 && max_speed < 5.0 {
                r.rested_at = Some(r.age);
                runlog::kv("ragdoll_rested", &format!("id={} seconds={:.2}", z.id, r.age));
            }
            if log_now && let Some(root) = root {
                runlog::kv(
                    "ragdoll",
                    &format!(
                        "id={} age={:.1} root_unreal=({:.0}, {:.0}, {:.0}) root_height_above_death_spot_unreal={:.0} max_body_speed_unreal={max_speed:.0} fastest={} max_spin_rad_s={max_spin:.2} spinning={} twist_now_deg=[{}] sleeping_bodies={sleeping}",
                        z.id,
                        r.age,
                        -root.z,
                        root.x,
                        root.y,
                        root.y - (z.centre.y / SCALE - c.collision_height),
                        def.part_name(fastest),
                        def.part_name(spinning),
                        r.twist_now(def),
                    ),
                );
            }
            continue;
        }
        let mut heard = Vec::new();
        let mut speech = Vec::new();
        if let Some(s) = z.sequence {
            let len = c.model.length(s).max(1e-3);
            z.frame += time.delta_secs() * c.model.rate(s);
            if z.looping {
                z.frame %= len;
            } else {
                z.frame = z.frame.min(len);
            }
            if z.health <= 0.0 && Some(s) == c.death {
                z.frame = z.frame.min(c.death_hold_frame);
            }
            let prev = (z.sounds_heard.0 == Some(s)).then_some(z.sounds_heard.1);
            let passed = passed_notifies(&c.model, s, prev, z.frame, z.looping);
            heard.extend(passed.iter().filter_map(|n| n.sound.clone()));
            speech.extend(passed.iter().filter(|_| c.boss.is_some()).filter_map(|n| boss_speech(&n.name)));
            z.sounds_heard = (Some(s), z.frame);
        }
        // Start a pending upper-body hit reaction (KnockDown is full body
        // and handled by the think system).
        if let Some(r) = z.pending_reaction
            && r != HitReaction::KnockDown
        {
            z.pending_reaction = None;
            let seq = match r {
                HitReaction::Front => c.hit_reactions[0],
                HitReaction::Back => c.hit_reactions[1],
                HitReaction::Left => c.hit_reactions[2],
                HitReaction::Right => c.hit_reactions[3],
                HitReaction::Stun if !c.hit_anims.is_empty() => {
                    let i = z.random() as usize % c.hit_anims.len();
                    Some(c.hit_anims[i])
                }
                _ => None,
            };
            if let (Some(seq), Some(root)) = (seq, c.flinch_root.or(c.spine_bone)) {
                z.overlay = Some((seq, 0.0, root));
                runlog::kv(
                    "zed_hit_reaction",
                    &format!("id={} reaction={r:?} sequence={} stunned={:.1}", z.id, c.model.sequence_name(seq).unwrap_or("?"), z.stunned),
                );
            }
        }
        // Upper-body layer, played once.
        let overlay = z.overlay;
        if let Some((seq, f, root)) = z.overlay {
            let next = f + dt * c.model.rate(seq);
            z.overlay = (next < c.model.length(seq)).then_some((seq, next, root));
            let reached = next.min(c.model.length(seq));
            let prev = z.overlay_sounds_heard.filter(|(s, _)| *s == seq).map(|(_, f)| f);
            let passed = passed_notifies(&c.model, seq, prev, reached, false);
            heard.extend(passed.iter().filter_map(|n| n.sound.clone()));
            speech.extend(passed.iter().filter(|_| c.boss.is_some()).filter_map(|n| boss_speech(&n.name)));
            z.overlay_sounds_heard = Some((seq, reached));
        }
        // AnimNotify_Sound: played on the zed. Its slot and radius handling
        // are native (not in the scripts): SLOT_None and the default radius
        // for 0 are guesses; volumes over 1 (Siren scream 255) are capped
        // by the mixer.
        z.sound_events.extend(speech.into_iter().map(ZedSound::Line));
        for n in heard {
            let radius = if n.radius > 0.0 { n.radius } else { crate::audio::mixer::DEFAULT_RADIUS };
            sounds.write(crate::audio::mixer::PlaySound::new(n.sound, crate::audio::mixer::Emitter::Entity(entity)).volume(n.volume).radius(radius));
        }
        // Decapitated: the head (and anything under it) shrinks into the neck.
        let (skinned, bones) = c.model.pose_layered(z.sequence, z.frame, overlay, &collapse);
        let to_actor = mesh_to_actor(c);
        c.model.upload_to(&z.meshes, &skinned, |p| coords::pos(to_actor(p).to_array()), &mut meshes);
        if let Some(gore) = gore.as_deref() {
            for (kind, tag, scale, handles) in &z.stumps {
                gore::place_stump(gore, *kind, &c.model, &bones, &to_actor, tag, *scale, handles, &mut meshes);
            }
        }
        z.last_pose = bones.clone();
        follow_tags(c, &z, t, &mut effects);
        // Extended collision: centre + (ColOffset >> Rotation), hard-attached
        // to the actor (KFMonster.PostBeginPlay).
        z.ext = c.ext_collision.map(|(offset, r, h)| {
            (t.translation + t.rotation * coords::pos(offset.to_array()), r, h)
        });
        // Head sphere for headshots: head bone origin plus HeadHeight x
        // HeadScale along the bone's X axis (KFMonster.IsHeadShot).
        z.head = c.head_bone.map(|b| {
            let (q, p) = bones[b];
            let origin = to_actor(p);
            let axis = (to_actor(p + q * Vec3::X) - origin).normalize_or_zero();
            let centre = origin + axis * c.head_offset;
            let world = t.rotation * coords::pos(centre.to_array()) + t.translation;
            let world_axis = t.rotation * coords::dir(axis.to_array());
            (world, world_axis)
        });
    }
}
