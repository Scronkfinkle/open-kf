//! Taking pickups into the inventory (game/pickups asks with `PickupUse`):
//! KF's touch rules for weapons (KFWeaponPickup.CheckCanCarry,
//! KFWeapon / Single / Dualies.HandlePickupQuery, DeaglePickup.SpawnCopy,
//! KFWeapon.GiveAmmo), ammo boxes (KFAmmoPickup.Touch), vests
//! (KFPawn.AddShieldStrength) and dosh (CashPickup). docs/DESIGN.md,
//! "Pickups".

use super::*;
use crate::game::pickups::{CarriedWeapon, PickupGives, PickupUse, PickupUsed};
use crate::game::hud::{LocalMessage, MessageClass};

/// KFMod.Dualies default Weight (Single.HandlePickupQuery checks it).
const DUALIES_WEIGHT: f32 = 4.0;

/// What a weapon pickup turns into for this inventory.
struct WeaponPlan {
    /// The class given.
    give: String,
    /// The owned single whose rounds join (Dualies.GiveTo, DeaglePickup).
    merge_single: Option<String>,
}

/// Why a pickup is refused, and the KFMainMessages switch shown for it.
fn weapon_plan(w: &Weapons, weapon: &str, weight: f32, max: f32) -> Result<WeaponPlan, (&'static str, Option<u8>)> {
    let carried = carried_weight(w);
    // KFWeaponPickup.Touch: CheckCanCarry(InventoryType's Weight) first.
    if weight > 0.0 && carried + weight > max {
        return Err(("too_heavy", Some(2)));
    }
    let is = |c: &str| weapon.eq_ignore_ascii_case(c);
    let owns = |c: &str| owned_index(w, c).is_some();
    // Dualies.HandlePickupQuery: a 9mm while carrying the duals.
    if is("KFMod.Single") && owns("KFMod.Dualies") {
        return Err(("owned", Some(1)));
    }
    if owns(weapon) {
        // Single.HandlePickupQuery: a second 9mm becomes the duals, if they
        // can be carried. DeaglePickup.SpawnCopy: a second Handcannon gives
        // the Dual Handcannons.
        if is("KFMod.Single") {
            if carried + DUALIES_WEIGHT > max {
                return Err(("too_heavy_for_duals", Some(2)));
            }
            return Ok(WeaponPlan { give: "KFMod.Dualies".into(), merge_single: Some("KFMod.Single".into()) });
        }
        if is("KFMod.Deagle") {
            return Ok(WeaponPlan { give: "KFMod.DualDeagle".into(), merge_single: Some("KFMod.Deagle".into()) });
        }
        // KFWeapon.HandlePickupQuery: "You already have this weapon".
        return Err(("owned", Some(1)));
    }
    // Dualies.GiveTo takes the owned 9mm.
    let merge_single = crate::game::buy_menu::DUAL_SINGLES.iter().find(|(d, s)| is(d) && owns(s)).map(|(_, s)| s.to_string());
    Ok(WeaponPlan { give: weapon.to_string(), merge_single })
}

/// What an ammo box would add: (weapon index, secondary, rounds to add,
/// AmmoPickupAmount was 1). Empty: nothing can take ammo.
fn ammo_plan(w: &Weapons) -> Vec<(usize, bool, u32, bool)> {
    let mut out = Vec::new();
    for (i, d) in w.defs.iter().enumerate() {
        if d.gone {
            continue;
        }
        if let (Some(n), Some(a)) = (d.pickup_amount, d.ammo) {
            let total = a.mag + a.spare;
            if total < a.max_total {
                let m = d.ammo_class.as_ref().map_or(1.0, |c| w.vet.ammo_pickup_mod(c));
                // AmmoPickupAmount = float(amount) x mod, an int.
                let add = if n > 1 { ((n as f32 * m) as u32).min(a.max_total - total) } else { 1 };
                out.push((i, false, add, n <= 1));
            }
        }
        if let (Some(n), Some((cur, max))) = (d.alt_pickup_amount, d.alt_ammo)
            && cur < max
        {
            let m = d.alt_ammo_class.as_ref().map_or(1.0, |c| w.vet.ammo_pickup_mod(c));
            let add = if n > 1 { ((n as f32 * m) as u32).min(max - cur) } else { 1 };
            out.push((i, true, add, n <= 1));
        }
    }
    out
}

/// A dropped weapon's state onto the given copy (fresh, with the perk's
/// starting ammo): KFWeapon.GiveTo takes the pickup's MagAmmoRemaining
/// (up to MagCapacity); GiveAmmo the pickup's AmmoAmount if it was thrown
/// (bThrown), else the fresh ammo; Dualies.GiveTo adds the single's.
fn apply_carried(a: &mut Ammo, c: CarriedWeapon, single: Option<Ammo>) {
    let (extra_mag, extra_total) = single.map_or((0, 0), |s| (s.mag, s.mag + s.spare));
    let base = if c.thrown { c.total } else { a.mag + a.spare };
    let total = (base + extra_total).min(a.max_total);
    let mag = (c.mag + extra_mag).min(a.capacity).min(total);
    a.mag = mag;
    a.spare = total - mag;
}

#[allow(clippy::too_many_arguments)] // Bevy system parameters
pub(super) fn pickup_inventory(
    time: Res<Time>,
    mut uses: MessageReader<PickupUse>,
    mut answers: MessageWriter<PickupUsed>,
    weapons: Option<ResMut<Weapons>>,
    assets: NonSend<WeaponAssets>,
    mut commands: Commands,
    (mut meshes, mut images, mut materials): MeshAssets,
    (mut armour, mut dosh, health): (ResMut<crate::player::armour::Armour>, ResMut<crate::game::dosh::Dosh>, Res<crate::game::combat::PlayerHealth>),
    effects: Option<ResMut<WeaponEffects>>,
    mut messages: MessageWriter<LocalMessage>,
    // KFWeapon.LastHasGunMsgTime, KFWeaponPickup.LastCantCarryTime.
    mut last_msg: Local<[f32; 2]>,
) {
    let mut weapons = weapons;
    let mut effects = effects;
    let now = time.elapsed_secs();
    for u in uses.read() {
        // CashPickup.GetLocalString: "Found (N) Pounds." instead of the
        // class's PickupMessage.
        let message = match u.gives {
            PickupGives::Cash { amount } => Some(format!("Found ({amount}) Pounds.")),
            _ => None,
        };
        let mut answer = |ok: bool, detail: String| {
            answers.write(PickupUsed { id: u.id, class: u.class.clone(), apply: u.apply, ok, detail, message: message.clone() });
        };
        if health.dead {
            answer(false, "dead".into());
            continue;
        }
        match &u.gives {
            PickupGives::Weapon { weapon, weight, carried } => {
                let Some(w) = weapons.as_deref_mut() else {
                    answer(false, "no_weapons".into());
                    continue;
                };
                let max = max_carry_weight(&w.vet);
                let plan = match weapon_plan(w, weapon, *weight, max) {
                    Ok(p) => p,
                    Err((why, msg)) => {
                        // At most every 0.5 s (LastCantCarryTime / LastHasGunMsgTime).
                        if let Some(m) = msg {
                            let k = if m == 2 { 1 } else { 0 };
                            if last_msg[k] < now {
                                last_msg[k] = now + 0.5;
                                messages.write(LocalMessage::new(MessageClass::Main, m));
                            }
                        }
                        answer(false, format!("{why} weapon={weapon} weight={weight} carried={} max={max}", carried_weight(w)));
                        continue;
                    }
                };
                if !u.apply {
                    answer(true, format!("would_give={} merge={:?}", plan.give, plan.merge_single));
                    continue;
                }
                let i = match give_weapon(w, &plan.give, &assets, &mut commands, &mut meshes, &mut images, &mut materials) {
                    Ok(i) => i,
                    Err(e) => {
                        answer(false, format!("load_failed:{e}"));
                        continue;
                    }
                };
                let single = plan.merge_single.as_deref().and_then(|s| owned_index(w, s));
                let single_ammo = single.and_then(|si| w.defs[si].ammo);
                if let Some(a) = w.defs[i].ammo.as_mut() {
                    match (carried, single_ammo) {
                        (Some(c), s) => apply_carried(a, *c, s),
                        (None, Some(s)) => *a = merge_dual_ammo(s, *a),
                        (None, None) => {}
                    }
                }
                // AmmoAmount[1] of a thrown weapon (the M4 203's grenades).
                if let (Some(c), Some(alt)) = (carried.filter(|c| c.thrown), w.defs[i].alt_ammo.as_mut())
                    && let Some(n) = c.alt
                {
                    alt.0 = n.min(alt.1);
                }
                if let Some(si) = single {
                    w.defs[si].gone = true;
                }
                // KFWeaponPickup.Touch: SellValue from the pickup (-1 for a
                // fresh one: sells for 75% of the price).
                w.defs[i].sell_value = carried.and_then(|c| c.sell_value);
                // Weapon.ClientWeaponSet: brought up if rated higher
                // (Controller.RateWeapon: Priority) and not firing.
                let cur = w.current;
                let switch = !w.firing[0] && !w.firing[1] && w.defs.get(cur).is_none_or(|d| d.gone || w.defs[i].priority > d.priority);
                if switch {
                    force_change(w, i);
                }
                if let Some(fx) = effects.as_deref_mut() {
                    fx.weight_speed_mult = weight_speed_mult(carried_weight(w), &w.vet);
                }
                let ammo = w.defs[i].ammo.map_or("none".to_string(), |a| format!("{}+{}", a.mag, a.spare));
                answer(true, format!("weapon={} replaced_single={} ammo={ammo} sell_value={:?} weight={} switched={switch}", plan.give, single.is_some(), w.defs[i].sell_value, carried_weight(w)));
            }
            PickupGives::Ammo => {
                let Some(w) = weapons.as_deref_mut() else {
                    answer(false, "no_weapons".into());
                    continue;
                };
                let plan = ammo_plan(w);
                if plan.is_empty() {
                    answer(false, "ammo_full".into());
                    continue;
                }
                if !u.apply {
                    answer(true, format!("would_fill={}", plan.len()));
                    continue;
                }
                let mut added = Vec::new();
                for (i, secondary, mut add, single_round) in plan {
                    // AmmoPickupAmount 1 (frags, pipe bombs): one round with
                    // chance 1 / GameDifficulty.
                    if single_round && w.random() > 1.0 / crate::game::difficulty::game_difficulty() {
                        add = 0;
                    }
                    let d = &mut w.defs[i];
                    if secondary {
                        if let Some(a) = d.alt_ammo.as_mut() {
                            a.0 = (a.0 + add).min(a.1);
                        }
                    } else if let Some(a) = d.ammo.as_mut() {
                        a.spare += add;
                        // BoomStick.AmmoPickedUp: both barrels empty -> loaded.
                        if d.boomstick_reload.is_some() && a.mag == 0 {
                            let m = a.spare.min(2);
                            a.mag = m;
                            a.spare -= m;
                        }
                    }
                    let total = d.ammo.map_or(0, |a| a.mag + a.spare);
                    added.push(format!("{}{}:+{add}:total={}", d.item_name, if secondary { "(alt)" } else { "" }, if secondary { d.alt_ammo.map_or(0, |a| a.0) } else { total }));
                }
                answer(true, format!("perk={} ammo=[{}]", w.vet.label(), added.join(" ")));
            }
            PickupGives::Armour { amount } => {
                if armour.strength >= crate::player::armour::MAX_ARMOUR {
                    answer(false, format!("armour_full armour={}", armour.strength));
                    continue;
                }
                if !u.apply {
                    answer(true, format!("would_add={amount}"));
                    continue;
                }
                let before = armour.strength;
                armour.strength = (armour.strength + amount).min(crate::player::armour::MAX_ARMOUR);
                answer(true, format!("armour={before}->{}", armour.strength));
            }
            PickupGives::Cash { amount } => {
                if !u.apply {
                    answer(true, format!("would_add={amount}"));
                    continue;
                }
                let before = dosh.score;
                dosh.score += *amount as f32;
                answer(true, format!("dosh={before:.0}->{:.0}", dosh.score));
            }
        }
    }
}
