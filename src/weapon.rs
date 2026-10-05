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

/// What a weapon's alt fire toggles.
#[derive(Clone, Copy, Debug, PartialEq)]
enum AltToggle {
    /// KFWeapon.DoToggle: FireMode[0].bWaitForRelease (full / semi auto).
    FireMode,
    /// KSGShotgun.DoToggle: bWideSpread.
    WideSpread,
}

/// Weapons whose AltFire calls KFWeapon.DoToggle (full / semi auto), from
/// their scripts (the KSG's DoToggle switches the spread instead). Exact
/// classes: the M4 203 extends the M4 but overrides AltFire.
const TOGGLE_ON_ALT_FIRE: [&str; 8] = [
    "AA12AutoShotgun",
    "AK47AssaultRifle",
    "Bullpup",
    "FNFAL_ACOG_AssaultRifle",
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
/// "gorefast_far" = one 900 units away, "zed_line" = three Clots in a
/// line ahead ("zed_line_far": 700-900 away), "cycle_zed" = press N,
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
    /// Fire mode 1's own ammo when it uses another ammo class (the M4 203's
    /// M203Ammo): rounds left and MaxAmmo.
    alt_ammo: Option<(u32, u32)>,
    /// What alt fire toggles (the class's AltFire calls DoToggle), if anything.
    toggles_on_alt: Option<AltToggle>,
    /// KSGShotgun.bWideSpread (KSGFire: Spread x 2.05).
    wide_spread: bool,
    /// Thrown away (the last pipe bomb placed: PipeBombFire.Timer destroys
    /// the weapon); no longer selectable.
    gone: bool,
    /// Frag: TossAnim, TossTime and TossSpawnTime (Frag.StartThrow).
    toss: Option<(String, f32, f32)>,
    /// The Syringe's healing charge (AmmoCharge).
    heal_charge: Option<HealCharge>,
    /// The Welder's fuel.
    weld_fuel: Option<WeldFuel>,
    /// KFWeapon QuickPutDownTime / QuickBringUpTime (around a frag throw).
    quick_put_down_time: f32,
    quick_bring_up_time: f32,
    /// KFMeleeGun.ChopSlowRate: each melee attack scales the walking
    /// velocity by this (KFMeleeFire.ModeDoFire), 1 for other weapons.
    chop_slow_rate: f32,
    /// BoomStick: ReloadCountDown (both barrels reload by themselves this
    /// long after the last one is fired).
    boomstick_reload: Option<f32>,
    /// First-person firing effects (KFFire.InitEffects).
    fx: FireFx,
    /// The 3D scope (bHasScope), if any.
    scope: Option<WeaponScope>,
    /// A Dualies class: two guns firing in turn.
    dual: bool,
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

/// A bone's origin and axes, Unreal world.
type BoneFrame = (Vec3, Mat3);

/// Dual pistols and the single pistol each replaces (their GiveTo).
const DUAL_PAIRS: [(&str, &str); 4] = [
    ("Dualies", "Single"),
    ("DualDeagle", "Deagle"),
    ("Dual44Magnum", "Magnum44Pistol"),
    ("DualMK23Pistol", "MK23Pistol"),
];

/// Dualies.GiveTo (given, not picked up): the magazine is the single's
/// plus the single's MagCapacity, up to the duals' capacity; the ammo is
/// the duals' InitialAmount plus all of the single's, up to MaxAmmo
/// (AddAmmo caps). My reading: KF weapons keep ammo as separate items
/// (bNoAmmoInstances false); whether the single's item survives is not
/// checked.
fn merge_dual_ammo(single: Ammo, dual: Ammo) -> Ammo {
    let mag = (single.mag + single.capacity).min(dual.capacity);
    let total = (dual.initial + single.mag + single.spare).min(dual.max_total).max(mag);
    Ammo {
        mag,
        spare: total - mag,
        ..dual
    }
}

/// Fire classes whose DoTrace lets a bullet pass through up to 5 zeds,
/// halving its damage each time (the same function in all six).
const PENETRATING_FIRE: [&str; 6] = [
    "DeagleFire",
    "Magnum44Fire",
    "MK23Fire",
    "DualDeagleFire",
    "Dual44MagnumFire",
    "DualMK23Fire",
];

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
    shell_class: Option<String>,
    /// One per gun: [0] the main one; dual pistols have [1], the left gun
    /// (DualiesFire: Flash2Emitter on altFlashBoneName).
    hands: Vec<FxHand>,
    spawn_tried: bool,
}

/// A gun's muzzle flash and shell ejector bones, and the spawned effects
/// (spawned once the effect library is loaded).
#[derive(Default)]
struct FxHand {
    flash_bone: Option<usize>,
    shell_bone: Option<usize>,
    flash: Option<Entity>,
    shell: Option<Entity>,
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
    /// WeldFire / UnWeldFire: needs a weldable door in front (none yet).
    weld: bool,
    /// bModeExclusive: false lets the other mode fire at the same time
    /// (the ZED MKII).
    mode_exclusive: bool,
    /// ZEDGunAltFire's zapping beam.
    beam: Option<BeamFire>,
    /// HuskGunFire's charged release.
    charge: Option<ChargeFire>,
    /// The bullets' damage type burns (instant fire; W7).
    fire: Option<crate::combat::FireType>,
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
    /// DualiesFire: the other hand's FireAnim2 / FireAimedAnim2 (swapped
    /// with FireAnim / FireAimedAnim after every shot).
    anim2: String,
    aimed_anim2: String,
    /// Zeds a bullet passes through (DeagleFire.DoTrace: 5, else 1).
    penetrations: u32,
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
    /// ChainsawFire: hits at every shot (no swing delay, no wide hits) for
    /// MeleeDamage + Rand(maxAdditionalDamage).
    chainsaw: bool,
    extra_damage: u32,
    /// Pellet / nail fire (KFShotgunFire), when kind is Pellets.
    pellets: Option<PelletFire>,
    /// BoomStick fire modes: FireLastAnim / FireLastAimedAnim and
    /// FireLastRate (BoomStickAltFire: the shot that empties the gun).
    last_anim: String,
    last_aimed_anim: String,
    last_rate: Option<f32>,
    /// M79Fire / M203Fire / LAWFire.AllowFire: only the ammo total counts
    /// (no magazine check, also while reloading).
    total_ammo_only: bool,
    /// LAWFire.AllowFire: only when aimed and fully zoomed in.
    requires_aim: bool,
    /// PipeBombFire.ModeDoFire: the projectile (and the ammo use) comes
    /// ProjectileSpawnDelay after the click, as the Toss animation lets go.
    spawn_delay: Option<f32>,
    /// When the last animation plays: BoomStickAltFire when the shot empties
    /// the gun (and more ammo is left), BoomStickFire when it is the very
    /// last ammo.
    last_rule: LastShot,
    combat: CombatStats,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum LastShot {
    Never,
    WhenEmptied,
    WhenVeryLast,
}

/// KFShotgunFire values and its projectile's.
#[derive(Clone, Copy, Debug)]
struct PelletFire {
    stats: crate::projectile::ProjectileStats,
    /// ProjPerFire, times Load (= AmmoPerFire) per shot.
    per_fire: u32,
    ammo_per_fire: u32,
    /// Spread (rotator units; SS_Random: each of yaw, pitch, roll
    /// Spread x (FRand() - 0.5)).
    spread: f32,
    /// KickMomentum (Unreal, view axes) and ProjSpawnOffset.
    kick: Vec3,
    spawn_offset: Vec3,
    /// A grenade or rocket instead of pellets.
    explosive: Option<crate::projectile::ExplosiveStats>,
    /// A thrown frag or pipe bomb instead.
    thrown: Option<crate::projectile::ThrownStats>,
    /// Flamethrower flames (FlameTendril) instead.
    flame: Option<crate::projectile::FlameStats>,
    /// Medic darts (HealingProjectile) instead.
    dart: Option<crate::projectile::DartStats>,
    /// KFShotgunFire.DoFireEffect spawns ProjPerFire x Load; the medic
    /// alt fires and ZEDMKIIAltFire override it with ProjPerFire only.
    per_load: bool,
}

/// ZEDGunAltFire: a beam while held. Every FireRate (ModeDoFire) it uses a
/// round and zaps the zeds near where it lands; every frame (ModeTick) the
/// zed it touches is zapped by the frame time.
#[derive(Clone, Debug)]
struct BeamFire {
    /// TraceRange; MaxZedSphereChargeTime (the splash grows to 250 units
    /// over it); ProjSpawnOffset (GetFirstPersonBeamFireStart);
    /// ChargeEmitterClass.
    range: f32,
    sphere_time: f32,
    offset: Vec3,
    effect: Option<String>,
}

/// A beam in progress: ChargeUpTime, UpTime, bDoHit.
#[derive(Clone, Copy, Debug)]
struct BeamState {
    charge_up: f32,
    up_time: f32,
    do_hit: bool,
}

/// ZEDGunAltFire's splash: 250 x ChargeScale units.
const BEAM_SPHERE_RADIUS: f32 = 250.0;

/// The Welder's fuel (WelderAmmo, both fire modes share it): MaxAmmo 300,
/// refilled AmmoRegenRate (40) a second by Welder.Tick, whether held or
/// not; whole units only, the fraction carried (AmmoRegenCount).
#[derive(Clone, Copy, Debug)]
struct WeldFuel {
    amount: u32,
    max: u32,
    regen_rate: f32,
    regen_count: f32,
    /// AmmoPerFire: WeldFire 20, UnWeldFire 15.
    cost: [u32; 2],
}

/// Syringe / KFMedicGun healing charge: up to 500 (MaxAmmoCount), +10 every
/// AmmoRegenRate seconds (Tick, whether held or not).
#[derive(Clone, Copy, Debug)]
struct HealCharge {
    charge: u32,
    regen_rate: f32,
    /// RegenTimer (absolute time of the next +10).
    next_regen: f32,
    /// HealBoostAmount (Syringe.PostBeginPlay: 50 with one player).
    boost: f32,
    /// AmmoPerFire and InjectDelay of each mode (SyringeFire,
    /// SyringeAltFire).
    cost: [u32; 2],
    inject_delay: [f32; 2],
    /// The Syringe (its own fire modes); otherwise a KFMedicGun, whose
    /// charge feeds the alt fire's darts (HealAmmoCharge).
    syringe: bool,
}

/// Syringe / KFMedicGun MaxAmmoCount.
const HEAL_CHARGE_MAX: u32 = 500;

/// KFPawn.QuickHeal (Q) progress (KFPawn.bIsQuickHealing 1 and 2).
#[derive(Clone, Copy, Debug, PartialEq)]
enum QuickHeal {
    Off,
    /// Bringing the Syringe out; when ready, inject (retried every 0.2 s).
    Inject { back: usize, retry: f32 },
    /// Injected; switch back to `back` after FireRate + 0.5 s.
    Back { back: usize, timer: f32 },
}

/// HuskGunFire: bFireOnRelease with a charge (HoldTime up to MaxChargeTime)
/// that picks the projectile and scales it (PostSpawnProjectile).
#[derive(Clone, Debug)]
struct ChargeFire {
    max_time: f32,
    /// Under a third of MaxChargeTime, under two thirds, the rest.
    weak: crate::projectile::ExplosiveStats,
    medium: crate::projectile::ExplosiveStats,
    strong: crate::projectile::ExplosiveStats,
    /// ChargeEmitterClass, on the 'tip' bone while charging.
    effect: Option<String>,
}

impl ChargeFire {
    /// The projectile for a release after `hold` seconds:
    /// GetDesiredProjectileClass, then PostSpawnProjectile's scaling
    /// (ImpactDamage x HoldTime x 2.5, Damage x (1 + HoldTime / Max),
    /// DamageRadius x (1 + HoldTime / (Max / 2)); at full charge x 7.5, x 2, x 3).
    fn projectile(&self, hold: f32) -> crate::projectile::ExplosiveStats {
        let mut x = if hold < self.max_time * 0.33 {
            self.weak
        } else if hold < self.max_time * 0.66 {
            self.medium
        } else {
            self.strong
        };
        let h = hold.min(self.max_time);
        x.impact_damage *= h * 2.5;
        x.damage *= 1.0 + h / self.max_time;
        x.radius *= 1.0 + h / (self.max_time / 2.0);
        x
    }

    /// HuskGunFire.ModeDoFire: 1 + HoldTime / (Max / 9) rounds, 10 at full
    /// charge (an int when used), at most what is left.
    fn ammo(&self, hold: f32, left: u32) -> u32 {
        let n = if hold < self.max_time { (1.0 + hold / (self.max_time / 9.0)) as u32 } else { 10 };
        n.min(left)
    }
}

/// A HuskGunProjectile class's values: `base` with the class's own
/// ExplosionEmitter, ExplosionDecal and FlameTrailEmitterClass.
fn husk_projectile(
    set: &PackageSet,
    defaults: &ClassDefaults,
    class: &ObjectHandle,
    base: crate::projectile::ExplosiveStats,
) -> crate::projectile::ExplosiveStats {
    use crate::decals::DecalKind;
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
    crate::projectile::ExplosiveStats {
        class: leak(class.path()),
        effect: path("ExplosionEmitter").map_or(base.effect, leak),
        trail: path("FlameTrailEmitterClass").map(leak).or(base.trail),
        decal,
        ..base
    }
}

/// Which burn rules a damage type follows, if it has bDealBurningDamage
/// (KFMonster.TakeDamage / ZombieBloat / ZombieHusk tell them apart by class).
fn fire_type(defaults: &ClassDefaults, dt: &ObjectHandle) -> Option<crate::combat::FireType> {
    use crate::combat::FireType;
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
        let explosive = (defaults.is_a(pc, "M79GrenadeProjectile") || is_law).then(|| crate::projectile::ExplosiveStats {
            class: projectile_path,
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
            hurts_self: !(is_husk || is_zed_bolt || is_zed_orb),
            // Explode: LAWProj spawns LawExplosion, the M79 family
            // KFNadeLExplosion; ExplosionDecal RocketMarkDirt / KFScorchMark.
            effect: if is_law { "KFMod.LawExplosion" } else { "KFMod.KFNadeLExplosion" },
            decal: if is_law { crate::decals::DecalKind::RocketMark } else { crate::decals::DecalKind::NadeScorch },
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
        let thrown = (defaults.is_a(pc, "Nade") || is_pipe).then(|| crate::projectile::ThrownStats {
            class: projectile_path,
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
            decal: crate::decals::DecalKind::NadeScorch,
            kind: if is_pipe {
                crate::projectile::ThrownKind::Pipe {
                    arming: pfloat("ArmingCountDown", 1.0),
                    detection_radius: pfloat("DetectionRadius", 150.0),
                    countdown: pfloat("CountDown", 5.0) as u32,
                    threshold: pfloat("ThreatThreshhold", 1.0),
                }
            } else {
                crate::projectile::ThrownKind::Frag { fuse: pfloat("ExplodeTimer", 2.0) }
            },
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
        let flame = defaults.is_a(pc, "FlameTendril").then(|| crate::projectile::FlameStats {
            speed: pfloat("Speed", 2300.0),
            toss_z: pfloat("TossZ", 200.0),
            damage: pfloat("Damage", 12.0),
            radius: pfloat("DamageRadius", 150.0),
            life_span: pfloat("LifeSpan", 5.0),
        });
        let dart = defaults.is_a(pc, "HealingProjectile").then(|| crate::projectile::DartStats {
            class: projectile_path,
            speed: pfloat("Speed", 10000.0),
            life_span: pfloat("LifeSpan", 10.0),
            heal: pfloat("HealBoostAmount", 20.0),
        });
        let per_load = !["MP7MAltFire", "M7A3MAltFire", "ZEDMKIIAltFire"].iter().any(|c| defaults.is_a(fm_class, c));
        mode.pellets = Some(PelletFire {
            per_load,
            dart,
            flame,
            thrown,
            explosive,
            stats: crate::projectile::ProjectileStats {
                class: projectile_path,
                speed: pfloat("Speed", 3500.0),
                damage: pfloat("Damage", 0.0),
                max_penetrations: pfloat("MaxPenetrations", 1.0),
                pen_damage_reduction: pfloat("PenDamageReduction", 0.5),
                headshot_mult: pfloat("HeadShotDamageMult", 1.5),
                damage_type_headshot_mult: dt_mult,
                life_span: pfloat("LifeSpan", 3.0),
                bounces: pfloat("Bounces", 0.0) as u32,
                rule: if is_bolt { crate::projectile::PenRule::Bolt } else { crate::projectile::PenRule::Pellet },
                pickup: defaults.is_a(pc, "CrossbowArrow"),
                fire: match pget("MyDamageType") {
                    Some((Value::Object(r), rp)) => set.resolve(&rp, r).and_then(|dt| fire_type(defaults, &dt)),
                    _ => None,
                },
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

/// How a fire mode works, from its class defaults.
#[derive(Clone, Copy, PartialEq, Debug)]
enum FireKind {
    /// KFMeleeFire: MeleeDamage after a delay, in a cone.
    Melee,
    /// InstantFire / KFFire: a hit trace (KFFire.DoTrace).
    Instant,
    /// KFShotgunFire with pellets or nails (ShotgunBullet family,
    /// TrenchgunBullet): ProjPerFire projectiles in a spread (projectile.rs).
    Pellets,
    /// Other BaseProjectileFire: spawns ProjectileClass (not done yet: W6).
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
    /// The ammo class's InitialAmount and MaxAmmo (total, magazine included).
    initial: u32,
    max_total: u32,
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
    /// KFPawn.ThrowGrenade (G): the weapon goes down quickly, the frag is
    /// tossed, the weapon comes back up quickly; `back_to` is the weapon.
    Grenade { phase: NadePhase, back_to: usize },
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum NadePhase {
    /// PutDown at QuickPutDownTime.
    Down,
    /// Frag.StartThrow: TossAnim; the Nade at TossSpawnTime.
    Toss { spawned: bool },
    /// BringUp at QuickBringUpTime.
    Up,
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
    /// BoomStick: seconds until both barrels reload by themselves.
    boomstick_pending: Option<f32>,
    /// PipeBombFire: (seconds left, weapon index, mode) of a placement
    /// waiting for its ProjectileSpawnDelay.
    pending_spawn: Option<(f32, usize, usize)>,
    /// HuskGunFire: HoldTime of the charge in progress (mode 0), and the
    /// ChargeEmitter effect.
    charge_hold: Option<f32>,
    charge_fx: Option<Entity>,
    /// SyringeAltFire's InjectDelay timer: (seconds left, weapon index,
    /// mode), then the charge is used and the heal given.
    pending_inject: Option<(f32, usize, usize)>,
    /// The ZED Gun's beam, while it is on.
    beam: Option<BeamState>,
    /// SyringeFire.AttemptHeal's LastHealAttempt (the "no one to heal"
    /// message at most every HealAttemptDelay).
    last_heal_attempt: f32,
    /// WeldFire.FailTime.
    last_weld_fail: f32,
    quick_heal: QuickHeal,
    /// Seconds left of the bring-up or put-down (Weapon's Timer), and
    /// whether the put-down is still waiting out DownDelay.
    switch_timer: f32,
    down_delayed: bool,
    /// Melee swings waiting for their damage moment: (seconds left,
    /// stats, weapon name). KFMeleeFire.ModeDoFire sets a timer per swing.
    pending_swings: Vec<(f32, CombatStats, &'static str)>,
    /// Welder hits waiting for WeldFire.Timer: (seconds left, damage,
    /// range, unweld).
    pending_welds: Vec<(f32, f32, f32, bool)>,
    /// Bound for bWaitForRelease modes: a press not yet turned into a shot.
    press_waiting: [bool; 2],
    /// Iron sights wanted (bAimingRifle) and the zoom blend, 0 = hip, 1 = aimed.
    aiming: bool,
    zoom: f32,
    /// Seconds a full zoom transition takes right now.
    zoom_time: f32,
    /// Simple random number state (spread, damage rolls).
    rng: u64,
    /// Shots whose flash and shell are still to be triggered: the hand.
    fx_shots: Vec<usize>,
    /// Per hand of the current weapon: the flash bone frame (Unreal world
    /// origin and axes, as last posed: where tracers start,
    /// KFWeapon.GetEffectStart) and the shell ejector frame.
    hand_frames: Vec<(Option<BoneFrame>, Option<BoneFrame>)>,
    /// Dual pistols: the next shot is the left gun's, from the hip / aimed
    /// (DualiesFire swaps FireAnim and FireAimedAnim separately).
    dual_left: [bool; 2],
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
    // Start with the 9mm, as KF does (the dual 9mms if they replaced it).
    let current = ["KFMod.Single", "KFMod.Dualies"]
        .iter()
        .find_map(|c| defs.iter().position(|d| d.class.eq_ignore_ascii_case(c)))
        .unwrap_or(0);
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
            let max_total = match defaults.get(&ammo_class, "MaxAmmo") {
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
        scope,
        dual,
        toggles_on_alt: if class_name.eq_ignore_ascii_case("KSGShotgun") {
            Some(AltToggle::WideSpread)
        } else {
            TOGGLE_ON_ALT_FIRE.iter().any(|c| class_name.eq_ignore_ascii_case(c)).then_some(AltToggle::FireMode)
        },
        wide_spread: false,
        gone: false,
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
            selectable: !self.class.eq_ignore_ascii_case("KFMod.Frag") && !self.gone,
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
        Action::Grenade { .. } => return,
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
/// WeldFire / UnWeldFire while the button is held (KFMeleeFire.ModeDoFire
/// with WeldFire.AllowFire). `door` is the door in view (WeldFire.GetDoor
/// traces the fire mode's weaponRange, 70, from the eye).
fn weld_fire(w: &mut Weapons, mode: usize, fm: &FireMode, door: Option<crate::door::WeldDoor>, now: f32) {
    let cur = w.current;
    let Some(fuel) = w.defs[cur].weld_fuel else {
        return;
    };
    let door = door.filter(|d| d.distance <= fm.combat.range);
    // Weapon.ReadyToFire: ready, the other mode not firing (bModeExclusive),
    // NextFireTime passed. AllowFire runs only then.
    let alt = 1 - mode;
    let exclusive = fm.mode_exclusive || w.defs[cur].modes[alt].mode_exclusive;
    if w.action != Action::Idle || (exclusive && (w.firing[alt] || w.fire_cooldown[alt] > 0.0)) || w.fire_cooldown[mode] > 0.0 {
        return;
    }
    let allow = match (mode, door) {
        // NoWeldTargetMessage at most every 0.5 s (FailTime).
        (0, None) => {
            if now - w.last_weld_fail > 0.5 {
                w.last_weld_fail = now;
                runlog::kv("message", "text=\"You must be near a weldable door to use the welder.\"");
            }
            false
        }
        // CantWeldTargetMessage every AllowFire call in KF; rate-limited
        // like the other here so the log stays readable.
        (0, Some(d)) if d.disallow_weld => {
            if now - w.last_weld_fail > 0.5 {
                w.last_weld_fail = now;
                runlog::kv("message", "text=\"You cannot weld this door.\"");
            }
            false
        }
        // UnWeldFire: a door with weld left; fails silently.
        (1, None) => false,
        (1, Some(d)) if d.weld <= 0.0 => false,
        _ => fuel.amount >= fuel.cost[mode],
    };
    if !allow {
        return;
    }
    if !w.firing[mode] {
        w.firing[mode] = true;
        w.shots_this_press[mode] = 0;
        w.fire_cooldown[mode] = 0.0;
    }
    if let Some(f) = w.defs[cur].weld_fuel.as_mut() {
        f.amount -= fuel.cost[mode];
    }
    w.fire_cooldown[mode] = (w.fire_cooldown[mode] + fm.rate).max(0.0);
    // WeldFire.PlayFiring: FireLoopAnim after the first, if the mesh has it.
    play_firing(w, mode, false);
    w.shots_this_press[mode] += 1;
    w.fire_count += 1;
    w.pending_welds.push((fm.combat.damage_delay, fm.combat.damage_min, fm.combat.range, mode == 1));
    runlog::kv(
        "weld_fire",
        &format!(
            "mode={} door={} distance={:.0} screen_percent={:.0} fuel={} damage={} delay={}",
            if mode == 1 { "unweld" } else { "weld" },
            door.map_or(0, |d| d.index),
            door.map_or(0.0, |d| d.distance),
            // Welder.ScreenWeldPercent, shown on the welder's screen in KF.
            door.map_or(0.0, |d| if d.max_weld > 0.0 { d.weld / d.max_weld * 100.0 } else { 0.0 }),
            fuel.amount - fuel.cost[mode],
            fm.combat.damage_min,
            fm.combat.damage_delay
        ),
    );
}

fn play_firing(w: &mut Weapons, mode: usize, last: bool) {
    let mut m = w.defs[w.current].modes[mode].clone();
    // BoomStick: FireLastAnim / FireLastAimedAnim (fire and reload).
    if last {
        let (name, rate) = if w.aiming && has_anim(w, &m.last_aimed_anim) {
            (m.last_aimed_anim.clone(), m.anim_rate)
        } else {
            (m.last_anim.clone(), m.anim_rate)
        };
        if has_anim(w, &name) {
            return play(w, &name, rate, false);
        }
    }
    // DualiesFire: the left gun's turn plays FireAnim2 / FireAimedAnim2.
    if w.defs[w.current].dual && mode == 0 {
        if w.dual_left[0] {
            m.anims = vec![m.anim2.clone()];
        }
        if w.dual_left[1] {
            m.aimed_anim = m.aimed_anim2.clone();
        }
    }
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

/// The Syringe's two modes while their button is held. SyringeFire (left)
/// heals a teammate in front of you (GetHealee: within 80 units); alone
/// there is none, so AttemptHeal only shows "You must be near another
/// player to heal them!" (at most every HealAttemptDelay). SyringeAltFire
/// (alt): with Health under HealthMax and a full charge, ModeDoFire plays
/// AltFire and the heal follows InjectDelay later (`pending_inject`).
fn syringe_fire(w: &mut Weapons, mode: usize, pressed: bool, h: HealCharge, health: f32, now: f32) {
    let alt = 1 - mode;
    let fm = &w.defs[w.current].modes[mode];
    let ready = w.action == Action::Idle && w.fire_cooldown[mode] <= 0.0 && w.fire_cooldown[alt] <= 0.0 && !w.firing[alt];
    if mode == 0 {
        // bWaitForRelease: one attempt per click.
        if pressed && ready && now - w.last_heal_attempt > 0.5 {
            w.last_heal_attempt = now;
            runlog::kv(
                "syringe_no_target",
                &format!("message=\"You must be near another player to heal them!\" charge={}", h.charge),
            );
        }
        return;
    }
    if !ready {
        return;
    }
    if health >= crate::combat::PLAYER_HEALTH_MAX || h.charge < h.cost[1] {
        if pressed {
            runlog::kv("syringe_refused", &format!("health={health:.0} charge={}", h.charge));
        }
        return;
    }
    let rate = fm.rate;
    w.firing[mode] = true;
    w.fire_cooldown[mode] = rate;
    w.pending_inject = Some((h.inject_delay[1], w.current, 1));
    play_firing(w, 1, false);
    runlog::kv("syringe_self_heal", &format!("health={health:.0} charge={} inject_delay={} fire_rate={rate}", h.charge, h.inject_delay[1]));
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
    let def = &w.defs[w.current];
    // BoomStick.AllowReload: not with one barrel loaded.
    if def.boomstick_reload.is_some() && def.ammo.is_some_and(|a| a.mag == 1) {
        return false;
    }
    def.ammo.is_some_and(|a| a.mag < a.capacity && a.spare > 0)
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
    // Dualies.ZoomOut, when animated (toggling off, not reload or switch):
    // GOTO_Hip stretched to the zoom time.
    if !fast && w.defs[cur].dual && has_anim(w, "goto_hip") {
        let def = &w.defs[cur];
        let seq = def.model.sequence("goto_hip").expect("checked");
        let seconds = def.model.length(seq) / def.model.rate(seq).max(1e-3);
        let rate = if w.zoom_time > 0.0 && seconds > 0.0 { seconds / w.zoom_time } else { 1.0 };
        play(w, "goto_hip", rate, false);
    }
    runlog::kv("iron_sights", &format!("weapon={} aiming=false reason={reason}", w.defs[cur].class));
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)] // Bevy system parameters
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
    (mut shots, mut swings, mut pellets, mut kicks): (
        MessageWriter<ShotFired>,
        MessageWriter<MeleeSwing>,
        MessageWriter<crate::projectile::SpawnPlayerProjectile>,
        MessageWriter<crate::walk::PlayerAddVelocity>,
    ),
    mut ammo_display: ResMut<crate::combat::AmmoDisplay>,
    mut recoil: ResMut<crate::firing::Recoil>,
    (health, mut bolt_room, mut bolts_picked, mut heals, mut beam_zaps): (
        Res<crate::combat::PlayerHealth>,
        ResMut<crate::projectile::BoltRoom>,
        MessageReader<crate::projectile::BoltPickedUp>,
        MessageWriter<crate::combat::GiveHealth>,
        MessageWriter<crate::projectile::BeamZap>,
    ),
    (weld_view, mut weld_hits): (Res<crate::door::WeldView>, MessageWriter<crate::door::WeldHit>),
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
    let now_s = time.elapsed_secs();
    // Syringe.Tick: +10 charge every AmmoRegenRate while under the maximum.
    for d in &mut w.defs {
        if let Some(h) = d.heal_charge.as_mut()
            && h.charge < HEAL_CHARGE_MAX
            && h.next_regen < now_s
        {
            h.next_regen = now_s + h.regen_rate;
            h.charge = (h.charge + 10).min(HEAL_CHARGE_MAX);
        }
    }
    // Welder.Tick: fuel back at AmmoRegenRate while under MaxAmmo.
    for d in &mut w.defs {
        if let Some(f) = d.weld_fuel.as_mut()
            && f.amount < f.max
        {
            f.regen_count += time.delta_secs() * f.regen_rate;
            let whole = f.regen_count.floor();
            f.amount = (f.amount + whole as u32).min(f.max);
            f.regen_count -= whole;
        }
    }
    // SyringeAltFire.Timer (InjectDelay after the shot): use the charge,
    // GiveHealth(HealBoostAmount, 100).
    if let Some((t, wi, mode)) = w.pending_inject {
        let t = t - time.delta_secs();
        if t > 0.0 {
            w.pending_inject = Some((t, wi, mode));
        } else {
            w.pending_inject = None;
            if let Some(h) = w.defs[wi].heal_charge.as_mut() {
                if h.cost[mode] <= h.charge {
                    h.charge -= h.cost[mode];
                }
                let amount = h.boost;
                heals.write(crate::combat::GiveHealth { amount, max: crate::combat::PLAYER_HEALTH_MAX, source: "syringe" });
                runlog::kv("syringe_inject", &format!("heal={amount} charge_left={}", h.charge));
            }
        }
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
    // KFPawn.QuickHeal (Q): hurt, a Syringe charged to 95% or more: bring
    // it out (or, if it is in hand, inject now).
    let mut force_alt = false;
    let syringe = w.defs.iter().position(|d| d.heal_charge.is_some_and(|h| h.syringe));
    if keys.just_pressed(KeyCode::KeyQ) || scripted("quickheal") {
        let charge = syringe.and_then(|i| w.defs[i].heal_charge).map_or(0, |h| h.charge);
        if health.health >= crate::combat::PLAYER_HEALTH_MAX {
            runlog::kv("quick_heal_refused", "reason=full_health");
        } else if let Some(si) = syringe {
            if (charge as f32) < 0.95 * HEAL_CHARGE_MAX as f32 {
                runlog::kv("quick_heal_refused", &format!("reason=charge charge={charge}"));
            } else if w.current == si {
                force_alt = true;
                runlog::kv("quick_heal", "syringe_in_hand=true");
            } else {
                w.quick_heal = QuickHeal::Inject { back: w.current, retry: 0.0 };
                choice = Some(si);
                runlog::kv("quick_heal", &format!("syringe_in_hand=false back={}", w.defs[w.current].item_name));
            }
        }
    }
    // Syringe.Timer while quick healing: once the Syringe is ready, inject
    // (HackClientStartFire); if that fails and you are full or the charge
    // is under 75%, give up, else try again in 0.2 s. After FireRate +
    // 0.5 s, SwitchToLastWeapon. A weapon change ends it.
    let heading_to_syringe = |w: &Weapons, si: usize| w.current == si || matches!(w.action, Action::PutDown { next } if next == si);
    match w.quick_heal {
        QuickHeal::Off => {}
        _ if choice.is_none() && syringe.is_none_or(|si| !heading_to_syringe(&w, si)) => {
            w.quick_heal = QuickHeal::Off;
        }
        QuickHeal::Inject { back, retry } if w.action == Action::Idle && syringe == Some(w.current) => {
            let si = syringe.expect("checked");
            let retry = retry - time.delta_secs();
            if retry <= 0.0 {
                let h = w.defs[si].heal_charge.expect("syringe");
                let can = health.health < crate::combat::PLAYER_HEALTH_MAX
                    && h.charge >= h.cost[1]
                    && w.fire_cooldown[0] <= 0.0
                    && w.fire_cooldown[1] <= 0.0;
                if can {
                    force_alt = true;
                    let rate = w.defs[si].modes[1].rate;
                    w.quick_heal = QuickHeal::Back { back, timer: rate + 0.5 };
                } else if health.health >= crate::combat::PLAYER_HEALTH_MAX || (h.charge as f32) < 0.75 * HEAL_CHARGE_MAX as f32 {
                    w.quick_heal = QuickHeal::Back { back, timer: 0.2 };
                } else {
                    w.quick_heal = QuickHeal::Inject { back, retry: 0.2 };
                }
            } else {
                w.quick_heal = QuickHeal::Inject { back, retry };
            }
        }
        QuickHeal::Back { back, timer } if w.action == Action::Idle && syringe == Some(w.current) => {
            let timer = timer - time.delta_secs();
            if timer <= 0.0 {
                w.quick_heal = QuickHeal::Off;
                if back < w.defs.len() && !w.defs[back].gone && back != w.current {
                    choice = Some(back);
                }
                runlog::kv("quick_heal_done", &format!("back={}", w.defs[back].item_name));
            } else {
                w.quick_heal = QuickHeal::Back { back, timer };
            }
        }
        _ => {}
    }
    if let Some(next) = choice {
        match w.action {
            Action::PutDown { .. } => w.action = Action::PutDown { next },
            Action::Grenade { .. } => {}
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
    // KFPawn.ThrowGrenade (G): a frag in stock, the weapon's next shot due
    // within 0.1 s, not reloading (a one-round reload is interrupted).
    if keys.just_pressed(KeyCode::KeyG) || scripted("nade") {
        let frag = w.defs.iter().position(|d| d.toss.is_some());
        let frags = frag.and_then(|i| w.defs[i].ammo).map_or(0, |a| a.mag + a.spare);
        interrupt_reload(&mut w, "grenade");
        if let Some(fi) = frag
            && frags > 0
            && w.action == Action::Idle
            && w.fire_cooldown[0] <= 0.1
            && !w.firing.iter().any(|&f| f)
        {
            zoom_out(&mut w, true, "grenade");
            let back = w.current;
            let d = &w.defs[back];
            let (anim, rate, time) = (d.put_down_anim.clone(), d.put_down_anim_rate * d.put_down_time / d.quick_put_down_time, d.quick_put_down_time);
            play(&mut w, &anim, rate, false);
            w.switch_timer = time;
            w.action = Action::Grenade { phase: NadePhase::Down, back_to: back };
            runlog::kv("grenade_throw_start", &format!("weapon={} frags={frags} frag_index={fi}", w.defs[back].item_name));
        } else {
            runlog::kv("grenade_throw_refused", &format!("frags={frags} action={:?} cooldown={:.2}", w.action, w.fire_cooldown[0]));
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
            // Dualies.ZoomIn plays GOTO_Iron.
            if w.defs[cur].dual && has_anim(&w, "goto_iron") {
                play(&mut w, "goto_iron", 1.0, false);
            }
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
    let mut pressed: [bool; 2] = std::array::from_fn(|i| {
        (grabbed && mouse.just_pressed(buttons[i].0)) || scripted(buttons[i].1) || pressed_by_script[i]
    });
    let mut held = held;
    if force_alt {
        held[1] = true;
        pressed[1] = true;
    }
    // BoomStick.ClientStartFire: both barrels asked for with one loaded
    // fires the single barrel.
    if w.defs[w.current].boomstick_reload.is_some() && w.defs[w.current].ammo.is_some_and(|a| a.mag == 1) && held[1] {
        held[0] = true;
        pressed[0] |= pressed[1];
        held[1] = false;
        pressed[1] = false;
    }
    // A charge ends without a shot if the weapon leaves its ready state.
    if w.charge_hold.is_some() && !matches!(w.action, Action::Idle) {
        runlog::kv("charge_cancelled", &format!("weapon={} action={:?}", w.defs[w.current].item_name, w.action));
        w.charge_hold = None;
        w.firing[0] = false;
    }
    for mode in 0..2 {
        let alt = 1 - mode;
        let cur = w.current;
        // bFireOnRelease (HuskGunFire): letting go fires the charged shot.
        let charge_release = if !held[mode] && mode == 0 { w.charge_hold.take() } else { None };
        // Weapon.StopFire on release: the fire end animation.
        if !held[mode] && charge_release.is_none() {
            // ZEDGunAltFire: letting go ends the beam (StopFiring; the
            // Timer then plays PlayFireEnd: ChargeDown).
            if w.defs[cur].modes[mode].beam.is_some() && w.beam.take().is_some() {
                w.firing[mode] = false;
                play(&mut w, "ChargeDown", 1.0, false);
                runlog::kv("beam_stop", &format!("weapon={} reason=released", w.defs[cur].item_name));
                w.press_waiting[mode] = false;
                continue;
            }
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
        if mode == 1 && let Some(toggle) = w.defs[cur].toggles_on_alt {
            if pressed[mode] {
                let mag_ok = w.defs[cur].ammo.is_none_or(|a| a.mag >= 1);
                if ready_state && w.action != Action::Reload && mag_ok && w.fire_cooldown[0] <= 0.0 && !w.firing[0] {
                    match toggle {
                        AltToggle::FireMode => {
                            let m = &mut w.defs[cur].modes[0];
                            m.wait_for_release = !m.wait_for_release;
                            let semi = m.wait_for_release;
                            runlog::kv("fire_mode_toggle", &format!("weapon={} semi_auto={semi}", w.defs[cur].item_name));
                        }
                        AltToggle::WideSpread => {
                            w.defs[cur].wide_spread = !w.defs[cur].wide_spread;
                            let wide = w.defs[cur].wide_spread;
                            runlog::kv("fire_mode_toggle", &format!("weapon={} wide_spread={wide}", w.defs[cur].item_name));
                        }
                    }
                } else {
                    runlog::kv("fire_mode_toggle_refused", &format!("weapon={} action={:?}", w.defs[cur].item_name, w.action));
                }
            }
            continue;
        }
        let fm = w.defs[cur].modes[mode].clone();
        if let Some(bf) = fm.beam.clone() {
            // ZEDGunAltFire.AllowFire: not reloading, a round in the magazine.
            let reloading = w.action == Action::Reload;
            let mag_ok = !reloading && w.defs[cur].ammo.is_none_or(|a| a.mag >= 1);
            if w.beam.is_none() {
                let exclusive = fm.mode_exclusive || w.defs[cur].modes[alt].mode_exclusive;
                let alt_busy = exclusive && (w.firing[alt] || w.fire_cooldown[alt] > 0.0);
                if w.action != Action::Idle || alt_busy || w.fire_cooldown[mode] > 0.0 || !mag_ok {
                    continue;
                }
                w.beam = Some(BeamState { charge_up: 0.0, up_time: 0.0, do_hit: false });
                w.firing[mode] = true;
                w.fire_cooldown[mode] = 0.0;
            }
            let dt = time.delta_secs();
            let mut st = w.beam.expect("set above");
            // ModeDoFire every FireRate: DoFireEffect sets bDoHit and UpTime.
            if mag_ok && w.fire_cooldown[mode] <= 0.0 {
                w.fire_cooldown[mode] += fm.rate;
                st.do_hit = true;
                st.up_time = fm.rate + 0.1;
            }
            // ModeTick.
            if mag_ok && st.up_time > 0.0 {
                st.up_time -= dt;
                if st.charge_up == 0.0 {
                    // PlayPreFire.
                    play(&mut w, "Charge", 1.0, false);
                    runlog::kv("beam_start", &format!("weapon={}", w.defs[cur].item_name));
                }
                st.charge_up += dt;
                if st.do_hit {
                    if let Some(a) = w.defs[cur].ammo.as_mut() {
                        a.mag = a.mag.saturating_sub(1);
                    }
                    // ZEDGunAltFire.HandleRecoil: no movement term.
                    let p = crate::firing::RecoilParams { right_only: false, velocity_scale: 0.0, ..fm.recoil };
                    let r = [w.random(), w.random(), w.random()];
                    let kick = crate::firing::recoil_kick(p, 0.0, health.health, 100.0, r);
                    recoil.add(kick, fm.recoil.rate, now);
                }
                if let Ok((cam, _)) = main_cam.single() {
                    let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
                    let eye = to_ue(cam.translation) / coords::SCALE;
                    let (x, y, z) = (to_ue(*cam.forward()), to_ue(*cam.right()), to_ue(*cam.up()));
                    let scale = (st.charge_up / bf.sphere_time).min(1.0);
                    beam_zaps.write(crate::projectile::BeamZap {
                        start: eye + x * bf.offset.x + y * bf.offset.y + z * bf.offset.z,
                        dir: x,
                        range: bf.range,
                        dt,
                        do_hit: st.do_hit,
                        sphere_radius: BEAM_SPHERE_RADIUS * scale,
                        sphere_amount: fm.rate * 0.75,
                    });
                }
                st.do_hit = false;
                w.beam = Some(st);
            } else {
                // StopFiring, then the Timer: PlayFireEnd (ChargeDown), StopFire.
                w.beam = None;
                w.firing[mode] = false;
                play(&mut w, "ChargeDown", 1.0, false);
                runlog::kv("beam_stop", &format!("weapon={} reason=cannot_fire", w.defs[cur].item_name));
            }
            continue;
        }
        if fm.weld {
            weld_fire(&mut w, mode, &fm, weld_view.door, now);
            continue;
        }
        if let Some(h) = w.defs[cur].heal_charge.filter(|h| h.syringe) {
            syringe_fire(&mut w, mode, pressed[mode], h, health.health, now);
            continue;
        }
        if mode == 1 && !matches!(fm.kind, FireKind::Melee | FireKind::Pellets) {
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
        // Rounds a shot needs: AmmoPerFire for pellet fire (BoomStickFire:
        // both barrels, 2); 1 for other primary fire; none for melee alt.
        let needs = match fm.pellets {
            Some(pf) => pf.ammo_per_fire.max(1),
            None => u32::from(mode == 0),
        };
        let total = w.defs[cur].ammo.map(|a| a.mag + a.spare);
        // The alt fire's own pool: the M4 203's grenades, or a medic gun's
        // HealAmmoCharge.
        let medic_charge = w.defs[cur].heal_charge.filter(|h| !h.syringe);
        let alt_pool = if mode == 1 { w.defs[cur].alt_ammo.map(|a| a.0).or(medic_charge.map(|h| h.charge)) } else { None };
        let aimed_in = w.aiming && w.zoom >= 1.0;
        let allow_fire = if let (Some(left), Some(_)) = (alt_pool, medic_charge) {
            // MP7MAltFire.AllowFire: not while reloading; HealAmmoCharge >= AmmoPerFire.
            !reloading && left >= needs
        } else if let Some(left) = alt_pool {
            // M203Fire.AllowFire: AmmoAmount(1) >= AmmoPerFire.
            left >= needs
        } else if fm.total_ammo_only {
            (!fm.requires_aim || aimed_in) && total.is_none_or(|t| t >= needs)
        } else {
            (!reloading || (fm.fire_while_reloading && mag.is_some_and(|m| m >= 2))) && mag.is_none_or(|m| m >= needs)
        };
        if !w.firing[mode] {
            // StartFire. bWaitForRelease modes need a fresh click (a click
            // made while not ready is kept while the button stays down; not
            // checked against KF). Weapon.ReadyToFire: not while the other
            // mode fires, and after both modes' NextFireTime.
            if fm.wait_for_release && !w.press_waiting[mode] {
                continue;
            }
            // KFWeapon.Fire: a click on an empty magazine asks for a reload.
            if mode == 0 && !fm.total_ammo_only && pressed[mode] && mag == Some(0) && !reloading && w.fire_cooldown[0] <= 0.0 {
                runlog::kv("dry_fire", &format!("weapon={} auto_reload={}", w.defs[cur].item_name, w.defs[cur].can_dry_fire));
                if w.defs[cur].can_dry_fire && allow_reload(&w) {
                    start_reload(&mut w, "dry_fire");
                }
                continue;
            }
            // Weapon.ReadyToFire: the other mode blocks only if either is
            // bModeExclusive.
            let exclusive = fm.mode_exclusive || w.defs[cur].modes[alt].mode_exclusive;
            let alt_busy = exclusive && (w.firing[alt] || w.fire_cooldown[alt] > 0.0);
            if !ready_state || alt_busy || w.fire_cooldown[mode] > 0.0 || !allow_fire {
                continue;
            }
            w.firing[mode] = true;
            w.press_waiting[mode] = false;
            w.shots_this_press[mode] = 0;
            // HuskGunFire: PlayPreFire (Charge / Charge_Iron), then hold.
            if fm.charge.is_some() && mode == 0 {
                w.charge_hold = Some(0.0);
                let anim = if w.aiming { "Charge_Iron" } else { "Charge" };
                play(&mut w, anim, 1.0, false);
                runlog::kv("charge_start", &format!("weapon={}", w.defs[cur].item_name));
                continue;
            }
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
        // Charging: HoldTime grows until the release.
        if fm.charge.is_some() && charge_release.is_none() {
            if let Some(h) = w.charge_hold.as_mut() {
                *h += time.delta_secs();
            }
            continue;
        }
        // ModeDoFire: once per press in semi auto, every FireRate in full auto.
        if w.fire_cooldown[mode] > 0.0 || !allow_fire || (fm.wait_for_release && w.shots_this_press[mode] > 0) {
            if charge_release.is_some() {
                w.firing[mode] = false;
                runlog::kv("charge_dropped", &format!("weapon={} allow_fire={allow_fire}", w.defs[cur].item_name));
            }
            // KFHighROFFire.ModeTick: an empty magazine ends the loop.
            if fm.high_rof && !fm.wait_for_release && !allow_fire && w.looping && w.firing[mode] {
                w.firing[mode] = false;
                play_fire_end(&mut w, mode);
            }
            continue;
        }
        // BoomStick: bVeryLastShotAnim = AmmoAmount <= AmmoPerFire (before
        // the shot); the gun is emptied when the magazine reaches 0.
        let total_before = w.defs[cur].ammo.map_or(0, |a| a.mag + a.spare);
        // PipeBombFire: the ammo goes when the projectile spawns.
        let needs = if fm.spawn_delay.is_some() { 0 } else { needs };
        let needs = match (&fm.charge, charge_release) {
            (Some(c), Some(h)) => c.ammo(h, total.unwrap_or(0)),
            _ => needs,
        };
        if needs > 0 {
            if alt_pool.is_some() {
                if let Some(h) = w.defs[cur].heal_charge.as_mut().filter(|h| !h.syringe) {
                    h.charge -= needs.min(h.charge);
                } else if let Some(a) = w.defs[cur].alt_ammo.as_mut() {
                    a.0 -= needs.min(a.0);
                }
            } else if let Some(a) = w.defs[cur].ammo.as_mut() {
                // Total-only fire takes from the spare rounds once the
                // magazine is empty (my choice for the HUD; KF counts one total).
                let from_mag = needs.min(a.mag);
                a.mag -= from_mag;
                if fm.total_ammo_only {
                    a.spare -= (needs - from_mag).min(a.spare);
                }
            }
        }
        let emptied = w.defs[cur].ammo.is_some_and(|a| a.mag == 0);
        let very_last = total_before <= needs;
        let last_anim = match fm.last_rule {
            LastShot::WhenEmptied => emptied && !very_last,
            LastShot::WhenVeryLast => very_last,
            LastShot::Never => false,
        };
        // ModeDoFire: NextFireTime = max(NextFireTime + FireRate, now); the
        // BoomStick waits FireLastRate after emptying the gun.
        let rate = match fm.last_rate {
            Some(r) if emptied => r,
            _ => fm.rate,
        };
        w.fire_cooldown[mode] = (w.fire_cooldown[mode] + rate).max(0.0);
        if emptied && let Some(t) = w.defs[cur].boomstick_reload {
            w.boomstick_pending = Some(t);
        }
        w.fire_count += 1;
        // Dual pistols: the hand whose turn it is (DualiesFire.ModeDoFire).
        let dual_side = if w.aiming { 1 } else { 0 };
        let hand = usize::from(w.defs[cur].dual && mode == 0 && w.dual_left[dual_side]);
        if mode == 0 || fm.kind == FireKind::Pellets {
            w.fx_shots.push(hand);
        }
        if !(fm.high_rof && !fm.wait_for_release) {
            play_firing(&mut w, mode, last_anim);
        }
        if w.defs[cur].dual && mode == 0 {
            w.dual_left[dual_side] = !w.dual_left[dual_side];
        }
        w.shots_this_press[mode] += 1;
        let stats = fm.combat;
        let item_name = w.defs[cur].item_name;
        match fm.kind {
            FireKind::Melee => {
                let mut stats = stats;
                if fm.chainsaw {
                    // ChainsawFire.ModeDoFire / DoFireEffect: damage now,
                    // MeleeDamage + Rand(maxAdditionalDamage), traced only.
                    stats.damage_delay = 0.0;
                    stats.damage_min += (w.random() * fm.extra_damage as f32).floor().min(fm.extra_damage.saturating_sub(1) as f32);
                    stats.min_dot = 0.0;
                }
                // KFMeleeFire.ModeDoFire: velocity x ChopSlowRate on the ground.
                let chop = w.defs[cur].chop_slow_rate;
                if chop < 1.0
                    && main_cam.single().is_ok_and(|(_, wk)| wk.is_some_and(|wk| wk.on_ground))
                    && let Some(e) = effects.as_mut()
                {
                    e.fire_velocity_scale = Some(chop);
                }
                runlog::kv(
                    "melee_swing",
                    &format!(
                        "weapon={item_name} mode={mode} class={} damage={} range={} delay={} min_dot={} chop_slow={chop}",
                        fm.class, stats.damage_min, stats.range, stats.damage_delay, stats.min_dot
                    ),
                );
                w.pending_swings.push((stats.damage_delay, stats, item_name));
            }
            FireKind::Pellets if fm.spawn_delay.is_some() => {
                let delay = fm.spawn_delay.unwrap_or(0.0);
                w.pending_spawn = Some((delay, cur, mode));
                runlog::kv("projectile_spawn_delayed", &format!("weapon={item_name} delay={delay}"));
            }
            FireKind::Pellets => {
                let mut pf = fm.pellets.expect("pellet fire has pellet values");
                if let (Some(c), Some(h)) = (&fm.charge, charge_release) {
                    let x = c.projectile(h);
                    runlog::kv(
                        "charge_fired",
                        &format!(
                            "weapon={item_name} hold={h:.2} class={} ammo={needs} damage={:.1} radius={:.0} impact={:.0}",
                            x.class, x.damage, x.radius, x.impact_damage
                        ),
                    );
                    pf.explosive = Some(x);
                    // ModeDoFire with bFireOnRelease: NextFireTime = now + FireRate.
                    w.firing[mode] = false;
                }
                if let Ok((cam, walker)) = main_cam.single() {
                    let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
                    let eye = to_ue(cam.translation) / coords::SCALE;
                    let (x, y, z) = (to_ue(*cam.forward()), to_ue(*cam.right()), to_ue(*cam.up()));
                    // KFShotgunFire.DoFireEffect: StartProj = eye + X x
                    // ProjSpawnOffset.X, plus Y and Z offsets from the hip.
                    let mut start = eye + x * pf.spawn_offset.x;
                    if !w.aiming {
                        start += y * pf.spawn_offset.y + z * pf.spawn_offset.z;
                    }
                    // KSGFire: wide spread x 2.05.
                    let spread = if w.defs[cur].wide_spread { pf.spread * 2.05 } else { pf.spread };
                    // KFShotgunFire: ProjPerFire x Load; MP7MAltFire /
                    // M7A3MAltFire: ProjPerFire only (Load is the 250 charge).
                    let count = if pf.per_load { (pf.per_fire * pf.ammo_per_fire.max(1)).max(1) } else { pf.per_fire.max(1) };
                    let tip = w.hand_frames.get(hand).and_then(|h| h.0).map(|t| t.0);
                    for _ in 0..count {
                        // SS_Random: X >> R, R = Spread x (FRand() - 0.5) each.
                        let r = ue_assets::properties::Rotator {
                            yaw: (spread * (w.random() - 0.5)) as i32,
                            pitch: (spread * (w.random() - 0.5)) as i32,
                            roll: (spread * (w.random() - 0.5)) as i32,
                        };
                        let dir = coords::ue_rotation_matrix(r) * x;
                        pellets.write(crate::projectile::SpawnPlayerProjectile {
                            origin: start,
                            trace_from: eye,
                            dir,
                            stats: pf.stats,
                            weapon: item_name,
                            tracer_start: if pf.explosive.is_some() || pf.thrown.is_some() || pf.flame.is_some() || pf.dart.is_some() {
                                None
                            } else {
                                tip
                            },
                            explosive: pf.explosive,
                            thrown: pf.thrown,
                            flame: pf.flame,
                            dart: pf.dart,
                            extra_speed: 0.0,
                        });
                    }
                    // AddVelocity(KickMomentum >> view rotation), not when falling.
                    let on_ground = walker.is_some_and(|wk| wk.on_ground);
                    if on_ground && pf.kick != Vec3::ZERO {
                        kicks.write(crate::walk::PlayerAddVelocity {
                            velocity: x * pf.kick.x + y * pf.kick.y + z * pf.kick.z,
                        });
                    }
                    if fm.slows_movement && on_ground {
                        let scale = if fm.rate > 0.25 { 0.1 } else { 0.5 };
                        if let Some(e) = effects.as_mut() {
                            e.fire_velocity_scale = Some(scale);
                        }
                    }
                    let speed = walker.map_or(0.0, |wk| wk.velocity.length() / coords::SCALE);
                    let r = [w.random(), w.random(), w.random()];
                    let kick = crate::firing::recoil_kick(fm.recoil, speed, health.health, 100.0, r);
                    recoil.add(kick, fm.recoil.rate, now);
                    runlog::kv(
                        "pellet_shot",
                        &format!(
                            "weapon={item_name} mode={mode} anim={} pellets={count} damage={} spread={spread:.0} wide={} kick_unreal=({:.0}, {:.0}, {:.0}) recoil_pitch={:.0} mag_left={} emptied={emptied} last_anim={last_anim}",
                            w.anim,
                            pf.stats.damage,
                            w.defs[cur].wide_spread,
                            pf.kick.x,
                            pf.kick.y,
                            pf.kick.z,
                            kick.0,
                            w.defs[cur].ammo.map_or(0, |a| a.mag)
                        ),
                    );
                }
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
                        effect_start: w.hand_frames.get(hand).and_then(|h| h.0).map(|t| t.0),
                        max_penetrations: fm.penetrations,
                        fire: fm.fire,
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
                            "weapon={item_name} hand={hand} anim={} shot_in_press={} semi_auto={} aiming={} spread={spread:.4} burst={} recoil_pitch={:.0} recoil_yaw={:.0} speed_unreal={speed:.0} mag_left={}",
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
    // WeldFire.Timer: the weld lands DamagedelayMin after the fire, traced
    // from where the view is then.
    let mut welds_landed = Vec::new();
    w.pending_welds.retain_mut(|(t, damage, range, unweld)| {
        *t -= dt;
        if *t <= 0.0 {
            welds_landed.push((*damage, *range, *unweld));
        }
        *t > 0.0
    });
    for (damage, range, unweld) in welds_landed {
        if let Ok((cam, _)) = main_cam.single() {
            weld_hits.write(crate::door::WeldHit {
                origin: cam.translation,
                dir: *cam.forward(),
                range,
                damage,
                unweld,
            });
        }
    }
    // Reloading (KFWeapon.Tick): after ReloadRate the magazine is full and
    // the weapon idles (ClientFinishReloading), even if the reload
    // animation is still playing. One-round reloads add a round every
    // ReloadRate until full.
    // PipeBombFire.Timer: ConsumeAmmo, DoFireEffect; out of ammo: the
    // weapon is gone and the best other one comes up.
    if let Some((t, wi, mode)) = w.pending_spawn {
        let t = t - dt;
        if t > 0.0 {
            w.pending_spawn = Some((t, wi, mode));
        } else {
            w.pending_spawn = None;
            let pf = w.defs[wi].modes[mode].pellets;
            if let Some(a) = w.defs[wi].ammo.as_mut() {
                if a.mag > 0 {
                    a.mag -= 1;
                } else {
                    a.spare = a.spare.saturating_sub(1);
                }
            }
            if let (Some(pf), Ok((cam, _))) = (pf, main_cam.single()) {
                let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
                let eye = to_ue(cam.translation) / coords::SCALE;
                let (x, y, z) = (to_ue(*cam.forward()), to_ue(*cam.right()), to_ue(*cam.up()));
                let mut start = eye + x * pf.spawn_offset.x;
                if !w.aiming {
                    start += y * pf.spawn_offset.y + z * pf.spawn_offset.z;
                }
                let r = ue_assets::properties::Rotator {
                    yaw: (pf.spread * (w.random() - 0.5)) as i32,
                    pitch: (pf.spread * (w.random() - 0.5)) as i32,
                    roll: (pf.spread * (w.random() - 0.5)) as i32,
                };
                pellets.write(crate::projectile::SpawnPlayerProjectile {
                    origin: start,
                    trace_from: eye,
                    dir: coords::ue_rotation_matrix(r) * x,
                    stats: pf.stats,
                    weapon: w.defs[wi].item_name,
                    tracer_start: None,
                    explosive: pf.explosive,
                    thrown: pf.thrown,
                    flame: pf.flame,
                    dart: pf.dart,
                    extra_speed: 0.0,
                });
            }
            let left = w.defs[wi].ammo.map_or(0, |a| a.mag + a.spare);
            if left == 0 {
                w.defs[wi].gone = true;
                runlog::kv("weapon_gone", &format!("weapon={}", w.defs[wi].item_name));
                if w.current == wi
                    && !matches!(w.action, Action::Grenade { .. } | Action::PutDown { .. })
                    && let Some(next) = step_weapon(&slots(&w.defs), wi, false)
                {
                    w.firing = [false; 2];
                    set_action(&mut w, Action::PutDown { next });
                }
            }
        }
    }
    // BoomStick.WeaponTick: ReloadCountDown after the last barrel, both
    // load (MagAmmoRemaining = Min(AmmoAmount, 2)); only while in hand.
    if let Some(t) = w.boomstick_pending
        && w.defs[w.current].boomstick_reload.is_some()
    {
        let t = t - dt;
        if t <= 0.0 {
            w.boomstick_pending = None;
            let cur = w.current;
            if let Some(a) = w.defs[cur].ammo.as_mut() {
                let n = (a.capacity - a.mag).min(a.spare);
                a.mag += n;
                a.spare -= n;
                runlog::kv("boomstick_loaded", &format!("mag={} spare={}", a.mag, a.spare));
            }
        } else {
            w.boomstick_pending = Some(t);
        }
    }
    // Weapon.Timer: the bring-up ends in idle; the put-down plays its
    // animation after any DownDelay, then the next weapon comes up.
    if matches!(w.action, Action::Select | Action::PutDown { .. } | Action::Grenade { .. }) {
        w.switch_timer -= dt;
        if w.switch_timer <= 0.0 {
            match w.action {
                Action::Grenade { phase, back_to } => {
                    let frag = w.defs.iter().position(|d| d.toss.is_some()).unwrap_or(back_to);
                    match phase {
                        NadePhase::Down => {
                            // KFPawn.WeaponDown -> Frag.StartThrow: TossAnim.
                            w.current = frag;
                            let (anim, _, spawn_at) = w.defs[frag].toss.clone().unwrap_or_default();
                            play(&mut w, &anim, 1.0, false);
                            w.switch_timer = spawn_at;
                            w.action = Action::Grenade { phase: NadePhase::Toss { spawned: false }, back_to };
                        }
                        NadePhase::Toss { spawned: false } => {
                            // Frag.ServerThrow: ConsumeAmmo, FragFire.DoFireEffect.
                            if let Some(a) = w.defs[frag].ammo.as_mut() {
                                if a.mag > 0 {
                                    a.mag -= 1;
                                } else {
                                    a.spare = a.spare.saturating_sub(1);
                                }
                            }
                            let fm = w.defs[frag].modes[0].clone();
                            if let (Some(pf), Ok((cam, walker))) = (fm.pellets, main_cam.single())
                                && let Some(t) = pf.thrown
                            {
                                let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
                                let eye = to_ue(cam.translation) / coords::SCALE;
                                let (x, y, z) = (to_ue(*cam.forward()), to_ue(*cam.right()), to_ue(*cam.up()));
                                // StartProj = eye + X x 25, + Hand (right, 1) x Y x -10 + Z x 0.
                                let start = eye + x * pf.spawn_offset.x + y * pf.spawn_offset.y + z * pf.spawn_offset.z;
                                // PostSpawnProjectile: + the player's speed along the view.
                                let pawn_speed = walker.map_or(0.0, |wk| x.dot(to_ue(wk.velocity) / coords::SCALE));
                                pellets.write(crate::projectile::SpawnPlayerProjectile {
                                    origin: start,
                                    trace_from: eye,
                                    dir: x,
                                    stats: pf.stats,
                                    weapon: w.defs[frag].item_name,
                                    tracer_start: None,
                                    explosive: None,
                                    thrown: Some(t),
                                    flame: None,
                                    dart: None,
                                    extra_speed: pawn_speed,
                                });
                            }
                            let (_, toss_time, spawn_at) = w.defs[frag].toss.clone().unwrap_or_default();
                            w.switch_timer = (toss_time - spawn_at).max(0.0);
                            w.action = Action::Grenade { phase: NadePhase::Toss { spawned: true }, back_to };
                        }
                        NadePhase::Toss { spawned: true } => {
                            // ThrowGrenadeFinished: the weapon comes up quickly.
                            w.current = back_to;
                            let d = &w.defs[back_to];
                            let (anim, rate, time) =
                                (d.select_anim.clone(), d.select_anim_rate * d.bring_up_time / d.quick_bring_up_time, d.quick_bring_up_time);
                            play(&mut w, &anim, rate, false);
                            w.switch_timer = time;
                            w.action = Action::Grenade { phase: NadePhase::Up, back_to };
                        }
                        NadePhase::Up => set_action(&mut w, Action::Idle),
                    }
                }
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
    ammo_display.alt_ammo = w.defs[w.current].alt_ammo.map(|a| a.0);
    ammo_display.syringe = w.defs.iter().find_map(|d| d.heal_charge.filter(|h| h.syringe)).map(|h| h.charge * 100 / HEAL_CHARGE_MAX);
    ammo_display.heal = w.defs[w.current].heal_charge.filter(|h| !h.syringe).map(|h| h.charge * 100 / HEAL_CHARGE_MAX);
    // CrossbowArrow pickups: room for one more bolt?
    if let Some(d) = w.defs.iter().find(|d| d.class.eq_ignore_ascii_case("KFMod.Crossbow")) {
        bolt_room.0 = d.ammo.is_some_and(|a| a.mag + a.spare < a.max_total);
    }
    for _ in bolts_picked.read() {
        if let Some(a) = w.defs.iter_mut().find(|d| d.class.eq_ignore_ascii_case("KFMod.Crossbow")).and_then(|d| d.ammo.as_mut()) {
            a.spare += 1;
        }
    }
    ammo_display.frags = w.defs.iter().find(|d| d.toss.is_some()).and_then(|d| d.ammo).map(|a| a.mag + a.spare);
    ammo_display.fire_mode = match w.defs[w.current].toggles_on_alt {
        Some(AltToggle::FireMode) => Some(if w.defs[w.current].modes[0].wait_for_release { "SEMI" } else { "AUTO" }),
        Some(AltToggle::WideSpread) => Some(if w.defs[w.current].wide_spread { "WIDE" } else { "NARROW" }),
        None => None,
    };
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
                Action::PutDown { .. } | Action::Select | Action::Grenade { .. } => w.frame = length,
                // Weapon.AnimEnd: after FireAnim comes FireEndAnim if the
                // weapon has it; otherwise idle unless a mode is firing
                // (then the last frame holds until the next shot).
                Action::Idle | Action::Reload => {
                    let next_end = (0..2).find_map(|m| {
                        let fm = &w.defs[w.current].modes[m];
                        (w.anim == fm.anims[0].to_ascii_lowercase() && has_anim(&w, &fm.end_anim))
                            .then(|| (fm.end_anim.clone(), fm.end_anim_rate))
                    });
                    // PipeBombExplosive.AnimEnd: after the toss, the next bomb
                    // comes out (SelectAnim) if there is one.
                    let def = &w.defs[w.current];
                    let pipe_next = def.modes[0].spawn_delay.is_some()
                        && w.anim == def.modes[0].anims[0].to_ascii_lowercase()
                        && def.ammo.is_some_and(|a| a.mag + a.spare > 0)
                        && !def.gone;
                    if pipe_next {
                        let (anim, rate) = (def.select_anim.clone(), def.select_anim_rate);
                        play(&mut w, &anim, rate, false);
                    } else if let Some((end, rate)) = next_end {
                        play(&mut w, &end, rate, false);
                    } else if w.charge_hold.is_some() {
                        // HuskGun.AnimEnd while charging: ChargeLoop(_Iron).
                        let anim = if w.aiming { "ChargeLoop_Iron" } else { "ChargeLoop" };
                        play(&mut w, anim, 1.0, true);
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
    let frames: Vec<_> = match main_cam.single() {
        Ok(main) => {
            let to_world = |bone: Option<usize>| -> Option<(Vec3, Mat3)> {
                let (o, axes) = def.model.bone_frame(&bones, bone?)?;
                let local = coords::pos(((o - origin) * scale).to_array()) + part_translation;
                let pos = to_ue(main.transform_point(local)) / coords::SCALE;
                let axes = axes.map(|a| to_ue(main.rotation * coords::dir((a * scale).to_array())).normalize_or_zero());
                Some((pos, Mat3::from_cols(axes[0], axes[1], axes[2])))
            };
            def.fx.hands.iter().map(|h| (to_world(h.flash_bone), to_world(h.shell_bone))).collect()
        }
        Err(_) => Vec::new(),
    };
    w.hand_frames = frames;
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
    mut charge_fx_failed: Local<bool>,
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
            let (flash_class, shell_class) = (def.fx.flash_class.clone(), def.fx.shell_class.clone());
            for hand in &mut def.fx.hands {
                for (class, slot) in [(&flash_class, &mut hand.flash), (&shell_class, &mut hand.shell)] {
                    if let (Some(class), None) = (class, &slot) {
                        *slot =
                            crate::particles::spawn_effect_with(&mut commands, lib, &mut meshes, class, Vec3::ZERO, Mat3::IDENTITY, 7, options);
                        if slot.is_none() {
                            runlog::kv("weapon_fx_missing", &format!("weapon={} class={class}", def.class));
                        }
                    }
                }
            }
        }
    }
    // HuskGunFire.InitChargeEffect / DestroyChargeEffect: the charge glow
    // on the 'tip' bone while charging. Its growth with the charge (Timer
    // changing emitter sizes) is not done.
    let tip = w.hand_frames.first().and_then(|h| h.0);
    match (w.charge_hold.is_some() || w.beam.is_some(), w.charge_fx) {
        (true, None) => {
            let modes = &w.defs[current].modes;
            let class = modes[0]
                .charge
                .as_ref()
                .and_then(|c| c.effect.clone())
                .or_else(|| modes[1].beam.as_ref().and_then(|b| b.effect.clone()));
            if let (Some(class), Some(lib)) = (class, library.as_deref()) {
                let options = crate::particles::SpawnOptions {
                    persistent: true,
                    layer: Some(WEAPON_LAYER),
                    ..default()
                };
                let (at, axes) = tip.unwrap_or((Vec3::ZERO, Mat3::IDENTITY));
                w.charge_fx = crate::particles::spawn_effect_with(&mut commands, lib, &mut meshes, &class, at, axes, 11, options);
                if w.charge_fx.is_some() || !*charge_fx_failed {
                    runlog::kv("charge_fx", &format!("class={class} spawned={}", w.charge_fx.is_some()));
                }
                *charge_fx_failed = w.charge_fx.is_none();
            }
        }
        (true, Some(e)) => {
            if let (Ok((mut fx, _)), Some(f)) = (effects.get_mut(e), tip) {
                fx.frame = f;
            }
        }
        (false, Some(e)) => {
            commands.entity(e).despawn();
            w.charge_fx = None;
        }
        (false, None) => {}
    }
    let shots = std::mem::take(&mut w.fx_shots);
    let frames = w.hand_frames.clone();
    for (i, def) in w.defs.iter().enumerate() {
        for (h, hand) in def.fx.hands.iter().enumerate() {
            let (tip, shell) = frames.get(h).copied().unwrap_or((None, None));
            for (entity, frame) in [(hand.flash, tip), (hand.shell, shell)] {
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
                for _ in shots.iter().filter(|&&s| s == h) {
                    fx.trigger();
                }
            }
        }
    }
    if !shots.is_empty() {
        let def = &w.defs[current];
        let fmt = |f: Option<(Vec3, Mat3)>| {
            f.map_or("none".to_string(), |(p, m)| {
                format!("({:.1}, {:.1}, {:.1}) x_axis=({:.2}, {:.2}, {:.2})", p.x, p.y, p.z, m.x_axis.x, m.x_axis.y, m.x_axis.z)
            })
        };
        for &h in &shots {
            let (tip, shell) = frames.get(h).copied().unwrap_or((None, None));
            runlog::kv(
                "weapon_fx",
                &format!(
                    "weapon={} hand={h} flash={} shell={} tip_unreal={} shell_unreal={}",
                    def.class,
                    def.fx.hands.get(h).is_some_and(|x| x.flash.is_some()),
                    def.fx.hands.get(h).is_some_and(|x| x.shell.is_some()),
                    fmt(tip),
                    fmt(shell)
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn husk_charge() -> ChargeFire {
        let x = crate::projectile::ExplosiveStats {
            class: "medium",
            speed: 1800.0,
            damage: 25.0,
            radius: 150.0,
            momentum: 0.0,
            impact_damage: 100.0,
            impact_headshot_mult: 1.5,
            impact_on_touch: Some(1.5),
            fire: Some(crate::combat::FireType::HuskGun),
            hurts_self: false,
            zap: None,
            arm_dist: 0.0,
            straight_time: None,
            life_span: 10.0,
            fleshpound_mult: None,
            effect: "",
            decal: crate::decals::DecalKind::BurnMedium,
            trail: None,
        };
        ChargeFire {
            max_time: 3.0,
            weak: crate::projectile::ExplosiveStats { class: "weak", ..x },
            medium: x,
            strong: crate::projectile::ExplosiveStats { class: "strong", ..x },
            effect: None,
        }
    }

    #[test]
    fn husk_charge_picks_and_scales_the_fireball() {
        let c = husk_charge();
        // A tap: weak, impact x 0 (HoldTime x 2.5), 1 round.
        let x = c.projectile(0.0);
        assert_eq!((x.class, x.impact_damage, x.damage, x.radius), ("weak", 0.0, 25.0, 150.0));
        assert_eq!(c.ammo(0.0, 75), 1);
        // 1.5 s: the middle class, x 1.5 damage, x 2 radius, 375 impact, 5 rounds.
        let x = c.projectile(1.5);
        assert_eq!((x.class, x.impact_damage, x.damage, x.radius), ("medium", 375.0, 37.5, 300.0));
        assert_eq!(c.ammo(1.5, 75), 5);
        // Held past full: strong, x 7.5 impact, x 2 damage, x 3 radius, 10 rounds
        // (or what is left).
        let x = c.projectile(5.0);
        assert_eq!((x.class, x.impact_damage, x.damage, x.radius), ("strong", 750.0, 50.0, 450.0));
        assert_eq!(c.ammo(5.0, 75), 10);
        assert_eq!(c.ammo(5.0, 4), 4);
    }

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
    fn dualies_take_the_single_pistols_rounds() {
        let single = Ammo { mag: 15, spare: 105, capacity: 15, initial: 120, max_total: 240 };
        let dual = Ammo { mag: 30, spare: 90, capacity: 30, initial: 120, max_total: 240 };
        let m = merge_dual_ammo(single, dual);
        assert_eq!((m.mag, m.spare), (30, 210));
        // A half-empty single: 7 + 15 in the magazine, 120 + 7 + 40 total.
        let single = Ammo { mag: 7, spare: 40, ..single };
        let m = merge_dual_ammo(single, dual);
        assert_eq!((m.mag, m.spare), (22, 145));
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
