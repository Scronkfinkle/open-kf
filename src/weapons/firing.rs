//! KF's bullet-gun firing rules that need no game data: spread (KFFire.GetSpread
//! and InstantFire.DoFireEffect) and recoil (KFFire.HandleRecoil and
//! KFPlayerController.SetRecoil / RecoilHandler).
//!
//! Angles are Unreal rotator units (65536 = a full turn) unless named
//! otherwise. Unreal pitch up and yaw right are positive.

use bevy::prelude::*;

use crate::engine::camera::FlyCamera;
use crate::engine::runlog;

/// Unreal rotator units to radians.
pub const ROT_TO_RAD: f32 = std::f32::consts::TAU / 65536.0;

/// A fire mode's spread values (KFFire).
#[derive(Clone, Copy, Debug, Default)]
pub struct SpreadParams {
    /// Spread (the default; KFFire.GetSpread starts from Default.Spread).
    pub spread: f32,
    /// MaxSpread: the limit as a burst goes on.
    pub max_spread: f32,
    /// bAccuracyBonusForSemiAuto: x 0.85 while in semi-auto.
    pub semi_auto_bonus: bool,
}

/// KFFire's burst tracking (LastFireTime, NumShotsInBurst).
#[derive(Clone, Copy, Debug, Default)]
pub struct SpreadState {
    pub last_fire_time: Option<f32>,
    pub shots_in_burst: u32,
}

/// KFFire.GetSpread, called once per shot (ModeDoFire) at time `now`.
/// A shot more than 0.5 s after the last starts a new burst at Spread;
/// otherwise the spread grows by MaxSpread / 6 per shot in the burst, up to
/// MaxSpread. Aiming x 0.5; semi-auto x 0.85 if the weapon has the bonus.
/// Crouching (x 0.85) is not done: there is no crouching yet.
pub fn kf_spread(p: SpreadParams, state: &mut SpreadState, now: f32, aiming: bool, semi_auto: bool) -> f32 {
    let mut accuracy = 1.0;
    if aiming {
        accuracy *= 0.5;
    }
    if p.semi_auto_bonus && semi_auto {
        accuracy *= 0.85;
    }
    state.shots_in_burst += 1;
    let spread = match state.last_fire_time {
        Some(last) if now - last <= 0.5 => (p.spread + state.shots_in_burst as f32 * (p.max_spread / 6.0)).min(p.max_spread),
        _ => {
            state.shots_in_burst = 0;
            p.spread
        }
    };
    // ModeDoFire sets LastFireTime after GetSpread.
    state.last_fire_time = Some(now);
    spread * accuracy
}

/// InstantFire.DoFireEffect: `vector(Aim) + VRand() * FRand() * Spread`,
/// normalised. `vrand` is a random unit vector, `frand` in [0, 1).
pub fn spread_dir(aim: Vec3, spread: f32, vrand: Vec3, frand: f32) -> Vec3 {
    (aim + vrand * frand * spread).normalize()
}

/// A fire mode's recoil values (KFFire).
#[derive(Clone, Copy, Debug, Default)]
pub struct RecoilParams {
    /// RecoilRate: seconds over which each kick is applied.
    pub rate: f32,
    /// maxVerticalRecoilAngle / maxHorizontalRecoilAngle (rotator units).
    pub max_vertical: f32,
    pub max_horizontal: f32,
    /// bRecoilRightOnly: the sideways kick always goes right.
    pub right_only: bool,
    /// RecoilVelocityScale: extra kick per unit of player speed.
    pub velocity_scale: f32,
}

/// KFFire.HandleRecoil's kick for one shot, as (pitch, yaw) rotator units.
/// `r` are three random numbers in [0, 1): vertical, horizontal, side.
/// `speed` is the player's speed in Unreal units/s (VSize(Velocity), normal
/// gravity). The health term HealthMax / Health x 5 is added to both
/// (also to a leftward yaw, as KF does).
pub fn recoil_kick(p: RecoilParams, speed: f32, health: f32, health_max: f32, r: [f32; 3]) -> (f32, f32) {
    // RandRange(max * 0.5, max)
    let mut pitch = p.max_vertical * 0.5 + r[0] * p.max_vertical * 0.5;
    let mut yaw = p.max_horizontal * 0.5 + r[1] * p.max_horizontal * 0.5;
    if !p.right_only && r[2] < 0.5 {
        yaw = -yaw;
    }
    if p.velocity_scale > 0.0 {
        pitch += speed * p.velocity_scale;
        yaw += speed * p.velocity_scale;
    }
    let health_term = health_max / health.max(1.0) * 5.0;
    pitch += health_term;
    yaw += health_term;
    (pitch, yaw)
}

/// KFPlayerController's recoil buffer (RecoilRotator, LastRecoilTime,
/// RecoilSpeed).
#[derive(Resource, Default, Debug)]
pub struct Recoil {
    /// (pitch, yaw), rotator units.
    pub rotator: Vec2,
    pub last_time: f32,
    pub speed: f32,
}

impl Recoil {
    /// SetRecoil: kicks add up; the window restarts.
    pub fn add(&mut self, kick: (f32, f32), speed: f32, now: f32) {
        self.rotator += Vec2::new(kick.0, kick.1);
        self.last_time = now;
        self.speed = speed;
    }

    /// RecoilHandler: within RecoilSpeed of the last kick, turn by
    /// RecoilRotator / RecoilSpeed x dt; after it, clear the buffer. The
    /// whole buffer is reapplied each window, so fast shots compound (KF).
    pub fn step(&mut self, now: f32, dt: f32) -> Vec2 {
        if self.speed > 0.0 && now - self.last_time <= self.speed {
            self.rotator / self.speed * dt
        } else {
            self.rotator = Vec2::ZERO;
            Vec2::ZERO
        }
    }
}

/// Turns the view by the recoil (KFPlayerController.UpdateRotation calls
/// RecoilHandler after mouse look, before LimitPitch).
pub fn apply_recoil(
    time: Res<Time>,
    mut recoil: ResMut<Recoil>,
    mut cams: Query<(&mut Transform, &mut FlyCamera)>,
    mut log_timer: Local<f32>,
) {
    let turn = recoil.step(time.elapsed_secs(), time.delta_secs());
    if turn == Vec2::ZERO {
        return;
    }
    for (mut t, mut cam) in &mut cams {
        // Unreal yaw right = Bevy yaw negative (Bevy yaw turns left).
        cam.pitch = (cam.pitch + turn.x * ROT_TO_RAD).clamp(-1.54, 1.54);
        cam.yaw -= turn.y * ROT_TO_RAD;
        t.rotation = Quat::from_euler(EulerRot::YXZ, cam.yaw, cam.pitch, 0.0);
    }
    *log_timer += time.delta_secs();
    if *log_timer >= 0.25 {
        *log_timer = 0.0;
        runlog::kv(
            "recoil_turn",
            &format!("buffer_pitch={:.0} buffer_yaw={:.0} speed={:.3}", recoil.rotator.x, recoil.rotator.y, recoil.speed),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AK: SpreadParams = SpreadParams {
        spread: 0.015,
        max_spread: 0.12,
        semi_auto_bonus: true,
    };

    #[test]
    fn spread_grows_in_a_burst_and_resets_after_half_a_second() {
        let mut s = SpreadState::default();
        // First shot: Default.Spread.
        assert!((kf_spread(AK, &mut s, 0.0, false, false) - 0.015).abs() < 1e-6);
        // Next shots within 0.5 s: + 0.02 per shot in the burst.
        assert!((kf_spread(AK, &mut s, 0.1, false, false) - 0.035).abs() < 1e-6);
        assert!((kf_spread(AK, &mut s, 0.2, false, false) - 0.055).abs() < 1e-6);
        for t in 3..20 {
            kf_spread(AK, &mut s, t as f32 * 0.1, false, false);
        }
        // Capped at MaxSpread.
        assert!((kf_spread(AK, &mut s, 2.0, false, false) - 0.12).abs() < 1e-6);
        // A pause over 0.5 s starts over.
        assert!((kf_spread(AK, &mut s, 2.6, false, false) - 0.015).abs() < 1e-6);
    }

    #[test]
    fn spread_bonuses_multiply() {
        let mut s = SpreadState::default();
        let v = kf_spread(AK, &mut s, 0.0, true, true);
        assert!((v - 0.015 * 0.5 * 0.85).abs() < 1e-6);
        // No semi-auto bonus for weapons without bAccuracyBonusForSemiAuto.
        let mut s = SpreadState::default();
        let p = SpreadParams {
            semi_auto_bonus: false,
            ..AK
        };
        assert!((kf_spread(p, &mut s, 0.0, false, true) - 0.015).abs() < 1e-6);
    }

    #[test]
    fn recoil_kick_ranges() {
        let p = RecoilParams {
            rate: 0.07,
            max_vertical: 500.0,
            max_horizontal: 250.0,
            right_only: true,
            velocity_scale: 3.0,
        };
        // Standing, full health: pitch 250..500 + 5, yaw 125..250 + 5.
        assert_eq!(recoil_kick(p, 0.0, 100.0, 100.0, [0.0, 0.0, 0.0]), (255.0, 130.0));
        assert_eq!(recoil_kick(p, 0.0, 100.0, 100.0, [1.0, 1.0, 0.0]), (505.0, 255.0));
        // Walking at 200: + 600 each. Half health: + 10 instead of + 5.
        assert_eq!(recoil_kick(p, 200.0, 50.0, 100.0, [0.0, 0.0, 0.0]), (860.0, 735.0));
        // Not right-only: r[2] < 0.5 turns the kick left (before the extras).
        let p2 = RecoilParams { right_only: false, ..p };
        assert_eq!(recoil_kick(p2, 0.0, 100.0, 100.0, [0.0, 0.0, 0.2]), (255.0, -120.0));
    }

    #[test]
    fn recoil_buffer_applies_over_the_rate_then_clears() {
        let mut r = Recoil::default();
        r.add((700.0, 70.0), 0.07, 0.0);
        let mut total = Vec2::ZERO;
        // Seven 0.01 s frames inside the 0.07 s window.
        for i in 0..7 {
            total += r.step(i as f32 * 0.01, 0.01);
        }
        assert!((total.x - 700.0).abs() < 1.0, "{total}");
        // After the window the buffer empties.
        assert_eq!(r.step(0.2, 0.01), Vec2::ZERO);
        assert_eq!(r.rotator, Vec2::ZERO);
    }
}
