//! The view on another actor (G3b): PlayerController.SetViewTarget with
//! bBehindView, used by the Patriarch's grand entrance, his death and his
//! victory laugh. Only what is drawn changes: the render camera's
//! GlobalTransform is replaced after transform propagation, so gameplay
//! (aim, sounds, fog) keeps the player's own view, and the player still
//! controls their pawn (KF does not stop it). The first-person weapon is
//! not drawn in behind view. See DESIGN.md, G3b.

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::engine::camera::FlyCamera;
use crate::engine::coords::SCALE;
use crate::engine::runlog;

/// PlayerController.CameraDist (behind-view distance in collision radii).
const CAMERA_DIST: f32 = 9.0;

/// The zed the view is on (None: the player's own), and why.
#[derive(Resource, Default)]
pub struct ViewTarget {
    zed: Option<usize>,
    reason: &'static str,
    /// Behind view on the player's own pawn (the match is over:
    /// ClientSetBehindView(true)). We draw no player model, so only the
    /// first-person weapon goes.
    behind_self: bool,
}

impl ViewTarget {
    pub fn zed(&self) -> Option<usize> {
        self.zed
    }

    pub fn set(&mut self, zed: Option<usize>, reason: &'static str) {
        if self.zed != zed || self.reason != reason {
            runlog::kv("view_target", &format!("target={} reason={reason}", zed.map_or("player".into(), |z| format!("zed{z}"))));
        }
        self.zed = zed;
        self.reason = reason;
    }

    pub fn reason(&self) -> &'static str {
        self.reason
    }

    pub fn set_behind_self(&mut self, on: bool) {
        if self.behind_self != on {
            runlog::kv("view_target", &format!("behind_self={on}"));
        }
        self.behind_self = on;
    }
}

pub struct ViewTargetPlugin;

impl Plugin for ViewTargetPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ViewTarget>()
            .add_systems(PostUpdate, behind_view.after(bevy::transform::TransformSystems::Propagate));
    }
}

/// PlayerController.CalcBehindView on the view target: the player's own
/// view rotation (roll 0); the target's Location + 12 up, moved back along
/// the view by CameraDist x its CollisionRadius, or less if a 10-unit box
/// traced back hits the level.
fn behind_view(
    view: Res<ViewTarget>,
    zeds: Query<&crate::zeds::zed::Zed>,
    mut camera: Query<(&Transform, &mut GlobalTransform), With<FlyCamera>>,
    mut weapon_cams: Query<&mut Camera, With<crate::weapons::weapon::WeaponCamera>>,
    spatial: SpatialQuery,
) {
    let target = view.zed.and_then(|id| zeds.iter().find(|z| z.id == id));
    let first_person = target.is_none() && !view.behind_self;
    for mut c in &mut weapon_cams {
        if c.is_active != first_person {
            c.is_active = first_person;
        }
    }
    let (Some(z), Ok((t, mut global))) = (target, camera.single_mut()) else { return };
    let rotation = t.rotation;
    let back = -(rotation * Vec3::NEG_Z);
    let from = z.centre + Vec3::Y * 12.0 * SCALE;
    let dist = CAMERA_DIST * z.radius * SCALE;
    let filter = SpatialQueryFilter::from_mask([crate::world::collision::GameLayer::World, crate::world::collision::GameLayer::TraceBlocking]);
    let config = ShapeCastConfig { max_distance: dist, target_distance: 0.0, compute_contact_on_penetration: true, ignore_origin_penetration: true };
    let shape = Collider::cuboid(20.0 * SCALE, 20.0 * SCALE, 20.0 * SCALE);
    let view_dist = match Dir3::new(back) {
        Ok(d) => spatial.cast_shape(&shape, from, Quat::IDENTITY, d, &config, &filter).map_or(dist, |h| h.distance.min(dist)),
        Err(_) => dist,
    };
    *global = GlobalTransform::from(Transform::from_translation(from + back * view_dist).with_rotation(rotation));
}
