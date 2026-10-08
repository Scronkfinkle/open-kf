//! Physics volumes: gravity and ZoneVelocity per place in the map. Every
//! map has a default physics volume (the whole level); brush volumes
//! (PhysicsVolume, KFPhysicsVolume, WaterVolume, LavaVolume...) can
//! override it where they are. A pawn uses the highest-priority volume
//! whose brush contains its centre: starting from the default volume, a
//! volume replaces the current pick only with a strictly higher Priority.
//! Unreal units and axes. See DESIGN.md, "Player movement details from
//! KF" PM7 (details in the local RE.md).

use std::rc::Rc;

use bevy::prelude::*;
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package::ObjectRef;
use ue_assets::package_set::LoadedPackage;
use ue_assets::properties::{Value, read_export_properties};

use crate::engine::runlog;

/// Engine.PhysicsVolume default Gravity.Z.
pub const DEFAULT_GRAVITY_Z: f32 = -950.0;

#[derive(Clone, Debug, PartialEq)]
pub struct PhysicsVolume {
    pub name: String,
    pub priority: i32,
    /// Unreal units/s^2 (default (0, 0, -950)).
    pub gravity: Vec3,
    /// Added to a falling pawn's movement (not kept in its velocity).
    pub zone_velocity: Vec3,
    /// The brush in world space; empty for the default volume.
    pub polys: Vec<Vec<Vec3>>,
}

impl PhysicsVolume {
    /// The level's default volume when the map does not save one.
    pub fn engine_default() -> Self {
        PhysicsVolume {
            name: "DefaultPhysicsVolume".into(),
            priority: 0,
            gravity: Vec3::new(0.0, 0.0, DEFAULT_GRAVITY_Z),
            zone_velocity: Vec3::ZERO,
            polys: Vec::new(),
        }
    }
}

#[derive(Resource, Debug, Clone)]
pub struct PhysicsVolumes {
    pub default: PhysicsVolume,
    pub volumes: Vec<PhysicsVolume>,
}

impl Default for PhysicsVolumes {
    fn default() -> Self {
        PhysicsVolumes { default: PhysicsVolume::engine_default(), volumes: Vec::new() }
    }
}

impl PhysicsVolumes {
    /// The volume a pawn whose centre is at `p` (Unreal units) is in.
    pub fn at(&self, p: Vec3) -> &PhysicsVolume {
        let mut best = &self.default;
        for v in &self.volumes {
            if v.priority > best.priority && crate::world::zvolume::encompasses(&v.polys, p) {
                best = v;
            }
        }
        best
    }
}

/// Reads the map's default physics volume and its brush physics volumes
/// that change gravity or ZoneVelocity (the only parts used so far).
pub fn load(map: &Rc<LoadedPackage>, defaults: &ClassDefaults) -> PhysicsVolumes {
    let pkg = &map.pkg;
    let mut out = PhysicsVolumes::default();
    let mut saved_default = 0;
    // The level's own actors only (deleted ones stay in the file).
    for i in pkg.level_actor_exports() {
        if !pkg.export_class_name(i).ends_with("Volume") {
            continue;
        }
        let Some(class) = defaults.class_of(map, i) else { continue };
        if !defaults.is_a(&class, "PhysicsVolume") {
            continue;
        }
        let Ok(props) = read_export_properties(pkg, i) else { continue };
        let value = |n: &str| defaults.actor_value(map, i, &props, n);
        let vector = |n: &str, d: Vec3| match value(n) {
            Some(Value::Vector(v)) => Vec3::from_array(v),
            _ => d,
        };
        let priority = match value("Priority") {
            Some(Value::Int(p)) => p,
            _ => 0,
        };
        let is_default = defaults.is_a(&class, "DefaultPhysicsVolume");
        let v = PhysicsVolume {
            name: pkg.object_name(ObjectRef::Export(i)).to_string(),
            priority,
            gravity: vector("Gravity", Vec3::new(0.0, 0.0, DEFAULT_GRAVITY_Z)),
            zone_velocity: vector("ZoneVelocity", Vec3::ZERO),
            polys: if is_default { Vec::new() } else { crate::world::zvolume::brush_polys(pkg, &props) },
        };
        if is_default {
            saved_default += 1;
            out.default = v;
        } else if v.gravity.z != DEFAULT_GRAVITY_Z || v.gravity.x != 0.0 || v.gravity.y != 0.0 || v.zone_velocity != Vec3::ZERO {
            out.volumes.push(v);
        } else if v.priority > out.default.priority {
            // Default gravity, but it can still win over a changed default
            // volume (none in KF's maps).
            out.volumes.push(v);
        }
    }
    let d = &out.default;
    runlog::kv(
        "physics_volumes",
        &format!(
            "default={} saved_defaults={saved_default} default_gravity_z={} default_zone_velocity=({}, {}, {}) default_priority={} volumes={} [{}]",
            d.name,
            d.gravity.z,
            d.zone_velocity.x,
            d.zone_velocity.y,
            d.zone_velocity.z,
            d.priority,
            out.volumes.len(),
            out.volumes.iter().map(|v| format!("{}:p{}:g{}:zv{}", v.name, v.priority, v.gravity.z, v.zone_velocity.z)).collect::<Vec<_>>().join(" ")
        ),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(c: Vec3, r: f32) -> Vec<Vec<Vec3>> {
        let v = |x: f32, y: f32, z: f32| c + Vec3::new(x, y, z) * r;
        vec![
            vec![v(-1., -1., -1.), v(1., -1., -1.), v(1., 1., -1.), v(-1., 1., -1.)],
            vec![v(-1., -1., 1.), v(1., -1., 1.), v(1., 1., 1.), v(-1., 1., 1.)],
            vec![v(-1., -1., -1.), v(1., -1., -1.), v(1., -1., 1.), v(-1., -1., 1.)],
            vec![v(-1., 1., -1.), v(1., 1., -1.), v(1., 1., 1.), v(-1., 1., 1.)],
            vec![v(-1., -1., -1.), v(-1., 1., -1.), v(-1., 1., 1.), v(-1., -1., 1.)],
            vec![v(1., -1., -1.), v(1., 1., -1.), v(1., 1., 1.), v(1., -1., 1.)],
        ]
    }

    fn vol(name: &str, priority: i32, g: f32, c: Vec3) -> PhysicsVolume {
        PhysicsVolume { name: name.into(), priority, gravity: Vec3::new(0.0, 0.0, g), zone_velocity: Vec3::ZERO, polys: cube(c, 100.0) }
    }

    #[test]
    fn highest_priority_containing_volume_wins() {
        let mut default = PhysicsVolume::engine_default();
        default.priority = 1;
        default.gravity.z = -320.0;
        let vols = PhysicsVolumes {
            default,
            // As KF-MoonBase: a priority 0 volume never beats the default
            // (priority 1); priority 2 volumes do.
            volumes: vec![vol("zero", 0, -950.0, Vec3::ZERO), vol("low", 2, -150.0, Vec3::new(1000.0, 0.0, 0.0))],
        };
        assert_eq!(vols.at(Vec3::ZERO).gravity.z, -320.0);
        assert_eq!(vols.at(Vec3::new(1000.0, 50.0, 0.0)).name, "low");
        assert_eq!(vols.at(Vec3::new(5000.0, 0.0, 0.0)).gravity.z, -320.0);
    }

    #[test]
    fn equal_priority_keeps_the_first() {
        let vols = PhysicsVolumes {
            default: PhysicsVolume::engine_default(),
            volumes: vec![vol("a", 2, -100.0, Vec3::ZERO), vol("b", 2, -200.0, Vec3::new(50.0, 0.0, 0.0))],
        };
        assert_eq!(vols.at(Vec3::new(25.0, 0.0, 0.0)).name, "a");
    }
}
