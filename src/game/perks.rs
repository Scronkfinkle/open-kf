//! Perks (KF's veterancy, stage 1): the seven KFVeterancyTypes subclasses
//! and their levels 0-6. You pick the perk and level (`--perk`,
//! `--perk-level`, the buy menu's keys 1-7); nothing is earned yet.
//!
//! Every effect is a copy of one static function of the perk classes
//! (KFMod.KFVetFieldMedic, KFVetSupportSpec, KFVetSharpshooter,
//! KFVetCommando, KFVetBerserker, KFVetFirebug, KFVetDemolitions); the
//! places that call them (weapons, damage, the shop, walking...) use the
//! methods of [`Vet`]. With no perk every method returns "no change", as
//! KF's callers skip the call when ClientVeteranSkill is none. See
//! DESIGN.md, "Perks".

use std::sync::Mutex;

use bevy::prelude::*;

use crate::engine::runlog;

/// The seven perks, in KF's PerkIndex order (0-6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Perk {
    Medic,
    Support,
    Sharpshooter,
    Commando,
    Berserker,
    Firebug,
    Demolitions,
}

impl Perk {
    pub const ALL: [Perk; 7] = [Perk::Medic, Perk::Support, Perk::Sharpshooter, Perk::Commando, Perk::Berserker, Perk::Firebug, Perk::Demolitions];

    /// PerkIndex (also the buy menu's sale list for the perk).
    pub fn index(self) -> usize {
        self as usize
    }

    /// The KFMod class.
    pub fn class(self) -> &'static str {
        match self {
            Perk::Medic => "KFVetFieldMedic",
            Perk::Support => "KFVetSupportSpec",
            Perk::Sharpshooter => "KFVetSharpshooter",
            Perk::Commando => "KFVetCommando",
            Perk::Berserker => "KFVetBerserker",
            Perk::Firebug => "KFVetFirebug",
            Perk::Demolitions => "KFVetDemolitions",
        }
    }

    /// VeterancyName (class defaults).
    pub fn name(self) -> &'static str {
        match self {
            Perk::Medic => "Field Medic",
            Perk::Support => "Support Specialist",
            Perk::Sharpshooter => "Sharpshooter",
            Perk::Commando => "Commando",
            Perk::Berserker => "Berserker",
            Perk::Firebug => "Firebug",
            Perk::Demolitions => "Demolitions",
        }
    }

    /// OnHUDIcon and OnHUDGoldIcon (class defaults).
    pub fn icons(self) -> (&'static str, &'static str) {
        match self {
            Perk::Medic => ("KillingFloorHUD.Perks.Perk_Medic", "KillingFloor2HUD.Perk_Icons.Perk_Medic_Gold"),
            Perk::Support => ("KillingFloorHUD.Perks.Perk_Support", "KillingFloor2HUD.Perk_Icons.Perk_Support_Gold"),
            Perk::Sharpshooter => ("KillingFloorHUD.Perks.Perk_SharpShooter", "KillingFloor2HUD.Perk_Icons.Perk_SharpShooter_Gold"),
            Perk::Commando => ("KillingFloorHUD.Perks.Perk_Commando", "KillingFloor2HUD.Perk_Icons.Perk_Commando_Gold"),
            Perk::Berserker => ("KillingFloorHUD.Perks.Perk_Berserker", "KillingFloor2HUD.Perk_Icons.Perk_Berserker_Gold"),
            Perk::Firebug => ("KillingFloorHUD.Perks.Perk_Firebug", "KillingFloor2HUD.Perk_Icons.Perk_Firebug_Gold"),
            Perk::Demolitions => ("KillingFloor2HUD.Perk_Icons.Perk_Demolition", "KillingFloor2HUD.Perk_Icons.Perk_Demolition_Gold"),
        }
    }

    /// `--perk` names: our short names, a few abbreviations, or the class.
    pub fn parse(s: &str) -> Option<Perk> {
        let s = s.trim().to_ascii_lowercase();
        let s = s.strip_prefix("kfmod.").unwrap_or(&s);
        Some(match s {
            "medic" | "fieldmedic" | "field_medic" | "kfvetfieldmedic" => Perk::Medic,
            "support" | "supportspec" | "support_specialist" | "kfvetsupportspec" => Perk::Support,
            "sharpshooter" | "sharp" | "kfvetsharpshooter" => Perk::Sharpshooter,
            "commando" | "kfvetcommando" => Perk::Commando,
            "berserker" | "zerk" | "kfvetberserker" => Perk::Berserker,
            "firebug" | "kfvetfirebug" => Perk::Firebug,
            "demolitions" | "demo" | "kfvetdemolitions" => Perk::Demolitions,
            _ => return None,
        })
    }
}

/// KFVeterancyTypes StartingWeaponSellPriceLevel5 / Level6 (KFVetDemolitions
/// sets Level5 to 0).
const SELL_PRICE_LEVEL5: f32 = 200.0;
const SELL_PRICE_LEVEL6: f32 = 225.0;

/// A class and its parents, lower case, nearest first: KF's class tests
/// without the package set (`is` for `Item == class'X'`, `is_a` for
/// `X(Other) != none`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClassChain(pub Vec<String>);

impl ClassChain {
    pub fn new(names: Vec<String>) -> Self {
        ClassChain(names.into_iter().map(|n| n.to_ascii_lowercase()).collect())
    }

    /// The class itself.
    pub fn name(&self) -> &str {
        self.0.first().map_or("", String::as_str)
    }

    /// `Item == class'X'`.
    pub fn is(&self, name: &str) -> bool {
        self.name().eq_ignore_ascii_case(name)
    }

    pub fn is_any(&self, names: &[&str]) -> bool {
        names.iter().any(|n| self.is(n))
    }

    /// `X(Other) != none`: the class or a subclass of X.
    pub fn is_a(&self, name: &str) -> bool {
        self.0.iter().any(|c| c.eq_ignore_ascii_case(name))
    }

    pub fn is_a_any(&self, names: &[&str]) -> bool {
        names.iter().any(|n| self.is_a(n))
    }
}

/// A damage type class: its chain and KFWeaponDamageType.bIsMeleeDamage.
#[derive(PartialEq)]
pub struct DamTypeInfo {
    pub chain: ClassChain,
    pub melee: bool,
}

/// Logs show the class name only.
impl std::fmt::Debug for DamTypeInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.chain.name(), if self.melee { "(melee)" } else { "" })
    }
}

/// Damage types are interned once (hits are Copy messages).
pub type DamType = &'static DamTypeInfo;

static DAM_TYPES: Mutex<Vec<DamType>> = Mutex::new(Vec::new());

/// The one shared copy of a damage type.
pub fn intern_dam_type(chain: ClassChain, melee: bool) -> DamType {
    let mut all = DAM_TYPES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(d) = all.iter().find(|d| d.chain == chain && d.melee == melee) {
        return d;
    }
    let d: DamType = Box::leak(Box::new(DamTypeInfo { chain, melee }));
    all.push(d);
    d
}

/// A damage type we name ourselves, with its chain from the scripts
/// (class X extends Y), for damage not read from a weapon: burn ticks,
/// bile, the Husk's fireball.
pub fn known_dam_type(name: &str) -> DamType {
    let chain: &[&str] = match name.to_ascii_lowercase().as_str() {
        "damtypeflamethrower" => &["damtypeflamethrower", "kfweapondamagetype", "weapondamagetype", "damagetype"],
        "damtypetrenchgun" => &["damtypetrenchgun", "kfprojectileweapondamagetype", "kfweapondamagetype", "weapondamagetype", "damagetype"],
        "damtypemac10mpinc" => &["damtypemac10mpinc", "kfprojectileweapondamagetype", "kfweapondamagetype", "weapondamagetype", "damagetype"],
        "damtypeburned" => &["damtypeburned", "kfweapondamagetype", "weapondamagetype", "damagetype"],
        "damtypevomit" => &["damtypevomit", "damtypezombieattack", "kfweapondamagetype", "weapondamagetype", "damagetype"],
        other => return intern_dam_type(ClassChain(vec![other.to_string(), "damagetype".into()]), false),
    };
    intern_dam_type(ClassChain(chain.iter().map(|s| s.to_string()).collect()), false)
}

/// What the perk code needs to know about a weapon: its class chain
/// (prices use the shop's pickup class). Read at load.
#[derive(Clone, Debug, Default)]
pub struct PerkWeapon {
    pub class: ClassChain,
}

/// ClientVeteranSkill and ClientVeteranSkillLevel: the perk in use (None:
/// no perk) and its level 0-6.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Vet {
    pub perk: Option<Perk>,
    pub level: u8,
}

/// Damage types listed by name in the perk scripts.
const SUPPORT_DAMAGE: [&str; 7] =
    ["damtypeshotgun", "damtypedbshotgun", "damtypeaa12shotgun", "damtypebenelli", "damtypeksgshotgun", "damtypenailgun", "damtypespshotgun"];
const COMMANDO_DAMAGE: [&str; 9] = [
    "damtypebullpup",
    "damtypeak47assaultrifle",
    "damtypescarmk17assaultrifle",
    "damtypem4assaultrifle",
    "damtypefnfalassaultrifle",
    "damtypemkb42assaultrifle",
    "damtypethompson",
    "damtypethompsondrum",
    "damtypespthompson",
];
/// KFVetFirebug: class<DamTypeBurned>, <DamTypeFlamethrower>,
/// <DamTypeHuskGunProjectileImpact>, <DamTypeFlareProjectileImpact>.
const FIREBUG_DAMAGE: [&str; 4] = ["damtypeburned", "damtypeflamethrower", "damtypehuskgunprojectileimpact", "damtypeflareprojectileimpact"];
/// KFVetDemolitions: the explosive damage types (subclasses count).
const DEMO_DAMAGE: [&str; 9] = [
    "damtypefrag",
    "damtypepipebomb",
    "damtypem79grenade",
    "damtypem32grenade",
    "damtypem203grenade",
    "damtyperocketimpact",
    "damtypespgrenade",
    "damtypesealsquealexplosion",
    "damtypeseekersixrocket",
];
/// KFVetSharpshooter.GetHeadShotDamMulti's list (DamTypeDualies only below
/// Hell on Earth, which is always true for us).
const SHARP_HEADSHOT: [&str; 14] = [
    "damtypecrossbow",
    "damtypecrossbowheadshot",
    "damtypewinchester",
    "damtypedeagle",
    "damtypedualdeagle",
    "damtypem14ebr",
    "damtypemagnum44pistol",
    "damtypedual44magnum",
    "damtypemk23pistol",
    "damtypedualmk23pistol",
    "damtypem99sniperrifle",
    "damtypem99headshot",
    "damtypespsniper",
    "damtypedualies",
];

/// Weapon classes (IsA tests).
const MEDIC_GUNS: [&str; 5] = ["mp7mmedicgun", "mp5mmedicgun", "m7a3mmedicgun", "krissmmedicgun", "blowerthrower"];
const COMMANDO_GUNS: [&str; 9] = [
    "bullpup",
    "ak47assaultrifle",
    "scarmk17assaultrifle",
    "m4assaultrifle",
    "fnfal_acog_assaultrifle",
    "mkb42assaultrifle",
    "thompsonsmg",
    "thompsondrumsmg",
    "spthompsonsmg",
];
const SHARP_RECOIL_GUNS: [&str; 9] =
    ["crossbow", "winchester", "single", "dualies", "deagle", "dualdeagle", "m14ebrbattlerifle", "m99sniperrifle", "spsniperrifle"];
const SHARP_RELOAD_GUNS: [&str; 12] = [
    "crossbow",
    "winchester",
    "single",
    "dualies",
    "deagle",
    "dualdeagle",
    "mk23pistol",
    "dualmk23pistol",
    "m14ebrbattlerifle",
    "magnum44pistol",
    "dual44magnum",
    "spsniperrifle",
];
const SHARP_FIRE_GUNS: [&str; 4] = ["winchester", "crossbow", "m99sniperrifle", "spsniperrifle"];
const FIREBUG_RELOAD_GUNS: [&str; 5] = ["flamethrower", "mac10mp", "trenchgun", "flarerevolver", "dualflarerevolver"];

/// Pickup classes (exact tests) per perk discount.
const MEDIC_PICKUPS: [&str; 7] = ["mp7mpickup", "mp5mpickup", "m7a3mpickup", "krissmpickup", "blowerthrowerpickup", "camomp5mpickup", "neonkrissmpickup"];
const SUPPORT_PICKUPS: [&str; 11] = [
    "shotgunpickup",
    "boomstickpickup",
    "aa12pickup",
    "benellipickup",
    "ksgpickup",
    "nailgunpickup",
    "goldenbenellipickup",
    "spshotgunpickup",
    "goldenaa12pickup",
    "camoshotgunpickup",
    "neonksgpickup",
];
const SHARP_PICKUPS: [&str; 11] = [
    "deaglepickup",
    "dualdeaglepickup",
    "mk23pickup",
    "dualmk23pickup",
    "magnum44pickup",
    "dual44magnumpickup",
    "m14ebrpickup",
    "m99pickup",
    "spsniperpickup",
    "goldendeaglepickup",
    "goldendualdeaglepickup",
];
const COMMANDO_PICKUPS: [&str; 13] = [
    "bullpuppickup",
    "ak47pickup",
    "scarmk17pickup",
    "m4pickup",
    "fnfal_acog_pickup",
    "mkb42pickup",
    "thompsonpickup",
    "goldenak47pickup",
    "thompsondrumpickup",
    "spthompsonpickup",
    "camom4pickup",
    "neonak47pickup",
    "neonscarmk17pickup",
];
const BERSERKER_PICKUPS: [&str; 10] = [
    "chainsawpickup",
    "katanapickup",
    "claymoreswordpickup",
    "crossbuzzsawpickup",
    "scythepickup",
    "goldenkatanapickup",
    "machetepickup",
    "axepickup",
    "dwarfaxepickup",
    "goldenchainsawpickup",
];
const FIREBUG_PICKUPS: [&str; 7] =
    ["flamethrowerpickup", "mac10pickup", "huskgunpickup", "trenchgunpickup", "flarerevolverpickup", "dualflarerevolverpickup", "goldenftpickup"];
const DEMO_PICKUPS: [&str; 9] = [
    "m79pickup",
    "m32pickup",
    "lawpickup",
    "m4203pickup",
    "goldenm79pickup",
    "spgrenadepickup",
    "camom32pickup",
    "sealsquealpickup",
    "seekersixpickup",
];

/// Ammo classes (exact tests) for AddExtraAmmoFor.
const SUPPORT_AMMO: [&str; 11] = [
    "shotgunammo",
    "dbshotgunammo",
    "aa12ammo",
    "benelliammo",
    "ksgammo",
    "nailgunammo",
    "goldenbenelliammo",
    "spshotgunammo",
    "goldenaa12ammo",
    "camoshotgunammo",
    "neonksgammo",
];
const COMMANDO_AMMO: [&str; 13] = [
    "bullpupammo",
    "ak47ammo",
    "scarmk17ammo",
    "m4ammo",
    "fnfalammo",
    "mkb42ammo",
    "thompsonammo",
    "goldenak47ammo",
    "thompsondrumammo",
    "spthompsonammo",
    "camom4ammo",
    "neonak47ammo",
    "neonscarmk17ammo",
];
const FIREBUG_AMMO: [&str; 6] = ["flameammo", "mac10ammo", "huskgunammo", "trenchgunammo", "flarerevolverammo", "goldenflameammo"];
/// GetAmmoPickupMod's lists (casts: the class or a subclass).
const COMMANDO_PICKUP_AMMO: [&str; 12] = [
    "bullpupammo",
    "ak47ammo",
    "scarmk17ammo",
    "m4ammo",
    "fnfalammo",
    "mkb42ammo",
    "thompsonammo",
    "goldenak47ammo",
    "thompsondrumammo",
    "spthompsonammo",
    "camom4ammo",
    "neonak47ammo",
];
const MEDIC_PICKUP_AMMO: [&str; 7] = ["mp7mammo", "mp5mammo", "m7a3mammo", "krissmammo", "blowerthrowerammo", "camomp5mammo", "neonkrissmammo"];
const FIREBUG_PICKUP_AMMO: [&str; 5] = ["flameammo", "mac10ammo", "huskgunammo", "trenchgunammo", "flarerevolverammo"];

impl Vet {
    #[cfg(test)]
    pub const NONE: Vet = Vet { perk: None, level: 0 };

    fn l(&self) -> f32 {
        self.level as f32
    }

    /// Min(ClientVeteranSkillLevel, 5).
    fn l5(&self) -> f32 {
        self.level.min(5) as f32
    }

    /// For logs: "Support Specialist 6" or "none".
    pub fn label(&self) -> String {
        match self.perk {
            Some(p) => format!("{}:{}", p.class(), self.level),
            None => "none".into(),
        }
    }

    /// GetSyringeChargeRate (Syringe.Tick / KFMedicGun.Tick: +10 x this).
    pub fn syringe_charge_rate(&self) -> f32 {
        match self.perk {
            Some(Perk::Medic) => match self.level {
                0 => 1.10,
                1..=4 => 1.25 + 0.25 * self.l(),
                5 => 2.50,
                _ => 3.00,
            },
            _ => 1.0,
        }
    }

    /// GetHealPotency (the Syringe's heal x this).
    pub fn heal_potency(&self) -> f32 {
        match self.perk {
            Some(Perk::Medic) => match self.level {
                0 => 1.10,
                1..=2 => 1.25,
                3..=5 => 1.5,
                _ => 1.75,
            },
            _ => 1.0,
        }
    }

    /// GetMovementSpeedModifier (KFHumanPawn.ModifyVelocity: GroundSpeed x
    /// this). `difficulty` is GameDifficulty.
    pub fn movement_speed(&self, difficulty: f32) -> f32 {
        match self.perk {
            Some(Perk::Medic) if difficulty >= 5.0 => {
                if self.level <= 2 {
                    1.0
                } else {
                    1.05 + 0.05 * (self.l() - 3.0)
                }
            }
            Some(Perk::Medic) => {
                if self.level <= 1 {
                    1.0
                } else {
                    1.05 + 0.05 * (self.l() - 2.0)
                }
            }
            _ => 1.0,
        }
    }

    /// GetMeleeMovementSpeedModifier: added to BaseMeleeIncrease (0.2)
    /// while holding a bSpeedMeUp weapon (KFHumanPawn.ChangedWeapon).
    pub fn melee_movement_speed(&self) -> f32 {
        match self.perk {
            Some(Perk::Berserker) => match self.level {
                0 => 0.05,
                1 => 0.10,
                2 => 0.15,
                6 => 0.30,
                _ => 0.20,
            },
            _ => 0.0,
        }
    }

    /// ReduceDamage: damage the player takes (an int in KF; float x factor
    /// cut to a whole number). `self_hit`: Injured == Instigator.
    pub fn reduce_damage(&self, damage: i32, self_hit: bool, dt: Option<DamType>) -> i32 {
        let is_a = |names: &[&str]| dt.is_some_and(|d| d.chain.is_a_any(names));
        let vomit = is_a(&["damtypevomit"]);
        let f = damage as f32;
        match self.perk {
            Some(Perk::Medic) if vomit => {
                // Medics don't damage themselves with the bile shooter.
                if self_hit {
                    return 0;
                }
                (f * match self.level {
                    0 => 0.90,
                    1 => 0.75,
                    2..=4 => 0.50,
                    _ => 0.25,
                }) as i32
            }
            Some(Perk::Berserker) => {
                let m = if vomit {
                    [0.90, 0.75, 0.65, 0.50, 0.35, 0.25, 0.20][self.level.min(6) as usize]
                } else {
                    [1.0, 0.95, 0.90, 0.85, 0.80, 0.70, 0.60][self.level.min(6) as usize]
                };
                if m == 1.0 { damage } else { (f * m) as i32 }
            }
            Some(Perk::Firebug) if is_a(&FIREBUG_DAMAGE) => {
                if self.level <= 3 {
                    (f * (0.50 - 0.10 * self.l())) as i32
                } else {
                    0
                }
            }
            Some(Perk::Demolitions) if is_a(&DEMO_DAMAGE) => (f * (0.75 - 0.05 * self.l())) as i32,
            _ => damage,
        }
    }

    /// AddDamage's factor for a damage type, if the perk changes it
    /// (KFMonster.TakeDamage: Damage = int(Damage x factor)).
    pub fn add_damage(&self, dt: Option<DamType>) -> Option<f32> {
        let dt = dt?;
        match self.perk? {
            Perk::Support if dt.chain.is_any(&SUPPORT_DAMAGE) => Some(if self.level == 0 { 1.10 } else { 1.0 + 0.10 * self.l() }),
            Perk::Support if dt.chain.is("damtypefrag") && self.level > 0 => Some(if self.level == 1 { 1.05 } else { 0.90 + 0.10 * self.l() }),
            Perk::Commando if dt.chain.is_any(&COMMANDO_DAMAGE) => Some(if self.level == 0 { 1.05 } else { 1.0 + 0.10 * self.l5() }),
            // class<KFWeaponDamageType>(DmgType).default.bIsMeleeDamage.
            Perk::Berserker if dt.melee => Some(if self.level == 0 { 1.10 } else { 1.0 + 0.20 * self.l5() }),
            Perk::Firebug if dt.chain.is_a_any(&FIREBUG_DAMAGE) => Some(if self.level == 0 { 1.05 } else { 1.0 + 0.10 * self.l() }),
            Perk::Demolitions if dt.chain.is_a_any(&DEMO_DAMAGE) => Some(if self.level == 0 { 1.05 } else { 1.0 + 0.10 * self.l() }),
            _ => None,
        }
    }

    /// GetHeadShotDamMulti (headshots and headless zeds, not melee or fire).
    pub fn headshot_damage(&self, dt: Option<DamType>) -> f32 {
        if self.perk != Some(Perk::Sharpshooter) {
            return 1.0;
        }
        let listed = dt.is_some_and(|d| d.chain.is_any(&SHARP_HEADSHOT));
        let ret = if listed {
            match self.level {
                0..=3 => 1.05 + 0.05 * self.l(),
                4 => 1.30,
                5 => 1.50,
                _ => 1.60,
            }
        } else {
            1.0
        };
        if self.level == 0 { ret * 1.05 } else { ret * (1.0 + 0.10 * self.l5()) }
    }

    /// AddCarryMaxWeight (MaxCarryWeight 15 + this).
    pub fn carry_weight_bonus(&self) -> f32 {
        match self.perk {
            Some(Perk::Support) => match self.level {
                0 => 0.0,
                1..=4 => 1.0 + self.l(),
                5 => 8.0,
                _ => 9.0,
            },
            _ => 0.0,
        }
    }

    /// GetWeldSpeedModifier (WeldFire.Timer: weld damage x this).
    pub fn weld_speed(&self) -> f32 {
        match self.perk {
            Some(Perk::Support) if self.level <= 3 => 1.0 + 0.25 * self.l(),
            Some(Perk::Support) => 2.5,
            _ => 1.0,
        }
    }

    /// AddExtraAmmoFor: MaxAmmo x this, for an ammo class (exact class).
    pub fn extra_ammo(&self, ammo: &ClassChain) -> f32 {
        let tiered = |l: u8| match l {
            1 => 1.10,
            2 => 1.20,
            _ => 1.25,
        };
        match self.perk {
            Some(Perk::Support) if ammo.is("fragammo") => 1.0 + 0.20 * self.l(),
            Some(Perk::Support) if ammo.is_any(&SUPPORT_AMMO) && self.level > 0 => match self.level {
                6 => 1.30,
                l => tiered(l),
            },
            Some(Perk::Commando) if ammo.is_any(&COMMANDO_AMMO) && self.level > 0 => tiered(self.level),
            Some(Perk::Firebug) if ammo.is_any(&FIREBUG_AMMO) && self.level > 0 => 1.0 + 0.10 * self.l(),
            Some(Perk::Demolitions) if ammo.is("fragammo") => 1.0 + 0.20 * self.l(),
            Some(Perk::Demolitions) if ammo.is("pipebombammo") => 1.0 + 0.5 * self.l(),
            Some(Perk::Demolitions) if ammo.is("lawammo") => 1.0 + 0.20 * self.l(),
            _ => 1.0,
        }
    }

    /// GetAmmoPickupMod: an ammo box's AmmoPickupAmount x this
    /// (KFAmmoPickup.Touch). The Support Specialist has none.
    pub fn ammo_pickup_mod(&self, ammo: &ClassChain) -> f32 {
        if self.level == 0 {
            return 1.0;
        }
        match self.perk {
            Some(Perk::Commando) if ammo.is_a_any(&COMMANDO_PICKUP_AMMO) => match self.level {
                1 => 1.10,
                2 => 1.20,
                _ => 1.25,
            },
            Some(Perk::Medic) if ammo.is_a_any(&MEDIC_PICKUP_AMMO) => 1.0 + 0.20 * self.l5(),
            Some(Perk::Firebug) if ammo.is_a_any(&FIREBUG_PICKUP_AMMO) => 1.0 + 0.10 * self.l(),
            _ => 1.0,
        }
    }

    /// GetMagCapacityMod (KFWeapon.UpdateMagCapacity: default MagCapacity
    /// x this, an int).
    pub fn mag_capacity(&self, w: &ClassChain) -> f32 {
        match self.perk {
            Some(Perk::Medic) if w.is_a_any(&MEDIC_GUNS) && self.level > 0 => 1.0 + 0.20 * self.l5(),
            Some(Perk::Commando) if w.is_a_any(&COMMANDO_GUNS) && self.level > 0 => match self.level {
                1 => 1.10,
                2 => 1.20,
                _ => 1.25,
            },
            Some(Perk::Firebug) if w.is_a("flamethrower") && self.level > 0 => 1.0 + 0.10 * self.l(),
            Some(Perk::Firebug) if w.is_a("mac10mp") && self.level > 0 => 1.0 + 0.12 * self.l5(),
            _ => 1.0,
        }
    }

    /// ModifyRecoilSpread: Spread and the recoil kick x this (the weapon in
    /// hand). KF quirk kept: the Sharpshooter's level 0 falls into the
    /// last branch (0.25), as the script is written.
    pub fn recoil_spread(&self, w: &ClassChain) -> f32 {
        match self.perk {
            Some(Perk::Sharpshooter) if w.is_a_any(&SHARP_RECOIL_GUNS) => match self.level {
                1 => 0.75,
                2 => 0.50,
                _ => 0.25,
            },
            Some(Perk::Commando) if w.is_a_any(&COMMANDO_GUNS) => match self.level {
                0..=3 => 0.95 - 0.05 * self.l(),
                4..=5 => 0.70,
                _ => 0.60,
            },
            _ => 1.0,
        }
    }

    /// GetReloadSpeedModifier (ReloadRate = default / this, ReloadAnimRate x this).
    pub fn reload_speed(&self, w: &ClassChain) -> f32 {
        match self.perk {
            Some(Perk::Commando) => 1.05 + 0.05 * self.l(),
            Some(Perk::Sharpshooter | Perk::Firebug) if self.level == 0 => 1.0,
            Some(Perk::Sharpshooter) if w.is_a_any(&SHARP_RELOAD_GUNS) => 1.0 + 0.10 * self.l(),
            Some(Perk::Firebug) if w.is_a_any(&FIREBUG_RELOAD_GUNS) => 1.0 + 0.10 * self.l(),
            _ => 1.0,
        }
    }

    /// GetFireSpeedMod (FireRate = default / this, FireAnimRate x this).
    pub fn fire_speed(&self, w: &ClassChain) -> f32 {
        match self.perk {
            Some(Perk::Sharpshooter) if w.is_a_any(&SHARP_FIRE_GUNS) && self.level > 0 => 1.0 + 0.10 * self.l(),
            Some(Perk::Berserker) if w.is_a_any(&["kfmeleegun", "crossbuzzsaw"]) => match self.level {
                1 => 1.05,
                2 | 3 => 1.10,
                4 => 1.15,
                5 => 1.20,
                6 => 1.25,
                _ => 1.0,
            },
            _ => 1.0,
        }
    }

    /// CanBeGrabbed (ZombieClot.ClawDamageTarget): Berserkers not by Clots.
    pub fn can_be_grabbed_by_clot(&self) -> bool {
        self.perk != Some(Perk::Berserker)
    }

    /// ExtraRange (FlameTendril.Timer: bursts after 2 + this timers).
    pub fn flame_extra_range(&self) -> u32 {
        match self.perk {
            Some(Perk::Firebug) => match self.level {
                0..=2 => 0,
                3..=4 => 1,
                _ => 2,
            },
            _ => 0,
        }
    }

    /// GetCostScaling for a pickup class ("vest" for the armour).
    pub fn cost_scaling(&self, pickup: &str) -> f32 {
        let p = pickup.to_ascii_lowercase();
        let p = p.rsplit('.').next().unwrap_or("");
        let has = |list: &[&str]| list.contains(&p);
        let tiered = 0.9 - 0.10 * self.l();
        match self.perk {
            Some(Perk::Medic) if p == "vest" || has(&MEDIC_PICKUPS) => tiered,
            Some(Perk::Support) if has(&SUPPORT_PICKUPS) => tiered,
            Some(Perk::Sharpshooter) if has(&SHARP_PICKUPS) => tiered,
            Some(Perk::Commando) if has(&COMMANDO_PICKUPS) => tiered,
            Some(Perk::Berserker) if has(&BERSERKER_PICKUPS) => tiered,
            Some(Perk::Firebug) if has(&FIREBUG_PICKUPS) => tiered,
            Some(Perk::Demolitions) if p == "pipebombpickup" => 0.5 - 0.04 * self.l(),
            Some(Perk::Demolitions) if has(&DEMO_PICKUPS) => tiered,
            _ => 1.0,
        }
    }

    /// GetAmmoCostScaling for a pickup class.
    pub fn ammo_cost_scaling(&self, pickup: &str) -> f32 {
        let p = pickup.to_ascii_lowercase();
        let p = p.rsplit('.').next().unwrap_or("");
        match self.perk {
            Some(Perk::Sharpshooter) if p == "crossbowpickup" => 1.0 - 0.07 * self.l(),
            Some(Perk::Demolitions) if p == "pipebombpickup" => 0.5 - 0.04 * self.l(),
            Some(Perk::Demolitions) if DEMO_PICKUPS.contains(&p) => 1.0 - 0.05 * self.l(),
            _ => 1.0,
        }
    }

    /// GetBodyArmorDamageModifier (KFPawn.ShieldAbsorb: damage x this).
    pub fn body_armor(&self) -> f32 {
        match self.perk {
            Some(Perk::Medic) if self.level <= 5 => 1.0 - 0.10 * self.l(),
            Some(Perk::Medic) => 0.25,
            _ => 1.0,
        }
    }

    /// AddDefaultInventory: (weapon class, SellValue) to add, and whether
    /// ShieldStrength is set to 100. `difficulty` is GameDifficulty.
    pub fn default_inventory(&self, difficulty: f32) -> (Vec<(&'static str, f32)>, bool) {
        let (l5, l6) = (SELL_PRICE_LEVEL5, SELL_PRICE_LEVEL6);
        let lv = self.level;
        let mut items = Vec::new();
        let mut armour = false;
        match self.perk {
            Some(Perk::Medic) => {
                armour = lv >= 5;
                if lv == 6 {
                    items.push(("KFMod.MP7MMedicGun", l6));
                }
            }
            Some(Perk::Support) => match lv {
                5 => items.push(("KFMod.Shotgun", l5)),
                6 => items.push(("KFMod.BoomStick", l6)),
                _ => {}
            },
            Some(Perk::Sharpshooter) => match lv {
                5 => items.push(("KFMod.Winchester", l5)),
                6 => items.push(("KFMod.Crossbow", l6)),
                _ => {}
            },
            Some(Perk::Commando) => match lv {
                5 => items.push(("KFMod.Bullpup", l5)),
                6 => items.push(("KFMod.AK47AssaultRifle", l6)),
                _ => {}
            },
            Some(Perk::Berserker) => {
                match lv {
                    5 => items.push(("KFMod.Machete", l5)),
                    6 => items.push(("KFMod.Axe", l6)),
                    _ => {}
                }
                // Removed from Suicidal and HoE in Balance Round 7.
                armour = difficulty < 5.0 && lv == 6;
            }
            Some(Perk::Firebug) => {
                if lv >= 5 {
                    items.push(("KFMod.FlameThrower", l5));
                }
                armour = lv == 6;
            }
            Some(Perk::Demolitions) => {
                // KFVetDemolitions: StartingWeaponSellPriceLevel5 = 0.
                match lv {
                    5 => items.push(("KFMod.PipeBombExplosive", 0.0)),
                    6 => {
                        items.push(("KFMod.PipeBombExplosive", 0.0));
                        items.push(("KFMod.M79GrenadeLauncher", l6));
                    }
                    _ => {}
                }
            }
            None => {}
        }
        (items, armour)
    }

    /// GetShotgunPenetrationDamageMulti (ShotgunBullet.ProcessTouch:
    /// PenDamageReduction).
    pub fn shotgun_penetration(&self, default_reduction: f32) -> f32 {
        if self.perk != Some(Perk::Support) {
            return default_reduction;
        }
        let inverse = 1.0 - default_reduction.max(0.0);
        if self.level == 0 {
            default_reduction + inverse / 10.0
        } else {
            default_reduction + (inverse / 5.5555) * self.l5()
        }
    }

    /// ZedTimeExtensions (KFGameType.Killed).
    pub fn zed_time_extensions(&self) -> u32 {
        match self.perk {
            Some(Perk::Commando) if self.level >= 3 => self.level as u32 - 2,
            Some(Perk::Berserker) => self.level.min(4) as u32,
            _ => 0,
        }
    }

    /// ShowStalkers (KFHumanPawn.ShowStalkers): only the Commando sees
    /// cloaked Stalkers and the cloaked Patriarch.
    pub fn shows_stalkers(&self) -> bool {
        self.perk == Some(Perk::Commando)
    }

    /// GetStalkerViewDistanceMulti (ZombieStalker.Tick: spotted when the
    /// squared distance < this x 640000, 800 units squared).
    pub fn stalker_view_distance_multi(&self) -> f32 {
        match self.perk {
            Some(Perk::Commando) => match self.level {
                0 => 0.0625,
                1 => 0.25,
                2 => 0.36,
                3 => 0.49,
                4 => 0.64,
                _ => 1.0,
            },
            _ => 0.0,
        }
    }

    /// GetMAC10DamageType: the Firebug's MAC-10 shoots DamTypeMAC10MPInc.
    pub fn mac10_incendiary(&self) -> bool {
        self.perk == Some(Perk::Firebug)
    }
}

/// KFPlayerController's perk state: the perk in use (ClientVeteranSkill and
/// its level), the one asked for (SelectedVeterancy) and
/// bChangedVeterancyThisWave. `level` is the `--perk-level` used for any
/// perk (stage 1).
#[derive(Resource, Debug, Default)]
pub struct Veterancy {
    pub vet: Vet,
    pub selected: Option<Perk>,
    pub level: u8,
    pub changed_this_wave: bool,
}

/// Command-line choice (`--perk`, `--perk-level`).
#[derive(Clone, Copy, Debug, Default)]
pub struct PerkOptions {
    pub perk: Option<Perk>,
    pub level: u8,
}

impl Veterancy {
    pub fn from_options(o: PerkOptions) -> Self {
        Veterancy { vet: Vet { perk: o.perk, level: o.level }, selected: o.perk, level: o.level, changed_this_wave: false }
    }
}

/// What a perk change request did (KFPlayerController.SelectVeterancy).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeResult {
    /// Applied now ("You are now a 'X'"; no message if it was already X).
    Now,
    /// During a wave: remembered for the wave end.
    AtWaveEnd,
    /// "You can only change your Perk once per Wave".
    Refused,
}

/// SelectVeterancy's rule. `wave` = bWaveInProgress; `match_begun` =
/// GRI.bMatchHasBegun (ours: the wave game is running).
pub fn select_veterancy(v: &mut Veterancy, perk: Perk, wave: bool, match_begun: bool) -> ChangeResult {
    v.selected = Some(perk);
    if wave && Some(perk) != v.vet.perk {
        v.changed_this_wave = false;
        return ChangeResult::AtWaveEnd;
    }
    if !v.changed_this_wave {
        if match_begun {
            v.changed_this_wave = true;
        }
        v.vet = Vet { perk: Some(perk), level: v.level };
        return ChangeResult::Now;
    }
    ChangeResult::Refused
}

/// Asks for a perk (the buy menu's keys, test action `perk:NAME`).
#[derive(Message, Clone, Copy, Debug)]
pub struct PerkRequest(pub Perk);

/// The lobby's Ready: the pawn spawns now with the current perk
/// (AddDefaultInventory). `had` is the perk whose start items were given
/// when the weapons were loaded.
#[derive(Message, Clone, Copy, Debug)]
pub struct NewPawn {
    pub had: Vet,
}

/// A dead player's new pawn (a network game's wave-end respawn,
/// net/starts.rs): the starting inventory again (AddDefaultInventory).
#[derive(Message, Clone, Copy, Debug)]
pub struct RespawnPawn;

pub struct PerksPlugin;

impl Plugin for PerksPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Veterancy>()
            .add_message::<PerkRequest>()
            .add_message::<NewPawn>()
            .add_message::<RespawnPawn>()
            .add_systems(Startup, log_perk)
            .add_systems(Update, (perk_requests, wave_end_change));
    }
}

/// The startup log, and AddDefaultInventory's armour (P.ShieldStrength =
/// 100); the perk's weapons are added with the starting inventory
/// (weapons/weapon/load.rs).
fn log_perk(v: Res<Veterancy>, mut vest: ResMut<crate::player::armour::Armour>) {
    let difficulty = crate::game::dosh::GAME_DIFFICULTY;
    let (items, armour) = v.vet.default_inventory(difficulty);
    if armour {
        vest.strength = crate::player::armour::MAX_ARMOUR;
        runlog::kv("perk_mod", &format!("kind=start_armour perk={} armour={}", v.vet.label(), vest.strength));
    }
    runlog::kv(
        "perk_selected",
        &format!(
            "perk={} name=\"{}\" level={} source=command_line start_items=[{}] start_armour={armour}",
            v.vet.perk.map_or("none", Perk::class),
            v.vet.perk.map_or("", Perk::name),
            v.vet.level,
            items.iter().map(|(c, s)| format!("{c}:sell{s}")).collect::<Vec<_>>().join(" ")
        ),
    );
    log_summary(&v.vet);
}

/// The perk's constant values, for checking against the scripts.
pub fn log_summary(v: &Vet) {
    if v.perk.is_none() {
        return;
    }
    runlog::kv(
        "perk_values",
        &format!(
            "perk={} syringe_charge={} heal_potency={} move_speed={} melee_move={} carry_bonus={} weld_speed={} body_armor={} zed_time_extensions={} flame_extra_range={} clot_grab={}",
            v.label(),
            v.syringe_charge_rate(),
            v.heal_potency(),
            v.movement_speed(crate::game::dosh::GAME_DIFFICULTY),
            v.melee_movement_speed(),
            v.carry_weight_bonus(),
            v.weld_speed(),
            v.body_armor(),
            v.zed_time_extensions(),
            v.flame_extra_range(),
            v.can_be_grabbed_by_clot(),
        ),
    );
}

fn perk_requests(
    mut requests: MessageReader<PerkRequest>,
    mut v: ResMut<Veterancy>,
    game: Res<crate::game::waves::WaveGame>,
    options: Res<crate::game::waves::GameOptions>,
    (script, frames): (Res<crate::weapons::weapon::ScriptedInput>, Res<bevy::diagnostic::FrameCount>),
    mut messages: MessageWriter<crate::game::hud::LocalMessage>,
    menus: Res<crate::game::menus::MenuState>,
) {
    let scripted: Vec<Perk> = script
        .0
        .iter()
        .filter(|(f, _)| *f == frames.0)
        .filter_map(|(_, a)| a.strip_prefix("perk:"))
        .filter_map(|n| {
            let p = Perk::parse(n);
            if p.is_none() {
                runlog::kv("perk_change", &format!("refused=unknown_perk name={n}"));
            }
            p
        })
        .collect();
    let wave = matches!(game.phase, crate::game::waves::Phase::Wave | crate::game::waves::Phase::BossWave);
    // GRI.bMatchHasBegun: not while the lobby waits for Ready.
    let match_begun = options.mode == crate::game::waves::GameMode::Waves && !menus.lobby_open();
    for perk in requests.read().map(|r| r.0).chain(scripted) {
        let before = v.vet;
        let result = select_veterancy(&mut v, perk, wave, match_begun);
        let text = match result {
            ChangeResult::Now if before.perk != Some(perk) => format!("You are now a '{}'", perk.name()),
            ChangeResult::Now => String::new(),
            ChangeResult::AtWaveEnd => format!("You will become a '{}' at the end of this Wave", perk.name()),
            ChangeResult::Refused => "You can only change your Perk once per Wave".into(),
        };
        runlog::kv(
            "perk_change",
            &format!(
                "asked={} result={result:?} before={} now={} wave_in_progress={wave} changed_this_wave={} message=\"{text}\"",
                perk.class(),
                before.label(),
                v.vet.label(),
                v.changed_this_wave
            ),
        );
        if !text.is_empty() {
            messages.write(crate::game::hud::LocalMessage::text(text));
        }
        if v.vet != before {
            log_summary(&v.vet);
        }
    }
}

/// KFGameType.DoWaveEnd: bChangedVeterancyThisWave = false, and the perk
/// asked for during the wave is applied (SendSelectedVeterancyToServer).
fn wave_end_change(
    mut v: ResMut<Veterancy>,
    game: Res<crate::game::waves::WaveGame>,
    mut seen: Local<Option<u32>>,
    mut messages: MessageWriter<crate::game::hud::LocalMessage>,
) {
    let ended = game.waves_ended;
    let last = seen.get_or_insert(ended);
    if *last == ended {
        return;
    }
    *last = ended;
    v.changed_this_wave = false;
    if let Some(p) = v.selected
        && v.vet.perk != Some(p)
    {
        let before = v.vet;
        let result = select_veterancy(&mut v, p, false, true);
        runlog::kv("perk_change", &format!("asked={} result={result:?} before={} now={} reason=wave_end", p.class(), before.label(), v.vet.label()));
        messages.write(crate::game::hud::LocalMessage::text(format!("You are now a '{}'", p.name())));
        log_summary(&v.vet);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(perk: Perk, level: u8) -> Vet {
        Vet { perk: Some(perk), level }
    }

    fn chain(names: &[&str]) -> ClassChain {
        ClassChain::new(names.iter().map(|s| s.to_string()).collect())
    }

    fn dt(names: &[&str], melee: bool) -> DamType {
        intern_dam_type(chain(names), melee)
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-5
    }

    #[test]
    fn no_perk_changes_nothing() {
        let n = Vet::NONE;
        let shotgun = dt(&["DamTypeShotgun", "KFProjectileWeaponDamageType"], false);
        assert_eq!(n.add_damage(Some(shotgun)), None);
        assert_eq!(n.reduce_damage(30, false, Some(known_dam_type("DamTypeVomit"))), 30);
        assert_eq!(n.cost_scaling("KFMod.ShotgunPickup"), 1.0);
        assert_eq!(n.reload_speed(&chain(&["AK47AssaultRifle", "KFWeapon"])), 1.0);
        assert_eq!(n.default_inventory(2.0), (vec![], false));
        assert_eq!(n.headshot_damage(None), 1.0);
    }

    #[test]
    fn support_shotgun_damage_discount_ammo_weight_weld() {
        let shotgun = dt(&["DamTypeShotgun", "KFProjectileWeaponDamageType"], false);
        let frag = dt(&["DamTypeFrag", "KFWeaponDamageType"], false);
        // AddDamage: level 0 x 1.10, then 1 + 0.1 x level (60% at 6).
        assert!(close(v(Perk::Support, 0).add_damage(Some(shotgun)).unwrap(), 1.10));
        assert!(close(v(Perk::Support, 6).add_damage(Some(shotgun)).unwrap(), 1.60));
        // Frags: nothing at 0, 1.05 at 1, 0.9 + 0.1 x level after.
        assert_eq!(v(Perk::Support, 0).add_damage(Some(frag)), None);
        assert!(close(v(Perk::Support, 1).add_damage(Some(frag)).unwrap(), 1.05));
        assert!(close(v(Perk::Support, 6).add_damage(Some(frag)).unwrap(), 1.50));
        // 70% off shotguns at 6 (0.9 - 0.6).
        assert!(close(v(Perk::Support, 6).cost_scaling("KFMod.ShotgunPickup"), 0.30));
        assert!(close(v(Perk::Support, 0).cost_scaling("KFMod.BoomStickPickup"), 0.90));
        assert_eq!(v(Perk::Support, 6).cost_scaling("KFMod.AK47Pickup"), 1.0);
        // Max ammo: frags + 20% per level, shotgun shells 1.1 / 1.2 / 1.25 / 1.3.
        assert!(close(v(Perk::Support, 6).extra_ammo(&chain(&["FragAmmo"])), 2.2));
        assert!(close(v(Perk::Support, 3).extra_ammo(&chain(&["ShotgunAmmo"])), 1.25));
        assert!(close(v(Perk::Support, 6).extra_ammo(&chain(&["ShotgunAmmo"])), 1.30));
        assert_eq!(v(Perk::Support, 0).extra_ammo(&chain(&["ShotgunAmmo"])), 1.0);
        // Carry weight 15 + 0 / 2..5 / 8 / 9; welding 1 + 0.25 x level, 2.5 from 4.
        assert_eq!(v(Perk::Support, 4).carry_weight_bonus(), 5.0);
        assert_eq!(v(Perk::Support, 6).carry_weight_bonus(), 9.0);
        assert_eq!(v(Perk::Support, 3).weld_speed(), 1.75);
        assert_eq!(v(Perk::Support, 4).weld_speed(), 2.5);
        // Penetration: 0.5 default -> 0.55 at 0, 0.5 + 0.5 / 5.5555 x 5 at 5+.
        assert!(close(v(Perk::Support, 0).shotgun_penetration(0.5), 0.55));
        assert!(close(v(Perk::Support, 6).shotgun_penetration(0.5), 0.5 + 0.5 / 5.5555 * 5.0));
        // Starting items.
        assert_eq!(v(Perk::Support, 5).default_inventory(2.0), (vec![("KFMod.Shotgun", 200.0)], false));
        assert_eq!(v(Perk::Support, 6).default_inventory(2.0), (vec![("KFMod.BoomStick", 225.0)], false));
    }

    #[test]
    fn commando_reload_recoil_mag_zed_time() {
        let ak = chain(&["AK47AssaultRifle", "KFWeapon"]);
        let m4203 = chain(&["M4203AssaultRifle", "M4AssaultRifle", "KFWeapon"]);
        let pistol = chain(&["Single", "KFWeapon"]);
        // Reload: 1.05 + 0.05 x level, every weapon.
        assert!(close(v(Perk::Commando, 6).reload_speed(&pistol), 1.35));
        assert!(close(v(Perk::Commando, 0).reload_speed(&ak), 1.05));
        // Recoil: 0.95 - 0.05 x level to 3, 0.7 at 4-5, 0.6 at 6; IsA, so the M4 203 counts.
        assert!(close(v(Perk::Commando, 3).recoil_spread(&ak), 0.80));
        assert!(close(v(Perk::Commando, 6).recoil_spread(&m4203), 0.60));
        assert_eq!(v(Perk::Commando, 6).recoil_spread(&pistol), 1.0);
        // Magazines 1.1 / 1.2 / 1.25.
        assert_eq!(v(Perk::Commando, 0).mag_capacity(&ak), 1.0);
        assert!(close(v(Perk::Commando, 2).mag_capacity(&ak), 1.20));
        assert!(close(v(Perk::Commando, 6).mag_capacity(&ak), 1.25));
        // Damage x 1.05 at 0, up to x 1.5 (Min(level, 5)).
        let akd = dt(&["DamTypeAK47AssaultRifle", "KFProjectileWeaponDamageType"], false);
        assert!(close(v(Perk::Commando, 6).add_damage(Some(akd)).unwrap(), 1.5));
        assert_eq!(v(Perk::Commando, 2).zed_time_extensions(), 0);
        assert_eq!(v(Perk::Commando, 6).zed_time_extensions(), 4);
    }

    #[test]
    fn sharpshooter_headshots_reload_fire_rate() {
        let xbow = dt(&["DamTypeCrossbow", "KFProjectileWeaponDamageType"], false);
        let other = dt(&["DamTypeAK47AssaultRifle", "KFProjectileWeaponDamageType"], false);
        // Listed: 1.6 x (1 + 0.1 x 5) at 6.
        assert!(close(v(Perk::Sharpshooter, 6).headshot_damage(Some(xbow)), 1.6 * 1.5));
        // Any other weapon: the general 1 + 0.1 x level.
        assert!(close(v(Perk::Sharpshooter, 3).headshot_damage(Some(other)), 1.3));
        assert!(close(v(Perk::Sharpshooter, 0).headshot_damage(Some(xbow)), 1.05 * 1.05));
        let win = chain(&["Winchester", "KFWeaponShotgun", "KFWeapon"]);
        assert!(close(v(Perk::Sharpshooter, 6).reload_speed(&win), 1.6));
        assert!(close(v(Perk::Sharpshooter, 6).fire_speed(&win), 1.6));
        assert_eq!(v(Perk::Sharpshooter, 0).fire_speed(&win), 1.0);
        // The script's level 0 gives 0.25 recoil (falls into the else branch).
        assert_eq!(v(Perk::Sharpshooter, 0).recoil_spread(&win), 0.25);
        assert_eq!(v(Perk::Sharpshooter, 1).recoil_spread(&win), 0.75);
        // Bolts 42% off at 6.
        assert!(close(v(Perk::Sharpshooter, 6).ammo_cost_scaling("KFMod.CrossbowPickup"), 0.58));
    }

    #[test]
    fn berserker_melee_and_resistance() {
        let knife = dt(&["DamTypeKnife", "DamTypeMelee"], true);
        let gun = dt(&["DamTypeDeagle"], false);
        assert!(close(v(Perk::Berserker, 0).add_damage(Some(knife)).unwrap(), 1.10));
        assert!(close(v(Perk::Berserker, 6).add_damage(Some(knife)).unwrap(), 2.0));
        assert_eq!(v(Perk::Berserker, 6).add_damage(Some(gun)), None);
        let melee_gun = chain(&["Axe", "KFMeleeGun", "KFWeapon"]);
        assert_eq!(v(Perk::Berserker, 6).fire_speed(&melee_gun), 1.25);
        assert_eq!(v(Perk::Berserker, 0).fire_speed(&melee_gun), 1.0);
        // 40% less of everything at 6, bile 80% less; level 0 no general cut.
        assert_eq!(v(Perk::Berserker, 6).reduce_damage(100, false, None), 60);
        assert_eq!(v(Perk::Berserker, 0).reduce_damage(100, false, None), 100);
        assert_eq!(v(Perk::Berserker, 6).reduce_damage(10, false, Some(known_dam_type("DamTypeVomit"))), 2);
        assert_eq!(v(Perk::Berserker, 6).melee_movement_speed(), 0.30);
        assert!(!v(Perk::Berserker, 0).can_be_grabbed_by_clot());
        assert_eq!(v(Perk::Berserker, 6).default_inventory(2.0), (vec![("KFMod.Axe", 225.0)], true));
        assert!(!v(Perk::Berserker, 6).default_inventory(5.0).1);
    }

    #[test]
    fn medic_heal_speed_armour() {
        assert_eq!(v(Perk::Medic, 6).syringe_charge_rate(), 3.0);
        assert_eq!(v(Perk::Medic, 3).syringe_charge_rate(), 2.0);
        assert_eq!(v(Perk::Medic, 6).heal_potency(), 1.75);
        // Normal difficulty: 1.05 + 0.05 x (level - 2) from level 2: 1.25 at 6.
        assert!(close(v(Perk::Medic, 6).movement_speed(2.0), 1.25));
        assert_eq!(v(Perk::Medic, 1).movement_speed(2.0), 1.0);
        assert!(close(v(Perk::Medic, 6).movement_speed(5.0), 1.20));
        assert!(close(v(Perk::Medic, 6).cost_scaling("vest"), 0.30));
        assert_eq!(v(Perk::Medic, 6).body_armor(), 0.25);
        assert!(close(v(Perk::Medic, 5).body_armor(), 0.5));
        assert_eq!(v(Perk::Medic, 6).reduce_damage(10, true, Some(known_dam_type("DamTypeVomit"))), 0);
        assert_eq!(v(Perk::Medic, 6).reduce_damage(10, false, Some(known_dam_type("DamTypeVomit"))), 2);
        let mp7 = chain(&["MP7MMedicGun", "KFMedicGun"]);
        assert!(close(v(Perk::Medic, 6).mag_capacity(&mp7), 2.0));
        assert_eq!(v(Perk::Medic, 6).default_inventory(2.0), (vec![("KFMod.MP7MMedicGun", 225.0)], true));
    }

    #[test]
    fn firebug_fire_damage_resistance_range() {
        let flame = known_dam_type("DamTypeFlamethrower");
        let husk = dt(&["DamTypeHuskGun", "DamTypeBurned"], false);
        assert!(close(v(Perk::Firebug, 6).add_damage(Some(flame)).unwrap(), 1.6));
        assert!(close(v(Perk::Firebug, 0).add_damage(Some(husk)).unwrap(), 1.05));
        assert_eq!(v(Perk::Firebug, 0).reduce_damage(10, false, Some(known_dam_type("DamTypeBurned"))), 5);
        assert_eq!(v(Perk::Firebug, 4).reduce_damage(10, false, Some(known_dam_type("DamTypeBurned"))), 0);
        // The incendiary MAC-10 is not a DamTypeBurned: no bonus.
        assert_eq!(v(Perk::Firebug, 6).add_damage(Some(known_dam_type("DamTypeMAC10MPInc"))), None);
        assert_eq!(v(Perk::Firebug, 2).flame_extra_range(), 0);
        assert_eq!(v(Perk::Firebug, 5).flame_extra_range(), 2);
        let ft = chain(&["FlameThrower", "KFWeapon"]);
        assert!(close(v(Perk::Firebug, 6).mag_capacity(&ft), 1.6));
        assert!(close(v(Perk::Firebug, 6).mag_capacity(&chain(&["MAC10MP"])), 1.6));
        assert_eq!(v(Perk::Firebug, 6).default_inventory(2.0), (vec![("KFMod.FlameThrower", 200.0)], true));
    }

    #[test]
    fn demolitions_explosives() {
        let law = dt(&["DamTypeLAW", "DamTypeFrag"], false);
        assert!(close(v(Perk::Demolitions, 6).add_damage(Some(law)).unwrap(), 1.6));
        assert_eq!(v(Perk::Demolitions, 6).reduce_damage(100, true, Some(law)), 45);
        assert!(close(v(Perk::Demolitions, 6).extra_ammo(&chain(&["PipeBombAmmo"])), 4.0));
        assert!(close(v(Perk::Demolitions, 6).cost_scaling("KFMod.PipeBombPickup"), 0.26));
        assert!(close(v(Perk::Demolitions, 6).ammo_cost_scaling("KFMod.M79Pickup"), 0.70));
        assert_eq!(
            v(Perk::Demolitions, 6).default_inventory(2.0),
            (vec![("KFMod.PipeBombExplosive", 0.0), ("KFMod.M79GrenadeLauncher", 225.0)], false)
        );
    }

    #[test]
    fn change_rule_follows_select_veterancy() {
        let mut s = Veterancy::from_options(PerkOptions { perk: Some(Perk::Support), level: 6 });
        // Trader time: immediate, once.
        assert_eq!(select_veterancy(&mut s, Perk::Medic, false, true), ChangeResult::Now);
        assert_eq!(s.vet, v(Perk::Medic, 6));
        assert_eq!(select_veterancy(&mut s, Perk::Commando, false, true), ChangeResult::Refused);
        assert_eq!(s.vet.perk, Some(Perk::Medic));
        // During a wave: deferred, and the allowance comes back.
        assert_eq!(select_veterancy(&mut s, Perk::Firebug, true, true), ChangeResult::AtWaveEnd);
        assert_eq!(s.vet.perk, Some(Perk::Medic));
        assert_eq!(s.selected, Some(Perk::Firebug));
        assert!(!s.changed_this_wave);
        // Before the match has begun: as often as you like.
        let mut t = Veterancy::default();
        assert_eq!(select_veterancy(&mut t, Perk::Medic, false, false), ChangeResult::Now);
        assert_eq!(select_veterancy(&mut t, Perk::Support, false, false), ChangeResult::Now);
    }

    #[test]
    fn parse_names() {
        assert_eq!(Perk::parse("support"), Some(Perk::Support));
        assert_eq!(Perk::parse("KFVetSupportSpec"), Some(Perk::Support));
        assert_eq!(Perk::parse("demo"), Some(Perk::Demolitions));
        assert_eq!(Perk::parse("nope"), None);
    }
}
