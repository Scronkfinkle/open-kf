//! First-person weapons: skeletal mesh + animations, CPU skinning, a separate
//! weapon camera with the weapon's own field of view (like UE2's DisplayFOV,
//! which also keeps the weapon from clipping into walls).
//!
//! Mesh loading, animation sampling and skinning live in `skinned.rs`.

use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;

use crate::audio::mixer::{AmbientSound, Emitter, PlaySound, Slot as SoundSlot};

use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package::ObjectRef;
use ue_assets::package_set::{ObjectHandle, PackageSet};
use ue_assets::properties::Value;

use crate::engine::camera::FlyCamera;
use crate::engine::coords;
use crate::world::map::MapRequest;
use crate::engine::runlog;
use crate::game::combat::{MeleeSwing, ShotFired};
use crate::render::skinned::{SkinnedModel, Skins};

mod load;
mod input;
mod animate;
mod inventory;
mod sounds;
mod welder_screen;
mod torch;
mod perk;
mod sleeve;
mod pickup;
mod drop;
pub(crate) use load::*;
use input::*;
use animate::*;
use inventory::*;
use welder_screen::update_welder_screen;
use perk::*;
pub(crate) use sounds::*;
use torch::*;

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
            .init_resource::<crate::weapons::firing::Recoil>()
            .add_systems(
                Update,
                crate::weapons::firing::apply_recoil
                    .after(crate::engine::camera::look)
                    .before(crate::engine::camera::follow_sky),
            )
            .insert_non_send(WeaponAssets::default())
            .add_systems(PostStartup, load_weapons.after(crate::engine::camera::spawn_camera))
            .add_message::<crate::player::character::ChangeCharacter>()
            .add_systems(Update, (sync_perk, new_pawn_inventory, respawn_inventory, shop_requests, sleeve::change_character).chain().before(weapon_input))
            .add_systems(
                Update,
                (weapon_input, torch_update, animate_weapon, update_welder_screen, torch_beam.in_set(crate::weapons::flashlight::FlashlightBeamSet), weapon_fire_fx, weapon_loop_sound, send_weapon_sounds)
                    .chain()
                    .after(crate::engine::camera::follow_sky),
            )
            .add_systems(Update, publish_pawn_weapon.in_set(PublishPawnWeapon).after(weapon_input))
            .add_systems(Update, pickup::pickup_inventory.after(crate::game::pickups::PickupSystems::Touch).before(crate::game::pickups::PickupSystems::Answer))
            .init_resource::<drop::WeaponDrops>()
            .add_systems(Update, drop::drop_input.after(weapon_input).before(crate::game::pickups::PickupSystems::Rules))
            .add_systems(Update, drop::drop_results.after(crate::game::pickups::PickupSystems::Rules).before(crate::game::pickups::PickupSystems::Touch))
            .add_plugins(crate::weapons::flashlight::FlashlightPlugin);
    }
}

/// `publish_pawn_weapon`, for ordering.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct PublishPawnWeapon;

/// The weapon's part of the local pawn's `PawnState` (what KF's weapon
/// attachment replicates): the class in hand (Pawn.ChangedWeapon ->
/// AttachToPawn), the shot counter and firing mode (FlashCount /
/// FiringMode; FlashCount is zeroed when firing stops), and reload starts
/// (KFWeapon.ReloadMeNow: SetAnimAction(WeaponReloadAnim)).
fn publish_pawn_weapon(
    w: Option<Res<Weapons>>,
    mut pawns: Query<&mut crate::player::body::PawnState, With<FlyCamera>>,
    mut was_reloading: Local<bool>,
) {
    let (Some(w), Ok(mut s)) = (w, pawns.single_mut()) else { return };
    let class = w.defs.get(w.current).map(|d| d.class.clone());
    if s.weapon_class != class {
        s.weapon_class = class;
    }
    let shots = w.fire_count as u32;
    if shots != s.flash_count {
        s.firing_mode = if w.firing[1] && !w.firing[0] { 1 } else { 0 };
        s.flash_count = shots;
    }
    s.firing = w.firing[0] || w.firing[1];
    let reloading = w.action == Action::Reload;
    if reloading && !*was_reloading {
        s.reloads = s.reloads.wrapping_add(1);
    }
    *was_reloading = reloading;
}

/// Scripted input for tests: at frame N do an action ("fire", "fire_down" /
/// "fire_up" = hold / release, the same for "altfire", "1" to "5" =
/// weapon slot keys, "next" / "prev" = mouse wheel, "reload", "aim" = toggle iron sights, "walk_on" / "walk_off" = hold / release forward (walk.rs), "zed" = spawn a Clot, "zed_drop" =
/// spawn one 200 units up, "gorefast" = spawn a Gorefast,
/// "gorefast_far" = one 900 units away, "zed_line" = three Clots in a
/// line ahead ("zed_line_far": 700-900 away), "cycle_zed" = press N,
/// "spawn_<kind>" = spawn that zed, "hurt_zeds" = 100 damage to every zed,
/// "kill_boss" = the Patriarch dies, "warp_shop" = stand in the current shop,
/// "turn:DEG" = turn the view, "aim_zed" = look at the nearest zed's head,
/// "add_dosh" = + 5000, "buy_menu" / "menu_*" / "buy:C" / "sell:C" /
/// "ammo_fill:C" / "ammo_clip:C" = the buy menu, buy_menu.rs).
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
    /// The perk's GetMovementSpeedModifier (GroundSpeed x this, after the
    /// weight and the melee bonus); 1 without a perk.
    pub perk_speed_mult: f32,
}

impl Default for WeaponEffects {
    fn default() -> Self {
        WeaponEffects {
            ground_speed_bonus: 0.0,
            weight_speed_mult: 1.0,
            fire_velocity_scale: None,
            perk_speed_mult: 1.0,
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
    /// KFWeapon.SleeveNum: the skin slot the character's sleeves go on.
    sleeve_num: usize,
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
    /// SelectSound (KFWeapon.BringUp) at the weapon's TransientSoundVolume
    /// (KFWeapon: 100; capped by the mixer).
    select_sound: Option<String>,
    select_volume: f32,
    /// PickupClass (full path): what a thrown or dropped copy becomes.
    pickup_class: Option<String>,
    /// The pickup class's PickupSound, played when bought
    /// (KFTab_BuyMenu.MakeSomeBuyNoise).
    pickup_sound: Option<String>,
    /// The third-person attachment's own AmbientSound (AttachmentClass
    /// defaults: AmbientSound, SoundVolume, SoundRadius), playing while the
    /// weapon is in hand: the chainsaw's idle engine.
    idle_ambient: Option<AmbientSound>,
    /// Frag.ThrowSound (ServerThrow), the grenade leaving the hand.
    throw_sound: Option<String>,
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
    /// KFWeapon.SellValue (None = -1: never bought, sells for 75% of Cost).
    sell_value: Option<f32>,
    /// bKFNeverThrow: cannot be sold.
    never_throw: bool,
    /// bTorchEnabled: the weapon's flashlight (torch.rs).
    torch: Option<TorchDef>,
    /// The class chain and pickup class, for the perks.
    perk: crate::game::perks::PerkWeapon,
    /// The ammo classes (primary, the alt fire's own), for AddExtraAmmoFor.
    ammo_class: Option<crate::game::perks::ClassChain>,
    alt_ammo_class: Option<crate::game::perks::ClassChain>,
    /// The alt ammo's default MaxAmmo.
    alt_default_max: u32,
    /// The ammo classes' AmmoPickupAmount, None if they take no ammo
    /// boxes (not a KFAmmunition, or bAcceptsAmmoPickups false).
    pickup_amount: Option<u32>,
    alt_pickup_amount: Option<u32>,
    /// KFHumanPawn.ChangedWeapon: bSpeedMeUp (`speed_bonus` is then
    /// GroundSpeed x (BaseMeleeIncrease + the perk's melee speed) - Weight x 2).
    speed_me_up: bool,
    /// The default ReloadRate and ReloadAnimRate; the perk's reload speed
    /// sets `reload_rate` / `reload_anim_rate` at each reload (ReloadMeNow).
    base_reload_rate: f32,
    base_reload_anim_rate: f32,
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

/// A fire mode's sounds (WeaponFire, KFFire, KFHighROFFire), as full
/// object paths. See DESIGN.md, "Sound and music", S3a.
#[derive(Clone, Debug, Default)]
struct FireSounds {
    /// FireSound / FireSoundRef.
    fire: Option<String>,
    /// KFFire / KFShotgunFire: StereoFireSound, played instead of FireSound
    /// in first person (at 0.85 x the volume). None for other fire classes.
    stereo: Option<String>,
    /// NoAmmoSound: KFWeapon.Fire's dry click.
    no_ammo: Option<String>,
    /// TransientSoundVolume (WeaponFire default 0.5) and
    /// TransientSoundRadius (400).
    volume: f32,
    radius: f32,
    /// KFFire family with bRandomPitchFireSound: RandomPitchAdjustAmt
    /// (pitch 1 +- up to this); 0 for none.
    random_pitch: f32,
    /// KFHighROFFire, FlameBurstFire and ChainsawFire while held, and
    /// HuskGunFire fully charged: AmbientFireSound loops
    /// (AmbientFireVolume 0-255, AmbientFireSoundRadius), then
    /// FireEndStereoSound (first person) at AmbientFireVolume / 127.
    ambient: Option<String>,
    end_stereo: Option<String>,
    ambient_volume: u8,
    ambient_radius: f32,
    /// KFMeleeFire: MeleeHitSounds (or MeleeHitSoundRefs), one at random
    /// per zed hit, at MeleeHitVolume (default 1).
    melee_hits: std::sync::Arc<[String]>,
    melee_hit_volume: f32,
    /// ChainsawFire: FireStartSound when the loop starts; the loop sound
    /// waits until it has played (Timer after GetSoundDuration), and after
    /// FireEndSound the idle sound waits the same way.
    fire_start: Option<String>,
    chainsaw: bool,
    /// HuskGunFire and ZEDGunAltFire: AmbientChargeUpSound while charging,
    /// AmbientFireSound once charged for MaxChargeTime (Husk 3 s, ZED 1 s).
    charge_up: Option<String>,
    charge_max: f32,
    /// PipeBombFire.Timer: Sound'KF_AxeSnd.Axe_Fire' (written in the
    /// script) as the bomb is placed, SLOT_Interact, TransientSoundVolume.
    placed: Option<String>,
    /// FlameBurstFire.AllowFire: NoAmmoSound at TransientSoundVolume,
    /// SLOT_Interact, at most every FireRate while held on empty.
    empty_click: bool,
}

/// One fire mode (a WeaponFire class): what it does, its animations and timing.
#[derive(Clone, Debug)]
struct FireMode {
    kind: FireKind,
    sounds: FireSounds,
    /// WeldFire / UnWeldFire: needs a weldable door in view (door.rs).
    weld: bool,
    /// bModeExclusive: false lets the other mode fire at the same time
    /// (the ZED MKII).
    mode_exclusive: bool,
    /// ZEDGunAltFire's zapping beam.
    beam: Option<BeamFire>,
    /// HuskGunFire's charged release.
    charge: Option<ChargeFire>,
    /// The bullets' damage type burns (instant fire; W7).
    fire: Option<crate::game::combat::FireType>,
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
    spread: crate::weapons::firing::SpreadParams,
    recoil: crate::weapons::firing::RecoilParams,
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
    /// MAC10Fire.DoTrace: the perk's GetMAC10DamageType (the Firebug's
    /// DamTypeMAC10MPInc burns).
    mac10_inc: Option<crate::game::perks::DamType>,
    /// This fire class's ModeDoFire applies the perk's GetFireSpeedMod /
    /// ModifyRecoilSpread (KFFire, KFShotgunFire, KFMeleeFire and the
    /// overrides that copy them; see DESIGN.md, "Perks").
    perk_speed: bool,
    perk_recoil: bool,
    /// The class defaults FireRate, FireAnimRate and DamagedelayMin: the
    /// perk's fire speed divides / multiplies these into `rate`,
    /// `anim_rate` and `combat.damage_delay` (ModeDoFire).
    base_rate: f32,
    base_anim_rate: f32,
    base_damage_delay: f32,
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
    stats: crate::weapons::projectile::ProjectileStats,
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
    explosive: Option<crate::weapons::projectile::ExplosiveStats>,
    /// A thrown frag or pipe bomb instead.
    thrown: Option<crate::weapons::projectile::ThrownStats>,
    /// Flamethrower flames (FlameTendril) instead.
    flame: Option<crate::weapons::projectile::FlameStats>,
    /// Medic darts (HealingProjectile) instead.
    dart: Option<crate::weapons::projectile::DartStats>,
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
    weak: crate::weapons::projectile::ExplosiveStats,
    medium: crate::weapons::projectile::ExplosiveStats,
    strong: crate::weapons::projectile::ExplosiveStats,
    /// ChargeEmitterClass, on the 'tip' bone while charging.
    effect: Option<String>,
}

impl ChargeFire {
    /// The projectile for a release after `hold` seconds:
    /// GetDesiredProjectileClass, then PostSpawnProjectile's scaling
    /// (ImpactDamage x HoldTime x 2.5, Damage x (1 + HoldTime / Max),
    /// DamageRadius x (1 + HoldTime / (Max / 2)); at full charge x 7.5, x 2, x 3).
    fn projectile(&self, hold: f32) -> crate::weapons::projectile::ExplosiveStats {
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
    /// Other BaseProjectileFire: spawns ProjectileClass (not implemented;
    /// logged as fire_not_implemented).
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
    /// The damage type (KFFire DamageType, KFMeleeFire hitDamageClass), for
    /// the perks (AddDamage, GetHeadShotDamMulti).
    dam: Option<crate::game::perks::DamType>,
}

#[derive(Clone, Copy, Debug)]
struct Ammo {
    mag: u32,
    spare: u32,
    capacity: u32,
    /// The ammo class's InitialAmount and MaxAmmo (total, magazine included).
    initial: u32,
    max_total: u32,
    /// default.MagCapacity and the ammo class's default MaxAmmo, before the
    /// perk's GetMagCapacityMod / AddExtraAmmoFor.
    default_capacity: u32,
    default_max: u32,
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

/// A melee swing waiting for its damage moment: (seconds left, stats,
/// weapon name, MeleeHitSounds, MeleeHitVolume).
type PendingSwing = (f32, CombatStats, &'static str, std::sync::Arc<[String]>, f32);

#[derive(Resource)]
struct Weapons {
    defs: Vec<WeaponDef>,
    /// The weapon camera (bought weapons' model parts go under it).
    camera: Entity,
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
    /// The frame up to which the sequence's sound notifies have played
    /// (-1 at the start, so notifies at time 0 play).
    notify_frame: f32,
    /// Seconds until each fire mode may fire again (NextFireTime).
    fire_cooldown: [f32; 2],
    fire_count: usize,
    /// bIsFiring per mode: the button is down and StartFire succeeded.
    firing: [bool; 2],
    /// Shots since the button was pressed (WeaponFire.FireCount).
    shots_this_press: [u32; 2],
    /// KFFire burst tracking for the spread.
    spread_state: [crate::weapons::firing::SpreadState; 2],
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
    pending_swings: Vec<PendingSwing>,
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
    /// Sounds started this frame, sent to the mixer by `send_weapon_sounds`.
    sounds: Vec<crate::audio::mixer::PlaySound>,
    /// FlameBurstFire's LastClickTime (game seconds).
    last_click: f32,
    /// Random pitch of fire sounds: its own stream, so the spread and
    /// damage rolls stay as they were before sound.
    sound_rng: u32,
    /// The flashlight on the weapon in hand (torch.rs).
    torch: TorchState,
    /// The perk whose values the weapons carry now (perk.rs re-applies
    /// them when the perk changes).
    vet: crate::game::perks::Vet,
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

/// The first-person weapon's camera (view_target.rs switches it off in
/// behind view).
#[derive(Component)]
pub struct WeaponCamera;

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

/// The package set `load_weapons` used, kept to load weapons bought later
/// (it holds `Rc`s: a non-send resource), and the character's sleeve
/// texture every weapon gets (KFWeapon.HandleSleeveSwapping).
#[derive(Default)]
pub struct WeaponAssets(Option<PackageSet>, Option<ObjectHandle>);

type MeshAssets<'w> = (ResMut<'w, Assets<Mesh>>, ResMut<'w, Assets<Image>>, ResMut<'w, Assets<StandardMaterial>>);

#[cfg(test)]
mod tests {
    use super::*;

    fn husk_charge() -> ChargeFire {
        let x = crate::weapons::projectile::ExplosiveStats {
            sounds: Default::default(),
            class: "medium",
            speed: 1800.0,
            damage: 25.0,
            radius: 150.0,
            momentum: 0.0,
            impact_damage: 100.0,
            impact_headshot_mult: 1.5,
            impact_on_touch: Some(1.5),
            fire: Some(crate::game::combat::FireType::HuskGun),
            dam: None,
            impact_dam: None,
            hurts_self: false,
            zap: None,
            arm_dist: 0.0,
            straight_time: None,
            life_span: 10.0,
            fleshpound_mult: None,
            effect: "",
            decal: crate::render::decals::DecalKind::BurnMedium,
            trail: None,
        };
        ChargeFire {
            max_time: 3.0,
            weak: crate::weapons::projectile::ExplosiveStats { class: "weak", ..x },
            medium: x,
            strong: crate::weapons::projectile::ExplosiveStats { class: "strong", ..x },
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
        let single = Ammo { mag: 15, spare: 105, capacity: 15, initial: 120, max_total: 240, default_capacity: 15, default_max: 240 };
        let dual = Ammo { mag: 30, spare: 90, capacity: 30, initial: 120, max_total: 240, default_capacity: 30, default_max: 240 };
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
