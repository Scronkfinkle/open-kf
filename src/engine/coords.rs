//! The one place where Unreal coordinates become Bevy coordinates.
//!
//! Unreal: X forward, Y right, Z up (left-handed), units of roughly 2 cm.
//! Bevy:   X right, Y up, -Z forward (right-handed), metres.
//!
//! Mapping: bevy = (ue.y, ue.z, -ue.x) * SCALE. This swaps handedness, so
//! triangle winding must be reversed when meshes cross this boundary.

use bevy::math::{Mat3, Quat, Vec3};
use ue_assets::properties::Rotator;

/// Metres per Unreal unit. To be confirmed against player height in milestone 2.
pub const SCALE: f32 = 1.0 / 50.0;

/// The axis permutation as a matrix: bevy_vector = C * ue_vector.
pub fn c() -> Mat3 {
    Mat3::from_cols(
        Vec3::new(0.0, 0.0, -1.0), // where ue X goes
        Vec3::new(1.0, 0.0, 0.0),  // where ue Y goes
        Vec3::new(0.0, 1.0, 0.0),  // where ue Z goes
    )
}

/// Position or offset, with scaling.
pub fn pos(v: [f32; 3]) -> Vec3 {
    Vec3::new(v[1], v[2], -v[0]) * SCALE
}

/// Direction (normals), no scaling.
pub fn dir(v: [f32; 3]) -> Vec3 {
    Vec3::new(v[1], v[2], -v[0])
}

/// Per-axis scale given in Unreal local axes, reordered to Bevy local axes.
pub fn scale(s: [f32; 3]) -> Vec3 {
    Vec3::new(s[1], s[2], s[0])
}

/// Unreal rotation matrix (columns are the rotated X, Y, Z axes), using the
/// standard Unreal convention. 65536 units = one full turn.
pub fn ue_rotation_matrix(r: Rotator) -> Mat3 {
    let k = std::f32::consts::TAU / 65536.0;
    let (sp, cp) = (r.pitch as f32 * k).sin_cos();
    let (sy, cy) = (r.yaw as f32 * k).sin_cos();
    let (sr, cr) = (r.roll as f32 * k).sin_cos();
    Mat3::from_cols(
        Vec3::new(cp * cy, cp * sy, sp),
        Vec3::new(sr * sp * cy - cr * sy, sr * sp * sy + cr * cy, -sr * cp),
        Vec3::new(-(cr * sp * cy + sr * sy), cy * sr - cr * sp * sy, cr * cp),
    )
}

/// The inverse of `ue_rotation_matrix`: (pitch, yaw, roll) in Unreal
/// rotation units for a rotation given by its X, Y, Z axis columns.
pub fn ue_rotator_of(m: Mat3) -> Vec3 {
    let k = 65536.0 / std::f32::consts::TAU;
    let (x, y, z) = (m.col(0), m.col(1), m.col(2));
    let pitch = x.z.clamp(-1.0, 1.0).asin();
    let yaw = x.y.atan2(x.x);
    let roll = (-y.z).atan2(z.z);
    Vec3::new(pitch, yaw, roll) * k
}

/// Unreal rotator as a Bevy rotation.
pub fn rotation(r: Rotator) -> Quat {
    let c = c();
    Quat::from_mat3(&(c * ue_rotation_matrix(r) * c.transpose()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-4
    }

    #[test]
    fn axes_map_as_documented() {
        assert!(close(dir([1.0, 0.0, 0.0]), Vec3::NEG_Z)); // forward
        assert!(close(dir([0.0, 1.0, 0.0]), Vec3::X)); // right
        assert!(close(dir([0.0, 0.0, 1.0]), Vec3::Y)); // up
        assert!(close(pos([50.0, 0.0, 0.0]), Vec3::new(0.0, 0.0, -1.0)));
    }

    #[test]
    fn rotation_agrees_with_converting_rotated_vectors() {
        // Rotating in Unreal space then converting must equal converting then
        // rotating in Bevy space.
        let r = Rotator { pitch: 3000, yaw: 12000, roll: -7000 };
        let m = ue_rotation_matrix(r);
        let q = rotation(r);
        for v in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.3, -0.5, 0.8]] {
            let ue_rotated = m * Vec3::from_array(v);
            assert!(close(dir(ue_rotated.to_array()), q * dir(v)));
        }
    }

    #[test]
    fn yaw_quarter_turn_faces_unreal_y() {
        // vector(Rotator) in UnrealScript: yaw 16384 points along +Y.
        let fwd = ue_rotation_matrix(Rotator { pitch: 0, yaw: 16384, roll: 0 }) * Vec3::X;
        assert!(close(fwd, Vec3::Y));
    }

    #[test]
    fn rotation_is_proper() {
        let m = ue_rotation_matrix(Rotator { pitch: 1234, yaw: -5678, roll: 9012 });
        assert!((m.determinant() - 1.0).abs() < 1e-4);
    }
}

#[cfg(test)]
mod rotator_tests {
    use super::*;

    #[test]
    fn rotator_round_trip() {
        for (p, y, r) in [(0, 0, 0), (3000, -16384, 1200), (-12000, 30000, -20000), (16000, 5000, 9000)] {
            let back = ue_rotator_of(ue_rotation_matrix(Rotator { pitch: p, yaw: y, roll: r }));
            let m1 = ue_rotation_matrix(Rotator { pitch: p, yaw: y, roll: r });
            let m2 = ue_rotation_matrix(Rotator {
                pitch: back.x.round() as i32,
                yaw: back.y.round() as i32,
                roll: back.z.round() as i32,
            });
            assert!((m1 - m2).abs().to_cols_array().iter().all(|d| *d < 1e-3), "{p} {y} {r} -> {back}");
        }
    }
}
