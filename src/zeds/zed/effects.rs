//! Gore and attached effects: hit bones, severed limbs and stumps, effect anchors and notifies, burning, cloaks, the death launch.

use super::*;

/// DoDamageFX on a killing hit: the hit bone's name as KF maps it
/// (KFMonster default bone names; neither the Clot nor the Gorefast
/// overrides them).
pub(super) fn bone_on_death(name: &str) -> String {
    let n = name.to_ascii_lowercase();
    match n.as_str() {
        "neck" => "head".into(),
        "lfoot" | "lleg" => "lthigh".into(),
        "rfoot" | "rleg" => "rthigh".into(),
        "righthand" | "rshoulder" | "rarm" => "rfarm".into(),
        "lefthand" | "lshoulder" | "larm" => "lfarm".into(),
        _ => n,
    }
}

/// A mesh-space frame (origin, axes) of a zed at `t` in Unreal world space:
/// position and axes (columns X, Y, Z).
pub(super) fn world_axes(c: &ZedClass, t: &Transform, (o, axes): (Vec3, [Vec3; 3])) -> (Vec3, Mat3) {
    let to_actor = mesh_to_actor(c);
    let origin = to_actor(o);
    let axes = axes.map(|a| ue_dir(t.rotation * coords::dir((to_actor(o + a) - origin).normalize_or_zero().to_array())));
    let pos = ue_pos(t.transform_point(coords::pos(origin.to_array())));
    (pos, Mat3::from_cols(axes[0], axes[1], axes[2]))
}

/// Like `world_axes`, with the rotation as a rotator.
pub(super) fn world_frame(c: &ZedClass, t: &Transform, frame: (Vec3, [Vec3; 3])) -> (Vec3, Vec3) {
    let (pos, m) = world_axes(c, t, frame);
    (pos, coords::ue_rotator_of(m))
}

/// AttachEmitterEffect: starts `class` at the zed's tag `tag`; it follows
/// the tag from then on.
#[allow(clippy::too_many_arguments)]
pub(super) fn attach_effect(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    library: Option<&EffectLibrary>,
    c: &ZedClass,
    z: &mut Zed,
    t: &Transform,
    class: &str,
    tag: &'static str,
    seed: u32,
) {
    let (Some(library), Some(frame)) = (library, c.model.tag_frame(&z.last_pose, tag)) else {
        return;
    };
    let (pos, axes) = world_axes(c, t, frame);
    if let Some(e) = particles::spawn_effect(commands, library, meshes, class, pos, axes, seed) {
        z.effects.push((e, EffectAnchor::Tag(tag)));
    }
}

/// The bone a shot hit (KF: native CalcHitLoc; here the bone segment, bone
/// to its first child, nearest the shot's line), its distance from the line
/// in Unreal units, and its name as KF sees it (its tag, else its name).
pub(super) fn hit_bone(c: &ZedClass, pose: &[(Quat, Vec3)], t: &Transform, point: Vec3, dir: Vec3) -> Option<(usize, f32, String)> {
    let to_actor = mesh_to_actor(c);
    let world: Vec<Vec3> = pose.iter().map(|(_, p)| t.transform_point(coords::pos(to_actor(*p).to_array()))).collect();
    let bones = &c.model.mesh.bones;
    let d = dir.normalize_or_zero();
    (0..world.len())
        .map(|i| {
            let child = (i + 1..bones.len()).find(|&k| bones[k].parent == i);
            let end = child.map_or(world[i], |k| world[k]);
            (i, line_segment_distance(point, d, world[i], end) / SCALE)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, dist)| (i, dist, c.model.bone_tag(i).unwrap_or(&bones[i].name).to_string()))
}

/// KFMonster.DoDamageFX / ProcessHitFX for a zed's new damage events:
/// decapitation effects (gun: neck stump and brain chunks; knife: neck stump
/// and a flying head) and, on the killing hit, maybe a severed limb.
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_gore(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    gore: &GoreAssets,
    library: Option<&EffectLibrary>,
    c: &ZedClass,
    z: &mut Zed,
    entity: Entity,
    t: &Transform,
    clock: u32,
    always_sever: bool,
    decals: &mut MessageWriter<SpawnDecal>,
) {
    let hits = std::mem::take(&mut z.gore_hits);
    let velocity = ue_dir(z.velocity) / SCALE;
    let size = c.collision_radius * c.collision_height / 1100.0;
    for hit in hits {
        // Seeded with the clock too: KF's FRand differs every game.
        let mut rng = gore::Rng((z.random() ^ clock.wrapping_mul(2_654_435_761)) | 1);
        // KFMonster.OldPlayHit: the damage type's PawnDamageEmitter
        // (ROBloodPuff for the 9mm and knife) at the hit point pushed one
        // CollisionRadius away from the attacker, X axis toward the attacker.
        if hit.damage > 0.0
            && let Some(library) = library
        {
            let n = ue_dir(-hit.dir).normalize_or_zero();
            let at = ue_pos(hit.point) + n - n * c.collision_radius;
            let k = 65536.0 / std::f32::consts::TAU;
            let axes = coords::ue_rotation_matrix(Rotator {
                pitch: (n.z.clamp(-1.0, 1.0).asin() * k) as i32,
                yaw: (n.y.atan2(n.x) * k) as i32,
                roll: 0,
            });
            particles::spawn_effect(commands, library, meshes, "ROEffects.ROBloodPuff", at, axes, rng.0);
        }
        // KFMonster.TakeDamage: a ProjectileBloodSplat for blood-causing
        // damage, only 20% of the time for a hit within 0.2 s of the last;
        // it traces 350 units along the shot and splats the wall it reaches.
        let recent = z.since_hit < 0.2;
        z.since_hit = 0.0;
        if !recent || rng.frand() > 0.8 {
            decals.write(SpawnDecal {
                kind: DecalKind::WallSplat,
                at: ue_pos(hit.point),
                dir: ue_dir(hit.dir),
                trace: true,
            });
        }
        if z.last_pose.is_empty() {
            continue;
        }
        let mut next = z.id * 100 + z.next_piece;
        let first = next;
        let (point, attacker) = (ue_pos(hit.point), ue_pos(hit.attacker));
        if hit.decapitated {
            // HideBone(HeadBone) / SpecialHideHead: the neck stump.
            if !z.stumps.iter().any(|s| s.0 == StumpKind::Neck)
                && let Some(handles) = gore::new_stump(gore, StumpKind::Neck, commands, meshes, entity)
            {
                z.stumps.push((StumpKind::Neck, "neck", c.attach_scale[0], handles));
            }
            let Some(head) = c.model.tag_frame(&z.last_pose, "head") else {
                continue;
            };
            let (at, head_rot) = world_frame(c, t, head);
            // Neck spurt (NeckSpurtEmitterClass, or NeckSpurtNoGibEmitterClass
            // for the knife's SpecialHideHead) and DecapFX's BrainSplash.
            let jet = if hit.melee { "KFMod.DismembermentJetDecapitate" } else { "KFMod.DismembermentJetHead" };
            attach_effect(commands, meshes, library, c, z, t, jet, "neck", rng.0);
            if let Some(library) = library {
                particles::spawn_effect(commands, library, meshes, "ROEffects.BrainSplash", at, Mat3::IDENTITY, rng.0 ^ 0x9e37);
            }
            let (mut chunks, mut flying_head) = (0, false);
            if hit.melee {
                // DecapFX(.., bSpawnDetachedHead): the head flies off.
                if let Some(piece) = &c.severed_pieces[2] {
                    let rot_dir = gore::hit_normal_rotator(point, attacker, &mut rng);
                    gore::spawn_severed(commands, &mut gore::Effects { library, meshes: &mut *meshes }, piece, at, rot_dir, 0.06, head_rot, velocity, true, &mut next, &mut rng);
                    flying_head = true;
                }
            } else {
                // Brain chunks along the zed's own rotation.
                let rot = Vec3::new(0.0, z.yaw, 0.0);
                chunks = gore::spawn_giblets(commands, &mut gore::Effects { library, meshes: &mut *meshes }, gore, 3, at, rot, 0.06, 250.0, velocity, size, &mut next, &mut rng);
            }
            runlog::kv(
                "decap_fx",
                &format!(
                    "id={} melee={} stump={} brain_chunks={chunks} flying_head={flying_head} head_unreal=({:.0}, {:.0}, {:.0}) dead={}",
                    z.id,
                    hit.melee,
                    z.stumps.iter().any(|s| s.0 == StumpKind::Neck),
                    at.x,
                    at.y,
                    at.z,
                    hit.health_after <= 0.0
                ),
            );
        } else if hit.health_after <= 0.0 {
            let Some((bone, dist, name)) = hit_bone(c, &z.last_pose, t, hit.point, hit.dir) else {
                continue;
            };
            let mapped = bone_on_death(&name);
            runlog::kv(
                "gore_hit_bone",
                &format!("id={} bone={} tag={name} as={mapped} distance_to_shot_line_unreal={dist:.1}", z.id, c.model.mesh.bones[bone].name),
            );
            let Some(&(limb, stump_tag, stump_kind, piece, giblets)) = LIMBS.iter().find(|l| l.0 == mapped) else {
                continue;
            };
            if z.severed.contains(&limb) || (limb == "lfarm" && c.left_arm_gibbed) {
                continue;
            }
            // DismemberProbability, GibModifier 1 for our weapons.
            let chance = if always_sever { 1.0 } else { (hit.health_after - hit.damage).abs() / 130.0 };
            let roll = rng.frand();
            runlog::kv(
                "limb_roll",
                &format!(
                    "id={} limb={limb} health_after={:.1} damage={:.1} chance={chance:.2} roll={roll:.2} severed={}",
                    z.id,
                    hit.health_after,
                    hit.damage,
                    roll < chance
                ),
            );
            if roll >= chance {
                continue;
            }
            let Some(frame) = c.model.tag_frame(&z.last_pose, limb) else {
                continue;
            };
            let (at, bone_rot) = world_frame(c, t, frame);
            let rot_dir = gore::hit_normal_rotator(point, attacker, &mut rng);
            let piece_model = c.severed_pieces[piece].as_ref();
            if let Some(m) = piece_model {
                gore::spawn_severed(commands, &mut gore::Effects { library, meshes: &mut *meshes }, m, at, rot_dir, 0.25, bone_rot, velocity, false, &mut next, &mut rng);
            }
            let chunks = gore::spawn_giblets(commands, &mut gore::Effects { library, meshes: &mut *meshes }, gore, giblets, at, rot_dir, 0.25, 250.0, velocity, size, &mut next, &mut rng);
            if let Some(b) = c.model.tag_bone(limb) {
                z.hidden_bones.push(b);
            }
            let scale = c.attach_scale[if stump_kind == StumpKind::Arm { 1 } else { 2 }];
            // HideBone: LimbSpurtEmitterClass on the stump's tag.
            attach_effect(commands, meshes, library, c, z, t, "KFMod.DismembermentJetLimb", stump_tag, rng.0);
            let stump = gore::new_stump(gore, stump_kind, commands, meshes, entity);
            let has_stump = stump.is_some();
            if let Some(handles) = stump {
                z.stumps.push((stump_kind, stump_tag, scale, handles));
            }
            z.severed.push(limb);
            runlog::kv(
                "limb_severed",
                &format!(
                    "id={} limb={limb} hidden_bone={} piece={} brain_chunks={chunks} stump={has_stump} stump_tag={stump_tag} stump_scale={scale} at_unreal=({:.0}, {:.0}, {:.0}) piece_ids={first}..{}",
                    z.id,
                    c.model.tag_bone(limb).map_or("none", |b| c.model.mesh.bones[b].name.as_str()),
                    piece_model.map_or("none", |m| m.name.as_str()),
                    at.x,
                    at.y,
                    at.z,
                    next
                ),
            );
        }
        z.next_piece = next - z.id * 100;
    }
}

pub(super) fn anchor_frame(c: &ZedClass, z: &Zed, t: &Transform, anchor: &EffectAnchor) -> Option<(Vec3, Mat3)> {
    match anchor {
        EffectAnchor::Tag(tag) => c.model.tag_frame(&z.last_pose, tag).map(|f| world_axes(c, t, f)),
        EffectAnchor::Bone { bone, offset, rotation } => {
            let (pos, axes) = world_axes(c, t, c.model.bone_frame(&z.last_pose, *bone)?);
            // The bone frame from world_axes is in actor scale; the offset is
            // in the bone's axes (Unreal units, DrawScale applied).
            Some((pos + axes * (*offset * c.draw_scale), axes * *rotation))
        }
    }
}

/// AnimNotify_Effect: starts the notify's effect at its bone; attached
/// effects follow the bone from then on.
#[allow(clippy::too_many_arguments)]
pub(super) fn notify_effect(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    library: Option<&EffectLibrary>,
    c: &ZedClass,
    z: &mut Zed,
    t: &Transform,
    effect: &ue_assets::skeletal::NotifyEffect,
    seed: u32,
) {
    let Some(library) = library else {
        return;
    };
    let Some(bone) = c.model.find_bone(&effect.bone) else {
        runlog::kv("notify_effect_error", &format!("id={} class={} bone={} error=\"no bone\"", z.id, effect.class, effect.bone));
        return;
    };
    let [pitch, yaw, roll] = effect.rotation;
    let anchor = EffectAnchor::Bone {
        bone,
        offset: Vec3::from_array(effect.offset),
        rotation: coords::ue_rotation_matrix(Rotator { pitch, yaw, roll }),
    };
    let Some((pos, axes)) = anchor_frame(c, z, t, &anchor) else {
        return;
    };
    let spawned = particles::spawn_effect(commands, library, meshes, &effect.class, pos, axes, seed);
    runlog::kv(
        "notify_effect",
        &format!(
            "id={} class={} bone={} at_unreal=({:.0}, {:.0}, {:.0}) spawned={}",
            z.id,
            effect.class,
            effect.bone,
            pos.x,
            pos.y,
            pos.z,
            spawned.is_some()
        ),
    );
    if let Some(e) = spawned
        && effect.attach
    {
        z.effects.push((e, anchor));
    }
}

/// Moves a zed's attached effects to their anchors' current frames.
pub(super) fn follow_tags(c: &ZedClass, z: &Zed, t: &Transform, effects: &mut Query<&mut ParticleEffect>) {
    for (e, anchor) in &z.effects {
        if let (Ok(mut fx), Some(frame)) = (effects.get_mut(*e), anchor_frame(c, z, t, anchor)) {
            fx.frame = frame;
        }
    }
}

/// KFMonster.Timer while burning: every second, TakeFireDamage with
/// LastBurnDamage plus 3 or 4 and FireDamageClass (back through TakeDamage,
/// so the burn grows), and BurnDown goes down by 1; the fire goes out at 0. The flames
/// (StartBurnFX / StopBurnFX) follow the zed while it burns.
/// KF spawns the flames on the skeleton (UseSkeletalLocationAs); here they
/// come from the zed's centre: an approximation.
pub(super) fn burn_zeds(
    mut commands: Commands,
    time: Res<Time>,
    library: Option<Res<crate::render::particles::EffectLibrary>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut effects: Query<&mut crate::render::particles::ParticleEffect>,
    mut zeds: Query<(&mut Zed, &Transform)>,
    (mut kills, vet): (ResMut<crate::game::combat::KillCount>, Res<crate::game::perks::Veterancy>),
) {
    let dt = time.delta_secs();
    for (mut z, t) in &mut zeds {
        let b = t.translation;
        let ue = Vec3::new(-b.z, b.x, b.y) / SCALE;
        let alive = z.health > 0.0;
        // Zap (KFMonster.Tick). SetZappedBehavior uncloaks (the Stalker
        // and the Patriarch do not cloak while zapped); when it wears off
        // during a run, rage or charge, that state keeps the normal speed.
        if alive {
            if z.zapped() && z.cloaked {
                z.cloaked = false;
                z.cloak_dirty = true;
            }
            if z.zapped() {
                z.clear_glow();
            }
            if z.zap_tick(dt) {
                z.run_speed_lost = z.running || z.raging || z.fp_rage.is_some() || z.boss.is_some_and(|b| b.charge.is_some() || b.escaping());
                runlog::kv("zed_unzapped", &format!("zed={} next_threshold={:.2} run_speed_lost={}", z.id, z.zap_threshold, z.run_speed_lost));
            }
        }
        if z.burn_down > 0 && alive {
            z.burn_timer -= dt;
            if z.burn_timer <= 0.0 {
                z.burn_timer += 1.0;
                let damage = (z.last_burn_damage + (z.random() % 2) as f32 + 3.0).floor();
                let fire = z.fire_class;
                // KFMonster.Timer: TakeFireDamage(.., BurnInstigator) with
                // FireDamageClass, through TakeDamage (the perk's AddDamage).
                let dam = Some(crate::game::perks::known_dam_type(match fire {
                    crate::game::combat::FireType::Trenchgun => "DamTypeTrenchgun",
                    crate::game::combat::FireType::Mac10 => "DamTypeMAC10MPInc",
                    _ => "DamTypeFlamethrower",
                }));
                let source = crate::game::combat::HitSource { point: b, attacker: b, melee: false, explosive: None, fire: Some(fire), dam, vet: vet.vet };
                crate::game::combat::damage_zed(&mut z, damage, false, 1.0, "fire", 0.0, source, &mut kills);
                z.burn_down -= 1;
                runlog::kv(
                    "zed_burn_tick",
                    &format!("zed={} damage={damage} fire={fire:?} left={} health={:.1}", z.id, z.burn_down, z.health),
                );
                if z.burn_down == 0 {
                    runlog::kv("zed_burn_out", &format!("zed={}", z.id));
                }
            }
        }
        let burning = z.burn_down > 0 && z.health > 0.0;
        match (burning, z.burn_fx) {
            (true, None) => {
                if let Some(lib) = library.as_deref() {
                    let id = z.id as u32;
                    let opts = crate::render::particles::SpawnOptions { persistent: true, ..default() };
                    z.burn_fx = crate::render::particles::spawn_effect_with(&mut commands, lib, &mut meshes, BURN_EFFECT, ue, Mat3::IDENTITY, id, opts);
                    runlog::kv("zed_burn_fx", &format!("zed={} started={}", z.id, z.burn_fx.is_some()));
                }
            }
            (true, Some(e)) => {
                if let Ok(mut fx) = effects.get_mut(e) {
                    fx.frame.0 = ue;
                }
            }
            (false, Some(e)) => {
                if let Ok(mut fx) = effects.get_mut(e) {
                    fx.kill();
                }
                z.burn_fx = None;
            }
            (false, None) => {}
        }
    }
}

/// The mesh-to-world mapping for ragdoll bodies, for a zed at `actor`.
pub(super) fn mesh_frame(c: &ZedClass, actor: &Transform) -> MeshFrame {
    let r = c.model.mesh.rot_origin;
    let rot = coords::ue_rotation_matrix(Rotator {
        pitch: r[0],
        yaw: r[1],
        roll: r[2],
    });
    MeshFrame::new(
        rot,
        c.model.mesh.scale[0],
        Vec3::from_array(c.model.mesh.origin),
        c.draw_scale,
        c.pre_pivot,
        actor,
    )
}

/// Mesh space -> actor-local Unreal space for a zed class.
pub(super) fn mesh_to_actor(c: &ZedClass) -> impl Fn(Vec3) -> Vec3 + '_ {
    c.model.mesh_to_actor(c.pre_pivot, c.draw_scale)
}

/// KFMonster.PlayDyingAnimation's start motion: 0.6 x the zed's horizontal
/// velocity (full vertical) plus RagDeathVel along the shot; spin =
/// RagInvInertia x (hit offset, sideways part scaled by RagSpinScale and
/// capped at RagMaxSpinAmount) cross that push, capped at Karma's
/// KMaxAngularSpeed. RagDeathUpKick is 0 for KF zeds.
pub(super) fn death_launch(z: &Zed) -> Launch {
    let Some((hit, dir)) = z.last_hit else {
        return Launch {
            velocity: Vec3::new(0.6 * z.velocity.x, z.velocity.y, 0.6 * z.velocity.z),
            angular_velocity: Vec3::ZERO,
        };
    };
    let push = dir.normalize_or_zero() * RAG_DEATH_VEL; // Unreal units/s, Bevy axes
    let mut velocity = Vec3::new(0.6 * z.velocity.x, z.velocity.y, 0.6 * z.velocity.z) + push * SCALE;
    let max_speed = ragdoll::MAX_SPEED * SCALE;
    if velocity.length() > max_speed {
        velocity = velocity.normalize() * max_speed;
    }
    // Hit offset in Unreal axes (X, Y horizontal), scaled and capped as KF does.
    let r = (hit - z.centre) / SCALE;
    let mut rel = Vec3::new(-r.z, r.x, r.y);
    rel.x = (rel.x * RAG_SPIN_SCALE).clamp(-RAG_MAX_SPIN_AMOUNT, RAG_MAX_SPIN_AMOUNT);
    rel.y = (rel.y * RAG_SPIN_SCALE).clamp(-RAG_MAX_SPIN_AMOUNT, RAG_MAX_SPIN_AMOUNT);
    // Back to Bevy axes (Unreal units) for a right-handed cross product.
    let rel_bevy = Vec3::new(rel.y, rel.z, -rel.x);
    // Units of RagInvInertia are not known; the result is capped anyway.
    let mut angular_velocity = RAG_INV_INERTIA * rel_bevy.cross(push) * SCALE * SCALE;
    if angular_velocity.length() > ragdoll::MAX_ANGULAR_SPEED {
        angular_velocity = angular_velocity.normalize() * ragdoll::MAX_ANGULAR_SPEED;
    }
    Launch {
        velocity,
        angular_velocity,
    }
}

/// Swaps a Stalker's materials when her cloak changes.
pub(super) fn apply_cloaks(mut commands: Commands, classes: Option<Res<ZedClasses>>, mut zeds: Query<(&mut Zed, &ZedParts)>) {
    let Some(classes) = classes else {
        return;
    };
    for (mut z, parts) in &mut zeds {
        if !z.cloak_dirty {
            continue;
        }
        z.cloak_dirty = false;
        let c = &classes.0[z.class];
        for (i, &e) in parts.0.iter().enumerate() {
            let material = match (&c.cloak_material, z.cloaked, &c.fp_red_device) {
                // Spotted by a Commando: Skins[0] and Skins[1] =
                // KFX.StalkerGlow (spotted.rs).
                _ if z.glow && let Some(glow) = &c.spotted_material => glow.clone(),
                (Some(cloak), true, _) => cloak.clone(),
                (_, true, _) if !c.cloak_parts.is_empty() => c.cloak_parts[i.min(c.cloak_parts.len() - 1)].clone(),
                // DeviceGoRed while raging (Skins[1]).
                (_, _, Some(red)) if i == 1 && (z.fp_rage.is_some() || z.state == ZedState::Enraging) => red.clone(),
                _ => c.model.parts[i].material.clone(),
            };
            commands.entity(e).insert(MeshMaterial3d(material));
        }
    }
}
