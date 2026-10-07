//! Tossing dosh and dropping weapons, the inventory side (KFPawn.TossCash,
//! PlayerController.ThrowWeapon, KFWeapon / Dualies.DropFrom, Pawn.Died,
//! KFHumanPawn.VeterancyChanged). The pickup core (game/pickups) makes the
//! pickup and answers with `DropDone`; only then does the cash or the
//! weapon leave the inventory (at once in single player and on a host,
//! after the host's answer on a client). docs/DESIGN.md, "Tossed dosh and
//! dropped weapons".

use std::collections::HashMap;

use super::*;
use crate::game::pickups::drop::velocity;
use crate::game::pickups::{CarriedWeapon, DropDone, DropItem, DropRequest, DropWhy, PickupGives};
use crate::player::walk::{Walker, kf};

/// KFPawn.TossCash: "if( Amount<=0 ) Amount = 50" (User.ini `B=TossCash`
/// gives no amount).
const TOSS_AMOUNT: i32 = 50;
/// KFPawn.TossCash spawns class'CashPickup'.
const CASH_PICKUP: &str = "KFMod.CashPickup";

/// The dual pistols' DropFrom: the pistol a living player keeps, the
/// pickup class dropped (None: the duals' own PickupClass) and the single
/// pistol's Weight (for the perk drop's weight count).
const DUAL_DROPS: [(&str, &str, Option<&str>, f32); 4] = [
    ("KFMod.Dualies", "KFMod.Single", None, 0.0),
    ("KFMod.DualDeagle", "KFMod.Deagle", Some("KFMod.DeaglePickup"), 2.0),
    ("KFMod.Dual44Magnum", "KFMod.Magnum44Pistol", Some("KFMod.Magnum44Pickup"), 2.0),
    ("KFMod.DualMK23Pistol", "KFMod.MK23Pistol", Some("KFMod.MK23Pickup"), 2.0),
];

fn dual_drop(class: &str) -> Option<(&'static str, Option<&'static str>, f32)> {
    DUAL_DROPS.iter().find(|(d, ..)| d.eq_ignore_ascii_case(class)).map(|(_, s, p, w)| (*s, *p, *w))
}

/// Drops asked for and not answered yet.
#[derive(Resource, Default)]
pub struct WeaponDrops {
    next_token: u32,
    pending: HashMap<u32, Pending>,
}

enum Pending {
    Cash { amount: i32 },
    Weapon { class: String, why: DropWhy, keep: Option<KeptPistol> },
}

/// The dual pistols' DropFrom while alive: the pistol given back, with
/// half the magazine and half the ammo.
#[derive(Clone)]
struct KeptPistol {
    class: String,
    mag: u32,
    total: u32,
}

impl WeaponDrops {
    fn token(&mut self) -> u32 {
        self.next_token += 1;
        self.next_token
    }

    /// Cash waiting for the host's answer (not to be tossed twice).
    fn reserved_cash(&self) -> i32 {
        self.pending.values().map(|p| if let Pending::Cash { amount } = p { *amount } else { 0 }).sum()
    }

    fn dropping(&self, class: &str) -> bool {
        self.pending.values().any(|p| matches!(p, Pending::Weapon { class: c, .. } if c.eq_ignore_ascii_case(class)))
    }
}

/// The pawn, in Unreal units: centre, velocity, view direction, facing
/// (yaw only) and yaw (Unreal rotator units).
struct Pawn {
    centre: Vec3,
    velocity: Vec3,
    view: Vec3,
    facing: Vec3,
    yaw: i32,
}

fn unreal_axes(v: Vec3) -> Vec3 {
    Vec3::new(-v.z, v.x, v.y)
}

pub(super) type CamQuery<'w, 's> = Query<'w, 's, (&'static Transform, Option<&'static Walker>), With<FlyCamera>>;

fn pawn(cam: &CamQuery) -> Option<Pawn> {
    let (t, walker) = cam.single().ok()?;
    let scale = coords::SCALE;
    let centre_b = walker.map_or(t.translation - Vec3::Y * kf::EYE_HEIGHT * scale, |w| w.center);
    let velocity = walker.map_or(Vec3::ZERO, |w| unreal_axes(w.velocity) / scale);
    let view = unreal_axes(t.rotation * Vec3::NEG_Z).normalize_or_zero();
    let facing = Vec3::new(view.x, view.y, 0.0).normalize_or(Vec3::X);
    let yaw = (facing.y.atan2(facing.x) * 65536.0 / std::f32::consts::TAU).round() as i32;
    Some(Pawn { centre: unreal_axes(centre_b) / scale, velocity, view, facing, yaw })
}

/// The best weapon to bring up after a throw (ClientSwitchToBestWeapon;
/// our guess: the highest Priority one that can be held).
fn best_weapon(w: &Weapons) -> Option<usize> {
    let s = slots(&w.defs);
    (0..w.defs.len()).filter(|&i| s[i].selectable).max_by_key(|&i| (w.defs[i].priority, std::cmp::Reverse(i)))
}

/// KFWeapon.CanThrow: not bKFNeverThrow, not reloading, ready (not
/// switching), no shot waiting (NextFireTime), not mid-throw of a grenade.
fn can_throw(w: &Weapons) -> Result<usize, &'static str> {
    let i = w.current;
    let d = w.defs.get(i).ok_or("no_weapon")?;
    if d.gone {
        return Err("no_weapon");
    }
    if d.never_throw {
        return Err("never_throw");
    }
    if w.action != Action::Idle {
        return Err("busy");
    }
    if w.firing.iter().any(|f| *f) || w.fire_cooldown.iter().any(|c| *c > 0.0) {
        return Err("firing");
    }
    Ok(i)
}

/// The request for dropping weapon `i` (KFWeaponPickup.InitDroppedPickupFor,
/// Dualies.DropFrom), and what the inventory keeps.
fn weapon_request(w: &Weapons, i: usize, alive: bool) -> Option<(String, PickupGives, Option<KeptPistol>, bool)> {
    let d = &w.defs[i];
    let dual = dual_drop(&d.class);
    let class = match dual {
        Some((_, Some(p), _)) => p.to_string(),
        _ => d.pickup_class.clone()?,
    };
    let (mut mag, mut total) = d.ammo.map_or((0, 0), |a| (a.mag, a.mag + a.spare));
    let mut keep = None;
    if let Some((single, _, _)) = dual
        && alive
    {
        // Dualies.DropFrom: OtherAmmo = AmmoThrown / 2 to the pistol kept,
        // its MagAmmoRemaining = MagAmmoRemaining / 2.
        let other = total / 2;
        let kept_mag = mag / 2;
        total -= other;
        mag = mag.saturating_sub(kept_mag);
        keep = Some(KeptPistol { class: single.to_string(), mag: kept_mag, total: other });
    }
    let carried = CarriedWeapon { mag, total, sell_value: d.sell_value, thrown: alive, alt: d.alt_ammo.filter(|a| a.1 > 0).map(|a| a.0) };
    // The pickup core puts the pickup class's own InventoryType and Weight here.
    let gives = PickupGives::Weapon { weapon: d.class.clone(), weight: d.weight, carried: Some(carried) };
    // KFWeapon.DropFrom adds the facing x 100; the dual pistols' does not.
    Some((class, gives, keep, dual.is_none()))
}

/// Sends a weapon drop: (start, velocity) from the pawn.
#[allow(clippy::too_many_arguments)]
fn send_weapon_drop(w: &Weapons, drops: &mut WeaponDrops, out: &mut MessageWriter<DropItem>, i: usize, why: DropWhy, p: &Pawn, alive: bool, start: Option<Vec3>) -> bool {
    let Some((class, gives, keep, plus_facing)) = weapon_request(w, i, alive) else {
        runlog::kv("drop_refused", &format!("why={why:?} weapon={} reason=no_pickup_class", w.defs[i].class));
        return false;
    };
    let v = match why {
        DropWhy::Throw => velocity::throw_weapon(p.view, p.facing, p.velocity, plus_facing),
        DropWhy::Death => velocity::death(p.view, p.facing, p.velocity, plus_facing),
        _ => velocity::perk(p.facing, p.velocity, plus_facing),
    };
    let start = start.unwrap_or_else(|| velocity::start(p.centre, p.facing, kf::RADIUS));
    let token = drops.token();
    drops.pending.insert(token, Pending::Weapon { class: w.defs[i].class.clone(), why, keep });
    out.write(DropItem(DropRequest { token, class, gives, origin: p.centre.to_array(), start: start.to_array(), velocity: v.to_array(), yaw: p.yaw, why }));
    true
}

/// The keys (User.ini `B=TossCash`, `Backslash=ThrowWeapon`; test inputs
/// `toss_cash`, `throw_weapon`) and the death drop (Pawn.Died: the weapon
/// in hand).
#[allow(clippy::too_many_arguments)] // Bevy system parameters
pub(super) fn drop_input(
    keys: Res<ButtonInput<KeyCode>>,
    (script, frames): (Res<ScriptedInput>, Res<bevy::diagnostic::FrameCount>),
    weapons: Option<Res<Weapons>>,
    mut drops: ResMut<WeaponDrops>,
    dosh: Res<crate::game::dosh::Dosh>,
    health: Res<crate::game::combat::PlayerHealth>,
    (menus, buy_menu): (Res<crate::game::menus::MenuState>, Res<crate::game::buy_menu::BuyMenu>),
    match_over: Option<Res<crate::game::end_game::MatchOver>>,
    cam: CamQuery,
    mut out: MessageWriter<DropItem>,
    mut was_dead: Local<bool>,
) {
    let scripted = |action: &str| script.0.iter().any(|(f, a)| *f == frames.0 && a == action);
    let typing = !menus.stack.is_empty() || buy_menu.open;
    let toss = (keys.just_pressed(KeyCode::KeyB) && !typing) || scripted("toss_cash");
    let throw = (keys.just_pressed(KeyCode::Backslash) && !typing) || scripted("throw_weapon");
    let died = health.dead && !*was_dead;
    *was_dead = health.dead;
    let Some(w) = weapons else { return };
    let Some(p) = pawn(&cam) else { return };
    if died {
        // Pawn.Died: TossWeapon for the weapon in hand (only bCanThrow is
        // checked, which KF's weapons leave true).
        let i = w.current;
        if w.defs.get(i).is_some_and(|d| !d.gone) && !drops.dropping(&w.defs[i].class) {
            send_weapon_drop(&w, &mut drops, &mut out, i, DropWhy::Death, &p, false, None);
        }
        return;
    }
    let blocked = if health.dead {
        Some("dead")
    } else if menus.lobby_open() {
        Some("lobby")
    } else if match_over.is_some_and(|m| m.active()) {
        Some("match_over")
    } else {
        None
    };
    if let Some(why) = blocked {
        if toss || throw {
            runlog::kv("drop_refused", &format!("toss={toss} throw={throw} reason={why}"));
        }
        return;
    }
    if toss {
        // KFPawn.TossCash: Score = int(Score); nothing at 0 or less;
        // Amount = Min(50, Score).
        let cash = dosh.score.trunc() as i32 - drops.reserved_cash();
        if cash <= 0 {
            runlog::kv("drop_refused", &format!("why=Toss reason=no_cash dosh={:.0} reserved={}", dosh.score, drops.reserved_cash()));
        } else {
            let amount = TOSS_AMOUNT.min(cash);
            let token = drops.token();
            drops.pending.insert(token, Pending::Cash { amount });
            let start = velocity::start(p.centre, p.facing, kf::RADIUS);
            let v = velocity::toss_cash(p.view, p.velocity);
            out.write(DropItem(DropRequest {
                token,
                class: CASH_PICKUP.into(),
                gives: PickupGives::Cash { amount },
                origin: p.centre.to_array(),
                start: start.to_array(),
                velocity: v.to_array(),
                yaw: p.yaw,
                why: DropWhy::Toss,
            }));
        }
    }
    if throw {
        match can_throw(&w) {
            Ok(i) if drops.dropping(&w.defs[i].class) => runlog::kv("drop_refused", &format!("why=Throw weapon={} reason=already_dropping", w.defs[i].class)),
            Ok(i) => {
                send_weapon_drop(&w, &mut drops, &mut out, i, DropWhy::Throw, &p, true, None);
            }
            Err(why) => runlog::kv("drop_refused", &format!("why=Throw weapon={} reason={why}", w.defs.get(w.current).map_or("none", |d| d.class.as_str()))),
        }
    }
}

/// KFHumanPawn.VeterancyChanged: over the new carry weight, weapons (not
/// bKFNeverThrow, in inventory order) are dropped until the weight fits,
/// each from the pawn's centre + a random 10 units. Returns the classes
/// asked for. (perk.rs calls this.)
pub(super) fn perk_drops(w: &mut Weapons, drops: &mut WeaponDrops, out: &mut MessageWriter<DropItem>, cam: &CamQuery, max: f32) -> Vec<String> {
    let Some(p) = pawn(cam) else { return Vec::new() };
    let mut weight = carried_weight(w);
    let mut asked = Vec::new();
    for i in 0..w.defs.len() {
        if weight <= max {
            break;
        }
        let d = &w.defs[i];
        if d.gone || d.never_throw || d.weight <= 0.0 || drops.dropping(&d.class) {
            continue;
        }
        // Dualies.DropFrom gives a pistol back.
        let back = dual_drop(&d.class).map_or(0.0, |(_, _, single_weight)| single_weight);
        let class = d.class.clone();
        let lost = d.weight - back;
        // VRand() x 10: a random direction.
        let r = Vec3::new(w.random() * 2.0 - 1.0, w.random() * 2.0 - 1.0, w.random() * 2.0 - 1.0).normalize_or(Vec3::X) * 10.0;
        if send_weapon_drop(w, drops, out, i, DropWhy::Perk, &p, true, Some(p.centre + r)) {
            weight -= lost;
            asked.push(class);
        }
    }
    asked
}

/// The pickup core's answers: the cash or the weapon leaves the
/// inventory now (the dual pistols leave one pistol), the best weapon
/// comes up after a throw.
#[allow(clippy::too_many_arguments)] // Bevy system parameters
pub(super) fn drop_results(
    mut done: MessageReader<DropDone>,
    mut drops: ResMut<WeaponDrops>,
    weapons: Option<ResMut<Weapons>>,
    assets: NonSend<WeaponAssets>,
    mut commands: Commands,
    (mut meshes, mut images, mut materials): MeshAssets,
    mut dosh: ResMut<crate::game::dosh::Dosh>,
    health: Res<crate::game::combat::PlayerHealth>,
    effects: Option<ResMut<WeaponEffects>>,
) {
    let mut weapons = weapons;
    let mut changed = false;
    for d in done.read() {
        let Some(pending) = drops.pending.remove(&d.token) else { continue };
        let Some(id) = d.id else {
            let what = match &pending {
                Pending::Cash { amount } => format!("cash={amount}"),
                Pending::Weapon { class, why, .. } => format!("weapon={class} why={why:?}"),
            };
            runlog::kv("drop_refused", &format!("token={} {what} reason={}", d.token, d.reason));
            continue;
        };
        match pending {
            Pending::Cash { amount } => {
                let before = dosh.score;
                dosh.score = (dosh.score.trunc() - amount as f32).max(0.0);
                runlog::kv("dosh", &format!("reason=toss amount=-{amount} total={:.0} pickup={id}", dosh.score));
                runlog::kv("drop_done", &format!("token={} pickup={id} cash={amount} dosh={before:.0}->{:.0}", d.token, dosh.score));
            }
            Pending::Weapon { class, why, keep } => {
                let Some(w) = weapons.as_deref_mut() else { continue };
                let Some(i) = owned_index(w, &class) else {
                    runlog::kv("drop_done", &format!("token={} pickup={id} weapon={class} why={why:?} note=no_longer_owned", d.token));
                    continue;
                };
                let was_current = i == w.current || matches!(w.action, Action::PutDown { next } if next == i);
                let ammo = w.defs[i].ammo.map_or("none".into(), |a| format!("{}+{}", a.mag, a.spare));
                let sell = w.defs[i].sell_value;
                w.defs[i].gone = true;
                if was_current {
                    w.firing = [false; 2];
                }
                // Dualies.DropFrom: the pistol kept, half the ammo.
                let mut kept = String::new();
                if let Some(k) = keep {
                    match give_weapon(w, &k.class, &assets, &mut commands, &mut meshes, &mut images, &mut materials) {
                        Ok(si) => {
                            if let Some(a) = w.defs[si].ammo.as_mut() {
                                let total = k.total.min(a.max_total);
                                a.mag = k.mag.min(a.capacity).min(total);
                                a.spare = total - a.mag;
                            }
                            w.defs[si].sell_value = None;
                            kept = format!(" kept={}:{}+{}", k.class, w.defs[si].ammo.map_or(0, |a| a.mag), w.defs[si].ammo.map_or(0, |a| a.spare));
                        }
                        Err(e) => runlog::kv("drop_warning", &format!("single={} load_failed={e}", k.class)),
                    }
                }
                // ClientSwitchToBestWeapon.
                let mut switched = String::new();
                if was_current
                    && !health.dead
                    && let Some(b) = best_weapon(w)
                {
                    switched = format!(" switched_to={}", w.defs[b].class);
                    force_change(w, b);
                }
                changed = true;
                runlog::kv("drop_done", &format!("token={} pickup={id} weapon={class} why={why:?} ammo={ammo} sell_value={sell:?} weight={}{kept}{switched}", d.token, carried_weight(w)));
            }
        }
    }
    if changed
        && let (Some(w), Some(mut fx)) = (weapons.as_deref(), effects)
    {
        fx.weight_speed_mult = weight_speed_mult(carried_weight(w), &w.vet);
    }
}
