//! The buy menu (T3a): what the trader sells (KFLevelRules' per-perk
//! lists and each pickup's prices), a keyboard text menu standing in for
//! KF's GUI (KFBuyMenuSaleList / KFBuyMenuInvList), and the requests it
//! sends; weapon.rs carries them out with KFPawn's rules. See DESIGN.md,
//! T3a.

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
            weapon,
        });
    }
    // The map's own KFLevelRules, if it sets the lists.
    let pkg = &map.pkg;
    let map_rules = (0..pkg.exports.len()).find(|&i| pkg.export_class_name(i) == "KFLevelRules").and_then(|i| read_export_properties(pkg, i).ok());
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
    cat
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
    /// (AmmoAmount, MaxAmmo, MagCapacity) of the first and second ammo.
    pub ammo: Option<(u32, u32, u32)>,
    pub alt_ammo: Option<(u32, u32)>,
}

#[derive(Resource, Default)]
pub struct ShopInventory {
    pub owned: Vec<OwnedWeapon>,
    pub weight: f32,
}

/// KFHumanPawn MaxCarryWeight.
pub const MAX_CARRY_WEIGHT: f32 = 15.0;

/// KF opens on your perk's list; without perks, the first (Medic).
#[derive(Resource, Default)]
pub struct BuyMenu {
    pub open: bool,
    /// 0 "For sale", 1 "Yours".
    tab: usize,
    cursor: [usize; 2],
    /// The perk filter (index into the catalogue lists).
    filter: usize,
}

/// A row of the "For sale" list: item, price and weight as shown.
fn sale_rows(cat: &ShopCatalogue, inv: &ShopInventory, filter: usize) -> Vec<(usize, i32, f32)> {
    let owns = |w: &str| inv.owned.iter().any(|o| o.weapon.eq_ignore_ascii_case(w));
    let Some((_, list)) = cat.lists.get(filter) else { return Vec::new() };
    list.iter()
        .filter_map(|&i| {
            let it = &cat.items[i];
            if it.never_throw || owns(&it.weapon) {
                return None;
            }
            // A single while its duals are owned.
            if DUAL_SINGLES.iter().any(|(d, s)| s.eq_ignore_ascii_case(&it.weapon) && owns(d)) {
                return None;
            }
            let half = HALF_PRICE_DUALS.iter().any(|(d, s)| d.eq_ignore_ascii_case(&it.weapon) && owns(s));
            Some(if half { (i, it.cost / 2, it.weight / 2.0) } else { (i, it.cost, it.weight) })
        })
        .collect()
}

/// Ammo prices: (clip price, fill price) for a weapon's first ammo, as
/// ServerBuyAmmo works them out.
pub fn ammo_prices(item: &ShopItem, total: u32, max: u32, capacity: u32) -> (i32, i32) {
    let used = if item.weapon.eq_ignore_ascii_case("KFMod.HuskGun") { item.buy_clip_size.max(1) as f32 } else { capacity.max(1) as f32 };
    // One magazine (UsedMagCapacity rounds) costs AmmoCost.
    let clip = item.ammo_cost;
    let fill = ((max.saturating_sub(total)) as f32 / used * item.ammo_cost as f32) as i32;
    (clip, fill)
}

pub struct BuyMenuPlugin;

impl Plugin for BuyMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BuyMenu>()
            .init_resource::<ShopCatalogue>()
            .init_resource::<ShopInventory>()
            .add_message::<ShopRequest>()
            .add_systems(Startup, spawn_menu_text)
            .add_systems(PreUpdate, menu_input.after(bevy::input::InputSystems))
            .add_systems(Update, draw_menu);
    }
}

#[derive(Component)]
struct MenuText;

fn spawn_menu_text(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: bevy::text::FontSize::Px(26.0),
            ..default()
        },
        TextColor(Color::srgb(0.95, 0.9, 0.8)),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(40.0),
            left: Val::Px(40.0),
            padding: UiRect::all(Val::Px(12.0)),
            display: Display::None,
            ..default()
        },
        MenuText,
    ));
}

/// Opens and runs the menu. While it is open the keys, mouse buttons,
/// mouse motion and wheel are cleared after the menu has read them, so
/// nothing else (walking, looking, firing, doors) sees them.
#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn menu_input(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut motion: ResMut<AccumulatedMouseMotion>,
    mut scroll: ResMut<AccumulatedMouseScroll>,
    mut menu: ResMut<BuyMenu>,
    (game, shops, cat, inv): (Res<crate::game::waves::WaveGame>, Res<crate::game::trader::Shops>, Res<ShopCatalogue>, Res<ShopInventory>),
    (script, frames): (Res<crate::weapons::weapon::ScriptedInput>, Res<bevy::diagnostic::FrameCount>),
    mut requests: MessageWriter<ShopRequest>,
    mut vest: MessageWriter<crate::player::armour::BuyVest>,
) {
    // Scripted test actions this frame ("buy_menu", "menu_down", ...,
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
            runlog::kv("buy_menu", "open=true");
            keys.reset_all();
        }
        return;
    }
    // BootPlayers closes the menu (ClientCloseMenu); so does leaving.
    if wave_running || !in_shop || use_pressed || keys.just_pressed(KeyCode::Backspace) {
        menu.open = false;
        runlog::kv("buy_menu", &format!("open=false wave_running={wave_running} in_shop={in_shop}"));
    } else {
        // "Yours" ends with the vest row (KFBuyMenuInvList adds it last).
        let rows = [sale_rows(&cat, &inv, menu.filter).len(), inv.owned.len() + 1];
        let tab = menu.tab;
        let n = rows[tab];
        if keys.just_pressed(KeyCode::ArrowDown) || act("menu_down") {
            menu.cursor[tab] = (menu.cursor[tab] + 1).min(n.saturating_sub(1));
        }
        if keys.just_pressed(KeyCode::ArrowUp) || act("menu_up") {
            menu.cursor[tab] = menu.cursor[tab].saturating_sub(1);
        }
        if keys.just_pressed(KeyCode::ArrowRight) || act("menu_right") {
            menu.filter = (menu.filter + 1) % cat.lists.len().max(1);
            menu.cursor[0] = 0;
        }
        if keys.just_pressed(KeyCode::ArrowLeft) || act("menu_left") {
            let len = cat.lists.len().max(1);
            menu.filter = (menu.filter + len - 1) % len;
            menu.cursor[0] = 0;
        }
        if keys.just_pressed(KeyCode::Tab) || act("menu_tab") {
            menu.tab = 1 - menu.tab;
        }
        let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        let cursor = menu.cursor[menu.tab];
        if keys.just_pressed(KeyCode::Enter) || act("menu_enter") {
            if menu.tab == 0 {
                if let Some(&(i, _, _)) = sale_rows(&cat, &inv, menu.filter).get(cursor) {
                    requests.write(ShopRequest::Buy(cat.items[i].weapon.clone()));
                }
            } else if let Some(o) = inv.owned.get(cursor) {
                requests.write(ShopRequest::Sell(o.weapon.clone()));
            } else if cursor == inv.owned.len() {
                vest.write(crate::player::armour::BuyVest);
            }
        }
        if (keys.just_pressed(KeyCode::KeyF) || act("menu_fill")) && menu.tab == 1 && cursor == inv.owned.len() {
            vest.write(crate::player::armour::BuyVest);
        }
        for (key, name, fill) in [(KeyCode::KeyF, "menu_fill", true), (KeyCode::KeyC, "menu_clip", false)] {
            if (keys.just_pressed(key) || act(name))
                && menu.tab == 1
                && let Some(o) = inv.owned.get(cursor)
            {
                requests.write(ShopRequest::Ammo { weapon: o.weapon.clone(), secondary: shift, fill });
            }
        }
    }
    keys.reset_all();
    mouse.reset_all();
    motion.delta = Vec2::ZERO;
    scroll.delta = Vec2::ZERO;
}

fn draw_menu(
    menu: Res<BuyMenu>,
    cat: Res<ShopCatalogue>,
    inv: Res<ShopInventory>,
    dosh: Res<crate::game::dosh::Dosh>,
    game: Res<crate::game::waves::WaveGame>,
    armour: Res<crate::player::armour::Armour>,
    mut text: Query<(&mut Text, &mut Node), With<MenuText>>,
) {
    let Ok((mut t, mut node)) = text.single_mut() else { return };
    if !menu.open {
        node.display = Display::None;
        return;
    }
    node.display = Display::Flex;
    let score = dosh.score as i32;
    let mut s = format!(
        "TRADER    DOSH {score}    WEIGHT {:.0}/{MAX_CARRY_WEIGHT:.0}    NEXT WAVE IN {}\n",
        inv.weight,
        game.countdown.max(0)
    );
    s += "Up/Down select   Tab switch list   Enter buy/sell   C clip   F fill (Shift: 2nd ammo)   E close\n\n";
    let filter = cat.lists.get(menu.filter).map_or("", |l| l.0);
    s += &format!("{}FOR SALE   < {filter} >\n", if menu.tab == 0 { "> " } else { "  " });
    let rows = sale_rows(&cat, &inv, menu.filter);
    if rows.is_empty() {
        s += "    (nothing)\n";
    }
    for (k, &(i, price, weight)) in rows.iter().enumerate() {
        let it = &cat.items[i];
        let mark = if menu.tab == 0 && k == menu.cursor[0] { ">>" } else { "  " };
        let why = if price > score {
            "  (not enough dosh)"
        } else if weight > 0.0 && inv.weight + weight > MAX_CARRY_WEIGHT {
            "  (too heavy)"
        } else {
            ""
        };
        s += &format!("  {mark} {:<28} {:>5}   {:>2} kg{why}\n", it.name, price, weight);
    }
    s += &format!("\n{}YOURS\n", if menu.tab == 1 { "> " } else { "  " });
    for (k, o) in inv.owned.iter().enumerate() {
        let mark = if menu.tab == 1 && k == menu.cursor[1] { ">>" } else { "  " };
        let ammo = match (o.ammo, cat.item(&o.weapon)) {
            (Some((total, max, cap)), Some(item)) => {
                let (clip, fill) = ammo_prices(item, total, max, cap);
                format!("ammo {total}/{max}  clip {clip}  fill {fill}")
            }
            _ => String::new(),
        };
        let alt = match (o.alt_ammo, cat.item(&o.weapon)) {
            (Some((cur, max)), Some(item)) => format!("  2nd {cur}/{max} ({} each)", item.ammo_cost),
            _ => String::new(),
        };
        let sell = if o.sellable { format!("sell {}", o.sell_value) } else { String::new() };
        s += &format!("  {mark} {:<28} {:<10} {ammo}{alt}\n", o.name, sell);
    }
    let mark = if menu.tab == 1 && menu.cursor[1] == inv.owned.len() { ">>" } else { "  " };
    let (points, fill) = crate::player::armour::menu_row(&armour);
    // BuyableVest.ItemName.
    s += &format!("  {mark} {:<28} {:<10} armour {points}/100  fill {fill}\n", "Combat armour", "");
    **t = s;
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
        }
    }

    fn owned(weapon: &str) -> OwnedWeapon {
        OwnedWeapon { weapon: weapon.into(), name: weapon.into(), sell_value: 0, sellable: true, ammo: None, alt_ammo: None }
    }

    #[test]
    fn sale_list_hides_owned_never_throw_and_halves_duals() {
        let cat = ShopCatalogue {
            items: vec![item("KFMod.Single", 150, 0.0), item("KFMod.Deagle", 500, 2.0), item("KFMod.DualDeagle", 1000, 4.0), item("KFMod.Shotgun", 500, 8.0)],
            lists: vec![("Test", vec![0, 1, 2, 3])],
        };
        let inv = ShopInventory { owned: vec![owned("KFMod.Deagle")], weight: 2.0 };
        let rows = sale_rows(&cat, &inv, 0);
        assert_eq!(rows, vec![(2, 500, 2.0), (3, 500, 8.0)]);
        // With the duals owned, the single is hidden.
        let inv = ShopInventory { owned: vec![owned("KFMod.DualDeagle")], weight: 4.0 };
        assert_eq!(sale_rows(&cat, &inv, 0), vec![(3, 500, 8.0)]);
    }

    #[test]
    fn ammo_prices_follow_server_buy_ammo() {
        let shotgun = ShopItem { ammo_cost: 15, ..item("KFMod.Shotgun", 500, 8.0) };
        // Clip: one magazine (8) = AmmoCost; fill 64 -> 80: 16 / 8 x 15.
        assert_eq!(ammo_prices(&shotgun, 64, 80, 8), (15, 30));
        // Odd amounts round down: 3 shells = int(3 / 8 x 15) = 5.
        assert_eq!(ammo_prices(&shotgun, 77, 80, 8), (15, 5));
    }
}
