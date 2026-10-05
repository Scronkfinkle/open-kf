//! Zones and distance fog. The BSP splits the level into zones, each with a
//! ZoneInfo (zone 0: the LevelInfo). KF's sight checks skip anything beyond
//! the fog of the zone the player is in ((!Zone.bDistanceFog || dist <
//! Zone.DistanceFogEnd): ZombieVolume.RateZombieVolume and
//! PlayerCanSeePoint, KFMonster.Tick), and the fog is drawn. See DESIGN.md,
//! "Map fixes" M2.

use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use ue_assets::bsp::Model;

use crate::coords::SCALE;
use crate::runlog;

/// A zone's fog (DistanceFogStart / End in Unreal units, colour as RGBA).
#[derive(Clone, Debug)]
pub struct ZoneFog {
    pub name: String,
    pub fog: bool,
    pub start: f32,
    pub end: f32,
    pub color: [u8; 4],
    /// The vision overlay's colour in this zone (overlay.rs): the fog
    /// colour, or KFOverlayColor with bNewKFColorCorrection; None with
    /// bNoKFColorCorrection.
    pub overlay: Option<[u8; 3]>,
}

/// The level's BSP (for point -> zone) and each zone's fog. Inserted by the
/// map loader.
#[derive(Resource)]
pub struct Zones {
    pub bsp: Model,
    pub zones: Vec<ZoneFog>,
    /// KFSPLevelInfo.bUseVisionOverlay (true without one).
    pub vision_overlay: bool,
}

impl Zones {
    /// DistanceFogEnd of a zone, if it has fog.
    pub fn fog_end(&self, zone: usize) -> Option<f32> {
        self.zones.get(zone).filter(|z| z.fog).map(|z| z.end)
    }
}

/// The zone the player is in now, and its fog end (Unreal units).
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct PlayerZone {
    pub zone: usize,
    pub fog_end: Option<f32>,
}

impl PlayerZone {
    /// KF's fog test: true if `dist` is inside the fog (or there is none).
    pub fn in_fog_range(&self, dist: f32) -> bool {
        self.fog_end.is_none_or(|e| dist < e)
    }
}

pub struct ZonesPlugin;

impl Plugin for ZonesPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlayerZone>().add_systems(Update, track_player_zone);
    }
}

/// Which zone the player (camera) is in; the camera's fog follows it.
fn track_player_zone(
    zones: Option<Res<Zones>>,
    mut player: ResMut<PlayerZone>,
    mut commands: Commands,
    cams: Query<(Entity, &Transform), With<crate::camera::FlyCamera>>,
    mut last: Local<Option<usize>>,
) {
    let Some(zones) = zones else { return };
    let Ok((cam, t)) = cams.single() else { return };
    let p = t.translation / SCALE;
    let zone = zones.bsp.point_zone([-p.z, p.x, p.y]);
    player.zone = zone;
    player.fog_end = zones.fog_end(zone);
    if *last == Some(zone) {
        return;
    }
    *last = Some(zone);
    let z = zones.zones.get(zone);
    runlog::kv(
        "player_zone",
        &format!(
            "zone={zone} name={} fog={} start={} end={}",
            z.map_or("?", |z| z.name.as_str()),
            z.is_some_and(|z| z.fog),
            z.map_or(0.0, |z| z.start),
            z.map_or(0.0, |z| z.end)
        ),
    );
    match z.filter(|z| z.fog) {
        Some(z) => {
            let c = z.color;
            commands.entity(cam).insert(DistanceFog {
                color: Color::srgb_u8(c[0], c[1], c[2]),
                falloff: FogFalloff::Linear {
                    start: z.start * SCALE,
                    end: z.end * SCALE,
                },
                ..default()
            });
        }
        None => {
            commands.entity(cam).remove::<DistanceFog>();
        }
    }
}
