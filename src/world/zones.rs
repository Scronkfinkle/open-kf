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
use crate::world::map_change::{MapEpoch, OncePerMap};

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
    /// ZoneInfo.DistanceFogBlendTime (seconds, default 1): how long the
    /// view's fog takes to fade to this zone's on entering it.
    pub blend_time: f32,
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

/// KF's far clipping distance: "no fog" fades toward start = end = this.
const FAR_PLANE: f32 = 65536.0;

/// The view's fog as drawn: start / end (Unreal units) and colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogValues {
    pub start: f32,
    pub end: f32,
    pub color: [u8; 3],
}

/// KF's per-player fog fade state: the fog last settled on, the zone it
/// belongs to, and the time since the last zone change (game time).
#[derive(Resource, Debug)]
pub struct FogBlend {
    last_zone: Option<usize>,
    timer: f32,
    last: FogValues,
}

impl Default for FogBlend {
    fn default() -> Self {
        Self { last_zone: None, timer: 1e6, last: FogValues { start: FAR_PLANE, end: FAR_PLANE, color: [0; 3] } }
    }
}

impl FogBlend {
    /// One frame of KF's fog fade (details in the local RE.md). On a zone
    /// change the fade restarts (a change from zone 0 or the first one
    /// snaps); the fog moves linearly from the last settled fog to the new
    /// zone's over its DistanceFogBlendTime; a zone without fog fades to
    /// start = end = the far plane and then switches fog off; zone 0
    /// snaps. Returns the fog to draw (None: off) and the fade fraction.
    pub fn step(&mut self, dt: f32, zone: usize, z: Option<&ZoneFog>) -> (Option<FogValues>, f32) {
        self.timer += dt;
        let fog = z.is_some_and(|z| z.fog);
        if self.last_zone != Some(zone) {
            if let Some(z) = z.filter(|_| fog)
                && self.last.end == FAR_PLANE
            {
                self.last.color = [z.color[0], z.color[1], z.color[2]];
            }
            self.timer = if matches!(self.last_zone, None | Some(0)) { 1e6 } else { 0.0 };
            self.last_zone = Some(zone);
        }
        let blend_time = z.map_or(1.0, |z| z.blend_time);
        let mut delta = if blend_time <= 0.0 { 1.0 } else { self.timer / blend_time };
        let target = match z.filter(|_| fog) {
            Some(z) => FogValues { start: z.start, end: z.end, color: [z.color[0], z.color[1], z.color[2]] },
            None => FogValues { start: FAR_PLANE, end: FAR_PLANE, color: self.last.color },
        };
        if delta > 1.0 || zone == 0 {
            self.last = target;
            self.timer = 1e6;
            delta = 1.0;
        }
        let l = self.last;
        // Colour bytes are cut off, not rounded (KF).
        let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * delta) as u8;
        let current = FogValues {
            start: l.start + (target.start - l.start) * delta,
            end: l.end + (target.end - l.end) * delta,
            color: [mix(l.color[0], target.color[0]), mix(l.color[1], target.color[1]), mix(l.color[2], target.color[2])],
        };
        ((fog || delta < 1.0).then_some(current), delta)
    }
}

pub struct ZonesPlugin;

impl Plugin for ZonesPlugin {
    fn build(&self, app: &mut App) {
        use crate::world::map_change::MapResourceExt;
        // Per map (world/map_change.rs): the map loader inserts `Zones`.
        app.init_resource::<PlayerZone>()
            .init_resource::<FogBlend>()
            .remove_on_map_unload::<Zones>()
            .reset_on_map_unload::<PlayerZone>()
            .reset_on_map_unload::<FogBlend>()
            .add_systems(Update, track_player_zone);
    }
}

/// Which zone the player (camera) is in; the camera's fog follows it.
fn track_player_zone(
    zones: Option<Res<Zones>>,
    mut player: ResMut<PlayerZone>,
    (mut clear, mut blend, time): (ResMut<ClearColor>, ResMut<FogBlend>, Res<Time>),
    mut commands: Commands,
    cams: Query<(Entity, &Transform, Option<&DistanceFog>), With<crate::engine::camera::FlyCamera>>,
    (mut last, mut applied, mut seen): (Local<Option<usize>>, Local<Option<FogValues>>, Local<OncePerMap>),
    epoch: Res<MapEpoch>,
) {
    let Some(zones) = zones else { return };
    // A new map: its zone numbers are not the old map's.
    if !seen.done(&epoch) {
        seen.set(&epoch);
        *last = None;
        *applied = None;
    }
    let Ok((cam, t, drawn)) = cams.single() else { return };
    let p = t.translation / SCALE;
    let zone = zones.bsp.point_zone([-p.z, p.x, p.y]);
    player.zone = zone;
    player.fog_end = zones.fog_end(zone);
    let z = zones.zones.get(zone);
    if *last != Some(zone) {
        *last = Some(zone);
        runlog::kv(
            "player_zone",
            &format!(
                "zone={zone} name={} fog={} start={} end={} blend_time={}",
                z.map_or("?", |z| z.name.as_str()),
                z.is_some_and(|z| z.fog),
                z.map_or(0.0, |z| z.start),
                z.map_or(0.0, |z| z.end),
                z.map_or(0.0, |z| z.blend_time)
            ),
        );
        // What shows where nothing is drawn: black, or the fog colour when
        // the camera's zone (not zone 0) has fog and bClearToFogColor (KF's
        // rule).
        let clear_rgb = z
            .filter(|z| zone != 0 && z.fog && z.clear_to_fog)
            .map_or([0, 0, 0], |z| [z.color[0], z.color[1], z.color[2]]);
        let new_clear = Color::srgb_u8(clear_rgb[0], clear_rgb[1], clear_rgb[2]);
        if clear.0 != new_clear {
            clear.0 = new_clear;
            runlog::kv("clear_colour", &format!("zone={zone} rgb={},{},{}", clear_rgb[0], clear_rgb[1], clear_rgb[2]));
        }
    }
    // The drawn fog fades between zones (KF's per-player fog fade).
    let (fog, delta) = blend.step(time.delta_secs(), zone, z);
    if delta < 1.0 {
        runlog::kv(
            "fog_blend",
            &format!(
                "zone={zone} delta={delta:.3} start={:.0} end={:.0} colour={:?}",
                fog.map_or(0.0, |f| f.start),
                fog.map_or(0.0, |f| f.end),
                fog.map(|f| f.color)
            ),
        );
    }
    if drawn.is_some() && *applied == fog {
        return;
    }
    *applied = fog;
    match fog {
        Some(f) => {
            commands.entity(cam).insert(distance_fog(f.start, f.end, [f.color[0], f.color[1], f.color[2], 255]));
        }
        None => {
            commands.entity(cam).remove::<DistanceFog>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(start: f32, end: f32, c: u8) -> ZoneFog {
        ZoneFog {
            name: "z".into(),
            fog: true,
            start,
            end,
            color: [c, c, c, 255],
            clear_to_fog: false,
            blend_time: 1.0,
            overlay: None,
            ambient: [0, 0, 0],
            ambient_vector: None,
        }
    }

    #[test]
    fn fog_fades_over_blend_time() {
        let (a, b) = (zone(0.0, 4000.0, 100), zone(0.0, 2000.0, 200));
        let mut f = FogBlend::default();
        // First zone: snaps.
        let (fog, d) = f.step(0.016, 3, Some(&a));
        assert_eq!((fog.unwrap().end, d), (4000.0, 1.0));
        // Into zone 5 (the timer restarts at 0 that frame): halfway 0.5 s
        // later.
        f.step(0.016, 5, Some(&b));
        let (fog, d) = f.step(0.5, 5, Some(&b));
        assert!((d - 0.5).abs() < 1e-3, "delta {d}");
        let fog = fog.unwrap();
        assert!((fog.end - 3000.0).abs() < 1.0, "end {}", fog.end);
        assert_eq!(fog.color, [150; 3]);
        // Settled after 1 s.
        let (fog, d) = f.step(0.6, 5, Some(&b));
        assert_eq!((fog.unwrap().end, d), (2000.0, 1.0));
    }

    #[test]
    fn fogless_zone_fades_out_then_off() {
        let a = zone(0.0, 4000.0, 100);
        let mut none = zone(0.0, 0.0, 0);
        none.fog = false;
        let mut f = FogBlend::default();
        f.step(0.016, 3, Some(&a));
        f.step(0.0, 4, Some(&none));
        let (fog, _) = f.step(0.5, 4, Some(&none));
        let fog = fog.expect("still fading");
        assert!(fog.end > 4000.0 && fog.end < FAR_PLANE);
        assert_eq!(fog.color, [100; 3], "keeps the last colour");
        let (fog, _) = f.step(0.6, 4, Some(&none));
        assert!(fog.is_none(), "off after the fade");
    }

    #[test]
    fn zone_zero_snaps() {
        let a = zone(0.0, 4000.0, 100);
        let b = zone(0.0, 2000.0, 200);
        let mut f = FogBlend::default();
        f.step(0.016, 0, Some(&a));
        // Leaving zone 0 snaps too.
        let (fog, d) = f.step(0.016, 2, Some(&b));
        assert_eq!((fog.unwrap().end, d), (2000.0, 1.0));
    }
}
