//! The inventory and the shop: weight, slot order and switching, giving and removing weapons, the trader's buy / sell / ammo requests.

use super::*;

/// Dualies.GiveTo (given, not picked up): the magazine is the single's
/// plus the single's MagCapacity, up to the duals' capacity; the ammo is
/// the duals' InitialAmount plus all of the single's, up to MaxAmmo
/// (AddAmmo caps). My reading: KF weapons keep ammo as separate items
/// (bNoAmmoInstances false); whether the single's item survives is not
/// checked.
pub(super) fn merge_dual_ammo(single: Ammo, dual: Ammo) -> Ammo {
    let mag = (single.mag + single.capacity).min(dual.capacity);
    let total = (dual.initial + single.mag + single.spare).min(dual.max_total).max(mag);
    Ammo {
        mag,
        spare: total - mag,
        ..dual
    }
}

pub(super) fn slots(defs: &[WeaponDef]) -> Vec<Slot> {
    defs.iter().map(WeaponDef::slot).collect()
}

/// Pawn.AddInventory: a new weapon goes before the first weapon of its
/// group with a lower Priority, or right after the last of its group;
/// otherwise at the back of the list.
pub(super) fn inventory_position(inv: &[Slot], new: Slot) -> usize {
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
pub(super) fn switch_group(inv: &[Slot], current: usize, group: u8) -> Option<usize> {
    let n = inv.len();
    (1..=n)
        .map(|k| (current + k) % n)
        .find(|&i| inv[i].group == group && inv[i].selectable)
}

/// KFWeapon.NextWeapon / PrevWeapon: the next weapon by (InventoryGroup,
/// GroupOffset), wrapping around.
pub(super) fn step_weapon(inv: &[Slot], from: usize, forward: bool) -> Option<usize> {
    let mut order: Vec<usize> = (0..inv.len()).filter(|&i| inv[i].selectable || i == from).collect();
    order.sort_by_key(|&i| (inv[i].group, inv[i].group_offset, i));
    let at = order.iter().position(|&i| i == from)?;
    let n = order.len();
    let next = if forward { order[(at + 1) % n] } else { order[(at + n - 1) % n] };
    (next != from).then_some(next)
}

/// The owned (not sold, not thrown away) copy of a weapon class.
pub(super) fn owned_index(w: &Weapons, class: &str) -> Option<usize> {
    w.defs.iter().position(|d| !d.gone && d.class.eq_ignore_ascii_case(class))
}

/// KFHumanPawn.CurrentWeight: the weights of what is carried.
pub(super) fn carried_weight(w: &Weapons) -> f32 {
    w.defs.iter().filter(|d| !d.gone).map(|d| d.weight).sum()
}

/// After a weapon is inserted at `at`, every stored index at or past it
/// moves up one.
pub(super) fn shift_indices(w: &mut Weapons, at: usize) {
    let bump = |i: &mut usize| {
        if *i >= at {
            *i += 1;
        }
    };
    bump(&mut w.current);
    match &mut w.action {
        Action::PutDown { next } => bump(next),
        Action::Grenade { back_to, .. } => bump(back_to),
        _ => {}
    }
    if let Some((_, i, _)) = w.pending_spawn.as_mut() {
        bump(i);
    }
    if let Some((_, i, _)) = w.pending_inject.as_mut() {
        bump(i);
    }
    match &mut w.quick_heal {
        QuickHeal::Inject { back, .. } | QuickHeal::Back { back, .. } => bump(back),
        QuickHeal::Off => {}
    }
}

/// A fresh copy of `class` in the inventory (its InitialAmount ammo):
/// replaces a sold or thrown-away copy in place, else goes where
/// Pawn.AddInventory puts it. Returns its index.
#[allow(clippy::too_many_arguments)]
pub(super) fn give_weapon(
    w: &mut Weapons,
    class: &str,
    assets: &WeaponAssets,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> Result<usize, String> {
    let Some(set) = assets.0.as_ref() else {
        return Err("no_assets".into());
    };
    let started = std::time::Instant::now();
    let defaults = ClassDefaults::new(set);
    let mut def = load_weapon(set, &defaults, class, meshes, images, materials)?;
    spawn_parts(commands, &mut def, w.camera);
    let i = match w.defs.iter().position(|d| d.gone && d.class.eq_ignore_ascii_case(class)) {
        Some(i) => {
            for &e in &w.defs[i].entities {
                commands.entity(e).despawn();
            }
            w.defs[i] = def;
            i
        }
        None => {
            let at = inventory_position(&slots(&w.defs), def.slot());
            shift_indices(w, at);
            w.defs.insert(at, def);
            at
        }
    };
    runlog::kv("weapon_given", &format!("class={class} index={i} load_seconds={:.2}", started.elapsed().as_secs_f64()));
    Ok(i)
}

/// ClientForceChangeWeapon: bring `next` up (put the current one down
/// first, unless it is gone or busy reloading / throwing).
pub(super) fn force_change(w: &mut Weapons, next: usize) {
    if next == w.current {
        return;
    }
    w.firing = [false; 2];
    if w.defs[w.current].gone || matches!(w.action, Action::Reload | Action::Grenade { .. }) {
        w.current = next;
        set_action(w, Action::Select);
    } else {
        set_action(w, Action::PutDown { next });
    }
}

/// Carries out the buy menu's requests with KFPawn's rules
/// (ServerBuyWeapon, ServerSellWeapon, ServerBuyAmmo) and keeps the
/// menu's view of the inventory current.
#[allow(clippy::too_many_arguments)] // Bevy system parameters
pub(super) fn shop_requests(
    mut requests: MessageReader<crate::game::buy_menu::ShopRequest>,
    weapons: Option<ResMut<Weapons>>,
    assets: NonSend<WeaponAssets>,
    mut commands: Commands,
    (mut meshes, mut images, mut materials): MeshAssets,
    mut dosh: ResMut<crate::game::dosh::Dosh>,
    cat: Res<crate::game::buy_menu::ShopCatalogue>,
    (game, shops): (Res<crate::game::waves::WaveGame>, Res<crate::game::trader::Shops>),
    effects: Option<ResMut<WeaponEffects>>,
    mut inv: ResMut<crate::game::buy_menu::ShopInventory>,
) {
    use crate::game::buy_menu::{DUAL_SINGLES, HALF_PRICE_DUALS, MAX_CARRY_WEIGHT as MAX, ShopRequest};
    let Some(mut w) = weapons else { return };
    let w = &mut *w;
    // CanBuyNow: no wave in progress, touching a shop.
    let can_buy = !matches!(game.phase, crate::game::waves::Phase::Wave | crate::game::waves::Phase::BossWave) && shops.player_inside().is_some();
    let refuse = |what: &str, class: &str, why: &str| runlog::kv("shop_refused", &format!("request={what} weapon={class} reason={why}"));
    let mut changed = false;
    for r in requests.read() {
        match r {
            ShopRequest::Buy(class) => {
                if !can_buy {
                    refuse("buy", class, "not_in_open_shop_time");
                    continue;
                }
                let Some(item) = cat.item(class) else {
                    refuse("buy", class, "not_for_sale");
                    continue;
                };
                // The catalogue's spelling of the class.
                let class = &item.weapon;
                if owned_index(w, class).is_some() {
                    refuse("buy", class, "owned");
                    continue;
                }
                let single_class = DUAL_SINGLES.iter().find(|(d, _)| d.eq_ignore_ascii_case(class)).map(|(_, s)| *s);
                let half = HALF_PRICE_DUALS.iter().any(|(d, s)| d.eq_ignore_ascii_case(class) && owned_index(w, s).is_some());
                let price = if half { item.cost as f32 / 2.0 } else { item.cost as f32 };
                let item_weight = if half { item.weight / 2.0 } else { item.weight };
                let weight = carried_weight(w);
                if item_weight > 0.0 && weight + item_weight > MAX {
                    refuse("buy", class, &format!("too_heavy weight={weight} item_weight={item_weight}"));
                    w.sounds.push(trader_refusal("KF_Trader.TooHeavy"));
                    continue;
                }
                if dosh.score < price {
                    refuse("buy", class, &format!("dosh score={:.0} price={price}", dosh.score));
                    w.sounds.push(trader_refusal("KF_Trader.TooExpensive"));
                    continue;
                }
                let i = match give_weapon(w, class, &assets, &mut commands, &mut meshes, &mut images, &mut materials) {
                    Ok(i) => i,
                    Err(e) => {
                        refuse("buy", class, &format!("load_failed:{e}"));
                        continue;
                    }
                };
                // Dualies.GiveTo: the single goes, its rounds join the duals'.
                let single = single_class.and_then(|s| owned_index(w, s));
                if let Some(si) = single {
                    if let (Some(s_ammo), Some(d_ammo)) = (w.defs[si].ammo, w.defs[i].ammo) {
                        w.defs[i].ammo = Some(merge_dual_ammo(s_ammo, d_ammo));
                    }
                    w.defs[si].gone = true;
                }
                w.defs[i].sell_value = Some(price * 0.75);
                dosh.score -= price;
                changed = true;
                runlog::kv(
                    "shop_buy",
                    &format!("weapon={class} price={price} half={half} weight={} dosh={:.0} replaced_single={}", weight + item_weight, dosh.score, single.is_some()),
                );
                // MakeSomeBuyNoise: Pawn.PlaySound(PickupSound,
                // SLOT_Interface, 255.0, , 120).
                if let Some(snd) = w.defs[i].pickup_sound.clone() {
                    w.sounds.push(PlaySound::new(snd, Emitter::Listener).slot(SoundSlot::Interface).volume(255.0).radius(120.0));
                }
                force_change(w, i);
            }
            ShopRequest::Sell(class) => {
                let Some(i) = owned_index(w, class).filter(|_| can_buy) else {
                    refuse("sell", class, if can_buy { "not_owned" } else { "not_in_open_shop_time" });
                    continue;
                };
                if w.defs[i].never_throw {
                    refuse("sell", class, "never_throw");
                    continue;
                }
                let cost = cat.item(class).map_or(0, |it| it.cost);
                let mut price = w.defs[i].sell_value.unwrap_or((cost as f32 * 0.75).trunc());
                let was_current = i == w.current || matches!(w.action, Action::PutDown { next } if next == i);
                w.defs[i].gone = true;
                // Selling duals gives the single back; except for the 9mm,
                // half the price stays in the single.
                let mut back = None;
                if let Some((_, single)) = DUAL_SINGLES.iter().find(|(d, _)| d.eq_ignore_ascii_case(class)) {
                    match give_weapon(w, single, &assets, &mut commands, &mut meshes, &mut images, &mut materials) {
                        Ok(si) => {
                            if !class.eq_ignore_ascii_case("KFMod.Dualies") {
                                price /= 2.0;
                                w.defs[si].sell_value = Some(price);
                            }
                            back = Some(si);
                        }
                        Err(e) => runlog::kv("shop_warning", &format!("single={single} load_failed={e}")),
                    }
                }
                dosh.score += price;
                changed = true;
                runlog::kv("shop_sell", &format!("weapon={class} price={price} dosh={:.0} single_back={}", dosh.score, back.is_some()));
                // ClientCurrentWeaponSold: something else comes up.
                if was_current {
                    let next = back.or_else(|| step_weapon(&slots(&w.defs), w.current, false)).unwrap_or(0);
                    force_change(w, next);
                }
            }
            ShopRequest::Ammo { weapon, secondary, fill } => {
                let Some(i) = owned_index(w, weapon).filter(|_| can_buy) else {
                    refuse("ammo", weapon, if can_buy { "not_owned" } else { "not_in_open_shop_time" });
                    continue;
                };
                let Some(item) = cat.item(weapon) else {
                    refuse("ammo", weapon, "no_price");
                    continue;
                };
                // (AmmoAmount, MaxAmmo, UsedMagCapacity).
                let (total, max, used) = if *secondary {
                    let Some((cur, max)) = w.defs[i].alt_ammo.filter(|a| a.1 > 0) else {
                        refuse("ammo", weapon, "no_second_ammo");
                        continue;
                    };
                    (cur, max, 1.0)
                } else {
                    let Some(a) = w.defs[i].ammo else {
                        refuse("ammo", weapon, "no_ammo");
                        continue;
                    };
                    let used = if weapon.eq_ignore_ascii_case("KFMod.HuskGun") { item.buy_clip_size.max(1) as f32 } else { a.capacity as f32 };
                    (a.mag + a.spare, a.max_total, used)
                };
                if total >= max {
                    refuse("ammo", weapon, "full");
                    continue;
                }
                let clip_price = item.ammo_cost as f32;
                let mut c = if *fill { (max - total) as f32 } else { used };
                let price = (c / used * clip_price).trunc();
                let paid;
                if dosh.score < price {
                    // Buy what the dosh covers.
                    c = (c * (dosh.score / price)).trunc();
                    if c < 1.0 {
                        refuse("ammo", weapon, "dosh");
                        continue;
                    }
                    paid = c / used * price;
                    dosh.score = (dosh.score - paid).max(0.0);
                } else {
                    paid = price;
                    dosh.score = (dosh.score - price).trunc();
                }
                // Ammunition.AddAmmo: up to MaxAmmo.
                let added = (c as u32).min(max - total);
                if *secondary {
                    if let Some(a) = w.defs[i].alt_ammo.as_mut() {
                        a.0 += added;
                    }
                } else if let Some(a) = w.defs[i].ammo.as_mut() {
                    a.spare += added;
                }
                changed = true;
                runlog::kv(
                    "shop_ammo",
                    &format!("weapon={weapon} secondary={secondary} fill={fill} added={added} paid={paid:.1} total={} max={max} dosh={:.0}", total + added, dosh.score),
                );
            }
        }
    }
    if changed && let Some(mut fx) = effects {
        // KFHumanPawn.ModifyVelocity: the weight counts up to MaxCarryWeight.
        let encumbrance = carried_weight(w).min(MAX_CARRY_WEIGHT) / MAX_CARRY_WEIGHT;
        fx.weight_speed_mult = 1.0 - encumbrance * WEIGHT_SPEED_MODIFIER;
    }
    inv.weight = carried_weight(w);
    inv.owned = w
        .defs
        .iter()
        .filter(|d| !d.gone)
        .map(|d| crate::game::buy_menu::OwnedWeapon {
            weapon: d.class.clone(),
            name: d.item_name.to_string(),
            sell_value: d.sell_value.unwrap_or_else(|| (cat.item(&d.class).map_or(0, |it| it.cost) as f32 * 0.75).trunc()) as i32,
            sellable: !d.never_throw,
            ammo: d.ammo.map(|a| (a.mag + a.spare, a.max_total, a.capacity)),
            alt_ammo: d.alt_ammo.filter(|a| a.1 > 0),
        })
        .collect();
}
