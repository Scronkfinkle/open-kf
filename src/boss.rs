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
//!
//! Step B5: cloak and sneak. States InitialSneak (from spawn, until he first
//! sees the player) and SneakAround (RangedAttack, every 20 s at most, 70%):
//! both extend Escaping, which extends Charging. Cloaked, running at
//! GroundSpeed x 2.5 (normal speed while attacking), only MeleeClaw when
//! close (uncloaking), and the first damage check ends the sneak.
//!
//! Step B6: knockdown, escape and healing. TakeDamage: under the next
//! HealingLevel (Health / 1.25, / 2, / 3.2 of his starting health) with
//! fewer than 3 syringes used, KnockDown (full body); then cloaked state
//! Escaping (extends Charging) to a hiding spot (BossZombieController
//! SyrRetreat), where BeginHealing plays Heal: a syringe at NotifySyringeA,
//! + Health / 4 at NotifySyringeB.

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
/// AddTraceHitFX: the tracer's speed (SpawnDir x 10000).
pub const MG_TRACER_SPEED: f32 = 10000.0;
/// RangedAttack: rockets only at targets over 500 away; FRand() > 0.75
/// puts it off for FRand() x 5 s, else the next is in 10 + FRand() x 15 s.
const MISSILE_MIN_DISTANCE: f32 = 500.0;
const MISSILE_SKIP_CHANCE: f32 = 0.25;
/// RangedAttack: sneak when LastSneakedTime is over 20 s ago; FRand() <
/// 0.3 puts it off another 20 s. SneakAround ends after 10 s.
const SNEAK_GAP: f32 = 20.0;
const SNEAK_SKIP_CHANCE: f32 = 0.3;
const SNEAK_SECONDS: f32 = 10.0;
/// The sneak states' Begin loops: Sleep(0.5).
const SNEAK_LOOP: f32 = 0.5;
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
    /// KnockDown and Heal: (sequence, seconds).
    pub knockdown_anim: Option<(usize, f32)>,
    pub heal_anim: Option<(usize, f32)>,
    /// Syrange1..3: hidden one per syringe used (PostNetReceive SetBoneScale 0).
    pub syringe_bones: [Option<usize>; 3],
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
            // KF quirk, kept: MGLostSightTimeout is set again at every FireMG
            // end (every 11/30 s), not only when sight is first lost. A
            // timeout of 0.25 + 0.35 x FRand() s longer than one FireMG is
            // pushed back before it can run out, so he only stops on a short
            // roll, and keeps firing at the last seen spot meanwhile.
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
            // KF quirk, kept: the timeout is only checked here, so not during
            // a 0.5-1.25 s pause between bursts.
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

/// Heal's notifies (fractions of the animation, from the mesh):
/// NotifySyringeA (SyringeCount++, a syringe bone hidden), NotifySyringeB
/// (Health += HealingAmount).
pub const HEAL_SYRINGE_A: f32 = 0.068;
pub const HEAL_SYRINGE_B: f32 = 0.464;
/// Our addition, not KF's: an escape that has not reached its hiding spot
/// after this long heals where it is (KF's route finding would have given up
/// and called BeginHealing; ours walks straight on instead).
pub const ESCAPE_GIVE_UP: f32 = 30.0;

/// State Escaping: running to `goal` (a navigation point; None = not
/// chosen yet).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Escape {
    pub seconds: f32,
    next_check: f32,
    pub goal: Option<usize>,
}

/// State Healing: the Heal animation, `seconds` into it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Heal {
    pub seconds: f32,
    pub syringe_done: bool,
    pub health_done: bool,
}

/// A sneak in progress (SneakAround, or InitialSneak from spawn).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sneak {
    pub seconds: f32,
    /// Time to the next pass of the Begin loop.
    next_check: f32,
    pub initial: bool,
}

/// What the sneak loop did this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SneakChange {
    /// CloakBoss: cloaked again (after an attack had uncloaked him).
    Cloaked,
    Ended(&'static str),
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
    /// State KnockDown: seconds of the animation left.
    pub knockdown: Option<f32>,
    pub escape: Option<Escape>,
    pub heal: Option<Heal>,
    /// TakeDamage asked for a knockdown (started by the think code).
    pub pending_knockdown: bool,
    /// SyringeCount, HealingLevels, HealingAmount.
    pub syringes: usize,
    pub healing_levels: [f32; 3],
    pub healing_amount: f32,
    pub charge: Option<Charge>,
    pub sneak: Option<Sneak>,
    /// Seconds since LastSneakedTime.
    pub since_sneak: f32,
    /// bCloaked.
    pub cloaked: bool,
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
    /// KnockDowns that ran to the end. KnockDown's Begin then calls
    /// AddBossBuddySquad (if FinalSquadNum == SyringeCount); the wave code
    /// watches this count.
    pub knockdowns_done: u32,
}

/// What RangedAttack decided this frame (beyond melee).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Decision {
    Nothing,
    StartCharge { attacks: u32 },
    EndCharge(&'static str),
    StartChaingun { shots: i32 },
    StartMissile,
    /// GoToState('SneakAround') (ends a charge first).
    StartSneak,
    /// RangedAttack put the sneak off another 20 s (FRand() < 0.3).
    DelaySneak,
    /// RangedAttack put the rocket off (FRand() > 0.75).
    DelayMissile(f32),
    /// RangedAttack put the chaingun off (FRand() > 0.85).
    DelayChaingun(f32),
}

impl Default for BossState {
    fn default() -> Self {
        BossState::new(4000.0)
    }
}

impl BossState {
    /// ZombieBoss.PostBeginPlay: HealingLevels and HealingAmount from his
    /// Health (4000: 3200, 2000, 1250; 1000), truncated to whole points.
    pub fn new(health: f32) -> Self {
        BossState {
            knockdown: None,
            escape: None,
            heal: None,
            pending_knockdown: false,
            syringes: 0,
            healing_levels: [(health / 1.25).trunc(), (health / 2.0).trunc(), (health / 3.2).trunc()],
            healing_amount: (health / 4.0).trunc(),
            // MakeGrandEntry -> InitialSneak: he arrives cloaked (we skip the
            // Entrance animation), and LastSneakedTime is set when it ends.
            charge: None,
            sneak: Some(Sneak {
                seconds: 0.0,
                next_check: SNEAK_LOOP,
                initial: true,
            }),
            since_sneak: f32::MAX,
            cloaked: true,
            chaingun: None,
            missile: None,
            missile_wait: 0.0,
            chaingun_wait: 0.0,
            hit_from_close: false,
            mg_focal: bevy::math::Vec3::ZERO,
            since_charge: f32::MAX,
            since_force_charge: f32::MAX,
            knockdowns_done: 0,
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
        if self.sneak.is_none() {
            self.since_sneak += dt;
        }
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

    /// The sneak states' Begin loops, every 0.5 s: SneakAround ends after
    /// 10 s; InitialSneak when he has found the player (bAlreadyFoundEnemy,
    /// set by InitialHunting.SeePlayer: `sees`); both cloak again when
    /// uncloaked and not attacking.
    pub fn sneak_step(&mut self, dt: f32, sees: bool, attacking: bool) -> Option<SneakChange> {
        let s = self.sneak.as_mut()?;
        s.seconds += dt;
        s.next_check -= dt;
        if s.next_check > 0.0 {
            return None;
        }
        s.next_check += SNEAK_LOOP;
        let ended = if s.initial {
            sees.then_some("found_player")
        } else {
            (s.seconds > SNEAK_SECONDS).then_some("time")
        };
        if let Some(why) = ended {
            self.end_sneak();
            return Some(SneakChange::Ended(why));
        }
        if !self.cloaked && !attacking {
            self.cloaked = true;
            return Some(SneakChange::Cloaked);
        }
        None
    }

    /// Escaping.EndState (uncloak) and SneakAround.EndState
    /// (LastSneakedTime = now).
    pub fn end_sneak(&mut self) {
        if self.sneak.take().is_some() {
            self.cloaked = false;
            self.since_sneak = 0.0;
        }
    }

    /// The sneak states (SneakAround, InitialSneak).
    pub fn sneaking(&self) -> bool {
        self.sneak.is_some()
    }

    /// State Escaping or one extending it (the sneak states): run at x 2.5
    /// with ChargingAnim, only MeleeClaw (uncloaking), push x 1.5.
    pub fn escaping(&self) -> bool {
        self.sneak.is_some() || self.escape.is_some()
    }

    /// ZombieBoss.TakeDamage, after the damage: below the next healing level,
    /// knocked down. Not when dead, after 3 syringes, or in states Escaping
    /// (KF quirk, kept: IsInState('Escaping') is also true in the sneak
    /// states, which extend it, so he cannot be knocked down while sneaking),
    /// KnockDown or RadialAttack. Returns whether it asked for one.
    pub fn check_knockdown(&mut self, health: f32) -> bool {
        if health <= 0.0 || self.syringes >= 3 || self.escaping() || self.knockdown.is_some() || self.pending_knockdown {
            return false;
        }
        if health < self.healing_levels[self.syringes] {
            self.pending_knockdown = true;
            return true;
        }
        false
    }

    /// GoToState('KnockDown') from whatever he was doing: each state's
    /// EndState (Charging: LastChargeTime; FireChaingun: LastChainGunTime).
    pub fn start_knockdown(&mut self, seconds: f32, roll: f32) {
        self.pending_knockdown = false;
        // Healing is a state too: a knockdown ends it.
        self.heal = None;
        self.end_charge();
        self.end_chaingun(roll);
        self.missile = None;
        self.knockdown = Some(seconds);
    }

    /// KnockDown's Begin: when the animation is done, CloakBoss and
    /// GoToState('Escaping'). Returns true when that happens.
    pub fn knockdown_step(&mut self, dt: f32) -> bool {
        let Some(left) = self.knockdown.as_mut() else {
            return false;
        };
        *left -= dt;
        if *left > 0.0 {
            return false;
        }
        self.knockdown = None;
        self.knockdowns_done += 1;
        self.cloaked = true;
        self.escape = Some(Escape {
            seconds: 0.0,
            next_check: SNEAK_LOOP,
            goal: None,
        });
        true
    }

    /// Escaping's Begin loop (every 0.5 s): cloak again when uncloaked and
    /// not attacking.
    pub fn escape_step(&mut self, dt: f32, attacking: bool) -> bool {
        let Some(e) = self.escape.as_mut() else {
            return false;
        };
        e.seconds += dt;
        e.next_check -= dt;
        if e.next_check > 0.0 {
            return false;
        }
        e.next_check += SNEAK_LOOP;
        if !self.cloaked && !attacking {
            self.cloaked = true;
            return true;
        }
        false
    }

    /// Escaping.BeginHealing: Heal (full body), state Healing;
    /// Escaping.EndState uncloaks him.
    pub fn begin_healing(&mut self) {
        self.escape = None;
        self.cloaked = false;
        self.heal = Some(Heal {
            seconds: 0.0,
            syringe_done: false,
            health_done: false,
        });
    }

    /// The Heal animation, `length` seconds long. Returns what happened
    /// this frame: (a syringe used, health added, finished).
    pub fn heal_step(&mut self, dt: f32, length: f32) -> (bool, Option<f32>, bool) {
        let Some(h) = self.heal.as_mut() else {
            return (false, None, false);
        };
        h.seconds += dt;
        let f = h.seconds / length.max(1e-3);
        let mut syringe = false;
        let mut added = None;
        if f >= HEAL_SYRINGE_A && !h.syringe_done {
            h.syringe_done = true;
            // NotifySyringeA: if( SyringeCount<3 ) SyringeCount++.
            if self.syringes < 3 {
                self.syringes += 1;
                syringe = true;
            }
        }
        if f >= HEAL_SYRINGE_B && !h.health_done {
            h.health_done = true;
            // NotifySyringeB: Health += HealingAmount. KF quirk, kept: not
            // capped at HealthMax.
            added = Some(self.healing_amount);
        }
        let done = f >= 1.0;
        if done {
            self.heal = None;
        }
        (syringe, added, done)
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
    ///
    /// KF quirk, kept by leaving it out: TakeDamage's "charge after 200
    /// damage" (ShouldChargeFromDamage && ChargeDamage > 200) never fires.
    /// ZombieBoss.TakeDamage only sets LastDamageTime inside the "hit within
    /// 10 s of LastDamageTime" branch, and LastDamageTime starts at 0, so
    /// the first branch (ChargeDamage = 0) is taken on every hit.
    pub fn decide(&mut self, dist: f32, attacking: bool, rng: &mut impl FnMut() -> f32) -> Decision {
        // Escaping.RangedAttack (the sneak states): only the claw, when
        // close (done by the caller).
        if self.escaping() {
            return Decision::Nothing;
        }
        // Charging.RangedAttack: too far, then the global one.
        if self.charge.is_some() && dist > CHARGE_DISTANCE && self.since_force_charge > 3.0 {
            self.end_charge();
            return Decision::EndCharge("far");
        }
        let desire_chaingun = rng() < DESIRE_CHAINGUN_CHANCE && self.chaingun_wait <= 0.0;
        if attacking {
            return Decision::Nothing;
        }
        // (Close enough: the melee, done by the caller before this.)
        // The sneak comes before the charge checks, so it can also cut a
        // charge short.
        if self.since_sneak > SNEAK_GAP {
            if rng() < SNEAK_SKIP_CHANCE {
                self.since_sneak = 0.0;
                return Decision::DelaySneak;
            }
            self.end_charge();
            self.sneak = Some(Sneak {
                seconds: 0.0,
                next_check: SNEAK_LOOP,
                initial: false,
            });
            self.cloaked = true;
            return Decision::StartSneak;
        }
        // Charging alone with the player (bOnlyE): nothing more.
        if self.charge.is_some() {
            return Decision::Nothing;
        }
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
        // KF quirk, kept: MeleeClaw2 is in the mesh but never played.
        // RangedAttack calls SetAnimAction('MeleeClaw') by name; only the
        // name 'Claw' would pick from MeleeAnims, and he has none.
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
            knockdown_anim: model.sequence("KnockDown").map(|s| (s, model.length(s) / model.rate(s).max(1e-3))),
            heal_anim: model.sequence("Heal").map(|s| (s, model.length(s) / model.rate(s).max(1e-3))),
            syringe_bones: ["Syrange1", "Syrange2", "Syrange3"].map(|n| model.find_bone(n)),
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
            knockdown_anim: None,
            heal_anim: None,
            syringe_bones: [None; 3],
        }
    }

    /// After the initial sneak, with the next sneak far off.
    fn awake() -> BossState {
        BossState {
            sneak: None,
            cloaked: false,
            since_sneak: 0.0,
            ..BossState::default()
        }
    }

    #[test]
    fn boss_initial_sneak_until_seen() {
        let mut b = BossState::default();
        assert!(b.sneaking() && b.cloaked);
        // Escaping.RangedAttack: no other attacks while sneaking.
        assert_eq!(b.decide(900.0, false, &mut rolls(&[])), Decision::Nothing);
        // Checked every 0.5 s; ends once he has seen the player.
        assert_eq!(b.sneak_step(0.4, true, false), None);
        assert_eq!(b.sneak_step(0.2, true, false), Some(SneakChange::Ended("found_player")));
        assert!(!b.sneaking() && !b.cloaked);
        assert_eq!(b.since_sneak, 0.0);
    }

    #[test]
    fn boss_sneak_every_20s_and_10s_long() {
        let mut b = BossState { chaingun_wait: 100.0, missile_wait: 100.0, ..awake() };
        b.tick(20.1);
        // 30%: put off another 20 s.
        assert_eq!(b.decide(900.0, false, &mut rolls(&[0.9, 0.1])), Decision::DelaySneak);
        assert_eq!(b.since_sneak, 0.0);
        b.tick(20.1);
        assert_eq!(b.decide(900.0, false, &mut rolls(&[0.9, 0.5])), Decision::StartSneak);
        assert!(b.cloaked);
        // An attack uncloaks him; the loop cloaks him again when it ends.
        b.cloaked = false;
        assert_eq!(b.sneak_step(0.5, false, true), None);
        assert_eq!(b.sneak_step(0.5, false, false), Some(SneakChange::Cloaked));
        // Ends after 10 s (checked every 0.5 s).
        let mut ended = None;
        for _ in 0..30 {
            if let Some(SneakChange::Ended(why)) = b.sneak_step(0.5, true, false) {
                ended = Some((b.since_sneak, why));
                break;
            }
        }
        assert_eq!(ended, Some((0.0, "time")));
        // A sneak also cuts a charge short.
        let mut b = BossState { since_sneak: 25.0, ..awake() };
        b.charge = Some(Charge { seconds: 1.0, attacks_left: 2 });
        assert_eq!(b.decide(300.0, false, &mut rolls(&[0.9, 0.5])), Decision::StartSneak);
        assert!(b.charge.is_none());
    }

    #[test]
    fn boss_knockdown_levels() {
        let mut b = awake();
        assert_eq!(b.healing_levels, [3200.0, 2000.0, 1250.0]);
        assert_eq!(b.healing_amount, 1000.0);
        assert!(!b.check_knockdown(3200.0));
        assert!(b.check_knockdown(3199.0));
        // Not again while one is pending or under way.
        assert!(!b.check_knockdown(3000.0));
        b.start_knockdown(2.0, 0.5);
        assert!(!b.check_knockdown(3000.0));
        // KF quirk: not while sneaking (the sneak states extend Escaping).
        let mut b = BossState::default();
        assert!(b.sneaking());
        assert!(!b.check_knockdown(100.0));
        // Not after three syringes, nor when dead.
        let mut b = BossState { syringes: 3, ..awake() };
        assert!(!b.check_knockdown(100.0));
        let mut b = awake();
        assert!(!b.check_knockdown(0.0));
    }

    #[test]
    fn boss_knockdown_escape_heal() {
        let mut b = BossState { chaingun: Some(Chaingun::start(40, 1.0)), ..awake() };
        b.check_knockdown(3100.0);
        b.start_knockdown(2.0, 0.5);
        // FireChaingun.EndState: the next chaingun in 5 + 10 x 0.5 s.
        assert!(b.chaingun.is_none());
        assert_eq!(b.chaingun_wait, 10.0);
        assert!(!b.knockdown_step(1.9));
        assert!(b.knockdown_step(0.2));
        assert!(b.escape.is_some() && b.cloaked && b.escaping());
        // Escaping.RangedAttack: no other attacks.
        assert_eq!(b.decide(900.0, false, &mut rolls(&[])), Decision::Nothing);
        b.begin_healing();
        assert!(!b.cloaked && b.escape.is_none());
        // Heal 5 s: syringe at 0.068, +1000 at 0.464, done at the end.
        let mut events = Vec::new();
        for i in 0..400 {
            let (syringe, added, done) = b.heal_step(1.0 / 60.0, 5.0);
            if syringe || added.is_some() || done {
                events.push((i, syringe, added, done));
            }
            if done {
                break;
            }
        }
        assert_eq!(events.len(), 3, "{events:?}");
        assert!(events[0].1 && (20..=21).contains(&events[0].0), "{events:?}");
        assert_eq!(events[1].2, Some(1000.0));
        assert!((138..=140).contains(&events[1].0), "{events:?}");
        assert!(events[2].3);
        assert_eq!(b.syringes, 1);
    }

    /// FRand() values in order, then 0.5 forever.
    fn rolls(v: &[f32]) -> impl FnMut() -> f32 {
        let mut it = v.to_vec().into_iter();
        move || it.next().unwrap_or(0.5)
    }

    #[test]
    fn boss_charge_start_and_limits() {
        let mut b = BossState { chaingun_wait: 100.0, missile_wait: 100.0, ..awake() };
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
        let mut b = awake();
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
        let mut b = awake();
        // Close and ready to charge, but the 15% chaingun wish: chaingun.
        // Rolls: wish 0.1, skip check 0.5, wait 0.5 (10 s), shots 0.0 (35).
        assert_eq!(b.decide(300.0, false, &mut rolls(&[0.1, 0.5, 0.5, 0.0])), Decision::StartChaingun { shots: 35 });
        assert_eq!(b.chaingun_wait, 10.0);
        // Far, ready (rocket not ready): 15% puts it off for FRand() x 4 s.
        let mut b = BossState { missile_wait: 100.0, ..awake() };
        assert_eq!(b.decide(900.0, false, &mut rolls(&[0.9, 0.9, 0.5])), Decision::DelayChaingun(2.0));
        assert_eq!(b.decide(900.0, false, &mut rolls(&[0.9])), Decision::Nothing);
        b.tick(2.1);
        assert_eq!(b.decide(900.0, false, &mut rolls(&[0.9, 0.5, 0.0, 0.999])), Decision::StartChaingun { shots: 94 });
    }

    #[test]
    fn boss_missile_choice_and_timing() {
        // Over 500 away, chaingun not ready: rocket; next in 10 + 15 x 0.4 s.
        let mut b = BossState { chaingun_wait: 100.0, ..awake() };
        assert_eq!(b.decide(900.0, false, &mut rolls(&[0.9, 0.5, 0.4])), Decision::StartMissile);
        assert_eq!(b.missile_wait, 16.0);
        // 25%: put off for FRand() x 5 s.
        let mut b = BossState { chaingun_wait: 100.0, ..awake() };
        assert_eq!(b.decide(900.0, false, &mut rolls(&[0.9, 0.8, 0.5])), Decision::DelayMissile(2.5));
        // Within 500 (and just charged): no rocket (the chaingun's turn, if ready).
        let mut b = BossState { chaingun_wait: 100.0, since_charge: 0.0, ..awake() };
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
