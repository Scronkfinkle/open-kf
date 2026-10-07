//! The view on another actor (G3b): PlayerController.SetViewTarget with
//! bBehindView, used by the Patriarch's grand entrance, his death and his
//! victory laugh; and behind view on the player's own pawn (the end of the
//! match: ClientSetBehindView; F4: KF's ToggleBehindView). Only what is
//! drawn changes: the render camera's GlobalTransform is replaced after
//! transform propagation, so gameplay (aim, sounds, fog) keeps the
//! player's own view, and the player still controls their pawn (KF does
//! not stop it). The first-person weapon is not drawn in behind view; the
//! player's body is (player/body). See DESIGN.md, G3b and "The player's
//! third-person body".

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::engine::camera::FlyCamera;
use crate::engine::coords::SCALE;
use crate::engine::runlog;

/// PlayerController.CameraDist (behind-view distance in collision radii).
const CAMERA_DIST: f32 = 9.0;
/// KFPawn CollisionRadius (the player's pawn).
const PLAYER_RADIUS: f32 = 20.0;

/// The zed the view is on (None: the player's own), and why.
#[derive(Resource, Default)]
pub struct ViewTarget {
    zed: Option<usize>,
    reason: &'static str,
    /// Behind view on the player's own pawn because the match is over
    /// (KFGameType.CheckEndGame: ClientSetBehindView(true)).
    behind_self: bool,
    /// Behind view asked for by the player (F4 = KF's ToggleBehindView,
    /// `--behind-view`, test action `behind_view`).
    behind_toggle: bool,
    /// PlayerController.CameraDeltaRotation.Yaw (KF's free camera "for
    /// checking out player models and animations"), radians, Unreal sense
    /// (positive turns right); set by the test flag `--behind-yaw DEG`.
    camera_delta_yaw: f32,
}

impl ViewTarget {
    /// Starts in behind view on the player's pawn if `on` (`--behind-view`),
    /// the camera turned by `delta_yaw_deg` around the pawn (`--behind-yaw`).
    pub fn starting_behind(on: bool, delta_yaw_deg: f32) -> Self {
        if on {
            runlog::kv("behind_view", &format!("on=true reason=command_line camera_delta_yaw_deg={delta_yaw_deg}"));
        }
        ViewTarget { behind_toggle: on, camera_delta_yaw: delta_yaw_deg.to_radians(), ..default() }
    }

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
            runlog::kv("behind_view", &format!("on={} reason=end_of_match", on || self.behind_toggle));
        }
        self.behind_self = on;
    }

    /// bBehindView on the player's own pawn (end of match or toggled).
    pub fn behind_self(&self) -> bool {
        self.behind_self || self.behind_toggle
    }

    /// The player sees through their own pawn's eyes: their body is not
    /// drawn (UE2 does not draw the view target's pawn in first person),
    /// the first-person weapon is.
    pub fn first_person(&self) -> bool {
        self.zed.is_none() && !self.behind_self()
    }
}

pub struct ViewTargetPlugin;

impl Plugin for ViewTargetPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ViewTarget>()
            .add_systems(Update, toggle_behind_view)
            .add_systems(PostUpdate, behind_view.after(bevy::transform::TransformSystems::Propagate));
    }
}

/// F4 (KF's User.ini: `F4=ToggleBehindView`; ServerToggleBehindView
/// allows it in standalone games) or the test action `behind_view`.
fn toggle_behind_view(
    keys: Res<ButtonInput<KeyCode>>,
    script: Res<crate::weapons::weapon::ScriptedInput>,
    frames: Res<bevy::diagnostic::FrameCount>,
    mut view: ResMut<ViewTarget>,
) {
    let scripted = script.0.iter().any(|(f, a)| *f == frames.0 && a == "behind_view");
    if keys.just_pressed(KeyCode::F4) || scripted {
        view.behind_toggle = !view.behind_toggle;
        runlog::kv(
            "behind_view",
            &format!("on={} reason={}", view.behind_self(), if scripted { "test_action" } else { "key_f4" }),
        );
    }
}

/// PlayerController.CalcBehindView on the view target: the player's own
/// view rotation (roll 0); the target's Location + 12 up, moved back along
/// the view by CameraDist x its CollisionRadius, or less if a 10-unit box
/// traced back hits the level. The target is a zed, or the player's own
/// pawn in behind view.
#[allow(clippy::type_complexity)] // Bevy system parameters
fn behind_view(
    time: Res<Time>,
    view: Res<ViewTarget>,
    zeds: Query<&crate::zeds::zed::Zed>,
    mut camera: Query<
        (&Transform, &mut GlobalTransform, Option<&crate::player::walk::Walker>, Option<&crate::player::body::PawnBody>),
        With<FlyCamera>,
    >,
    mut weapon_cams: Query<&mut Camera, With<crate::weapons::weapon::WeaponCamera>>,
    spatial: SpatialQuery,
    mut last_log: Local<f32>,
) {
    let target = view.zed.and_then(|id| zeds.iter().find(|z| z.id == id));
    let first_person = target.is_none() && !view.behind_self();
    for mut c in &mut weapon_cams {
        if c.is_active != first_person {
            c.is_active = first_person;
        }
    }
    let Ok((t, mut global, walker, body)) = camera.single_mut() else { return };
    let (centre, radius, who) = match target {
        Some(z) => (z.centre, z.radius, "zed"),
        None if view.behind_self() => {
            // The pawn's Location: the ragdoll's root part when dead, else
            // the walking cylinder's centre (the eye is BaseEyeHeight 44
            // above it while flying).
            let centre = body.and_then(|b| b.ragdoll_location()).unwrap_or_else(|| {
                walker.map_or(t.translation - Vec3::Y * crate::player::walk::kf::EYE_HEIGHT * SCALE, |w| w.center)
            });
            (centre, PLAYER_RADIUS, "player")
        }
        None => return,
    };
    // CameraRotation = Rotation + CameraDeltaRotation (the player's own
    // pawn only; the boss views use none).
    let delta = if who == "player" { view.camera_delta_yaw } else { 0.0 };
    let rotation = Quat::from_rotation_y(-delta) * t.rotation;
    let back = -(rotation * Vec3::NEG_Z);
    let from = centre + Vec3::Y * 12.0 * SCALE;
    let dist = CAMERA_DIST * radius * SCALE;
    let filter = SpatialQueryFilter::from_mask([crate::world::collision::GameLayer::World, crate::world::collision::GameLayer::TraceBlocking]);
    let config = ShapeCastConfig { max_distance: dist, target_distance: 0.0, compute_contact_on_penetration: true, ignore_origin_penetration: true };
    let shape = Collider::cuboid(20.0 * SCALE, 20.0 * SCALE, 20.0 * SCALE);
    let view_dist = match Dir3::new(back) {
        Ok(d) => spatial.cast_shape(&shape, from, Quat::IDENTITY, d, &config, &filter).map_or(dist, |h| h.distance.min(dist)),
        Err(_) => dist,
    };
    *global = GlobalTransform::from(Transform::from_translation(from + back * view_dist).with_rotation(rotation));
    let now = time.elapsed_secs();
    if who == "player" && now - *last_log >= 1.0 {
        *last_log = now;
        runlog::kv(
            "behind_view_camera",
            &format!("target=player distance_unreal={:.0} full_unreal={:.0}", view_dist / SCALE, dist / SCALE),
        );
    }
}
