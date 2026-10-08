//! NuMenu: our own trader menu (not a copy of KF's GUIBuyMenu), the
//! default (`--trader-menu nu`). Opening and closing, the input freeze and
//! every purchase rule are shared with the KF-style list (buy_menu.rs): this
//! file only draws the screen and turns keys, clicks and test actions into
//! the same `ShopRequest` / `BuyVest` / `PerkRequest` messages. See
//! DESIGN.md, "NuMenu".

use bevy::ecs::system::SystemParam;
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use crate::engine::runlog;
use crate::game::buy_menu::{BuyMenu, HALF_PRICE_DUALS, MenuKind, OwnedWeapon, ShopCatalogue, ShopInventory, ShopRequest, ammo_prices, shop_price, single_hidden};
use crate::game::menus::gui::{Align, Painter};
use crate::game::perks::{Perk, Vet, Veterancy};
use crate::player::armour::{Armour, BuyVest};

/// The grenades (bought as the Frag's ammo).
const FRAG: &str = "KFMod.Frag";

/// How a shop row looks: what would happen if you bought it now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowState {
    CanBuy,
    Owned,
    TooExpensive,
    TooHeavy,
}

impl RowState {
    fn word(self) -> &'static str {
        match self {
            RowState::CanBuy => "can_buy",
            RowState::Owned => "owned",
            RowState::TooExpensive => "too_expensive",
            RowState::TooHeavy => "too_heavy",
        }
    }
}

/// One weapon in the shop list.
#[derive(Clone, Debug)]
pub struct ShopRow {
    /// Index into the catalogue's items.
    pub item: usize,
    pub group: usize,
    /// As shown (int, KF's rule) and as charged (float).
    pub price: i32,
    pub price_exact: f32,
    pub weight: f32,
    /// The perk's GetCostScaling for it (1: no discount).
    pub scaling: f32,
    pub state: RowState,
}

/// A perk group of the shop list.
#[derive(Clone, Debug)]
pub struct Group {
    pub label: &'static str,
    pub perk: Option<Perk>,
    /// Your perk's group (listed first).
    pub yours: bool,
    /// The largest discount in the group (0.3 = 30% off).
    pub discount: f32,
    /// Its first row (`Model::rows`).
    pub first: usize,
}

/// A line of the shop list: a group header or a weapon row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Line {
    Header(usize),
    Row(usize),
}

/// What the shop shows now (rebuilt every frame from the shared state).
#[derive(Default, Debug)]
pub struct Model {
    pub groups: Vec<Group>,
    pub rows: Vec<ShopRow>,
    pub lines: Vec<Line>,
}

/// The shop list: every weapon for sale (not bKFNeverThrow; a single is
/// hidden while its duals are owned, as KF), grouped by the catalogue's
/// perk lists, your perk's group first.
pub fn model(cat: &ShopCatalogue, inv: &ShopInventory, vet: &Vet, dosh: f32) -> Model {
    let yours = vet.perk.map(|p| p.index()).filter(|&y| y < cat.lists.len());
    let mut order: Vec<usize> = (0..cat.lists.len()).collect();
    if let Some(y) = yours {
        order.retain(|&i| i != y);
        order.insert(0, y);
    }
    let mut m = Model::default();
    for li in order {
        let (label, list) = &cat.lists[li];
        let g = m.groups.len();
        let first = m.rows.len();
        let mut discount = 0.0f32;
        for &i in list {
            let it = &cat.items[i];
            if it.never_throw || single_hidden(inv, &it.weapon) || m.rows.iter().any(|r| r.item == i) {
                continue;
            }
            let (price, weight) = shop_price(it, inv, vet);
            let scaling = vet.cost_scaling(&it.pickup);
            let half = HALF_PRICE_DUALS.iter().any(|(d, s)| d.eq_ignore_ascii_case(&it.weapon) && inv.owns(s));
            let price_exact = it.cost as f32 * scaling / if half { 2.0 } else { 1.0 };
            discount = discount.max(1.0 - scaling);
            // The same checks as ServerBuyWeapon (weapon.rs), weight first.
            let state = if inv.owns(&it.weapon) {
                RowState::Owned
            } else if weight > 0.0 && inv.weight + weight > inv.max_weight {
                RowState::TooHeavy
            } else if dosh < price_exact {
                RowState::TooExpensive
            } else {
                RowState::CanBuy
            };
            m.rows.push(ShopRow { item: i, group: g, price, price_exact, weight, scaling, state });
        }
        let len = m.rows.len() - first;
        if len == 0 {
            continue;
        }
        m.lines.push(Line::Header(g));
        m.lines.extend((first..first + len).map(Line::Row));
        m.groups.push(Group { label, perk: Perk::ALL.get(li).copied(), yours: Some(li) == yours, discount, first });
    }
    m
}

/// A weapon's ammo as the menu shows it: (total, max, one-magazine price,
/// fill price), KFBuyMenuInvList's prices (`ammo_prices`).
pub fn ammo_info(cat: &ShopCatalogue, o: &OwnedWeapon) -> Option<(u32, u32, i32, i32)> {
    let (total, max, cap) = o.ammo?;
    let item = cat.item(&o.weapon)?;
    let (clip, fill) = ammo_prices(item, total, max, cap, o.ammo_scale, o.mag_mod);
    Some((total, max, clip, fill))
}

/// The second ammo's fill price (M4 203 grenades): each costs AmmoCost x
/// GetAmmoCostScaling (weapon.rs `shop_requests`, UsedMagCapacity 1).
pub fn alt_fill(cat: &ShopCatalogue, o: &OwnedWeapon) -> Option<i32> {
    let (cur, max) = o.alt_ammo?;
    let item = cat.item(&o.weapon)?;
    (max > cur).then(|| ((max - cur) as f32 * (item.ammo_cost as f32 * o.ammo_scale)) as i32)
}

/// REFILL ALL: (requests, total price) for every weapon's missing ammo,
/// grenades and second ammo included.
pub fn fill_all_plan(cat: &ShopCatalogue, inv: &ShopInventory) -> (Vec<ShopRequest>, i32) {
    let mut reqs = Vec::new();
    let mut total = 0;
    for o in &inv.owned {
        if let Some((t, max, _, fill)) = ammo_info(cat, o)
            && t < max
        {
            reqs.push(ShopRequest::Ammo { weapon: o.weapon.clone(), secondary: false, fill: true });
            total += fill;
        }
        if let Some(f) = alt_fill(cat, o) {
            reqs.push(ShopRequest::Ammo { weapon: o.weapon.clone(), secondary: true, fill: true });
            total += f;
        }
    }
    (reqs, total)
}

/// Which column has the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Focus {
    Gear,
    #[default]
    Shop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Buy,
    Sell,
    Ammo { fill: bool },
    FillAll { count: usize, expected: i32 },
    Armour,
}

/// A request sent this frame; its result is read (and logged) next frame.
#[derive(Clone, Debug)]
struct Pending {
    kind: Kind,
    weapon: String,
    name: String,
    /// The refusal the menu foresaw (the shared logic decides).
    predicted: Option<&'static str>,
    dosh: f32,
    weight: f32,
    ammo: u32,
    armour: f32,
}

#[derive(Resource, Default)]
pub struct NuMenu {
    pub focus: Focus,
    /// Gear rows: owned weapons, then the armour row.
    pub gear: usize,
    /// Shop rows (`Model::rows`).
    pub shop: usize,
    /// The shop list's first shown line.
    pub scroll: usize,
    /// Scroll the list to the selection when it is drawn next.
    follow: bool,
    /// The last action's result (text, done).
    status: Option<(String, bool)>,
    pending: Vec<Pending>,
    /// Clickable boxes drawn last frame (physical pixels).
    hits: Vec<(String, Rect)>,
    was_open: bool,
    cursor_was_grabbed: bool,
    /// Window size the layout was last logged for.
    logged_size: Vec2,
}

pub struct NuMenuPlugin;

impl Plugin for NuMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NuMenu>()
            .add_systems(PreUpdate, input.after(bevy::input::InputSystems).before(crate::game::buy_menu::menu_input));
    }
}

/// One thing to do, from a key, a click or a test action.
#[derive(Clone, Debug, PartialEq)]
enum Cmd {
    Up,
    Down,
    SwitchColumn,
    Group(usize),
    PrevGroup,
    NextGroup,
    SelectShop(usize),
    SelectGear(usize),
    Enter,
    Buy,
    Sell,
    Fill,
    Clip,
    FillAll,
    Armour,
    Grenade,
    Perk(Perk),
    Wheel(i32),
    Close,
}

/// A clicked box's id as a command.
fn click_cmd(id: &str) -> Option<Cmd> {
    let num = |p: &str| id.strip_prefix(p).and_then(|n| n.parse::<usize>().ok());
    Some(match id {
        "nu.buy" => Cmd::Buy,
        "nu.sell" => Cmd::Sell,
        "nu.fill" => Cmd::Fill,
        "nu.clip" => Cmd::Clip,
        "nu.fill_all" => Cmd::FillAll,
        "nu.armour" => Cmd::Armour,
        "nu.grenade" => Cmd::Grenade,
        "nu.close" => Cmd::Close,
        _ => {
            if let Some(n) = num("nu.shop:") {
                Cmd::SelectShop(n)
            } else if let Some(n) = num("nu.gear:") {
                Cmd::SelectGear(n)
            } else if let Some(n) = num("nu.group:") {
                Cmd::Group(n)
            } else {
                Cmd::Perk(*Perk::ALL.get(num("nu.perk:")?)?)
            }
        }
    })
}

/// The shared shop state the menu reads.
type ShopState<'w> = (Res<'w, ShopCatalogue>, Res<'w, ShopInventory>, Res<'w, crate::game::dosh::Dosh>, Res<'w, Armour>, Res<'w, Veterancy>);

#[allow(clippy::too_many_arguments)] // Bevy system parameters
fn input(
    (keys, mouse, scroll): (Res<ButtonInput<KeyCode>>, Res<ButtonInput<MouseButton>>, Res<AccumulatedMouseScroll>),
    mut menu: ResMut<BuyMenu>,
    mut nu: ResMut<NuMenu>,
    (cat, inv, dosh, armour, vet): ShopState,
    (script, frames): (Res<crate::weapons::weapon::ScriptedInput>, Res<bevy::diagnostic::FrameCount>),
    (mut requests, mut vest, mut perks): (MessageWriter<ShopRequest>, MessageWriter<BuyVest>, MessageWriter<crate::game::perks::PerkRequest>),
    mut window: Query<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
) {
    let nu = &mut *nu;
    // Last frame's requests have been carried out (weapon.rs, armour.rs).
    for p in std::mem::take(&mut nu.pending) {
        report(nu, &p, &cat, &inv, dosh.score, &armour);
    }
    let open = menu.open && menu.kind == MenuKind::Nu;
    let mut win = window.single_mut().ok();
    if open && !nu.was_open {
        // Opened (buy_menu.rs, shared): the shop starts on your perk's
        // group, the mouse cursor is freed.
        nu.focus = Focus::Shop;
        nu.shop = 0;
        nu.gear = 0;
        nu.scroll = 0;
        nu.follow = true;
        nu.status = None;
        if let Some((_, c)) = win.as_mut() {
            nu.cursor_was_grabbed = c.grab_mode != CursorGrabMode::None;
            c.grab_mode = CursorGrabMode::None;
            c.visible = true;
        }
        let m = model(&cat, &inv, &vet.vet, dosh.score);
        let count = |s: RowState| m.rows.iter().filter(|r| r.state == s).count();
        let (_, fill_all) = fill_all_plan(&cat, &inv);
        runlog::kv(
            "numenu_open",
            &format!(
                "dosh={:.0} weight={}/{} perk={} first_group={} discount={:.0}% groups={} rows={} can_buy={} owned={} too_expensive={} too_heavy={} gear={} fill_all={fill_all} armour={:.0}",
                dosh.score,
                inv.weight,
                inv.max_weight,
                vet.vet.label(),
                m.groups.first().map_or("none", |g| g.label),
                m.groups.first().map_or(0.0, |g| g.discount * 100.0),
                m.groups.len(),
                m.rows.len(),
                count(RowState::CanBuy),
                count(RowState::Owned),
                count(RowState::TooExpensive),
                count(RowState::TooHeavy),
                inv.owned.iter().map(|o| o.weapon.trim_start_matches("KFMod.")).collect::<Vec<_>>().join("+"),
                armour.strength
            ),
        );
    } else if !open && nu.was_open {
        if nu.cursor_was_grabbed
            && let Some((_, c)) = win.as_mut()
        {
            c.grab_mode = CursorGrabMode::Locked;
            c.visible = false;
        }
        runlog::kv("numenu_close", &format!("dosh={:.0} weight={}/{}", dosh.score, inv.weight, inv.max_weight));
    }
    nu.was_open = open;
    if !open {
        return;
    }
    let mut cmds = Vec::new();
    // Keys (buy_menu.rs clears them after: nothing else sees them).
    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    const DIGITS: [KeyCode; 8] = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6, KeyCode::Digit7, KeyCode::Digit8];
    for (i, k) in DIGITS.iter().enumerate() {
        if keys.just_pressed(*k) {
            match (shift, Perk::ALL.get(i)) {
                (true, Some(p)) => cmds.push(Cmd::Perk(*p)),
                (true, None) => {}
                (false, _) => cmds.push(Cmd::Group(i)),
            }
        }
    }
    for (k, c) in [
        (KeyCode::ArrowUp, Cmd::Up),
        (KeyCode::ArrowDown, Cmd::Down),
        (KeyCode::Tab, Cmd::SwitchColumn),
        (KeyCode::ArrowLeft, Cmd::SwitchColumn),
        (KeyCode::ArrowRight, Cmd::SwitchColumn),
        (KeyCode::PageUp, Cmd::PrevGroup),
        (KeyCode::PageDown, Cmd::NextGroup),
        (KeyCode::Enter, Cmd::Enter),
        (KeyCode::NumpadEnter, Cmd::Enter),
        (KeyCode::KeyB, Cmd::Buy),
        (KeyCode::KeyS, Cmd::Sell),
        (KeyCode::KeyR, Cmd::Fill),
        (KeyCode::KeyC, Cmd::Clip),
        (KeyCode::KeyA, Cmd::FillAll),
        (KeyCode::KeyV, Cmd::Armour),
        (KeyCode::KeyG, Cmd::Grenade),
    ] {
        if keys.just_pressed(k) {
            cmds.push(c);
        }
    }
    // The mouse: a click on a box drawn last frame, the wheel.
    if mouse.just_pressed(MouseButton::Left)
        && let Some(pos) = win.as_ref().and_then(|(w, _)| w.physical_cursor_position())
        && let Some((id, _)) = nu.hits.iter().rev().find(|(_, r)| r.contains(pos))
        && let Some(c) = click_cmd(id)
    {
        runlog::kv("numenu_click", &format!("id={id} source=mouse"));
        cmds.push(c);
    }
    if scroll.delta.y != 0.0 {
        cmds.push(Cmd::Wheel(if scroll.delta.y > 0.0 { -3 } else { 3 }));
    }
    // Test actions ("nu:down", "nu:select:Shotgun", "nu:click:nu.buy", ...).
    let m = model(&cat, &inv, &vet.vet, dosh.score);
    for (_, a) in script.0.iter().filter(|(f, _)| *f == frames.0) {
        let Some(a) = a.strip_prefix("nu:") else { continue };
        let c = match a {
            "up" => Some(Cmd::Up),
            "down" => Some(Cmd::Down),
            "tab" => Some(Cmd::SwitchColumn),
            "enter" => Some(Cmd::Enter),
            "buy" => Some(Cmd::Buy),
            "sell" => Some(Cmd::Sell),
            "fill" => Some(Cmd::Fill),
            "clip" => Some(Cmd::Clip),
            "fill_all" => Some(Cmd::FillAll),
            "armour" => Some(Cmd::Armour),
            "grenade" => Some(Cmd::Grenade),
            "close" => Some(Cmd::Close),
            "prev_group" => Some(Cmd::PrevGroup),
            "next_group" => Some(Cmd::NextGroup),
            _ => {
                if let Some(n) = a.strip_prefix("group:").and_then(|n| n.parse::<usize>().ok()) {
                    Some(Cmd::Group(n.saturating_sub(1)))
                } else if let Some(n) = a.strip_prefix("wheel:").and_then(|n| n.parse::<i32>().ok()) {
                    Some(Cmd::Wheel(n))
                } else if let Some(c) = a.strip_prefix("select:") {
                    let class = if c.contains('.') { c.to_string() } else { format!("KFMod.{c}") };
                    match m.rows.iter().position(|r| cat.items[r.item].weapon.eq_ignore_ascii_case(&class)) {
                        Some(r) => Some(Cmd::SelectShop(r)),
                        None => match inv.owned.iter().position(|o| o.weapon.eq_ignore_ascii_case(&class)) {
                            Some(g) => Some(Cmd::SelectGear(g)),
                            None => {
                                runlog::kv("numenu_action", &format!("action=select weapon={class} result=not_listed"));
                                None
                            }
                        },
                    }
                } else if let Some(id) = a.strip_prefix("click:") {
                    // As a mouse click: only on a box that was drawn.
                    if nu.hits.iter().any(|(h, _)| h == id) {
                        runlog::kv("numenu_click", &format!("id={id} source=test"));
                        click_cmd(id)
                    } else {
                        runlog::kv("numenu_click", &format!("id={id} source=test result=no_such_box"));
                        None
                    }
                } else {
                    runlog::kv("numenu_action", &format!("action={a} result=unknown"));
                    None
                }
            }
        };
        cmds.extend(c);
    }
    for c in cmds {
        let ctx = Ctx { cat: &cat, inv: &inv, dosh: dosh.score, armour: &armour, vet: &vet, model: &m };
        for e in apply(nu, &ctx, c) {
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
                    runlog::kv("buy_menu", "open=false reason=numenu_close_button");
                }
            }
        }
    }
}

struct Ctx<'a> {
    cat: &'a ShopCatalogue,
    inv: &'a ShopInventory,
    dosh: f32,
    armour: &'a Armour,
    vet: &'a Veterancy,
    model: &'a Model,
}

enum Effect {
    Request(ShopRequest),
    Vest,
    Perk(Perk),
    Close,
}

/// The weapon the selection points at: (class, name, owned index).
fn selected(nu: &NuMenu, c: &Ctx) -> Option<(String, String, Option<usize>)> {
    match nu.focus {
        Focus::Gear => c.inv.owned.get(nu.gear).map(|o| (o.weapon.clone(), o.name.clone(), Some(nu.gear))),
        Focus::Shop => c.model.rows.get(nu.shop).map(|r| {
            let it = &c.cat.items[r.item];
            (it.weapon.clone(), it.name.clone(), c.inv.owned.iter().position(|o| o.weapon.eq_ignore_ascii_case(&it.weapon)))
        }),
    }
}

fn log_select(nu: &NuMenu, c: &Ctx, how: &str) {
    match nu.focus {
        Focus::Shop => {
            if let Some(r) = c.model.rows.get(nu.shop) {
                let it = &c.cat.items[r.item];
                let g = &c.model.groups[r.group];
                runlog::kv(
                    "numenu_select",
                    &format!("column=shop row={} weapon={} group={} price={} weight={} state={} scaling={:.2} how={how}", nu.shop, it.weapon, g.label, r.price, r.weight, r.state.word(), r.scaling),
                );
            }
        }
        Focus::Gear => {
            let what = c.inv.owned.get(nu.gear).map_or("armour".to_string(), |o| o.weapon.clone());
            runlog::kv("numenu_select", &format!("column=gear row={} item={what} how={how}", nu.gear));
        }
    }
}

fn pending(c: &Ctx, kind: Kind, weapon: &str, name: &str, predicted: Option<&'static str>) -> Pending {
    let ammo = c.inv.owned.iter().find(|o| o.weapon.eq_ignore_ascii_case(weapon)).map_or(0, |o| o.ammo.map_or(0, |a| a.0) + o.alt_ammo.map_or(0, |a| a.0));
    Pending { kind, weapon: weapon.to_string(), name: name.to_string(), predicted, dosh: c.dosh, weight: c.inv.weight, ammo, armour: c.armour.strength }
}

/// Carries out one command: moves the selection, or sends requests
/// (logged as `numenu_*_request`; the result is logged next frame).
fn apply(nu: &mut NuMenu, c: &Ctx, cmd: Cmd) -> Vec<Effect> {
    let mut out = Vec::new();
    let rows = c.model.rows.len();
    let gear_rows = c.inv.owned.len() + 1;
    match cmd {
        Cmd::Up | Cmd::Down => {
            let down = cmd == Cmd::Down;
            match nu.focus {
                Focus::Shop if rows > 0 => nu.shop = if down { (nu.shop + 1).min(rows - 1) } else { nu.shop.saturating_sub(1) },
                Focus::Gear => nu.gear = if down { (nu.gear + 1).min(gear_rows - 1) } else { nu.gear.saturating_sub(1) },
                _ => {}
            }
            nu.follow = true;
            log_select(nu, c, "key");
        }
        Cmd::SwitchColumn => {
            nu.focus = if nu.focus == Focus::Shop { Focus::Gear } else { Focus::Shop };
            log_select(nu, c, "column");
        }
        Cmd::Group(g) => {
            if let Some(gr) = c.model.groups.get(g) {
                nu.focus = Focus::Shop;
                nu.shop = gr.first;
                nu.follow = true;
                log_select(nu, c, "group");
            }
        }
        Cmd::PrevGroup | Cmd::NextGroup => {
            let cur = c.model.rows.get(nu.shop).map_or(0, |r| r.group);
            let g = if cmd == Cmd::NextGroup { (cur + 1).min(c.model.groups.len().saturating_sub(1)) } else { cur.saturating_sub(1) };
            if let Some(gr) = c.model.groups.get(g) {
                nu.focus = Focus::Shop;
                nu.shop = gr.first;
                nu.follow = true;
                log_select(nu, c, "group");
            }
        }
        Cmd::SelectShop(r) if r < rows => {
            nu.focus = Focus::Shop;
            nu.shop = r;
            nu.follow = true;
            log_select(nu, c, "pick");
        }
        Cmd::SelectGear(g) if g < gear_rows => {
            nu.focus = Focus::Gear;
            nu.gear = g;
            log_select(nu, c, "pick");
        }
        Cmd::Wheel(n) => {
            // Clamped to the list when drawn.
            nu.scroll = (nu.scroll as i64 + n as i64).clamp(0, c.model.lines.len() as i64) as usize;
            runlog::kv("numenu_scroll", &format!("by={n} top_line={} lines={}", nu.scroll, c.model.lines.len()));
        }
        Cmd::Enter => {
            // Shop: buy; gear: fill the weapon's ammo (armour row: armour).
            let next = match nu.focus {
                Focus::Shop => Cmd::Buy,
                Focus::Gear if nu.gear >= c.inv.owned.len() => Cmd::Armour,
                Focus::Gear => Cmd::Fill,
            };
            out.extend(apply(nu, c, next));
        }
        Cmd::Buy => {
            let Some(r) = c.model.rows.get(nu.shop).filter(|_| nu.focus == Focus::Shop) else {
                nu.status = Some(("Pick a weapon in the shop to buy.".into(), false));
                runlog::kv("numenu_refused", "action=buy weapon=none reason=no_shop_selection");
                return out;
            };
            let it = &c.cat.items[r.item];
            let predicted = match r.state {
                RowState::CanBuy => None,
                s => Some(s.word()),
            };
            runlog::kv(
                "numenu_buy_request",
                &format!("weapon={} price={} weight={} dosh={:.0} carry={}/{} predicted={}", it.weapon, r.price, r.weight, c.dosh, c.inv.weight, c.inv.max_weight, predicted.unwrap_or("ok")),
            );
            nu.pending.push(pending(c, Kind::Buy, &it.weapon, &it.name, predicted));
            out.push(Effect::Request(ShopRequest::Buy(it.weapon.clone())));
        }
        Cmd::Sell => {
            let Some((weapon, name, Some(i))) = selected(nu, c) else {
                nu.status = Some(("Pick a weapon you own to sell.".into(), false));
                runlog::kv("numenu_refused", "action=sell weapon=none reason=not_owned");
                return out;
            };
            let o = &c.inv.owned[i];
            let predicted = (!o.sellable).then_some("not_sellable");
            runlog::kv("numenu_sell_request", &format!("weapon={weapon} value={} predicted={}", o.sell_value, predicted.unwrap_or("ok")));
            nu.pending.push(pending(c, Kind::Sell, &weapon, &name, predicted));
            out.push(Effect::Request(ShopRequest::Sell(weapon)));
        }
        Cmd::Fill | Cmd::Clip => {
            let fill = cmd == Cmd::Fill;
            let Some((weapon, name, Some(i))) = selected(nu, c) else {
                nu.status = Some(("Pick a weapon you own to buy ammo for.".into(), false));
                runlog::kv("numenu_refused", &format!("action={} weapon=none reason=not_owned", if fill { "fill" } else { "clip" }));
                return out;
            };
            ammo_request(nu, c, &mut out, &c.inv.owned[i], &weapon, &name, fill);
        }
        Cmd::Grenade => match c.inv.owned.iter().find(|o| o.weapon.eq_ignore_ascii_case(FRAG)) {
            Some(o) => {
                let (w, n) = (o.weapon.clone(), o.name.clone());
                ammo_request(nu, c, &mut out, o, &w, &n, false);
            }
            None => {
                nu.status = Some(("You carry no grenades.".into(), false));
                runlog::kv("numenu_refused", "action=grenade weapon=KFMod.Frag reason=not_owned");
            }
        },
        Cmd::FillAll => {
            let (reqs, total) = fill_all_plan(c.cat, c.inv);
            let predicted = if reqs.is_empty() {
                Some("full")
            } else if c.dosh < 1.0 {
                Some("too_expensive")
            } else {
                None
            };
            runlog::kv("numenu_fill_all_request", &format!("requests={} cost={total} dosh={:.0} predicted={}", reqs.len(), c.dosh, predicted.unwrap_or("ok")));
            nu.pending.push(pending(c, Kind::FillAll { count: reqs.len(), expected: total }, "all", "all ammo", predicted));
            out.extend(reqs.into_iter().map(Effect::Request));
        }
        Cmd::Armour => {
            let (points, fill) = crate::player::armour::menu_row(c.armour, c.vet.vet.cost_scaling("vest"));
            let predicted = if points >= 100 {
                Some("full")
            } else if c.dosh <= 0.0 {
                Some("too_expensive")
            } else {
                None
            };
            runlog::kv("numenu_armour_request", &format!("points={points} fill_price={fill} dosh={:.0} predicted={}", c.dosh, predicted.unwrap_or("ok")));
            nu.pending.push(pending(c, Kind::Armour, "vest", "Combat armour", predicted));
            out.push(Effect::Vest);
        }
        Cmd::Perk(p) => {
            runlog::kv("numenu_perk_request", &format!("perk={}", p.name()));
            out.push(Effect::Perk(p));
        }
        Cmd::Close => out.push(Effect::Close),
        _ => {}
    }
    out
}

fn ammo_request(nu: &mut NuMenu, c: &Ctx, out: &mut Vec<Effect>, o: &OwnedWeapon, weapon: &str, name: &str, fill: bool) {
    let info = ammo_info(c.cat, o);
    let predicted = match info {
        None => Some("no_ammo"),
        Some((t, max, _, _)) if t >= max => Some("full"),
        // Short of dosh, weapon.rs buys what the dosh covers; nothing
        // under one round's price.
        Some(_) if c.dosh < 1.0 => Some("too_expensive"),
        _ => None,
    };
    let price = info.map_or(0, |(_, _, clip, f)| if fill { f } else { clip });
    runlog::kv("numenu_ammo_request", &format!("weapon={weapon} fill={fill} price={price} dosh={:.0} predicted={}", c.dosh, predicted.unwrap_or("ok")));
    nu.pending.push(pending(c, Kind::Ammo { fill }, weapon, name, predicted));
    out.push(Effect::Request(ShopRequest::Ammo { weapon: weapon.to_string(), secondary: false, fill }));
}

/// Logs what a request did (read from the shared state one frame later)
/// and sets the status line.
fn report(nu: &mut NuMenu, p: &Pending, cat: &ShopCatalogue, inv: &ShopInventory, dosh: f32, armour: &Armour) {
    let owned = inv.owned.iter().find(|o| o.weapon.eq_ignore_ascii_case(&p.weapon));
    let ammo = owned.map_or(0, |o| o.ammo.map_or(0, |a| a.0) + o.alt_ammo.map_or(0, |a| a.0));
    let spent = p.dosh - dosh;
    let refused = |nu: &mut NuMenu, action: &str, why: &str| {
        let reason = p.predicted.unwrap_or("unknown");
        runlog::kv("numenu_refused", &format!("action={action} weapon={} reason={reason} dosh={dosh:.0} weight={}/{}", p.weapon, inv.weight, inv.max_weight));
        nu.status = Some((format!("{why}: {}", say(reason, p, cat, inv, dosh)), false));
    };
    match p.kind {
        Kind::Buy => {
            if owned.is_some() {
                runlog::kv("numenu_buy", &format!("weapon={} price={spent:.0} dosh_after={dosh:.0} weight_before={} weight_after={}/{}", p.weapon, p.weight, inv.weight, inv.max_weight));
                nu.status = Some((format!("Bought {} for £{spent:.0}.  Carry {} / {} kg.", p.name, inv.weight, inv.max_weight), true));
            } else {
                refused(nu, "buy", &format!("Can't buy {}", p.name));
            }
        }
        Kind::Sell => {
            if owned.is_none() {
                runlog::kv("numenu_sell", &format!("weapon={} price={:.0} dosh_after={dosh:.0} weight_after={}/{}", p.weapon, -spent, inv.weight, inv.max_weight));
                nu.status = Some((format!("Sold {} for £{:.0}.", p.name, -spent), true));
            } else {
                refused(nu, "sell", &format!("Can't sell {}", p.name));
            }
        }
        Kind::Ammo { fill } => {
            if ammo > p.ammo {
                runlog::kv("numenu_ammo", &format!("weapon={} fill={fill} added={} paid={spent:.0} dosh_after={dosh:.0}", p.weapon, ammo - p.ammo));
                nu.status = Some((format!("{} +{} rounds for £{spent:.0}.", p.name, ammo - p.ammo), true));
            } else {
                refused(nu, if fill { "fill" } else { "clip" }, &format!("No ammo bought for {}", p.name));
            }
        }
        Kind::FillAll { count, expected } => {
            if spent > 0.0 {
                runlog::kv("numenu_fill_all", &format!("requests={count} expected={expected} paid={spent:.0} dosh_after={dosh:.0}"));
                nu.status = Some((format!("Refilled all ammo for £{spent:.0}."), true));
            } else {
                refused(nu, "fill_all", "No ammo bought");
            }
        }
        Kind::Armour => {
            if armour.strength > p.armour {
                runlog::kv("numenu_armour", &format!("armour_before={:.0} armour_after={:.0} paid={spent:.0} dosh_after={dosh:.0}", p.armour, armour.strength));
                nu.status = Some((format!("Armour {:.0} -> {:.0} for £{spent:.0}.", p.armour, armour.strength), true));
            } else {
                refused(nu, "armour", "No armour bought");
            }
        }
    }
}

/// A refusal reason in plain words.
fn say(reason: &str, p: &Pending, cat: &ShopCatalogue, inv: &ShopInventory, dosh: f32) -> String {
    match reason {
        "too_heavy" => {
            let w = cat.item(&p.weapon).map_or(0.0, |i| i.weight);
            format!("too heavy (needs {w} kg, {} kg free)", inv.max_weight - inv.weight)
        }
        "too_expensive" => format!("not enough dosh (you have £{dosh:.0})"),
        "owned" => "you already own it".into(),
        "full" => "already full".into(),
        "not_sellable" => "it cannot be sold".into(),
        "no_ammo" => "it uses no ammo".into(),
        _ => "the trader said no".into(),
    }
}

// ---------------------------------------------------------------- drawing

type Rgba = [u8; 4];
const BACKDROP: Rgba = [6, 5, 5, 228];
const PANEL: Rgba = [20, 16, 15, 235];
const PANEL_EDGE: Rgba = [70, 22, 18, 255];
const RULE: Rgba = [150, 26, 20, 255];
const BONE: Rgba = [230, 220, 200, 255];
const DIM: Rgba = [135, 125, 115, 255];
const FAINT: Rgba = [90, 82, 76, 255];
const GREEN: Rgba = [125, 200, 95, 255];
const AMBER: Rgba = [235, 165, 45, 255];
const RED: Rgba = [225, 60, 48, 255];
const GOLD: Rgba = [230, 190, 95, 255];
const SELECT: Rgba = [105, 22, 17, 230];
const HOVER: Rgba = [48, 36, 32, 230];
const TRACK: Rgba = [44, 37, 34, 255];

/// The fonts by screen height: (small, body, heading, big).
fn fonts(h: f32) -> (&'static str, &'static str, &'static str, &'static str) {
    if h < 850.0 {
        ("ROFonts.ROBtsrmVr8", "ROFonts.ROBtsrmVr10", "ROFonts.ROBtsrmVr12", "ROFonts.ROBtsrmVr16")
    } else if h < 1000.0 {
        ("ROFonts.ROBtsrmVr10", "ROFonts.ROBtsrmVr12", "ROFonts.ROBtsrmVr14", "ROFonts.ROBtsrmVr18")
    } else {
        ("ROFonts.ROBtsrmVr12", "ROFonts.ROBtsrmVr14", "ROFonts.ROBtsrmVr16", "ROFonts.ROBtsrmVr18")
    }
}

/// What the screen reads (menus/mod.rs draws it with the other pages).
#[derive(SystemParam)]
pub struct NuDraw<'w> {
    menu: Res<'w, BuyMenu>,
    nu: ResMut<'w, NuMenu>,
    cat: Res<'w, ShopCatalogue>,
    inv: Res<'w, ShopInventory>,
    dosh: Res<'w, crate::game::dosh::Dosh>,
    armour: Res<'w, Armour>,
    vet: Res<'w, Veterancy>,
    game: Res<'w, crate::game::waves::WaveGame>,
}

impl NuDraw<'_> {
    pub fn showing(&self) -> bool {
        self.menu.open && self.menu.kind == MenuKind::Nu
    }
}

fn frame(p: &mut Painter, r: Rect, what: &str) {
    p.fill(r, PANEL, what);
    let t = 1.0f32.max((p.screen.height() / 720.0).round());
    p.fill(Rect::new(r.min.x, r.min.y, r.max.x, r.min.y + t), PANEL_EDGE, what);
    p.fill(Rect::new(r.min.x, r.max.y - t, r.max.x, r.max.y), PANEL_EDGE, what);
    p.fill(Rect::new(r.min.x, r.min.y, r.min.x + t, r.max.y), PANEL_EDGE, what);
    p.fill(Rect::new(r.max.x - t, r.min.y, r.max.x, r.max.y), PANEL_EDGE, what);
}

/// A bar: `frac` of the track filled.
fn bar(p: &mut Painter, r: Rect, frac: f32, color: Rgba, what: &str) {
    p.fill(r, TRACK, what);
    let w = r.width() * frac.clamp(0.0, 1.0);
    if w > 0.5 {
        p.fill(Rect::new(r.min.x, r.min.y, r.min.x + w, r.max.y), color, what);
    }
}

/// A flat button: id for clicks, key hint at the right.
#[allow(clippy::too_many_arguments)]
fn button(p: &mut Painter, id: &str, r: Rect, caption: &str, key: &str, enabled: bool, strong: bool, font: &str, small: &str) {
    let hover = enabled && p.hover(r);
    let bg = match (enabled, strong, hover) {
        (false, _, _) => [30, 26, 24, 235],
        (true, true, false) => [125, 26, 20, 245],
        (true, true, true) => [165, 38, 28, 250],
        (true, false, false) => [52, 42, 38, 240],
        (true, false, true) => [78, 60, 52, 245],
    };
    p.fill(r, bg, id);
    p.fill(Rect::new(r.min.x, r.max.y - 2.0, r.max.x, r.max.y), if enabled { RULE } else { [50, 40, 36, 255] }, id);
    let pad = r.height() * 0.3;
    p.text_in(font, caption, Rect::new(r.min.x + pad, r.min.y, r.max.x - pad, r.max.y), Align::Left, true, if enabled { BONE } else { FAINT }, &format!("{id}.Caption"));
    if !key.is_empty() {
        p.text_in(small, key, Rect::new(r.min.x + pad, r.min.y, r.max.x - pad, r.max.y), Align::Right, true, if enabled { DIM } else { FAINT }, &format!("{id}.Key"));
    }
    if enabled {
        p.hit(id, r);
    }
}

fn money(v: i32) -> String {
    format!("£{v}")
}

fn kg(w: f32) -> String {
    if w.fract() == 0.0 { format!("{w:.0} kg") } else { format!("{w:.1} kg") }
}

/// Cuts a text to a width (with "..").
fn fit(p: &Painter, font: &str, text: &str, width: f32) -> String {
    if p.text_size(font, text).x <= width {
        return text.to_string();
    }
    let mut s: String = text.to_string();
    while !s.is_empty() && p.text_size(font, &format!("{s}..")).x > width {
        s.pop();
    }
    format!("{s}..")
}

/// Draws NuMenu (called by menus/mod.rs while it is open).
pub fn draw(p: &mut Painter, d: &mut NuDraw) {
    let nu = &mut *d.nu;
    let (cat, inv, vet, armour) = (&*d.cat, &*d.inv, &*d.vet, &*d.armour);
    let dosh = d.dosh.score;
    let m = model(cat, inv, &vet.vet, dosh);
    let s = p.screen;
    let (sw, sh) = (s.width(), s.height());
    let (small, body, head, big) = fonts(sh);
    let (lh_s, lh_b, lh_h, lh_big) = (p.line_height(small), p.line_height(body), p.line_height(head), p.line_height(big));
    let u = (sh / 720.0).max(1.0);
    let margin = (sh * 0.03).round();
    let gap = (10.0 * u).round();
    p.fill(s, BACKDROP, "Nu.Backdrop");
    let inner = Rect::new(s.min.x + margin, s.min.y + margin, s.max.x - margin, s.max.y - margin);

    // ---- Top bar.
    let pad = (8.0 * u).round();
    let top = Rect::new(inner.min.x, inner.min.y, inner.max.x, inner.min.y + lh_h + lh_big + pad * 3.0);
    frame(p, top, "Nu.Top");
    p.fill(Rect::new(top.min.x, top.min.y, top.min.x + 4.0 * u, top.max.y), RULE, "Nu.Top.Accent");
    let x0 = top.min.x + pad * 2.0;
    let y1 = top.min.y + pad;
    p.text(head, "TRADER", Vec2::new(x0, y1), RED, "Nu.Title");
    let g = &d.game;
    let wave_text = match g.phase {
        crate::game::waves::Phase::Countdown => format!("Wave {} of {} starts in {}:{:02}", g.wave_num + 1, g.final_wave.max(g.wave_num + 1), g.countdown.max(0) / 60, g.countdown.max(0) % 60),
        _ => "Wave running: the shop is closing".to_string(),
    };
    let tw = p.text_size(head, "TRADER").x;
    p.text(body, &wave_text, Vec2::new(x0 + tw + pad * 3.0, y1 + (lh_h - lh_b) / 2.0), DIM, "Nu.Wave");
    // CLOSE, top right.
    let cw = p.text_size(body, "CLOSE   Esc").x + pad * 4.0;
    let close = Rect::new(top.max.x - pad - cw, y1, top.max.x - pad, y1 + lh_h + pad * 0.5);
    button(p, "nu.close", close, "CLOSE", "Esc", true, false, body, small);
    // The perk icons, left of CLOSE: yours lit.
    let icon = lh_h + pad * 0.5;
    let mut ix = close.min.x - pad * 2.0 - icon * 7.0 - pad * 6.0 * 0.5;
    let label = "PERK  Shift+1-7";
    let lw = p.text_size(small, label).x;
    p.text(small, label, Vec2::new(ix - lw - pad, y1 + (icon - lh_s) / 2.0), FAINT, "Nu.PerkLabel");
    for (i, perk) in Perk::ALL.iter().enumerate() {
        let r = Rect::new(ix, y1, ix + icon, y1 + icon);
        let on = vet.vet.perk == Some(*perk);
        let picked = vet.selected == Some(*perk);
        if on || picked {
            p.fill(r.inflate(2.0 * u), if on { RULE } else { PANEL_EDGE }, "Nu.PerkOn");
        } else if p.hover(r) {
            p.fill(r.inflate(2.0 * u), HOVER, "Nu.PerkHover");
        }
        let tex = p.gui.tex(perk.icons().0);
        p.tile(tex, r, if on { [255, 255, 255, 255] } else { [255, 255, 255, 110] }, "Nu.PerkIcon");
        p.hit(&format!("nu.perk:{i}"), r);
        ix += icon + pad * 0.5;
    }
    // Row 2: dosh, carry weight.
    let y2 = y1 + lh_h + pad;
    p.text(small, "DOSH", Vec2::new(x0, y2 + (lh_big - lh_s) / 2.0), DIM, "Nu.DoshLabel");
    let dl = p.text_size(small, "DOSH").x + pad;
    let dosh_text = money(dosh as i32);
    p.text(big, &dosh_text, Vec2::new(x0 + dl, y2), GOLD, "Nu.Dosh");
    let cx = x0 + dl + p.text_size(big, "£00000").x + pad * 4.0;
    p.text(small, "CARRY", Vec2::new(cx, y2 + (lh_big - lh_s) / 2.0), DIM, "Nu.CarryLabel");
    let bx = cx + p.text_size(small, "CARRY").x + pad;
    let bw = (sw * 0.25).round();
    let free = inv.max_weight - inv.weight;
    let wcol = if free <= 0.0 {
        RED
    } else if free <= 2.0 {
        AMBER
    } else {
        GREEN
    };
    let br = Rect::new(bx, y2 + lh_big * 0.25, bx + bw, y2 + lh_big * 0.75);
    bar(p, br, inv.weight / inv.max_weight.max(1.0), wcol, "Nu.CarryBar");
    // One tick per kilogram.
    let n = inv.max_weight.round() as i32;
    for k in 1..n {
        let x = br.min.x + br.width() * k as f32 / n as f32;
        p.fill(Rect::new(x - 0.5 * u, br.min.y, x + 0.5 * u, br.max.y), [10, 8, 8, 160], "Nu.CarryTick");
    }
    p.text(body, &format!("{} / {}   ({} free)", kg(inv.weight), kg(inv.max_weight), kg(free.max(0.0))), Vec2::new(br.max.x + pad, y2 + (lh_big - lh_b) / 2.0), BONE, "Nu.Carry");

    // ---- Footer.
    let foot = Rect::new(inner.min.x, inner.max.y - (lh_b + lh_s + pad * 3.0), inner.max.x, inner.max.y);
    frame(p, foot, "Nu.Foot");
    match &nu.status {
        Some((t, ok)) => p.text(body, t, Vec2::new(foot.min.x + pad * 2.0, foot.min.y + pad), if *ok { GREEN } else { RED }, "Nu.Status"),
        None => p.text(body, "Pick a weapon. Green prices you can pay; red you can't; amber weight won't fit.", Vec2::new(foot.min.x + pad * 2.0, foot.min.y + pad), DIM, "Nu.Status"),
    }
    let help = "Up/Down select   Tab column   1-8 / PgUp PgDn groups   Enter/B buy   S sell   R fill   C +mag   A refill all   V armour   G grenade   Esc/E close";
    let help = fit(p, small, help, foot.width() - pad * 4.0);
    p.text(small, &help, Vec2::new(foot.min.x + pad * 2.0, foot.min.y + pad * 2.0 + lh_b), FAINT, "Nu.Help");

    // ---- Columns.
    let body_area = Rect::new(inner.min.x, top.max.y + gap, inner.max.x, foot.min.y - gap);
    let w = body_area.width() - gap * 2.0;
    let gear = Rect::new(body_area.min.x, body_area.min.y, body_area.min.x + (w * 0.29).round(), body_area.max.y);
    let shop = Rect::new(gear.max.x + gap, body_area.min.y, gear.max.x + gap + (w * 0.37).round(), body_area.max.y);
    let det = Rect::new(shop.max.x + gap, body_area.min.y, body_area.max.x, body_area.max.y);
    let row_h = (lh_b * 1.45).round();

    // Column title: name, then a red rule; returns the area below.
    let title = |p: &mut Painter, r: Rect, t: &str, focus: bool, what: &str| -> Rect {
        frame(p, r, what);
        p.text(head, t, Vec2::new(r.min.x + pad * 1.5, r.min.y + pad), if focus { BONE } else { DIM }, what);
        let y = r.min.y + pad * 2.0 + lh_h;
        p.fill(Rect::new(r.min.x + pad, y - 2.0 * u, r.max.x - pad, y), if focus { RULE } else { PANEL_EDGE }, what);
        Rect::new(r.min.x + pad, y + pad * 0.5, r.max.x - pad, r.max.y - pad)
    };

    // ---- Your gear.
    let ga = title(p, gear, "YOUR GEAR", nu.focus == Focus::Gear, "Nu.Gear");
    let btn_h = (lh_b * 1.6).round();
    let buttons_top = ga.max.y - (btn_h * 3.0 + pad * 2.0);
    let mut y = ga.min.y;
    let num_w = p.text_size(small, "000/000").x;
    for (i, o) in inv.owned.iter().enumerate() {
        let info = ammo_info(cat, o);
        let lines = if info.is_some() { 2.0 } else { 1.0 };
        let r = Rect::new(ga.min.x, y, ga.max.x, y + row_h * 0.8 * lines + pad * 0.3);
        if r.max.y > buttons_top - pad {
            break;
        }
        let sel = nu.focus == Focus::Gear && nu.gear == i;
        if sel {
            p.fill(r, SELECT, "Nu.GearSel");
            p.fill(Rect::new(r.min.x, r.min.y, r.min.x + 3.0 * u, r.max.y), RED, "Nu.GearSel");
        } else if p.hover(r) {
            p.fill(r, HOVER, "Nu.GearHover");
        }
        p.hit(&format!("nu.gear:{i}"), r);
        let tx = r.min.x + pad;
        let sell = if o.sellable { format!("sell {}", money(o.sell_value)) } else { "keeps".to_string() };
        let sw_ = p.text_size(small, &sell).x;
        p.text(body, &fit(p, body, &o.name, r.width() - sw_ - pad * 3.0), Vec2::new(tx, r.min.y + pad * 0.3), BONE, "Nu.GearName");
        p.text(small, &sell, Vec2::new(r.max.x - pad - sw_, r.min.y + pad * 0.3 + (lh_b - lh_s) / 2.0), if o.sellable { DIM } else { FAINT }, "Nu.GearSell");
        if let Some((t, max, clip, fill)) = info {
            let ly = r.min.y + pad * 0.3 + row_h * 0.8;
            let frac = t as f32 / max.max(1) as f32;
            let col = if frac >= 1.0 {
                GREEN
            } else if frac < 0.34 {
                RED
            } else {
                AMBER
            };
            let price = if t >= max {
                "FULL".to_string()
            } else if o.weapon.eq_ignore_ascii_case(FRAG) {
                format!("+1 {}", money(clip))
            } else {
                format!("fill {}", money(fill))
            };
            let pw = p.text_size(small, &price).x;
            let bar_r = Rect::new(tx, ly + lh_s * 0.3, r.max.x - pad * 3.0 - pw - num_w, ly + lh_s * 0.8);
            bar(p, bar_r, frac, col, "Nu.GearAmmo");
            p.text(small, &format!("{t}/{max}"), Vec2::new(bar_r.max.x + pad, ly), DIM, "Nu.GearAmmoNum");
            p.text(small, &price, Vec2::new(r.max.x - pad - pw, ly), if t >= max { FAINT } else { BONE }, "Nu.GearFill");
        }
        y = r.max.y + pad * 0.3;
    }
    // The armour row.
    let (points, vest_fill) = crate::player::armour::menu_row(armour, vet.vet.cost_scaling("vest"));
    let ai = inv.owned.len();
    let r = Rect::new(ga.min.x, y, ga.max.x, y + row_h * 1.6 + pad * 0.3);
    if r.max.y <= buttons_top {
        let sel = nu.focus == Focus::Gear && nu.gear == ai;
        if sel {
            p.fill(r, SELECT, "Nu.GearSel");
            p.fill(Rect::new(r.min.x, r.min.y, r.min.x + 3.0 * u, r.max.y), RED, "Nu.GearSel");
        } else if p.hover(r) {
            p.fill(r, HOVER, "Nu.GearHover");
        }
        p.hit(&format!("nu.gear:{ai}"), r);
        let tx = r.min.x + pad;
        p.text(body, "Combat armour", Vec2::new(tx, r.min.y + pad * 0.3), BONE, "Nu.Armour");
        let ly = r.min.y + pad * 0.3 + row_h * 0.8;
        let price = if points >= 100 { "FULL".to_string() } else { format!("fill {}", money(vest_fill)) };
        let pw = p.text_size(small, &price).x;
        let bar_r = Rect::new(tx, ly + lh_s * 0.3, r.max.x - pad * 3.0 - pw - num_w, ly + lh_s * 0.8);
        bar(p, bar_r, points as f32 / 100.0, [110, 150, 200, 255], "Nu.ArmourBar");
        p.text(small, &format!("{points}/100"), Vec2::new(bar_r.max.x + pad, ly), DIM, "Nu.ArmourNum");
        p.text(small, &price, Vec2::new(r.max.x - pad - pw, ly), if points >= 100 { FAINT } else { BONE }, "Nu.ArmourFill");
    }
    // Quick buttons.
    let (fill_reqs, fill_total) = fill_all_plan(cat, inv);
    let frag = inv.owned.iter().find(|o| o.weapon.eq_ignore_ascii_case(FRAG)).and_then(|o| ammo_info(cat, o));
    let mut by = buttons_top;
    let bb = |y: f32| Rect::new(ga.min.x, y, ga.max.x, y + btn_h);
    let fill_caption = if fill_reqs.is_empty() { "AMMO FULL".to_string() } else { format!("REFILL ALL AMMO  {}", money(fill_total)) };
    button(p, "nu.fill_all", bb(by), &fill_caption, "A", !fill_reqs.is_empty(), !fill_reqs.is_empty() && dosh >= fill_total as f32, body, small);
    by += btn_h + pad;
    let armour_caption = if points >= 100 { "ARMOUR FULL".to_string() } else { format!("{} ARMOUR  {}", if points > 0 { "FILL" } else { "BUY" }, money(vest_fill)) };
    button(p, "nu.armour", bb(by), &armour_caption, "V", points < 100, false, body, small);
    by += btn_h + pad;
    let (gcap, gok) = match frag {
        Some((t, max, clip, _)) if t < max => (format!("GRENADE  {}   ({t}/{max})", money(clip)), true),
        Some((t, max, _, _)) => (format!("GRENADES FULL  ({t}/{max})"), false),
        None => ("NO GRENADES".to_string(), false),
    };
    button(p, "nu.grenade", bb(by), &gcap, "G", gok, false, body, small);

    // ---- Shop.
    let sa = title(p, shop, "SHOP", nu.focus == Focus::Shop, "Nu.Shop");
    let visible = ((sa.height() / row_h).floor() as usize).max(1);
    let line_of_sel = m.lines.iter().position(|l| *l == Line::Row(nu.shop)).unwrap_or(0);
    if nu.follow {
        nu.follow = false;
        // Keep the group header in view above a group's first row.
        let want_top = if line_of_sel > 0 && matches!(m.lines[line_of_sel - 1], Line::Header(_)) { line_of_sel - 1 } else { line_of_sel };
        if want_top < nu.scroll {
            nu.scroll = want_top;
        } else if line_of_sel >= nu.scroll + visible {
            nu.scroll = line_of_sel + 1 - visible;
        }
    }
    nu.scroll = nu.scroll.min(m.lines.len().saturating_sub(visible));
    let price_w = p.text_size(body, "£0000").x;
    let weight_w = p.text_size(small, "10 kg").x;
    let tag_w = p.text_size(small, "TOO HEAVY").x;
    for (k, line) in m.lines.iter().enumerate().skip(nu.scroll).take(visible) {
        let ly = sa.min.y + (k - nu.scroll) as f32 * row_h;
        let r = Rect::new(sa.min.x, ly, sa.max.x, ly + row_h);
        match *line {
            Line::Header(gi) => {
                let gr = &m.groups[gi];
                let ic = row_h * 0.8;
                let iy = r.min.y + (row_h - ic) / 2.0;
                if let Some(perk) = gr.perk {
                    let tex = p.gui.tex(perk.icons().0);
                    p.tile(tex, Rect::new(r.min.x, iy, r.min.x + ic, iy + ic), if gr.yours { [255, 255, 255, 255] } else { [255, 255, 255, 150] }, "Nu.GroupIcon");
                }
                let gx = r.min.x + ic + pad;
                let name = format!("{}{}", gi + 1, if gr.yours { "  YOUR PERK: " } else { "  " });
                let label = format!("{name}{}", gr.label.to_uppercase());
                p.text_in(head, &label, Rect::new(gx, r.min.y, r.max.x, r.max.y), Align::Left, true, if gr.yours { GOLD } else { DIM }, "Nu.GroupName");
                if gr.discount > 0.001 {
                    p.text_in(head, &format!("-{:.0}%", gr.discount * 100.0), Rect::new(r.min.x, r.min.y, r.max.x - pad, r.max.y), Align::Right, true, GREEN, "Nu.GroupDiscount");
                }
                p.fill(Rect::new(gx, r.max.y - 1.0 * u, r.max.x, r.max.y), PANEL_EDGE, "Nu.GroupRule");
                p.hit(&format!("nu.group:{gi}"), r);
            }
            Line::Row(ri) => {
                let row = &m.rows[ri];
                let it = &cat.items[row.item];
                let sel = nu.focus == Focus::Shop && nu.shop == ri;
                if sel {
                    p.fill(r, SELECT, "Nu.ShopSel");
                    p.fill(Rect::new(r.min.x, r.min.y, r.min.x + 3.0 * u, r.max.y), RED, "Nu.ShopSel");
                } else if nu.shop == ri {
                    p.fill(r, HOVER, "Nu.ShopSel");
                } else if p.hover(r) {
                    p.fill(r, HOVER, "Nu.ShopHover");
                }
                p.hit(&format!("nu.shop:{ri}"), r);
                let (name_c, price_c, weight_c, tag): (Rgba, Rgba, Rgba, String) = match row.state {
                    RowState::CanBuy => (BONE, GREEN, DIM, String::new()),
                    RowState::Owned => (FAINT, FAINT, FAINT, "OWNED".into()),
                    RowState::TooExpensive => (DIM, RED, DIM, format!("NEED {}", money((row.price_exact - dosh).ceil() as i32))),
                    RowState::TooHeavy => (DIM, if dosh < row.price_exact { RED } else { DIM }, AMBER, "TOO HEAVY".into()),
                };
                let wx = r.max.x - pad - weight_w;
                let px = wx - pad * 2.0 - price_w;
                let tx = px - pad * 2.0 - tag_w;
                let nx = r.min.x + pad * 1.5;
                p.text_in(body, &fit(p, body, &it.name, tx - nx - pad), Rect::new(nx, r.min.y, tx, r.max.y), Align::Left, true, name_c, "Nu.ShopName");
                if !tag.is_empty() {
                    p.text_in(small, &tag, Rect::new(tx, r.min.y, px - pad, r.max.y), Align::Right, true, if row.state == RowState::TooHeavy { AMBER } else if row.state == RowState::Owned { DIM } else { RED }, "Nu.ShopTag");
                }
                let price = if row.state == RowState::Owned { String::new() } else { money(row.price) };
                p.text_in(body, &price, Rect::new(px, r.min.y, wx - pad * 1.5, r.max.y), Align::Right, true, price_c, "Nu.ShopPrice");
                p.text_in(small, &kg(row.weight), Rect::new(wx, r.min.y, r.max.x - pad, r.max.y), Align::Right, true, weight_c, "Nu.ShopWeight");
            }
        }
    }
    // A scroll bar when the list is longer than the panel.
    if m.lines.len() > visible {
        let track = Rect::new(shop.max.x - 4.0 * u, sa.min.y, shop.max.x - 1.0 * u, sa.max.y);
        p.fill(track, TRACK, "Nu.Scroll");
        let h = track.height() * visible as f32 / m.lines.len() as f32;
        let t0 = track.min.y + track.height() * nu.scroll as f32 / m.lines.len() as f32;
        p.fill(Rect::new(track.min.x, t0, track.max.x, t0 + h), RULE, "Nu.Scroll");
    }

    // ---- Details.
    let da = title(p, det, "SELECTED", false, "Nu.Det");
    details(p, nu, da, cat, inv, vet, armour, &m, dosh, (small, body, head), pad, u);

    // Clicks go to the input system next frame.
    nu.hits = p.hits.clone();
    if nu.logged_size != Vec2::new(sw, sh) {
        nu.logged_size = Vec2::new(sw, sh);
        runlog::kv(
            "numenu_layout",
            &format!(
                "screen={sw:.0}x{sh:.0} fonts={small},{body},{head},{big} line_heights={lh_s:.0},{lh_b:.0},{lh_h:.0},{lh_big:.0} top=({:.0},{:.0})-({:.0},{:.0}) gear=({:.0},{:.0})-({:.0},{:.0}) shop=({:.0},{:.0})-({:.0},{:.0}) details=({:.0},{:.0})-({:.0},{:.0}) shop_lines={} visible={visible} quads={}",
                top.min.x, top.min.y, top.max.x, top.max.y, gear.min.x, gear.min.y, gear.max.x, gear.max.y, shop.min.x, shop.min.y, shop.max.x, shop.max.y, det.min.x, det.min.y, det.max.x, det.max.y,
                m.lines.len(),
                p.canvas.quads.len()
            ),
        );
    }
}

/// The details panel for the selection.
#[allow(clippy::too_many_arguments)]
fn details(
    p: &mut Painter,
    nu: &NuMenu,
    a: Rect,
    cat: &ShopCatalogue,
    inv: &ShopInventory,
    vet: &Veterancy,
    armour: &Armour,
    m: &Model,
    dosh: f32,
    (small, body, head): (&str, &str, &str),
    pad: f32,
    u: f32,
) {
    let (lh_s, lh_b, lh_h) = (p.line_height(small), p.line_height(body), p.line_height(head));
    let mut y = a.min.y;
    let btn_h = (lh_b * 1.6).round();
    // What is selected: a shop row, an owned weapon, or the armour.
    let (item, row, owned) = match nu.focus {
        Focus::Shop => match m.rows.get(nu.shop) {
            Some(r) => (Some(&cat.items[r.item]), Some(r), inv.owned.iter().position(|o| o.weapon.eq_ignore_ascii_case(&cat.items[r.item].weapon))),
            None => (None, None, None),
        },
        Focus::Gear => match inv.owned.get(nu.gear) {
            Some(o) => (cat.item(&o.weapon), m.rows.iter().find(|r| cat.items[r.item].weapon.eq_ignore_ascii_case(&o.weapon)), Some(nu.gear)),
            None => (None, None, None),
        },
    };
    if nu.focus == Focus::Gear && nu.gear >= inv.owned.len() {
        // The armour.
        let (points, fill) = crate::player::armour::menu_row(armour, vet.vet.cost_scaling("vest"));
        p.text(head, "Combat armour", Vec2::new(a.min.x, y), BONE, "Nu.DetName");
        y += lh_h + pad;
        let scaling = vet.vet.cost_scaling("vest");
        let line = if scaling < 1.0 { format!("Your perk: {:.0}% off armour", (1.0 - scaling) * 100.0) } else { "No perk discount on armour".into() };
        p.text(small, &line, Vec2::new(a.min.x, y), if scaling < 1.0 { GREEN } else { DIM }, "Nu.DetGroup");
        y += lh_s + pad * 2.0;
        p.text(body, &format!("Armour {points} / 100"), Vec2::new(a.min.x, y), BONE, "Nu.DetArmour");
        y += lh_b + pad * 0.5;
        bar(p, Rect::new(a.min.x, y, a.max.x, y + lh_s * 0.6), points as f32 / 100.0, [110, 150, 200, 255], "Nu.DetArmourBar");
        y += lh_s + pad;
        let text = if points >= 100 { "Full.".to_string() } else { format!("Fill to 100: {}   (dosh after: {})", money(fill), money((dosh - fill as f32).max(0.0) as i32)) };
        p.text(body, &text, Vec2::new(a.min.x, y), if points < 100 && dosh < fill as f32 { RED } else { BONE }, "Nu.DetArmourFill");
        let b = Rect::new(a.min.x, a.max.y - btn_h, a.max.x, a.max.y);
        button(p, "nu.armour", b, &format!("{} ARMOUR  {}", if points > 0 { "FILL" } else { "BUY" }, money(fill)), "Enter / V", points < 100, true, body, small);
        return;
    }
    let Some(it) = item else {
        p.text(body, "Nothing selected.", Vec2::new(a.min.x, y), DIM, "Nu.DetNone");
        return;
    };
    let name = owned.and_then(|i| inv.owned.get(i)).map_or(it.name.clone(), |o| o.name.clone());
    p.text(head, &fit(p, head, &name, a.width()), Vec2::new(a.min.x, y), BONE, "Nu.DetName");
    y += lh_h + pad * 0.5;
    if let Some(r) = row {
        let g = &m.groups[r.group];
        let mut line = g.label.to_string();
        if r.scaling < 1.0 {
            line += &format!("   your perk: -{:.0}%  (was {})", (1.0 - r.scaling) * 100.0, money(it.cost));
        }
        p.text(small, &line, Vec2::new(a.min.x, y), if r.scaling < 1.0 { GREEN } else { DIM }, "Nu.DetGroup");
    }
    y += lh_s + pad * 1.5;
    // The comparable stats: bars against the shop's largest, square root.
    let sellable: Vec<&crate::game::buy_menu::ShopItem> = m.rows.iter().map(|r| &cat.items[r.item]).collect();
    let max_of = |f: &dyn Fn(&crate::game::buy_menu::ShopItem) -> f32| sellable.iter().map(|i| f(i)).fold(0.0f32, f32::max).max(1.0);
    let st = &it.stats;
    let price = row.map_or(it.cost, |r| r.price);
    let stats: [(&str, f32, f32, String); 6] = [
        ("Damage", st.shot_damage(), max_of(&|i| i.stats.shot_damage()), if st.per_fire > 1 { format!("{:.0} x {}", st.damage, st.per_fire) } else { format!("{:.0}", st.damage) }),
        ("Fire rate", st.shots_per_second(), max_of(&|i| i.stats.shots_per_second()), if st.fire_rate > 0.0 { format!("{:.1} /s", st.shots_per_second()) } else { "-".into() }),
        ("Magazine", st.mag as f32, max_of(&|i| i.stats.mag as f32), if st.melee || st.mag == 0 { "-".into() } else { st.mag.to_string() }),
        ("Total ammo", st.max_ammo as f32, max_of(&|i| i.stats.max_ammo as f32), if st.melee || st.max_ammo == 0 { "-".into() } else { st.max_ammo.to_string() }),
        ("Weight", it.weight, max_of(&|i| i.weight), kg(row.map_or(it.weight, |r| r.weight))),
        ("Price", price as f32, max_of(&|i| i.cost as f32), money(price)),
    ];
    let label_w = p.text_size(small, "Total ammo").x + pad;
    let value_w = p.text_size(small, "0000 x 00").x + pad;
    let sh = (lh_b * 1.25).round();
    for (label, v, max, text) in stats {
        p.text_in(small, label, Rect::new(a.min.x, y, a.min.x + label_w, y + sh), Align::Left, true, DIM, "Nu.DetStat");
        let br = Rect::new(a.min.x + label_w, y + sh * 0.3, a.max.x - value_w, y + sh * 0.7);
        bar(p, br, (v / max).max(0.0).sqrt(), [175, 160, 140, 255], "Nu.DetStatBar");
        p.text_in(small, &text, Rect::new(br.max.x, y, a.max.x, y + sh), Align::Right, true, BONE, "Nu.DetStatValue");
        y += sh;
    }
    y += pad;
    p.fill(Rect::new(a.min.x, y, a.max.x, y + 1.0 * u), PANEL_EDGE, "Nu.DetRule");
    y += pad;
    let bottom = a.max.y;
    match owned.and_then(|i| inv.owned.get(i)) {
        None => {
            // If you buy it: dosh and carry weight before -> after.
            let r = row.expect("a shop row");
            p.text(body, "If you buy it", Vec2::new(a.min.x, y), DIM, "Nu.DetAfter");
            y += lh_b + pad * 0.5;
            let after = dosh - r.price_exact;
            p.text(body, &format!("Dosh    {}  ->  {}", money(dosh as i32), money(after.floor() as i32)), Vec2::new(a.min.x, y), if after < 0.0 { RED } else { BONE }, "Nu.DetDosh");
            y += lh_b + pad * 0.3;
            let wa = inv.weight + r.weight;
            p.text(body, &format!("Carry   {}  ->  {} / {}", kg(inv.weight), kg(wa), kg(inv.max_weight)), Vec2::new(a.min.x, y), if wa > inv.max_weight { AMBER } else { BONE }, "Nu.DetCarry");
            y += lh_b + pad * 0.3;
            let why = match r.state {
                RowState::CanBuy => ("You can buy this.".to_string(), GREEN),
                RowState::TooHeavy => (format!("Too heavy: sell {} first.", kg(wa - inv.max_weight)), AMBER),
                RowState::TooExpensive => (format!("You need {} more.", money((r.price_exact - dosh).ceil() as i32)), RED),
                RowState::Owned => (String::new(), DIM),
            };
            p.text(body, &why.0, Vec2::new(a.min.x, y), why.1, "Nu.DetWhy");
            let b = Rect::new(a.min.x, bottom - btn_h, a.max.x, bottom);
            button(p, "nu.buy", b, &format!("BUY  {}", money(r.price)), "Enter / B", true, r.state == RowState::CanBuy, body, small);
        }
        Some(o) => {
            p.text(body, "You own this.", Vec2::new(a.min.x, y), GOLD, "Nu.DetOwned");
            y += lh_b + pad * 0.5;
            let info = ammo_info(cat, o);
            if let Some((t, max, clip, fill)) = info {
                p.text(body, &format!("Ammo  {t} / {max}"), Vec2::new(a.min.x, y), BONE, "Nu.DetAmmo");
                y += lh_b + pad * 0.3;
                p.text(small, &format!("One magazine {}   fill {}", money(clip), money(fill)), Vec2::new(a.min.x, y), DIM, "Nu.DetAmmoPrices");
                y += lh_s + pad * 0.3;
            }
            if let Some((cur, max)) = o.alt_ammo {
                p.text(small, &format!("Second ammo  {cur} / {max}   fill {}", money(alt_fill(cat, o).unwrap_or(0))), Vec2::new(a.min.x, y), DIM, "Nu.DetAlt");
                y += lh_s + pad * 0.3;
            }
            let sell = if o.sellable { format!("Sells for {}  (dosh after: {})", money(o.sell_value), money(dosh as i32 + o.sell_value)) } else { "Cannot be sold.".into() };
            p.text(body, &sell, Vec2::new(a.min.x, y), if o.sellable { BONE } else { FAINT }, "Nu.DetSell");
            // Buttons: sell, fill, one magazine.
            let full = info.is_none_or(|(t, max, _, _)| t >= max);
            let half = (a.width() - pad) / 2.0;
            let b1 = Rect::new(a.min.x, bottom - btn_h * 2.0 - pad, a.max.x, bottom - btn_h - pad);
            let fill_cap = match info {
                Some((t, max, _, f)) if t < max => format!("FILL AMMO  {}", money(f)),
                Some(_) => "AMMO FULL".into(),
                None => "NO AMMO".into(),
            };
            button(p, "nu.fill", b1, &fill_cap, "R", !full, false, body, small);
            let b2 = Rect::new(a.min.x, bottom - btn_h, a.min.x + half, bottom);
            button(p, "nu.sell", b2, &if o.sellable { format!("SELL  {}", money(o.sell_value)) } else { "SELL".into() }, "S", o.sellable, false, body, small);
            let b3 = Rect::new(a.min.x + half + pad, bottom - btn_h, a.max.x, bottom);
            let clip_cap = info.map_or("+1 MAG".into(), |(_, _, c, _)| format!("+1 MAG  {}", money(c)));
            button(p, "nu.clip", b3, &clip_cap, "C", !full, false, body, small);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::buy_menu::{ShopItem, ShopStats};

    fn item(weapon: &str, cost: i32, weight: f32) -> ShopItem {
        ShopItem {
            weapon: weapon.into(),
            pickup: format!("{weapon}Pickup"),
            name: weapon.into(),
            cost,
            ammo_cost: 10,
            weight,
            buy_clip_size: 0,
            never_throw: weapon == "KFMod.Single",
            stats: ShopStats::default(),
            info: Default::default(),
        }
    }

    fn owned(weapon: &str, ammo: Option<(u32, u32, u32)>) -> OwnedWeapon {
        OwnedWeapon { weapon: weapon.into(), name: weapon.into(), sell_value: 0, sellable: true, ammo, alt_ammo: None, ammo_scale: 1.0, mag_mod: 1.0 }
    }

    fn cat() -> ShopCatalogue {
        ShopCatalogue {
            items: vec![
                item("KFMod.MP7MMedicGun", 825, 3.0),
                item("KFMod.Shotgun", 500, 8.0),
                item("KFMod.AA12AutoShotgun", 4000, 10.0),
                item("KFMod.Single", 0, 0.0),
                item("KFMod.Deagle", 500, 2.0),
                item("KFMod.DualDeagle", 1000, 4.0),
            ],
            lists: vec![("Medic", vec![0]), ("Support", vec![1, 2]), ("Sharpshooter", vec![3, 4, 5])],
        }
    }

    #[test]
    fn your_perk_group_first_with_states() {
        let c = cat();
        let inv = ShopInventory { owned: vec![owned("KFMod.Deagle", None)], weight: 6.0, max_weight: 15.0 };
        let vet = Vet { perk: Some(Perk::Support), level: 3 };
        let m = model(&c, &inv, &vet, 1000.0);
        let labels: Vec<_> = m.groups.iter().map(|g| g.label).collect();
        assert_eq!(labels, ["Support", "Medic", "Sharpshooter"]);
        assert!(m.groups[0].yours && !m.groups[1].yours);
        // Support level 3: 0.9 - 0.3 = 0.6 of the price.
        assert!((m.groups[0].discount - 0.4).abs() < 0.01);
        let state = |w: &str| m.rows.iter().find(|r| c.items[r.item].weapon == w).map(|r| (r.price, r.state));
        // Shotgun: 500 x 0.6 is 299 (0.9 - 0.3 is just under 0.6 in 32-bit
        // floats, as in KF; the game charges 299.99997), 8 kg fits (6 + 8).
        assert_eq!(state("KFMod.Shotgun"), Some((299, RowState::CanBuy)));
        // AA12 (this test's made-up pickup name gets no discount): 4000 >
        // 1000 dosh, and 10 kg does not fit: too heavy wins.
        assert_eq!(state("KFMod.AA12AutoShotgun"), Some((4000, RowState::TooHeavy)));
        // Deagle owned; its duals at half price and weight; the 9mm never listed.
        assert_eq!(state("KFMod.Deagle"), Some((500, RowState::Owned)));
        assert_eq!(state("KFMod.DualDeagle"), Some((500, RowState::CanBuy)));
        assert_eq!(state("KFMod.Single"), None);
        assert_eq!(m.lines.len(), m.rows.len() + m.groups.len());
        assert_eq!(m.lines[0], Line::Header(0));
    }

    #[test]
    fn too_expensive_and_hidden_single() {
        let c = cat();
        let inv = ShopInventory { owned: vec![owned("KFMod.DualDeagle", None)], weight: 4.0, max_weight: 15.0 };
        let m = model(&c, &inv, &Vet::NONE, 400.0);
        let state = |w: &str| m.rows.iter().find(|r| c.items[r.item].weapon == w).map(|r| r.state);
        assert_eq!(state("KFMod.Shotgun"), Some(RowState::TooExpensive));
        assert_eq!(state("KFMod.Deagle"), None);
        // No perk: KF's order, nothing starred, no discount.
        assert_eq!(m.groups[0].label, "Medic");
        assert!(m.groups.iter().all(|g| !g.yours && g.discount == 0.0));
    }

    #[test]
    fn fill_all_sums_every_missing_ammo() {
        let mut c = cat();
        c.items.push(ShopItem { ammo_cost: 15, ..item("KFMod.Shotgun2", 500, 8.0) });
        let inv = ShopInventory {
            owned: vec![owned("KFMod.Shotgun2", Some((64, 80, 8))), owned("KFMod.MP7MMedicGun", Some((400, 400, 20))), owned("KFMod.Deagle", Some((30, 40, 8)))],
            weight: 13.0,
            max_weight: 15.0,
        };
        let (reqs, total) = fill_all_plan(&c, &inv);
        // Shotgun2: 16 / 8 x 15 = 30; MP7 full; Deagle: int(10 x 10 / 8) = 12.
        assert_eq!(reqs.len(), 2);
        assert_eq!(total, 42);
    }

    #[test]
    fn click_ids_map_to_commands() {
        assert_eq!(click_cmd("nu.buy"), Some(Cmd::Buy));
        assert_eq!(click_cmd("nu.shop:7"), Some(Cmd::SelectShop(7)));
        assert_eq!(click_cmd("nu.perk:6"), Some(Cmd::Perk(Perk::Demolitions)));
        assert_eq!(click_cmd("nu.perk:7"), None);
        assert_eq!(click_cmd("lobby.ready"), None);
    }
}
