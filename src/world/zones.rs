//! Zones and distance fog. The BSP splits the level into zones, each with a
//! ZoneInfo (zone 0: the LevelInfo). KF's sight checks skip anything beyond
//! the fog of the zone the player is in ((!Zone.bDistanceFog || dist <
//! Zone.DistanceFogEnd): ZombieVolume.RateZombieVolume and
//! PlayerCanSeePoint, KFMonster.Tick), and the fog is drawn. See DESIGN.md,
//! "Map fixes" M2.

use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use ue_assets::bsp::Model;

use crate::engine::coords::SCALE;
use crate::engine::runlog;

/// A zone's fog (DistanceFogStart / End in Unreal units, colour as RGBA).
#[derive(Clone, Debug)]
pub struct ZoneFog {
    pub name: String,
    pub fog: bool,
    pub start: f32,
    pub end: f32,
    pub color: [u8; 4],
    /// ZoneInfo.bClearToFogColor: with fog, KF clears the screen to the fog
    /// colour before drawing a view from this zone (otherwise black).
    pub clear_to_fog: bool,
    /// The vision overlay's colour in this zone (overlay.rs): the fog
    /// colour, or KFOverlayColor with bNewKFColorCorrection; None with
    /// bNoKFColorCorrection.
    pub overlay: Option<[u8; 3]>,
    /// ZoneInfo AmbientBrightness, AmbientHue, AmbientSaturation: the
    /// zone's ambient light on actors (render/actor_light.rs).
    pub ambient: [u8; 3],
    /// ZoneInfo.AmbientVector as saved in the map (the editor's
    /// FGetHSV of the three above), if saved.
    pub ambient_vector: Option<[f32; 3]>,
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

/// Bevy's linear distance fog for start / end in Unreal units and an sRGB
/// colour.
pub fn distance_fog(start: f32, end: f32, color: [u8; 4]) -> DistanceFog {
    DistanceFog {
        color: Color::srgb_u8(color[0], color[1], color[2]),
        falloff: FogFalloff::Linear { start: start * SCALE, end: end * SCALE },
        ..default()
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
    mut clear: ResMut<ClearColor>,
    mut commands: Commands,
    cams: Query<(Entity, &Transform), With<crate::engine::camera::FlyCamera>>,
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
    // What shows where nothing is drawn: black, or the fog colour when the
    // camera's zone (not zone 0) has fog and bClearToFogColor (KF's rule).
    let clear_rgb = z
        .filter(|z| zone != 0 && z.fog && z.clear_to_fog)
        .map_or([0, 0, 0], |z| [z.color[0], z.color[1], z.color[2]]);
    let new_clear = Color::srgb_u8(clear_rgb[0], clear_rgb[1], clear_rgb[2]);
    if clear.0 != new_clear {
        clear.0 = new_clear;
        runlog::kv("clear_colour", &format!("zone={zone} rgb={},{},{}", clear_rgb[0], clear_rgb[1], clear_rgb[2]));
    }
    match z.filter(|z| z.fog) {
        Some(z) => {
            commands.entity(cam).insert(distance_fog(z.start, z.end, z.color));
        }
        None => {
            commands.entity(cam).remove::<DistanceFog>();
        }
    }
}
