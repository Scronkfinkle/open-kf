//! The buy menu (T3a): what the trader sells (KFLevelRules' per-perk
//! lists and each pickup's prices), opening and closing the menu, and
//! the requests a menu sends; weapon.rs carries them out with KFPawn's
//! rules. The screens are numenu.rs (ours) and classic_menu.rs (KF's
//! GUIBuyMenu). See DESIGN.md, T3a.

use std::rc::Rc;

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package::ObjectRef;
use ue_assets::package_set::{LoadedPackage, ObjectHandle, PackageSet};
use ue_assets::properties::{Value, read_export_properties};

use crate::engine::runlog;

/// A weapon's pickup values (KFWeaponPickup defaults).
#[derive(Clone, Debug)]
pub struct ShopItem {
    /// The weapon class, "KFMod.Shotgun".
    pub weapon: String,
    pub pickup: String,
    pub name: String,
    pub cost: i32,
    pub ammo_cost: i32,
    pub weight: f32,
    pub buy_clip_size: i32,
    /// The weapon's bKFNeverThrow: never listed for sale, cannot be sold.
    pub never_throw: bool,
    /// Numbers to compare weapons by (NuMenu's details panel only).
    pub stats: ShopStats,
    /// What the classic menu shows (display only, no rule reads these).
    pub info: ShopInfo,
}

/// A pickup's trader texts and pictures (KFWeaponPickup defaults, the
/// weapon's Description), read for the classic menu (classic_menu.rs).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShopInfo {
    /// ItemShortName (KFBuyMenuInvList's names), else ItemName.
    pub short_name: String,
    /// The weapon's Description (the inventory list's, KFTab_BuyMenu.SetInfoText).
    pub description: String,
    /// PowerValue, RangeValue, SpeedValue (GUIBuyWeaponInfoPanel's bars).
    pub power: f32,
    pub range: f32,
    pub speed: f32,
    /// CorrespondingPerkIndex (0-6 a perk's icon, 7 none).
    pub perk_index: usize,
    /// TraderInfoTexture, as "Package.Group.Name".
    pub image: String,
    /// SecondaryAmmoShortName (the M4 203's grenades).
    pub secondary_name: String,
}

/// A weapon's comparable numbers from its class defaults (display only;
/// no game rule reads these): FireModeClass[0]'s damage (MeleeDamage,
/// else its ProjectileClass's Damage, else DamageMax), ProjPerFire,
/// FireRate; the weapon's MagCapacity; the ammo class's MaxAmmo.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShopStats {
    pub damage: f32,
    pub per_fire: i32,
    /// Seconds between shots.
    pub fire_rate: f32,
    pub mag: i32,
    pub max_ammo: i32,
    pub melee: bool,
}

impl ShopStats {
    /// Damage of one shot (all its pellets).
    pub fn shot_damage(&self) -> f32 {
        self.damage * self.per_fire.max(1) as f32
    }

    /// Shots per second.
    pub fn shots_per_second(&self) -> f32 {
        if self.fire_rate > 0.0 { 1.0 / self.fire_rate } else { 0.0 }
    }
}

/// The perk filter's lists, in KF's order (BuyMenuFilterIndex 0-7).
const LISTS: [(&str, &str); 8] = [
    ("Medic", "MediItemForSale"),
    ("Support", "SuppItemForSale"),
    ("Sharpshooter", "ShrpItemForSale"),
    ("Commando", "CommItemForSale"),
    ("Berserker", "BersItemForSale"),
    ("Firebug", "FireItemForSale"),
    ("Demolitions", "DemoItemForSale"),
    ("Neutral", "NeutItemForSale"),
];

#[derive(Resource, Default)]
pub struct ShopCatalogue {
    /// Every base weapon's pickup (also those never for sale: their ammo).
    pub items: Vec<ShopItem>,
    /// (filter name, indices into `items`).
    pub lists: Vec<(&'static str, Vec<usize>)>,
}

impl ShopCatalogue {
    pub fn item(&self, weapon: &str) -> Option<&ShopItem> {
        self.items.iter().find(|i| i.weapon.eq_ignore_ascii_case(weapon))
    }
}

/// Dual pistols whose single halves their price and weight when owned
/// (KFBuyMenuSaleList.PopulateBuyables, KFPawn.ServerBuyWeapon). Dualies
/// is not among them.
pub const HALF_PRICE_DUALS: [(&str, &str); 3] =
    [("KFMod.DualDeagle", "KFMod.Deagle"), ("KFMod.Dual44Magnum", "KFMod.Magnum44Pistol"), ("KFMod.DualMK23Pistol", "KFMod.MK23Pistol")];

/// Every dual and its single (a single is hidden while its duals are owned).
pub const DUAL_SINGLES: [(&str, &str); 4] = [
    ("KFMod.Dualies", "KFMod.Single"),
    ("KFMod.DualDeagle", "KFMod.Deagle"),
    ("KFMod.Dual44Magnum", "KFMod.Magnum44Pistol"),
    ("KFMod.DualMK23Pistol", "KFMod.MK23Pistol"),
];

/// Reads the pickups of the base weapons and the map's (else the class
/// default) KFLevelRules sale lists.
pub fn load_catalogue(set: &PackageSet, defaults: &ClassDefaults, map: &Rc<LoadedPackage>) -> ShopCatalogue {
    let mut cat = ShopCatalogue::default();
    let class = |path: &str| -> Option<ObjectHandle> {
        let (p, c) = path.split_once('.')?;
        let lp = set.load(p)?;
        (0..lp.pkg.exports.len())
            .find(|&i| lp.pkg.export_class_name(i) == "Class" && lp.pkg.object_name(ObjectRef::Export(i)).eq_ignore_ascii_case(c))
            .map(|export| ObjectHandle { package: lp.clone(), export })
    };
    for name in crate::weapons::weapon::BASE_WEAPONS {
        let weapon = format!("KFMod.{name}");
        let Some(wc) = class(&weapon) else { continue };
        let pickup = match defaults.get(&wc, "PickupClass") {
            Some((Value::Object(r), rp)) if r != ObjectRef::Null => set.resolve(&rp, r),
            _ => None,
        };
        let Some(pickup) = pickup else {
            runlog::kv("shop_catalogue_warning", &format!("weapon={weapon} reason=no_pickup"));
            continue;
        };
        let num = |p: &str, d: f32| match defaults.get(&pickup, p) {
            Some((Value::Int(i), _)) => i as f32,
            Some((Value::Float(f), _)) => f,
            _ => d,
        };
        cat.items.push(ShopItem {
            name: match defaults.get(&pickup, "ItemName") {
                Some((Value::Str(s), _)) => s,
                _ => name.to_string(),
            },
            pickup: pickup.path(),
            cost: num("Cost", 0.0) as i32,
            ammo_cost: num("AmmoCost", 0.0) as i32,
            weight: num("Weight", 0.0),
            buy_clip_size: num("BuyClipSize", 0.0) as i32,
            never_throw: matches!(defaults.get(&wc, "bKFNeverThrow"), Some((Value::Bool(true), _))),
            stats: read_stats(set, defaults, &wc),
            info: read_info(set, defaults, &wc, &pickup, name),
            weapon,
        });
    }
    // The map's own KFLevelRules, if it sets the lists.
    let pkg = &map.pkg;
    let map_rules = pkg.level_actor_exports().find(|&i| pkg.export_class_name(i) == "KFLevelRules").and_then(|i| read_export_properties(pkg, i).ok());
    let rules_class = class("KFMod.KFLevelRules");
    let mut source = Vec::new();
    for (label, prop) in LISTS {
        let (value, from) = match map_rules.as_ref().and_then(|p| p.get(pkg, prop)) {
            Some(v) => (Some(v.clone()), Some(map.clone())),
            None => match rules_class.as_ref().and_then(|c| defaults.get(c, prop)) {
                Some((v, f)) => (Some(v), Some(f)),
                None => (None, None),
            },
        };
        if from.as_ref().is_some_and(|f| Rc::ptr_eq(f, map)) {
            source.push(label);
        }
        let mut list = Vec::new();
        let mut dropped = Vec::new();
        if let (Some(Value::Array { count, raw }), Some(from)) = (value, from) {
            let mut r = ue_assets::reader::Reader::new(&raw);
            for _ in 0..count {
                let Ok(idx) = r.compact_index() else { break };
                let Some(h) = set.resolve(&from, ObjectRef::from_raw(idx)) else { continue };
                let path = h.path();
                match cat.items.iter().position(|i| i.pickup.eq_ignore_ascii_case(&path)) {
                    Some(i) => list.push(i),
                    None => dropped.push(path),
                }
            }
        }
        runlog::kv(
            "shop_catalogue",
            &format!("list={label} items=[{}] not_ours=[{}]", list.iter().map(|&i| cat.items[i].name.as_str()).collect::<Vec<_>>().join(", "), dropped.join(" ")),
        );
        cat.lists.push((label, list));
    }
    runlog::kv(
        "shop_catalogue",
        &format!(
            "items={} map_lists=[{}] prices=[{}]",
            cat.items.len(),
            source.join(" "),
            cat.items.iter().map(|i| format!("{}:{}/{}/w{}", i.weapon.trim_start_matches("KFMod."), i.cost, i.ammo_cost, i.weight)).collect::<Vec<_>>().join(" ")
        ),
    );
    runlog::kv(
        "shop_stats",
        &cat.items
            .iter()
            .map(|i| {
                let s = &i.stats;
                format!("{}:dmg={}x{}/rate={}/mag={}/ammo={}{}", i.weapon.trim_start_matches("KFMod."), s.damage, s.per_fire, s.fire_rate, s.mag, s.max_ammo, if s.melee { "/melee" } else { "" })
            })
            .collect::<Vec<_>>()
            .join(" "),
    );
    runlog::kv(
        "shop_info",
        &cat.items
            .iter()
            .map(|i| {
                let f = &i.info;
                format!("{}:p{}/r{}/s{}/perk{}/img={}", i.weapon.trim_start_matches("KFMod."), f.power, f.range, f.speed, f.perk_index, if f.image.is_empty() { "none" } else { f.image.as_str() })
            })
            .collect::<Vec<_>>()
            .join(" "),
    );
    cat
}

/// `ShopInfo` from a pickup's (and its weapon's) defaults.
fn read_info(set: &PackageSet, defaults: &ClassDefaults, wc: &ObjectHandle, pickup: &ObjectHandle, name: &str) -> ShopInfo {
    let text = |h: &ObjectHandle, p: &str| match defaults.get(h, p) {
        Some((Value::Str(s), _)) => Some(s),
        _ => None,
    };
    let num = |p: &str| match defaults.get(pickup, p) {
        Some((Value::Int(i), _)) => i as f32,
        Some((Value::Float(f), _)) => f,
        Some((Value::Byte(b), _)) => b as f32,
        _ => 0.0,
    };
    let image = match defaults.get(pickup, "TraderInfoTexture").or_else(|| defaults.get(wc, "TraderInfoTexture")) {
        Some((Value::Object(r), rp)) if r != ObjectRef::Null => set.resolve(&rp, r).map(|h| h.path()).unwrap_or_default(),
        _ => String::new(),
    };
    ShopInfo {
        short_name: text(pickup, "ItemShortName").or_else(|| text(pickup, "ItemName")).unwrap_or_else(|| name.to_string()),
        description: text(wc, "Description").or_else(|| text(pickup, "Description")).unwrap_or_default(),
        power: num("PowerValue"),
        range: num("RangeValue"),
        speed: num("SpeedValue"),
        perk_index: num("CorrespondingPerkIndex") as usize,
        image,
        secondary_name: text(pickup, "SecondaryAmmoShortName").unwrap_or_default(),
    }
}

/// `ShopStats` from a weapon class's defaults.
fn read_stats(set: &PackageSet, defaults: &ClassDefaults, wc: &ObjectHandle) -> ShopStats {
    let num = |h: &ObjectHandle, p: &str| match defaults.get(h, p) {
        Some((Value::Int(i), _)) => Some(i as f32),
        Some((Value::Float(f), _)) => Some(f),
        Some((Value::Byte(b), _)) => Some(b as f32),
        _ => None,
    };
    let mut st = ShopStats { mag: num(wc, "MagCapacity").unwrap_or(0.0) as i32, per_fire: 1, ..Default::default() };
    let fm = match defaults.get_at(wc, "FireModeClass", 0) {
        Some((Value::Object(r), rp)) if r != ObjectRef::Null => set.resolve(&rp, r),
        _ => None,
    };
    let Some(fm) = fm else { return st };
    st.fire_rate = num(&fm, "FireRate").unwrap_or(0.0);
    st.per_fire = num(&fm, "ProjPerFire").unwrap_or(1.0).max(1.0) as i32;
    let melee = num(&fm, "MeleeDamage").unwrap_or(0.0);
    let instant = num(&fm, "DamageMax").unwrap_or(0.0);
    let proj = match defaults.get(&fm, "ProjectileClass") {
        Some((Value::Object(r), rp)) if r != ObjectRef::Null => set.resolve(&rp, r).and_then(|p| num(&p, "Damage")),
        _ => None,
    };
    if melee > 0.0 {
        st.melee = true;
        st.damage = melee;
        st.per_fire = 1;
        st.mag = 0;
    } else if let Some(d) = proj.filter(|d| *d > 0.0) {
        st.damage = d;
    } else {
        st.damage = instant;
    }
    if !st.melee
        && let Some((Value::Object(r), rp)) = defaults.get(&fm, "AmmoClass")
        && let Some(ac) = set.resolve(&rp, r)
    {
        st.max_ammo = num(&ac, "MaxAmmo").unwrap_or(0.0) as i32;
    }
    st
}

/// What the menu asks of the inventory (weapon.rs carries it out).
#[derive(Message, Clone, Debug)]
pub enum ShopRequest {
    Buy(String),
    Sell(String),
    /// Ammo for a weapon: `secondary` = its second ammo (M4 203 grenades),
    /// `fill` = to MaxAmmo, else one magazine.
    Ammo { weapon: String, secondary: bool, fill: bool },
}

/// One owned weapon as the menu shows it (weapon.rs keeps this current).
#[derive(Clone, Debug)]
pub struct OwnedWeapon {
    pub weapon: String,
    pub name: String,
    pub sell_value: i32,
    pub sellable: bool,
    /// (AmmoAmount, MaxAmmo, default MagCapacity) of the first and second ammo.
    pub ammo: Option<(u32, u32, u32)>,
    /// The perk's GetAmmoCostScaling and GetMagCapacityMod for this weapon.
    pub ammo_scale: f32,
    pub mag_mod: f32,
    pub alt_ammo: Option<(u32, u32)>,
}

#[derive(Resource)]
pub struct ShopInventory {
    pub owned: Vec<OwnedWeapon>,
    pub weight: f32,
    /// MaxCarryWeight with the perk's AddCarryMaxWeight.
    pub max_weight: f32,
}

impl Default for ShopInventory {
    fn default() -> Self {
        ShopInventory { owned: Vec::new(), weight: 0.0, max_weight: MAX_CARRY_WEIGHT }
    }
}

/// KFHumanPawn MaxCarryWeight.
pub const MAX_CARRY_WEIGHT: f32 = 15.0;

/// Which trader menu draws and reads the keys (`--trader-menu`): our
/// NuMenu (numenu.rs, the default) or KF's classic screen
/// (classic_menu.rs). Both send the same requests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MenuKind {
    #[default]
    Nu,
    Kf,
}

impl MenuKind {
    pub fn parse(s: &str) -> Option<MenuKind> {
        match s.trim().to_ascii_lowercase().as_str() {
            "nu" | "numenu" => Some(MenuKind::Nu),
            "kf" | "classic" => Some(MenuKind::Kf),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            MenuKind::Nu => "nu",
            MenuKind::Kf => "kf",
        }
    }
}

/// Whether the menu is open, and which one is shown.
#[derive(Resource, Default)]
pub struct BuyMenu {
    pub open: bool,
    /// Which menu is shown (opening and closing are shared).
    pub kind: MenuKind,
}

impl BuyMenu {
    pub fn with_kind(kind: MenuKind) -> Self {
        BuyMenu { kind, ..Default::default() }
    }
}

/// A row of the "For sale" list: item, price and weight as shown
/// (KFBuyMenuSaleList.PopulateBuyables: int(Cost x GetCostScaling /
/// DualDivider)).
pub(crate) fn sale_rows(cat: &ShopCatalogue, inv: &ShopInventory, filter: usize, vet: &crate::game::perks::Vet) -> Vec<(usize, i32, f32)> {
    let Some((_, list)) = cat.lists.get(filter) else { return Vec::new() };
    list.iter()
        .filter_map(|&i| {
            let it = &cat.items[i];
            if it.never_throw || inv.owns(&it.weapon) || single_hidden(inv, &it.weapon) {
                return None;
            }
            let (price, weight) = shop_price(it, inv, vet);
            Some((i, price, weight))
        })
        .collect()
}

impl ShopInventory {
    pub fn owns(&self, weapon: &str) -> bool {
        self.owned.iter().any(|o| o.weapon.eq_ignore_ascii_case(weapon))
    }
}

/// A single is not sold while its duals are owned (both menus).
pub fn single_hidden(inv: &ShopInventory, weapon: &str) -> bool {
    DUAL_SINGLES.iter().any(|(d, s)| s.eq_ignore_ascii_case(weapon) && inv.owns(d))
}

/// A weapon's price and weight as the menus show them
/// (KFBuyMenuSaleList.PopulateBuyables: int(Cost x GetCostScaling /
/// DualDivider); a dual whose single is owned costs and weighs half).
/// The purchase itself is priced by weapon.rs (`shop_requests`) with the
/// same rule.
pub fn shop_price(it: &ShopItem, inv: &ShopInventory, vet: &crate::game::perks::Vet) -> (i32, f32) {
    let half = HALF_PRICE_DUALS.iter().any(|(d, s)| d.eq_ignore_ascii_case(&it.weapon) && inv.owns(s));
    let divider = if half { 2.0 } else { 1.0 };
    let price = (it.cost as f32 * vet.cost_scaling(&it.pickup) / divider) as i32;
    (price, it.weight / divider)
}

/// Ammo prices as KFBuyMenuInvList shows them: (clip price, fill price)
/// for a weapon's first ammo. `capacity` is the default MagCapacity;
/// `ammo_scale` and `mag_mod` the perk's GetAmmoCostScaling and
/// GetMagCapacityMod (1 without a perk). ItemAmmoCost = AmmoCost x
/// ammo_scale x mag_mod; ItemFillAmmoCost = int(missing x AmmoCost /
/// MagCapacity (the Husk Gun: BuyClipSize)) x ammo_scale.
pub fn ammo_prices(item: &ShopItem, total: u32, max: u32, capacity: u32, ammo_scale: f32, mag_mod: f32) -> (i32, i32) {
    let used = if item.weapon.eq_ignore_ascii_case("KFMod.HuskGun") { item.buy_clip_size.max(1) as f32 } else { capacity.max(1) as f32 };
    let clip = (item.ammo_cost as f32 * ammo_scale * mag_mod) as i32;
    let fill = (((max.saturating_sub(total)) as f32 * item.ammo_cost as f32 / used) as i32 as f32 * ammo_scale) as i32;
    (clip, fill)
}

pub struct BuyMenuPlugin;

impl Plugin for BuyMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BuyMenu>()
            .init_resource::<ShopCatalogue>()
            .init_resource::<ShopInventory>()
            .add_message::<ShopRequest>()
            .add_plugins((crate::game::numenu::NuMenuPlugin, crate::game::classic_menu::ClassicMenuPlugin))
            .add_systems(PreUpdate, menu_input.after(bevy::input::InputSystems));
    }
}

/// Opens and runs the menu. While it is open the keys, mouse buttons,
/// mouse motion and wheel are cleared after the menu has read them, so
/// nothing else (walking, looking, firing, doors) sees them.
#[allow(clippy::too_many_arguments)] // Bevy system parameters
pub(crate) fn menu_input(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut motion: ResMut<AccumulatedMouseMotion>,
    mut scroll: ResMut<AccumulatedMouseScroll>,
    mut menu: ResMut<BuyMenu>,
    (game, shops, cat): (Res<crate::game::waves::WaveGame>, Res<crate::game::trader::Shops>, Res<ShopCatalogue>),
    (script, frames): (Res<crate::weapons::weapon::ScriptedInput>, Res<bevy::diagnostic::FrameCount>),
    mut requests: MessageWriter<ShopRequest>,
    mut vest: MessageWriter<crate::player::armour::BuyVest>,
    vet: Res<crate::game::perks::Veterancy>,
) {
    // Scripted test actions this frame ("buy_menu", ...,
    // "buy:Shotgun", "sell:Shotgun", "ammo_fill:Shotgun", "ammo_clip:Shotgun").
    let actions: Vec<&str> = script.0.iter().filter(|(f, _)| *f == frames.0).map(|(_, a)| a.as_str()).collect();
    let act = |name: &str| actions.contains(&name);
    for a in &actions {
        if *a == "buy_vest" {
            vest.write(crate::player::armour::BuyVest);
        }
        let class = |c: &str| if c.contains('.') { c.to_string() } else { format!("KFMod.{c}") };
        if let Some(c) = a.strip_prefix("buy:") {
            requests.write(ShopRequest::Buy(class(c)));
        } else if let Some(c) = a.strip_prefix("sell:") {
            requests.write(ShopRequest::Sell(class(c)));
        } else if let Some(c) = a.strip_prefix("ammo_fill:") {
            requests.write(ShopRequest::Ammo { weapon: class(c), secondary: false, fill: true });
        } else if let Some(c) = a.strip_prefix("ammo_clip2:") {
            requests.write(ShopRequest::Ammo { weapon: class(c), secondary: true, fill: false });
        } else if let Some(c) = a.strip_prefix("ammo_clip:") {
            requests.write(ShopRequest::Ammo { weapon: class(c), secondary: false, fill: false });
        }
    }
    let wave_running = matches!(game.phase, crate::game::waves::Phase::Wave | crate::game::waves::Phase::BossWave);
    let in_shop = shops.player_inside().is_some();
    let use_pressed = keys.just_pressed(KeyCode::KeyE) || act("buy_menu");
    if !menu.open {
        // ShopVolume.UsedBy: USE inside a shop with no wave running.
        if use_pressed && in_shop && !wave_running {
            menu.open = true;
            // GUIBuyMenu: the sale list starts on your perk's
            // (BuyMenuFilterIndex = the selected perk's index; each menu
            // sets its own).
            let filter = vet.selected.map_or(0, |p| p.index());
            runlog::kv("buy_menu", &format!("open=true filter={} kind={}", cat.lists.get(filter).map_or("", |l| l.0), menu.kind.word()));
            keys.reset_all();
        }
        return;
    }
    // BootPlayers closes the menu (ClientCloseMenu); so does leaving.
    // Escape closes the top menu page (KF: the buy menu is a GUI page).
    if wave_running || !in_shop || use_pressed || keys.just_pressed(KeyCode::Backspace) || keys.just_pressed(KeyCode::Escape) {
        menu.open = false;
        runlog::kv("buy_menu", &format!("open=false wave_running={wave_running} in_shop={in_shop}"));
    }
    keys.reset_all();
    mouse.reset_all();
    motion.delta = Vec2::ZERO;
    scroll.delta = Vec2::ZERO;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(weapon: &str, cost: i32, weight: f32) -> ShopItem {
        ShopItem {
            weapon: weapon.into(),
            pickup: String::new(),
            name: weapon.into(),
            cost,
            ammo_cost: 10,
            weight,
            buy_clip_size: 0,
            never_throw: weapon == "KFMod.Single",
            stats: ShopStats::default(),
            info: ShopInfo::default(),
        }
    }

    fn owned(weapon: &str) -> OwnedWeapon {
        OwnedWeapon { weapon: weapon.into(), name: weapon.into(), sell_value: 0, sellable: true, ammo: None, alt_ammo: None, ammo_scale: 1.0, mag_mod: 1.0 }
    }

    #[test]
    fn sale_list_hides_owned_never_throw_and_halves_duals() {
        let cat = ShopCatalogue {
            items: vec![item("KFMod.Single", 150, 0.0), item("KFMod.Deagle", 500, 2.0), item("KFMod.DualDeagle", 1000, 4.0), item("KFMod.Shotgun", 500, 8.0)],
            lists: vec![("Test", vec![0, 1, 2, 3])],
        };
        let inv = ShopInventory { owned: vec![owned("KFMod.Deagle")], weight: 2.0, max_weight: MAX_CARRY_WEIGHT };
        let none = crate::game::perks::Vet::NONE;
        let rows = sale_rows(&cat, &inv, 0, &none);
        assert_eq!(rows, vec![(2, 500, 2.0), (3, 500, 8.0)]);
        // With the duals owned, the single is hidden.
        let inv = ShopInventory { owned: vec![owned("KFMod.DualDeagle")], weight: 4.0, max_weight: MAX_CARRY_WEIGHT };
        assert_eq!(sale_rows(&cat, &inv, 0, &none), vec![(3, 500, 8.0)]);
        // Support 6: shotguns 70% off: 0.9 - 0.1 x 6 is 0.29999998 in KF's
        // (and our) 32-bit floats, so int(500 x it) = 149.
        let support = crate::game::perks::Vet { perk: Some(crate::game::perks::Perk::Support), level: 6 };
        let shotgun_cat = ShopCatalogue { items: vec![ShopItem { pickup: "KFMod.ShotgunPickup".into(), ..item("KFMod.Shotgun", 500, 8.0) }], lists: vec![("Test", vec![0])] };
        assert_eq!(sale_rows(&shotgun_cat, &inv, 0, &support), vec![(0, 149, 8.0)]);
    }

    #[test]
    fn ammo_prices_follow_server_buy_ammo() {
        let shotgun = ShopItem { ammo_cost: 15, ..item("KFMod.Shotgun", 500, 8.0) };
        // Clip: one magazine (8) = AmmoCost; fill 64 -> 80: 16 / 8 x 15.
        assert_eq!(ammo_prices(&shotgun, 64, 80, 8, 1.0, 1.0), (15, 30));
        // Odd amounts round down: 3 shells = int(3 x 15 / 8) = 5.
        assert_eq!(ammo_prices(&shotgun, 77, 80, 8, 1.0, 1.0), (15, 5));
        // A perk: clip AmmoCost x 0.7 x 1.25 = 13; fill int(16 x 15 / 8) x 0.7 = 21.
        assert_eq!(ammo_prices(&shotgun, 64, 80, 8, 0.7, 1.25), (13, 21));
    }
}
