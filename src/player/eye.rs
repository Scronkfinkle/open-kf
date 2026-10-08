//! The player's eye height (Pawn.UpdateEyeHeight): the view keeps its
//! world height when the body steps up or down and catches up with the
//! normal eye height; after a hard landing it dips and recovers. Unreal
//! units; per frame. See DESIGN.md, "Player movement details from KF".

/// KFPawn BaseEyeHeight, above the cylinder centre.
pub const BASE_EYE_HEIGHT: f32 = 44.0;
/// KFHumanPawn CollisionHeight (the eye may sink to half of it below the centre).
const COLLISION_HEIGHT: f32 = 50.0;
/// MAXSTEPHEIGHT (Pawn.uc).
const MAX_STEP_HEIGHT: f32 = 35.0;
/// Gap kept between the eye and a ceiling above it.
pub const CEILING_GAP: f32 = 14.0;

/// How far above the centre the ceiling check reaches.
pub const CEILING_CHECK: f32 = COLLISION_HEIGHT + MAX_STEP_HEIGHT + CEILING_GAP;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Eye {
    /// EyeHeight: the view's height above the cylinder centre.
    pub height: f32,
    /// LandBob: grows while the landing dip falls; the bob and the weapon
    /// use it.
    pub land_bob: f32,
    /// bJustLanded: the landing dip is running.
    pub just_landed: bool,
    /// bLandRecovery: the dip reached its bottom and is coming back.
    pub land_recovery: bool,
}

impl Default for Eye {
    fn default() -> Self {
        Eye { height: BASE_EYE_HEIGHT, land_bob: 0.0, just_landed: false, land_recovery: false }
    }
}

/// MaxEyeHeight: 85 above the centre, or 14 below a ceiling found by the
/// line check from the top of the cylinder (`ceiling` = the hit's height
/// above the centre).
pub fn max_eye_height(ceiling: Option<f32>) -> f32 {
    match ceiling {
        None => COLLISION_HEIGHT + MAX_STEP_HEIGHT,
        Some(z) => z - CEILING_GAP,
    }
}

impl Eye {
    /// Pawn.Landed: landing faster than 200 down starts the dip
    /// (PlayerController.bLandingShake, true in KF's default settings).
    /// Returns true if it started.
    pub fn landed(&mut self, vertical_speed: f32) -> bool {
        if vertical_speed < -200.0 {
            self.just_landed = true;
            return true;
        }
        false
    }

    /// One frame. `dz`: how far the centre moved up this frame by its own
    /// physics (since OldZ); `walking`: on the ground (Controller.
    /// WantsSmoothedView); `max`: `max_eye_height`.
    pub fn update(&mut self, dt: f32, dz: f32, walking: bool, max: f32) {
        let base = BASE_EYE_HEIGHT;
        if dz.abs() > 15.0 {
            self.just_landed = false;
            self.land_recovery = false;
        }
        if !self.just_landed {
            let s = (10.0 * dt).min(0.9);
            self.land_bob *= 1.0 - s;
            if walking {
                // Smooth up / down stairs: keep the world height, ease back.
                self.height = ((self.height - dz) * (1.0 - s) + base * s).max(-0.5 * COLLISION_HEIGHT).min(max);
            } else {
                self.height = (self.height * (1.0 - s) + base * s).min(max);
            }
        } else if self.land_recovery {
            let s = (10.0 * dt).min(0.9);
            self.height = (self.height * (1.0 - 0.6 * s) + base * 0.6 * s).min(base);
            self.land_bob *= 1.0 - s;
            if self.height >= base - 1.0 {
                self.just_landed = false;
                self.land_recovery = false;
                self.height = base;
            }
        } else {
            let s = (10.0 * dt).min(0.65);
            let old = self.height;
            self.height = (self.height * (1.0 - 1.5 * s)).min(max);
            self.land_bob += 0.03 * (old - self.height);
            if self.height < 0.25 * base + 1.0 || self.land_bob > 3.0 {
                self.land_recovery = true;
                self.height = 0.25 * base + 1.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: f32 = 85.0;

    #[test]
    fn step_up_keeps_world_height_then_catches_up() {
        let mut e = Eye::default();
        // The body rose 30 units in one frame: the eye stays where it was
        // (14 above the new centre), then eases back to 44.
        e.update(1.0 / 60.0, 30.0, true, MAX);
        assert!((e.height - (14.0 * (1.0 - 1.0 / 6.0) + 44.0 / 6.0)).abs() < 1e-3, "{}", e.height);
        for _ in 0..18 {
            e.update(1.0 / 60.0, 0.0, true, MAX);
        }
        assert!(e.height > 43.0, "after 0.3 s: {}", e.height);
    }

    #[test]
    fn landing_dips_and_recovers() {
        let mut e = Eye::default();
        assert!(e.landed(-325.0));
        let mut min = e.height;
        let mut frames_to_min = 0;
        let mut frames = 0;
        while e.just_landed && frames < 600 {
            e.update(1.0 / 60.0, 0.0, true, MAX);
            frames += 1;
            if e.height < min {
                min = e.height;
                frames_to_min = frames;
            }
        }
        assert!((min - 12.0).abs() < 0.01, "bottom {min}");
        assert!(frames_to_min <= 6, "bottom after {frames_to_min} frames");
        assert!(frames > 20 && frames < 60, "back to 44 after {frames} frames");
        assert_eq!(e.height, BASE_EYE_HEIGHT);
        assert!(e.land_bob > 0.0);
    }

    #[test]
    fn slow_landing_and_big_steps_do_not_dip() {
        let mut e = Eye::default();
        assert!(!e.landed(-150.0));
        assert!(e.landed(-400.0));
        e.update(1.0 / 60.0, 20.0, true, MAX);
        assert!(!e.just_landed);
    }

    #[test]
    fn ceiling_limits_the_eye() {
        assert_eq!(max_eye_height(None), 85.0);
        let mut e = Eye::default();
        e.update(1.0 / 60.0, 0.0, true, max_eye_height(Some(52.0)));
        assert_eq!(e.height, 38.0);
    }
}
