//! The Patriarch (KFChar.ZombieBoss): the parts of his script that no other
//! zed shares. `zed.rs` calls in here; see `docs/DESIGN.md`, "Patriarch".
//!
//! Step B1: his melee. He has no MeleeAnims; ZombieBoss.RangedAttack starts
//! MeleeImpale or MeleeClaw when he is close enough (IsCloseEnuf), both on the
//! upper body from SpineBone1, and ClawDamageTarget lands the hits at the
//! animations' notifies, with each attack's own reach.
//!
//! Step B2: the charge (state Charging). RangedAttack runs about every frame
//! while he sees the player (BossZombieController.TimedFireWeaponAtEnemy:
//! FireWeaponAt always returns false, so SetTimer(0.01)), so its FRand()
//! rolls are re-rolled each frame. KF's "charge from damage" never happens:
//! TakeDamage only sets LastDamageTime inside the branch that needs it set
//! already, so ChargeDamage is reset on every hit. We copy that by leaving
//! it out.
//!
//! Step B3: the chaingun (state FireChaingun), as `Chaingun::step`: the
//! state's Begin loop (bursts of shots 0.05 s apart, pauses) and its AnimEnd
//! (PreFireMG, then FireMG over and over, checking sight each time).
//!
//! Step B4: the rocket (state FireMissile): over 500 away, PreFireMissile,
//! a BossLAWProj from `tip` when it ends (`fireball.rs`), FireEndMissile.

use crate::skinned::SkinnedModel;

/// ZombieBossBase defaults.
const CLAW_RANGE: f32 = 85.0;
const IMPALE_RANGE: f32 = 45.0;
/// RangedAttack: MeleeImpale only above this health, half the time.
const IMPALE_MIN_HEALTH: f32 = 1500.0;
const IMPALE_CHANCE: f32 = 0.5;
/// IsCloseEnuf: flat distance under both radii plus this.
const CLOSE_EXTRA: f32 = 25.0;
/// Charging: GroundSpeed x 2.5, x 1.25 while attacking; push x 1.5.
pub const CHARGE_SPEED: f32 = 2.5;
pub const CHARGE_ATTACK_SPEED: f32 = 1.25;
pub const CHARGE_PUSH: f32 = 1.5;
/// Charging: Sleep(6) then GoToState('').
const CHARGE_SECONDS: f32 = 6.0;
/// RangedAttack: charge within 700 (the only enemy around: always, solo);
/// Charging.RangedAttack: stop when over 700 away (3 s after a forced charge).
const CHARGE_DISTANCE: f32 = 700.0;
/// RangedAttack: FRand() < 0.15 "wants the chaingun" blocks a charge.
const DESIRE_CHAINGUN_CHANCE: f32 = 0.15;
/// RangedAttack: FRand() > 0.85 puts the chaingun off for FRand() x 4 s.
const CHAINGUN_SKIP_CHANCE: f32 = 0.15;
/// FireMGShot: VRand() x 0.06 spread, a 10000-unit trace, MGDamage
/// 6 x 0.75 (one player, Normal) + Rand(3) in whole points, momentum 500.
pub const MG_SPREAD: f32 = 0.06;
pub const MG_RANGE: f32 = 10000.0;
pub const MG_DAMAGE: f32 = 6.0 * 0.75;
pub const MG_MOMENTUM: f32 = 500.0;
/// RangedAttack: rockets only at targets over 500 away; FRand() > 0.75
/// puts it off for FRand() x 5 s, else the next is in 10 + FRand() x 15 s.
const MISSILE_MIN_DISTANCE: f32 = 500.0;
const MISSILE_SKIP_CHANCE: f32 = 0.25;
/// FireChaingun.TakeDamage: shot from closer than this, charge instead.
pub const MG_CLOSE_DAMAGE_DISTANCE: f32 = 100.0;

/// One of his melee attacks: the sequence, when ClawDamageTarget fires
/// (fractions of the animation), and the reach used for those hits.
#[derive(Clone, Debug)]
pub struct BossMelee {
    pub seq: usize,
    pub hits: Vec<f32>,
    pub range: f32,
}

/// What the Patriarch's class needs beyond the shared zed data.
pub struct BossClass {
    pub claw: BossMelee,
    pub impale: BossMelee,
    /// ChargingAnim (RunF): his walk animation while charging.
    pub charge_walk: Option<usize>,
    /// SetAnimAction('transition') on starting a charge, upper body.
    pub transition: Option<usize>,
    /// PreFireMG, FireMG, FireEndMG: (sequence, seconds).
    pub mg_anims: Option<[(usize, f32); 3]>,
    /// The `tip` bone: where his shots start.
    pub tip_bone: Option<usize>,
    /// PreFireMissile, FireEndMissile: (sequence, seconds).
    pub missile_anims: Option<[(usize, f32); 2]>,
}

/// State FireMissile: PreFireMissile, then (at its AnimEnd) the rocket and
/// FireEndMissile, which plays out back in the normal state.
#[derive(Clone, Copy, Debug)]
pub struct Missile {
    anim_left: f32,
    pub fired: bool,
}

/// What the rocket wants done this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MissileEvent {
    /// Fire the rocket and play FireEndMissile.
    Fire,
    /// FireEndMissile finished.
    Done,
}

impl Missile {
    pub fn start(prefire_seconds: f32) -> Missile {
        Missile {
            anim_left: prefire_seconds,
            fired: false,
        }
    }

    /// One frame; `end_seconds` is FireEndMissile's length.
    pub fn step(&mut self, dt: f32, end_seconds: f32) -> Option<MissileEvent> {
        self.anim_left -= dt;
        if self.anim_left > 0.0 {
            return None;
        }
        if self.fired {
            return Some(MissileEvent::Done);
        }
        self.fired = true;
        self.anim_left += end_seconds;
        Some(MissileEvent::Fire)
    }
}

/// Which chaingun animation to play (full body).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MgAnim {
    Fire,
    End,
}

/// What the chaingun wants done this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MgEvent {
    Play(MgAnim),
    /// One FireMGShot (the counter is already lowered).
    Shoot,
    /// FireEndMG finished: back to chasing.
    Done,
}

/// State FireChaingun. Times are seconds since it started.
#[derive(Clone, Copy, Debug)]
pub struct Chaingun {
    pub clock: f32,
    /// MGFireCounter: shots left.
    pub shots_left: i32,
    ending: bool,
    /// Time until the playing animation's AnimEnd.
    anim_left: f32,
    fire_at_will: bool,
    /// MGFireDuration: the burst lasts until this time.
    fire_until: f32,
    /// The Begin loop: time until it runs again; set a new burst length then.
    loop_wait: f32,
    new_burst: bool,
    /// MGLostSightTimeout (None = 0: in sight).
    lost_sight_at: Option<f32>,
}

impl Chaingun {
    /// RangedAttack: PreFireMG, MGFireCounter = Rand(60) + 35.
    pub fn start(shots: i32, prefire_seconds: f32) -> Chaingun {
        Chaingun {
            clock: 0.0,
            shots_left: shots,
            ending: false,
            anim_left: prefire_seconds,
            fire_at_will: false,
            fire_until: 0.0,
            loop_wait: 0.0,
            new_burst: false,
            lost_sight_at: None,
        }
    }

    /// Whether he is shooting at the player (Controller.Focus), not at
    /// where the player was last seen.
    pub fn has_focus(&self) -> bool {
        self.lost_sight_at.is_none()
    }

    /// One frame. `sees` = LineOfSightTo and FastTrace from `tip` to the
    /// player; `seconds` the length of each animation; `rng` gives FRand().
    pub fn step(&mut self, dt: f32, sees: bool, seconds: [f32; 3], rng: &mut impl FnMut() -> f32) -> Vec<MgEvent> {
        let mut out = Vec::new();
        self.clock += dt;
        self.anim_left -= dt;
        if self.ending {
            if self.anim_left <= 0.0 {
                out.push(MgEvent::Done);
            }
            return out;
        }
        // AnimEnd (PreFireMG, then each FireMG).
        if self.anim_left <= 0.0 {
            if self.shots_left <= 0 {
                return self.end(seconds, out);
            }
            self.lost_sight_at = if sees { None } else { Some(self.clock + 0.25 + 0.35 * rng()) };
            if !self.fire_at_will {
                self.fire_until = self.clock + 0.75 + 0.5 * rng();
            }
            self.fire_at_will = true;
            self.anim_left += seconds[1];
            out.push(MgEvent::Play(MgAnim::Fire));
        }
        // The Begin loop.
        self.loop_wait -= dt;
        while self.loop_wait <= 0.0 {
            if std::mem::take(&mut self.new_burst) {
                self.fire_until = self.clock + 0.75 + 0.5 * rng();
            }
            if self.lost_sight_at.is_some_and(|t| self.clock > t) || self.shots_left <= 0 {
                return self.end(seconds, out);
            }
            if self.clock > self.fire_until {
                // Pause (the barrels spin): Sleep(0.5 + FRand() x 0.75).
                self.loop_wait += 0.5 + 0.75 * rng();
                self.new_burst = true;
            } else {
                if self.fire_at_will {
                    self.shots_left -= 1;
                    out.push(MgEvent::Shoot);
                }
                self.loop_wait += 0.05;
            }
        }
        out
    }

    /// FireEndMG and GoToState('').
    fn end(&mut self, seconds: [f32; 3], mut out: Vec<MgEvent>) -> Vec<MgEvent> {
        self.ending = true;
        self.anim_left = seconds[2];
        out.push(MgEvent::Play(MgAnim::End));
        out
    }
}

/// A charge in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Charge {
    pub seconds: f32,
    /// NumChargeAttacks: Rand(2) + 1; each damage check uses one.
    pub attacks_left: u32,
}

/// The Patriarch's own state, on his Zed.
#[derive(Clone, Copy, Debug)]
pub struct BossState {
    pub charge: Option<Charge>,
    pub chaingun: Option<Chaingun>,
    pub missile: Option<Missile>,
    /// Seconds until LastMissileTime (0 = may fire).
    pub missile_wait: f32,
    /// Seconds until LastChainGunTime (0 = may shoot).
    pub chaingun_wait: f32,
    /// Hit from closer than MG_CLOSE_DAMAGE_DISTANCE since the last frame.
    pub hit_from_close: bool,
    /// FocalPoint: where the player was last seen while firing (Bevy).
    pub mg_focal: bevy::math::Vec3,
    /// Seconds since LastChargeTime and LastForceChargeTime (both start at
    /// 0 in KF, long before he spawns).
    pub since_charge: f32,
    pub since_force_charge: f32,
}

/// What RangedAttack decided this frame (beyond melee).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Decision {
    Nothing,
    StartCharge { attacks: u32 },
    EndCharge(&'static str),
    StartChaingun { shots: i32 },
    StartMissile,
    /// RangedAttack put the rocket off (FRand() > 0.75).
    DelayMissile(f32),
    /// RangedAttack put the chaingun off (FRand() > 0.85).
    DelayChaingun(f32),
}

impl Default for BossState {
    fn default() -> Self {
        BossState {
            charge: None,
            chaingun: None,
            missile: None,
            missile_wait: 0.0,
            chaingun_wait: 0.0,
            hit_from_close: false,
            mg_focal: bevy::math::Vec3::ZERO,
            since_charge: f32::MAX,
            since_force_charge: f32::MAX,
        }
    }
}

impl BossState {
    /// Timers, and the Charging state's own ends: Sleep(6), and Tick's
    /// NumChargeAttacks <= 0.
    pub fn tick(&mut self, dt: f32) -> Option<&'static str> {
        self.since_charge += dt;
        self.since_force_charge += dt;
        self.chaingun_wait = (self.chaingun_wait - dt).max(0.0);
        self.missile_wait = (self.missile_wait - dt).max(0.0);
        let c = self.charge.as_mut()?;
        c.seconds += dt;
        let why = if c.attacks_left == 0 {
            "attacks_used"
        } else if c.seconds >= CHARGE_SECONDS {
            "timeout"
        } else {
            return None;
        };
        self.end_charge();
        Some(why)
    }

    /// Charging.EndState: LastChargeTime = now.
    pub fn end_charge(&mut self) {
        if self.charge.take().is_some() {
            self.since_charge = 0.0;
        }
    }

    /// FireChaingun.EndState: LastChainGunTime = now + 5 + FRand() x 10.
    pub fn end_chaingun(&mut self, roll: f32) {
        if self.chaingun.take().is_some() {
            self.chaingun_wait = 5.0 + 10.0 * roll;
        }
    }

    /// Charging.MeleeDamageTarget: one attack used per damage check; a
    /// landed hit ends the charge. Returns whether the charge ended.
    pub fn charge_hit(&mut self, landed: bool) -> bool {
        let Some(c) = self.charge.as_mut() else {
            return false;
        };
        c.attacks_left = c.attacks_left.saturating_sub(1);
        if landed {
            self.end_charge();
            return true;
        }
        false
    }

    /// ZombieBoss.RangedAttack after the melee check (and Charging's
    /// override), with the player in sight. `dist` in Unreal units,
    /// `attacking` = bShotAnim, `rng` gives FRand().
    pub fn decide(&mut self, dist: f32, attacking: bool, rng: &mut impl FnMut() -> f32) -> Decision {
        if self.charge.is_some() {
            // Charging.RangedAttack: too far, then the global one, which
            // does nothing more while charging alone with the player.
            if dist > CHARGE_DISTANCE && self.since_force_charge > 3.0 {
                self.end_charge();
                return Decision::EndCharge("far");
            }
            return Decision::Nothing;
        }
        if attacking {
            return Decision::Nothing;
        }
        let desire_chaingun = rng() < DESIRE_CHAINGUN_CHANCE && self.chaingun_wait <= 0.0;
        if !desire_chaingun && dist < CHARGE_DISTANCE && self.since_charge > 5.0 + 5.0 * rng() {
            let attacks = if rng() < 0.5 { 1 } else { 2 };
            self.charge = Some(Charge { seconds: 0.0, attacks_left: attacks });
            return Decision::StartCharge { attacks };
        }
        if self.missile_wait <= 0.0 && dist > MISSILE_MIN_DISTANCE {
            if rng() > 1.0 - MISSILE_SKIP_CHANCE {
                self.missile_wait = 5.0 * rng();
                return Decision::DelayMissile(self.missile_wait);
            }
            self.missile_wait = 10.0 + 15.0 * rng();
            return Decision::StartMissile;
        }
        if self.chaingun_wait <= 0.0 {
            if rng() > 1.0 - CHAINGUN_SKIP_CHANCE {
                self.chaingun_wait = 4.0 * rng();
                return Decision::DelayChaingun(self.chaingun_wait);
            }
            self.chaingun_wait = 5.0 + 10.0 * rng();
            let shots = 35 + (rng() * 60.0).min(59.0) as i32;
            return Decision::StartChaingun { shots };
        }
        Decision::Nothing
    }
}

impl BossClass {
    /// Reads MeleeClaw and MeleeImpale and their ClawDamageTarget notifies.
    pub fn load(model: &SkinnedModel, charging_anim: Option<&str>) -> Result<BossClass, String> {
        let attack = |name: &str, range: f32| -> Result<BossMelee, String> {
            let seq = model.sequence(name).ok_or(format!("no {name} animation"))?;
            let hits: Vec<f32> = model
                .notifies(seq)
                .iter()
                .filter(|n| n.name.eq_ignore_ascii_case("ClawDamageTarget"))
                .map(|n| n.time)
                .collect();
            if hits.is_empty() {
                return Err(format!("{name} has no ClawDamageTarget notify"));
            }
            Ok(BossMelee { seq, hits, range })
        };
        Ok(BossClass {
            claw: attack("MeleeClaw", CLAW_RANGE)?,
            impale: attack("MeleeImpale", IMPALE_RANGE)?,
            charge_walk: charging_anim.and_then(|n| model.sequence(n)),
            transition: model.sequence("transition"),
            mg_anims: {
                let anim = |n: &str| model.sequence(n).map(|s| (s, model.length(s) / model.rate(s).max(1e-3)));
                match (anim("PreFireMG"), anim("FireMG"), anim("FireEndMG")) {
                    (Some(a), Some(b), Some(c)) => Some([a, b, c]),
                    _ => None,
                }
            },
            tip_bone: model.find_bone("tip"),
            missile_anims: {
                let anim = |n: &str| model.sequence(n).map(|s| (s, model.length(s) / model.rate(s).max(1e-3)));
                anim("PreFireMissile").zip(anim("FireEndMissile")).map(|(a, b)| [a, b])
            },
        })
    }

    /// The attack whose sequence this is.
    pub fn melee_for(&self, seq: usize) -> Option<&BossMelee> {
        [&self.claw, &self.impale].into_iter().find(|m| m.seq == seq)
    }

    /// RangedAttack: MeleeImpale if Health > 1500 and FRand() < 0.5, else
    /// MeleeClaw. `roll` is in 0..1.
    pub fn choose_melee(&self, health: f32, roll: f32) -> &BossMelee {
        if health > IMPALE_MIN_HEALTH && roll < IMPALE_CHANCE {
            &self.impale
        } else {
            &self.claw
        }
    }
}

/// ZombieBoss.IsCloseEnuf, in Unreal units: heights overlap and the flat
/// distance is under both radii + 25.
pub fn is_close_enough(flat_distance: f32, height_difference: f32, radius: f32, height: f32, target_radius: f32, target_height: f32) -> bool {
    height_difference.abs() <= height + target_height && flat_distance < radius + target_radius + CLOSE_EXTRA
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class() -> BossClass {
        BossClass {
            claw: BossMelee { seq: 1, hits: vec![0.5], range: CLAW_RANGE },
            impale: BossMelee { seq: 2, hits: vec![0.5, 0.609], range: IMPALE_RANGE },
            charge_walk: None,
            transition: None,
            mg_anims: None,
            tip_bone: None,
            missile_anims: None,
        }
    }

    /// FRand() values in order, then 0.5 forever.
    fn rolls(v: &[f32]) -> impl FnMut() -> f32 {
        let mut it = v.to_vec().into_iter();
        move || it.next().unwrap_or(0.5)
    }

    #[test]
    fn boss_charge_start_and_limits() {
        let mut b = BossState { chaingun_wait: 100.0, missile_wait: 100.0, ..Default::default() };
        // Over 700 away, or wanting the chaingun (it is not ready, so the
        // wish does not count): no charge without the gap.
        assert_eq!(b.decide(701.0, false, &mut rolls(&[0.9])), Decision::Nothing);
        // Attacking: nothing.
        assert_eq!(b.decide(300.0, true, &mut rolls(&[0.9])), Decision::Nothing);
        assert_eq!(b.decide(300.0, false, &mut rolls(&[0.9, 0.0, 0.7])), Decision::StartCharge { attacks: 2 });
        // Charging: the 6 s limit.
        assert_eq!(b.tick(5.9), None);
        assert_eq!(b.tick(0.2), Some("timeout"));
        assert!(b.charge.is_none());
        // 5 + 5 x FRand() s before the next one.
        assert_eq!(b.decide(300.0, false, &mut rolls(&[0.9, 0.0])), Decision::Nothing);
        b.tick(5.1);
        assert_eq!(b.decide(300.0, false, &mut rolls(&[0.9, 0.5])), Decision::Nothing);
        assert_eq!(b.decide(300.0, false, &mut rolls(&[0.9, 0.0, 0.0])), Decision::StartCharge { attacks: 1 });
        // Over 700 while charging ends it.
        assert_eq!(b.decide(800.0, false, &mut rolls(&[])), Decision::EndCharge("far"));
    }

    #[test]
    fn boss_charge_ends_on_hit_or_attacks_used() {
        let mut b = BossState::default();
        b.decide(300.0, false, &mut rolls(&[0.9, 0.0, 0.7]));
        // A miss uses an attack; a landed hit ends the charge.
        assert!(!b.charge_hit(false));
        assert_eq!(b.charge.map(|c| c.attacks_left), Some(1));
        assert!(b.charge_hit(true));
        assert!(b.charge.is_none());
        // Two misses: the next tick ends it.
        b.since_charge = 100.0;
        b.decide(300.0, false, &mut rolls(&[0.9, 0.0, 0.7]));
        b.charge_hit(false);
        b.charge_hit(false);
        assert_eq!(b.tick(0.01), Some("attacks_used"));
    }

    #[test]
    fn boss_chaingun_choice() {
        let mut b = BossState::default();
        // Close and ready to charge, but the 15% chaingun wish: chaingun.
        // Rolls: wish 0.1, skip check 0.5, wait 0.5 (10 s), shots 0.0 (35).
        assert_eq!(b.decide(300.0, false, &mut rolls(&[0.1, 0.5, 0.5, 0.0])), Decision::StartChaingun { shots: 35 });
        assert_eq!(b.chaingun_wait, 10.0);
        // Far, ready (rocket not ready): 15% puts it off for FRand() x 4 s.
        let mut b = BossState { missile_wait: 100.0, ..Default::default() };
        assert_eq!(b.decide(900.0, false, &mut rolls(&[0.9, 0.9, 0.5])), Decision::DelayChaingun(2.0));
        assert_eq!(b.decide(900.0, false, &mut rolls(&[0.9])), Decision::Nothing);
        b.tick(2.1);
        assert_eq!(b.decide(900.0, false, &mut rolls(&[0.9, 0.5, 0.0, 0.999])), Decision::StartChaingun { shots: 94 });
    }

    #[test]
    fn boss_missile_choice_and_timing() {
        // Over 500 away, chaingun not ready: rocket; next in 10 + 15 x 0.4 s.
        let mut b = BossState { chaingun_wait: 100.0, ..Default::default() };
        assert_eq!(b.decide(900.0, false, &mut rolls(&[0.9, 0.5, 0.4])), Decision::StartMissile);
        assert_eq!(b.missile_wait, 16.0);
        // 25%: put off for FRand() x 5 s.
        let mut b = BossState { chaingun_wait: 100.0, ..Default::default() };
        assert_eq!(b.decide(900.0, false, &mut rolls(&[0.9, 0.8, 0.5])), Decision::DelayMissile(2.5));
        // Within 500 (and just charged): no rocket (the chaingun's turn, if ready).
        let mut b = BossState { chaingun_wait: 100.0, since_charge: 0.0, ..Default::default() };
        assert_eq!(b.decide(450.0, false, &mut rolls(&[0.9, 0.9])), Decision::Nothing);
        // PreFireMissile 2 s, then fire, FireEndMissile 1 s, then done.
        let mut m = Missile::start(2.0);
        let mut events = Vec::new();
        for i in 0..240 {
            if let Some(e) = m.step(1.0 / 60.0, 1.0) {
                events.push((i, e));
                if e == MissileEvent::Done {
                    break;
                }
            }
        }
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].1, MissileEvent::Fire);
        assert!((119..=121).contains(&events[0].0), "{events:?}");
        assert_eq!(events[1].1, MissileEvent::Done);
        assert!((179..=181).contains(&events[1].0), "{events:?}");
    }

    /// Runs a chaingun at 60 frames/s; `sees(clock)` says when he sees the
    /// player. Returns (shot times, end time, whether it finished).
    fn run_mg(shots: i32, frames: usize, roll: f32, sees: impl Fn(f32) -> bool) -> (Vec<f32>, Option<f32>, bool) {
        // PreFireMG 51 frames, FireMG 11, FireEndMG: lengths at 30 fps.
        let secs = [51.0 / 30.0, 11.0 / 30.0, 1.0];
        let mut mg = Chaingun::start(shots, secs[0]);
        let mut rng = || roll;
        let (mut shot_times, mut end, mut done) = (Vec::new(), None, false);
        for _ in 0..frames {
            let see = sees(mg.clock);
            for e in mg.step(1.0 / 60.0, see, secs, &mut rng) {
                match e {
                    MgEvent::Shoot => shot_times.push(mg.clock),
                    MgEvent::Play(MgAnim::End) => end = Some(mg.clock),
                    MgEvent::Done => done = true,
                    MgEvent::Play(_) => {}
                }
            }
            if done {
                break;
            }
        }
        (shot_times, end, done)
    }

    #[test]
    fn boss_chaingun_bursts() {
        let (shots, end, done) = run_mg(35, 1200, 0.5, |_| true);
        assert_eq!(shots.len(), 35);
        // No shot before PreFireMG ends (1.7 s).
        assert!(shots[0] >= 1.7 - 1e-3, "{}", shots[0]);
        // 0.05 s apart within a burst (one frame of slack).
        let gaps: Vec<f32> = shots.windows(2).map(|w| w[1] - w[0]).collect();
        assert!(gaps.iter().all(|g| *g > 0.03), "{gaps:?}");
        // Bursts of 0.75 + 0.5 x 0.5 = 1 s, then a 0.5 + 0.75 x 0.5 s pause.
        assert!(gaps.iter().any(|g| *g > 0.8), "{gaps:?}");
        assert!(end.is_some() && done);
    }

    #[test]
    fn boss_chaingun_stops_when_sight_lost() {
        // Loses sight at 2.5 s. Each FireMG end (every 11/30 s) sets the
        // timeout again to 0.25 + 0.35 x FRand() s, so it only runs out
        // when that is shorter than one FireMG, and the Begin loop only
        // checks it when awake (not during a 0.5-1.25 s pause): FRand() 0.2
        // (0.32 s) ends it in the next burst (3.9 s here), FRand() 0.5
        // (0.425 s) never does.
        let (shots, end, _) = run_mg(94, 1200, 0.2, |t| t < 2.5);
        let end = end.expect("ended");
        assert!(shots.len() < 94);
        assert!(end > 2.5 && end < 4.5, "{end}");
        let (shots, _, _) = run_mg(94, 1200, 0.5, |t| t < 2.5);
        assert_eq!(shots.len(), 94);
    }

    #[test]
    fn boss_impale_only_above_1500_and_half_the_time() {
        let c = class();
        assert_eq!(c.choose_melee(4000.0, 0.2).seq, 2);
        assert_eq!(c.choose_melee(4000.0, 0.7).seq, 1);
        assert_eq!(c.choose_melee(1500.0, 0.2).seq, 1);
    }

    #[test]
    fn boss_close_enough() {
        // Patriarch 26x44, player 20x50: under 71 flat, heights within 94.
        assert!(is_close_enough(70.0, 0.0, 26.0, 44.0, 20.0, 50.0));
        assert!(!is_close_enough(71.0, 0.0, 26.0, 44.0, 20.0, 50.0));
        assert!(!is_close_enough(30.0, 95.0, 26.0, 44.0, 20.0, 50.0));
    }
}
