//! Zed movement state and rules that do not need the world: falling
//! bookkeeping (DESIGN.md, "Pathfinding").

/// Per-zed movement bookkeeping.
#[derive(Default, Clone, Copy, Debug)]
pub struct Motion {
    /// Seconds in the current fall (0 when not falling).
    pub fall_seconds: f32,
    /// The surface hit last while falling (Bevy normal), for KF's "ditch"
    /// landing.
    pub fall_hit: Option<bevy::math::Vec3>,
}

/// KF's "ditch" landing (falling physics): two surfaces hit one after the
/// other both face up but toward each other, and the pawn did not move down:
/// it is wedged in a V and counts as landed even though neither surface is
/// flat enough to stand on.
pub fn ditch(previous: Option<bevy::math::Vec3>, normal: bevy::math::Vec3, moved_down: f32) -> bool {
    previous.is_some_and(|p| p.y > 0.0 && normal.y > 0.0 && p.dot(normal) < 0.0) && moved_down <= 0.0
}

/// A fall longer than this is logged (`zed_fall_long`): a normal drop or
/// jump lasts well under a second.
pub const LONG_FALL: f32 = 3.0;

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::Vec3;

    #[test]
    fn v_between_two_slopes_is_a_landing() {
        // Two 60-degree slopes (too steep to stand on): normals 120 degrees
        // apart.
        let left = Vec3::new(0.866, 0.5, 0.0);
        let right = Vec3::new(-0.866, 0.5, 0.0);
        assert!(ditch(Some(left), right, 0.0));
        // Still sliding down: not landed.
        assert!(!ditch(Some(left), right, 0.5));
        // Same slope twice, a wall, or no earlier hit: not a ditch.
        assert!(!ditch(Some(left), left, 0.0));
        assert!(!ditch(Some(Vec3::X), right, 0.0));
        assert!(!ditch(None, right, 0.0));
    }
}
