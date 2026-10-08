//! Animation each frame: playing sequences and the upper-body layer, sound and script notifies, posing the mesh, head and extended collision.

use super::*;

/// The frame spans `(from, to]` passed on the way from frame `prev` to
/// `frame` of an animation `len` frames long: a sequence just started
/// counts from frame 0; a lower frame than before means it wrapped
/// (looping) or restarted. KF fires a notify only when the frame moves
/// from strictly before it to at or after it, and a sequence starts on
/// frame 0 (or just after), so a notify at time 0 never fires.
pub(super) fn notify_spans(prev: Option<f32>, frame: f32, len: f32, looping: bool) -> [(f32, f32); 2] {
    match prev {
        None => [(0.0, frame), (0.0, 0.0)],
        Some(p) if frame >= p => [(p, frame), (0.0, 0.0)],
        Some(p) if looping => [(p, len), (0.0, frame)],
        Some(_) => [(0.0, frame), (0.0, 0.0)],
    }
}

/// True if a notify at `time` (0..1 of the sequence) lies in one of the spans.
pub(super) fn notify_in_spans(time: f32, len: f32, spans: &[(f32, f32)]) -> bool {
    spans.iter().any(|&(a, b)| time * len > a && time * len <= b)
}

/// The notifies of `seq` passed on the way from frame `prev` to `frame`
/// (see `notify_spans`).
pub(super) fn passed_notifies(model: &SkinnedModel, seq: usize, prev: Option<f32>, frame: f32, looping: bool) -> Vec<ue_assets::skeletal::Notify> {
    let len = model.length(seq);
    let spans = notify_spans(prev, frame, len, looping);
    model.notifies(seq).iter().filter(|n| notify_in_spans(n.time, len, &spans)).cloned().collect()
}

/// Tween in time of the upper-body layer (KFMonster.DoAnimAction plays
/// channel 1 with tween 0.1) and its fade out once it ends (xPawn.AnimEnd:
/// AnimBlendToAlpha(1, 0, 0.12)).
pub(super) const LAYER_TWEEN: f32 = 0.1;
pub(super) const LAYER_FADE_OUT: f32 = 0.12;
/// Tween into a new main-channel animation: zed actions (DoAnimAction),
/// movement, turning and air animations 0.1 s; the idle (PlayIdle,
/// IdleRestAnim) 0.25 s.
pub(super) const ANIM_TWEEN: f32 = 0.1;
pub(super) const IDLE_TWEEN: f32 = 0.25;

/// Animation state beyond the playing sequences: tweens, the fading
/// upper-body layer, and the local bone pose last shown.
pub(super) struct ZedAnim {
    /// PlayAnim rate of the main sequence (x the sequence's own frames per
    /// second): the movement animations scale it with speed, everything
    /// else plays at 1.
    pub rate: f32,
    /// Direction index of the last movement animation (0 forward, 1 back,
    /// 2 left, 3 right).
    pub move_dir: usize,
    /// The main channel's tween into its animation; the animation's clock
    /// (frame, notifies, attack progress) waits for it.
    pub tween: Option<crate::render::anim::Tween>,
    /// Asked for by `start_anim`; started by `animate_zeds` from the pose
    /// last shown.
    pub tween_request: Option<f32>,
    /// Local bone transforms of the pose last drawn (tweens start here).
    pub last_locals: Vec<(Quat, Vec3)>,
    /// The upper-body layer's tween into its first frame.
    pub overlay_tween: Option<crate::render::anim::Tween>,
    /// `Zed::overlay` as this system left it; anything else there means
    /// another system started a new layer animation.
    pub overlay_written: Option<(usize, f32, usize)>,
    /// A finished upper-body layer holding its last key while its weight
    /// fades out: (sequence, frame, root bone, fade).
    pub overlay_fade: Option<(usize, f32, usize, crate::render::anim::Fade)>,
}

impl Default for ZedAnim {
    fn default() -> Self {
        ZedAnim {
            rate: 1.0,
            move_dir: 0,
            tween: None,
            tween_request: None,
            last_locals: Vec::new(),
            overlay_tween: None,
            overlay_written: None,
            overlay_fade: None,
        }
    }
}

/// Engine rule for which of the four movement animations plays: from the
/// horizontal velocity (Unreal x, y) and the actor's yaw (rotation units).
/// Forward if the movement direction's dot with facing is over 0.82
/// (within about 35 degrees), back under -0.82, else right if it points to
/// the actor's right (+Y), else left.
pub(super) fn four_way(velocity: Vec2, yaw: f32) -> usize {
    if velocity.x.abs() < 1e-4 && velocity.y.abs() < 1e-4 {
        return 0;
    }
    let d = velocity.normalize_or_zero();
    let a = yaw * std::f32::consts::TAU / 65536.0;
    let (x, y) = (Vec2::new(a.cos(), a.sin()), Vec2::new(-a.sin(), a.cos()));
    let f = d.dot(x);
    if f > 0.82 {
        0
    } else if f < -0.82 {
        1
    } else if d.dot(y) > 0.0 {
        3
    } else {
        2
    }
}

/// Engine rule for the movement animation's rate: speed over the class's
/// default GroundSpeed x 1.1 (not the zed's own randomised or raging
/// speed), no clamp.
pub(super) fn move_rate(speed: f32, default_ground_speed: f32) -> f32 {
    speed / (default_ground_speed * 1.1).max(1e-3)
}

/// A chasing zed's animation each tick. Moving: `forward` (MovementAnims[0]
/// as its state sets it) or the direction's animation, looping at a rate
/// from its speed; a direction whose animation the mesh lacks keeps the
/// current animation playing (KF's PlayAnim fails and nothing changes).
/// Standing: `still` (idle or turning) at rate 1. `velocity` in Unreal
/// units/s, Bevy axes.
pub(super) fn play_chase_anim(z: &mut Zed, c: &ZedClass, moving: bool, forward: Option<usize>, still: Option<usize>, velocity: Vec3) {
    if !moving {
        let tween = if still == c.idle { IDLE_TWEEN } else { ANIM_TWEEN };
        start_anim_tween(z, still, true, tween);
        return;
    }
    let dir = four_way(Vec2::new(-velocity.z, velocity.x), z.yaw);
    let burning = z.zapped() || (z.burn_down > 0 && z.burn_down < CRISP_UP_THRESHOLD);
    let anim = match dir {
        0 => forward,
        // ZombieBoss charging: all four are ChargingAnim.
        _ if z.boss.is_some_and(|b| b.charge.is_some() || b.escaping()) && z.attack.is_none() => forward,
        _ if z.decapitated => c.headless_dirs[dir],
        _ if burning => c.burning_dirs[dir - 1],
        _ => c.walk_dirs[dir],
    };
    let Some(anim) = anim else {
        return;
    };
    let rate = move_rate(velocity.length(), c.ground_speed);
    let changed = z.sequence != Some(anim) || z.anim.move_dir != dir;
    start_anim(z, Some(anim), true);
    z.anim.rate = rate;
    z.anim.move_dir = dir;
    if changed {
        runlog::kv(
            "zed_move_anim",
            &format!("id={} sequence={} dir={dir} speed_unreal={:.0} rate={rate:.2}", z.id, c.model.sequence_name(anim).unwrap_or("?"), velocity.length()),
        );
    }
}

/// Plays `seq` on the main channel at rate 1 (restarting it unless it is
/// already playing), tweening into it over 0.1 s.
pub(super) fn start_anim(z: &mut Zed, seq: Option<usize>, looping: bool) {
    start_anim_tween(z, seq, looping, ANIM_TWEEN);
}

/// `start_anim` with a given tween time (KF PlayAnim's TweenTime): a new
/// animation starts from the pose on screen and reaches its first frame
/// after `tween` seconds; only then does its clock run.
pub(super) fn start_anim_tween(z: &mut Zed, seq: Option<usize>, looping: bool, tween: f32) {
    if z.sequence != seq {
        z.sequence = seq;
        z.frame = 0.0;
        z.anim.tween = None;
        z.anim.tween_request = Some(tween);
    }
    z.looping = looping;
    z.anim.rate = 1.0;
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
                // A puppet's globs come from the host's Bloat (net/zeds.rs).
                for _ in 0..if z.net.puppet { 0 } else { 4 } {
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
        // A new main animation tweens from the pose last shown; its clock
        // waits, then runs the leftover time of the tick the tween ends in.
        if let Some(tween) = z.anim.tween_request.take() {
            z.anim.tween = crate::render::anim::Tween::start(&z.anim.last_locals, tween);
            if z.anim.tween.is_some() {
                runlog::kv(
                    "zed_anim_tween",
                    &format!("id={} sequence={} tween={tween} start", z.id, z.sequence.and_then(|s| c.model.sequence_name(s)).unwrap_or("none")),
                );
            }
        }
        let play_dt = match z.anim.tween.as_mut().map(|t| t.advance(dt)) {
            None => dt,
            Some(None) => 0.0,
            Some(Some(left)) => {
                let total = z.anim.tween.take().map_or(0.0, |t| t.total);
                runlog::kv(
                    "zed_anim_tween",
                    &format!("id={} sequence={} tween={total} end", z.id, z.sequence.and_then(|s| c.model.sequence_name(s)).unwrap_or("none")),
                );
                left
            }
        };
        if let Some(s) = z.sequence {
            let len = c.model.length(s).max(1e-3);
            z.frame += play_dt * c.model.rate(s) * z.anim.rate;
            if z.looping {
                z.frame %= len;
            } else {
                // A one-shot stops on its last key (N - 1) and holds it.
                z.frame = z.frame.min(c.model.last_frame(s));
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
        // Upper-body layer, played once. A new one (set by another system)
        // tweens from the pose on screen to its first frame over 0.1 s and
        // cancels a fading one; it ends on its last key, which then holds
        // while the layer's weight fades to 0 over 0.12 s.
        if let Some((_, _, _, fade)) = z.anim.overlay_fade.as_mut()
            && fade.advance(dt)
        {
            z.anim.overlay_fade = None;
            runlog::kv("zed_layer_fade_done", &format!("id={}", z.id));
        }
        if z.overlay.is_some() && z.overlay != z.anim.overlay_written {
            z.anim.overlay_fade = None;
            z.anim.overlay_tween = crate::render::anim::Tween::start(&z.anim.last_locals, LAYER_TWEEN);
            if let Some((seq, _, _)) = z.overlay {
                runlog::kv("zed_layer_start", &format!("id={} sequence={} tween={LAYER_TWEEN}", z.id, c.model.sequence_name(seq).unwrap_or("?")));
            }
        }
        if let Some((seq, f, root)) = z.overlay {
            // The layer's clock waits for its tween; the leftover time of
            // the tick the tween ends in is played.
            let play_dt = match z.anim.overlay_tween.as_mut().map(|t| t.advance(dt)) {
                None => dt,
                Some(None) => 0.0,
                Some(Some(left)) => {
                    z.anim.overlay_tween = None;
                    left
                }
            };
            let next = f + play_dt * c.model.rate(seq);
            let last = c.model.last_frame(seq);
            z.overlay = (next < last).then_some((seq, next, root));
            if next >= last {
                z.anim.overlay_tween = None;
                z.anim.overlay_fade = Some((seq, last, root, crate::render::anim::Fade { alpha: 1.0, target: 0.0, left: LAYER_FADE_OUT }));
                runlog::kv("zed_layer_end", &format!("id={} sequence={} fade_out={LAYER_FADE_OUT}", z.id, c.model.sequence_name(seq).unwrap_or("?")));
            }
            let reached = next.min(last);
            let prev = z.overlay_sounds_heard.filter(|(s, _)| *s == seq).map(|(_, f)| f);
            let passed = passed_notifies(&c.model, seq, prev, reached, false);
            heard.extend(passed.iter().filter_map(|n| n.sound.clone()));
            speech.extend(passed.iter().filter(|_| c.boss.is_some()).filter_map(|n| boss_speech(&n.name)));
            z.overlay_sounds_heard = Some((seq, reached));
        } else {
            z.anim.overlay_tween = None;
        }
        z.anim.overlay_written = z.overlay;
        // AnimNotify_Sound: played on the zed. Its slot and radius handling
        // are native (not in the scripts): SLOT_None and the default radius
        // for 0 are guesses; volumes over 1 (Siren scream 255) are capped
        // by the mixer.
        z.sound_events.extend(speech.into_iter().map(ZedSound::Line));
        for n in heard {
            let radius = if n.radius > 0.0 { n.radius } else { crate::audio::mixer::DEFAULT_RADIUS };
            sounds.write(crate::audio::mixer::PlaySound::new(n.sound, crate::audio::mixer::Emitter::Entity(entity)).volume(n.volume).radius(radius));
        }
        // The pose: the main sequence, the fading layer at its weight, the
        // playing layer over it (through its tween).
        let mut locals = c.model.sample_locals(z.sequence, z.frame);
        if let Some(tw) = z.anim.tween.as_ref().filter(|t| t.from.len() == locals.len()) {
            let mut from = tw.from.clone();
            c.model.blend_locals(&mut from, &locals, tw.weight(), None);
            locals = from;
        }
        if let Some((seq, f, root, fade)) = z.anim.overlay_fade {
            let layer = c.model.sample_locals(Some(seq), f);
            c.model.blend_locals(&mut locals, &layer, fade.alpha, Some(root));
        }
        if let Some((seq, f, root)) = z.overlay {
            let mut layer = c.model.sample_locals(Some(seq), f);
            if let Some(tw) = z.anim.overlay_tween.as_ref().filter(|t| t.from.len() == layer.len()) {
                let mut from = tw.from.clone();
                c.model.blend_locals(&mut from, &layer, tw.weight(), None);
                layer = from;
            }
            c.model.blend_locals(&mut locals, &layer, 1.0, Some(root));
        }
        // Decapitated: the head (and anything under it) shrinks into the neck.
        let bones = c.model.pose_from_locals(&locals, &[]);
        let skinned = c.model.skin(&bones, &collapse);
        z.anim.last_locals = locals;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Steps a looping 30-frame animation at 30 fps over two cycles and
    /// returns how often a notify at `time` fired.
    fn fires_over_two_loops(time: f32) -> usize {
        let len = 30.0;
        let (mut prev, mut frame, mut count) = (None, 0.0f32, 0);
        for _ in 0..120 {
            frame = (frame + 0.5) % len;
            if notify_in_spans(time, len, &notify_spans(prev, frame, len, true)) {
                count += 1;
            }
            prev = Some(frame);
        }
        count
    }

    #[test]
    fn four_way_uses_the_0_82_rule() {
        // Facing +X (yaw 0); the actor's right is +Y.
        assert_eq!(four_way(Vec2::new(1.0, 0.0), 0.0), 0);
        assert_eq!(four_way(Vec2::new(0.83, 0.5578), 0.0), 0); // dot 0.83
        assert_eq!(four_way(Vec2::new(1.0, 1.0), 0.0), 3); // 45 deg: right
        assert_eq!(four_way(Vec2::new(1.0, -1.0), 0.0), 2);
        assert_eq!(four_way(Vec2::new(-1.0, 0.1), 0.0), 1);
        assert_eq!(four_way(Vec2::ZERO, 0.0), 0);
        // Facing +Y (yaw 16384): its right is (-sin, cos) = -X.
        assert_eq!(four_way(Vec2::new(0.0, 1.0), 16384.0), 0);
        assert_eq!(four_way(Vec2::new(-1.0, 0.0), 16384.0), 3);
    }

    #[test]
    fn move_rate_is_speed_over_default_ground_speed_x_1_1() {
        // Clot: default GroundSpeed 105.
        assert!((move_rate(115.5, 105.0) - 1.0).abs() < 1e-5);
        // Raging Fleshpound: 2.3 x 130 = 299 uu/s.
        assert!((move_rate(299.0, 130.0) - 2.0909).abs() < 1e-3);
    }

    /// The main channel's clock as `animate_zeds` runs it: a tween first,
    /// then frames. Checks when a notify at 0.5 fires and when the
    /// one-shot reaches its last key, for a 30-frame, 30 fps animation.
    #[test]
    fn tween_delays_the_clock_notifies_and_end() {
        let (len, fps, dt) = (30.0f32, 30.0f32, 1.0 / 60.0);
        let pose = vec![(Quat::IDENTITY, Vec3::ZERO)];
        let mut tween = crate::render::anim::Tween::start(&pose, ANIM_TWEEN);
        // The frame the attack starts in (its time step is before the start).
        assert_eq!(tween.as_mut().unwrap().advance(dt), None);
        let (mut frame, mut prev, mut t) = (0.0f32, None, 0.0f32);
        let (mut notify_at, mut end_at) = (None, None);
        let last = crate::render::skinned::last_frame(30);
        for _ in 0..200 {
            t += dt;
            let play_dt = match tween.as_mut().map(|tw| tw.advance(dt)) {
                None => dt,
                Some(None) => 0.0,
                Some(Some(left)) => {
                    tween = None;
                    left
                }
            };
            if tween.is_some() {
                assert_eq!(frame, 0.0, "clock waits during the tween");
            }
            frame = (frame + play_dt * fps).min(last);
            if notify_at.is_none() && notify_in_spans(0.5, len, &notify_spans(prev, frame, len, false)) {
                notify_at = Some(t);
            }
            if end_at.is_none() && frame >= last {
                end_at = Some(t);
            }
            prev = Some(frame);
        }
        let (n, e) = (notify_at.unwrap(), end_at.unwrap());
        assert!((n - 0.6).abs() <= dt + 1e-4, "notify at {n}");
        assert!((e - (0.1 + 29.0 / 30.0)).abs() <= dt + 1e-4, "end at {e}");
    }

    #[test]
    fn notify_at_time_zero_never_fires() {
        assert_eq!(fires_over_two_loops(0.0), 0);
        // Just after 0 it fires once per cycle; also at 0.5.
        assert_eq!(fires_over_two_loops(0.001), 2);
        assert_eq!(fires_over_two_loops(0.5), 2);
        // A sequence that just started from frame 0 (no previous frame).
        assert!(!notify_in_spans(0.0, 30.0, &notify_spans(None, 1.0, 30.0, false)));
        assert!(notify_in_spans(0.01, 30.0, &notify_spans(None, 1.0, 30.0, false)));
    }
}
