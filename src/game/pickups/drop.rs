//! Dropped pickups (tossed dosh, thrown and dropped weapons) as plain code
//! with no Bevy in it: the falling arc (Pickup.InitDroppedPickupFor:
//! PHYS_Falling until Landed) and the timers of the states FallingPickup,
//! Pickup and FadeOut. docs/DESIGN.md, "Tossed dosh and dropped weapons".
//! Unreal units, z up.

use bevy::math::Vec3;
use serde::{Deserialize, Serialize};

/// PhysicsVolume Gravity.Z (and LevelInfo DefaultGravity).
pub const GRAVITY: f32 = 950.0;
/// Seconds per step of the worked-out arc (and between its stored points).
pub const STEP: f32 = 1.0 / 30.0;
/// Longest arc worked out: a pickup still falling then has fallen out of
/// the level.
const MAX_FLIGHT: f32 = 10.0;
/// UE2's walkable floor (a normal at least this much up): the pickup lands.
const FLOOR_NORMAL_Z: f32 = 0.7;
/// How far sideways the arc keeps from walls (the pickup's size, roughly).
const SIDE_GAP: f32 = 4.0;
/// Pickup.InitDroppedPickupFor LifeSpan.
pub const CASH_LIFESPAN: f64 = 16.0;
/// Pickup's FallingPickup and Pickup states: SetTimer(8) before FadeOut.
pub const FADE_TIMER: f64 = 8.0;
/// FadeOut.BeginState: LifeSpan = 1.
pub const FADE_SECONDS: f64 = 1.0;
/// FadeOut.BeginState: RotationRate.Yaw (Unreal units a second).
pub const FADE_YAW_RATE: f32 = 60000.0;

/// The arc a dropped pickup flies along, as every game draws it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Flight {
    /// Its place every `STEP` seconds from the drop; the last one is
    /// where it lands (or where it was when we stopped following it).
    pub path: Vec<[f32; 3]>,
    /// It came to rest on a floor (false: fell out of the level).
    pub landed: bool,
}

impl Flight {
    /// Seconds from the drop to the last point.
    pub fn seconds(&self) -> f32 {
        (self.path.len().saturating_sub(1)) as f32 * STEP
    }

    /// Where it is `t` seconds after the drop.
    pub fn at(&self, t: f32) -> Vec3 {
        let Some(last) = self.path.last() else { return Vec3::ZERO };
        if t <= 0.0 {
            return Vec3::from_array(self.path[0]);
        }
        let f = t / STEP;
        let i = f.floor() as usize;
        if i + 1 >= self.path.len() {
            return Vec3::from_array(*last);
        }
        Vec3::from_array(self.path[i]).lerp(Vec3::from_array(self.path[i + 1]), f - i as f32)
    }
}

/// What kind of dropped item: their timers differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Life {
    /// A Pickup with InitDroppedPickupFor's 16 s LifeSpan and FadeOut
    /// (CashPickup).
    Cash,
    /// A KFWeaponPickup: no LifeSpan, an empty FadeOut: it stays (until the
    /// trader closes, KFGameType.CloseShops).
    Weapon,
}

/// A dropped item's times, in seconds after the drop.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropTimes {
    /// When it lands (None: it never does).
    pub land: Option<f64>,
    /// When the fade-out starts.
    pub fade: Option<f64>,
    /// When it is gone.
    pub gone: Option<f64>,
}

/// The timers of Pickup's states for a dropped item whose flight takes
/// `flight` seconds (`landed`: it lands then).
pub fn drop_times(life: Life, flight: f64, landed: bool) -> DropTimes {
    let land = landed.then_some(flight);
    match life {
        Life::Cash => {
            // FallingPickup.BeginState: SetTimer(8); landing goes to state
            // Pickup, whose BeginState sets the 8 s timer again; Timer ->
            // FadeOut (LifeSpan 1). LifeSpan 16 from the drop at most.
            let fade = match land {
                Some(t) if t < FADE_TIMER => t + FADE_TIMER,
                _ => FADE_TIMER,
            };
            DropTimes { land, fade: Some(fade), gone: Some((fade + FADE_SECONDS).min(CASH_LIFESPAN)) }
        }
        // Fallen out of the level: gone when the arc ends (UE2's KillZ).
        Life::Weapon => DropTimes { land, fade: None, gone: if landed { None } else { Some(flight) } },
    }
}

/// A ray into the level: the distance to the first hit along `dir` within
/// `max`, and the surface normal there (Unreal axes).
pub trait Tracer {
    fn trace(&self, from: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3)>;
}

/// Works out the arc: falling under gravity from `start` at `velocity`;
/// the pickup's centre stays `height` (its CollisionHeight) above floors;
/// walls stop the sideways motion (the velocity slides along them); a
/// floor ends it (Landed).
pub fn fly(tracer: &dyn Tracer, start: Vec3, velocity: Vec3, height: f32) -> Flight {
    let mut p = start;
    let mut v = velocity;
    let mut path = vec![p.to_array()];
    let steps = (MAX_FLIGHT / STEP) as usize;
    for _ in 0..steps {
        v.z -= GRAVITY * STEP;
        let d = v * STEP;
        // Sideways.
        let h = Vec3::new(d.x, d.y, 0.0);
        let hl = h.length();
        if hl > 1e-4 {
            let dir = h / hl;
            match tracer.trace(p, dir, hl + SIDE_GAP) {
                Some((dist, n)) => {
                    p += dir * (dist - SIDE_GAP).clamp(0.0, hl);
                    let nh = Vec3::new(n.x, n.y, 0.0).normalize_or_zero();
                    let into = v.dot(nh);
                    if into < 0.0 {
                        v -= nh * into;
                    }
                }
                None => p += h,
            }
        }
        // Up or down.
        if d.z < 0.0 {
            let fall = -d.z;
            match tracer.trace(p, Vec3::NEG_Z, height + fall) {
                Some((dist, n)) if dist - height <= fall => {
                    p.z -= (dist - height).max(0.0);
                    if n.z >= FLOOR_NORMAL_Z {
                        path.push(p.to_array());
                        return Flight { path, landed: true };
                    }
                    // A steep slope: slide down it.
                    let into = v.dot(n);
                    if into < 0.0 {
                        v -= n * into;
                    }
                }
                _ => p.z -= fall,
            }
        } else if d.z > 0.0 {
            match tracer.trace(p, Vec3::Z, height + d.z) {
                Some((dist, _)) => {
                    p.z += (dist - height).clamp(0.0, d.z);
                    v.z = 0.0;
                }
                None => p.z += d.z,
            }
        }
        path.push(p.to_array());
    }
    Flight { path, landed: false }
}

/// KF's toss and throw velocities (Unreal units a second).
pub mod velocity {
    use bevy::math::Vec3;

    /// `view`: the view direction (with pitch); `facing`: the pawn's facing
    /// (yaw only); `pawn`: the pawn's velocity.
    /// KFPawn.TossCash: view x (pawn velocity along it + 500) + 200 up.
    pub fn toss_cash(view: Vec3, pawn: Vec3) -> Vec3 {
        view * (pawn.dot(view) + 500.0) + Vec3::Z * 200.0
    }

    /// PlayerController.ServerThrowWeapon: view x (velocity along it +
    /// 150) + 100 up; KFWeapon.DropFrom adds facing x 100 (`plus_facing`;
    /// the dual pistols' DropFrom does not).
    pub fn throw_weapon(view: Vec3, facing: Vec3, pawn: Vec3, plus_facing: bool) -> Vec3 {
        view * (pawn.dot(view) + 150.0) + Vec3::Z * 100.0 + if plus_facing { facing * 100.0 } else { Vec3::ZERO }
    }

    /// Pawn.Died: view x (velocity along it + 500) + 200 up (+ facing x 100
    /// in KFWeapon.DropFrom).
    pub fn death(view: Vec3, facing: Vec3, pawn: Vec3, plus_facing: bool) -> Vec3 {
        view * (pawn.dot(view) + 500.0) + Vec3::Z * 200.0 + if plus_facing { facing * 100.0 } else { Vec3::ZERO }
    }

    /// KFHumanPawn.VeterancyChanged: the pawn's velocity (+ facing x 100).
    pub fn perk(facing: Vec3, pawn: Vec3, plus_facing: bool) -> Vec3 {
        pawn + if plus_facing { facing * 100.0 } else { Vec3::ZERO }
    }

    /// Pawn.TossWeapon / KFPawn.TossCash: Location + 0.8 x CollisionRadius
    /// x X - 0.5 x CollisionRadius x Y (X forward, Y right, from the pawn's
    /// facing).
    pub fn start(centre: Vec3, facing: Vec3, radius: f32) -> Vec3 {
        let right = Vec3::new(-facing.y, facing.x, 0.0);
        centre + facing * 0.8 * radius - right * 0.5 * radius
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat floor at z = 0 and a wall at x = `wall` (facing -x).
    struct Room {
        wall: f32,
    }

    impl Tracer for Room {
        fn trace(&self, from: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3)> {
            let mut best: Option<(f32, Vec3)> = None;
            if dir.z < 0.0 {
                let t = from.z / -dir.z;
                if t >= 0.0 && t <= max {
                    best = Some((t, Vec3::Z));
                }
            }
            if dir.x > 0.0 {
                let t = (self.wall - from.x) / dir.x;
                if t >= 0.0 && t <= max && best.is_none_or(|b| t < b.0) {
                    best = Some((t, Vec3::NEG_X));
                }
            }
            best
        }
    }

    #[test]
    fn lands_on_the_floor_at_its_height() {
        let room = Room { wall: 1.0e6 };
        // Tossed cash from a standing pawn: 50 up from the floor, facing +x.
        let v = velocity::toss_cash(Vec3::X, Vec3::ZERO);
        assert_eq!(v, Vec3::new(500.0, 0.0, 200.0));
        let f = fly(&room, Vec3::new(0.0, 0.0, 50.0), v, 5.0);
        assert!(f.landed);
        let end = Vec3::from_array(*f.path.last().unwrap());
        assert!((end.z - 5.0).abs() < 0.01, "rests 5 above the floor: {end}");
        // z(t) = 50 + 200 t - 475 t^2 = 5 -> t = 0.5847 s; x = 500 t.
        assert!((f.seconds() - 0.6).abs() < 0.05, "{}", f.seconds());
        assert!((end.x - 292.0).abs() < 20.0, "{end}");
    }

    #[test]
    fn walls_stop_the_sideways_motion() {
        let room = Room { wall: 100.0 };
        let f = fly(&room, Vec3::new(0.0, 0.0, 50.0), Vec3::new(500.0, 0.0, 200.0), 5.0);
        assert!(f.landed);
        let end = Vec3::from_array(*f.path.last().unwrap());
        assert!(end.x <= 100.0 - SIDE_GAP + 0.01 && end.x > 80.0, "{end}");
        assert!(f.path.iter().all(|p| p[0] <= 100.0));
    }

    #[test]
    fn falls_out_of_the_level_without_a_floor() {
        struct Void;
        impl Tracer for Void {
            fn trace(&self, _: Vec3, _: Vec3, _: f32) -> Option<(f32, Vec3)> {
                None
            }
        }
        let f = fly(&Void, Vec3::ZERO, Vec3::ZERO, 5.0);
        assert!(!f.landed);
        assert!((f.seconds() - MAX_FLIGHT).abs() < 0.05);
    }

    #[test]
    fn positions_along_the_path() {
        let f = Flight { path: vec![[0.0; 3], [30.0, 0.0, 0.0], [60.0, 0.0, 0.0]], landed: true };
        assert_eq!(f.at(-1.0), Vec3::ZERO);
        assert!((f.at(STEP * 0.5).x - 15.0).abs() < 1e-3);
        assert_eq!(f.at(10.0), Vec3::new(60.0, 0.0, 0.0));
    }

    #[test]
    fn cash_fades_eight_seconds_after_landing() {
        let t = drop_times(Life::Cash, 0.6, true);
        assert_eq!(t.land, Some(0.6));
        assert!((t.fade.unwrap() - 8.6).abs() < 1e-9);
        assert!((t.gone.unwrap() - 9.6).abs() < 1e-9);
        // Still falling after 8 s: fades then; LifeSpan 16 caps it.
        let t = drop_times(Life::Cash, 9.5, true);
        assert_eq!(t.fade, Some(8.0));
        assert_eq!(t.gone, Some(9.0));
        let t = drop_times(Life::Cash, 7.5, true);
        assert_eq!(t.gone, Some(CASH_LIFESPAN));
    }

    #[test]
    fn weapons_stay() {
        let t = drop_times(Life::Weapon, 0.4, true);
        assert_eq!(t.fade, None);
        assert_eq!(t.gone, None);
        assert_eq!(drop_times(Life::Weapon, 10.0, false).gone, Some(10.0));
    }

    #[test]
    fn throw_velocities() {
        // Standing, looking level along +x: 150 + 100 forward, 100 up.
        assert_eq!(velocity::throw_weapon(Vec3::X, Vec3::X, Vec3::ZERO, true), Vec3::new(250.0, 0.0, 100.0));
        assert_eq!(velocity::throw_weapon(Vec3::X, Vec3::X, Vec3::ZERO, false), Vec3::new(150.0, 0.0, 100.0));
        // Running at 200 along the view adds it.
        assert_eq!(velocity::toss_cash(Vec3::X, Vec3::new(200.0, 0.0, 0.0)), Vec3::new(700.0, 0.0, 200.0));
        // Start: 16 ahead, 10 to the left (Y is right in Unreal).
        let s = velocity::start(Vec3::ZERO, Vec3::X, 20.0);
        assert!((s - Vec3::new(16.0, -10.0, 0.0)).length() < 1e-4, "{s}");
    }
}
