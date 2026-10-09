//! The muzzle-flash light, the local player's side (DESIGN.md,
//! "Muzzle-flash light"): your own attachment's ThirdPersonEffects runs on
//! every shot and, seen in first person, lights the first-person weapon
//! with the weapon class's light for 0.15 s. This fills in the local
//! pawn's `MuzzleLight` (weapons/muzzle_light.rs places the light).

use bevy::prelude::*;

use super::Weapons;
use crate::engine::camera::FlyCamera;
use crate::weapons::muzzle_light::MuzzleLight;

/// The shots fired since the light last looked, and where the weapon is.
#[derive(Debug, Default)]
pub(super) struct LocalMuzzle {
    /// Fire mode of each shot this frame (the shot counter behind
    /// FlashCount, with its FiringMode).
    shots: Vec<u8>,
    /// The first-person weapon's location (Bevy world), as last placed.
    pub weapon_pos: Option<Vec3>,
}

impl LocalMuzzle {
    /// A shot of fire mode `mode` (FlashCount goes up).
    pub fn shot(&mut self, mode: usize) {
        self.shots.push(mode.min(1) as u8);
    }
}

pub(super) fn local_muzzle_light(mut commands: Commands, weapons: Option<ResMut<Weapons>>, mut holder: Query<(Entity, Option<&mut MuzzleLight>), With<FlyCamera>>) {
    let Some(mut w) = weapons else { return };
    let Ok((entity, light)) = holder.single_mut() else { return };
    let Some(mut m) = light else {
        commands.entity(entity).insert(MuzzleLight::default());
        return;
    };
    let shots = std::mem::take(&mut w.muzzle.shots);
    let Some(def) = w.defs.get(w.current) else { return };
    m.set_weapon(&def.class, def.muzzle_light, "local");
    m.pos = w.muzzle.weapon_pos;
    if shots.iter().any(|&mode| def.attachment_rule.allows(mode)) {
        m.flash();
    }
}
