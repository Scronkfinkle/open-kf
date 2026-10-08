//! KF's animation-channel timing rules shared by zeds and the first-person
//! weapon: the tween into a new animation and the linear fade of a layer's
//! weight. Behaviour read from the engine (details in the local RE.md).

use bevy::prelude::*;

/// A tween (UE2 PlayAnim's TweenTime): every bone moves from the pose last
/// shown (`from`, local bone transforms) to the new animation's first frame,
/// linearly in time over `total` seconds. The animation's own clock waits
/// until the tween is over (no frames advance, no notifies fire).
#[derive(Clone, Debug, Default)]
pub struct Tween {
    pub from: Vec<(Quat, Vec3)>,
    pub left: f32,
    pub total: f32,
    /// False until the first `advance`: a tween starts during a frame
    /// whose time step had already passed before it, so that step is not
    /// counted (the tween lasts `total` seconds from its start).
    pub running: bool,
}

impl Tween {
    /// A tween of `time` seconds from `from`; none for `time <= 0` or when
    /// no pose has been shown yet (KF snaps then).
    pub fn start(from: &[(Quat, Vec3)], time: f32) -> Option<Tween> {
        (time > 0.0 && !from.is_empty()).then(|| Tween { from: from.to_vec(), left: time, total: time, running: false })
    }

    /// Advances by `dt`. `None` while still tweening; `Some(leftover)` on
    /// the tick it ends, with the part of `dt` past its end (KF plays that
    /// leftover time of the animation in the same tick).
    pub fn advance(&mut self, dt: f32) -> Option<f32> {
        if !self.running {
            self.running = true;
            return None;
        }
        self.left -= dt;
        (self.left <= 0.0).then(|| -self.left)
    }

    /// How far toward the target pose (0 = `from`, 1 = target).
    pub fn weight(&self) -> f32 {
        (1.0 - self.left / self.total.max(1e-6)).clamp(0.0, 1.0)
    }
}

/// A layer weight fading to `target` over `left` seconds (UE2
/// AnimBlendToAlpha): each tick alpha moves (target - alpha) x
/// min(dt / left, 1), i.e. linearly in time.
#[derive(Clone, Copy, Debug)]
pub struct Fade {
    pub alpha: f32,
    pub target: f32,
    pub left: f32,
}

impl Fade {
    /// Advances by `dt`; true once the target is reached.
    pub fn advance(&mut self, dt: f32) -> bool {
        if dt > 0.0 {
            let k = if self.left > 0.0 { (dt / self.left).min(1.0) } else { 1.0 };
            self.alpha += (self.target - self.alpha) * k;
            self.left = (self.left - dt).max(0.0);
        }
        self.left <= 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tween_weight_is_linear_and_returns_leftover() {
        let from = vec![(Quat::IDENTITY, Vec3::ZERO)];
        assert!(Tween::start(&from, 0.0).is_none());
        assert!(Tween::start(&[], 0.1).is_none());
        let mut t = Tween::start(&from, 0.1).unwrap();
        assert_eq!(t.weight(), 0.0);
        // The frame it starts in does not count.
        assert_eq!(t.advance(0.5), None);
        assert_eq!(t.weight(), 0.0);
        assert_eq!(t.advance(0.025), None);
        assert!((t.weight() - 0.25).abs() < 1e-5);
        assert_eq!(t.advance(0.05), None);
        assert!((t.weight() - 0.75).abs() < 1e-5);
        let left = t.advance(0.04).unwrap();
        assert!((left - 0.015).abs() < 1e-5);
        assert_eq!(t.weight(), 1.0);
    }

    #[test]
    fn fade_is_linear_over_its_time() {
        let mut f = Fade { alpha: 1.0, target: 0.0, left: 0.12 };
        let mut alphas = Vec::new();
        for _ in 0..4 {
            let done = f.advance(0.03);
            alphas.push((f.alpha, done));
        }
        for (i, (a, _)) in alphas.iter().enumerate() {
            assert!((a - (1.0 - 0.25 * (i + 1) as f32)).abs() < 1e-5, "{alphas:?}");
        }
        assert!(alphas[3].1 && !alphas[2].1);
    }
}
