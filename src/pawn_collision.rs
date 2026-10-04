//! Pawns blocking pawns, as in Unreal: upright cylinders that cannot overlap.
//!
//! Only the horizontal part of a move is clipped (a swept circle test in the
//! ground plane); the vertical part is left to the world sweep. Run this on a
//! move before sweeping it through the world, so walls still have the last say.

use bevy::prelude::*;

/// An upright cylinder in Bevy space (metres).
#[derive(Clone, Copy, Debug)]
pub struct Cylinder {
    pub centre: Vec3,
    pub radius: f32,
    pub half_height: f32,
}

/// Gap kept between cylinders, metres (0.5 Unreal units).
const SKIN: f32 = 0.5 * crate::coords::SCALE;

/// Clips `delta` so `me` does not move into any of `others`, sliding along
/// the side of a cylinder it touches. Returns the clipped move and the index
/// of the last cylinder that blocked it.
pub fn clip_move(me: &Cylinder, delta: Vec3, others: &[Cylinder]) -> (Vec3, Option<usize>) {
    let mut pos = me.centre.xz();
    let mut rest = delta.xz();
    let mut moved = Vec2::ZERO;
    let mut blocker = None;
    for _ in 0..3 {
        if rest.length_squared() < 1e-12 {
            break;
        }
        // Earliest contact along `rest`: time in [0, 1] and contact normal.
        let mut first: Option<(f32, Vec2, usize)> = None;
        for (i, o) in others.iter().enumerate() {
            if (me.centre.y - o.centre.y).abs() >= me.half_height + o.half_height {
                continue;
            }
            let p = pos - o.centre.xz();
            let r = me.radius + o.radius + SKIN;
            let hit = if p.length_squared() <= r * r {
                // Already touching or overlapping: block only motion inward.
                (p.dot(rest) < 0.0).then(|| (0.0, p.normalize_or(Vec2::X)))
            } else {
                // Solve |p + t rest| = r for the first t.
                let a = rest.dot(rest);
                let b = 2.0 * p.dot(rest);
                let c = p.dot(p) - r * r;
                let disc = b * b - 4.0 * a * c;
                if b < 0.0 && disc >= 0.0 {
                    let t = (-b - disc.sqrt()) / (2.0 * a);
                    (t <= 1.0).then(|| (t.max(0.0), (p + rest * t).normalize_or(Vec2::X)))
                } else {
                    None
                }
            };
            if let Some((t, n)) = hit
                && first.is_none_or(|(ft, _, _)| t < ft)
            {
                first = Some((t, n, i));
            }
        }
        let Some((t, n, i)) = first else {
            moved += rest;
            break;
        };
        moved += rest * t;
        pos += rest * t;
        let remaining = rest * (1.0 - t);
        rest = remaining - n * remaining.dot(n).min(0.0);
        blocker = Some(i);
    }
    (Vec3::new(moved.x, delta.y, moved.y), blocker)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cyl(x: f32, z: f32) -> Cylinder {
        Cylinder {
            centre: Vec3::new(x, 0.0, z),
            radius: 0.4,
            half_height: 1.0,
        }
    }

    #[test]
    fn stops_at_contact_head_on() {
        let (d, b) = clip_move(&cyl(0.0, 0.0), Vec3::new(2.0, 0.0, 0.0), &[cyl(1.5, 0.0)]);
        assert_eq!(b, Some(0));
        assert!((d.x - (1.5 - 0.8 - SKIN)).abs() < 1e-4, "{d}");
        assert!(d.z.abs() < 1e-4);
    }

    #[test]
    fn slides_around_glancing() {
        let (d, b) = clip_move(&cyl(0.0, 0.0), Vec3::new(2.0, 0.0, 0.0), &[cyl(1.5, 0.5)]);
        assert_eq!(b, Some(0));
        // Deflected away from the other cylinder (negative z), still moving on.
        assert!(d.z < 0.0 && d.x > 0.5, "{d}");
        let end = Vec2::new(d.x, d.z);
        assert!(end.distance(Vec2::new(1.5, 0.5)) >= 0.8, "{d}");
    }

    #[test]
    fn ignores_cylinders_above() {
        let mut high = cyl(1.0, 0.0);
        high.centre.y = 2.5;
        let (d, b) = clip_move(&cyl(0.0, 0.0), Vec3::new(2.0, 0.0, 0.0), &[high]);
        assert_eq!(b, None);
        assert_eq!(d.x, 2.0);
    }

    #[test]
    fn overlapping_can_move_apart_not_closer() {
        let me = cyl(0.0, 0.0);
        let other = [cyl(0.5, 0.0)];
        let (away, _) = clip_move(&me, Vec3::new(-0.3, 0.0, 0.0), &other);
        assert!((away.x + 0.3).abs() < 1e-5);
        let (into, b) = clip_move(&me, Vec3::new(0.3, 0.0, 0.0), &other);
        assert_eq!(b, Some(0));
        assert!(into.x.abs() < 1e-5);
    }

    #[test]
    fn keeps_vertical_part() {
        let (d, _) = clip_move(&cyl(0.0, 0.0), Vec3::new(1.0, -0.2, 0.0), &[cyl(1.5, 0.0)]);
        assert_eq!(d.y, -0.2);
    }
}
