//! Loading weapons: the weapon and fire-mode classes' defaults (animations, timing, damage, spread, recoil, ammo, projectiles, sounds), meshes and skins.

use super::*;

/// A sound property as a full object path. KF's `XRef` strings (loaded by
/// name in PreloadAssets) win over the `X` object reference.
pub(crate) fn sound_prop(defaults: &ClassDefaults, class: &ObjectHandle, prop: &str) -> Option<String> {
    if let Some((Value::Str(s), _)) = defaults.get(class, &format!("{prop}Ref"))
        && !s.is_empty()
    {
        return Some(s);
    }
    match defaults.get(class, prop) {
        Some((Value::Object(r @ ObjectRef::Import(_)), lp)) => Some(lp.pkg.object_path(r)),
        Some((Value::Object(r @ ObjectRef::Export(_)), lp)) => Some(format!("{}.{}", lp.name, lp.pkg.object_path(r))),
        _ => None,
    }
}

/// An array of sounds as full paths: the `refs` strings (e.g.
/// MeleeHitSoundRefs) win over the `prop` objects (MeleeHitSounds).
pub(crate) fn sound_array(defaults: &ClassDefaults, class: &ObjectHandle, prop: &str, refs: &str) -> Vec<String> {
    if let Some((Value::Array { count, raw }, _)) = defaults.get(class, refs)
        && count > 0
    {
        let mut r = ue_assets::reader::Reader::new(&raw);
        return (0..count).filter_map(|_| r.fstring().ok()).filter(|s| !s.is_empty()).collect();
    }
    match defaults.get(class, prop) {
        Some((Value::Array { count, raw }, lp)) => {
            let mut r = ue_assets::reader::Reader::new(&raw);
            (0..count)
                .filter_map(|_| r.compact_index().ok().map(ObjectRef::from_raw))
                .filter_map(|rf| match rf {
                    ObjectRef::Import(_) => Some(lp.pkg.object_path(rf)),
                    ObjectRef::Export(_) => Some(format!("{}.{}", lp.name, lp.pkg.object_path(rf))),
                    ObjectRef::Null => None,
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

pub(super) fn load_fire_sounds(defaults: &ClassDefaults, fm: &ObjectHandle) -> FireSounds {
    let float = |p: &str, d: f32| match defaults.get(fm, p) {
        Some((Value::Float(f), _)) => f,
        _ => d,
    };
    let kf_fire = defaults.is_a(fm, "KFFire") || defaults.is_a(fm, "KFShotgunFire");
    let fire = sound_prop(defaults, fm, "FireSound");
    // KFFire.PreloadAssets: no stereo sound given -> the plain one.
    let stereo = kf_fire.then(|| sound_prop(defaults, fm, "StereoFireSound").or(fire.clone())).flatten();
    let random_pitch = if kf_fire && matches!(defaults.get(fm, "bRandomPitchFireSound"), Some((Value::Bool(true), _))) { float("RandomPitchAdjustAmt", 0.0) } else { 0.0 };
    let end = sound_prop(defaults, fm, "FireEndSound");
    FireSounds {
        fire,
        stereo,
        no_ammo: sound_prop(defaults, fm, "NoAmmoSound"),
        volume: float("TransientSoundVolume", 0.5),
        radius: float("TransientSoundRadius", 400.0),
        random_pitch,
        ambient: ["KFHighROFFire", "FlameBurstFire", "ChainsawFire", "HuskGunFire", "ZEDGunAltFire"]
            .iter()
            .any(|c| defaults.is_a(fm, c))
            .then(|| sound_prop(defaults, fm, "AmbientFireSound"))
            .flatten(),
        end_stereo: sound_prop(defaults, fm, "FireEndStereoSound").or(end),
        ambient_volume: match defaults.get(fm, "AmbientFireVolume") {
            Some((Value::Byte(b), _)) => b,
            _ => 255,
        },
        ambient_radius: float("AmbientFireSoundRadius", 500.0),
        melee_hits: sound_array(defaults, fm, "MeleeHitSounds", "MeleeHitSoundRefs").into(),
        melee_hit_volume: float("MeleeHitVolume", 1.0),
        fire_start: sound_prop(defaults, fm, "FireStartSound"),
        chainsaw: defaults.is_a(fm, "ChainsawFire"),
        charge_up: sound_prop(defaults, fm, "AmbientChargeUpSound"),
        charge_max: float("MaxChargeTime", 0.0),
        placed: defaults.is_a(fm, "PipeBombFire").then(|| "KF_AxeSnd.Axe_Fire".to_string()),
        empty_click: defaults.is_a(fm, "FlameBurstFire"),
    }
}

/// A HuskGunProjectile class's values: `base` with the class's own
/// ExplosionEmitter, ExplosionDecal and FlameTrailEmitterClass.
pub(super) fn husk_projectile(
    set: &PackageSet,
    defaults: &ClassDefaults,
    class: &ObjectHandle,
    base: crate::weapons::projectile::ExplosiveStats,
) -> crate::weapons::projectile::ExplosiveStats {
    use crate::render::decals::DecalKind;
    let path = |prop: &str| match defaults.get(class, prop) {
        Some((Value::Object(r), rp)) => set.resolve(&rp, r).map(|h| h.path()),
        _ => None,
    };
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let decal = match path("ExplosionDecal").as_deref().map(|p| p.rsplit('.').next().unwrap_or("").to_ascii_lowercase()) {
        Some(n) if n.ends_with("_small") => DecalKind::BurnSmall,
        Some(n) if n.ends_with("_large") => DecalKind::BurnLarge,
        _ => DecalKind::BurnMedium,
    };
    let sounds = projectile_sounds(defaults, class);
    crate::weapons::projectile::ExplosiveStats {
        class: leak(class.path()),
        // The Husk Gun's three classes differ in ExplosionSoundVolume
        // (1.25, 1.65, 2.0).
        sounds: crate::weapons::projectile::ProjectileSounds { explode_volume: sounds.explode_volume, ..base.sounds },
        effect: path("ExplosionEmitter").map_or(base.effect, leak),
        trail: path("FlameTrailEmitterClass").map(leak).or(base.trail),
        decal,
        ..base
    }
}

/// A projectile class's sounds (S5c): flight loop, explosion(s), bounce,
/// pipe-bomb beep, dud.
pub(super) fn projectile_sounds(defaults: &ClassDefaults, pc: &ObjectHandle) -> crate::weapons::projectile::ProjectileSounds {
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let float = |p: &str, d: f32| match defaults.get(pc, p) {
        Some((Value::Float(f), _)) => f,
        _ => d,
    };
    let byte = |p: &str| match defaults.get(pc, p) {
        Some((Value::Byte(b), _)) => b,
        _ => 0,
    };
    let mut explode = sound_array(defaults, pc, "ExplodeSounds", "ExplodeSoundRefs");
    if explode.is_empty() {
        explode.extend(sound_prop(defaults, pc, "ExplosionSound"));
    }
    let explode: Vec<&'static str> = explode.into_iter().map(leak).collect();
    let volume = byte("SoundVolume");
    let law_or_m79 = defaults.is_a(pc, "LAWProj") || defaults.is_a(pc, "M79GrenadeProjectile");
    crate::weapons::projectile::ProjectileSounds {
        flight: sound_prop(defaults, pc, "AmbientSound").filter(|_| volume > 0).map(|s| (leak(s), volume, float("SoundRadius", 64.0))),
        explode: Box::leak(explode.into_boxed_slice()),
        explode_volume: float("ExplosionSoundVolume", 2.0),
        explode_radius: float("TransientSoundRadius", 300.0),
        bounce: sound_prop(defaults, pc, "ImpactSound").map(leak),
        bounce_volume: float("TransientSoundVolume", 0.3),
        beep: sound_prop(defaults, pc, "BeepSound").map(leak),
        dud: law_or_m79.then_some("ProjectileSounds.PTRD_deflect04"),
    }
}

/// Which burn rules a damage type follows, if it has bDealBurningDamage
/// (KFMonster.TakeDamage / ZombieBloat / ZombieHusk tell them apart by class).
pub(super) fn fire_type(defaults: &ClassDefaults, dt: &ObjectHandle) -> Option<crate::game::combat::FireType> {
    use crate::game::combat::FireType;
    if !matches!(defaults.get(dt, "bDealBurningDamage"), Some((Value::Bool(true), _))) {
        return None;
    }
    let path = dt.path();
    let name = path.rsplit('.').next().unwrap_or("");
    Some(match name.to_ascii_lowercase().as_str() {
        "damtypetrenchgun" => FireType::Trenchgun,
        "damtypemac10mpinc" => FireType::Mac10,
        "damtypehuskgun" => FireType::HuskGun,
        "damtypeburned" => FireType::Burned,
        _ => FireType::Flamethrower,
    })
}

/// A damage type class for the perks: its class chain and
/// KFWeaponDamageType.bIsMeleeDamage.
pub(super) fn dam_type(defaults: &ClassDefaults, dt: &ObjectHandle) -> crate::game::perks::DamType {
    let melee = matches!(defaults.get(dt, "bIsMeleeDamage"), Some((Value::Bool(true), _)));
    crate::game::perks::intern_dam_type(crate::game::perks::ClassChain::new(defaults.chain_names(dt)), melee)
}

/// The damage type a class property names (MyDamageType, DamageType...).
pub(super) fn dam_type_prop(set: &PackageSet, defaults: &ClassDefaults, class: &ObjectHandle, prop: &str) -> Option<crate::game::perks::DamType> {
    match defaults.get(class, prop) {
        Some((Value::Object(r), rp)) if r != ObjectRef::Null => set.resolve(&rp, r).map(|dt| dam_type(defaults, &dt)),
        _ => None,
    }
}

/// Whether a fire class's ModeDoFire applies the perk's fire speed
/// (GetFireSpeed) and recoil (ModifyRecoilSpread): the nearest class in
/// its chain that overrides ModeDoFire decides (from the scripts; classes
/// whose override calls Super pass through).
fn perk_fire_rules(defaults: &ClassDefaults, fm: &ObjectHandle) -> (bool, bool) {
    // (class, applies fire speed, applies recoil)
    const OWNERS: [(&str, bool, bool); 14] = [
        ("WinchesterFire", true, true),
        ("KSGFire", true, true),
        ("NailGunFire", true, true),
        ("ChainsawFire", true, false),
        ("BoomStickAltFire", false, true),
        ("HuskGunFire", false, true),
        ("PipeBombFire", false, false),
        ("FragFire", false, false),
        ("ZEDGunAltFire", false, false),
        ("SyringeAltFire", false, false),
        ("KFFire", true, true),
        ("KFShotgunFire", true, true),
        ("KFMeleeFire", true, false),
        ("WeaponFire", false, false),
    ];
    for c in defaults.chain_names(fm) {
        if let Some(&(_, speed, recoil)) = OWNERS.iter().find(|(n, _, _)| n.eq_ignore_ascii_case(&c)) {
            return (speed, recoil);
        }
    }
    (false, false)
}

/// Reads a fire mode class's defaults (KFMeleeFire, KFFire, BaseProjectileFire...).
pub(super) fn load_fire_mode(set: &PackageSet, defaults: &ClassDefaults, fm_class: Option<&ObjectHandle>) -> FireMode {
    let mut mode = FireMode {
        kind: FireKind::None,
        sounds: FireSounds::default(),
        class: "None".to_string(),
        anims: vec!["Fire".to_string()],
        anim_rate: 1.0,
        aimed_anim: "none".into(),
        loop_anim: "none".into(),
        loop_aimed_anim: "none".into(),
        end_anim: "none".into(),
        end_aimed_anim: "none".into(),
        loop_anim_rate: 1.0,
        end_anim_rate: 1.0,
        anim2: "none".into(),
        aimed_anim2: "none".into(),
        penetrations: 1,
        rate: 0.5,
        wait_for_release: false,
        high_rof: false,
        fire_while_reloading: false,
        slows_movement: false,
        spread: Default::default(),
        recoil: Default::default(),
        chainsaw: false,
        extra_damage: 0,
        pellets: None,
        last_anim: "none".into(),
        last_aimed_anim: "none".into(),
        last_rate: None,
        last_rule: LastShot::Never,
        total_ammo_only: false,
        requires_aim: false,
        spawn_delay: None,
        fire: None,
        charge: None,
        weld: false,
        mode_exclusive: true,
        beam: None,
        combat: CombatStats {
            headshot_mult: 1.0,
            ..default()
        },
        mac10_inc: None,
        perk_speed: false,
        perk_recoil: false,
        base_rate: 0.5,
        base_anim_rate: 1.0,
        base_damage_delay: 0.0,
    };
    let Some(fm_class) = fm_class else {
        return mode;
    };
    mode.class = fm_class.path();
    mode.sounds = load_fire_sounds(defaults, fm_class);
    let fget = |p: &str| defaults.get(fm_class, p);
    let ffloat = |p: &str, d: f32| match fget(p) {
        Some((Value::Float(f), _)) => f,
        Some((Value::Int(i), _)) => i as f32,
        _ => d,
    };
    // Melee fire modes have MeleeDamage; projectile ones a
    // ProjectileClass; instant-hit ones DamageMin/Max.
    let combat = &mut mode.combat;
    let melee_damage = ffloat("MeleeDamage", 0.0);
    combat.melee = melee_damage > 0.0;
    let has_projectile = matches!(fget("ProjectileClass"), Some((Value::Object(r), _)) if r != ObjectRef::Null);
    // WeldFire / UnWeldFire are melee classes that only work on doors
    // (WeldFire.Timer looks for a KFDoorMover): `weld_fire` and door.rs.
    let welds = mode.class.contains("WeldFire");
    let projectile_class = match fget("ProjectileClass") {
        Some((Value::Object(r), rp)) if r != ObjectRef::Null => set.resolve(&rp, r),
        _ => None,
    };
    let pellet_class = projectile_class.as_ref().filter(|p| {
        // Frags (FragFire, thrown with G) and pipe bombs.
        defaults.is_a(p, "Nade") || defaults.is_a(p, "PipeBombProjectile") ||
        defaults.is_a(fm_class, "KFShotgunFire")
            && (defaults.is_a(p, "ShotgunBullet")
                || defaults.is_a(p, "TrenchgunBullet")
                // Grenades, rockets and the Husk Gun's fireball (a LAWProj).
                || defaults.is_a(p, "M79GrenadeProjectile")
                || defaults.is_a(p, "LAWProj")
                // Medic darts (W8b).
                || defaults.is_a(p, "HealingProjectile")
                || defaults.is_a(p, "CrossbowArrow")
                || defaults.is_a(p, "M99Bullet"))
    });
    mode.kind = if welds {
        FireKind::None
    } else if combat.melee {
        FireKind::Melee
    } else if pellet_class.is_some() {
        FireKind::Pellets
    } else if has_projectile {
        FireKind::Projectile
    } else if ffloat("DamageMax", 0.0) > 0.0 {
        FireKind::Instant
    } else {
        FireKind::None
    };
    if combat.melee {
        combat.damage_min = melee_damage;
        combat.damage_max = melee_damage;
        combat.range = ffloat("weaponRange", 70.0);
        combat.damage_delay = ffloat("DamagedelayMin", 0.3);
        combat.min_dot = ffloat("WideDamageMinHitAngle", 1.0);
    } else {
        combat.damage_min = ffloat("DamageMin", 0.0);
        combat.damage_max = ffloat("DamageMax", 0.0);
        combat.spread = ffloat("Spread", 0.0);
    }
    // HeadShotDamageMult from the damage type class.
    let dt_name = if combat.melee { "hitDamageClass" } else { "DamageType" };
    if let Some((Value::Object(dt), dt_pkg)) = fget(dt_name)
        && let Some(dt_class) = set.resolve(&dt_pkg, dt)
        && let Some((Value::Float(m), _)) = defaults.get(&dt_class, "HeadShotDamageMult")
    {
        combat.headshot_mult = m;
    }
    if let Some((Value::Object(dt), dt_pkg)) = fget(dt_name)
        && let Some(dt_class) = set.resolve(&dt_pkg, dt)
    {
        mode.fire = fire_type(defaults, &dt_class);
        mode.combat.dam = Some(dam_type(defaults, &dt_class));
    }
    // MAC10Fire.DoTrace: DamageType = GetMAC10DamageType (KFVetFirebug:
    // DamTypeMAC10MPInc).
    if defaults.is_a(fm_class, "MAC10Fire") {
        mode.mac10_inc = crate::zeds::gore::find_class(set, "KFMod.DamTypeMAC10MPInc").map(|dt| dam_type(defaults, &dt));
    }
    (mode.perk_speed, mode.perk_recoil) = perk_fire_rules(defaults, fm_class);
    if let Some((Value::Name(n), np)) = fget("FireAnim") {
        mode.anims = vec![np.pkg.name(n).to_string()];
    }
    if let Some((Value::Array { count, raw }, np)) = fget("FireAnims") {
        let mut r = ue_assets::reader::Reader::new(&raw);
        let names: Vec<String> = (0..count)
            .filter_map(|_| r.compact_index().ok())
            .filter(|&i| i >= 0 && (i as usize) < np.pkg.names.len())
            .map(|i| np.pkg.name(i as usize).to_string())
            .collect();
        if !names.is_empty() {
            mode.anims = names;
        }
    }
    mode.anim_rate = ffloat("FireAnimRate", 1.0);
    mode.rate = ffloat("FireRate", 0.5);
    mode.base_anim_rate = mode.anim_rate;
    mode.base_rate = mode.rate;
    mode.base_damage_delay = mode.combat.damage_delay;
    let fbool = |p: &str| matches!(fget(p), Some((Value::Bool(true), _)));
    let fname = |p: &str| match fget(p) {
        Some((Value::Name(n), np)) => np.pkg.name(n).to_ascii_lowercase(),
        _ => "none".to_string(),
    };
    mode.wait_for_release = fbool("bWaitForRelease");
    mode.aimed_anim = fname("FireAimedAnim");
    mode.loop_anim = fname("FireLoopAnim");
    mode.loop_aimed_anim = fname("FireLoopAimedAnim");
    mode.end_anim = fname("FireEndAnim");
    mode.end_aimed_anim = fname("FireEndAimedAnim");
    mode.loop_anim_rate = ffloat("FireLoopAnimRate", 1.0);
    mode.end_anim_rate = ffloat("FireEndAnimRate", 1.0);
    mode.chainsaw = defaults.is_a(fm_class, "ChainsawFire");
    mode.extra_damage = match fget("maxAdditionalDamage") {
        Some((Value::Int(i), _)) => i.max(0) as u32,
        _ => 0,
    };
    // ChainsawFire's and FlameBurstFire's FireLoop states work like
    // KFHighROFFire's (loop FireLoopAnim while held, no PlayFiring).
    mode.weld = defaults.is_a(fm_class, "WeldFire");
    mode.mode_exclusive = !matches!(fget("bModeExclusive"), Some((Value::Bool(false), _)));
    mode.beam = defaults.is_a(fm_class, "ZEDGunAltFire").then(|| BeamFire {
        range: ffloat("TraceRange", 2500.0),
        sphere_time: ffloat("MaxZedSphereChargeTime", 3.0),
        offset: match fget("ProjSpawnOffset") {
            Some((Value::Vector(v), _)) => Vec3::from_array(v),
            _ => Vec3::new(25.0, 18.0, -14.5),
        },
        effect: match fget("ChargeEmitterClass") {
            Some((Value::Object(r), rp)) => set.resolve(&rp, r).map(|h| h.path()),
            _ => None,
        },
    });
    mode.high_rof = defaults.is_a(fm_class, "KFHighROFFire") || mode.chainsaw || defaults.is_a(fm_class, "FlameBurstFire");
    mode.anim2 = fname("FireAnim2");
    mode.aimed_anim2 = fname("FireAimedAnim2");
    mode.penetrations = if PENETRATING_FIRE.iter().any(|c| defaults.is_a(fm_class, c)) { 5 } else { 1 };
    // ZEDGunFire / ZEDMKIIFire / ZEDMKIIAltFire.AllowFire: never while reloading.
    let zed_fire = ["ZEDGunFire", "ZEDMKIIFire", "ZEDMKIIAltFire"].iter().any(|c| defaults.is_a(fm_class, c));
    mode.fire_while_reloading = !zed_fire && (defaults.is_a(fm_class, "WinchesterFire") || defaults.is_a(fm_class, "KFShotgunFire"));
    // KFFire.ModeDoFire and KFShotgunFire.ModeDoFire slow the player.
    mode.slows_movement = (defaults.is_a(fm_class, "KFFire") || defaults.is_a(fm_class, "KFShotgunFire"))
        && !fbool("bFiringDoesntAffectMovement");
    mode.spread = crate::weapons::firing::SpreadParams {
        spread: mode.combat.spread,
        max_spread: ffloat("MaxSpread", 0.0),
        semi_auto_bonus: fbool("bAccuracyBonusForSemiAuto"),
    };
    mode.recoil = crate::weapons::firing::RecoilParams {
        rate: ffloat("RecoilRate", 0.09),
        max_vertical: ffloat("maxVerticalRecoilAngle", 0.0),
        max_horizontal: ffloat("maxHorizontalRecoilAngle", 0.0),
        right_only: fbool("bRecoilRightOnly"),
        velocity_scale: ffloat("RecoilVelocityScale", 0.0),
    };
    if let Some(pc) = pellet_class {
        // Leaked once per loaded fire mode (projectiles keep a &'static name).
        let projectile_path: &'static str = Box::leak(pc.path().into_boxed_str());
        let pget = |p: &str| defaults.get(pc, p);
        let pfloat = |p: &str, d: f32| match pget(p) {
            Some((Value::Float(f), _)) => f,
            Some((Value::Int(i), _)) => i as f32,
            Some((Value::Byte(b), _)) => b as f32,
            _ => d,
        };
        let dt_mult = match pget("MyDamageType") {
            Some((Value::Object(r), rp)) => set
                .resolve(&rp, r)
                .and_then(|dt| match defaults.get(&dt, "HeadShotDamageMult") {
                    Some((Value::Float(m), _)) => Some(m),
                    _ => None,
                })
                .unwrap_or(1.0),
            _ => 1.0,
        };
        let vector = |p: &str| match fget(p) {
            Some((Value::Vector(v), _)) => Vec3::from_array(v),
            _ => Vec3::ZERO,
        };
        let int = |p: &str, d: u32| match fget(p) {
            Some((Value::Int(i), _)) => i.max(0) as u32,
            Some((Value::Byte(b), _)) => b as u32,
            _ => d,
        };
        let class_mult = |prop: &str| match pget(prop) {
            Some((Value::Object(r), rp)) => set
                .resolve(&rp, r)
                .and_then(|dt| match defaults.get(&dt, "HeadShotDamageMult") {
                    Some((Value::Float(m), _)) => Some(m),
                    _ => None,
                })
                .unwrap_or(1.0),
            _ => 1.0,
        };
        let damage_type = match pget("MyDamageType") {
            Some((Value::Object(r), rp)) => set.resolve(&rp, r).map(|h| h.path()).unwrap_or_default(),
            _ => String::new(),
        };
        let is_law = defaults.is_a(pc, "LAWProj");
        // HuskGunProjectile (a LAWProj): impact damage on every touch, a
        // burning blast that spares the player, its own effects.
        let is_husk = defaults.is_a(pc, "HuskGunProjectile");
        // ZED gun bolts (LAWProj subclasses): ProcessTouch deals Damage
        // (x HeadShotDamageMult on a headshot) with MyDamageType; BlowUp
        // hurts nothing. The MKII's alt orb zaps instead (W9).
        let is_zed_bolt = defaults.is_a(pc, "ZEDGunProjectile") || defaults.is_a(pc, "ZEDMKIIPrimaryProjectile");
        let is_zed_orb = defaults.is_a(pc, "ZEDMKIISecondaryProjectile");
        let fire = match pget("MyDamageType") {
            Some((Value::Object(r), rp)) => set.resolve(&rp, r).and_then(|dt| fire_type(defaults, &dt)),
            _ => None,
        };
        let explosive = (defaults.is_a(pc, "M79GrenadeProjectile") || is_law).then(|| crate::weapons::projectile::ExplosiveStats {
            class: projectile_path,
            sounds: projectile_sounds(defaults, pc),
            speed: pfloat("Speed", 2000.0),
            damage: if is_zed_bolt { 0.0 } else { pfloat("Damage", 0.0) },
            radius: if is_zed_bolt { 0.0 } else { pfloat("DamageRadius", 0.0) },
            momentum: pfloat("MomentumTransfer", 0.0),
            impact_damage: if is_zed_bolt { pfloat("Damage", 0.0) } else { pfloat("ImpactDamage", 0.0) },
            impact_headshot_mult: if is_zed_bolt { dt_mult } else { class_mult("ImpactDamageType") },
            zap: is_zed_orb.then(|| pfloat("ZapAmount", 1.5)),
            arm_dist: pfloat("ArmDistSquared", 0.0).sqrt(),
            straight_time: (!is_law).then(|| pfloat("StraightFlightTime", 0.25)),
            life_span: pfloat("LifeSpan", 10.0),
            // ZombieFleshPound.TakeDamage: the frag and pipe bomb double,
            // the other explosive types (all of these) count fully.
            // DamTypeHuskGun is not in its list.
            fleshpound_mult: if is_husk || is_zed_bolt || is_zed_orb {
                None
            } else if damage_type.ends_with("DamTypeFrag") || damage_type.ends_with("DamTypePipeBomb") {
                Some(2.0)
            } else {
                Some(1.0)
            },
            impact_on_touch: (is_husk || is_zed_bolt).then(|| pfloat("HeadShotDamageMult", 1.5)),
            fire,
            dam: dam_type_prop(set, defaults, pc, "MyDamageType"),
            // ZED bolts deal Damage with MyDamageType on touch.
            impact_dam: if is_zed_bolt { dam_type_prop(set, defaults, pc, "MyDamageType") } else { dam_type_prop(set, defaults, pc, "ImpactDamageType") },
            hurts_self: !(is_husk || is_zed_bolt || is_zed_orb),
            // Explode: LAWProj spawns LawExplosion, the M79 family
            // KFNadeLExplosion; ExplosionDecal RocketMarkDirt / KFScorchMark.
            effect: if is_law { "KFMod.LawExplosion" } else { "KFMod.KFNadeLExplosion" },
            decal: if is_law { crate::render::decals::DecalKind::RocketMark } else { crate::render::decals::DecalKind::NadeScorch },
            trail: Some("ROEffects.PanzerfaustTrail"),
        });
        let explosive = match explosive {
            Some(x) if is_husk || is_zed_bolt || is_zed_orb => Some(husk_projectile(set, defaults, pc, x)),
            x => x,
        };
        // HuskGunFire.GetDesiredProjectileClass: Weak / ProjectileClass /
        // Strong by HoldTime; the subclasses only change the effects.
        if let Some(x) = explosive
            && defaults.is_a(fm_class, "HuskGunFire")
        {
            let variant = |prop: &str| match fget(prop) {
                Some((Value::Object(r), rp)) => set.resolve(&rp, r).map(|h| husk_projectile(set, defaults, &h, x)).unwrap_or(x),
                _ => x,
            };
            mode.charge = Some(ChargeFire {
                max_time: ffloat("MaxChargeTime", 3.0),
                weak: variant("WeakProjectileClass"),
                medium: x,
                strong: variant("StrongProjectileClass"),
                effect: match fget("ChargeEmitterClass") {
                    Some((Value::Object(r), rp)) => set.resolve(&rp, r).map(|h| h.path()),
                    _ => None,
                },
            });
        }
        let is_pipe = defaults.is_a(pc, "PipeBombProjectile");
        // CrossbowArrow / M99Bullet: TakeDamage with DamageTypeHeadShot on a
        // headshot (its HeadShotDamageMult is then the one KFMonster applies).
        let is_bolt = defaults.is_a(pc, "CrossbowArrow") || defaults.is_a(pc, "M99Bullet");
        let dt_mult = if is_bolt { class_mult("DamageTypeHeadShot") } else { dt_mult };
        let thrown = (defaults.is_a(pc, "Nade") || is_pipe).then(|| crate::weapons::projectile::ThrownStats {
            class: projectile_path,
            sounds: projectile_sounds(defaults, pc),
            // FragFire.PostSpawnProjectile: a quick throw (HoldTime 0) at
            // mHoldSpeedMin; the pipe bomb at its own Speed.
            speed: if is_pipe { pfloat("Speed", 50.0) } else { ffloat("mHoldSpeedMin", 850.0) },
            damage: pfloat("Damage", 0.0),
            radius: pfloat("DamageRadius", 0.0),
            dampen_normal: pfloat("DampenFactor", 0.25),
            dampen_parallel: pfloat("DampenFactorParallel", 0.4),
            fleshpound_mult: 2.0,
            // Nade.Explode: KFNadeExplosion; PipeBombProjectile: KFNadeLExplosion.
            effect: if is_pipe { "KFMod.KFNadeLExplosion" } else { "KFMod.KFNadeExplosion" },
            decal: crate::render::decals::DecalKind::NadeScorch,
            kind: if is_pipe {
                crate::weapons::projectile::ThrownKind::Pipe {
                    arming: pfloat("ArmingCountDown", 1.0),
                    detection_radius: pfloat("DetectionRadius", 150.0),
                    countdown: pfloat("CountDown", 5.0) as u32,
                    threshold: pfloat("ThreatThreshhold", 1.0),
                }
            } else {
                crate::weapons::projectile::ThrownKind::Frag { fuse: pfloat("ExplodeTimer", 2.0) }
            },
            dam: dam_type_prop(set, defaults, pc, "MyDamageType"),
        });
        // FlameBurstFire (a CrossbowFire) overrides AllowFire: it needs a
        // round in the magazine and never fires while reloading.
        let flame_fire = defaults.is_a(fm_class, "FlameBurstFire");
        mode.total_ammo_only = !flame_fire
            && ["M79Fire", "M203Fire", "LAWFire", "CrossbowFire", "M99Fire", "HuskGunFire"]
                .iter()
                .any(|c| defaults.is_a(fm_class, c));
        if flame_fire {
            mode.fire_while_reloading = false;
        }
        mode.requires_aim = defaults.is_a(fm_class, "LAWFire");
        mode.spawn_delay = defaults.is_a(fm_class, "PipeBombFire").then(|| ffloat("ProjectileSpawnDelay", 1.1));
        // FlameTendril: falls, bursts after two 0.2 s timers or on touch.
        let flame = defaults.is_a(pc, "FlameTendril").then(|| crate::weapons::projectile::FlameStats {
            speed: pfloat("Speed", 2300.0),
            toss_z: pfloat("TossZ", 200.0),
            damage: pfloat("Damage", 12.0),
            radius: pfloat("DamageRadius", 150.0),
            life_span: pfloat("LifeSpan", 5.0),
        });
        let dart = defaults.is_a(pc, "HealingProjectile").then(|| crate::weapons::projectile::DartStats {
            class: projectile_path,
            speed: pfloat("Speed", 10000.0),
            life_span: pfloat("LifeSpan", 10.0),
            heal: pfloat("HealBoostAmount", 20.0),
            flight: projectile_sounds(defaults, pc).flight,
        });
        let per_load = !["MP7MAltFire", "M7A3MAltFire", "ZEDMKIIAltFire"].iter().any(|c| defaults.is_a(fm_class, c));
        mode.pellets = Some(PelletFire {
            per_load,
            dart,
            flame,
            thrown,
            explosive,
            stats: crate::weapons::projectile::ProjectileStats {
                class: projectile_path,
                speed: pfloat("Speed", 3500.0),
                damage: pfloat("Damage", 0.0),
                max_penetrations: pfloat("MaxPenetrations", 1.0),
                pen_damage_reduction: pfloat("PenDamageReduction", 0.5),
                headshot_mult: pfloat("HeadShotDamageMult", 1.5),
                damage_type_headshot_mult: dt_mult,
                life_span: pfloat("LifeSpan", 3.0),
                bounces: pfloat("Bounces", 0.0) as u32,
                rule: if is_bolt { crate::weapons::projectile::PenRule::Bolt } else { crate::weapons::projectile::PenRule::Pellet },
                pickup: defaults.is_a(pc, "CrossbowArrow"),
                fire: match pget("MyDamageType") {
                    Some((Value::Object(r), rp)) => set.resolve(&rp, r).and_then(|dt| fire_type(defaults, &dt)),
                    _ => None,
                },
                dam: dam_type_prop(set, defaults, pc, "MyDamageType"),
            },
            per_fire: int("ProjPerFire", 1),
            ammo_per_fire: int("AmmoPerFire", 1),
            // SpreadStyle: SS_None (0) fires straight; SS_Random (1) spreads.
            spread: if int("SpreadStyle", 1) == 0 { 0.0 } else { mode.combat.spread },
            kick: vector("KickMomentum"),
            spawn_offset: vector("ProjSpawnOffset"),
        });
        // KFShotgunFire.HandleRecoil: the sideways kick may go either way,
        // and moving adds speed x 3 (RecoilVelocityScale is not used).
        mode.recoil.right_only = false;
        mode.recoil.velocity_scale = 3.0;
    }
    mode.last_anim = fname("FireLastAnim");
    mode.last_aimed_anim = fname("FireLastAimedAnim");
    mode.last_rule = if defaults.is_a(fm_class, "BoomStickAltFire") {
        LastShot::WhenEmptied
    } else if defaults.is_a(fm_class, "BoomStickFire") {
        LastShot::WhenVeryLast
    } else {
        LastShot::Never
    };
    mode.last_rate = match fget("FireLastRate") {
        Some((Value::Float(f), _)) => Some(f),
        _ => None,
    };
    mode
}

#[allow(clippy::too_many_arguments)]
pub(super) fn load_weapons(
    mut kept: NonSendMut<WeaponAssets>,
    mut commands: Commands,
    request: Res<MapRequest>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    main_cam: Query<(Entity, &Transform), With<FlyCamera>>,
    loadout: Res<WeaponLoadout>,
    character: Res<crate::player::character::CharacterChoice>,
    (vet, mut inv): (Res<crate::game::perks::Veterancy>, ResMut<crate::game::buy_menu::ShopInventory>),
) {
    let started = std::time::Instant::now();
    let set = PackageSet::new(&request.install_root);
    let defaults = ClassDefaults::new(&set);
    // The player's character gives every weapon its sleeves.
    let chosen = crate::player::character::choose(&set, &defaults, &request.install_root, character.0.as_deref());
    let character_name = chosen.as_ref().map_or("none".to_string(), |c| c.name.clone());
    let sleeve = chosen.and_then(|c| c.sleeve);
    let Ok((_, main_t)) = main_cam.single() else {
        return;
    };

    // Weapon camera: renders only the weapon layer, after the scene, with its
    // own depth buffer so the weapon is never hidden by walls.
    let cam = commands
        .spawn((
            Camera3d::default(),
            Camera {
                order: 1,
                clear_color: ClearColorConfig::None,
                ..default()
            },
            Projection::from(PerspectiveProjection {
                fov: crate::engine::camera::vertical_fov(70.0),
                near: 0.01,
                ..default()
            }),
            *main_t,
            // As the main camera: UE2 had no tonemapping.
            bevy::core_pipeline::tonemapping::Tonemapping::None,
            RenderLayers::layer(WEAPON_LAYER),
            WeaponCamera,
        ))
        .id();

    // The inventory: KF's starting weapons (RequiredEquipment), the perk's
    // (AddDefaultInventory -> CreateInventoryVeterancy, with its SellValue;
    // skipped if already carried), then any given with --give.
    let mut classes: Vec<String> = STARTING_WEAPONS.iter().map(|c| c.to_string()).collect();
    let (perk_items, _) = vet.vet.default_inventory(crate::game::dosh::GAME_DIFFICULTY);
    for (c, _) in &perk_items {
        if !classes.iter().any(|have| have.eq_ignore_ascii_case(c)) {
            classes.push(c.to_string());
        }
    }
    for c in &loadout.give {
        if !classes.iter().any(|have| have.eq_ignore_ascii_case(c)) {
            classes.push(c.clone());
        }
    }
    let mut defs = Vec::new();
    let mut failed = Vec::new();
    for class_path in &classes {
        match load_weapon(&set, &defaults, class_path, sleeve.as_ref(), &mut meshes, &mut images, &mut materials) {
            Ok(mut def) => {
                spawn_parts(&mut commands, &mut def, cam);
                let changes = apply_vet(&mut def, vet.vet, true);
                if !changes.is_empty() {
                    runlog::kv("perk_mod", &format!("kind=weapon perk={} weapon={} reason=start {}", vet.vet.label(), def.class, changes.join(" ")));
                }
                if let Some(&(_, sell)) = perk_items.iter().find(|(c, _)| c.eq_ignore_ascii_case(class_path)) {
                    def.sell_value = Some(sell);
                    runlog::kv("perk_start_weapon", &format!("perk={} weapon={class_path} sell_value={sell}", vet.vet.label()));
                }
                runlog::kv(
                    "weapon_loaded",
                    &format!(
                        "class={} name=\"{}\" group={} group_offset={} priority={} weight={} modes={:?} bones={} points={} triangles={} parts={} sequences={:?} display_fov={} view_offset_unreal={:?} speed_bonus={} ammo={:?} iron_sights={:?}",
                        def.class,
                        def.item_name,
                        def.group,
                        def.group_offset,
                        def.priority,
                        def.weight,
                        def.modes.iter().map(|m| format!("{}:{:?}:{:?}", m.class.rsplit('.').next().unwrap_or(""), m.kind, m.combat)).collect::<Vec<_>>(),
                        def.model.mesh.bones.len(),
                        def.model.mesh.points.len(),
                        def.model.mesh.triangles.len(),
                        def.model.parts.len(),
                        def.model.anim.as_ref().map(|a| a.sequences.iter().map(|s| s.name.clone()).collect::<Vec<_>>()),
                        def.display_fov,
                        (def.view_offset / coords::SCALE).to_array(),
                        def.speed_bonus,
                        def.ammo,
                        def.iron
                    ),
                );
                // Animations the data names that the model lacks (KF's
                // HasAnim checks skip them; listed to catch wrong names).
                let mut wanted: Vec<String> =
                    vec![def.reload_anim.clone(), def.select_anim.clone(), def.put_down_anim.clone(), def.idle_anim.clone()];
                if let Some(iron) = &def.iron {
                    wanted.push(iron.idle_anim.to_ascii_lowercase());
                }
                for m in def.modes.iter().filter(|m| m.kind != FireKind::None) {
                    wanted.extend(m.anims.iter().map(|a| a.to_ascii_lowercase()));
                    wanted.extend([&m.aimed_anim, &m.loop_anim, &m.loop_aimed_anim, &m.end_anim, &m.end_aimed_anim].map(|a| a.clone()));
                }
                let mut missing: Vec<String> =
                    wanted.into_iter().filter(|a| a != "none" && def.model.sequence(a).is_none()).collect();
                missing.dedup();
                if !missing.is_empty() {
                    runlog::kv("weapon_anims_missing", &format!("class={} anims={missing:?}", def.class));
                }
                let at = inventory_position(&slots(&defs), def.slot());
                defs.insert(at, def);
            }
            Err(e) => {
                runlog::kv("weapon_error", &format!("class={class_path} error=\"{e}\""));
                failed.push(class_path.clone());
            }
        }
    }
    // Dual pistols replace the single they pair with (Dualies.GiveTo).
    for (dual_class, single_class) in DUAL_PAIRS {
        let dual = defs.iter().position(|d| d.class.eq_ignore_ascii_case(&format!("KFMod.{dual_class}")));
        let single = defs.iter().position(|d| d.class.eq_ignore_ascii_case(&format!("KFMod.{single_class}")));
        if let (Some(di), Some(si)) = (dual, single) {
            let single_def = &defs[si];
            if let (Some(s_ammo), Some(d_ammo)) = (single_def.ammo, defs[di].ammo) {
                let merged = merge_dual_ammo(s_ammo, d_ammo);
                runlog::kv(
                    "dual_from_single",
                    &format!(
                        "dual={dual_class} single={single_class} single_mag={} single_spare={} -> mag={} spare={}",
                        s_ammo.mag, s_ammo.spare, merged.mag, merged.spare
                    ),
                );
                defs[di].ammo = Some(merged);
            }
            for &e in &defs[si].entities {
                commands.entity(e).despawn();
            }
            defs.remove(si);
        }
    }
    let weight: f32 = defs.iter().map(|d| d.weight).sum();
    let max_weight = max_carry_weight(&vet.vet);
    inv.max_weight = max_weight;
    runlog::kv(
        "inventory",
        &format!(
            "character={character_name} loaded={} failed={} failed_classes={:?} weight={weight} max_carry_weight={max_weight} over_limit={} given_by_test_flag={} order={:?}",
            defs.len(),
            failed.len(),
            failed,
            weight > max_weight,
            loadout.give.len(),
            defs.iter().map(|d| format!("{}:{}", d.group, d.item_name)).collect::<Vec<_>>()
        ),
    );
    if defs.is_empty() {
        return;
    }
    // The Welder's screen (welder_screen.rs).
    if defs.iter().any(|d| d.class.eq_ignore_ascii_case("KFMod.Welder")) {
        commands.insert_resource(super::welder_screen::load(&set, &defaults, &mut images, &mut materials));
    }
    // Start with the 9mm, as KF does (the dual 9mms if they replaced it).
    let current = ["KFMod.Single", "KFMod.Dualies"]
        .iter()
        .find_map(|c| defs.iter().position(|d| d.class.eq_ignore_ascii_case(c)))
        .unwrap_or(0);
    let mut w = Weapons {
        defs,
        camera: cam,
        current,
        action: Action::Select,
        sequence: None,
        anim: String::new(),
        frame: 0.0,
        play_rate: 30.0,
        looping: false,
        notify_frame: -1.0,
        fire_cooldown: [0.0; 2],
        fire_count: 0,
        firing: [false; 2],
        shots_this_press: [0; 2],
        spread_state: Default::default(),
        reload_timer: 0.0,
        boomstick_pending: None,
        pending_spawn: None,
        charge_hold: None,
        charge_fx: None,
        pending_inject: None,
        beam: None,
        last_heal_attempt: -10.0,
        last_weld_fail: -10.0,
        quick_heal: QuickHeal::Off,
        switch_timer: 0.0,
        down_delayed: false,
        pending_swings: Vec::new(),
        pending_welds: Vec::new(),
        press_waiting: [false; 2],
        aiming: false,
        zoom: 0.0,
        zoom_time: 0.25,
        rng: 0x2545_F491_4F6C_DD1D,
        fx_shots: Vec::new(),
        hand_frames: Vec::new(),
        dual_left: [false; 2],
        sounds: Vec::new(),
        last_click: -10.0,
        sound_rng: 0x1b87_3593,
        torch: TorchState::default(),
        vet: vet.vet,
    };
    set_action(&mut w, Action::Select);
    commands.insert_resource(w);
    // KFHumanPawn.ModifyVelocity: the weight counts up to MaxCarryWeight;
    // the perk's GetMovementSpeedModifier.
    let perk_speed_mult = vet.vet.movement_speed(crate::game::dosh::GAME_DIFFICULTY);
    if perk_speed_mult != 1.0 {
        runlog::kv("perk_mod", &format!("kind=move_speed perk={} ground_speed_mult={perk_speed_mult}", vet.vet.label()));
    }
    commands.insert_resource(WeaponEffects {
        ground_speed_bonus: 0.0,
        weight_speed_mult: weight_speed_mult(weight, &vet.vet),
        fire_velocity_scale: None,
        perk_speed_mult,
    });
    runlog::kv("weapons_ready", &format!("seconds={:.2}", started.elapsed().as_secs_f64()));
    drop(defaults);
    kept.0 = Some(set);
    kept.1 = sleeve;
}

pub(super) fn load_weapon(
    set: &PackageSet,
    defaults: &ClassDefaults,
    class_path: &str,
    sleeve: Option<&ObjectHandle>,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> Result<WeaponDef, String> {
    let (pkg_name, class_name) = class_path.split_once('.').ok_or("bad class path")?;
    let lp = set.load(pkg_name).ok_or("package not found")?;
    let class_export = (0..lp.pkg.exports.len())
        .find(|&i| {
            lp.pkg.export_class_name(i) == "Class"
                && lp.pkg.object_name(ObjectRef::Export(i)).eq_ignore_ascii_case(class_name)
        })
        .ok_or("class not found")?;
    let class = ObjectHandle {
        package: lp.clone(),
        export: class_export,
    };
    let get = |p: &str| defaults.get(&class, p);

    // The mesh: Mesh, or for weapons added after release MeshRef, a path
    // loaded by KFWeapon.PreloadAssets (DynamicLoadObject).
    let mesh_h = match (get("Mesh"), get("MeshRef")) {
        (Some((Value::Object(r), p)), _) if r != ObjectRef::Null => set.resolve(&p, r).ok_or("mesh not found")?,
        (_, Some((Value::Str(path), _))) => set
            .find_object(&path, Some("SkeletalMesh"))
            .ok_or_else(|| format!("MeshRef {path} not found"))?,
        _ => return Err("no Mesh or MeshRef default".into()),
    };

    let float = |p: &str, d: f32| match get(p) {
        Some((Value::Float(f), _)) => f,
        _ => d,
    };
    let view_offset = match get("PlayerViewOffset") {
        Some((Value::Vector(v), _)) => v,
        _ => [0.0; 3],
    };
    let display_fov = float("DisplayFOV", 90.0);
    let bob_damping = float("BobDamping", 0.96);
    let speed_me_up = matches!(get("bSpeedMeUp"), Some((Value::Bool(true), _)));
    let speed_bonus = if speed_me_up {
        // KFHumanPawn: default.GroundSpeed * BaseMeleeIncrease - Weight * 2
        // (the perk's GetMeleeMovementSpeedModifier is added by apply_vet).
        200.0 * 0.2 - float("Weight", 0.0) * 2.0
    } else {
        0.0
    };

    let name = |p: &str| match get(p) {
        Some((Value::Name(n), np)) => Some(np.pkg.name(n).to_string()),
        _ => None,
    };
    let int = |p: &str, d: i32| match get(p) {
        Some((Value::Int(i), _)) => i,
        Some((Value::Byte(b), _)) => b as i32,
        _ => d,
    };
    let item_name: &'static str = match get("ItemName") {
        // Leaked once per loaded weapon, so shot messages can stay Copy.
        Some((Value::Str(s), _)) => Box::leak(s.into_boxed_str()),
        _ => Box::leak(class_name.to_string().into_boxed_str()),
    };
    let group = int("InventoryGroup", 1).clamp(0, 255) as u8;
    let group_offset = int("GroupOffset", 0);
    let priority = int("Priority", 0);
    let weight = float("Weight", 0.0);
    let has_aiming = matches!(get("bHasAimingMode"), Some((Value::Bool(true), _)));
    // Fire modes: FireModeClass[0] (left mouse) and [1] (alt fire).
    let mode_class = |i: u32| match defaults.get_at(&class, "FireModeClass", i) {
        Some((Value::Object(fm), fm_pkg)) => set.resolve(&fm_pkg, fm),
        _ => None,
    };
    let primary_class = mode_class(0);
    let modes = [
        load_fire_mode(set, defaults, primary_class.as_ref()),
        load_fire_mode(set, defaults, mode_class(1).as_ref()),
    ];
    // Ammo, aimed animation and firing effects come from the primary mode.
    let mut ammo = None;
    let mut ammo_class: Option<crate::game::perks::ClassChain> = None;
    let mut fx = FireFx::default();
    let mut shell_bone_name = None;
    let mut shell2_bone_name = None;
    if let Some(fm_class) = &primary_class {
        let fget = |p: &str| defaults.get(fm_class, p);
        let class_of = |p: &str| match fget(p) {
            Some((Value::Object(r), rp)) => set.resolve(&rp, r).map(|h| h.path()),
            _ => None,
        };
        fx.flash_class = class_of("FlashEmitterClass");
        fx.shell_class = class_of("ShellEjectClass");
        if let Some((Value::Name(n), np)) = fget("ShellEjectBoneName") {
            shell_bone_name = Some(np.pkg.name(n).to_string());
        }
        if let Some((Value::Name(n), np)) = fget("ShellEject2BoneName") {
            shell2_bone_name = Some(np.pkg.name(n).to_string());
        }
        // Ammo: magazine size from the weapon, starting total from the ammo class.
        if let Some((Value::Object(ac), ac_pkg)) = fget("AmmoClass")
            && let Some(ammo_class_h) = set.resolve(&ac_pkg, ac)
        {
            let ammo_class = &mut ammo_class;
            let initial = match defaults.get(&ammo_class_h, "InitialAmount") {
                Some((Value::Int(i), _)) => i.max(0) as u32,
                _ => 0,
            };
            let capacity = match get("MagCapacity") {
                Some((Value::Int(i), _)) => i.max(1) as u32,
                _ => 1,
            };
            let max_total = match defaults.get(&ammo_class_h, "MaxAmmo") {
                Some((Value::Int(i), _)) => i.max(0) as u32,
                _ => initial,
            };
            let mag = capacity.min(initial);
            ammo = Some(Ammo {
                mag,
                spare: initial - mag,
                capacity,
                initial,
                max_total,
                default_capacity: capacity,
                default_max: max_total,
            });
            *ammo_class = Some(crate::game::perks::ClassChain::new(defaults.chain_names(&ammo_class_h)));
        }
    }

    // Skins override the mesh's texture slots.
    let skins: Vec<ObjectRef> = match get("Skins") {
        Some((Value::Array { count, raw }, _)) => {
            let mut r = ue_assets::reader::Reader::new(&raw);
            (0..count)
                .filter_map(|_| r.compact_index().ok().map(ObjectRef::from_raw))
                .collect()
        }
        _ => Vec::new(),
    };
    let skins_pkg = get("Skins").map(|(_, p)| p);
    // SkinRefs: material paths (KFWeapon.PreloadAssets), replacing Skins.
    let named: Vec<Option<ObjectHandle>> = match get("SkinRefs") {
        Some((Value::Array { count, raw }, _)) => {
            let mut r = ue_assets::reader::Reader::new(&raw);
            (0..count)
                .map(|_| {
                    let path = r.fstring().ok().filter(|p| !p.is_empty())?;
                    // A path that does not resolve leaves the mesh's own
                    // material (AK47: SkinRefs says "Rifles.AK47_cmb", the
                    // package has "Rifle.AK47_cmb").
                    let h = set.find_object(&path, None);
                    if h.is_none() {
                        runlog::kv("weapon_skin_missing", &format!("class={class_path} path={path}"));
                    }
                    h
                })
                .collect()
        }
        _ => Vec::new(),
    };

    // KFWeapon.BringUp -> HandleSleeveSwapping: Skins[SleeveNum] = the
    // character's SleeveTexture (after PreloadAssets set the SkinRefs).
    let mut named = named;
    if let Some(sleeve) = sleeve {
        let slot = int("SleeveNum", 1).max(0) as usize;
        if named.len() <= slot {
            named.resize(slot + 1, None);
        }
        named[slot] = Some(sleeve.clone());
        runlog::kv("weapon_sleeve", &format!("class={class_path} sleeve_num={slot} texture={}", sleeve.path()));
    } else {
        runlog::kv("weapon_sleeve", &format!("class={class_path} sleeve_num=none texture=weapon_default"));
    }
    let model = SkinnedModel::load(
        set,
        &mesh_h,
        &Skins {
            refs: skins,
            package: skins_pkg,
            named,
        },
        false,
        meshes,
        images,
        materials,
    )?;
    // KFWeapon: aiming plays IdleAimAnim / FireAimedAnim (falling back to the
    // normal ones when missing, as KFFire.PlayFiring does).
    let iron = has_aiming.then(|| IronSights {
        player_fov: float("PlayerIronSightFOV", 75.0),
        display_fov: float("ZoomedDisplayFOV", display_fov),
        zoom_time: float("ZoomTime", 0.25),
        fast_zoom_out_time: float("FastZoomOutTime", 0.2),
        idle_anim: name("IdleAimAnim").unwrap_or_else(|| "Idle".into()),
    });
    // Dual pistols (DualiesFire.FlashMuzzleFlash): the right shot flashes at
    // FlashBoneName and ejects at ShellEject2BoneName, the left shot at
    // altFlashBoneName and ShellEjectBoneName.
    let dual = defaults.is_a(&class, "Dualies");
    let bone = |n: Option<String>| n.and_then(|n| model.find_bone(&n));
    fx.hands = if dual {
        vec![
            FxHand {
                flash_bone: bone(name("FlashBoneName")),
                shell_bone: bone(shell2_bone_name.clone()),
                ..default()
            },
            FxHand {
                flash_bone: bone(name("altFlashBoneName")),
                shell_bone: bone(shell_bone_name.clone()),
                ..default()
            },
        ]
    } else {
        vec![FxHand {
            flash_bone: bone(name("FlashBoneName")),
            shell_bone: bone(shell_bone_name.clone()),
            ..default()
        }]
    };
    // KFWeapon.PostBeginPlay: no bHasScope, no scope.
    let scope = if matches!(get("bHasScope"), Some((Value::Bool(true), _))) {
        let lens_id = int("lenseMaterialID", 0).max(0) as usize;
        let reticle = SCOPE_RETICLES
            .iter()
            .find(|(c, _)| class_name.eq_ignore_ascii_case(c))
            .and_then(|(_, path)| set.find_object(path, Some("Texture")))
            .and_then(|t| crate::render::skinned::decode_image(&t, images));
        let lens_part = model.parts.iter().position(|p| p.material_index == lens_id);
        // The lens's UV range: the scope image is mapped through it.
        let (uv_lo, uv_hi) = model
            .mesh
            .triangles
            .iter()
            .filter(|t| t.material == lens_id)
            .flat_map(|t| t.wedges.iter().map(|&wi| Vec2::from_array(model.mesh.wedges[wi].uv)))
            .fold((Vec2::MAX, Vec2::MIN), |(lo, hi), uv| (lo.min(uv), hi.max(uv)));
        runlog::kv("weapon_scope_lens_uv", &format!("class={class_path} uv_min={uv_lo:?} uv_max={uv_hi:?}"));
        runlog::kv(
            "weapon_scope",
            &format!(
                "class={class_path} lense_material_id={lens_id} lens_part={lens_part:?} portal_fov={} reticle={}",
                float("scopePortalFOV", 12.0),
                reticle.is_some()
            ),
        );
        lens_part.map(|lens_part| WeaponScope {
            lens_part,
            portal_fov: float("scopePortalFOV", 12.0),
            reticle,
        })
    } else {
        None
    };
    // bTorchEnabled (own or inherited): the flashlight (torch.rs).
    let torch = matches!(get("bTorchEnabled"), Some((Value::Bool(true), _))).then(|| {
        let offset = match get("FirstPersonFlashlightOffset") {
            Some((Value::Vector(v), _)) => v,
            _ => [0.0; 3],
        };
        let t = TorchDef::new(&model, offset, name("ModeSwitchAnim"), |c| defaults.is_a(&class, c));
        t.log(class_path);
        t
    });
    let alt_ammo_h = mode_class(1).and_then(|c| match defaults.get(&c, "AmmoClass") {
        Some((Value::Object(r), rp)) if r != ObjectRef::Null => set.resolve(&rp, r),
        _ => None,
    });
    let alt_ammo_class = alt_ammo_h.as_ref().map(|h| crate::game::perks::ClassChain::new(defaults.chain_names(h)));
    let mut def = WeaponDef {
        perk: crate::game::perks::PerkWeapon {
            class: crate::game::perks::ClassChain::new(defaults.chain_names(&class)),
        },
        ammo_class,
        alt_ammo_class,
        alt_default_max: 0,
        speed_me_up,
        base_reload_rate: float("ReloadRate", 1.0),
        base_reload_anim_rate: float("ReloadAnimRate", 1.0),
        torch,
        class: class_path.to_string(),
        item_name,
        group,
        group_offset,
        priority,
        weight,
        modes,
        model,
        entities: Vec::new(),
        sleeve_num: int("SleeveNum", 1).max(0) as usize,
        view_offset: coords::pos(view_offset),
        display_fov,
        iron,
        speed_bonus,
        bob_damping,
        // The Welder's ammo is its fuel (weld_fuel), not a magazine.
        ammo: if defaults.is_a(&class, "Welder") { None } else { ammo },
        weld_fuel: defaults.is_a(&class, "Welder").then(|| {
            let cost = |i: u32, d: u32| match mode_class(i).and_then(|c| defaults.get(&c, "AmmoPerFire")) {
                Some((Value::Int(n), _)) => n.max(0) as u32,
                _ => d,
            };
            WeldFuel {
                amount: ammo.map_or(300, |a| a.initial),
                max: ammo.map_or(300, |a| a.max_total),
                regen_rate: float("AmmoRegenRate", 40.0),
                regen_count: 0.0,
                cost: [cost(0, 20), cost(1, 15)],
            }
        }),
        select_anim: name("SelectAnim").unwrap_or_else(|| "Select".into()).to_ascii_lowercase(),
        select_anim_rate: float("SelectAnimRate", 1.3636),
        bring_up_time: float("BringUpTime", 0.33),
        put_down_anim: name("PutDownAnim").unwrap_or_else(|| "PutDown".into()).to_ascii_lowercase(),
        put_down_anim_rate: float("PutDownAnimRate", 1.3636),
        put_down_time: float("PutDownTime", 0.33),
        idle_anim: name("IdleAnim").unwrap_or_else(|| "Idle".into()).to_ascii_lowercase(),
        reload_anim: name("ReloadAnim").unwrap_or_else(|| "Reload".into()).to_ascii_lowercase(),
        reload_anim_rate: float("ReloadAnimRate", 1.0),
        reload_rate: float("ReloadRate", 1.0),
        hold_to_reload: matches!(get("bHoldToReload"), Some((Value::Bool(true), _))),
        can_dry_fire: matches!(get("bModeZeroCanDryFire"), Some((Value::Bool(true), _))),
        select_sound: sound_prop(defaults, &class, "SelectSound"),
        select_volume: float("TransientSoundVolume", 0.3),
        throw_sound: sound_prop(defaults, &class, "ThrowSound"),
        pickup_sound: match get("PickupClass") {
            Some((Value::Object(r), rp)) if r != ObjectRef::Null => set.resolve(&rp, r).and_then(|pc| sound_prop(defaults, &pc, "PickupSound")),
            _ => None,
        },
        idle_ambient: match get("AttachmentClass") {
            Some((Value::Object(r), rp)) if r != ObjectRef::Null => set.resolve(&rp, r).and_then(|a| {
                let sound = sound_prop(defaults, &a, "AmbientSound")?;
                Some(AmbientSound {
                    sound,
                    volume: match defaults.get(&a, "SoundVolume") {
                        Some((Value::Byte(b), _)) => b,
                        _ => 128,
                    },
                    radius: match defaults.get(&a, "SoundRadius") {
                        Some((Value::Float(f), _)) => f,
                        _ => 64.0,
                    },
                    pitch: 64,
                    at_listener: true,
                    ..default()
                })
            }),
            _ => None,
        },
        scope,
        dual,
        toggles_on_alt: if class_name.eq_ignore_ascii_case("KSGShotgun") {
            Some(AltToggle::WideSpread)
        } else {
            TOGGLE_ON_ALT_FIRE.iter().any(|c| class_name.eq_ignore_ascii_case(c)).then_some(AltToggle::FireMode)
        },
        wide_spread: false,
        gone: false,
        sell_value: None,
        never_throw: matches!(get("bKFNeverThrow"), Some((Value::Bool(true), _))),
        toss: name("TossAnim").map(|a| (a.to_ascii_lowercase(), float("TossTime", 0.366), float("TossSpawnTime", 0.2))),
        heal_charge: (defaults.is_a(&class, "Syringe") || defaults.is_a(&class, "KFMedicGun")).then(|| {
            let syringe = defaults.is_a(&class, "Syringe");
            let mode_float = |i: u32, p: &str, d: f32| match mode_class(i).and_then(|c| defaults.get(&c, p)) {
                Some((Value::Float(f), _)) => f,
                Some((Value::Int(n), _)) => n as f32,
                _ => d,
            };
            HealCharge {
                charge: HEAL_CHARGE_MAX,
                regen_rate: float("AmmoRegenRate", 0.3),
                next_regen: 0.0,
                // Syringe.PostBeginPlay: 50 with one player; medic guns:
                // HealBoostAmount (their darts' own value is what heals).
                boost: if syringe { 50.0 } else { float("HealBoostAmount", 20.0) },
                syringe,
                cost: [mode_float(0, "AmmoPerFire", 250.0) as u32, mode_float(1, "AmmoPerFire", 500.0) as u32],
                inject_delay: [mode_float(0, "InjectDelay", 0.36), mode_float(1, "InjectDelay", 0.1)],
            }
        }),
        quick_put_down_time: float("QuickPutDownTime", 0.15),
        quick_bring_up_time: float("QuickBringUpTime", 0.15),
        alt_ammo: {
            let ammo_class_of = |c: Option<&ObjectHandle>| {
                let c = c?;
                match defaults.get(c, "AmmoClass") {
                    Some((Value::Object(r), rp)) if r != ObjectRef::Null => set.resolve(&rp, r),
                    _ => None,
                }
            };
            let alt_class = mode_class(1);
            match (ammo_class_of(primary_class.as_ref()), ammo_class_of(alt_class.as_ref())) {
                (primary, Some(alt)) if primary.as_ref().is_none_or(|p| p.path() != alt.path()) => {
                    let int = |p: &str| match defaults.get(&alt, p) {
                        Some((Value::Int(i), _)) => i.max(0) as u32,
                        _ => 0,
                    };
                    Some((int("InitialAmount"), int("MaxAmmo")))
                }
                _ => None,
            }
        },
        chop_slow_rate: if defaults.is_a(&class, "KFMeleeGun") { float("ChopSlowRate", 0.5) } else { 1.0 },
        boomstick_reload: defaults.is_a(&class, "BoomStick").then(|| float("ReloadCountDown", 2.5)),
        fx,
    };
    def.alt_default_max = def.alt_ammo.map_or(0, |a| a.1);
    Ok(def)
}

/// The weapon's model parts, hidden, under the weapon camera.
pub(super) fn spawn_parts(commands: &mut Commands, def: &mut WeaponDef, cam: Entity) {
    for part in &def.model.parts {
        let e = commands
            .spawn((
                Mesh3d(part.mesh.clone()),
                MeshMaterial3d(part.material.clone()),
                Transform::from_translation(def.view_offset * draw_offset_factor(def.display_fov)),
                RenderLayers::layer(WEAPON_LAYER),
                Visibility::Hidden,
                ChildOf(cam),
            ))
            .id();
        def.entities.push(e);
    }
}
