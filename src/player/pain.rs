//! Pain volumes and KillZ. PhysicsVolume with bPainCausing (KF maps:
//! LavaVolume, set to 1-2 DamagePerSec of Burned): a pawn entering takes
//! int(DamagePerSec) (CausePainTo) and everything inside takes it again
//! every second (the volume's VolumeTimer). ZoneInfo.KillZ: a pawn below its
//! zone's KillZ falls out of the world and dies (no map changes the default
//! -10000). See DESIGN.md, "Map fixes" M3.

use std::collections::HashSet;
use std::rc::Rc;

use bevy::prelude::*;
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package::ObjectRef;
use ue_assets::package_set::LoadedPackage;
use ue_assets::properties::{Value, read_export_properties};

use crate::engine::coords::SCALE;
use crate::engine::runlog;

pub struct PainVolume {
    pub name: String,
    pub polys: Vec<Vec<Vec3>>,
    pub damage_per_sec: f32,
    /// Seconds to the next VolumeTimer pop (None: no timer running).
    timer: Option<f32>,
    /// Pawns inside last frame (0 = the player, zed id + 1).
    inside: HashSet<usize>,
}

#[derive(Resource, Default)]
pub struct PainVolumes(pub Vec<PainVolume>);

/// Damage from the level to a zed (no instigator: no kill credit).
#[derive(Message, Clone, Copy, Debug)]
pub struct LevelDamageZed {
    pub zed: usize,
    pub amount: f32,
    pub cause: &'static str,
}

/// ZoneInfo.KillZ default (Engine.ZoneInfo).
const KILL_Z: f32 = -10000.0;

/// Reads the map's pain-causing PhysicsVolumes.
pub fn load(map: &Rc<LoadedPackage>, defaults: &ClassDefaults) -> PainVolumes {
    let pkg = &map.pkg;
    let mut out = Vec::new();
    for i in pkg.level_actor_exports() {
        let Some(class) = defaults.class_of(map, i) else { continue };
        if !pkg.export_class_name(i).ends_with("Volume") || !defaults.is_a(&class, "PhysicsVolume") {
            continue;
        }
        let Ok(props) = read_export_properties(pkg, i) else { continue };
        let pain = matches!(defaults.actor_value(map, i, &props, "bPainCausing"), Some(Value::Bool(true)));
        let dps = match defaults.actor_value(map, i, &props, "DamagePerSec") {
            Some(Value::Float(f)) => f,
            _ => 0.0,
        };
        if !pain || dps <= 0.0 {
            continue;
        }
        out.push(PainVolume {
            name: pkg.object_name(ObjectRef::Export(i)).to_string(),
            polys: crate::world::zvolume::brush_polys(pkg, &props),
            damage_per_sec: dps,
            timer: None,
            inside: HashSet::new(),
        });
    }
    runlog::kv(
        "pain_volumes",
        &format!("count={} [{}]", out.len(), out.iter().map(|v| format!("{}:{}", v.name, v.damage_per_sec)).collect::<Vec<_>>().join(" ")),
    );
    PainVolumes(out)
}

pub struct PainPlugin;

impl Plugin for PainPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PainVolumes>().add_message::<LevelDamageZed>().add_systems(Update, pain_and_kill_z);
    }
}

/// Touching a brush: the cylinder's centre, top or bottom is inside it.
fn touches(polys: &[Vec<Vec3>], centre: Vec3, half_height: f32) -> bool {
    [0.0, half_height, -half_height].iter().any(|dz| crate::world::zvolume::encompasses(polys, centre + Vec3::Z * *dz))
}

fn pain_and_kill_z(
    time: Res<Time>,
    mut volumes: ResMut<PainVolumes>,
    player: Query<&crate::player::walk::Walker>,
    zeds: Query<&crate::zeds::zed::Zed>,
    mut player_damage: MessageWriter<crate::game::combat::PlayerDamaged>,
    mut zed_damage: MessageWriter<LevelDamageZed>,
) {
    let dt = time.delta_secs();
    let ue = |p: Vec3| Vec3::new(-p.z, p.x, p.y) / SCALE;
    // (key, centre, half height, zed id)
    let mut pawns: Vec<(usize, Vec3, f32, Option<usize>)> = Vec::new();
    if let Ok(w) = player.single() {
        pawns.push((0, ue(w.center), 50.0, None));
    }
    for z in &zeds {
        if !z.is_dead() {
            pawns.push((z.id + 1, ue(z.centre), z.half_height, Some(z.id)));
        }
    }
    let hurt = |amount: f32, zed: Option<usize>, cause: &'static str, pd: &mut MessageWriter<crate::game::combat::PlayerDamaged>, zd: &mut MessageWriter<LevelDamageZed>| match zed {
        Some(id) => {
            zd.write(LevelDamageZed { zed: id, amount, cause });
        }
        None => {
            // FellOutOfWorld kills through Died(Gibbed), whose bArmorStops is
            // false; pain volumes (Burned) go through the armour.
            let armor_stops = cause != "fell_out_of_world";
            pd.write(crate::game::combat::PlayerDamaged { amount, zed_id: crate::game::combat::LEVEL_DAMAGE, kind: crate::game::combat::HurtKind::Plain, armor_stops, dam_type: crate::game::combat::DamType::Other, source: None, dam: None, to_peer: None });
        }
    };
    // KillZ: FellOutOfWorld.
    for &(_, c, _, zed) in &pawns {
        if c.z < KILL_Z {
            runlog::kv("fell_out_of_world", &format!("pawn={} z={:.0}", zed.map_or("player".into(), |id| format!("zed{id}")), c.z));
            hurt(10000.0, zed, "fell_out_of_world", &mut player_damage, &mut zed_damage);
        }
    }
    for v in volumes.0.iter_mut() {
        let now_inside: HashSet<usize> = pawns.iter().filter(|(_, c, h, _)| touches(&v.polys, *c, *h)).map(|p| p.0).collect();
        let amount = v.damage_per_sec.trunc();
        // Touch: pain on entering; the timer starts (1 s, repeating).
        for p in pawns.iter().filter(|p| now_inside.contains(&p.0) && !v.inside.contains(&p.0)) {
            runlog::kv("pain_volume_enter", &format!("volume={} pawn={} damage={amount}", v.name, p.3.map_or("player".into(), |id| format!("zed{id}"))));
            hurt(amount, p.3, "pain_volume", &mut player_damage, &mut zed_damage);
            v.timer.get_or_insert(1.0);
        }
        if let Some(t) = v.timer.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                // TimerPop: everything touching hurts; nobody left: timer gone.
                if now_inside.is_empty() {
                    v.timer = None;
                } else {
                    *t += 1.0;
                    for p in pawns.iter().filter(|p| now_inside.contains(&p.0)) {
                        hurt(amount, p.3, "pain_volume", &mut player_damage, &mut zed_damage);
                    }
                }
            }
        }
        v.inside = now_inside;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touching_uses_centre_top_or_bottom() {
        let c = |x: f32, y: f32, z: f32| Vec3::new(x, y, z) * 100.0;
        let slab = vec![
            vec![c(-1., -1., -1.), c(1., -1., -1.), c(1., 1., -1.), c(-1., 1., -1.)],
            vec![c(-1., -1., 0.), c(-1., 1., 0.), c(1., 1., 0.), c(1., -1., 0.)],
            vec![c(-1., -1., -1.), c(-1., -1., 0.), c(1., -1., 0.), c(1., -1., -1.)],
            vec![c(-1., 1., -1.), c(1., 1., -1.), c(1., 1., 0.), c(-1., 1., 0.)],
            vec![c(-1., -1., -1.), c(-1., 1., -1.), c(-1., 1., 0.), c(-1., -1., 0.)],
            vec![c(1., -1., -1.), c(1., -1., 0.), c(1., 1., 0.), c(1., 1., -1.)],
        ];
        // Standing on top of a 100-deep pool: the feet (centre - 50) dip in.
        assert!(touches(&slab, Vec3::new(0.0, 0.0, 30.0), 50.0));
        assert!(!touches(&slab, Vec3::new(0.0, 0.0, 60.0), 50.0));
    }
}
