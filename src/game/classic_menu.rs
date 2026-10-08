//! The classic trader menu (`--trader-menu kf`): a copy of KF's
//! GUIBuyMenu / KFTab_BuyMenu screen, its layout read from `System/KFGui.u`
//! (menus/gui.rs `COMPONENTS`, `CLASS_VALUES`) and drawn with KF's
//! textures and fonts by the menus' painter. Opening, closing, the input
//! freeze and every purchase rule are shared (buy_menu.rs, weapon.rs,
//! armour.rs): this file only draws and turns clicks, keys and test
//! actions into `ShopRequest` / `BuyVest` / `PerkRequest`. See DESIGN.md,
//! "The classic KF trader menu".

use bevy::ecs::system::SystemParam;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use crate::engine::runlog;
use crate::game::buy_menu::{BuyMenu, MenuKind, ShopCatalogue, ShopInventory, ShopRequest, ammo_prices, sale_rows, single_hidden};
use crate::game::menus::gui::{Align, Gui, Painter, named_font};
use crate::game::perks::{Perk, Veterancy};
use crate::player::armour::{Armour, BuyVest};

/// Textures the screen uses beyond menus/gui.rs `TEXTURES` (loaded only
/// with `--trader-menu kf`, with each weapon's TraderInfoTexture).
pub const TEXTURES: &[&str] = &[
    "KF_InterfaceArt_tex.Menu.Thick_border_Transparent",
    "KF_InterfaceArt_tex.Menu.Med_border_Transparent",
    "KF_InterfaceArt_tex.Menu.Thin_border_Transparent",
    "KF_InterfaceArt_tex.Menu.Innerborder_transparent",
    "KF_InterfaceArt_tex.Menu.Perk_box",
    "KF_InterfaceArt_tex.Menu.Perk_box_unselected",
    "KF_InterfaceArt_tex.Menu.Item_box_bar_Disabled",
    "KF_InterfaceArt_tex.Menu.button_Disabled",
    "KF_InterfaceArt_tex.Menu.Progress",
    "KillingFloorHUD.HUD.Hud_Weight",
    "PatchTex.Statics.BanknoteSkin",
    "KillingFloor2HUD.Perk_Icons.No_Perk_Icon",
    "KillingFloor2HUD.Perk_Icons.Favorite_Perk_Icon",
    VEST_IMAGE,
];

/// BuyableVest.ItemImage, ItemName, ItemDescription.
const VEST_IMAGE: &str = "KillingFloorHUD.Trader_Weapon_Images.Trader_Vest";
const VEST_NAME: &str = "Combat armour";
const VEST_DESCRIPTION: &str = "Kevlar vest. Affords the wearer limited protection from most forms of attack.";

/// KF's text colour for these labels (TextColor 175,176,158, also
/// KFWeightBar.CurrentColor and GUIBuyMenu.GreenGreyColor).
const GREEN_GREY: [u8; 4] = [175, 176, 158, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

/// KFTab_BuyMenu InfoText[0-2] (KFGui.int says the same).
const INFO_WELCOME: &str = "Welcome to my shop! You can buy ammo or sell from your inventory on the left. Or you can buy new items from the right.";
const INFO_HEAVY: &str = "Item is too heavy! It requires %1 free weight blocks, you only have %2 free. Sell some of your inventory!";
const INFO_EXPENSIVE: &str = "Item is too expensive! Ask some blokes to spare some money or sell some of your inventory!";

/// The inventory list's rows (KFBuyMenuInvList: 7 weapons, then
/// MyBuyables[7] = none (the "Equipment" label), 8 the knife (or the
/// second ammo), 9 the grenades, 10 the armour).
pub const INV_ROWS: usize = 11;
const MAX_WEAPONS: usize = 7;
/// KFBuyMenuSaleList.SaleItemHeight: the box's height / 10.
const SALE_ROWS: usize = 10;
/// The ninth filter (KFBuyMenuFilter PSI8 "Favorites": KFLR.FaveItemForSale).
const FILTERS: usize = 9;
/// UE2's double click is native: assumed 0.5 s (a guess).
const DOUBLE_CLICK: f32 = 0.5;

/// What an inventory row is.
#[derive(Clone, Debug, PartialEq)]
pub enum InvKind {
    /// A weapon, or its second ammo (`secondary`: the M4 203's grenades).
    Weapon { weapon: String, secondary: bool },
    Vest,
}

/// One row of the inventory list (a GUIBuyable of UpdateMyBuyables).
#[derive(Clone, Debug, PartialEq)]
pub struct InvRow {
    pub kind: InvKind,
    pub name: String,
    pub perk_index: usize,
    pub melee: bool,
    pub sellable: bool,
    pub sell_value: i32,
    /// (current, max) ammo; the armour: (points, 100).
    pub ammo: Option<(u32, u32)>,
    /// ItemAmmoCost (one magazine; the armour: one point, as a float in KF).
    pub clip_cost: f32,
    /// ItemFillAmmoCost (to full).
    pub fill_cost: i32,
    /// ItemCost (the armour's full price, for its button).
    pub cost: i32,
    /// Index into the catalogue's items (None: the armour).
    pub item: Option<usize>,
}

impl InvRow {
    fn weapon(&self) -> Option<&str> {
        match &self.kind {
            InvKind::Weapon { weapon, .. } => Some(weapon),
            InvKind::Vest => None,
        }
    }

    fn secondary(&self) -> bool {
        matches!(self.kind, InvKind::Weapon { secondary: true, .. })
    }

    /// KFBuyMenuInvList.UpdateList's AmmoStrings.
    pub fn ammo_text(&self) -> String {
        match (self.ammo, &self.kind) {
            (Some((c, m)), InvKind::Vest) => format!("{}%", (c as f32 / m.max(1) as f32 * 100.0) as i32),
            (Some((c, m)), _) => format!("{c}/{m}"),
            _ => String::new(),
        }
    }

    /// ClipPriceStrings: the fill price when it is the cheaper.
    pub fn clip_text(&self) -> String {
        match self.ammo {
            Some((c, m)) if c < m => format!("£ {}", if self.clip_cost > self.fill_cost as f32 { self.fill_cost } else { self.clip_cost as i32 }),
            _ => "£ 0".into(),
        }
    }

    /// FillPriceStrings (the armour: Buy / Purchased / Repair).
    pub fn fill_text(&self) -> String {
        match (&self.kind, self.ammo) {
            (InvKind::Vest, Some((0, _))) => format!("Buy : £ {}", self.fill_cost),
            (InvKind::Vest, Some((c, m))) if c >= m => "Purchased".into(),
            (InvKind::Vest, _) => format!("Repair : £ {}", self.fill_cost),
            _ => format!("£ {}", self.fill_cost),
        }
    }

    /// DrawInvItem's disabled ammo buttons.
    pub fn buttons_disabled(&self, dosh: f32) -> bool {
        let Some((c, m)) = self.ammo else { return true };
        match self.kind {
            InvKind::Vest => (c > 0 && dosh < self.clip_cost) || (c == 0 && dosh < self.cost as f32) || c >= m,
            _ => c >= m || (dosh < self.fill_cost as f32 && dosh < self.clip_cost),
        }
    }
}

fn is(class: &str, name: &str) -> bool {
    class.eq_ignore_ascii_case(name)
}

/// KFBuyMenuInvList.UpdateMyBuyables: the 11 rows. Weapons are listed in
/// our inventory's order (KF walks its linked list and inserts each at
/// the top; that order is not checked), at most 7; Welder and Syringe
/// are left out, a single while its duals are owned too; slot 8 is the
/// knife unless a weapon has a second ammo (then that ammo, KF's quirk:
/// the knife is not shown).
pub fn inv_rows(cat: &ShopCatalogue, inv: &ShopInventory, armour: f32, vest_scaling: f32) -> Vec<Option<InvRow>> {
    let mut rows: Vec<Option<InvRow>> = vec![None; INV_ROWS];
    let (mut knife, mut frag, mut second) = (None, None, None);
    let mut n = 0;
    for o in &inv.owned {
        if is(&o.weapon, "KFMod.Welder") || is(&o.weapon, "KFMod.Syringe") || single_hidden(inv, &o.weapon) {
            continue;
        }
        let Some(idx) = cat.items.iter().position(|i| i.weapon.eq_ignore_ascii_case(&o.weapon)) else { continue };
        let item = &cat.items[idx];
        let melee = item.stats.melee;
        let (ammo, clip_cost, fill_cost) = match o.ammo {
            Some((total, max, cap)) if !melee => {
                let (clip, fill) = ammo_prices(item, total, max, cap, o.ammo_scale, o.mag_mod);
                (Some((total, max)), clip as f32, fill)
            }
            _ => (None, 0.0, 0),
        };
        let row = InvRow {
            kind: InvKind::Weapon { weapon: o.weapon.clone(), secondary: false },
            name: item.info.short_name.clone(),
            perk_index: item.info.perk_index,
            melee,
            sellable: o.sellable,
            sell_value: o.sell_value,
            ammo,
            clip_cost,
            fill_cost,
            cost: item.cost,
            item: Some(idx),
        };
        if is(&o.weapon, "KFMod.Knife") {
            knife = Some(InvRow { sellable: false, ..row });
            continue;
        }
        if is(&o.weapon, "KFMod.Frag") {
            frag = Some(InvRow { sellable: false, ..row });
            continue;
        }
        if let Some((cur, max)) = o.alt_ammo {
            // The second ammo: each costs AmmoCost x GetAmmoCostScaling
            // (as weapon.rs charges it; numenu.rs `alt_fill`).
            let each = item.ammo_cost as f32 * o.ammo_scale;
            second = Some(InvRow {
                kind: InvKind::Weapon { weapon: o.weapon.clone(), secondary: true },
                name: if item.info.secondary_name.is_empty() { format!("{} grenades", item.info.short_name) } else { item.info.secondary_name.clone() },
                sellable: false,
                ammo: Some((cur, max)),
                clip_cost: each,
                fill_cost: (max.saturating_sub(cur) as f32 * each) as i32,
                ..row.clone()
            });
        }
        if n < MAX_WEAPONS {
            rows[n] = Some(row);
            n += 1;
        }
    }
    rows[8] = second.or(knife);
    rows[9] = frag;
    // The armour: ItemCost int(300 x GetCostScaling), ItemAmmoCost
    // ItemCost / 100, ItemFillAmmoCost int((100 - points) x it).
    let cost = (crate::player::armour::VEST_COST * vest_scaling) as i32;
    let each = cost as f32 / 100.0;
    rows[10] = Some(InvRow {
        kind: InvKind::Vest,
        name: VEST_NAME.into(),
        perk_index: 0,
        melee: false,
        sellable: false,
        sell_value: 0,
        ammo: Some((armour.max(0.0) as u32, 100)),
        clip_cost: each,
        fill_cost: ((100.0 - armour) * each) as i32,
        cost,
        item: None,
    });
    rows
}

/// The selected item: an inventory slot or a sale weapon (KF keeps one
/// of InvSelect / SaleSelect at -1).
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Sel {
    #[default]
    None,
    Inv(usize),
    Sale(String),
}

/// A request sent this frame; its result is logged next frame.
#[derive(Clone, Debug)]
struct Pending {
    action: &'static str,
    what: String,
    dosh: f32,
    weight: f32,
    armour: f32,
}

#[derive(Resource, Default)]
pub struct ClassicMenu {
    pub sel: Sel,
    /// The list the keys move in: false inventory, true for sale.
    pub sale_focus: bool,
    /// KFPlayerController.BuyMenuFilterIndex (0-8).
    pub filter: usize,
    /// The sale list's first shown row.
    pub top: usize,
    /// Clickable boxes drawn last frame (physical pixels).
    hits: Vec<(String, Rect)>,
    /// The last click (id, seconds) for double clicks.
    last_click: Option<(String, f32)>,
    pending: Vec<Pending>,
    was_open: bool,
    cursor_was_grabbed: bool,
    /// Window size the layout was last logged for.
    logged_size: Vec2,
}

pub struct ClassicMenuPlugin;

impl Plugin for ClassicMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClassicMenu>().add_systems(PreUpdate, input.after(bevy::input::InputSystems).before(crate::game::buy_menu::menu_input));
    }
}

/// One thing to do, from a key, a click or a test action.
#[derive(Clone, Debug, PartialEq)]
enum Cmd {
    Up,
    Down,
    SwitchList,
    Filter(usize),
    PrevFilter,
    NextFilter,
    SelectInv(usize),
    SelectSale(usize),
    /// A double click on a row (sale: buy; inventory name: sell).
    Double(bool, usize),
    Enter,
    Buy,
    Sell,
    Clip(Option<usize>),
    Fill(Option<usize>),
    Vest,
    FillAll,
    Perk(Perk),
    Scroll(i32),
    PerkTab,
    Close,
}

/// A clicked box's id as a command.
fn click_cmd(id: &str) -> Option<Cmd> {
    let num = |p: &str| id.strip_prefix(p).and_then(|n| n.parse::<usize>().ok());
    Some(match id {
        "kf.purchase" => Cmd::Buy,
        "kf.sell" => Cmd::Sell,
        "kf.autofill" => Cmd::FillAll,
        "kf.exit" => Cmd::Close,
        "kf.vest" => Cmd::Vest,
        "kf.perk_tab" => Cmd::PerkTab,
        "kf.store_tab" => return None,
        "kf.scroll:-1" => Cmd::Scroll(-1),
        "kf.scroll:1" => Cmd::Scroll(1),
        _ => {
            if let Some(n) = num("kf.sale:") {
                Cmd::SelectSale(n)
            } else if let Some(n) = num("kf.inv:") {
                Cmd::SelectInv(n)
            } else if let Some(n) = num("kf.clip:") {
                Cmd::Clip(Some(n))
            } else if let Some(n) = num("kf.fill:") {
                Cmd::Fill(Some(n))
            } else if let Some(n) = num("kf.filter:") {
                Cmd::Filter(n)
            } else {
                Cmd::Perk(*Perk::ALL.get(num("kf.perk:")?)?)
            }
        }
    })
}

/// The shared shop state the menu reads.
type ShopState<'w> = (Res<'w, ShopCatalogue>, Res<'w, ShopInventory>, Res<'w, crate::game::dosh::Dosh>, Res<'w, Armour>, Res<'w, Veterancy>);

/// What the input works on.
struct Ctx<'a> {
    cat: &'a ShopCatalogue,
    inv: &'a ShopInventory,
    dosh: f32,
    armour: f32,
    vet: &'a Veterancy,
    rows: Vec<Option<InvRow>>,
    /// (catalogue index, shown price, shown weight) of the filter's list.
    sale: Vec<(usize, i32, f32)>,
}

impl Ctx<'_> {
    fn new<'a>(cat: &'a ShopCatalogue, inv: &'a ShopInventory, dosh: f32, armour: f32, vet: &'a Veterancy, filter: usize) -> Ctx<'a> {
        Ctx {
            cat,
            inv,
            dosh,
            armour,
            vet,
            rows: inv_rows(cat, inv, armour, vet.vet.cost_scaling("vest")),
            sale: sale_rows(cat, inv, filter, &vet.vet),
        }
    }

    fn sale_index(&self, weapon: &str) -> Option<usize> {
        self.sale.iter().position(|&(i, _, _)| self.cat.items[i].weapon.eq_ignore_ascii_case(weapon))
    }

    /// The selection still listed (a sold or bought item is gone).
    fn valid(&self, sel: &Sel) -> bool {
        match sel {
            Sel::None => true,
            Sel::Inv(i) => self.rows.get(*i).is_some_and(Option::is_some),
            Sel::Sale(w) => self.sale_index(w).is_some(),
        }
    }

    fn describe(&self, sel: &Sel) -> String {
        match sel {
            Sel::None => "none".into(),
            Sel::Inv(i) => match self.rows.get(*i).and_then(Option::as_ref) {
                Some(r) => format!("inv:{i}:{}", r.weapon().unwrap_or("vest").trim_start_matches("KFMod.")),
                None => format!("inv:{i}:empty"),
            },
            Sel::Sale(w) => format!("sale:{}", w.trim_start_matches("KFMod.")),
        }
    }
}

enum Effect {
    Request(ShopRequest),
    Vest,
    Perk(Perk),
    Close,
}

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn input(
    (keys, mouse, scroll): (Res<ButtonInput<KeyCode>>, Res<ButtonInput<MouseButton>>, Res<AccumulatedMouseScroll>),
    mut menu: ResMut<BuyMenu>,
    mut cm: ResMut<ClassicMenu>,
    (cat, inv, dosh, armour, vet): ShopState,
    (script, frames, time): (Res<crate::weapons::weapon::ScriptedInput>, Res<bevy::diagnostic::FrameCount>, Res<Time>),
    (mut requests, mut vest, mut perks): (MessageWriter<ShopRequest>, MessageWriter<BuyVest>, MessageWriter<crate::game::perks::PerkRequest>),
    mut window: Query<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
) {
    let cm = &mut *cm;
    // Last frame's requests have been carried out: their results.
    for p in std::mem::take(&mut cm.pending) {
        runlog::kv(
            "classic_menu_result",
            &format!(
                "action={} item={} dosh_before={:.0} dosh_after={:.0} weight_before={} weight_after={}/{} armour_before={:.0} armour_after={:.0}",
                p.action, p.what, p.dosh, dosh.score, p.weight, inv.weight, inv.max_weight, p.armour, armour.strength
            ),
        );
    }
    let open = menu.open && menu.kind == MenuKind::Kf;
    let mut win = window.single_mut().ok();
    if open && !cm.was_open {
        // GUIBuyMenu.Opened / KFTab_BuyMenu.ShowPanel: the filter starts
        // on your perk (KFBuyMenuFilter.CheckPerks), the 9mm (or the
        // dualies) is selected, the mouse cursor is freed.
        cm.filter = vet.selected.map_or(0, |p| p.index());
        cm.top = 0;
        cm.sale_focus = false;
        let c = Ctx::new(&cat, &inv, dosh.score, armour.strength, &vet, cm.filter);
        cm.sel = c
            .rows
            .iter()
            .position(|r| r.as_ref().and_then(InvRow::weapon).is_some_and(|w| is(w, "KFMod.Single") || is(w, "KFMod.Dualies")))
            .map_or(Sel::None, Sel::Inv);
        cm.last_click = None;
        cm.logged_size = Vec2::ZERO;
        if let Some((_, c)) = win.as_mut() {
            cm.cursor_was_grabbed = c.grab_mode != CursorGrabMode::None;
            c.grab_mode = CursorGrabMode::None;
            c.visible = true;
        }
        let (_, fill_all) = crate::game::numenu::fill_all_plan(&cat, &inv);
        runlog::kv(
            "classic_menu_open",
            &format!(
                "dosh={:.0} weight={}/{} perk={} filter={} sale_rows={} can_buy={} inv_rows={} selected={} auto_fill={fill_all} armour={:.0}",
                dosh.score,
                inv.weight,
                inv.max_weight,
                vet.vet.label(),
                cm.filter,
                c.sale.len(),
                c.sale.iter().filter(|&&(_, p, w)| p as f32 <= c.dosh && inv.weight + w <= inv.max_weight).count(),
                c.rows.iter().filter(|r| r.is_some()).count(),
                c.describe(&cm.sel),
                armour.strength
            ),
        );
    } else if !open && cm.was_open {
        if cm.cursor_was_grabbed
            && let Some((_, c)) = win.as_mut()
        {
            c.grab_mode = CursorGrabMode::Locked;
            c.visible = false;
        }
        runlog::kv("classic_menu_close", &format!("dosh={:.0} weight={}/{}", dosh.score, inv.weight, inv.max_weight));
    }
    cm.was_open = open;
    if !open {
        return;
    }
    let mut cmds = Vec::new();
    // Keys (ours: KF's menu is driven by the mouse). buy_menu.rs clears
    // them afterwards, so nothing else sees them.
    const PERK_KEYS: [KeyCode; 7] = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6, KeyCode::Digit7];
    for (k, p) in PERK_KEYS.iter().zip(Perk::ALL) {
        if keys.just_pressed(*k) {
            cmds.push(Cmd::Perk(p));
        }
    }
    for (k, c) in [
        (KeyCode::ArrowUp, Cmd::Up),
        (KeyCode::ArrowDown, Cmd::Down),
        (KeyCode::Tab, Cmd::SwitchList),
        (KeyCode::ArrowLeft, Cmd::PrevFilter),
        (KeyCode::ArrowRight, Cmd::NextFilter),
        (KeyCode::Enter, Cmd::Enter),
        (KeyCode::NumpadEnter, Cmd::Enter),
        (KeyCode::KeyC, Cmd::Clip(None)),
        (KeyCode::KeyF, Cmd::Fill(None)),
        (KeyCode::KeyA, Cmd::FillAll),
    ] {
        if keys.just_pressed(k) {
            cmds.push(c);
        }
    }
    let now = time.elapsed_secs();
    let click = |cm: &mut ClassicMenu, id: &str, source: &str, cmds: &mut Vec<Cmd>| {
        let double = cm.last_click.as_ref().is_some_and(|(last, t)| last == id && now - t <= DOUBLE_CLICK);
        runlog::kv("classic_menu_click", &format!("id={id} source={source} double={double}"));
        let num = |p: &str| id.strip_prefix(p).and_then(|n| n.parse::<usize>().ok());
        match (double, num("kf.sale:"), num("kf.inv:")) {
            (true, Some(n), _) => cmds.push(Cmd::Double(true, n)),
            (true, _, Some(n)) => cmds.push(Cmd::Double(false, n)),
            _ => cmds.extend(click_cmd(id)),
        }
        cm.last_click = if double { None } else { Some((id.to_string(), now)) };
    };
    // The mouse: a click on a box drawn last frame, the wheel.
    if mouse.just_pressed(MouseButton::Left)
        && let Some(pos) = win.as_ref().and_then(|(w, _)| w.physical_cursor_position())
        && let Some((id, _)) = cm.hits.iter().rev().find(|(_, r)| r.contains(pos)).cloned()
    {
        click(cm, &id, "mouse", &mut cmds);
    }
    if scroll.delta.y != 0.0 {
        cmds.push(Cmd::Scroll(if scroll.delta.y > 0.0 { -1 } else { 1 }));
    }
    // Test actions: "kf:..." and the old list's "menu_*".
    let c0 = Ctx::new(&cat, &inv, dosh.score, armour.strength, &vet, cm.filter);
    for (_, a) in script.0.iter().filter(|(f, _)| *f == frames.0) {
        let old = match a.as_str() {
            "menu_down" => Some("down"),
            "menu_up" => Some("up"),
            "menu_left" => Some("left"),
            "menu_right" => Some("right"),
            "menu_tab" => Some("tab"),
            "menu_enter" => Some("enter"),
            "menu_fill" => Some("fill"),
            "menu_clip" => Some("clip"),
            _ => None,
        };
        let Some(a) = old.or_else(|| a.strip_prefix("kf:")) else { continue };
        let c = match a {
            "up" => Some(Cmd::Up),
            "down" => Some(Cmd::Down),
            "tab" => Some(Cmd::SwitchList),
            "left" => Some(Cmd::PrevFilter),
            "right" => Some(Cmd::NextFilter),
            "enter" => Some(Cmd::Enter),
            "buy" => Some(Cmd::Buy),
            "sell" => Some(Cmd::Sell),
            "clip" => Some(Cmd::Clip(None)),
            "fill" => Some(Cmd::Fill(None)),
            "fill_all" => Some(Cmd::FillAll),
            "vest" => Some(Cmd::Vest),
            "close" => Some(Cmd::Close),
            _ => {
                if let Some(n) = a.strip_prefix("filter:").and_then(|n| n.parse::<usize>().ok()) {
                    Some(Cmd::Filter(n))
                } else if let Some(n) = a.strip_prefix("wheel:").and_then(|n| n.parse::<i32>().ok()) {
                    Some(Cmd::Scroll(n))
                } else if let Some(c) = a.strip_prefix("select:") {
                    let class = if c.contains('.') { c.to_string() } else { format!("KFMod.{c}") };
                    match c0.sale_index(&class) {
                        Some(r) => Some(Cmd::SelectSale(r)),
                        None => match c0.rows.iter().position(|r| r.as_ref().and_then(InvRow::weapon).is_some_and(|w| is(w, &class))) {
                            Some(g) => Some(Cmd::SelectInv(g)),
                            None => {
                                runlog::kv("classic_menu_action", &format!("action=select weapon={class} result=not_listed"));
                                None
                            }
                        },
                    }
                } else if let Some(id) = a.strip_prefix("click:") {
                    // As a mouse click: only on a box that was drawn.
                    if cm.hits.iter().any(|(h, _)| h == id) {
                        click(cm, id, "test", &mut cmds);
                    } else {
                        runlog::kv("classic_menu_click", &format!("id={id} source=test result=no_such_box"));
                    }
                    None
                } else {
                    runlog::kv("classic_menu_action", &format!("action={a} result=unknown"));
                    None
                }
            }
        };
        cmds.extend(c);
    }
    for c in cmds {
        let ctx = Ctx::new(&cat, &inv, dosh.score, armour.strength, &vet, cm.filter);
        for e in apply(cm, &ctx, c) {
            match e {
                Effect::Request(r) => {
                    requests.write(r);
                }
                Effect::Vest => {
                    vest.write(BuyVest);
                }
                Effect::Perk(p) => {
                    perks.write(crate::game::perks::PerkRequest(p));
                }
                Effect::Close => {
                    menu.open = false;
                    runlog::kv("buy_menu", "open=false reason=classic_exit_button");
                }
            }
        }
    }
}

/// Keeps the sale selection's row on screen.
fn follow(cm: &mut ClassicMenu, c: &Ctx) {
    if let Sel::Sale(w) = &cm.sel
        && let Some(r) = c.sale_index(w)
    {
        if r < cm.top {
            cm.top = r;
        } else if r >= cm.top + SALE_ROWS {
            cm.top = r + 1 - SALE_ROWS;
        }
    }
}

fn log_select(cm: &ClassicMenu, c: &Ctx, how: &str) {
    runlog::kv("classic_menu_select", &format!("selected={} how={how} filter={} top={}", c.describe(&cm.sel), cm.filter, cm.top));
}

fn apply(cm: &mut ClassicMenu, c: &Ctx, cmd: Cmd) -> Vec<Effect> {
    let mut out = Vec::new();
    if !c.valid(&cm.sel) {
        cm.sel = Sel::None;
    }
    let inv_used: Vec<usize> = (0..INV_ROWS).filter(|&i| c.rows[i].is_some()).collect();
    let pending = |action: &'static str, what: &str| Pending { action, what: what.to_string(), dosh: c.dosh, weight: c.inv.weight, armour: c.armour };
    match cmd {
        Cmd::Up | Cmd::Down => {
            let down = cmd == Cmd::Down;
            if cm.sale_focus {
                let n = c.sale.len();
                if n > 0 {
                    let cur = match &cm.sel {
                        Sel::Sale(w) => c.sale_index(w),
                        _ => None,
                    };
                    let r = match (cur, down) {
                        (None, _) => 0,
                        (Some(r), true) => (r + 1).min(n - 1),
                        (Some(r), false) => r.saturating_sub(1),
                    };
                    cm.sel = Sel::Sale(c.cat.items[c.sale[r].0].weapon.clone());
                    follow(cm, c);
                }
            } else if !inv_used.is_empty() {
                let cur = match cm.sel {
                    Sel::Inv(i) => inv_used.iter().position(|&u| u == i),
                    _ => None,
                };
                let k = match (cur, down) {
                    (None, _) => 0,
                    (Some(k), true) => (k + 1).min(inv_used.len() - 1),
                    (Some(k), false) => k.saturating_sub(1),
                };
                cm.sel = Sel::Inv(inv_used[k]);
            }
            log_select(cm, c, "key");
        }
        Cmd::SwitchList => {
            cm.sale_focus = !cm.sale_focus;
            runlog::kv("classic_menu_focus", &format!("list={}", if cm.sale_focus { "sale" } else { "inventory" }));
        }
        Cmd::Filter(n) => set_filter(cm, n.min(FILTERS - 1)),
        Cmd::PrevFilter => set_filter(cm, (cm.filter + FILTERS - 1) % FILTERS),
        Cmd::NextFilter => set_filter(cm, (cm.filter + 1) % FILTERS),
        Cmd::SelectSale(r) => {
            if let Some(&(i, _, _)) = c.sale.get(r) {
                cm.sel = Sel::Sale(c.cat.items[i].weapon.clone());
                cm.sale_focus = true;
                log_select(cm, c, "click");
            }
        }
        Cmd::SelectInv(i) => {
            if c.rows.get(i).is_some_and(Option::is_some) {
                cm.sel = Sel::Inv(i);
                cm.sale_focus = false;
                log_select(cm, c, "click");
            }
        }
        // SaleDblClick: buys if it fits and is affordable; InvDblClick:
        // sells if sellable.
        Cmd::Double(true, r) => {
            if let Some(&(i, price, weight)) = c.sale.get(r) {
                cm.sel = Sel::Sale(c.cat.items[i].weapon.clone());
                if weight + c.inv.weight <= c.inv.max_weight && price as f32 <= c.dosh {
                    out.extend(buy(cm, c, &pending));
                } else {
                    runlog::kv("classic_menu_refused", &format!("action=buy weapon={} reason={} how=double_click", c.cat.items[i].weapon, if price as f32 > c.dosh { "too_expensive" } else { "too_heavy" }));
                }
            }
        }
        Cmd::Double(false, i) => {
            cm.sel = Sel::Inv(i);
            out.extend(sell(cm, c, &pending));
        }
        Cmd::Enter => match cm.sel {
            Sel::Sale(_) => out.extend(buy(cm, c, &pending)),
            Sel::Inv(10) => out.extend(vest(cm, c, &pending)),
            Sel::Inv(_) => out.extend(sell(cm, c, &pending)),
            Sel::None => {}
        },
        Cmd::Buy => out.extend(buy(cm, c, &pending)),
        Cmd::Sell => out.extend(sell(cm, c, &pending)),
        Cmd::Clip(row) | Cmd::Fill(row) => {
            let fill = matches!(cmd, Cmd::Fill(_));
            let i = row.or(match cm.sel {
                Sel::Inv(i) => Some(i),
                _ => None,
            });
            match i.and_then(|i| c.rows.get(i)).and_then(Option::as_ref) {
                Some(r) if r.kind == InvKind::Vest => out.extend(vest(cm, c, &pending)),
                Some(r) if r.ammo.is_some() => {
                    let w = r.weapon().unwrap_or_default().to_string();
                    let price = if fill { r.fill_cost } else { r.clip_text().trim_start_matches("£ ").parse().unwrap_or(0) };
                    runlog::kv(
                        "classic_menu_request",
                        &format!("action={} weapon={w} secondary={} price={price} dosh={:.0}", if fill { "fill" } else { "clip" }, r.secondary(), c.dosh),
                    );
                    cm.pending.push(pending(if fill { "fill" } else { "clip" }, &w));
                    out.push(Effect::Request(ShopRequest::Ammo { weapon: w, secondary: r.secondary(), fill }));
                }
                _ => runlog::kv("classic_menu_refused", &format!("action={} reason=no_ammo_row", if fill { "fill" } else { "clip" })),
            }
        }
        Cmd::Vest => out.extend(vest(cm, c, &pending)),
        Cmd::FillAll => {
            // KFTab_BuyMenu.DoFillAllAmmo; the shared plan (numenu.rs)
            // asks each weapon's fill, as the Auto Fill caption adds up.
            let (reqs, total) = crate::game::numenu::fill_all_plan(c.cat, c.inv);
            runlog::kv("classic_menu_request", &format!("action=fill_all requests={} price={total} dosh={:.0}", reqs.len(), c.dosh));
            if total >= 1 {
                cm.pending.push(pending("fill_all", &reqs.len().to_string()));
                out.extend(reqs.into_iter().map(Effect::Request));
                cm.sel = Sel::None;
            }
        }
        Cmd::Perk(p) => {
            runlog::kv("classic_menu_request", &format!("action=perk perk={}", p.name()));
            out.push(Effect::Perk(p));
        }
        Cmd::Scroll(n) => {
            let max = c.sale.len().saturating_sub(SALE_ROWS) as i32;
            cm.top = (cm.top as i32 + n).clamp(0, max) as usize;
            runlog::kv("classic_menu_scroll", &format!("by={n} top={} rows={}", cm.top, c.sale.len()));
        }
        Cmd::PerkTab => runlog::kv("classic_menu_action", "action=perk_tab result=not_built"),
        Cmd::Close => out.push(Effect::Close),
    }
    let _ = c.vet;
    out
}

fn set_filter(cm: &mut ClassicMenu, n: usize) {
    cm.filter = n;
    cm.top = 0;
    // KFBuyMenuFilter.InternalOnClick: the list is rebuilt; a sale
    // selection from another list stays shown in the middle (BuyableToDisplay).
    runlog::kv("classic_menu_filter", &format!("filter={n}"));
}

/// KFTab_BuyMenu.DoBuy (Purchase Weapon, a double click, Enter).
fn buy(cm: &mut ClassicMenu, c: &Ctx, pending: &dyn Fn(&'static str, &str) -> Pending) -> Option<Effect> {
    let Sel::Sale(w) = cm.sel.clone() else {
        runlog::kv("classic_menu_refused", "action=buy reason=no_sale_selection");
        return None;
    };
    let price = c.sale_index(&w).map_or(-1, |r| c.sale[r].1);
    runlog::kv("classic_menu_request", &format!("action=buy weapon={w} price={price} dosh={:.0} weight={}/{}", c.dosh, c.inv.weight, c.inv.max_weight));
    cm.pending.push(pending("buy", &w));
    // SaleSelect.List.SetIndex(-1); TheBuyable = none.
    cm.sel = Sel::None;
    Some(Effect::Request(ShopRequest::Buy(w)))
}

/// KFTab_BuyMenu.DoSell (Sell Weapon, a double click, Enter).
fn sell(cm: &mut ClassicMenu, c: &Ctx, pending: &dyn Fn(&'static str, &str) -> Pending) -> Option<Effect> {
    let row = match cm.sel {
        Sel::Inv(i) => c.rows.get(i).and_then(Option::as_ref),
        _ => None,
    };
    let Some(r) = row.filter(|r| r.sellable) else {
        runlog::kv("classic_menu_refused", &format!("action=sell selected={} reason=not_sellable", c.describe(&cm.sel)));
        return None;
    };
    let w = r.weapon().unwrap_or_default().to_string();
    runlog::kv("classic_menu_request", &format!("action=sell weapon={w} value={} dosh={:.0}", r.sell_value, c.dosh));
    cm.pending.push(pending("sell", &w));
    cm.sel = Sel::None;
    Some(Effect::Request(ShopRequest::Sell(w)))
}

/// The armour row's button (OnBuyVestClick: only when affordable).
fn vest(cm: &mut ClassicMenu, c: &Ctx, pending: &dyn Fn(&'static str, &str) -> Pending) -> Option<Effect> {
    let r = c.rows[10].as_ref()?;
    let affordable = (c.armour > 0.0 && c.dosh >= r.clip_cost) || c.dosh >= r.cost as f32;
    runlog::kv("classic_menu_request", &format!("action=vest armour={:.0} price={} dosh={:.0} affordable={affordable}", c.armour, r.fill_cost, c.dosh));
    if !affordable {
        return None;
    }
    cm.pending.push(pending("vest", "vest"));
    Some(Effect::Vest)
}

/// What the screen reads (menus/mod.rs draws it with the other pages).
#[derive(SystemParam)]
pub struct ClassicDraw<'w> {
    menu: Res<'w, BuyMenu>,
    cm: ResMut<'w, ClassicMenu>,
    cat: Res<'w, ShopCatalogue>,
    inv: Res<'w, ShopInventory>,
    dosh: Res<'w, crate::game::dosh::Dosh>,
    armour: Res<'w, Armour>,
    vet: Res<'w, Veterancy>,
    game: Res<'w, crate::game::waves::WaveGame>,
}

impl ClassicDraw<'_> {
    pub fn showing(&self) -> bool {
        self.menu.open && self.menu.kind == MenuKind::Kf
    }
}

/// The boxes of the screen, in physical pixels (also logged).
pub struct Layout {
    pub boxes: Vec<(&'static str, Rect)>,
}

impl Layout {
    pub fn get(&self, name: &str) -> Rect {
        self.boxes.iter().find(|(n, _)| *n == name).map_or(Rect::default(), |(_, r)| *r)
    }
}

/// A box that ShowPanel moves by pixels (InvSelect / SaleSelect.SetPosition).
fn inset(r: Rect, left: f32, top: f32, dw: f32, dh: f32) -> Rect {
    let min = r.min + Vec2::new(left, top);
    Rect::from_corners(min, min + Vec2::new(r.width() - dw, r.height() - dh))
}

/// Every box, from KFGui.u's components.
pub fn layout(gui: &Gui, s: Rect) -> Layout {
    let page = |n: &str| gui.comp(n).rect(s);
    let mut b: Vec<(&'static str, Rect)> = Vec::new();
    for n in [
        "GUIBuyMenu.HBGLeft", "GUIBuyMenu.HBGCenter", "GUIBuyMenu.HBGRight", "GUIBuyMenu.HBGLL", "GUIBuyMenu.Perk", "GUIBuyMenu.Time", "GUIBuyMenu.Wave",
        "GUIBuyMenu.QS", "GUIBuyMenu.filter", "GUIBuyMenu.PerkTabB", "GUIBuyMenu.StoreTabB", "GUIBuyMenu.Weight", "GUIBuyMenu.WeightIco",
        "GUIBuyMenu.WeightIcoBG", "GUIBuyMenu.WeightB",
    ] {
        b.push((n, page(n)));
    }
    // The tab panel docks under PageTabs (bDockPanels, native): assumed
    // from its bottom to the screen's, as wide as PageTabs (fitted to the
    // screenshot).
    let tabs = page("GUIBuyMenu.PageTabs");
    let panel = Rect::new(tabs.min.x, tabs.max.y, tabs.max.x, s.max.y);
    b.push(("KFTab_BuyMenu", panel));
    for n in [
        "KFTab_BuyMenu.Inv", "KFTab_BuyMenu.SaleB", "KFTab_BuyMenu.MagB", "KFTab_BuyMenu.FillB", "KFTab_BuyMenu.MoneyBack", "KFTab_BuyMenu.Cash",
        "KFTab_BuyMenu.Money", "KFTab_BuyMenu.Item", "KFTab_BuyMenu.SelectedItemL", "KFTab_BuyMenu.ItemInf", "KFTab_BuyMenu.SaleValue",
        "KFTab_BuyMenu.SaleValueBG", "KFTab_BuyMenu.Sale", "KFTab_BuyMenu.PurchaseB", "KFTab_BuyMenu.Info", "KFTab_BuyMenu.IScrollText",
        "KFTab_BuyMenu.AmmoExit", "KFTab_BuyMenu.AutoFill", "KFTab_BuyMenu.Exit",
    ] {
        b.push((n, gui.comp(n).rect(panel)));
    }
    // KFTab_BuyMenu.ShowPanel: the lists, in pixels from their backgrounds.
    let inv_bg = b.iter().find(|(n, _)| *n == "KFTab_BuyMenu.Inv").map(|x| x.1).unwrap_or_default();
    let sale_bg = b.iter().find(|(n, _)| *n == "KFTab_BuyMenu.Sale").map(|x| x.1).unwrap_or_default();
    b.push(("InvSelect", inset(inv_bg, 7.0, 55.0, 15.0, 45.0)));
    let sale = inset(sale_bg, 7.0, 55.0, 15.0, 63.0);
    b.push(("SaleSelect", sale));
    // The sale list's scroll bar (GUIVertScrollBar WinWidth, of the
    // screen's width: a guess, as KFModelSelect's).
    let bw = gui.num("XInterface.GUIVertScrollBar.WinWidth", 0.02) * s.width();
    b.push(("SaleSelect.List", Rect::new(sale.min.x, sale.min.y, sale.max.x - bw, sale.max.y)));
    b.push(("SaleSelect.ScrollBar", Rect::new(sale.max.x - bw, sale.min.y, sale.max.x, sale.max.y)));
    let info = b.iter().find(|(n, _)| *n == "KFTab_BuyMenu.ItemInf").map(|x| x.1).unwrap_or_default();
    // GUIBuyWeaponInfoPanel's parts, inside it (bBoundToParent /
    // bScaleToParent; INameBG and LWeightBG lack the flags but sit there
    // in the screenshot).
    for n in [
        "GUIBuyWeaponInfoPanel.INameBG", "GUIBuyWeaponInfoPanel.IName", "GUIBuyWeaponInfoPanel.IImage", "GUIBuyWeaponInfoPanel.PowerCap",
        "GUIBuyWeaponInfoPanel.RangeCap", "GUIBuyWeaponInfoPanel.SpeedCap", "GUIBuyWeaponInfoPanel.PowerBar", "GUIBuyWeaponInfoPanel.RangeBar",
        "GUIBuyWeaponInfoPanel.SpeedBar", "GUIBuyWeaponInfoPanel.LWeightBG", "GUIBuyWeaponInfoPanel.LWeight",
    ] {
        b.push((n, gui.comp(n).rect(info)));
    }
    Layout { boxes: b }
}

/// UE2 ImageStyle ISTY_Justified (native): the image keeps its aspect,
/// fitted and centred (a guess).
fn justified(r: Rect, size: Vec2) -> Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return r;
    }
    let s = (r.width() / size.x).min(r.height() / size.y);
    Rect::from_center_half_size(r.center(), size * s / 2.0)
}

/// A perk's OnHUDIcon by CorrespondingPerkIndex (7: KFBuyMenuSaleList.NoPerkIcon).
fn perk_icon(i: usize) -> &'static str {
    Perk::ALL.get(i).map_or("KillingFloor2HUD.Perk_Icons.No_Perk_Icon", |p| p.icons().0)
}

/// A KF_SquareButton: Button, button_Highlight when the mouse is on it
/// (white text), black text. Disabled: Button drawn darker (KF's
/// disabled look is native; fitted to the screenshot's grey Sell /
/// Purchase buttons).
fn kf_button(p: &mut Painter, id: &str, r: Rect, caption: &str, enabled: bool, font: &str) {
    let hover = enabled && p.hover(r);
    let (tex, tint, color) = match (enabled, hover) {
        (false, _) => ("KF_InterfaceArt_tex.Menu.Button", [128, 128, 128, 255], BLACK),
        (true, true) => ("KF_InterfaceArt_tex.Menu.button_Highlight", WHITE, WHITE),
        (true, false) => ("KF_InterfaceArt_tex.Menu.Button", WHITE, BLACK),
    };
    p.stretched(p.gui.tex(tex), r, tint, id);
    p.text_in(font, caption, r, Align::Center, true, color, &format!("{id}.Caption"));
    if enabled {
        p.hit(id, r);
    }
}

/// A GUILabel from its component (caption given), its font, colour and
/// alignment, centred vertically.
fn label(p: &mut Painter, gui: &Gui, comp: &str, r: Rect, caption: &str, color: Option<[u8; 4]>) {
    let c = gui.comp(comp);
    let font = named_font(&c.font, p.width());
    let color = color.or(c.color).unwrap_or(GREEN_GREY);
    p.text_in(font, caption, r, c.text_align(), true, color, comp);
}

/// GUIBuyMenu.UpdateHeader's time: "00:" under a minute, else "0M:".
pub fn trader_time(secs: i32) -> String {
    let secs = secs.max(0);
    let (m, s) = (secs / 60, secs % 60);
    let mm = if m < 1 { "00:".to_string() } else { format!("0{m}:") };
    format!("{mm}{s:02}")
}

/// Draws the screen (called by menus/mod.rs while it is open).
pub fn draw(p: &mut Painter, d: &mut ClassicDraw) {
    let gui = p.gui;
    let cm = &mut *d.cm;
    // The menu opened this frame after `input` ran: its state (filter,
    // selection) is set next frame, so nothing is drawn until then.
    if !cm.was_open {
        return;
    }
    let (cat, inv, vet) = (&*d.cat, &*d.inv, &*d.vet);
    let dosh = d.dosh.score;
    let s = p.screen;
    let (sw, sh) = (s.width(), s.height());
    let c = Ctx::new(cat, inv, dosh, d.armour.strength, vet, cm.filter);
    if !c.valid(&cm.sel) {
        cm.sel = Sel::None;
    }
    let lay = layout(gui, s);
    let white = WHITE;
    let tex = |n: &str| gui.tex(n);
    let small = named_font("UT2SmallFont", sw);
    let menu_font = named_font("UT2MenuFont", sw);
    // The lists' text: their font is not set in DrawInvItem; assumed
    // UT2SmallFont (fitted to the screenshot).
    let list_font = small;

    // PageBackground: WhiteSquareTexture, ImageColor 20,20,20.
    p.fill(s, [20, 20, 20, 255], "GUIBuyMenu.PageBackground");
    for n in ["GUIBuyMenu.HBGLeft", "GUIBuyMenu.HBGCenter", "GUIBuyMenu.HBGRight"] {
        p.stretched(tex("KF_InterfaceArt_tex.Menu.Thin_border"), lay.get(n), white, n);
    }
    label(p, gui, "GUIBuyMenu.HBGLL", lay.get("GUIBuyMenu.HBGLL"), "Quick Perk Select", None);

    // KFQuickPerkSelect.MyOnDraw: the current perk's box and icon, then
    // the other perks' boxes (PB0-5) and icons (PSI0-5), square
    // ((ClipY / ClipX) x BoxSize of the width).
    let qs = lay.get("GUIBuyMenu.QS");
    let cur = vet.selected.map_or(0, |p| p.index());
    let cur_box = Rect::new(qs.min.x, qs.min.y, qs.min.x + qs.height(), qs.min.y + qs.height());
    if let Some(t) = tex("KF_InterfaceArt_tex.Menu.Perk_box") {
        // DrawTileScaled by WinHeight x ClipY / USize.
        let size = gui.textures[t].size;
        let k = qs.height() / size.x.max(1.0);
        p.tile(Some(t), Rect::new(qs.min.x, qs.min.y, qs.min.x + size.x * k, qs.min.y + size.y * k), white, "KFQuickPerkSelect.CurPerkBack");
    }
    p.tile(tex(perk_icon(cur)), cur_box, white, "KFQuickPerkSelect.CurPerk");
    let qbox = gui.num("KFGui.KFQuickPerkSelect.BoxSizeX", 0.04) * sh;
    let qicon = gui.num("KFGui.KFQuickPerkSelect.BoxSizeY", 0.04) * sh;
    let alpha = if vet.changed_this_wave { 64 } else { 255 };
    for (j, perk) in Perk::ALL.iter().filter(|p| p.index() != cur).enumerate().take(6) {
        let pb = gui.comp(&format!("KFQuickPerkSelect.PB{j}")).rect(s);
        let pbr = Rect::new(pb.min.x, pb.min.y, pb.min.x + qbox, pb.min.y + pb.height());
        p.tile(tex("KF_InterfaceArt_tex.Menu.Perk_box_unselected"), pbr, white, &format!("KFQuickPerkSelect.PB{j}"));
        let pi = gui.comp(&format!("KFQuickPerkSelect.PSI{j}")).rect(s);
        let pir = Rect::new(pi.min.x, pi.min.y, pi.min.x + qicon, pi.min.y + pi.height());
        p.tile(tex(perk.icons().0), pir, [255, 255, 255, alpha], &format!("KFQuickPerkSelect.PSI{j}"));
        if !vet.changed_this_wave {
            p.hit(&format!("kf.perk:{}", perk.index()), pir);
        }
    }
    kf_button(p, "kf.perk_tab", lay.get("GUIBuyMenu.PerkTabB"), "Perk", true, small);
    kf_button(p, "kf.store_tab", lay.get("GUIBuyMenu.StoreTabB"), "Store", true, small);

    // UpdateHeader: time (red under 10 s), wave, current perk.
    let g = &d.game;
    let time_color = if g.countdown < 10 { [255, 0, 0, 255] } else { GREEN_GREY };
    label(p, gui, "GUIBuyMenu.Time", lay.get("GUIBuyMenu.Time"), &format!("Trader Closes in {}", trader_time(g.countdown)), Some(time_color));
    label(p, gui, "GUIBuyMenu.Wave", lay.get("GUIBuyMenu.Wave"), &format!("Wave: {}/{}", g.wave_num + 1, g.final_wave), None);
    let perk_text = match vet.selected {
        Some(p) => format!("Current Perk: {} Lv{}", p.name(), vet.vet.level),
        None => "Current Perk: No Active Perk!".to_string(),
    };
    label(p, gui, "GUIBuyMenu.Perk", lay.get("GUIBuyMenu.Perk"), &perk_text, None);

    // KFBuyMenuFilter: 9 icons spread over its width (RealignIcons;
    // bBoundToParent: placed inside the filter's box, sized by the
    // screen: fitted to the screenshot); others drawn at alpha 95.
    let f = lay.get("GUIBuyMenu.filter");
    let fsize = gui.num("KFGui.KFBuyMenuFilter.BoxSizeX", 0.04) * sh;
    let iw = gui.num("KFGui.KFBuyMenuFilter.BoxSizeY", 0.04) * sh / sw;
    let pad = (1.0 - iw * 9.0) / 9.0 / 2.0;
    let psi = gui.comp("KFBuyMenuFilter.PSI0");
    for i in 0..FILTERS {
        let x = f.min.x + (pad + (pad + iw + pad) * i as f32) * f.width();
        // PB WinTop 0.05, PSI WinTop 0.052 (of the filter's height).
        let back = Rect::new(x, f.min.y + 0.05 * f.height(), x + fsize, f.min.y + 0.05 * f.height() + fsize);
        p.tile(tex("KF_InterfaceArt_tex.Menu.Perk_box_unselected"), back, white, &format!("KFBuyMenuFilter.PB{i}"));
        let y = f.min.y + psi.win[1] * f.height();
        let icon = Rect::new(x, y, x + fsize, y + fsize);
        let name = match i {
            7 => "KillingFloor2HUD.Perk_Icons.No_Perk_Icon",
            8 => "KillingFloor2HUD.Perk_Icons.Favorite_Perk_Icon",
            _ => perk_icon(i),
        };
        p.tile(tex(name), icon, [255, 255, 255, if i == cm.filter { 255 } else { 95 }], &format!("KFBuyMenuFilter.PSI{i}"));
        p.hit(&format!("kf.filter:{i}"), icon);
    }

    // ---- The store panel.
    for (n, t) in [
        ("KFTab_BuyMenu.Inv", "KF_InterfaceArt_tex.Menu.Thick_border_Transparent"),
        ("KFTab_BuyMenu.Sale", "KF_InterfaceArt_tex.Menu.Thick_border_Transparent"),
        ("KFTab_BuyMenu.MoneyBack", "KF_InterfaceArt_tex.Menu.Thin_border_Transparent"),
        ("KFTab_BuyMenu.Item", "KF_InterfaceArt_tex.Menu.Med_border_Transparent"),
        ("KFTab_BuyMenu.Info", "KF_InterfaceArt_tex.Menu.Thin_border"),
        ("KFTab_BuyMenu.AmmoExit", "KF_InterfaceArt_tex.Menu.Thin_border"),
        ("KFTab_BuyMenu.MagB", "KF_InterfaceArt_tex.Menu.Innerborder_transparent"),
        ("KFTab_BuyMenu.FillB", "KF_InterfaceArt_tex.Menu.Innerborder_transparent"),
    ] {
        p.stretched(tex(t), lay.get(n), white, n);
    }
    label(p, gui, "KFTab_BuyMenu.MagL", lay.get("KFTab_BuyMenu.MagB"), "1 Mag", None);
    label(p, gui, "KFTab_BuyMenu.FillL", lay.get("KFTab_BuyMenu.FillB"), "Fill", None);
    // UpdateBuySellButtons.
    let sel_inv = match cm.sel {
        Sel::Inv(i) => c.rows.get(i).and_then(Option::as_ref),
        _ => None,
    };
    let sel_sale = match &cm.sel {
        Sel::Sale(w) => c.sale_index(w).map(|r| c.sale[r]),
        _ => None,
    };
    kf_button(p, "kf.sell", lay.get("KFTab_BuyMenu.SaleB"), "Sell Weapon", sel_inv.is_some_and(|r| r.sellable), menu_font);
    kf_button(p, "kf.purchase", lay.get("KFTab_BuyMenu.PurchaseB"), "Purchase Weapon", sel_sale.is_some_and(|(_, price, _)| price as f32 <= dosh), menu_font);

    // Money: the banknote (ImageStyle Scaled), "£" + int(Score).
    p.tile(tex("PatchTex.Statics.BanknoteSkin"), lay.get("KFTab_BuyMenu.Cash"), white, "KFTab_BuyMenu.Cash");
    label(p, gui, "KFTab_BuyMenu.Money", lay.get("KFTab_BuyMenu.Money"), &format!("£{}", dosh as i32), None);
    let sil = lay.get("KFTab_BuyMenu.SelectedItemL");
    label(p, gui, "KFTab_BuyMenu.SelectedItemL", sil, "Selected Item Info", None);

    // GUIBuyWeaponInfoPanel.Display.
    struct Shown {
        name: String,
        image: String,
        stats: Option<(f32, f32, f32, f32)>,
    }
    let shown = match (&cm.sel, sel_inv, sel_sale) {
        (Sel::Inv(_), Some(r), _) => Some(match r.item.map(|i| &cat.items[i]) {
            Some(it) => Shown { name: r.name.clone(), image: it.info.image.clone(), stats: Some((it.info.power, it.info.range, it.info.speed, it.weight)) },
            None => Shown { name: r.name.clone(), image: VEST_IMAGE.into(), stats: None },
        }),
        (Sel::Sale(_), _, Some((i, _, weight))) => {
            let it = &cat.items[i];
            Some(Shown { name: it.name.clone(), image: it.info.image.clone(), stats: Some((it.info.power, it.info.range, it.info.speed, weight)) })
        }
        _ => None,
    };
    if let Some(sh_) = &shown {
        let nbg = lay.get("GUIBuyWeaponInfoPanel.INameBG");
        p.stretched(tex("KF_InterfaceArt_tex.Menu.Innerborder_transparent"), nbg, white, "GUIBuyWeaponInfoPanel.INameBG");
        label(p, gui, "GUIBuyWeaponInfoPanel.IName", lay.get("GUIBuyWeaponInfoPanel.IName"), &sh_.name, None);
        if let Some(t) = tex(&sh_.image) {
            let r = justified(lay.get("GUIBuyWeaponInfoPanel.IImage"), gui.textures[t].size);
            p.tile(Some(t), r, white, "GUIBuyWeaponInfoPanel.IImage");
        }
        if let Some((power, range, speed, weight)) = sh_.stats {
            let high = gui.num("KFGui.GUIWeaponBar.High", 120.0);
            for (cap, bar, text, v) in [
                ("GUIBuyWeaponInfoPanel.PowerCap", "GUIBuyWeaponInfoPanel.PowerBar", "Power:", power),
                ("GUIBuyWeaponInfoPanel.RangeCap", "GUIBuyWeaponInfoPanel.RangeBar", "Range:", range),
                ("GUIBuyWeaponInfoPanel.SpeedCap", "GUIBuyWeaponInfoPanel.SpeedBar", "Speed:", speed),
            ] {
                label(p, gui, cap, lay.get(cap), text, None);
                // GUIWeaponBar (GUIProgressBar, native): BarBack over the
                // box, BarTop filled to (value + 20) / High inside
                // BorderSize 3 (fitted to the screenshot's bars).
                let r = lay.get(bar);
                p.stretched(tex("KF_InterfaceArt_tex.Menu.Innerborder_transparent"), r, white, bar);
                let frac = ((v + 20.0) / high).clamp(0.0, 1.0);
                let inner = Rect::new(r.min.x + 3.0, r.min.y + 3.0, r.max.x - 3.0, r.max.y - 3.0);
                let fill = Rect::new(inner.min.x, inner.min.y, inner.min.x + inner.width() * frac, inner.max.y);
                p.stretched(tex("InterfaceArt_tex.Menu.progress_bar"), fill, white, &format!("{bar}.Fill"));
            }
            p.stretched(tex("KF_InterfaceArt_tex.Menu.Innerborder_transparent"), lay.get("GUIBuyWeaponInfoPanel.LWeightBG"), white, "GUIBuyWeaponInfoPanel.LWeightBG");
            label(p, gui, "GUIBuyWeaponInfoPanel.LWeight", lay.get("GUIBuyWeaponInfoPanel.LWeight"), &format!("Weight: {} blocks", weight as i32), None);
        }
    }
    // UpdatePanel: the sell value of a sellable owned item.
    if let Some(r) = sel_inv.filter(|r| r.sellable) {
        p.stretched(tex("KF_InterfaceArt_tex.Menu.Innerborder_transparent"), lay.get("KFTab_BuyMenu.SaleValueBG"), white, "KFTab_BuyMenu.SaleValueBG");
        label(p, gui, "KFTab_BuyMenu.SaleValue", lay.get("KFTab_BuyMenu.SaleValue"), &format!("Sell Value: £{}", r.sell_value), None);
    }

    // SetInfoText (style TraderNoBackground: UT2MenuFont at FontScale
    // medium, 175,176,158).
    let info_text = match (&cm.sel, sel_inv, sel_sale) {
        (Sel::Sale(_), _, Some((i, price, weight))) => {
            if price as f32 > dosh {
                INFO_EXPENSIVE.to_string()
            } else if weight + inv.weight > inv.max_weight {
                INFO_HEAVY.replace("%1", &(weight as i32).to_string()).replace("%2", &((inv.max_weight - inv.weight) as i32).to_string())
            } else {
                cat.items[i].info.description.clone()
            }
        }
        (Sel::Inv(_), Some(r), _) => r.item.map_or(VEST_DESCRIPTION.to_string(), |i| cat.items[i].info.description.clone()),
        _ => INFO_WELCOME.to_string(),
    };
    p.scroll_text(menu_font, &info_text, lay.get("KFTab_BuyMenu.IScrollText"), GREEN_GREY, "KFTab_BuyMenu.IScrollText");

    // Auto fill (caption AutoFillString @ "(£" @ int(cost) $ ")"), exit.
    let (_, auto) = crate::game::numenu::fill_all_plan(cat, inv);
    kf_button(p, "kf.autofill", lay.get("KFTab_BuyMenu.AutoFill"), &format!("Auto Fill Ammo (£ {auto})"), auto >= 1, menu_font);
    kf_button(p, "kf.exit", lay.get("KFTab_BuyMenu.Exit"), "Exit Trader Menu", true, menu_font);

    // ---- KFBuyMenuInvList.DrawInvItem, 11 rows of box height / 11 - 1.
    let il = lay.get("InvSelect");
    let ih = il.height() / INV_ROWS as f32 - 1.0;
    let iw_ = il.width();
    let n = |k: &str, d: f32| gui.num(&format!("KFGui.KFBuyMenuInvList.{k}"), d);
    let (item_scale, ammo_scale, clip_scale) = (n("ItemBGWidthScale", 0.51), n("AmmoBGWidthScale", 0.19), n("ClipButtonWidthScale", 0.45));
    let (ammo_h, button_h) = (n("AmmoBGHeightScale", 0.5), n("ButtonBGHeightScale", 0.5));
    let (eq_w, eq_h, eq_x, eq_y) = (n("EquipmentBGWidthScale", 0.35), n("EquipmentBGHeightScale", 0.6), n("EquipmentBGXOffset", 3.0), n("EquipmentBGYOffset", 6.0));
    let (bar_y, ammo_gap, name_gap, button_gap) = (n("ItemBGYOffset", 6.0), n("AmmoSpacing", 1.0), n("ItemNameSpacing", 10.0), n("ButtonSpacing", 3.0));
    for (k, row) in c.rows.iter().enumerate() {
        let (x, y) = (il.min.x, il.min.y + k as f32 * ih);
        let what = format!("InvList[{k}]");
        let Some(r) = row else {
            if k >= MAX_WEAPONS {
                let eb = Rect::new(x + eq_x, y + ih - eq_y - eq_h * ih, x + eq_x + eq_w * iw_, y + ih - eq_y);
                p.stretched(tex("KF_InterfaceArt_tex.Menu.Innerborder_transparent"), eb, white, &format!("{what}.Equipment"));
                p.text_in(list_font, "Equipment", eb, Align::Center, true, GREEN_GREY, &format!("{what}.Equipment"));
            }
            continue;
        };
        let selected = cm.sel == Sel::Inv(k);
        let item_w = iw_ * item_scale - ih;
        let ammo_w = iw_ * ammo_scale;
        let (clip_w, fill_w) = if r.kind == InvKind::Vest {
            (0.0, (1.0 - item_scale - ammo_scale) * iw_)
        } else {
            let all = (1.0 - item_scale - ammo_scale) * iw_ - button_gap;
            (all * clip_scale, all - all * clip_scale)
        };
        let (left, right) = if selected {
            ("KF_InterfaceArt_tex.Menu.Item_box_box_Highlighted", "KF_InterfaceArt_tex.Menu.Item_box_bar_Highlighted")
        } else {
            ("KF_InterfaceArt_tex.Menu.Item_box_box", "KF_InterfaceArt_tex.Menu.Item_box_bar")
        };
        p.stretched(tex(left), Rect::new(x, y, x + ih, y + ih), white, &format!("{what}.Box"));
        p.tile(tex(perk_icon(r.perk_index)), Rect::new(x + 4.0, y + 4.0, x + ih - 4.0, y + ih - 4.0), white, &format!("{what}.Perk"));
        let bar = Rect::new(x + ih, y + bar_y, x + ih + item_w, y + ih - bar_y);
        p.stretched(tex(right), bar, white, &format!("{what}.Bar"));
        let name_box = Rect::new(x, y, x + iw_ * item_scale, y + ih);
        let hover_name = p.hover(name_box);
        p.text_in(list_font, &r.name, Rect::new(bar.min.x + name_gap, y, bar.max.x, y + ih), Align::Left, true, if hover_name { WHITE } else { BLACK }, &format!("{what}.Name"));
        p.hit(&format!("kf.inv:{k}"), name_box);
        if r.melee {
            continue;
        }
        let mut tx = x + ih + item_w + ammo_gap;
        let ab = Rect::new(tx, y + (ih - ammo_h * ih) / 2.0, tx + ammo_w, y + (ih + ammo_h * ih) / 2.0);
        p.stretched(tex("KF_InterfaceArt_tex.Menu.Innerborder_transparent"), ab, white, &format!("{what}.Ammo"));
        p.text_in(list_font, &r.ammo_text(), ab, Align::Center, true, GREEN_GREY, &format!("{what}.Ammo"));
        tx += ammo_w + ammo_gap;
        let by = (y + (ih - button_h * ih) / 2.0, y + (ih + button_h * ih) / 2.0);
        let disabled = r.buttons_disabled(dosh);
        let button = |p: &mut Painter, id: String, x0: f32, w: f32, text: String| {
            let b = Rect::new(x0, by.0, x0 + w, by.1);
            let hover = !disabled && p.hover(Rect::new(x0, y, x0 + w, y + ih));
            let (t, color) = if disabled {
                ("KF_InterfaceArt_tex.Menu.button_Disabled", BLACK)
            } else if hover {
                ("KF_InterfaceArt_tex.Menu.button_Highlight", WHITE)
            } else {
                ("KF_InterfaceArt_tex.Menu.Button", BLACK)
            };
            p.stretched(tex(t), b, white, &id);
            p.text_in(list_font, &text, b, Align::Center, true, color, &format!("{id}.Caption"));
            // KF takes the click over the whole row height.
            p.hit(&id, Rect::new(x0, y, x0 + w, y + ih));
        };
        if r.kind == InvKind::Vest {
            button(p, "kf.vest".into(), tx, fill_w, r.fill_text());
        } else {
            button(p, format!("kf.clip:{k}"), tx, clip_w, r.clip_text());
            tx += clip_w + button_gap;
            button(p, format!("kf.fill:{k}"), tx, fill_w, r.fill_text());
        }
    }

    // ---- KFBuyMenuSaleList.DrawInvItem, 10 rows of box height / 10 - 1.
    let sl = lay.get("SaleSelect.List");
    let sbox = lay.get("SaleSelect");
    let rh = sbox.height() / SALE_ROWS as f32 - 1.0;
    let max_top = c.sale.len().saturating_sub(SALE_ROWS);
    cm.top = cm.top.min(max_top);
    for k in 0..SALE_ROWS {
        let Some(&(i, price, weight)) = c.sale.get(cm.top + k) else { break };
        let it = &cat.items[i];
        let (x, y, w) = (sl.min.x, sl.min.y + k as f32 * rh, sl.width());
        let what = format!("SaleList[{}]", cm.top + k);
        let can_buy = price as f32 <= dosh && weight + inv.weight <= inv.max_weight;
        let selected = matches!(&cm.sel, Sel::Sale(s) if s.eq_ignore_ascii_case(&it.weapon));
        let (left, right) = if !can_buy {
            ("KF_InterfaceArt_tex.Menu.Item_box_box_Disabled", "KF_InterfaceArt_tex.Menu.Item_box_bar_Disabled")
        } else if selected {
            ("KF_InterfaceArt_tex.Menu.Item_box_box_Highlighted", "KF_InterfaceArt_tex.Menu.Item_box_bar_Highlighted")
        } else {
            ("KF_InterfaceArt_tex.Menu.Item_box_box", "KF_InterfaceArt_tex.Menu.Item_box_bar")
        };
        p.stretched(tex(left), Rect::new(x, y, x + rh, y + rh), white, &format!("{what}.Box"));
        let tx = x + rh - 1.0;
        let bar = Rect::new(tx, y + 6.0, tx + w - rh, y + rh - 6.0);
        p.stretched(tex(right), bar, white, &format!("{what}.Bar"));
        p.tile(tex(perk_icon(it.info.perk_index)), Rect::new(x + 4.0, y + 4.0, x + rh - 4.0, y + rh - 4.0), white, &format!("{what}.Perk"));
        let row = Rect::new(x, y, x + w, y + rh);
        let color = if p.hover(row) { WHITE } else { BLACK };
        p.text_in(list_font, &it.name, Rect::new(tx + 0.2 * rh, bar.min.y, bar.max.x, bar.max.y), Align::Left, true, color, &format!("{what}.Name"));
        p.text_in(list_font, &format!("£ {price}"), Rect::new(tx, bar.min.y, x - 1.0 + w - 0.2 * rh, bar.max.y), Align::Right, true, color, &format!("{what}.Price"));
        p.hit(&format!("kf.sale:{}", cm.top + k), row);
    }
    // The scroll bar (look guessed as KFModelSelect's: the scroll zone
    // 32,33,35, arrow buttons and grip KF_InterfaceArt_tex.Menu.scrollbar).
    let b = lay.get("SaleSelect.ScrollBar");
    let bw = b.width();
    p.fill(b, [32, 33, 35, 255], "SaleSelect.ScrollZone");
    let sb = tex("KF_InterfaceArt_tex.Menu.scrollbar");
    let up = Rect::new(b.min.x, b.min.y, b.max.x, b.min.y + bw);
    let down = Rect::new(b.min.x, b.max.y - bw, b.max.x, b.max.y);
    p.stretched(sb, up, white, "SaleSelect.ScrollUp");
    p.stretched(sb, down, white, "SaleSelect.ScrollDown");
    p.hit("kf.scroll:-1", up);
    p.hit("kf.scroll:1", down);
    let zone = Rect::new(b.min.x, up.max.y, b.max.x, down.min.y);
    let gh = (zone.height() * (SALE_ROWS as f32 / c.sale.len().max(1) as f32).min(1.0)).max(12.0);
    let gy = zone.min.y + (zone.height() - gh) * (cm.top as f32 / max_top.max(1) as f32).min(1.0);
    p.stretched(sb, Rect::new(b.min.x, gy, b.max.x, gy + gh), white, "SaleSelect.Grip");

    // ---- The footer: KFWeightBar.MyOnDraw (screen fractions; boxes
    // BoxSize x ClipX square).
    p.stretched(tex("KF_InterfaceArt_tex.Menu.Thin_border"), lay.get("GUIBuyMenu.Weight"), white, "GUIBuyMenu.Weight");
    p.tile(tex("KF_InterfaceArt_tex.Menu.Perk_box_unselected"), lay.get("GUIBuyMenu.WeightIcoBG"), white, "GUIBuyMenu.WeightIcoBG");
    p.tile(tex("KillingFloorHUD.HUD.Hud_Weight"), lay.get("GUIBuyMenu.WeightIco"), white, "GUIBuyMenu.WeightIco");
    let wb = gui.comp("GUIBuyMenu.WeightB");
    let (bx, by_, spacer) = (gui.num("KFGui.KFWeightBar.BoxSizeX", 0.015), gui.num("KFGui.KFWeightBar.BoxSizeY", 0.015), gui.num("KFGui.KFWeightBar.Spacer", 0.0012));
    let max_boxes = inv.max_weight as i32;
    let cur_boxes = inv.weight as i32;
    // NewBoxes: the selected sale item's weight (SaleChange).
    let new_boxes = sel_sale.map_or(0, |(_, _, w)| w as i32);
    let cur_y = wb.win[1] + wb.win[3] / 2.5;
    for i in 0..max_boxes {
        let x0 = (wb.win[0] + bx * i as f32) * sw;
        p.stretched(tex("KF_InterfaceArt_tex.Menu.Perk_box_unselected"), Rect::new(x0, cur_y * sh, x0 + bx * sw, cur_y * sh + by_ * sw), white, &format!("KFWeightBar.Back[{i}]"));
    }
    let enc = format!("Encumbrance Level: {cur_boxes}/{max_boxes}");
    let th = p.line_height(small) / sh;
    let ty = ((cur_y - th) - ((cur_y - th) - wb.win[1]) / 2.0) * sh;
    p.text(small, &enc, Vec2::new((wb.win[0] * sw).round(), ty.round()), GREEN_GREY, "KFWeightBar.Text");
    let fy = cur_y + spacer * 1.5;
    let fs = (bx - spacer * 2.0) * sw;
    let mut fx = wb.win[0] + spacer;
    let mut boxes = |p: &mut Painter, count: i32, tint: [u8; 4], what: &str| {
        for i in 0..count {
            let x0 = fx * sw;
            p.stretched(tex("KF_InterfaceArt_tex.Menu.Progress"), Rect::new(x0, fy * sh, x0 + fs, fy * sh + (by_ - spacer * 2.0) * sw), tint, &format!("{what}[{i}]"));
            fx += bx;
        }
    };
    // The current weight is drawn in CurrentColor (DrawColor left from the text).
    boxes(p, cur_boxes.min(max_boxes), GREEN_GREY, "KFWeightBar.Cur");
    if new_boxes != 0 {
        if cur_boxes + new_boxes <= max_boxes {
            boxes(p, new_boxes, [255, 128, 0, 255], "KFWeightBar.New");
        } else {
            boxes(p, new_boxes.min(max_boxes - cur_boxes).max(0), [255, 0, 0, 255], "KFWeightBar.Warn");
        }
    }

    cm.hits = p.hits.clone();
    // The layout, once per window size.
    if cm.logged_size != Vec2::new(sw, sh) {
        cm.logged_size = Vec2::new(sw, sh);
        let f = |r: Rect| format!("({:.0},{:.0},{:.0},{:.0})", r.min.x, r.min.y, r.width(), r.height());
        runlog::kv(
            "classic_menu_layout",
            &format!(
                "window={sw:.0}x{sh:.0} fonts=small:{small},menu:{menu_font} inv_row_h={ih:.1} sale_row_h={rh:.1} sale_rows={} inv_rows={} hits={} boxes=[{}]",
                c.sale.len(),
                c.rows.iter().filter(|r| r.is_some()).count(),
                p.hits.len(),
                lay.boxes.iter().map(|(n, r)| format!("{n}:{}", f(*r))).collect::<Vec<_>>().join(" ")
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::buy_menu::{OwnedWeapon, ShopInfo, ShopItem, ShopStats};

    fn item(weapon: &str, melee: bool) -> ShopItem {
        ShopItem {
            weapon: weapon.into(),
            pickup: format!("{weapon}Pickup"),
            name: weapon.into(),
            cost: 500,
            ammo_cost: 10,
            weight: 2.0,
            buy_clip_size: 0,
            never_throw: false,
            stats: ShopStats { melee, ..Default::default() },
            info: ShopInfo { short_name: weapon.trim_start_matches("KFMod.").into(), ..Default::default() },
        }
    }

    fn owned(weapon: &str, ammo: Option<(u32, u32, u32)>) -> OwnedWeapon {
        OwnedWeapon { weapon: weapon.into(), name: weapon.into(), sell_value: 375, sellable: true, ammo, alt_ammo: None, ammo_scale: 1.0, mag_mod: 1.0 }
    }

    #[test]
    fn inventory_rows_follow_kf_slots() {
        let cat = ShopCatalogue {
            items: vec![item("KFMod.Knife", true), item("KFMod.Single", false), item("KFMod.Frag", false), item("KFMod.Welder", true), item("KFMod.Shotgun", false)],
            lists: vec![],
        };
        let inv = ShopInventory {
            owned: vec![
                owned("KFMod.Knife", None),
                owned("KFMod.Single", Some((57, 240, 15))),
                owned("KFMod.Frag", Some((4, 5, 1))),
                owned("KFMod.Welder", None),
                owned("KFMod.Shotgun", Some((80, 80, 8))),
            ],
            weight: 8.0,
            max_weight: 15.0,
        };
        let rows = inv_rows(&cat, &inv, 0.0, 1.0);
        assert_eq!(rows.len(), INV_ROWS);
        let names: Vec<Option<&str>> = rows.iter().map(|r| r.as_ref().map(|r| r.name.as_str())).collect();
        assert_eq!(names[0], Some("Single"));
        assert_eq!(names[1], Some("Shotgun"));
        assert_eq!(names[2..8], [None; 6]);
        assert_eq!(names[8], Some("Knife"));
        assert_eq!(names[9], Some("Frag"));
        assert_eq!(names[10], Some(VEST_NAME));
        // Knife and grenades are never sold; the armour costs 300.
        assert!(!rows[8].as_ref().unwrap().sellable && !rows[9].as_ref().unwrap().sellable);
        let vest = rows[10].as_ref().unwrap();
        assert_eq!((vest.ammo_text(), vest.fill_text()), ("0%".to_string(), "Buy : £ 300".to_string()));
        // 9mm: clip int(10), fill int(183 x 10 / 15) = 122 (the screenshot's £ 10 / £ 122).
        let single = rows[0].as_ref().unwrap();
        assert_eq!((single.ammo_text(), single.clip_text(), single.fill_text()), ("57/240".into(), "£ 10".into(), "£ 122".into()));
        // A full weapon: clip £ 0, buttons disabled.
        let shotgun = rows[1].as_ref().unwrap();
        assert_eq!(shotgun.clip_text(), "£ 0");
        assert!(shotgun.buttons_disabled(1000.0));
    }

    #[test]
    fn armour_texts() {
        let cat = ShopCatalogue::default();
        let inv = ShopInventory::default();
        let half = inv_rows(&cat, &inv, 50.0, 1.0)[10].clone().unwrap();
        assert_eq!(half.fill_text(), "Repair : £ 150");
        let full = inv_rows(&cat, &inv, 100.0, 1.0)[10].clone().unwrap();
        assert_eq!(full.fill_text(), "Purchased");
        assert!(full.buttons_disabled(5000.0));
    }

    #[test]
    fn trader_time_as_kf_writes_it() {
        assert_eq!(trader_time(51), "00:51");
        assert_eq!(trader_time(65), "01:05");
        assert_eq!(trader_time(9), "00:09");
    }
}
