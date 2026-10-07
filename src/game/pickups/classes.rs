//! Pickup classes (KF's Pickup subclasses) as the game uses them: what
//! taking one gives, how it looks and sounds, its touch cylinder. Read
//! from the class defaults (docs/DESIGN.md, "Pickups").

use serde::{Deserialize, Serialize};
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package::ObjectRef;
use ue_assets::package_set::{ObjectHandle, PackageSet};
use ue_assets::properties::Value;

/// What taking a pickup gives. Sent over the network with each shown
/// pickup (the host decides what an item is; clients only draw it).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum PickupGives {
    /// A KFWeaponPickup: its InventoryType and that class's default
    /// Weight (CheckCanCarry). `carried`: the state of a weapon a player
    /// dropped (KFWeaponPickup.InitDroppedPickupFor); None for a fresh one.
    Weapon { weapon: String, weight: f32, carried: Option<CarriedWeapon> },
    /// A KFAmmoPickup: ammo for every carried weapon (KFAmmoPickup.Touch).
    Ammo,
    /// A ShieldPickup (the Vest): ShieldAmount armour.
    Armour { amount: f32 },
    /// A CashPickup (dosh tossed by a player, KFPawn.TossCash):
    /// CashPickup.GiveCashTo adds CashAmount to the taker's cash.
    Cash { amount: i32 },
}

/// A dropped weapon's state (KFWeaponPickup MagAmmoRemaining,
/// AmmoAmount[0] and [1], SellValue, WeaponPickup.bThrown).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct CarriedWeapon {
    pub mag: u32,
    /// The ammo, magazine included (AmmoAmount[0]).
    pub total: u32,
    /// None: -1 (never bought).
    pub sell_value: Option<f32>,
    /// Thrown by a living player (bThrown): the taker gets `total` and
    /// `alt`; dropped by a dying one: the class's fresh ammo (KFWeapon.GiveAmmo)
    /// with this magazine (KFWeapon.GiveTo).
    #[serde(default)]
    pub thrown: bool,
    /// The second fire mode's own ammo (AmmoAmount[1], the M4 203's grenades).
    #[serde(default)]
    pub alt: Option<u32>,
}

impl PickupGives {
    pub fn label(&self) -> String {
        match self {
            PickupGives::Weapon { weapon, weight, carried } => match carried {
                Some(c) => format!(
                    "weapon:{weapon}:weight{weight}:dropped:mag{}:total{}{}:sell{}:{}",
                    c.mag,
                    c.total,
                    c.alt.map_or(String::new(), |a| format!(":alt{a}")),
                    c.sell_value.map_or("-1".into(), |v| format!("{v:.0}")),
                    if c.thrown { "thrown" } else { "died" }
                ),
                None => format!("weapon:{weapon}:weight{weight}"),
            },
            PickupGives::Ammo => "ammo".into(),
            PickupGives::Armour { amount } => format!("armour:{amount}"),
            PickupGives::Cash { amount } => format!("cash:{amount}"),
        }
    }

    /// The kind name the test inputs use (`warp_pickup:KIND`).
    pub fn kind(&self) -> &'static str {
        match self {
            PickupGives::Weapon { .. } => "weapon",
            PickupGives::Ammo => "ammo",
            PickupGives::Armour { .. } => "vest",
            PickupGives::Cash { .. } => "cash",
        }
    }
}

/// One pickup class's defaults.
#[derive(Clone, Debug)]
pub struct PickupClass {
    /// Full path, e.g. "KFMod.ShotgunPickup".
    pub path: String,
    pub gives: PickupGives,
    /// StaticMesh (full path), None: nothing to draw.
    pub mesh: Option<String>,
    pub draw_scale: f32,
    pub draw_scale_3d: [f32; 3],
    pub pre_pivot: [f32; 3],
    /// CollisionRadius / CollisionHeight (Unreal units).
    pub radius: f32,
    pub height: f32,
    /// PickupSound with TransientSoundVolume / TransientSoundRadius.
    pub sound: Option<String>,
    pub sound_volume: f32,
    pub sound_radius: f32,
    /// PickupMessage (Vest.GetLocalString and Pickup's both return it).
    pub message: String,
    /// CullDistance (0: always drawn).
    pub cull_distance: f32,
    /// RespawnTime (directly placed pickups).
    pub respawn_time: f32,
    /// A KFWeaponPickup (Sleeping waits while a player sees it).
    pub weapon_pickup: bool,
}

/// Finds a class by its "Package.Class" path.
pub fn find_class(set: &PackageSet, path: &str) -> Option<ObjectHandle> {
    let (pkg_name, class_name) = path.split_once('.')?;
    let lp = set.load(pkg_name)?;
    let export = (0..lp.pkg.exports.len()).find(|&i| lp.pkg.export_class_name(i) == "Class" && lp.pkg.object_name(ObjectRef::Export(i)).eq_ignore_ascii_case(class_name))?;
    Some(ObjectHandle { package: lp, export })
}

/// An object property as a full path.
fn object_path(set: &PackageSet, defaults: &ClassDefaults, class: &ObjectHandle, prop: &str) -> Option<String> {
    match defaults.get(class, prop) {
        Some((Value::Object(r), from)) if r != ObjectRef::Null => set.resolve(&from, r).map(|h| h.path()),
        _ => None,
    }
}

fn float(defaults: &ClassDefaults, class: &ObjectHandle, prop: &str, default: f32) -> f32 {
    match defaults.get(class, prop) {
        Some((Value::Float(f), _)) => f,
        Some((Value::Int(i), _)) => i as f32,
        _ => default,
    }
}

fn vector(defaults: &ClassDefaults, class: &ObjectHandle, prop: &str, default: [f32; 3]) -> [f32; 3] {
    match defaults.get(class, prop) {
        Some((Value::Vector(v), _)) => v,
        _ => default,
    }
}

/// Reads a pickup class; Err for classes we do not handle (with the reason).
pub fn read_class(set: &PackageSet, defaults: &ClassDefaults, path: &str) -> Result<PickupClass, String> {
    let class = find_class(set, path).ok_or("class not found")?;
    let gives = if defaults.is_a(&class, "KFWeaponPickup") {
        let weapon = object_path(set, defaults, &class, "InventoryType").ok_or("no InventoryType")?;
        let weight = find_class(set, &weapon).map_or(0.0, |w| float(defaults, &w, "Weight", 0.0));
        PickupGives::Weapon { weapon, weight, carried: None }
    } else if defaults.is_a(&class, "KFAmmoPickup") {
        PickupGives::Ammo
    } else if defaults.is_a(&class, "ShieldPickup") {
        PickupGives::Armour { amount: float(defaults, &class, "ShieldAmount", 0.0) }
    } else if defaults.is_a(&class, "CashPickup") {
        PickupGives::Cash { amount: 0 }
    } else {
        return Err(format!("unhandled class chain {:?}", defaults.chain_names(&class)));
    };
    let mesh = object_path(set, defaults, &class, "StaticMesh");
    Ok(PickupClass {
        path: class.path(),
        gives,
        mesh,
        draw_scale: float(defaults, &class, "DrawScale", 1.0),
        draw_scale_3d: vector(defaults, &class, "DrawScale3D", [1.0; 3]),
        pre_pivot: vector(defaults, &class, "PrePivot", [0.0; 3]),
        radius: float(defaults, &class, "CollisionRadius", 22.0),
        height: float(defaults, &class, "CollisionHeight", 22.0),
        sound: crate::weapons::weapon::sound_prop(defaults, &class, "PickupSound"),
        sound_volume: float(defaults, &class, "TransientSoundVolume", 0.3),
        sound_radius: float(defaults, &class, "TransientSoundRadius", 300.0),
        message: match defaults.get(&class, "PickupMessage") {
            Some((Value::Str(s), _)) => s,
            _ => String::new(),
        },
        cull_distance: float(defaults, &class, "CullDistance", 0.0),
        respawn_time: float(defaults, &class, "RespawnTime", 0.0),
        weapon_pickup: defaults.is_a(&class, "KFWeaponPickup"),
    })
}
