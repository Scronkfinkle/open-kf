//! The perk's values on the weapons: magazine size (KFWeapon.UpdateMagCapacity),
//! maximum and starting ammo (GiveAmmo, VeterancyChanged), fire rate
//! (the fire modes' ModeDoFire), reload speed (ReloadMeNow), the melee
//! speed bonus (KFHumanPawn.ChangedWeapon) and the carry weight
//! (VeterancyChanged). See DESIGN.md, "Perks".

use super::*;
use crate::game::perks::Vet;

/// KFHumanPawn default GroundSpeed and BaseMeleeIncrease.
const GROUND_SPEED: f32 = 200.0;
const BASE_MELEE_INCREASE: f32 = 0.2;

/// MaxCarryWeight with the perk (VeterancyChanged: 15 + AddCarryMaxWeight).
pub(crate) fn max_carry_weight(vet: &Vet) -> f32 {
    MAX_CARRY_WEIGHT + vet.carry_weight_bonus()
}

/// KFHumanPawn.ModifyVelocity's WeightMod: the weight counts up to MaxCarryWeight.
pub(super) fn weight_speed_mult(weight: f32, vet: &Vet) -> f32 {
    let max = max_carry_weight(vet);
    1.0 - weight.min(max) / max * WEIGHT_SPEED_MODIFIER
}

/// Sets a weapon's perk-dependent values from its defaults and returns
/// what changed (for the log). `fresh`: the weapon was just given
/// (KFWeapon.GiveAmmo: InitialAmount x MagCapacity / default.MagCapacity
/// rounds, up to the new MaxAmmo); otherwise ammo over the new maximum is
/// cut (VeterancyChanged).
pub(super) fn apply_vet(def: &mut WeaponDef, vet: Vet, fresh: bool) -> Vec<String> {
    let mut changes = Vec::new();
    let class = def.perk.class.clone();
    // ModeDoFire: FireRate = default.FireRate / Rec, FireAnimRate x Rec;
    // KFMeleeFire also SetTimer(DamagedelayMin / Rec).
    let fire_speed = vet.fire_speed(&class);
    for (m, mode) in def.modes.iter_mut().enumerate() {
        if mode.mac10_inc.is_some() && vet.mac10_incendiary() {
            changes.push(format!("fire{m}_damage_type=DamTypeMAC10MPInc (incendiary)"));
        }
        let rec = if mode.perk_speed { fire_speed } else { 1.0 };
        mode.rate = mode.base_rate / rec;
        mode.anim_rate = mode.base_anim_rate * rec;
        if mode.combat.melee {
            mode.combat.damage_delay = mode.base_damage_delay / rec;
        }
        if rec != 1.0 {
            changes.push(format!(
                "fire{m}_rate={:.4}->{:.4} fire{m}_anim_rate={:.3}->{:.3} damage_delay={:.3}->{:.3} (fire_speed x{rec:.2})",
                mode.base_rate, mode.rate, mode.base_anim_rate, mode.anim_rate, mode.base_damage_delay, mode.combat.damage_delay
            ));
        }
    }
    // ReloadMeNow: ReloadRate = Default.ReloadRate / ReloadMulti;
    // ClientReload: ReloadAnimRate x ReloadMulti.
    let reload = vet.reload_speed(&class);
    def.reload_rate = def.base_reload_rate / reload;
    def.reload_anim_rate = def.base_reload_anim_rate * reload;
    if reload != 1.0 && def.ammo.is_some() {
        changes.push(format!("reload_rate={:.3}->{:.3} reload_anim_rate={:.3}->{:.3} (reload x{reload:.2})", def.base_reload_rate, def.reload_rate, def.base_reload_anim_rate, def.reload_anim_rate));
    }
    // ChangedWeapon: InventorySpeedModifier = GroundSpeed x (BaseMeleeIncrease
    // + GetMeleeMovementSpeedModifier) - Weight x 2.
    if def.speed_me_up {
        let before = def.speed_bonus;
        def.speed_bonus = GROUND_SPEED * (BASE_MELEE_INCREASE + vet.melee_movement_speed()) - def.weight * 2.0;
        if def.speed_bonus != before {
            changes.push(format!("melee_speed_bonus={before:.1}->{:.1}", def.speed_bonus));
        }
    }
    if let Some(a) = def.ammo.as_mut() {
        // UpdateMagCapacity: MagCapacity = default.MagCapacity x mod (an int).
        let cap_mod = vet.mag_capacity(&class);
        let capacity = ((a.default_capacity as f32 * cap_mod) as u32).max(1);
        // MaxAmmo x AddExtraAmmoFor (an int).
        let extra = def.ammo_class.as_ref().map_or(1.0, |c| vet.extra_ammo(c));
        let max_total = (a.default_max as f32 * extra) as u32;
        let before = (a.capacity, a.max_total, a.mag, a.spare);
        a.capacity = capacity;
        a.max_total = max_total;
        if fresh {
            let total = ((a.initial as f32 * (capacity as f32 / a.default_capacity.max(1) as f32)) as u32).min(max_total);
            a.mag = capacity.min(total);
            a.spare = total - a.mag;
        } else if a.mag + a.spare > max_total {
            let total = max_total;
            a.mag = a.mag.min(total);
            a.spare = total - a.mag;
        }
        if (a.capacity, a.max_total, a.mag, a.spare) != before && (cap_mod != 1.0 || extra != 1.0 || !fresh) {
            changes.push(format!(
                "mag_capacity={}->{} (x{cap_mod:.2}) max_ammo={}->{} (x{extra:.2}) ammo={}+{}",
                a.default_capacity, a.capacity, a.default_max, a.max_total, a.mag, a.spare
            ));
        }
    }
    if let (Some((cur, max)), Some(c)) = (def.alt_ammo.as_mut(), def.alt_ammo_class.as_ref()) {
        let extra = vet.extra_ammo(c);
        let new_max = (def.alt_default_max as f32 * extra) as u32;
        if new_max != *max {
            changes.push(format!("alt_max_ammo={}->{new_max} (x{extra:.2})", *max));
        }
        *max = new_max;
        *cur = (*cur).min(new_max);
    }
    changes
}

/// Applies the perk to every weapon and logs what changed.
pub(super) fn apply_vet_all(w: &mut Weapons, vet: Vet, reason: &str) {
    w.vet = vet;
    for def in w.defs.iter_mut().filter(|d| !d.gone) {
        let changes = apply_vet(def, vet, false);
        if !changes.is_empty() {
            runlog::kv("perk_mod", &format!("kind=weapon perk={} weapon={} reason={reason} {}", vet.label(), def.class, changes.join(" ")));
        }
    }
}

/// KFHumanPawn.VeterancyChanged and the weapons' per-tick perk values:
/// when the perk changes, every weapon is updated, weapons over the new
/// carry limit are dropped on the floor (drop.rs; they leave the
/// inventory when the pickup is made) and the walking speed factors are
/// recomputed.
#[allow(clippy::too_many_arguments)] // Bevy system parameters
pub(super) fn sync_perk(
    weapons: Option<ResMut<Weapons>>,
    vet: Res<crate::game::perks::Veterancy>,
    effects: Option<ResMut<WeaponEffects>>,
    mut inv: ResMut<crate::game::buy_menu::ShopInventory>,
    mut drops: ResMut<super::drop::WeaponDrops>,
    mut out: MessageWriter<crate::game::pickups::DropItem>,
    cam: super::drop::CamQuery,
) {
    let Some(mut w) = weapons else { return };
    let w = &mut *w;
    let v = vet.vet;
    if w.vet == v {
        return;
    }
    apply_vet_all(w, v, "perk_changed");
    // VeterancyChanged: drop weapons (not bKFNeverThrow) until the weight fits.
    let max = max_carry_weight(&v);
    let weight = carried_weight(w);
    if weight > max {
        let dropped = super::drop::perk_drops(w, &mut drops, &mut out, &cam, max);
        runlog::kv("perk_drop", &format!("dropping={dropped:?} weight={weight} max_carry_weight={max}"));
    }
    if let Some(mut fx) = effects {
        fx.weight_speed_mult = weight_speed_mult(weight, &v);
        fx.perk_speed_mult = v.movement_speed(crate::game::difficulty::game_difficulty());
    }
    inv.max_weight = max;
    runlog::kv("perk_weapons", &format!("perk={} max_carry_weight={max} weight={weight}", v.label()));
}

/// ModifyRecoilSpread for the weapon in hand, if this fire class's
/// ModeDoFire applies it (else 1).
pub(super) fn recoil_mod(w: &Weapons, cur: usize, fm: &FireMode) -> f32 {
    if fm.perk_recoil { w.vet.recoil_spread(&w.defs[cur].perk.class) } else { 1.0 }
}

/// HandleRecoil: SetRecoil(kick, RecoilRate / (default.FireRate / FireRate)).
pub(super) fn recoil_rate(fm: &FireMode) -> f32 {
    if fm.rate > 0.0 && fm.base_rate > 0.0 { fm.recoil.rate / (fm.base_rate / fm.rate) } else { fm.recoil.rate }
}
