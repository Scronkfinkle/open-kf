//! Moving map textures: the TexPanner and TexOscillator (OT_Pan) movements
//! of map materials (KF-Offices' rain sheets, scrolling signs, water).
//!
//! The map loader (map.rs) records, for each material whose texture is
//! reached through such modifiers, every Bevy material drawn with it (the
//! plain one and its unlit, baked and sky-sorted copies, or the additive
//! one). Each frame the texture-coordinate offset at the game time is
//! written into them. What KF does (its native code; details in the local
//! RE.md): TexPanner moves the coordinates by PanDirection x PanRate x time,
//! each part wrapped to (-8, 8); TexOscillator OT_Pan by Amplitude x
//! sin(2 pi x (fraction of Rate x time + Phase)). Assumed, not read from KF:
//! the time is the level's game time (stops when paused).

use bevy::math::Affine2;
use bevy::prelude::*;

use crate::engine::runlog;
use crate::world::map_change::MapResourceExt;
use ue_assets::material::UvAnim;

/// A Bevy material drawn with a moving map material.
#[derive(Clone)]
pub enum UvTarget {
    Standard(Handle<StandardMaterial>),
    Baked(Handle<crate::render::baked::BakedMaterial>),
    Additive(Handle<crate::render::particles::BlendMaterial>),
}

/// One moving map material.
pub struct UvAnimEntry {
    /// The Unreal material, for logs.
    pub name: String,
    /// Its movements, innermost first (offsets add up).
    pub anims: Vec<UvAnim>,
    pub targets: Vec<UvTarget>,
}

impl UvAnimEntry {
    pub fn offset(&self, t: f32) -> Vec2 {
        self.anims.iter().map(|a| Vec2::from(a.offset(t))).sum()
    }
}

/// The current map's moving materials (filled by the map loader).
#[derive(Resource, Default)]
pub struct MapUvAnims {
    pub entries: Vec<UvAnimEntry>,
    /// Game time of the first frame with this map's materials, and of the
    /// last `uv_anim_offset` log line.
    started: Option<f32>,
    last_log: Option<f32>,
}

impl MapUvAnims {
    pub fn new(entries: Vec<UvAnimEntry>) -> Self {
        MapUvAnims { entries, started: None, last_log: None }
    }
}

/// How long after a map loads the offsets are logged (twice a second), for
/// tests.
const LOG_SECONDS: f32 = 3.0;
/// At most this many materials in each `uv_anim_offset` round.
const LOG_ENTRIES: usize = 4;

pub struct UvAnimPlugin;

impl Plugin for UvAnimPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapUvAnims>().reset_on_map_unload::<MapUvAnims>().add_systems(Update, animate);
    }
}

fn animate(
    time: Res<Time>,
    mut anims: ResMut<MapUvAnims>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut baked: ResMut<Assets<crate::render::baked::BakedMaterial>>,
    mut additive: ResMut<Assets<crate::render::particles::BlendMaterial>>,
) {
    if anims.entries.is_empty() {
        return;
    }
    let t = time.elapsed_secs();
    let since = t - *anims.started.get_or_insert(t);
    for e in &anims.entries {
        let off = e.offset(t);
        for target in &e.targets {
            match target {
                UvTarget::Standard(h) => {
                    if let Some(mut m) = standard.get_mut(h) {
                        m.uv_transform = Affine2::from_translation(off);
                    }
                }
                UvTarget::Baked(h) => {
                    if let Some(mut m) = baked.get_mut(h) {
                        m.base.uv_transform = Affine2::from_translation(off);
                    }
                }
                UvTarget::Additive(h) => {
                    if let Some(mut m) = additive.get_mut(h) {
                        m.uv_offset = off.extend(0.0).extend(0.0);
                    }
                }
            }
        }
    }
    if since <= LOG_SECONDS && anims.last_log.is_none_or(|l| t - l >= 0.5) {
        anims.last_log = Some(t);
        for e in anims.entries.iter().take(LOG_ENTRIES) {
            let off = e.offset(t);
            runlog::kv("uv_anim_offset", &format!("material={} t={t:.2} offset=({:.4}, {:.4}) targets={}", e.name, off.x, off.y, e.targets.len()));
        }
    }
}
