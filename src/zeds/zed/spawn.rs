//! Spawning zeds: test spawns in front of the camera, and the wave game's SpawnZedAt.

use super::*;

/// Spawns a zed of class `class` `distance` units in front of the camera, on the floor.
#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_in_front(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    classes: &ZedClasses,
    spatial: &SpatialQuery,
    cam_t: &Transform,
    cam: &FlyCamera,
    class: usize,
    id: usize,
    distance: f32,
    lift: f32,
    in_line: bool,
) {
    let c = &classes.0[class];
    let forward = Vec3::new(-cam.yaw.sin(), 0.0, -cam.yaw.cos());
    let right = Vec3::new(cam.yaw.cos(), 0.0, -cam.yaw.sin());
    // Side by side, 60 units apart (0, +60, -60, +120, ...), so zeds spawned
    // in a row do not start inside each other.
    let side = if in_line { 0.0 } else { (id.div_ceil(2) as f32) * if id % 2 == 1 { 60.0 } else { -60.0 } };
    let probe = cam_t.translation + (forward * distance + right * side) * SCALE;
    let Some(hit) = spatial.cast_ray(probe, Dir3::NEG_Y, 20.0, true, &crate::world::collision::world_filter()) else {
        runlog::kv("zed_spawn_failed", "reason=no_floor_below");
        return;
    };
    let centre = probe - Vec3::Y * hit.distance + Vec3::Y * (c.collision_height + 1.0 + lift) * SCALE;
    spawn_zed(commands, meshes, classes, class, id, centre, yaw_of(-forward), false);
}

/// Spawns a zed of class `class` with its cylinder centre at `centre`
/// (Bevy space), facing `yaw` (Unreal units). `puppet`: a network
/// client's copy of a host zed (net.rs).
#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_zed(commands: &mut Commands, meshes: &mut Assets<Mesh>, classes: &ZedClasses, class: usize, id: usize, centre: Vec3, yaw: f32, puppet: bool) {
    let c = &classes.0[class];
    let handles = c.model.new_instance(meshes);
    let parent = commands
        .spawn((
            Transform {
                translation: centre,
                rotation: coords::rotation(Rotator { pitch: 0, yaw: yaw as i32, roll: 0 }),
                ..default()
            },
            Visibility::Visible,
            Zed {
                id,
                class,
                centre,
                radius: c.collision_radius,
                half_height: c.collision_height,
                health: c.health,
                health_max: c.health_max,
                head_health: c.head_health,
                decapitated: false,
                bleed_out: None,
                bleed_out_duration: c.bleed_out_duration,
                overlay: None,
                attack: None,
                since_decap: None,
                pending_reaction: None,
                since_pain_anim: f32::MAX,
                stunned: 0.0,
                default_health: c.health,
                melee_range: c.melee_range,
                melee_damage: c.melee_damage,
                headless_claws: !c.headless_melee.is_empty(),
                can_run: c.run_anim.is_some(),
                running: false,
                charge_check: 0.0,
                run_attack_timeout: 0.0,
                rng: 0x9E37_79B9 ^ (id as u32).wrapping_mul(2_654_435_761),
                head_radius: c.head_radius,
                head: None,
                ext: None,
                dead_for: 0.0,
                last_hit: None,
                velocity: Vec3::ZERO,
                gore_hits: Vec::new(),
                stumps: Vec::new(),
                hidden_bones: Vec::new(),
                severed: Vec::new(),
                last_pose: Vec::new(),
                next_piece: 0,
                effects: Vec::new(),
                since_hit: f32::MAX,
                router: Default::default(),
                air_velocity: Vec3::ZERO,
                mass: c.mass,
                jump_cooldown: 0.0,
                door_bash: None,
                door_checked: None,
                on_pad: None,
                last_seen: f32::MIN,
                last_render: f32::MIN,
                last_view_check: f32::MIN,
                hidden: false,
                pouncing: false,
                since_pounce: f32::MAX,
                can_flip: !c.no_flip,
                sawing: false,
                saw_charging: false,
                raging: false,
                flinch_min_damage: c.flinch_min_damage,
                uninterruptible: matches!(c.kind, ZedKind::Bloat | ZedKind::Husk),
                quick_headless_death: c.kind == ZedKind::Siren,
                ranged_wait: 0.0,
                bled_out: false,
                burst_done: false,
                pending_fx: Vec::new(),
                fp_two_sec_damage: 0.0,
                fp_since_damaged: f32::MAX,
                fp_start_rage: false,
                fp_rage: None,
                fp_frustration: 0.0,
                fp_frustration_limit: 10.0 + 5.0 * ((id as u32).wrapping_mul(2_654_435_761) % 1000) as f32 / 1000.0,
                fp_frustrated: false,
                fp_rage_threshold: c.fp_rage_threshold,
                small_arms_scale: c.small_arms_scale,
                motion_threat: c.motion_threat,
                burn_down: 0,
                last_burn_damage: 0.0,
                heat: 0,
                fire_class: crate::game::combat::FireType::Flamethrower,
                burn_timer: 0.0,
                burn_vet: crate::game::perks::Vet::default(),
                burn_fx: None,
                burned_scale: if c.kind == ZedKind::Bloat { 1.5 } else { 1.0 },
                fire_resist: c.fire_resist,
                total_zap: 0.0,
                remaining_zap: 0.0,
                since_zap: 1e6,
                zap_threshold: c.zap.threshold,
                zap: c.zap,
                run_speed_lost: false,
                keeps_head: c.boss.is_some(),
                no_hit_reactions: c.boss.is_some(),
                boss: c.boss.is_some().then(|| crate::zeds::boss::BossState::new(c.health)),
                braindead: false,
                scoring_value: c.scoring_value,
                killed_by_player: false,
                // A puppet's kill is paid by the host (KillCredit).
                kill_paid: puppet,
                headshot_kill: false,
                zed_time_rolled: false,
                mg_flash: None,
                mg_flash_shots: 0,
                // ZombieStalker.PostBeginPlay: CloakStalker.
                // The Stalker spawns cloaked; the Patriarch only cloaks after
                // his entrance (InitialSneak), see boss.rs.
                cloaked: c.cloak_material.is_some() && c.boss.is_none(),
                since_uncloak: f32::MAX,
                cloak_check: 0.0,
                cloak_dirty: c.cloak_material.is_some() && c.boss.is_none(),
                spotted: false,
                glow: false,
                yaw,
                state: ZedState::Idle,
                vertical_speed: 0.0,
                sequence: None,
                frame: 0.0,
                looping: true,
                sounds_heard: (None, -1.0),
                overlay_sounds_heard: None,
                sound_events: Vec::new(),
                moan_at: -1.0,
                since_pain_sound: f32::MAX,
                hit_by_fire: false,
                last_challenge: f32::MIN,
                challenge_check: 0.0,
                ambient_on: None,
                pain_on_fire: c.sounds.pain_on_fire,
                meshes: handles.clone(),
                net: ZedNetSide { puppet, kind: net::kind_code(c.kind), ..default() },
            },
        ))
        .id();
    let parts: Vec<Entity> = c
        .model
        .parts
        .iter()
        .zip(handles)
        .map(|(part, handle)| {
            commands
                .spawn((
                    Mesh3d(handle),
                    MeshMaterial3d(part.material.clone()),
                    Transform::IDENTITY,
                    ChildOf(parent),
                    // Swapped materials (cloak, spotted glow, the
                    // Fleshpound's red device) are drawn as before.
                    crate::render::actor_light::LitPart { owner: parent, animated: true, own: Some(part.material.id()) },
                ))
                .id()
        })
        .collect();
    // Lit by the map (render/actor_light.rs): KFMonster MaxLights 5,
    // AmbientGlow 0.
    commands.entity(parent).insert((ZedParts(parts), crate::render::actor_light::ActorLight::new(format!("zed_{id}_{}", c.name.rsplit('.').next().unwrap_or("")), Vec3::ZERO, 5, 0)));
    let u = centre / SCALE;
    runlog::kv(
        "zed_spawned",
        &format!("id={id} class={} centre_unreal=({:.0}, {:.0}, {:.0}) yaw={yaw:.0} puppet={puppet}", c.name, -u.z, u.x, u.y),
    );
}

#[allow(clippy::too_many_arguments)] // Bevy system parameters
pub(super) fn spawn_zeds(
    mut commands: Commands,
    frames: Res<bevy::diagnostic::FrameCount>,
    settings: Res<ZedSettings>,
    keys: Res<ButtonInput<KeyCode>>,
    classes: Option<Res<ZedClasses>>,
    spatial: SpatialQuery,
    cams: Query<(&Transform, &FlyCamera)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut active: ResMut<ZedsActive>,
    mut z_spawn: ResMut<ZSpawn>,
    script: Res<crate::weapons::weapon::ScriptedInput>,
    mut wave_spawns: MessageReader<crate::game::waves::SpawnZedAt>,
    mut next_id: Local<usize>,
    (mut puppets, net_mode): (MessageReader<SpawnPuppet>, Option<Res<crate::net::NetMode>>),
) {
    // Test action "toggle_zeds": the same as X.
    let toggle = script.0.iter().any(|(f, a)| *f == frames.0 && a == "toggle_zeds");
    if keys.just_pressed(KeyCode::KeyX) || toggle {
        active.0 = !active.0;
        runlog::kv("zeds_active", &format!("active={}", active.0));
    }
    let Some(classes) = classes else {
        return;
    };
    if classes.0.is_empty() || frames.0 < 6 {
        return;
    }
    // A network client never makes zeds of its own: every zed is a puppet
    // of one the host runs (net/zeds.rs).
    if net_mode.is_some_and(|m| matches!(*m, crate::net::NetMode::Client { .. })) {
        for p in puppets.read() {
            let Some(class) = classes.0.iter().position(|c| net::kind_code(c.kind) == p.kind) else {
                runlog::kv("zed_spawn_failed", &format!("reason=class_not_loaded kind_code={} puppet=true", p.kind));
                continue;
            };
            spawn_zed(&mut commands, &mut meshes, &classes, class, p.id, p.centre, p.yaw, true);
        }
        wave_spawns.clear();
        return;
    }
    // Zeds from the wave loop (game/waves.rs): a class standing on a floor point.
    for w in wave_spawns.read() {
        let Some(class) = classes.0.iter().position(|c| c.name.eq_ignore_ascii_case(&w.class)) else {
            runlog::kv("zed_spawn_failed", &format!("reason=class_not_loaded class={}", w.class));
            continue;
        };
        let centre = coords::pos(w.centre.to_array());
        spawn_zed(&mut commands, &mut meshes, &classes, class, *next_id, centre, w.yaw, false);
        *next_id += 1;
    }
    let start = frames.0 == 6;
    let scripted = |action: &str| script.0.iter().any(|(f, a)| *f == frames.0 && a == action);
    if keys.just_pressed(KeyCode::KeyN) || scripted("cycle_zed") {
        z_spawn.class = (z_spawn.class + 1) % classes.0.len();
        z_spawn.label = format!("{:?}", classes.0[z_spawn.class].kind);
        runlog::kv("z_spawn_selected", &format!("kind={}", z_spawn.label));
    }
    // What to spawn this frame: (kind, distance in front, lift).
    // (kind, distance ahead, lift, in line: no sideways offset)
    let mut wanted: Vec<(ZedKind, f32, f32, bool)> = Vec::new();
    // Z spawns the type picked with N.
    if keys.just_pressed(KeyCode::KeyZ) {
        wanted.push((classes.0[z_spawn.class.min(classes.0.len() - 1)].kind, 300.0, 0.0, false));
    }
    // H (G is KF's grenade key).
    if keys.just_pressed(KeyCode::KeyH) {
        wanted.push((ZedKind::Gorefast, 300.0, 0.0, false));
    }
    if start {
        if settings.spawn_at_start {
            wanted.push((ZedKind::Clot, 300.0, 0.0, false));
        }
        if settings.gorefast_at_start {
            wanted.push((ZedKind::Gorefast, 300.0, 0.0, false));
        }
        if let Some(name) = &settings.spawn_kind {
            match kind_named(name) {
                Some(k) => wanted.push((k, 300.0, 0.0, false)),
                None => runlog::kv("zed_spawn_failed", &format!("reason=unknown_kind name={name}")),
            }
        }
    }
    // Test actions: "zed" (Clot), "zed_drop" (a Clot 200 units up),
    // "gorefast", "gorefast_far" (900 away), "zed_line", "spawn_<kind>".
    for (f, a) in &script.0 {
        if *f != frames.0 {
            continue;
        }
        match a.as_str() {
            "zed" => wanted.push((ZedKind::Clot, 300.0, 0.0, false)),
            "zed_drop" => wanted.push((ZedKind::Clot, 300.0, 200.0, false)),
            "gorefast" => wanted.push((ZedKind::Gorefast, 300.0, 0.0, false)),
            "gorefast_far" => wanted.push((ZedKind::Gorefast, 900.0, 0.0, false)),
            // Three Clots straight ahead, one behind the other (penetration tests).
            "zed_line" => {
                for d in [250.0, 400.0, 550.0] {
                    wanted.push((ZedKind::Clot, d, 0.0, true));
                }
            }
            // The same farther away (explosives arm after 300-500 units).
            "zed_line_far" => {
                for d in [700.0, 800.0, 900.0] {
                    wanted.push((ZedKind::Clot, d, 0.0, true));
                }
            }
            other => {
                if let Some(k) = other.strip_prefix("spawn_").and_then(kind_named) {
                    wanted.push((k, 300.0, 0.0, false));
                }
            }
        }
    }
    let Ok((t, cam)) = cams.single() else {
        return;
    };
    for (kind, distance, lift, in_line) in wanted {
        let Some(class) = classes.0.iter().position(|c| c.kind == kind) else {
            runlog::kv("zed_spawn_failed", &format!("reason=class_not_loaded kind={kind:?}"));
            continue;
        };
        match settings.spawn_at {
            // --zed-at: the start zed at a given place, facing you.
            Some(at) if start => {
                let centre = coords::pos(at);
                spawn_zed(&mut commands, &mut meshes, &classes, class, *next_id, centre, yaw_of(t.translation - centre), false);
            }
            _ => spawn_in_front(&mut commands, &mut meshes, &classes, &spatial, t, cam, class, *next_id, distance, lift, in_line),
        }
        *next_id += 1;
    }
}
