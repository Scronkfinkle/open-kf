//! Loading the zed classes: meshes, animations, materials, ragdolls and the class defaults the AI and combat use.

use super::*;

pub(super) fn load_zed_classes(
    mut commands: Commands,
    request: Res<MapRequest>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let started = std::time::Instant::now();
    let set = PackageSet::new(&request.install_root);
    let defaults = ClassDefaults::new(&set);
    // KF's zed ragdolls all live in one Karma file.
    let ka_path = request.install_root.join("KarmaData").join("KF_Characters_Trip.ka");
    let ragdolls = match std::fs::read(&ka_path)
        .map_err(|e| e.to_string())
        .and_then(|b| ue_assets::karma::parse_ka(&String::from_utf8_lossy(&b)))
    {
        Ok(r) => r,
        Err(e) => {
            runlog::kv("ragdoll_file_error", &format!("path={} error=\"{e}\"", ka_path.display()));
            Default::default()
        }
    };
    let mut classes = Vec::new();
    for (kind, class_path) in ZED_CLASSES {
        match load_class(&set, &defaults, kind, class_path, &ragdolls, &mut meshes, &mut images, &mut materials) {
            Ok(c) => {
                runlog::kv(
                    "zed_class_loaded",
                    &format!(
                        "class={} bones={} triangles={} draw_scale={} pre_pivot={:?} collision={}x{} ground_speed={} turn_rate={} melee_range={} idle={:?} walk={:?} melee={:?} death={:?} death_hold_frame={} health={} head_health={} head_radius={} head_bone={:?} melee_damage={} melee_dam_type={:?} ext_collision={:?}",
                        c.name,
                        c.model.mesh.bones.len(),
                        c.model.mesh.triangles.len(),
                        c.draw_scale,
                        c.pre_pivot.to_array(),
                        c.collision_radius,
                        c.collision_height,
                        c.ground_speed,
                        c.turn_rate,
                        c.melee_range,
                        c.idle,
                        c.walk,
                        c.melee,
                        c.death,
                        c.death_hold_frame,
                        c.health,
                        c.head_health,
                        c.head_radius,
                        c.head_bone.map(|b| c.model.mesh.bones[b].name.clone()),
                        c.melee_damage,
                        c.melee_dam_type,
                        c.ext_collision
                    ),
                );
                runlog::kv(
                    "zed_class_fire",
                    &format!("class={} burning_walk={:?} fire_resist={}", c.name, c.burning_walk, c.fire_resist),
                );
                runlog::kv(
                    "zed_class_sequences",
                    &format!(
                        "class={} sequences={:?}",
                        c.name,
                        c.model.anim.as_ref().map(|a| a.sequences.iter().map(|s| s.name.clone()).collect::<Vec<_>>())
                    ),
                );
                classes.push(c);
            }
            Err(e) => runlog::kv("zed_class_error", &format!("class={class_path} error=\"{e}\"")),
        }
    }
    runlog::kv(
        "zed_classes_ready",
        &format!("count={} seconds={:.2}", classes.len(), started.elapsed().as_secs_f64()),
    );
    commands.insert_resource(ZedClasses(classes));
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_class(
    set: &PackageSet,
    defaults: &ClassDefaults,
    kind: ZedKind,
    class_path: &str,
    ragdolls: &std::collections::HashMap<String, ue_assets::karma::Ragdoll>,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> Result<ZedClass, String> {
    let (pkg_name, class_name) = class_path.split_once('.').ok_or("bad class path")?;
    let lp = set.load(pkg_name).ok_or("package not found")?;
    let export = (0..lp.pkg.exports.len())
        .find(|&i| {
            lp.pkg.export_class_name(i) == "Class"
                && lp.pkg.object_name(ObjectRef::Export(i)).eq_ignore_ascii_case(class_name)
        })
        .ok_or("class not found")?;
    let class = ObjectHandle { package: lp, export };
    let get = |p: &str| defaults.get(&class, p);
    // Some values (Health, MeleeDamage, ...) are stored as integers.
    let float = |p: &str, d: f32| match get(p) {
        Some((Value::Float(f), _)) => f,
        Some((Value::Int(i), _)) => i as f32,
        _ => d,
    };
    let name_of = |p: &str| match get(p) {
        Some((Value::Name(n), np)) => Some(np.pkg.name(n).to_string()),
        _ => None,
    };

    let (Value::Object(mesh_ref), mesh_pkg) = get("Mesh").ok_or("no Mesh default")? else {
        return Err("Mesh is not an object".into());
    };
    let mesh_h = set.resolve(&mesh_pkg, mesh_ref).ok_or("mesh not found")?;
    let skins = match get("Skins") {
        Some((Value::Array { count, raw }, p)) => {
            let mut r = ue_assets::reader::Reader::new(&raw);
            Skins {
                refs: (0..count)
                    .filter_map(|_| r.compact_index().ok().map(ObjectRef::from_raw))
                    .collect(),
                package: Some(p),
                named: Vec::new(),
            }
        }
        _ => Skins {
            refs: Vec::new(),
            package: None,
            named: Vec::new(),
        },
    };
    let model = SkinnedModel::load(set, &mesh_h, &skins, true, meshes, images, materials)?;
    let pre_pivot = match get("PrePivot") {
        Some((Value::Vector(v), _)) => Vec3::from_array(v),
        _ => Vec3::ZERO,
    };
    let turn_rate = match get("RotationRate") {
        Some((Value::Rotator(r), _)) => r.yaw as f32,
        _ => 20000.0,
    };
    // MeleeAnims is a fixed array of names: MeleeAnims, MeleeAnims[1], ...
    let melee = defaults
        .get_array_names(&class, "MeleeAnims")
        .iter()
        .filter_map(|n| model.sequence(n))
        .collect();
    let head_scale = float("HeadScale", 1.0);
    let head_bone = name_of("HeadBone").and_then(|n| model.find_bone(&n));
    let ext_collision = matches!(get("bUseExtendedCollision"), Some((Value::Bool(true), _))).then(|| {
        let offset = match get("ColOffset") {
            Some((Value::Vector(v), _)) => Vec3::from_array(v),
            _ => Vec3::ZERO,
        };
        (offset, float("ColRadius", 0.0), float("ColHeight", 0.0))
    });
    let death = model.sequence("KnockDown");
    // The frame of KnockDown where the root bone (pelvis) is lowest.
    let death_hold_frame = death.map_or(0.0, |d| {
        let len = model.length(d);
        (0..(len as usize).max(1))
            .map(|f| (f as f32, model.pose_with_bones(Some(d), f as f32).1[0].1.z))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(0.0, |(f, _)| f)
    });
    // KFMonster: RagdollOverride = KFRagdollName (a Karma asset name).
    let ragdoll = match get("KFRagdollName") {
        Some((Value::Str(name), _)) => match ragdolls.get(&name) {
            Some(ka) => match RagdollDef::new(ka, &model) {
                Ok((def, report)) => {
                    runlog::kv("ragdoll_loaded", &format!("class={class_path} {report}"));
                    log_hinge_check(&def, &model, name_of("MovementAnims").and_then(|n| model.sequence(&n)));
                    Some(def)
                }
                Err(e) => {
                    runlog::kv("ragdoll_error", &format!("class={class_path} ragdoll={name} error=\"{e}\""));
                    None
                }
            },
            None => {
                runlog::kv("ragdoll_error", &format!("class={class_path} ragdoll={name} error=\"not in file\""));
                None
            }
        },
        _ => None,
    };
    let hit_reactions = ["KFHitFront", "KFHitBack", "KFHitLeft", "KFHitRight"].map(|p| name_of(p).and_then(|n| model.sequence(&n)));
    let hit_anims: Vec<usize> = defaults
        .get_array_names(&class, "HitAnims")
        .iter()
        .filter_map(|n| model.sequence(n))
        .collect();
    let knock_down = model.sequence("KnockDown");
    // Attacks played on the upper body while walking (DoAnimAction
    // overrides): the Clot's grapples, the Scrake's saw swings.
    let layered_attacks: Vec<usize> = match kind {
        ZedKind::Clot => ["ClotGrapple", "ClotGrappleTwo", "ClotGrappleThree"].iter().filter_map(|n| model.sequence(n)).collect(),
        ZedKind::Scrake => ["SawZombieAttack1", "SawZombieAttack2"].iter().filter_map(|n| model.sequence(n)).collect(),
        ZedKind::Fleshpound => ["PoundAttack1", "PoundAttack2", "PoundAttack3", "FPRageAttack"].iter().filter_map(|n| model.sequence(n)).collect(),
        // ZombieSiren.DoAnimAction: bites and the scream from SpineBone1.
        ZedKind::Siren => ["Siren_Bite", "Siren_Bite2"].iter().filter_map(|n| model.sequence(n)).collect(),
        _ => Vec::new(),
    };
    let headless_melee: Vec<usize> = if kind == ZedKind::Clot {
        ["Claw", "Claw", "Claw2"].iter().filter_map(|n| model.sequence(n)).collect()
    } else {
        Vec::new()
    };
    // Severed pieces (DetachedArmClass, DetachedLegClass, DetachedHeadClass).
    let severed_pieces = ["DetachedArmClass", "DetachedLegClass", "DetachedHeadClass"].map(|p| {
        let (Value::Object(r), pkg) = get(p)? else {
            return None;
        };
        let h = set.resolve(&pkg, r)?;
        let name = h.package.pkg.object_name(ObjectRef::Export(h.export)).to_string();
        match gore::load_piece(set, defaults, &h, &name, meshes, images, materials) {
            Ok(m) => {
                runlog::kv("gore_piece_loaded", &format!("class={class_path} slot={p} piece={name}"));
                Some(m)
            }
            Err(e) => {
                runlog::kv("gore_piece_error", &format!("class={class_path} slot={p} piece={name} error=\"{e}\""));
                None
            }
        }
    });
    // ZombieGoreFast: PostNetReceive swaps MovementAnims[0] for ZombieRun.
    let run_anim = if kind == ZedKind::Gorefast { model.sequence("ZombieRun") } else { None };
    let ranged_anim = match kind {
        ZedKind::Bloat => model.sequence("ZombieBarf"),
        ZedKind::Siren => model.sequence("Siren_Scream"),
        ZedKind::Husk => model.sequence("ShootBurns"),
        _ => None,
    };
    let run_attack_seconds = model.sequence("GoreAttack1").map_or(0.0, |s| model.length(s) / model.rate(s).max(1e-3));
    let first_name = |p: &str| defaults.get_array_names(&class, p).first().and_then(|n| model.sequence(n));
    let air_anim = first_name("AirAnims");
    let land_anim = first_name("LandAnims");
    let turn_left = name_of("TurnLeftAnim").and_then(|n| model.sequence(&n));
    let turn_right = name_of("TurnRightAnim").and_then(|n| model.sequence(&n));
    let fire_root_bone = name_of("FireRootBone").and_then(|n| model.find_bone(&n));
    let spine_bone = name_of("SpineBone1").and_then(|n| model.find_bone(&n));
    // The upper-body layer's root (FireRootBone; the Siren and the Patriarch
    // layer from SpineBone1).
    let fire_root_bone = if matches!(kind, ZedKind::Siren | ZedKind::Patriarch) { spine_bone.or(fire_root_bone) } else { fire_root_bone };
    let headless_walk = defaults
        .get_array_names(&class, "HeadlessWalkAnims")
        .first()
        .and_then(|n| model.sequence(n));
    let s = model.mesh.scale;
    if s[0] != s[1] || s[1] != s[2] || s[0] <= 0.0 {
        runlog::kv("ragdoll_warning", &format!("class={class_path} mesh_scale={s:?} (ragdolls assume a uniform positive scale)"));
    }
    let sounds = load_zed_sounds(defaults, &class, kind);
    Ok(ZedClass {
        sounds,
        kind,
        ragdoll,
        health_max: float("HealthMax", float("Health", 100.0)),
        bleed_out_duration: float("BleedOutDuration", 5.0),
        headless_walk,
        hit_reactions,
        hit_anims,
        knock_down,
        spine_bone,
        layered_attacks,
        headless_melee,
        air_anim,
        land_anim,
        turn_left,
        turn_right,
        fire_root_bone,
        grapple_duration: float("GrappleDuration", 0.0),
        run_anim,
        run_attack_seconds,
        severed_pieces,
        attach_scale: [
            float("SeveredHeadAttachScale", 1.0),
            float("SeveredArmAttachScale", 1.0),
            float("SeveredLegAttachScale", 1.0),
        ],
        left_arm_gibbed: matches!(get("bLeftArmGibbed"), Some((Value::Bool(true), _))),
        jump_z: float("JumpZ", 320.0),
        pounce_speed: if kind == ZedKind::Crawler { float("PounceSpeed", 0.0) } else { 0.0 },
        no_flip: matches!(kind, ZedKind::Crawler | ZedKind::Fleshpound | ZedKind::Bloat | ZedKind::Siren | ZedKind::Patriarch),
        flinch_root: if kind == ZedKind::Crawler { name_of("NeckBone").and_then(|n| model.find_bone(&n)) } else { None },
        saw_impale: if kind == ZedKind::Scrake { model.sequence("SawImpaleLoop") } else { None },
        charge_anim: if kind == ZedKind::Scrake { model.sequence("ChargeF") } else { None },
        flinch_min_damage: match kind {
            ZedKind::Scrake => 150.0,
            ZedKind::Fleshpound => 10.0,
            _ => 5.0,
        },
        fp_rage_anim: if kind == ZedKind::Fleshpound { model.sequence("PoundRage") } else { None },
        fp_charge_walk: if kind == ZedKind::Fleshpound { name_of("ChargingAnim").and_then(|n| model.sequence(&n)) } else { None },
        fp_rage_attack: if kind == ZedKind::Fleshpound { model.sequence("FPRageAttack") } else { None },
        fp_rage_threshold: float("RageDamageThreshold", 0.0),
        fp_red_device: if kind == ZedKind::Fleshpound { load_named_material(set, "KFCharacters", "FPRedBloomShader", images, materials) } else { None },
        small_arms_scale: if kind == ZedKind::Fleshpound { 0.5 } else { 1.0 },
        motion_threat: float("MotionDetectorThreat", 1.0),
        fire_resist: if kind == ZedKind::Husk { float("BurnDamageScale", 1.0) } else { 1.0 },
        zap: {
            let d = ZapValues::default();
            ZapValues {
                duration: float("ZapDuration", d.duration),
                speed_mod: float("ZappedSpeedMod", d.speed_mod),
                threshold: float("ZapThreshold", d.threshold),
                damage_mod: float("ZappedDamageMod", d.damage_mod),
                resistance: float("ZapResistanceScale", d.resistance),
            }
        },
        burning_walk: defaults.get_array_names(&class, "BurningWalkFAnims").first().and_then(|n| model.sequence(n)),
        ranged_anim,
        ranged_distance: match kind {
            ZedKind::Siren => float("ScreamRadius", 700.0),
            // ZombieHusk.RangedAttack: anywhere within 65535 (no distance fog).
            ZedKind::Husk => 65535.0,
            _ => BLOAT_BARF_DISTANCE,
        },
        ranged_interval: float("ProjectileFireInterval", 0.0),
        barrel_bone: if kind == ZedKind::Husk { model.find_bone("Barrel") } else { None },
        ranged_shots: ranged_anim.map_or(Vec::new(), |s| {
            model.notifies(s).iter().filter(|n| n.name.eq_ignore_ascii_case("SpawnTwoShots")).map(|n| n.time).collect()
        }),
        ranged_effects: ranged_anim.map_or(Vec::new(), |s| {
            model.notifies(s).iter().filter_map(|n| n.effect.clone().map(|e| (n.time, e))).collect()
        }),
        ranged_moving_chance: match kind {
            ZedKind::Siren => 1.0,
            ZedKind::Bloat => BLOAT_CHARGE_CHANCE,
            _ => 0.0,
        },
        scream_shake: (kind == ZedKind::Siren).then(|| crate::player::hit_cam::ScreamShake::load(defaults, &class)),
        scream: (kind == ZedKind::Siren).then(|| (solo_damage(float("ScreamDamage", 8.0)), float("ScreamRadius", 700.0), float("ScreamForce", -150000.0))),
        boss: if kind == ZedKind::Patriarch {
            match crate::zeds::boss::BossClass::load(&model, name_of("ChargingAnim").as_deref()) {
                Ok(b) => {
                    runlog::kv(
                        "boss_loaded",
                        &format!("claw_hits={:?} claw_range={} impale_hits={:?} impale_range={}", b.claw.hits, b.claw.range, b.impale.hits, b.impale.range),
                    );
                    Some(b)
                }
                Err(e) => {
                    runlog::kv("boss_error", &format!("error=\"{e}\""));
                    None
                }
            }
        } else {
            None
        },
        burst_bone: if kind == ZedKind::Bloat { name_of("SpineBone2").and_then(|n| model.find_bone(&n)) } else { None },
        cloak_parts: if kind == ZedKind::Patriarch {
            model
                .parts
                .iter()
                .map(|p| {
                    let texture = materials.get(&p.material).and_then(|m| m.base_color_texture.clone());
                    materials.add(StandardMaterial {
                        base_color: Color::srgba(0.85, 0.9, 1.0, 0.15),
                        base_color_texture: texture,
                        alpha_mode: AlphaMode::Blend,
                        perceptual_roughness: 0.2,
                        reflectance: 0.5,
                        cull_mode: None,
                        double_sided: true,
                        ..default()
                    })
                })
                .collect()
        } else {
            Vec::new()
        },
        cloak_material: (kind == ZedKind::Stalker).then(|| {
            let texture = model.parts.first().and_then(|p| materials.get(&p.material)).and_then(|m| m.base_color_texture.clone());
            materials.add(StandardMaterial {
                base_color: Color::srgba(0.85, 0.9, 1.0, 0.15),
                base_color_texture: texture,
                alpha_mode: AlphaMode::Blend,
                perceptual_roughness: 0.2,
                reflectance: 0.5,
                cull_mode: None,
                double_sided: true,
                ..default()
            })
        }),
        ext_collision,
        death,
        death_hold_frame,
        health: float("Health", 100.0),
        scoring_value: float("ScoringValue", 0.0),
        head_health: float("HeadHealth", 25.0),
        head_radius: float("HeadRadius", 7.0) * head_scale,
        head_offset: float("HeadHeight", 2.0) * head_scale,
        head_bone,
        melee_damage: solo_damage(float("MeleeDamage", 6.0)),
        melee_dam_type: match get("ZombieDamType") {
            Some((Value::Object(r), p)) if p.pkg.object_name(r).eq_ignore_ascii_case("DamTypeSlashingAttack") => crate::game::combat::DamType::Slashing,
            _ => crate::game::combat::DamType::ZombieMelee,
        },
        name: class_path.to_string(),
        draw_scale: float("DrawScale", 1.0),
        pre_pivot,
        collision_radius: float("CollisionRadius", 22.0),
        collision_height: float("CollisionHeight", 22.0),
        ground_speed: float("GroundSpeed", 440.0),
        hidden_speed: float("HiddenGroundSpeed", 300.0),
        turn_rate,
        melee_range: float("MeleeRange", 50.0),
        idle: name_of("IdleRestAnim").and_then(|n| model.sequence(&n)),
        walk: name_of("MovementAnims").and_then(|n| model.sequence(&n)),
        melee,
        door_bash: {
            let seq = model.sequence("DoorBash");
            if let Some(s) = seq {
                let notes: Vec<String> = model.notifies(s).iter().map(|n| format!("{}@{:.2}", n.name, n.time)).collect();
                runlog::kv("zed_door_bash_anim", &format!("class={class_path} frames={} notifies=[{}]", model.length(s), notes.join(" ")));
            }
            seq
        },
        distance_door_attack: matches!(get("bCanDistanceAttackDoors"), Some((Value::Bool(true), _))),
        intelligence: match get("Intelligence") {
            Some((Value::Byte(b), _)) => b,
            _ => 3,
        },
        model,
    })
}

/// A material by package and object name, drawn the way skinned models
/// draw it (e.g. the Fleshpound's red device shader).
pub(super) fn load_named_material(
    set: &PackageSet,
    package: &str,
    name: &str,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> Option<Handle<StandardMaterial>> {
    let lp = set.load(package)?;
    let export = (0..lp.pkg.exports.len()).find(|&i| lp.pkg.object_name(ObjectRef::Export(i)).eq_ignore_ascii_case(name))?;
    let h = ObjectHandle { package: lp, export };
    let simple = ue_assets::material::resolve(set, &h, ObjectRef::Export(export));
    let image = simple.texture.as_ref().and_then(|t| crate::render::skinned::decode_image(t, images))?;
    Some(materials.add(StandardMaterial {
        base_color_texture: Some(image),
        alpha_mode: match simple.blend {
            ue_assets::material::Blend::Additive => AlphaMode::Add,
            ue_assets::material::Blend::Masked => AlphaMode::Mask(0.5),
            _ => AlphaMode::Opaque,
        },
        cull_mode: None,
        double_sided: true,
        ..default()
    }))
}

/// Logs each hinge's angle range over the walk cycle next to its file limits,
/// with both sign conventions, to confirm which one contains the real motion.
pub(super) fn log_hinge_check(def: &RagdollDef, model: &SkinnedModel, walk: Option<usize>) {
    let Some(walk) = walk else {
        return;
    };
    let len = model.length(walk) as usize;
    let mut ranges: Vec<(String, f32, f32, f32, f32)> = Vec::new();
    for f in 0..len.max(1) {
        let pose = model.pose_with_bones(Some(walk), f as f32).1;
        for (i, (name, phi, low, high)) in def.hinge_angles(&pose).into_iter().enumerate() {
            if ranges.len() <= i {
                ranges.push((name, phi, phi, low, high));
            }
            let r = &mut ranges[i];
            r.1 = r.1.min(phi);
            r.2 = r.2.max(phi);
        }
    }
    for (name, lo, hi, low, high) in ranges {
        let inside = |a: f32, b: f32| lo >= a - 0.05 && hi <= b + 0.05;
        runlog::kv(
            "ragdoll_hinge_check",
            &format!(
                "part={name} walk_angle_range=({lo:.2}, {hi:.2}) file_limits=({low:.2}, {high:.2}) inside_as_is={} inside_mirrored={}",
                inside(low, high),
                inside(-high, -low)
            ),
        );
    }
}
