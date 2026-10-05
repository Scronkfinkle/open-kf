//! First-person weapons: skeletal mesh + animations, CPU skinning, a separate
//! weapon camera with the weapon's own field of view (like UE2's DisplayFOV,
//! which also keeps the weapon from clipping into walls).
//!
//! Mesh loading, animation sampling and skinning live in `skinned.rs`.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;

use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package::ObjectRef;
use ue_assets::package_set::{ObjectHandle, PackageSet};
use ue_assets::properties::Value;

use crate::camera::FlyCamera;
use crate::coords;
use crate::map::MapRequest;
use crate::runlog;
use crate::combat::{MeleeSwing, ShotFired};
use crate::skinned::{SkinnedModel, Skins};

/// Render layer seen only by the weapon camera.
pub const WEAPON_LAYER: usize = 2;

/// KF's starting inventory (KFHumanPawn RequiredEquipment), in that order.
const STARTING_WEAPONS: [&str; 5] = ["KFMod.Knife", "KFMod.Single", "KFMod.Frag", "KFMod.Syringe", "KFMod.Welder"];

/// Every base-game weapon (no DLC: no Steam AppID on the class; no
/// deathmatch / story variants or leftovers). How the list was chosen:
/// docs/DESIGN.md, "Weapons".
pub const BASE_WEAPONS: [&str; 48] = [
    // Melee
    "Knife", "Machete", "Axe", "Katana", "ClaymoreSword", "Chainsaw",
    // Pistols
    "Single", "Dualies", "Deagle", "DualDeagle", "Magnum44Pistol", "Dual44Magnum", "MK23Pistol", "DualMK23Pistol",
    // Rifles and SMGs
    "Winchester", "Bullpup", "AK47AssaultRifle", "SCARMK17AssaultRifle", "M4AssaultRifle", "MKb42AssaultRifle",
    "FNFAL_ACOG_AssaultRifle", "M14EBRBattleRifle", "MAC10MP", "MP7MMedicGun", "MP5MMedicGun", "M7A3MMedicGun",
    "KrissMMedicGun",
    // Shotguns
    "Shotgun", "BoomStick", "AA12AutoShotgun", "BenelliShotgun", "KSGShotgun", "Trenchgun", "NailGun",
    // Projectiles and explosives
    "Crossbow", "M99SniperRifle", "LAW", "M79GrenadeLauncher", "M32GrenadeLauncher", "M4203AssaultRifle", "Frag",
    "PipeBombExplosive", "HuskGun",
    // Fire, equipment, special
    "FlameThrower", "Syringe", "Welder", "ZEDGun", "ZEDMKIIWeapon",
];

/// Weapons whose AltFire calls DoToggle (full / semi auto), from their
/// scripts. Exact classes: the M4 203 extends the M4 but overrides AltFire.
const TOGGLE_ON_ALT_FIRE: [&str; 9] = [
    "AA12AutoShotgun",
    "AK47AssaultRifle",
    "Bullpup",
    "FNFAL_ACOG_AssaultRifle",
    "KSGShotgun",
    "M4AssaultRifle",
    "MAC10MP",
    "MKb42AssaultRifle",
    "SCARMK17AssaultRifle",
];

/// KFHumanPawn MaxCarryWeight and WeightSpeedModifier.
const MAX_CARRY_WEIGHT: f32 = 15.0;
const WEIGHT_SPEED_MODIFIER: f32 = 0.13;

/// Extra weapons to carry (test flag `--give`), as class names.
#[derive(Resource, Default, Clone)]
pub struct WeaponLoadout {
    pub give: Vec<String>,
}

impl WeaponLoadout {
    /// `--give all` or `--give AK47AssaultRifle,Shotgun` (with or without "KFMod.").
    pub fn parse(arg: &str) -> Self {
        let names: Vec<&str> = if arg.eq_ignore_ascii_case("all") {
            BASE_WEAPONS.to_vec()
        } else {
            arg.split(',').map(str::trim).filter(|n| !n.is_empty()).collect()
        };
        WeaponLoadout {
            give: names
                .into_iter()
                .map(|n| if n.contains('.') { n.to_string() } else { format!("KFMod.{n}") })
                .collect(),
        }
    }
}

pub struct WeaponPlugin;

impl Plugin for WeaponPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScriptedInput>()
            .init_resource::<WeaponLoadout>()
            .init_resource::<crate::firing::Recoil>()
            .add_systems(
                Update,
                crate::firing::apply_recoil
                    .after(crate::camera::look)
                    .before(crate::camera::follow_sky),
            )
            .add_systems(PostStartup, load_weapons.after(crate::camera::spawn_camera))
            .add_systems(
                Update,
                (weapon_input, animate_weapon, weapon_fire_fx)
                    .chain()
                    .after(crate::camera::follow_sky),
            );
    }
}

/// Scripted input for tests: at frame N do an action ("fire", "fire_down" /
/// "fire_up" = hold / release, the same for "altfire", "1" to "5" =
/// weapon slot keys, "next" / "prev" = mouse wheel, "reload", "aim" = toggle iron sights, "zed" = spawn a Clot, "zed_drop" =
/// spawn one 200 units up, "gorefast" = spawn a Gorefast,
/// "gorefast_far" = one 900 units away, "cycle_zed" = press N,
/// "spawn_<kind>" = spawn that zed, "hurt_zeds" = 100 damage to every zed).
#[derive(Resource, Default, Clone)]
pub struct ScriptedInput(pub Vec<(u32, String)>);

/// Speed bonus etc. that other systems read (walking uses `ground_speed_bonus`).
#[derive(Resource)]
pub struct WeaponEffects {
    /// Added to GroundSpeed, in Unreal units/s (KFHumanPawn InventorySpeedModifier).
    pub ground_speed_bonus: f32,
    /// GroundSpeed multiplier from the carried weight (KFHumanPawn
    /// ModifyVelocity WeightMod); 1 when carrying nothing.
    pub weight_speed_mult: f32,
    /// A shot this frame scales the horizontal velocity (KFFire.ModeDoFire:
    /// x 0.1, or x 0.5 for FireRate <= 0.25); walking applies and clears it.
    pub fire_velocity_scale: Option<f32>,
}

impl Default for WeaponEffects {
    fn default() -> Self {
        WeaponEffects {
            ground_speed_bonus: 0.0,
            weight_speed_mult: 1.0,
            fire_velocity_scale: None,
        }
    }
}

struct WeaponDef {
    class: String,
    /// ItemName, for logs and the HUD.
    item_name: &'static str,
    /// Slot (InventoryGroup: 1 melee, 2 pistols, 3 primary, 4 specials,
    /// 5 equipment, 0 the grenade) and the order inside it (GroupOffset).
    group: u8,
    group_offset: i32,
    /// Weapon Priority: the order inside a group in the inventory list.
    priority: i32,
    weight: f32,
    /// Fire modes: [0] primary (left mouse), [1] alt fire (middle mouse).
    modes: [FireMode; 2],
    model: SkinnedModel,
    entities: Vec<Entity>,
    /// PlayerViewOffset, Bevy space, before the CalcDrawOffset FOV factor.
    view_offset: Vec3,
    display_fov: f32,
    /// Iron sights (bHasAimingMode); None for weapons without them.
    iron: Option<IronSights>,
    speed_bonus: f32,
    /// Inventory BobDamping (9mm 6, knife 8).
    bob_damping: f32,
    /// Magazine and spare rounds, for weapons that use ammo.
    ammo: Option<Ammo>,
    /// Reloading (KFWeapon): ReloadAnim at ReloadAnimRate; the magazine is
    /// filled ReloadRate seconds after the start, or with bHoldToReload one
    /// round every ReloadRate seconds.
    reload_anim: String,
    reload_anim_rate: f32,
    reload_rate: f32,
    hold_to_reload: bool,
    /// Switching (Weapon.BringUp / PutDown): SelectAnim and PutDownAnim at
    /// their rates; ready BringUpTime after the select starts, gone
    /// PutDownTime after the put-down starts. IdleAnim for PlayIdle.
    select_anim: String,
    select_anim_rate: f32,
    bring_up_time: f32,
    put_down_anim: String,
    put_down_anim_rate: f32,
    put_down_time: f32,
    idle_anim: String,
    /// bModeZeroCanDryFire: clicking with an empty magazine starts a reload.
    can_dry_fire: bool,
    /// Alt fire switches full / semi auto (the class's AltFire calls DoToggle).
    toggles_on_alt: bool,
    /// First-person firing effects (KFFire.InitEffects).
    fx: FireFx,
    /// The 3D scope (bHasScope), if any.
    scope: Option<WeaponScope>,
}

/// A scoped weapon's lens (Crossbow / M99SniperRifle): the model part that
/// shows the scope view, its zoom and reticle.
struct WeaponScope {
    /// Index into `entities` / `model.parts` of the lens part
    /// (Skins[lenseMaterialID]).
    lens_part: usize,
    /// scopePortalFOV, degrees.
    portal_fov: f32,
    /// The Combiner's Material1, set in UpdateScopeMode.
    reticle: Option<Handle<Image>>,
}

/// UpdateScopeMode's reticle texture for each scoped class (KF_ModelScope).
const SCOPE_RETICLES: [(&str, &str); 2] = [
    ("Crossbow", "KillingFloorWeapons.Xbow.CommandoCross"),
    ("M99SniperRifle", "KF_Weapons5_Scopes_Trip_T.Scope.MilDot"),
];

/// The fire mode's first-person effects: FlashEmitterClass on the weapon's
/// FlashBoneName, ShellEjectClass on ShellEjectBoneName.
#[derive(Default)]
struct FireFx {
    flash_class: Option<String>,
    flash_bone: Option<usize>,
    shell_class: Option<String>,
    shell_bone: Option<usize>,
    /// The spawned effects (spawned once the effect library is loaded).
    flash: Option<Entity>,
    shell: Option<Entity>,
    spawn_tried: bool,
}

/// Iron sight values from the weapon and fire mode classes.
#[derive(Clone, Debug)]
struct IronSights {
    /// Player FOV while aiming (PlayerIronSightFOV), horizontal at 4:3.
    player_fov: f32,
    /// Weapon DisplayFOV while aiming (ZoomedDisplayFOV).
    display_fov: f32,
    /// Seconds to zoom in or out (ZoomTime).
    zoom_time: f32,
    /// Seconds for the quick zoom out when reloading or switching (FastZoomOutTime).
    fast_zoom_out_time: f32,
    /// Idle animation while aiming (IdleAimAnim).
    idle_anim: String,
}

/// One fire mode (a WeaponFire class): what it does, its animations and timing.
#[derive(Clone, Debug)]
struct FireMode {
    kind: FireKind,
    /// The class path, for logs ("None" if the weapon has no such mode).
    class: String,
    /// FireAnims (one per shot, in turn) or FireAnim.
    anims: Vec<String>,
    anim_rate: f32,
    /// KFFire.PlayFiring / PlayFireEnd animations (lowercase; "none" when
    /// unset): FireAimedAnim, FireLoopAnim, FireLoopAimedAnim, FireEndAnim,
    /// FireEndAimedAnim, with FireLoopAnimRate and FireEndAnimRate.
    aimed_anim: String,
    loop_anim: String,
    loop_aimed_anim: String,
    end_anim: String,
    end_aimed_anim: String,
    loop_anim_rate: f32,
    end_anim_rate: f32,
    /// Seconds between shots (FireRate).
    rate: f32,
    /// bWaitForRelease: one shot per click.
    wait_for_release: bool,
    /// KFHighROFFire: in full auto the loop animation runs while held
    /// (state FireLoop) instead of one animation per shot.
    high_rof: bool,
    /// WinchesterFire / KFShotgunFire.AllowFire: may fire during a reload
    /// once 2 rounds are in (which interrupts a one-by-one reload).
    fire_while_reloading: bool,
    /// !bFiringDoesntAffectMovement: shots slow the player (KFFire.ModeDoFire).
    slows_movement: bool,
    spread: crate::firing::SpreadParams,
    recoil: crate::firing::RecoilParams,
    combat: CombatStats,
}

/// Reads a fire mode class's defaults (KFMeleeFire, KFFire, BaseProjectileFire...).
fn load_fire_mode(set: &PackageSet, defaults: &ClassDefaults, fm_class: Option<&ObjectHandle>) -> FireMode {
    let mut mode = FireMode {
        kind: FireKind::None,
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
        rate: 0.5,
        wait_for_release: false,
        high_rof: false,
        fire_while_reloading: false,
        slows_movement: false,
        spread: Default::default(),
        recoil: Default::default(),
        combat: CombatStats {
            headshot_mult: 1.0,
            ..default()
        },
    };
    let Some(fm_class) = fm_class else {
        return mode;
    };
    mode.class = fm_class.path();
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
    // (WeldFire.Timer looks for a KFDoorMover); doors are not done.
    let welds = mode.class.contains("WeldFire");
    mode.kind = if welds {
        FireKind::None
    } else if combat.melee {
        FireKind::Melee
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
    mode.high_rof = defaults.is_a(fm_class, "KFHighROFFire");
    mode.fire_while_reloading = defaults.is_a(fm_class, "WinchesterFire") || defaults.is_a(fm_class, "KFShotgunFire");
    // Only KFFire.ModeDoFire slows the player (shotguns: W4).
    mode.slows_movement = defaults.is_a(fm_class, "KFFire") && !fbool("bFiringDoesntAffectMovement");
    mode.spread = crate::firing::SpreadParams {
        spread: mode.combat.spread,
        max_spread: ffloat("MaxSpread", 0.0),
        semi_auto_bonus: fbool("bAccuracyBonusForSemiAuto"),
    };
    mode.recoil = crate::firing::RecoilParams {
        rate: ffloat("RecoilRate", 0.09),
        max_vertical: ffloat("maxVerticalRecoilAngle", 0.0),
        max_horizontal: ffloat("maxHorizontalRecoilAngle", 0.0),
        right_only: fbool("bRecoilRightOnly"),
        velocity_scale: ffloat("RecoilVelocityScale", 0.0),
    };
    mode
}

/// How a fire mode works, from its class defaults.
#[derive(Clone, Copy, PartialEq, Debug)]
enum FireKind {
    /// KFMeleeFire: MeleeDamage after a delay, in a cone.
    Melee,
    /// InstantFire / KFFire: a hit trace (KFFire.DoTrace).
    Instant,
    /// BaseProjectileFire: spawns ProjectileClass (not done yet: W4, W6).
    Projectile,
    /// No damage (NoFire, or a fire class not understood).
    None,
}

/// Damage values from the fire mode and damage type classes.
#[derive(Clone, Copy, Debug, Default)]
struct CombatStats {
    melee: bool,
    damage_min: f32,
    damage_max: f32,
    spread: f32,
    headshot_mult: f32,
    range: f32,
    damage_delay: f32,
    min_dot: f32,
}

#[derive(Clone, Copy, Debug)]
struct Ammo {
    mag: u32,
    spare: u32,
    capacity: u32,
}

/// The weapon's state (KFWeapon ClientState and bIsReloading). Which
/// animation plays is separate: e.g. a reload continues after its animation
/// has gone back to idle.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Action {
    Select,
    /// Ready to fire (WS_ReadyToFire), whatever animation is playing.
    Idle,
    Reload,
    PutDown { next: usize },
}

#[derive(Resource)]
struct Weapons {
    defs: Vec<WeaponDef>,
    current: usize,
    action: Action,
    sequence: Option<usize>,
    /// Name of the playing animation (lowercase).
    anim: String,
    /// Current time in the sequence, in frames.
    frame: f32,
    /// Frames per second for the current sequence.
    play_rate: f32,
    looping: bool,
    /// Seconds until each fire mode may fire again (NextFireTime).
    fire_cooldown: [f32; 2],
    fire_count: usize,
    /// bIsFiring per mode: the button is down and StartFire succeeded.
    firing: [bool; 2],
    /// Shots since the button was pressed (WeaponFire.FireCount).
    shots_this_press: [u32; 2],
    /// KFFire burst tracking for the spread.
    spread_state: [crate::firing::SpreadState; 2],
    /// Seconds since the reload started or the last round went in.
    reload_timer: f32,
    /// Seconds left of the bring-up or put-down (Weapon's Timer), and
    /// whether the put-down is still waiting out DownDelay.
    switch_timer: f32,
    down_delayed: bool,
    /// Melee swings waiting for their damage moment: (seconds left,
    /// stats, weapon name). KFMeleeFire.ModeDoFire sets a timer per swing.
    pending_swings: Vec<(f32, CombatStats, &'static str)>,
    /// Bound for bWaitForRelease modes: a press not yet turned into a shot.
    press_waiting: [bool; 2],
    /// Iron sights wanted (bAimingRifle) and the zoom blend, 0 = hip, 1 = aimed.
    aiming: bool,
    zoom: f32,
    /// Seconds a full zoom transition takes right now.
    zoom_time: f32,
    /// Simple random number state (spread, damage rolls).
    rng: u64,
    /// Shots whose flash and shell are still to be triggered.
    fx_shots: u32,
    /// The current weapon's FlashBoneName frame, Unreal world (origin,
    /// axes), as last posed: where tracers start (KFWeapon.GetEffectStart).
    pub tip: Option<(Vec3, Mat3)>,
    shell_frame: Option<(Vec3, Mat3)>,
}

impl Weapons {
    /// A random number in [0, 1) (xorshift).
    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }
}

#[derive(Component)]
struct WeaponCamera;

#[allow(clippy::too_many_arguments)]
fn load_weapons(
    mut commands: Commands,
    request: Res<MapRequest>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    main_cam: Query<(Entity, &Transform), With<FlyCamera>>,
    loadout: Res<WeaponLoadout>,
) {
    let started = std::time::Instant::now();
    let set = PackageSet::new(&request.install_root);
    let defaults = ClassDefaults::new(&set);
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
                fov: crate::camera::vertical_fov(70.0),
                near: 0.01,
                ..default()
            }),
            *main_t,
            RenderLayers::layer(WEAPON_LAYER),
            WeaponCamera,
        ))
        .id();

    // The inventory: KF's starting weapons, then any given with --give.
    let mut classes: Vec<String> = STARTING_WEAPONS.iter().map(|c| c.to_string()).collect();
    for c in &loadout.give {
        if !classes.iter().any(|have| have.eq_ignore_ascii_case(c)) {
            classes.push(c.clone());
        }
    }
    let mut defs = Vec::new();
    let mut failed = Vec::new();
    for class_path in &classes {
        match load_weapon(&set, &defaults, class_path, &mut meshes, &mut images, &mut materials) {
            Ok(mut def) => {
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
    let weight: f32 = defs.iter().map(|d| d.weight).sum();
    runlog::kv(
        "inventory",
        &format!(
            "loaded={} failed={} failed_classes={:?} weight={weight} max_carry_weight={MAX_CARRY_WEIGHT} over_limit={} given_by_test_flag={} order={:?}",
            defs.len(),
            failed.len(),
            failed,
            weight > MAX_CARRY_WEIGHT,
            loadout.give.len(),
            defs.iter().map(|d| format!("{}:{}", d.group, d.item_name)).collect::<Vec<_>>()
        ),
    );
    if defs.is_empty() {
        return;
    }
    // Start with the 9mm, as KF does.
    let current = defs.iter().position(|d| d.class.eq_ignore_ascii_case("KFMod.Single")).unwrap_or(0);
    let mut w = Weapons {
        defs,
        current,
        action: Action::Select,
        sequence: None,
        anim: String::new(),
        frame: 0.0,
        play_rate: 30.0,
        looping: false,
        fire_cooldown: [0.0; 2],
        fire_count: 0,
        firing: [false; 2],
        shots_this_press: [0; 2],
        spread_state: Default::default(),
        reload_timer: 0.0,
        switch_timer: 0.0,
        down_delayed: false,
        pending_swings: Vec::new(),
        press_waiting: [false; 2],
        aiming: false,
        zoom: 0.0,
        zoom_time: 0.25,
        rng: 0x2545_F491_4F6C_DD1D,
        fx_shots: 0,
        tip: None,
        shell_frame: None,
    };
    set_action(&mut w, Action::Select);
    commands.insert_resource(w);
    // KFHumanPawn.ModifyVelocity: the weight counts up to MaxCarryWeight.
    let encumbrance = weight.min(MAX_CARRY_WEIGHT) / MAX_CARRY_WEIGHT;
    commands.insert_resource(WeaponEffects {
        ground_speed_bonus: 0.0,
        weight_speed_mult: 1.0 - encumbrance * WEIGHT_SPEED_MODIFIER,
        fire_velocity_scale: None,
    });
    runlog::kv("weapons_ready", &format!("seconds={:.2}", started.elapsed().as_secs_f64()));
}

fn load_weapon(
    set: &PackageSet,
    defaults: &ClassDefaults,
    class_path: &str,
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
    let speed_bonus = if matches!(get("bSpeedMeUp"), Some((Value::Bool(true), _))) {
        // KFHumanPawn: default.GroundSpeed * BaseMeleeIncrease - Weight * 2
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
    let mut fx = FireFx::default();
    let mut shell_bone_name = None;
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
        // Ammo: magazine size from the weapon, starting total from the ammo class.
        if let Some((Value::Object(ac), ac_pkg)) = fget("AmmoClass")
            && let Some(ammo_class) = set.resolve(&ac_pkg, ac)
        {
            let initial = match defaults.get(&ammo_class, "InitialAmount") {
                Some((Value::Int(i), _)) => i.max(0) as u32,
                _ => 0,
            };
            let capacity = match get("MagCapacity") {
                Some((Value::Int(i), _)) => i.max(1) as u32,
                _ => 1,
            };
            let mag = capacity.min(initial);
            ammo = Some(Ammo {
                mag,
                spare: initial - mag,
                capacity,
            });
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
    fx.flash_bone = name("FlashBoneName").and_then(|n| model.find_bone(&n));
    // KFWeapon.PostBeginPlay: no bHasScope, no scope.
    let scope = if matches!(get("bHasScope"), Some((Value::Bool(true), _))) {
        let lens_id = int("lenseMaterialID", 0).max(0) as usize;
        let reticle = SCOPE_RETICLES
            .iter()
            .find(|(c, _)| class_name.eq_ignore_ascii_case(c))
            .and_then(|(_, path)| set.find_object(path, Some("Texture")))
            .and_then(|t| crate::skinned::decode_image(&t, images));
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
    fx.shell_bone = shell_bone_name.and_then(|n| model.find_bone(&n));
    Ok(WeaponDef {
        class: class_path.to_string(),
        item_name,
        group,
        group_offset,
        priority,
        weight,
        modes,
        model,
        entities: Vec::new(),
        view_offset: coords::pos(view_offset),
        display_fov,
        iron,
        speed_bonus,
        bob_damping,
        ammo,
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
        scope,
        toggles_on_alt: TOGGLE_ON_ALT_FIRE.iter().any(|c| class_name.eq_ignore_ascii_case(c)),
        fx,
    })
}

/// UE2 Pawn.CalcDrawOffset scales PlayerViewOffset by 0.9 / DisplayFOV * 100.
fn draw_offset_factor(display_fov: f32) -> f32 {
    0.9 / display_fov * 100.0
}


/// What the inventory rules look at.
#[derive(Clone, Copy, Debug)]
struct Slot {
    group: u8,
    group_offset: i32,
    priority: i32,
    /// Every weapon can be held except the Frag, which is only thrown
    /// (Frag.WeaponChange / NextWeapon / PrevWeapon never pick it).
    selectable: bool,
}

impl WeaponDef {
    fn slot(&self) -> Slot {
        Slot {
            group: self.group,
            group_offset: self.group_offset,
            priority: self.priority,
            selectable: !self.class.eq_ignore_ascii_case("KFMod.Frag"),
        }
    }
}

fn slots(defs: &[WeaponDef]) -> Vec<Slot> {
    defs.iter().map(WeaponDef::slot).collect()
}

/// Pawn.AddInventory: a new weapon goes before the first weapon of its
/// group with a lower Priority, or right after the last of its group;
/// otherwise at the back of the list.
fn inventory_position(inv: &[Slot], new: Slot) -> usize {
    for (i, item) in inv.iter().enumerate() {
        if item.group == new.group {
            if item.priority < new.priority {
                return i;
            }
        } else if i > 0 && inv[i - 1].group == new.group {
            return i;
        }
    }
    inv.len()
}

/// Pawn.SwitchWeapon(F): the first weapon in group F after the current one
/// in the inventory, else the first from the start (so pressing the key
/// again cycles through the group).
fn switch_group(inv: &[Slot], current: usize, group: u8) -> Option<usize> {
    let n = inv.len();
    (1..=n)
        .map(|k| (current + k) % n)
        .find(|&i| inv[i].group == group && inv[i].selectable)
}

/// KFWeapon.NextWeapon / PrevWeapon: the next weapon by (InventoryGroup,
/// GroupOffset), wrapping around.
fn step_weapon(inv: &[Slot], from: usize, forward: bool) -> Option<usize> {
    let mut order: Vec<usize> = (0..inv.len()).filter(|&i| inv[i].selectable || i == from).collect();
    order.sort_by_key(|&i| (inv[i].group, inv[i].group_offset, i));
    let at = order.iter().position(|&i| i == from)?;
    let n = order.len();
    let next = if forward { order[(at + 1) % n] } else { order[(at + n - 1) % n] };
    (next != from).then_some(next)
}

/// Changes the weapon's state and plays its animation (Select, Idle,
/// Reload, PutDown).
fn set_action(w: &mut Weapons, action: Action) {
    w.action = action;
    let def = &w.defs[w.current];
    let (name, rate) = match action {
        Action::Select => {
            w.switch_timer = def.bring_up_time;
            (def.select_anim.clone(), def.select_anim_rate)
        }
        Action::Idle => return play_idle(w),
        Action::Reload => (def.reload_anim.clone(), def.reload_anim_rate),
        Action::PutDown { .. } => {
            // Weapon.PutDown: wait DownDelay first if a mode fired just now
            // (NextFireTime more than FireRate x (1 - MinReloadPct 0.5) away).
            let delay = (0..2)
                .map(|m| w.fire_cooldown[m] - def.modes[m].rate * 0.5)
                .fold(0.0f32, f32::max);
            if delay > 0.0 {
                w.switch_timer = delay;
                w.down_delayed = true;
                runlog::kv("weapon_action", &format!("weapon={} action={action:?} down_delay={delay:.3}", def.class));
                return;
            }
            w.switch_timer = def.put_down_time;
            (def.put_down_anim.clone(), def.put_down_anim_rate)
        }
    };
    play(w, &name, rate, false);
    runlog::kv("weapon_action", &format!("weapon={} action={action:?} anim={name}", w.defs[w.current].class));
}

/// Whether the weapon in hand has this animation (Actor.HasAnim).
fn has_anim(w: &Weapons, name: &str) -> bool {
    w.defs[w.current].model.sequence(name).is_some()
}

/// Plays an animation on the weapon in hand (PlayAnim / LoopAnim) at
/// `rate` x its own rate.
fn play(w: &mut Weapons, name: &str, rate: f32, looping: bool) {
    let def = &w.defs[w.current];
    w.sequence = def.model.sequence(name);
    w.anim = name.to_ascii_lowercase();
    w.frame = 0.0;
    w.looping = looping;
    w.play_rate = w.sequence.map_or(30.0, |s| def.model.rate(s)) * rate;
    if w.sequence.is_none() {
        runlog::kv("weapon_anim_missing", &format!("weapon={} anim={name}", def.class));
    } else if !looping || name != "idle" {
        runlog::kv("weapon_anim", &format!("weapon={} anim={} rate={rate} looping={looping}", def.item_name, w.anim));
    }
}

/// KFWeapon.PlayIdle: IdleAimAnim while aiming, else Idle, looping.
fn play_idle(w: &mut Weapons) {
    let name = match &w.defs[w.current].iron {
        Some(iron) if w.aiming => iron.idle_anim.to_ascii_lowercase(),
        _ => w.defs[w.current].idle_anim.clone(),
    };
    play(w, &name, 1.0, true);
}

/// KFFire.PlayFiring: the first shot after pressing plays FireAnim (aimed:
/// FireAimedAnim); later shots of the same press FireLoopAnim (aimed:
/// FireLoopAimedAnim, else FireAimedAnim), when the weapon has them.
/// Melee modes cycle through FireAnims.
fn play_firing(w: &mut Weapons, mode: usize) {
    let m = w.defs[w.current].modes[mode].clone();
    let fire = m.anims[w.fire_count % m.anims.len()].to_ascii_lowercase();
    let later = w.shots_this_press[mode] > 0;
    let (name, rate) = if later && w.aiming && has_anim(w, &m.loop_aimed_anim) {
        (m.loop_aimed_anim, m.loop_anim_rate)
    } else if w.aiming && has_anim(w, &m.aimed_anim) {
        (m.aimed_anim, m.anim_rate)
    } else if later && !w.aiming && has_anim(w, &m.loop_anim) {
        (m.loop_anim, m.loop_anim_rate)
    } else {
        (fire, m.anim_rate)
    };
    play(w, &name, rate, false);
}

/// KFFire.PlayFireEnd (Weapon.StopFire on release): FireEndAimedAnim while
/// aiming if there is one, else FireEndAnim, if the weapon has it.
/// KFHighROFFire plays it only in full auto.
fn play_fire_end(w: &mut Weapons, mode: usize) {
    let m = &w.defs[w.current].modes[mode];
    if m.high_rof && m.wait_for_release {
        return;
    }
    let (aimed, end, rate) = (m.end_aimed_anim.clone(), m.end_anim.clone(), m.end_anim_rate);
    if w.aiming && has_anim(w, &aimed) {
        play(w, &aimed, rate, false);
    } else if has_anim(w, &end) {
        play(w, &end, rate, false);
    }
}

/// KFWeapon.InterruptReload: only one-round-at-a-time reloads stop.
fn interrupt_reload(w: &mut Weapons, reason: &str) -> bool {
    if w.action == Action::Reload && w.defs[w.current].hold_to_reload {
        runlog::kv("reload_interrupted", &format!("weapon={} reason={reason}", w.defs[w.current].item_name));
        set_action(w, Action::Idle);
        return true;
    }
    false
}

/// KFWeapon.AllowReload: not while firing, reloading or bringing the weapon
/// up; the magazine not full; spare ammo; the next shot due within 0.1 s.
fn allow_reload(w: &Weapons) -> bool {
    w.defs[w.current].ammo.is_some_and(|a| a.mag < a.capacity && a.spare > 0)
        && !w.firing.iter().any(|&f| f)
        && w.action == Action::Idle
        && w.fire_cooldown[0] <= 0.1
}

/// KFWeapon.ReloadMeNow.
fn start_reload(w: &mut Weapons, reason: &str) {
    zoom_out(w, true, "reload");
    w.reload_timer = 0.0;
    set_action(w, Action::Reload);
    let def = &w.defs[w.current];
    runlog::kv(
        "reload_start",
        &format!(
            "weapon={} reason={reason} reload_rate={} hold_to_reload={} anim={} anim_seconds={:.2}",
            def.item_name,
            def.reload_rate,
            def.hold_to_reload,
            def.reload_anim,
            w.sequence.map_or(0.0, |s| def.model.length(s) / w.play_rate.max(1e-3))
        ),
    );
}

fn grabbed_now(cursor: &Query<&bevy::window::CursorOptions, With<bevy::window::PrimaryWindow>>) -> bool {
    cursor
        .single()
        .is_ok_and(|c| c.grab_mode != bevy::window::CursorGrabMode::None)
}

/// Leaves iron sights; `fast` uses FastZoomOutTime (reload, switch).
fn zoom_out(w: &mut Weapons, fast: bool, reason: &str) {
    if !w.aiming {
        return;
    }
    let cur = w.current;
    if let Some(iron) = &w.defs[cur].iron {
        w.zoom_time = if fast { iron.fast_zoom_out_time } else { iron.zoom_time };
    }
    w.aiming = false;
    runlog::kv("iron_sights", &format!("weapon={} aiming=false reason={reason}", w.defs[cur].class));
}

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn weapon_input(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Query<&bevy::window::CursorOptions, With<bevy::window::PrimaryWindow>>,
    weapons: Option<ResMut<Weapons>>,
    mut effects: Option<ResMut<WeaponEffects>>,
    script: Res<ScriptedInput>,
    scroll: Res<bevy::input::mouse::AccumulatedMouseScroll>,
    frames: Res<bevy::diagnostic::FrameCount>,
    main_cam: Query<(&Transform, Option<&crate::walk::Walker>), With<FlyCamera>>,
    mut shots: MessageWriter<ShotFired>,
    mut swings: MessageWriter<MeleeSwing>,
    mut ammo_display: ResMut<crate::combat::AmmoDisplay>,
    mut recoil: ResMut<crate::firing::Recoil>,
    health: Res<crate::combat::PlayerHealth>,
    mut scripted_held: Local<[bool; 2]>,
) {
    let Some(mut w) = weapons else {
        return;
    };
    let scripted = |action: &str| script.0.iter().any(|(f, a)| *f == frames.0 && a == action);
    // Time to each mode's NextFireTime; negative = overdue (kept, so the
    // next shot comes FireRate after the last was due, as
    // `NextFireTime += FireRate` does, and the average rate is exact).
    for cd in &mut w.fire_cooldown {
        *cd = (*cd - time.delta_secs()).max(-1.0);
    }
    // Weapon switching. Slot keys: Pawn.SwitchWeapon; mouse wheel:
    // KFHumanPawn.NextWeapon / PrevWeapon. While putting a weapon down, a
    // new choice replaces the pending one (and the wheel steps from it).
    let slot_keys = [
        (KeyCode::Digit1, 1u8, "1"),
        (KeyCode::Digit2, 2, "2"),
        (KeyCode::Digit3, 3, "3"),
        (KeyCode::Digit4, 4, "4"),
        (KeyCode::Digit5, 5, "5"),
    ];
    let mut choice = None;
    for (key, group, name) in slot_keys {
        if keys.just_pressed(key) || scripted(name) {
            choice = switch_group(&slots(&w.defs), w.current, group);
            runlog::kv("weapon_slot_key", &format!("group={group} choice={:?}", choice.map(|i| w.defs[i].item_name)));
        }
    }
    let wheel = scroll.delta.y;
    let pending = match w.action {
        Action::PutDown { next } => next,
        _ => w.current,
    };
    if wheel > 0.0 || scripted("next") {
        choice = step_weapon(&slots(&w.defs), pending, true);
    } else if wheel < 0.0 || scripted("prev") {
        choice = step_weapon(&slots(&w.defs), pending, false);
    }
    if let Some(next) = choice {
        match w.action {
            Action::PutDown { .. } => w.action = Action::PutDown { next },
            _ if next != w.current => {
                // KFWeapon.PutDown: a one-round reload is interrupted; any
                // other reload refuses the switch.
                interrupt_reload(&mut w, "switch");
                if w.action == Action::Reload {
                    runlog::kv("switch_refused", &format!("weapon={} reason=reloading", w.defs[w.current].item_name));
                } else {
                    zoom_out(&mut w, true, "switch");
                    w.pending_swings.clear();
                    w.firing = [false; 2];
                    set_action(&mut w, Action::PutDown { next });
                }
            }
            _ => {}
        }
    }
    // Iron sights: right mouse toggles (KFWeapon.ToggleIronSights). Not while
    // switching or in the air; a one-round reload is interrupted, any other
    // reload refuses.
    if (grabbed_now(&cursor) && mouse.just_pressed(MouseButton::Right)) || scripted("aim") {
        let cur = w.current;
        let in_air = main_cam.single().is_ok_and(|(_, walker)| walker.is_some_and(|wk| !wk.on_ground));
        if w.aiming {
            zoom_out(&mut w, false, "toggle");
        } else if w.defs[cur].iron.is_some() && !in_air && {
            interrupt_reload(&mut w, "aim");
            w.action == Action::Idle
        } {
            let iron = w.defs[cur].iron.as_ref().expect("checked");
            w.zoom_time = iron.zoom_time;
            w.aiming = true;
            runlog::kv("iron_sights", &format!("weapon={} aiming=true reason=toggle", w.defs[cur].class));
        } else {
            runlog::kv(
                "iron_sights_refused",
                &format!(
                    "weapon={} has_iron_sights={} action={:?} in_air={in_air}",
                    w.defs[cur].class,
                    w.defs[cur].iron.is_some(),
                    w.action
                ),
            );
        }
    }
    // Firing: left mouse = mode 0, middle mouse = mode 1 (KF's AltFire key).
    let now = time.elapsed_secs();
    let grabbed = grabbed_now(&cursor);
    let buttons = [(MouseButton::Left, "fire"), (MouseButton::Middle, "altfire")];
    // Scripted "fire_down" / "fire_up" (and "altfire_...") hold a button.
    let mut pressed_by_script = [false; 2];
    for (i, (_, name)) in buttons.iter().enumerate() {
        if scripted(&format!("{name}_down")) {
            scripted_held[i] = true;
            pressed_by_script[i] = true;
        }
        if scripted(&format!("{name}_up")) {
            scripted_held[i] = false;
        }
    }
    let held: [bool; 2] =
        std::array::from_fn(|i| (grabbed && mouse.pressed(buttons[i].0)) || scripted(buttons[i].1) || scripted_held[i]);
    let pressed: [bool; 2] = std::array::from_fn(|i| {
        (grabbed && mouse.just_pressed(buttons[i].0)) || scripted(buttons[i].1) || pressed_by_script[i]
    });
    for mode in 0..2 {
        let alt = 1 - mode;
        let cur = w.current;
        // Weapon.StopFire on release: the fire end animation.
        if !held[mode] {
            if w.firing[mode] {
                w.firing[mode] = false;
                if matches!(w.action, Action::Idle | Action::Reload) {
                    play_fire_end(&mut w, mode);
                }
            }
            w.press_waiting[mode] = false;
            continue;
        }
        if pressed[mode] {
            w.press_waiting[mode] = true;
        }
        let ready_state = matches!(w.action, Action::Idle | Action::Reload);
        // Rifles: alt fire switches full / semi auto (KFWeapon.DoToggle),
        // if ReadyToFire(0).
        if mode == 1 && w.defs[cur].toggles_on_alt {
            if pressed[mode] {
                let mag_ok = w.defs[cur].ammo.is_none_or(|a| a.mag >= 1);
                if ready_state && w.action != Action::Reload && mag_ok && w.fire_cooldown[0] <= 0.0 && !w.firing[0] {
                    let m = &mut w.defs[cur].modes[0];
                    m.wait_for_release = !m.wait_for_release;
                    let semi = m.wait_for_release;
                    runlog::kv("fire_mode_toggle", &format!("weapon={} semi_auto={semi}", w.defs[cur].item_name));
                } else {
                    runlog::kv("fire_mode_toggle_refused", &format!("weapon={} action={:?}", w.defs[cur].item_name, w.action));
                }
            }
            continue;
        }
        let fm = w.defs[cur].modes[mode].clone();
        if mode == 1 && fm.kind != FireKind::Melee {
            // Only melee alt attacks are done; other alt fires come with
            // their weapon family (DESIGN "Weapons").
            if pressed[mode] {
                runlog::kv(
                    "alt_fire_not_implemented",
                    &format!("weapon={} fire_kind={:?} fire_class={}", w.defs[cur].item_name, fm.kind, fm.class),
                );
            }
            continue;
        }
        // KFFire.AllowFire: not while reloading (WinchesterFire and
        // KFShotgunFire: unless 2+ rounds are in), not with an empty magazine.
        let mag = w.defs[cur].ammo.map(|a| a.mag);
        let reloading = w.action == Action::Reload;
        let allow_fire = (!reloading || (fm.fire_while_reloading && mag.is_some_and(|m| m >= 2)))
            && (mode == 1 || mag.is_none_or(|m| m >= 1));
        if !w.firing[mode] {
            // StartFire. bWaitForRelease modes need a fresh click (a click
            // made while not ready is kept while the button stays down; not
            // checked against KF). Weapon.ReadyToFire: not while the other
            // mode fires, and after both modes' NextFireTime.
            if fm.wait_for_release && !w.press_waiting[mode] {
                continue;
            }
            // KFWeapon.Fire: a click on an empty magazine asks for a reload.
            if mode == 0 && pressed[mode] && mag == Some(0) && !reloading && w.fire_cooldown[0] <= 0.0 {
                runlog::kv("dry_fire", &format!("weapon={} auto_reload={}", w.defs[cur].item_name, w.defs[cur].can_dry_fire));
                if w.defs[cur].can_dry_fire && allow_reload(&w) {
                    start_reload(&mut w, "dry_fire");
                }
                continue;
            }
            if !ready_state || w.firing[alt] || w.fire_cooldown[mode] > 0.0 || w.fire_cooldown[alt] > 0.0 || !allow_fire {
                continue;
            }
            w.firing[mode] = true;
            w.press_waiting[mode] = false;
            w.shots_this_press[mode] = 0;
            // StartFire: NextFireTime = now (no PreFireTime).
            w.fire_cooldown[mode] = 0.0;
            // KFWeapon.StartFire interrupts a one-round reload.
            interrupt_reload(&mut w, "fire");
            // KFHighROFFire in full auto: state FireLoop loops the animation.
            if fm.high_rof && !fm.wait_for_release {
                let (name, rate) = if w.aiming && has_anim(&w, &fm.loop_aimed_anim) {
                    (fm.loop_aimed_anim.clone(), fm.loop_anim_rate)
                } else {
                    (fm.loop_anim.clone(), fm.loop_anim_rate)
                };
                play(&mut w, &name, rate, true);
            }
        }
        // ModeDoFire: once per press in semi auto, every FireRate in full auto.
        if w.fire_cooldown[mode] > 0.0 || !allow_fire || (fm.wait_for_release && w.shots_this_press[mode] > 0) {
            // KFHighROFFire.ModeTick: an empty magazine ends the loop.
            if fm.high_rof && !fm.wait_for_release && !allow_fire && w.looping && w.firing[mode] {
                w.firing[mode] = false;
                play_fire_end(&mut w, mode);
            }
            continue;
        }
        // ModeDoFire: NextFireTime = max(NextFireTime + FireRate, now).
        w.fire_cooldown[mode] = (w.fire_cooldown[mode] + fm.rate).max(0.0);
        if mode == 0
            && let Some(a) = w.defs[cur].ammo.as_mut()
        {
            a.mag -= 1;
        }
        w.fire_count += 1;
        if mode == 0 {
            w.fx_shots += 1;
        }
        if !(fm.high_rof && !fm.wait_for_release) {
            play_firing(&mut w, mode);
        }
        w.shots_this_press[mode] += 1;
        let stats = fm.combat;
        let item_name = w.defs[cur].item_name;
        match fm.kind {
            FireKind::Melee => {
                runlog::kv(
                    "melee_swing",
                    &format!(
                        "weapon={item_name} mode={mode} class={} damage={} range={} delay={} min_dot={}",
                        fm.class, stats.damage_max, stats.range, stats.damage_delay, stats.min_dot
                    ),
                );
                w.pending_swings.push((stats.damage_delay, stats, item_name));
            }
            FireKind::Projectile | FireKind::None => runlog::kv(
                "fire_not_implemented",
                &format!("weapon={item_name} fire_kind={:?} fire_class={}", fm.kind, fm.class),
            ),
            FireKind::Instant => {
                if let Ok((cam, walker)) = main_cam.single() {
                    // KFFire.ModeDoFire: GetSpread, then InstantFire's
                    // direction; KFFire.DoTrace deals DamageMax.
                    let mut spread_state = w.spread_state[mode];
                    let spread = crate::firing::kf_spread(fm.spread, &mut spread_state, now, w.aiming, fm.wait_for_release);
                    w.spread_state[mode] = spread_state;
                    let vrand = loop {
                        let v = Vec3::new(w.random() * 2.0 - 1.0, w.random() * 2.0 - 1.0, w.random() * 2.0 - 1.0);
                        if v.length_squared() > 1e-4 && v.length_squared() <= 1.0 {
                            break v.normalize();
                        }
                    };
                    let frand = w.random();
                    let dir = crate::firing::spread_dir(*cam.forward(), spread, vrand, frand);
                    shots.write(ShotFired {
                        origin: cam.translation,
                        dir,
                        damage: stats.damage_max,
                        headshot_mult: stats.headshot_mult,
                        weapon: item_name,
                        effect_start: w.tip.map(|t| t.0),
                    });
                    // HandleRecoil; speed in Unreal units/s.
                    let speed = walker.map_or(0.0, |wk| wk.velocity.length() / coords::SCALE);
                    let r = [w.random(), w.random(), w.random()];
                    let kick = crate::firing::recoil_kick(fm.recoil, speed, health.health, 100.0, r);
                    recoil.add(kick, fm.recoil.rate, now);
                    // ModeDoFire slows the player unless falling.
                    let on_ground = walker.is_some_and(|wk| wk.on_ground);
                    if fm.slows_movement && on_ground {
                        let scale = if fm.rate > 0.25 { 0.1 } else { 0.5 };
                        if let Some(e) = effects.as_mut() {
                            e.fire_velocity_scale = Some(scale);
                        }
                    }
                    runlog::kv(
                        "gun_shot",
                        &format!(
                            "weapon={item_name} anim={} shot_in_press={} semi_auto={} aiming={} spread={spread:.4} burst={} recoil_pitch={:.0} recoil_yaw={:.0} speed_unreal={speed:.0} mag_left={}",
                            w.anim,
                            w.shots_this_press[mode],
                            fm.wait_for_release,
                            w.aiming,
                            spread_state.shots_in_burst,
                            kick.0,
                            kick.1,
                            w.defs[cur].ammo.map_or(0, |a| a.mag)
                        ),
                    );
                }
            }
        }
    }
    // Melee: damage lands DamagedelayMin seconds into the swing.
    let dt = time.delta_secs();
    let mut landed = Vec::new();
    w.pending_swings.retain_mut(|(t, stats, name)| {
        *t -= dt;
        if *t <= 0.0 {
            landed.push((*stats, *name));
        }
        *t > 0.0
    });
    for (stats, name) in landed {
        if let Ok((cam, _)) = main_cam.single() {
            swings.write(MeleeSwing {
                origin: cam.translation,
                dir: *cam.forward(),
                damage: stats.damage_min,
                range: stats.range,
                min_dot: stats.min_dot,
                headshot_mult: stats.headshot_mult,
                weapon: name,
            });
        }
    }
    // Reloading (KFWeapon.Tick): after ReloadRate the magazine is full and
    // the weapon idles (ClientFinishReloading), even if the reload
    // animation is still playing. One-round reloads add a round every
    // ReloadRate until full.
    // Weapon.Timer: the bring-up ends in idle; the put-down plays its
    // animation after any DownDelay, then the next weapon comes up.
    if matches!(w.action, Action::Select | Action::PutDown { .. }) {
        w.switch_timer -= dt;
        if w.switch_timer <= 0.0 {
            match w.action {
                Action::Select => set_action(&mut w, Action::Idle),
                Action::PutDown { next } if w.down_delayed => {
                    w.down_delayed = false;
                    let (name, rate, time) = {
                        let d = &w.defs[w.current];
                        (d.put_down_anim.clone(), d.put_down_anim_rate, d.put_down_time)
                    };
                    play(&mut w, &name, rate, false);
                    w.switch_timer = time;
                    w.action = Action::PutDown { next };
                }
                Action::PutDown { next } => {
                    w.current = next;
                    set_action(&mut w, Action::Select);
                }
                _ => {}
            }
        }
    }
    if keys.just_pressed(KeyCode::KeyR) || scripted("reload") {
        if allow_reload(&w) {
            start_reload(&mut w, "key");
        } else {
            runlog::kv("reload_refused", &format!("weapon={} action={:?} firing={:?}", w.defs[w.current].item_name, w.action, w.firing));
        }
    }
    if w.action == Action::Reload {
        w.reload_timer += dt;
        let cur = w.current;
        let (rate, hold, item_name) = (w.defs[cur].reload_rate, w.defs[cur].hold_to_reload, w.defs[cur].item_name);
        if w.reload_timer >= rate
            && let Some(a) = w.defs[cur].ammo.as_mut()
        {
            let n = if hold { 1.min(a.spare) } else { (a.capacity - a.mag).min(a.spare) };
            a.mag += n;
            a.spare -= n;
            let full = a.mag >= a.capacity || a.spare == 0;
            runlog::kv("reloaded", &format!("weapon={item_name} added={n} mag={} spare={} done={}", a.mag, a.spare, !hold || full));
            if hold && !full {
                w.reload_timer = 0.0;
            } else {
                set_action(&mut w, Action::Idle);
            }
        }
    }
    if let Some(e) = effects.as_mut() {
        e.ground_speed_bonus = w.defs[w.current].speed_bonus;
    }
    ammo_display.weapon = w.defs[w.current].item_name;
    ammo_display.ammo = w.defs[w.current].ammo.map(|a| (a.mag, a.spare));
    ammo_display.fire_mode = w.defs[w.current]
        .toggles_on_alt
        .then(|| if w.defs[w.current].modes[0].wait_for_release { "SEMI" } else { "AUTO" });
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)] // Bevy system parameters
fn animate_weapon(
    time: Res<Time>,
    weapons: Option<ResMut<Weapons>>,
    mut meshes: ResMut<Assets<Mesh>>,
    main_cam: Query<&Transform, (With<FlyCamera>, Without<WeaponCamera>)>,
    mut weapon_cam: Query<(&mut Transform, &mut Projection), With<WeaponCamera>>,
    mut visibility: Query<&mut Visibility>,
    mut weapon_parts: Query<&mut Transform, (Without<FlyCamera>, Without<WeaponCamera>)>,
    bob: Res<crate::walk::ViewBob>,
    mut view_fov: ResMut<crate::camera::ViewFov>,
    mut log_timer: Local<f32>,
    mut scope_request: ResMut<crate::scope::ScopeRequest>,
    scope_view: Option<Res<crate::scope::ScopeView>>,
    mut part_materials: Query<&mut MeshMaterial3d<StandardMaterial>>,
) {
    let Some(mut w) = weapons else {
        return;
    };
    let dt = time.delta_secs();

    // Iron sight zoom: blend toward the target over the zoom time. When a
    // zoom finishes while idle, switch between the hip and iron idles
    // (KFWeapon.OnZoomInFinished / OnZoomOutFinished).
    let target = if w.aiming { 1.0 } else { 0.0 };
    if w.zoom != target {
        let step = dt / w.zoom_time.max(1e-3);
        w.zoom = if w.aiming { (w.zoom + step).min(1.0) } else { (w.zoom - step).max(0.0) };
        if w.zoom == target {
            runlog::kv("iron_sights_zoom_done", &format!("aiming={}", w.aiming));
            // Swap the idle for the aimed idle (or back) if idling.
            if w.action == Action::Idle && w.looping && !w.firing.iter().any(|&f| f) {
                play_idle(&mut w);
            }
        }
    }
    let def = &w.defs[w.current];
    let (display_fov, player_fov) = match &def.iron {
        Some(iron) => (
            def.display_fov + (iron.display_fov - def.display_fov) * w.zoom,
            crate::camera::DEFAULT_FOV + (iron.player_fov - crate::camera::DEFAULT_FOV) * w.zoom,
        ),
        None => (def.display_fov, crate::camera::DEFAULT_FOV),
    };
    if view_fov.0 != player_fov {
        view_fov.0 = player_fov;
    }
    // Draw offset: CalcDrawOffset at the current DisplayFOV, blended to zero
    // when aimed (a guess; the native ironsight code is not in the scripts).
    let offset = def.view_offset * draw_offset_factor(display_fov) * (1.0 - w.zoom);

    // Weapon camera follows the main camera and uses this weapon's FOV.
    if let (Ok(main), Ok((mut wt, mut proj))) = (main_cam.single(), weapon_cam.single_mut()) {
        *wt = *main;
        if let Projection::Perspective(p) = proj.as_mut() {
            p.fov = crate::camera::vertical_fov(display_fov);
        }
    }

    // Advance the animation; finished one-shots lead to the next action.
    w.frame += dt * w.play_rate;
    let length = w.sequence.map_or(1.0, |s| w.defs[w.current].model.length(s));
    if w.frame >= length {
        if w.looping {
            w.frame %= length.max(1e-3);
        } else {
            match w.action {
                // Switching runs on timers (weapon_input); hold the last frame.
                Action::PutDown { .. } | Action::Select => w.frame = length,
                // Weapon.AnimEnd: after FireAnim comes FireEndAnim if the
                // weapon has it; otherwise idle unless a mode is firing
                // (then the last frame holds until the next shot).
                Action::Idle | Action::Reload => {
                    let next_end = (0..2).find_map(|m| {
                        let fm = &w.defs[w.current].modes[m];
                        (w.anim == fm.anims[0].to_ascii_lowercase() && has_anim(&w, &fm.end_anim))
                            .then(|| (fm.end_anim.clone(), fm.end_anim_rate))
                    });
                    if let Some((end, rate)) = next_end {
                        play(&mut w, &end, rate, false);
                    } else if !w.firing.iter().any(|&f| f) {
                        play_idle(&mut w);
                    } else {
                        w.frame = length;
                    }
                }
            }
        }
    }

    // Weapon bob (Pawn.WeaponBob): BobDamping x WalkBob sideways, and
    // (0.45 + 0.55 x BobDamping) x WalkBob vertically. The camera already
    // moved by WalkBob, so relative to it the weapon moves by the difference.
    let mut part_translation = offset;
    if let Ok(main) = main_cam.single() {
        let d = w.defs[w.current].bob_damping;
        let world = bob.side * (d - 1.0) + Vec3::Y * bob.up * (0.45 + 0.55 * d - 1.0);
        let local = main.rotation.inverse() * world;
        part_translation = offset + local;
        for &e in &w.defs[w.current].entities {
            if let Ok(mut t) = weapon_parts.get_mut(e) {
                t.translation = part_translation;
            }
        }
    }

    // Show only the current weapon.
    for (i, def) in w.defs.iter().enumerate() {
        for &e in &def.entities {
            if let Ok(mut v) = visibility.get_mut(e) {
                *v = if i == w.current { Visibility::Visible } else { Visibility::Hidden };
            }
        }
    }

    // 3D scope (Crossbow.RenderOverlays): while aiming, the lens shows the
    // scope view; otherwise its own material (ScriptedTextureFallback).
    let scope_on = w.aiming && w.defs[w.current].scope.is_some();
    match &w.defs[w.current].scope {
        Some(sc) if scope_on => {
            scope_request.active = true;
            scope_request.fov_deg = sc.portal_fov;
            scope_request.reticle = sc.reticle.clone();
        }
        _ => scope_request.active = false,
    }
    if let Some(view) = scope_view.as_deref() {
        for (i, def) in w.defs.iter().enumerate() {
            let Some(sc) = &def.scope else { continue };
            let wanted = if i == w.current && scope_on {
                view.lens_material.clone()
            } else {
                def.model.parts[sc.lens_part].material.clone()
            };
            if let Ok(mut m) = part_materials.get_mut(def.entities[sc.lens_part])
                && m.0 != wanted
            {
                m.0 = wanted;
            }
        }
    }

    // Pose, then mesh space -> drawn: subtract MeshOrigin, scale by MeshScale (UE2).
    let def = &w.defs[w.current];
    let (skinned, bones) = def.model.pose_with_bones(w.sequence, w.frame);
    let scale = Vec3::from_array(def.model.mesh.scale);
    let origin = Vec3::from_array(def.model.mesh.origin);
    def.model.upload(&skinned, |p| coords::pos(((p - origin) * scale).to_array()), &mut meshes);
    // The effect bones in the world. The weapon camera sits where the main
    // camera is, so the weapon's camera-space parts are in world space as KF
    // places its first-person weapon (Instigator.Location + CalcDrawOffset).
    let (tip, shell) = match main_cam.single() {
        Ok(main) => {
            let to_world = |bone: Option<usize>| -> Option<(Vec3, Mat3)> {
                let (o, axes) = def.model.bone_frame(&bones, bone?)?;
                let local = coords::pos(((o - origin) * scale).to_array()) + part_translation;
                let pos = to_ue(main.transform_point(local)) / coords::SCALE;
                let axes = axes.map(|a| to_ue(main.rotation * coords::dir((a * scale).to_array())).normalize_or_zero());
                Some((pos, Mat3::from_cols(axes[0], axes[1], axes[2])))
            };
            (to_world(def.fx.flash_bone), to_world(def.fx.shell_bone))
        }
        Err(_) => (None, None),
    };
    w.tip = tip;
    w.shell_frame = shell;
    let def = &w.defs[w.current];

    *log_timer += dt;
    if *log_timer >= 1.0 {
        *log_timer = 0.0;
        let (lo, hi) = skinned.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        runlog::kv(
            "weapon_pose",
            &format!(
                "weapon={} action={:?} frame={:.1} aiming={} zoom={:.2} view_fov={:.1} display_fov={:.1} skinned_bounds_unreal=({:.1},{:.1},{:.1})..({:.1},{:.1},{:.1})",
                def.class, w.action, w.frame, w.aiming, w.zoom, view_fov.0, display_fov, lo.x, lo.y, lo.z, hi.x, hi.y, hi.z
            ),
        );
    }
}

/// Bevy direction or position -> Unreal axes (before any SCALE).
fn to_ue(v: Vec3) -> Vec3 {
    Vec3::new(-v.z, v.x, v.y)
}

/// KFFire.InitEffects / FlashMuzzleFlash: each weapon's flash and shell
/// ejector are spawned once (drawn on the weapon layer, as KF draws them
/// with the weapon, Canvas.DrawActor at DisplayFOV), kept on their bones,
/// and triggered on every shot.
fn weapon_fire_fx(
    mut commands: Commands,
    weapons: Option<ResMut<Weapons>>,
    library: Option<Res<crate::particles::EffectLibrary>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut effects: Query<(&mut crate::particles::ParticleEffect, &mut Visibility)>,
) {
    let Some(mut w) = weapons else {
        return;
    };
    let current = w.current;
    if let Some(lib) = library.as_deref() {
        let options = crate::particles::SpawnOptions {
            persistent: true,
            layer: Some(WEAPON_LAYER),
            ..default()
        };
        for def in w.defs.iter_mut().filter(|d| !d.fx.spawn_tried) {
            def.fx.spawn_tried = true;
            for (class, slot) in [(&def.fx.flash_class, &mut def.fx.flash), (&def.fx.shell_class, &mut def.fx.shell)] {
                if let (Some(class), None) = (class, &slot) {
                    *slot = crate::particles::spawn_effect_with(&mut commands, lib, &mut meshes, class, Vec3::ZERO, Mat3::IDENTITY, 7, options);
                    if slot.is_none() {
                        runlog::kv("weapon_fx_missing", &format!("weapon={} class={class}", def.class));
                    }
                }
            }
        }
    }
    let shots = std::mem::take(&mut w.fx_shots);
    let (tip, shell) = (w.tip, w.shell_frame);
    for (i, def) in w.defs.iter().enumerate() {
        for (entity, frame) in [(def.fx.flash, tip), (def.fx.shell, shell)] {
            let Some(Ok((mut fx, mut vis))) = entity.map(|e| effects.get_mut(e)) else {
                continue;
            };
            // Only the weapon in hand draws its effects.
            *vis = if i == current { Visibility::Inherited } else { Visibility::Hidden };
            if i != current {
                continue;
            }
            if let Some(f) = frame {
                fx.frame = f;
            }
            for _ in 0..shots {
                fx.trigger();
            }
        }
    }
    if shots > 0 {
        let def = &w.defs[current];
        let fmt = |f: Option<(Vec3, Mat3)>| f.map_or("none".to_string(), |(p, _)| format!("({:.1}, {:.1}, {:.1})", p.x, p.y, p.z));
        runlog::kv(
            "weapon_fx",
            &format!(
                "weapon={} shots={shots} flash={} shell={} tip_unreal={} shell_unreal={}",
                def.class,
                def.fx.flash.is_some(),
                def.fx.shell.is_some(),
                fmt(tip),
                fmt(shell)
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(group: u8, group_offset: i32, priority: i32) -> Slot {
        Slot {
            group,
            group_offset,
            priority,
            selectable: true,
        }
    }

    /// Adds slots in order with Pawn.AddInventory's rule.
    fn build(items: &[Slot]) -> Vec<Slot> {
        let mut inv = Vec::new();
        for &s in items {
            let at = inventory_position(&inv, s);
            inv.insert(at, s);
        }
        inv
    }

    #[test]
    fn inventory_orders_groups_by_priority() {
        // Knife (1, pri 5), 9mm (2, pri 10), then Axe (1, pri 30) and
        // Machete (1, pri 20): melee sorted by falling priority, kept
        // together ahead of the 9mm.
        let inv = build(&[slot(1, 1, 5), slot(2, 1, 10), slot(1, 3, 30), slot(1, 2, 20)]);
        let pri: Vec<i32> = inv.iter().map(|s| s.priority).collect();
        assert_eq!(pri, vec![30, 20, 5, 10]);
        // A new group goes to the back.
        let inv = build(&[slot(1, 1, 5), slot(2, 1, 10), slot(3, 2, 135)]);
        assert_eq!(inv[2].group, 3);
    }

    #[test]
    fn slot_key_cycles_inside_group() {
        // Axe, Machete, Knife, 9mm, Frag (not selectable).
        let mut inv = vec![slot(1, 3, 30), slot(1, 2, 20), slot(1, 1, 5), slot(2, 1, 10), slot(0, 0, 1)];
        inv[4].selectable = false;
        // From the 9mm, key 1 picks the first melee weapon after it: wraps to the Axe.
        assert_eq!(switch_group(&inv, 3, 1), Some(0));
        assert_eq!(switch_group(&inv, 0, 1), Some(1));
        assert_eq!(switch_group(&inv, 2, 1), Some(0));
        // Only one weapon in the group: it is the answer even if held.
        assert_eq!(switch_group(&inv, 3, 2), Some(3));
        // Nothing in group 4; the Frag never answers for group 0.
        assert_eq!(switch_group(&inv, 3, 4), None);
        assert_eq!(switch_group(&inv, 3, 0), None);
    }

    #[test]
    fn wheel_steps_by_group_then_offset_and_skips_frag() {
        let mut inv = vec![slot(1, 3, 30), slot(1, 1, 5), slot(2, 1, 10), slot(0, 0, 1), slot(3, 2, 135)];
        inv[3].selectable = false;
        // Order: Knife (1,1), Axe (1,3), 9mm (2,1), Shotgun (3,2).
        assert_eq!(step_weapon(&inv, 1, true), Some(0));
        assert_eq!(step_weapon(&inv, 0, true), Some(2));
        assert_eq!(step_weapon(&inv, 2, true), Some(4));
        assert_eq!(step_weapon(&inv, 4, true), Some(1)); // wraps, skipping the Frag
        assert_eq!(step_weapon(&inv, 1, false), Some(4));
    }
}
