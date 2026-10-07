//! Healing other players (KF: SyringeFire, the medic guns' darts,
//! HealingProjectile.ProcessTouch). Each player's health lives on their own
//! game, so a heal is a message: the healer's game decides who is healed
//! and how much (HealBoostAmount x the healer's GetHealPotency), pays the
//! healer (ReceiveRewardForHealing) and sends the heal; the network code
//! (net/heals.rs) carries it to the healed player's game, which applies it
//! with KFPawn.GiveHealth. Solo there is no one to heal. See docs/perks.md.

use bevy::prelude::*;

use crate::engine::runlog;
use crate::game::combat::PLAYER_HEALTH_MAX;

/// KFHumanPawn CollisionRadius (as the other modules' PLAYER_RADIUS).
pub const PAWN_RADIUS: f32 = 20.0;
/// KFHumanPawn CollisionHeight.
pub const PAWN_HALF_HEIGHT: f32 = 44.0;

/// Another player as this game knows them (from the network; empty solo).
#[derive(Clone, Debug)]
pub struct Teammate {
    pub peer: u64,
    pub name: String,
    /// Collision cylinder centre (KF's Location), Bevy coordinates.
    pub centre: Vec3,
    /// Health as their game last reported it (KFPRI.PlayerHealth; their
    /// healthToGive is not shared).
    pub health: f32,
    /// A living, walking pawn.
    pub alive: bool,
}

#[derive(Resource, Default, Debug)]
pub struct Teammates(pub Vec<Teammate>);

/// This game's player heals another (KFPawn.GiveHealth(HealSum,
/// HealthMax) on them).
#[derive(Message, Clone, Debug)]
pub struct HealTeammate {
    pub peer: u64,
    /// HealSum: HealBoostAmount x the healer's GetHealPotency.
    pub heal_sum: f32,
    pub source: &'static str,
}

/// Another player healed this game's player (from the network).
#[derive(Message, Clone, Debug)]
pub struct HealedByTeammate {
    pub amount: f32,
    pub healer: String,
    pub source: String,
}

/// SyringeFire.GetHealee: VisibleCollidingActors(KFHumanPawn, 80) around
/// the healer; living, hurt; `Dir dot (their Location - mine) > 0.7`
/// where Dir is the view direction and the offset is NOT normalised
/// (KF's code: in Unreal units, so nearly anyone in front within reach);
/// the biggest dot wins. `me`, `view`: Bevy; `units`: metres per Unreal
/// unit. Guesses: "colliding within 80" as the centre distance minus
/// their collision radius; the visibility trace is left out (80 units).
pub fn syringe_healee(me: Vec3, view: Vec3, teammates: &[Teammate], units: f32, radius: f32) -> Option<&Teammate> {
    let mut best: Option<(&Teammate, f32)> = None;
    for t in teammates {
        if !t.alive || t.health <= 0.0 || t.health >= PLAYER_HEALTH_MAX {
            continue;
        }
        let off = (t.centre - me) / units;
        if off.length() - radius > 80.0 {
            continue;
        }
        let dot = view.normalize_or_zero().dot(off);
        if dot > 0.7 && best.is_none_or(|(_, b)| dot > b) {
            best = Some((t, dot));
        }
    }
    best.map(|(t, _)| t)
}

/// Where a ray (Bevy, `dir` unit, up to `max` metres) first enters a
/// pawn's collision cylinder (`radius`, `half_height` in Unreal units,
/// `units` metres per unit): the distance, or None.
pub fn ray_cylinder(from: Vec3, dir: Vec3, max: f32, centre: Vec3, radius: f32, half_height: f32, units: f32) -> Option<f32> {
    let (r, h) = (radius * units, half_height * units);
    let o = from - centre;
    // Sides: solve |o.xz + t d.xz| = r.
    let (ox, oz, dx, dz) = (o.x, o.z, dir.x, dir.z);
    let a = dx * dx + dz * dz;
    let mut best: Option<f32> = None;
    let mut consider = |t: f32| {
        if (0.0..=max).contains(&t) && best.is_none_or(|b| t < b) {
            best = Some(t);
        }
    };
    if a > 1e-9 {
        let b = 2.0 * (ox * dx + oz * dz);
        let c = ox * ox + oz * oz - r * r;
        let disc = b * b - 4.0 * a * c;
        if disc >= 0.0 {
            for t in [(-b - disc.sqrt()) / (2.0 * a), (-b + disc.sqrt()) / (2.0 * a)] {
                if (o.y + t * dir.y).abs() <= h {
                    consider(t);
                }
            }
        }
    }
    // Caps.
    if dir.y.abs() > 1e-9 {
        for cap in [h, -h] {
            let t = (cap - o.y) / dir.y;
            let p = o + dir * t;
            if p.x * p.x + p.z * p.z <= r * r {
                consider(t);
            }
        }
    }
    // Starting inside counts as a hit at once.
    if o.x * o.x + o.z * o.z <= r * r && o.y.abs() <= h {
        consider(0.0);
    }
    best
}

/// The healer's reward (SyringeFire.Timer, HealingProjectile.ProcessTouch,
/// MedicNade.HealOrHurt): MedicReward = HealSum (an int) cut to what the
/// healee can still take (HealthMax - Health - healthToGive, at least 0),
/// then int(min(MedicReward, HealthMax) / HealthMax x 60).
pub fn medic_reward(heal_sum: f32, healee_health: f32, healee_to_give: f32) -> i32 {
    let mut reward = heal_sum.trunc();
    if healee_health + healee_to_give + reward > PLAYER_HEALTH_MAX {
        reward = (PLAYER_HEALTH_MAX - (healee_health + healee_to_give)).max(0.0);
    }
    (reward.min(PLAYER_HEALTH_MAX) / PLAYER_HEALTH_MAX * 60.0) as i32
}

/// The healer's side: PRI.ReceiveRewardForHealing (Score and Team.Score +
/// the reward) and "You healed NAME" (ClientSuccessfulHeal's
/// ClientMessage; shown when the heal lands, a stand-in for the syringe's
/// moment of the attempt).
fn reward_healer(
    mut heals: MessageReader<HealTeammate>,
    teammates: Res<Teammates>,
    mut dosh: ResMut<crate::game::dosh::Dosh>,
    mut messages: MessageWriter<crate::game::hud::LocalMessage>,
    vet: Res<crate::game::perks::Veterancy>,
) {
    for h in heals.read() {
        let Some(t) = teammates.0.iter().find(|t| t.peer == h.peer) else { continue };
        // Their healthToGive is not known here: taken as 0 (guess).
        let reward = medic_reward(h.heal_sum, t.health, 0.0);
        dosh.score += reward as f32;
        dosh.team += reward as f32;
        messages.write(crate::game::hud::LocalMessage::text(format!("You healed {}", t.name)));
        runlog::kv(
            "perk_effect",
            &format!(
                "perk={} effect=heal_teammate source={} peer={} name=\"{}\" heal={} their_health={:.0} reward={reward} heal_potency={}",
                vet.vet.label(),
                h.source,
                h.peer,
                t.name,
                h.heal_sum,
                t.health,
                vet.vet.heal_potency()
            ),
        );
    }
}

/// The healed side: KFPawn.GiveHealth(HealSum, HealthMax).
fn receive_heals(mut incoming: MessageReader<HealedByTeammate>, mut give: MessageWriter<crate::game::combat::GiveHealth>) {
    for h in incoming.read() {
        runlog::kv("healed_by_teammate", &format!("amount={} healer=\"{}\" source={}", h.amount, h.healer, h.source));
        give.write(crate::game::combat::GiveHealth { amount: h.amount, max: PLAYER_HEALTH_MAX, source: "teammate" });
    }
}

pub struct HealingPlugin;

impl Plugin for HealingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Teammates>()
            .add_message::<HealTeammate>()
            .add_message::<HealedByTeammate>()
            .add_systems(Update, (reward_healer, receive_heals));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const U: f32 = 1.0 / 50.0;

    fn mate(peer: u64, x_units: f32, health: f32) -> Teammate {
        // Bevy -Z is Unreal +X (forward here).
        Teammate { peer, name: format!("P{peer}"), centre: Vec3::new(0.0, 0.0, -x_units * U), health, alive: true }
    }

    #[test]
    fn healee_in_front_within_reach_and_hurt() {
        let fwd = Vec3::NEG_Z;
        let list = [mate(1, 60.0, 50.0), mate(2, 200.0, 10.0), mate(3, 50.0, 100.0)];
        // 2 is too far, 3 is not hurt.
        assert_eq!(syringe_healee(Vec3::ZERO, fwd, &list, U, 25.0).map(|t| t.peer), Some(1));
        // Behind: no.
        assert!(syringe_healee(Vec3::ZERO, Vec3::Z, &list, U, 25.0).is_none());
        // The nearer-in-front one wins only by the bigger (unnormalised) dot.
        let two = [mate(4, 30.0, 50.0), mate(5, 90.0, 50.0)];
        assert_eq!(syringe_healee(Vec3::ZERO, fwd, &two, U, 25.0).map(|t| t.peer), Some(5));
    }

    #[test]
    fn ray_hits_a_pawn_cylinder() {
        let c = Vec3::new(0.0, 0.0, -2.0);
        // Straight at it: enters 20 units (0.4 m) before the centre.
        let t = ray_cylinder(Vec3::ZERO, Vec3::NEG_Z, 5.0, c, 20.0, 44.0, U).unwrap();
        assert!((t - 1.6).abs() < 1e-4);
        // Too short, or passing above the head.
        assert!(ray_cylinder(Vec3::ZERO, Vec3::NEG_Z, 1.0, c, 20.0, 44.0, U).is_none());
        assert!(ray_cylinder(Vec3::new(0.0, 1.0, 0.0), Vec3::NEG_Z, 5.0, c, 20.0, 44.0, U).is_none());
    }

    #[test]
    fn reward_is_a_share_of_60_for_what_was_healed() {
        // Syringe 20 x 1.75 (Medic 6) = 35 on someone at 50: int(35/100 x 60) = 21.
        assert_eq!(medic_reward(35.0, 50.0, 0.0), 21);
        // At 90 only 10 fits: 6.
        assert_eq!(medic_reward(35.0, 90.0, 0.0), 6);
        assert_eq!(medic_reward(20.0, 100.0, 0.0), 0);
    }
}
