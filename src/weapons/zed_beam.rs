//! The ZED Gun's beam, drawn. KF's ZEDBeamEffect is an xEmitter beam
//! (mParticleType 6, a wavy textured strip from the gun to EndEffect),
//! which the particle code does not support. Here it is a straight strip
//! with the effect's own texture (Skins[0]), drawn additively (Style 6),
//! turned to face the camera about its length. The wave (mWaveAmplitude,
//! mBendStrength), the sparks (ZedBeamSparks) and the splash sphere
//! (ZEDBeamSplashEffect) are not drawn: an approximation.

use bevy::prelude::*;
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package::ObjectRef;
use ue_assets::package_set::PackageSet;
use ue_assets::properties::Value;

use crate::engine::camera::FlyCamera;
use crate::engine::coords::{self, SCALE};
use crate::world::map::MapRequest;
use crate::weapons::projectile::BeamView;
use crate::engine::runlog;

/// Strip width, Unreal units: twice ZEDBeamEffect's mSizeRange (6), read
/// as a half-width. A guess: the native beam code is not in the scripts.
const BEAM_WIDTH: f32 = 12.0;

const BEAM_CLASS: &str = "KFMod.ZEDBeamEffect";

pub struct ZedBeamPlugin;

impl Plugin for ZedBeamPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, spawn_beam)
            .add_systems(PostUpdate, place_beam.before(bevy::transform::TransformSystems::Propagate));
    }
}

#[derive(Component)]
struct BeamStrip;

fn spawn_beam(
    mut commands: Commands,
    request: Res<MapRequest>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let set = PackageSet::new(&request.install_root);
    let defaults = ClassDefaults::new(&set);
    // Skins is saved as a packed array of object references.
    let skin = crate::zeds::gore::find_class(&set, BEAM_CLASS).and_then(|class| match defaults.get(&class, "Skins") {
        Some((Value::Array { count, raw }, pkg)) if count > 0 => {
            let mut r = ue_assets::reader::Reader::new(&raw);
            let first = r.compact_index().ok().map(ObjectRef::from_raw)?;
            // ZED_FX_Beam_FB is a FinalBlend: follow it to its texture.
            let from = ue_assets::package_set::ObjectHandle { package: pkg, export: 0 };
            Some(ue_assets::material::resolve(&set, &from, first))
        }
        _ => None,
    });
    let texture = skin
        .as_ref()
        .and_then(|m| m.texture.as_ref())
        .and_then(|h| crate::render::particles::decode(h, true, false, &mut images));
    runlog::kv(
        "zed_beam_ready",
        &format!("chain={:?} texture={}", skin.as_ref().map(|m| m.chain.clone()), texture.is_some()),
    );
    let material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: texture,
        unlit: true,
        alpha_mode: AlphaMode::Add,
        cull_mode: None,
        double_sided: true,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(1.0, 1.0))),
        MeshMaterial3d(material),
        Transform::default(),
        Visibility::Hidden,
        bevy::light::NotShadowCaster,
        BeamStrip,
    ));
}

/// Stretches the strip from the beam start to its end, facing the camera.
fn place_beam(
    view: Res<BeamView>,
    camera: Query<&Transform, (With<FlyCamera>, Without<BeamStrip>)>,
    mut strip: Query<(&mut Transform, &mut Visibility), With<BeamStrip>>,
) {
    let Ok((mut t, mut vis)) = strip.single_mut() else {
        return;
    };
    let (a, b) = (coords::pos(view.start.to_array()), coords::pos(view.end.to_array()));
    let len = (b - a).length();
    let Ok(cam) = camera.single() else {
        return;
    };
    if !view.active || len < 1e-3 {
        *vis = Visibility::Hidden;
        return;
    }
    let along = (b - a) / len;
    let mid = (a + b) * 0.5;
    let side = along.cross(cam.translation - mid).normalize_or(Vec3::Y);
    t.translation = mid;
    t.rotation = Quat::from_mat3(&Mat3::from_cols(along, side, along.cross(side)));
    t.scale = Vec3::new(len, BEAM_WIDTH * SCALE, 1.0);
    *vis = Visibility::Visible;
}
