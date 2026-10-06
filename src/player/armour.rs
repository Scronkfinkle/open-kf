//! Body armour (T3b): the Kevlar vest's points (ShieldStrength), how they
//! soak up hits (KFPawn.ShieldAbsorb) and buying them at the trader
//! (KFPawn.ServerBuyKevlar). See DESIGN.md, T3b.

use bevy::prelude::*;

use crate::engine::runlog;

/// Vest.ItemCost (also BuyableVest.ItemCost): 300 for 100 points.
pub const VEST_COST: f32 = 300.0;
/// KFBuyMenuInvList: ItemAmmoMax 100; ServerBuyKevlar fills to 100.
pub const MAX_ARMOUR: f32 = 100.0;

/// Pawn.ShieldStrength and xPawn.SmallShieldStrength, both floats. A new
/// pawn (game start, after a death) has 0 of each.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq)]
pub struct Armour {
    pub strength: f32,
    pub small: f32,
}

impl Armour {
    /// KFPawn.ShieldAbsorb(int dam): the damage left for health. Copied
    /// line by line; `dam` is an int in KF and so is the result (float to
    /// int truncates). No perk: GetBodyArmorDamageModifier is skipped.
    pub fn absorb(&mut self, dam: f32) -> f32 {
        let mut damage = dam.trunc();
        if self.strength == 0.0 {
            return damage;
        }
        let mut remaining = 0.0;
        if self.strength > self.small {
            let interval = self.strength - self.small;
            if interval >= 0.75 * damage {
                self.strength -= 0.75 * damage;
                if self.strength < self.small {
                    self.small = self.strength;
                }
                return (0.25 * damage).trunc();
            }
            self.strength = self.small;
            damage -= interval;
            // KF writes 0.33, not a third.
            remaining = 0.33 * interval;
            if remaining <= damage {
                return damage.trunc();
            }
            damage -= remaining;
        }
        // KF quirk: only reached with ShieldStrength <= SmallShieldStrength,
        // and SmallShieldStrength only rises in here, so in a solo game
        // (where nothing else sets it) this half only ever sees
        // ShieldStrength 0 and passes the damage through. Kept as written.
        if self.strength >= 0.5 * damage {
            self.strength -= damage;
            self.small = self.strength;
            return (remaining + 0.25 * damage).trunc();
        }
        damage -= self.strength;
        self.strength = 0.0;
        self.small = 0.0;
        (damage + remaining).trunc()
    }
}

/// What ServerBuyKevlar did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum VestBuy {
    /// Paid this much; armour is now full.
    Full(f32),
    /// Short of dosh: paid this much for this many points.
    Partial { cost: f32, points: f32 },
    /// Refused: already full.
    Full100,
    /// Refused: not enough dosh for the full price and no armour (KF only
    /// sells a part to someone already wearing some).
    NoDosh,
}

/// KFPawn.ServerBuyKevlar, after the CanBuyNow check: `score` is
/// PRI.Score (a float). No perk discount (GetCostScaling) as perks are
/// not in.
pub fn buy_kevlar(armour: &mut Armour, score: &mut f32) -> VestBuy {
    let cost = VEST_COST * ((100.0 - armour.strength) / 100.0);
    if armour.strength == MAX_ARMOUR {
        return VestBuy::Full100;
    }
    if *score >= cost {
        *score -= cost;
        armour.strength = MAX_ARMOUR;
        return VestBuy::Full(cost);
    }
    if armour.strength > 0.0 {
        let cost = VEST_COST / 100.0;
        // UnitsAffordable = int(Score / Cost); Score -= int(Cost x Units).
        let units = (*score / cost).trunc();
        let paid = (cost * units).trunc();
        *score -= paid;
        // KF does not cap this: the units affordable here are always
        // fewer than the points missing (else the full buy above ran).
        armour.strength += units;
        return VestBuy::Partial { cost: paid, points: units };
    }
    VestBuy::NoDosh
}

/// The menu's vest row (KFBuyMenuInvList): (points shown, fill price).
pub fn menu_row(armour: &Armour) -> (i32, i32) {
    let ammo_cost = (VEST_COST as i32) / 100;
    (armour.strength as i32, ((100.0 - armour.strength) * ammo_cost as f32) as i32)
}

/// Asks to buy the vest (from the menu or the `buy_vest` test action).
#[derive(Message, Clone, Copy, Debug)]
pub struct BuyVest;

pub struct ArmourPlugin;

impl Plugin for ArmourPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Armour>().add_message::<BuyVest>().add_systems(Update, buy_vest);
    }
}

fn buy_vest(
    mut requests: MessageReader<BuyVest>,
    mut armour: ResMut<Armour>,
    mut dosh: ResMut<crate::game::dosh::Dosh>,
    (game, shops): (Res<crate::game::waves::WaveGame>, Res<crate::game::trader::Shops>),
    mut sounds: MessageWriter<crate::audio::mixer::PlaySound>,
) {
    for _ in requests.read() {
        // CanBuyNow: no wave in progress, touching a shop.
        let can_buy = !matches!(game.phase, crate::game::waves::Phase::Wave | crate::game::waves::Phase::BossWave) && shops.player_inside().is_some();
        if !can_buy {
            runlog::kv("shop_refused", "request=vest reason=not_in_open_shop_time");
            continue;
        }
        let before = armour.strength;
        match buy_kevlar(&mut armour, &mut dosh.score) {
            VestBuy::Full100 => runlog::kv("shop_refused", "request=vest reason=armour_full"),
            VestBuy::NoDosh => {
                runlog::kv("shop_refused", &format!("request=vest reason=dosh score={:.2} armour=0", dosh.score));
                sounds.write(crate::weapons::weapon::trader_refusal("KF_Trader.TooExpensive"));
            }
            VestBuy::Full(cost) => runlog::kv(
                "shop_vest",
                &format!("bought=full cost={cost:.2} armour_before={before:.2} armour={:.2} dosh_left={:.2}", armour.strength, dosh.score),
            ),
            VestBuy::Partial { cost, points } => runlog::kv(
                "shop_vest",
                &format!("bought=partial points={points} cost={cost} armour_before={before:.2} armour={:.2} dosh_left={:.2}", armour.strength, dosh.score),
            ),
        }
        // MakeSomeBuyNoise(class'Vest'): Vest_Pickup, SLOT_Interface, 255, radius 120.
        if armour.strength > before {
            sounds.write(
                crate::audio::mixer::PlaySound::new("KF_InventorySnd.Vest_Pickup", crate::audio::mixer::Emitter::Listener)
                    .slot(crate::audio::mixer::Slot::Interface)
                    .volume(255.0)
                    .radius(120.0),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vest(strength: f32) -> Armour {
        Armour { strength, small: 0.0 }
    }

    #[test]
    fn full_vest_takes_three_quarters() {
        let mut a = vest(100.0);
        // 20: the vest takes 15, you take 5.
        assert_eq!(a.absorb(20.0), 5.0);
        assert_eq!(a, vest(85.0));
        // 7: the vest takes 5.25, you take int(1.75) = 1.
        assert_eq!(a.absorb(7.0), 1.0);
        assert_eq!(a.strength, 79.75);
        // Fractional damage in is an int in KF.
        let mut b = vest(100.0);
        assert_eq!(b.absorb(20.9), 5.0);
    }

    #[test]
    fn breaking_vest_passes_the_rest_through() {
        // 10 points left, a 20 hit: 15 would be needed; you take 20 - 10.
        let mut a = vest(10.0);
        assert_eq!(a.absorb(20.0), 10.0);
        assert_eq!(a, vest(0.0));
        // A big hit on a full vest: 200 less the vest's 100.
        let mut b = vest(100.0);
        assert_eq!(b.absorb(200.0), 100.0);
        assert_eq!(b, vest(0.0));
    }

    #[test]
    fn no_armour_no_change() {
        let mut a = vest(0.0);
        assert_eq!(a.absorb(33.0), 33.0);
        assert_eq!(a, vest(0.0));
    }

    #[test]
    fn second_half_with_small_shield_set() {
        // Only reachable if SmallShieldStrength > 0 (nothing in solo KF
        // sets it); checks the copied lines: 30 >= 0.5 x 20, so the vest
        // takes all 20 and you take int(0.25 x 20).
        let mut a = Armour { strength: 30.0, small: 30.0 };
        assert_eq!(a.absorb(20.0), 5.0);
        assert_eq!(a, Armour { strength: 10.0, small: 10.0 });
    }

    #[test]
    fn buying_follows_server_buy_kevlar() {
        // Empty, 300 dosh: full price.
        let (mut a, mut s) = (vest(0.0), 300.0);
        assert_eq!(buy_kevlar(&mut a, &mut s), VestBuy::Full(300.0));
        assert_eq!((a.strength, s), (100.0, 0.0));
        // Full: refused.
        assert_eq!(buy_kevlar(&mut a, &mut s), VestBuy::Full100);
        // 79.75 left: 300 x 0.2025 = 60.75, taken off a float score.
        let (mut a, mut s) = (vest(79.75), 100.0);
        assert_eq!(buy_kevlar(&mut a, &mut s), VestBuy::Full(60.75));
        assert_eq!((a.strength, s), (100.0, 39.25));
        // Empty and under 300: nothing (KF quirk).
        let (mut a, mut s) = (vest(0.0), 299.0);
        assert_eq!(buy_kevlar(&mut a, &mut s), VestBuy::NoDosh);
        assert_eq!((a.strength, s), (0.0, 299.0));
        // Some armour, short of dosh: int(50 / 3) = 16 points for 48.
        let (mut a, mut s) = (vest(40.0), 50.0);
        assert_eq!(buy_kevlar(&mut a, &mut s), VestBuy::Partial { cost: 48.0, points: 16.0 });
        assert_eq!((a.strength, s), (56.0, 2.0));
    }

    #[test]
    fn menu_row_prices() {
        assert_eq!(menu_row(&vest(0.0)), (0, 300));
        assert_eq!(menu_row(&vest(79.75)), (79, 60));
    }
}
