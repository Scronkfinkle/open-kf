//! E2: KF's view shake and the hit-blur timer (DESIGN.md, "Hit effects").
//! Engine.PlayerController's ShakeView / ViewShake / CheckShake and the
//! ambient shake of CalcFirstPersonView, KFPlayerController.DamageShake,
//! KFHumanPawn's PlayTakeHit / DoHitCamEffects / AddBlur / Tick /
//! StopHitCamEffects, ZombieSiren.DoShakeEffect. Ported as written,
//! frame-rate dependence included.
//!
//! The shake changes only what is drawn: the main and sky cameras'
//! GlobalTransform after transform propagation (as view_target.rs does),
//! put back at the start of the next frame, so aim and sounds keep the
//! unshaken view (KF adds ShakeRot to CameraRotation only). The
//! first-person weapon camera is not shaken (a guess, see DESIGN.md).
//! The blur amount is worked out here; drawing it is E3.

use avian3d::prelude::*;
use bevy::prelude::*;
use ue_assets::class_defaults::ClassDefaults;
use ue_assets::package_set::ObjectHandle;
use ue_assets::properties::{Rotator, Value};

use crate::engine::camera::{FlyCamera, SkyCamera};
use crate::engine::coords::{self, SCALE};
use crate::engine::runlog;
use crate::game::combat::PlayerHurt;

/// KFHumanPawn defaults: JarrMoveMag, JarrMoveRate, JarrMoveDuration,
/// JarrRotateMag, JarrRotateRate, JarrRotateDuration,
/// NewSchoolHitBlurIntensity (the PostFX path).
const JARR_MOVE_MAG: f32 = 50.0;
const JARR_MOVE_RATE: f32 = 200.0;
const JARR_MOVE_DURATION: f32 = 3.0;
const JARR_ROTATE_MAG: f32 = 1000.0;
const JARR_ROTATE_RATE: f32 = 10000.0;
const JARR_ROTATE_DURATION: f32 = 4.0;
const HIT_BLUR_INTENSITY: f32 = 0.8;

/// ZombieSirenBase's DoShakeEffect settings (class defaults).
#[derive(Clone, Copy, Debug)]
pub struct ScreamShake {
    /// RotMag (pitch, yaw, roll), RotRate.
    rot_mag: Vec3,
    rot_rate: f32,
    /// OffsetMag (Unreal X, Y, Z), OffsetRate.
    offset_mag: Vec3,
    offset_rate: f32,
    shake_time: f32,
    shake_fade_time: f32,
    effect_scalar: f32,
    min_effect_scale: f32,
    blur_scale: f32,
}

impl ScreamShake {
    pub fn load(defaults: &ClassDefaults, class: &ObjectHandle) -> Self {
        let get = |p: &str| defaults.get(class, p).map(|(v, _)| v);
        let f = |p: &str, d: f32| match get(p) {
            Some(Value::Float(f)) => f,
            Some(Value::Int(i)) => i as f32,
            _ => d,
        };
        let rot_mag = match get("RotMag") {
            Some(Value::Rotator(r)) => Vec3::new(r.pitch as f32, r.yaw as f32, r.roll as f32),
            _ => Vec3::splat(150.0),
        };
        let offset_mag = match get("OffsetMag") {
            Some(Value::Vector(v)) => Vec3::from_array(v),
            _ => Vec3::new(0.0, 5.0, 1.0),
        };
        ScreamShake {
            rot_mag,
            rot_rate: f("RotRate", 500.0),
            offset_mag,
            offset_rate: f("OffsetRate", 500.0),
            shake_time: f("ShakeTime", 2.0),
            shake_fade_time: f("ShakeFadeTime", 0.25),
            effect_scalar: f("ShakeEffectScalar", 1.0),
            min_effect_scale: f("MinShakeEffectScale", 0.6),
            blur_scale: f("ScreamBlurScale", 0.85),
        }
    }
}

/// One scream pulse (ZombieSiren.SpawnTwoShots -> DoShakeEffect).
#[derive(Message, Clone, Copy, Debug)]
pub struct SirenScreamShake {
    /// The Siren's centre, Bevy world.
    pub at: Vec3,
    /// ScreamRadius.
    pub radius: f32,
    pub shake: ScreamShake,
}

/// Engine.PlayerController's shake: X = pitch, Y = yaw, Z = roll for the
/// rotation (Unreal rotation units), Unreal view axes for the offset.
#[derive(Default, Debug)]
struct Shake {
    rot_max: Vec3,
    rot_rate: Vec3,
    rot_time: Vec3,
    rot: [i32; 3],
    offset_max: Vec3,
    offset_rate: Vec3,
    offset_time: Vec3,
    offset: Vec3,
}

impl Shake {
    /// ShakeView: a new shake replaces the running one only if bigger.
    fn shake_view(&mut self, rot_mag: Vec3, rot_rate: Vec3, rot_time: f32, offset_mag: Vec3, offset_rate: Vec3, offset_time: f32) -> bool {
        let mut taken = false;
        if rot_mag.length() > self.rot_max.length() {
            self.rot_max = rot_mag;
            self.rot_rate = rot_rate;
            self.rot_time = Vec3::splat(rot_time);
            taken = true;
        }
        if offset_mag.length() > self.offset_max.length() {
            self.offset_max = offset_mag;
            self.offset_rate = offset_rate;
            self.offset_time = Vec3::splat(offset_time);
            taken = true;
        }
        taken
    }

    /// ViewShake.
    fn view_shake(&mut self, dt: f32) {
        if self.offset_rate != Vec3::ZERO {
            for i in 0..3 {
                self.offset[i] += dt * self.offset_rate[i];
                check_shake(&mut self.offset_max[i], &mut self.offset[i], &mut self.offset_rate[i], &mut self.offset_time[i], dt);
            }
        }
        if self.rot_rate != Vec3::ZERO {
            for i in 0..3 {
                update_shake_rot_component(&mut self.rot_max[i], &mut self.rot[i], &mut self.rot_rate[i], &mut self.rot_time[i], dt);
            }
        }
    }

    /// StopViewShaking: the maxima, rates and times go to 0 (the current
    /// offset and rotation stay, as in KF; with rate 0 they no longer move).
    fn stop(&mut self) {
        self.rot_max = Vec3::ZERO;
        self.rot_rate = Vec3::ZERO;
        self.rot_time = Vec3::ZERO;
        self.offset_max = Vec3::ZERO;
        self.offset_rate = Vec3::ZERO;
        self.offset_time = Vec3::ZERO;
    }

    fn active(&self) -> bool {
        self.rot_rate != Vec3::ZERO || self.offset_rate != Vec3::ZERO
    }
}

/// PlayerController.CheckShake.
fn check_shake(max: &mut f32, offset: &mut f32, rate: &mut f32, time: &mut f32, dt: f32) {
    if offset.abs() < max.abs() {
        return;
    }
    *offset = *max;
    if *time > 1.0 {
        if *time * (*max / *rate).abs() <= 1.0 {
            *max *= 1.0 / *time - 1.0;
        } else {
            *max = -*max;
        }
        *time -= dt;
        *rate = -*rate;
    } else {
        *max = 0.0;
        *offset = 0.0;
        *rate = 0.0;
    }
}

/// PlayerController.UpdateShakeRotComponent: the angle wraps at 65536
/// (UnrealScript turns the float sum into an int, cutting the fraction).
fn update_shake_rot_component(max: &mut f32, current: &mut i32, rate: &mut f32, time: &mut f32, dt: f32) {
    *current = (((*current & 65535) as f32 + *rate * dt) as i32) & 65535;
    if *current > 32768 {
        *current -= 65536;
    }
    let mut f = *current as f32;
    check_shake(max, &mut f, rate, time, dt);
    *current = f as i32;
}

/// PlayerController's ambient shake (SetAmbientShake).
#[derive(Default, Debug)]
struct Ambient {
    enabled: bool,
    falloff_start: f32,
    falloff_time: f32,
    offset_mag: Vec3,
    offset_freq: f32,
    rot_mag: Vec3,
    rot_freq: f32,
}

impl Ambient {
    /// CalcFirstPersonView's AmbShakeOffset (Unreal world) and AmbShakeRot
    /// (pitch, yaw, roll) at game time `now`; switches itself off after the
    /// falloff.
    fn sample(&mut self, now: f32) -> (Vec3, Vec3) {
        if !self.enabled {
            return (Vec3::ZERO, Vec3::ZERO);
        }
        if self.falloff_start > 0.0 && now - self.falloff_start > self.falloff_time {
            self.enabled = false;
            return (Vec3::ZERO, Vec3::ZERO);
        }
        let scaling = if self.falloff_start > 0.0 { (1.0 - (now - self.falloff_start) / self.falloff_time).clamp(0.0, 1.0) } else { 1.0 };
        let tau = std::f32::consts::TAU;
        let offset = self.offset_mag * scaling * (now * self.offset_freq * tau).sin();
        // Rotator x float: each component cut to an int.
        let rot = (self.rot_mag * scaling * (now * self.rot_freq * tau).sin()).trunc();
        (offset, rot)
    }
}

/// KFHumanPawn's blur fade (BlurFadeOutTime, StartingBlurFadeOutTime,
/// CurrentBlurIntensity) and the amount handed to SetBlur.
#[derive(Default, Debug)]
pub struct HitBlur {
    fade_out: f32,
    starting: f32,
    intensity: f32,
    /// KFPlayerController.SetBlur's amount (0: off). Drawn by E3.
    pub amount: f32,
}

impl HitBlur {
    /// AddBlur: restarts the fade, keeps the higher intensity.
    fn add(&mut self, duration: f32, intensity: f32) {
        self.starting = duration;
        self.fade_out = duration;
        if self.intensity < intensity {
            self.intensity = intensity;
        }
        self.amount = self.intensity;
    }
}

/// Everything the hit camera keeps between frames.
#[derive(Resource, Default)]
pub struct HitCam {
    shake: Shake,
    ambient: Ambient,
    pub blur: HitBlur,
    /// Pawn.LastPainTime (PlayHit plays PlayTakeHit at most every 0.1 s).
    last_pain_time: Option<f32>,
    /// FRand() for the bile jar (fixed seed, like the rest of the project).
    rng: u32,
    deaths: u32,
    /// What was added to the camera this frame (for the log): offset in
    /// Unreal units, rotation in rotation units.
    drawn_offset: Vec3,
    drawn_rot: Vec3,
}

impl HitCam {
    fn frand(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1_103_515_245).wrapping_add(12345);
        ((self.rng >> 8) & 0xFFFF) as f32 / 65535.0
    }

    /// KFHumanPawn.DoHitCamEffects(HitDirection, JarrScale, BlurDuration,
    /// JarDurationScale), `dir` in the view's Unreal axes.
    fn do_hit_cam_effects(&mut self, dir: Vec3, jar: f32, blur_duration: f32, jar_duration_scale: f32, why: &str) {
        self.blur.add(blur_duration, HIT_BLUR_INTENSITY);
        let move_mag = dir * JARR_MOVE_MAG;
        let sign = |x: f32| if x < 0.0 { -1.0 } else { 1.0 };
        let move_rate = Vec3::new(sign(dir.x), sign(dir.y), sign(dir.z)) * JARR_MOVE_RATE;
        let rot_mag = Vec3::new(JARR_ROTATE_MAG * -dir.x, 0.0, JARR_ROTATE_MAG * dir.y);
        let rot_rate = Vec3::new(JARR_ROTATE_RATE * -dir.x, 0.0, JARR_ROTATE_RATE * dir.y);
        let taken = self.shake.shake_view(
            rot_mag * jar,
            rot_rate * (2.0 - jar),
            JARR_ROTATE_DURATION * jar_duration_scale,
            move_mag * jar,
            move_rate * (2.0 - jar),
            JARR_MOVE_DURATION * jar_duration_scale,
        );
        runlog::kv(
            "view_shake",
            &format!(
                "source={why} dir=({:.2},{:.2},{:.2}) jar={jar:.2} rot_mag=({:.0},0,{:.0}) offset_mag=({:.1},{:.1},{:.1}) taken={taken} blur={blur_duration}s",
                dir.x,
                dir.y,
                dir.z,
                rot_mag.x * jar,
                rot_mag.z * jar,
                move_mag.x * jar,
                move_mag.y * jar,
                move_mag.z * jar
            ),
        );
    }

    /// StopHitCamEffects.
    fn stop(&mut self, why: &str) {
        self.blur.intensity = 0.0;
        self.blur.amount = 0.0;
        self.blur.fade_out = 0.0;
        self.shake.stop();
        runlog::kv("hit_blur", &format!("end=true reason={why}"));
    }
}

pub struct HitCamPlugin;

impl Plugin for HitCamPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HitCam>()
            .add_message::<SirenScreamShake>()
            .add_systems(First, unshake_cameras)
            .add_systems(Update, (hit_cam_events, tick_hit_cam).chain().after(crate::game::combat::PlayerDamageSet))
            .add_systems(PostUpdate, shake_cameras.after(bevy::transform::TransformSystems::Propagate));
    }
}

/// The view's Unreal rotation matrix (columns: forward, right, up in Unreal
/// world axes) from a Bevy camera rotation.
fn ue_view_matrix(rotation: Quat) -> Mat3 {
    let c = coords::c();
    c.transpose() * Mat3::from_quat(rotation) * c
}

/// Hits (PlayHit -> PlayTakeHit, NotifyTakeHit -> DamageShake,
/// TakeBileDamage's own jar), scream pulses, and death.
#[allow(clippy::too_many_arguments)]
fn hit_cam_events(
    time: Res<Time>,
    mut cam: ResMut<HitCam>,
    mut hurts: MessageReader<PlayerHurt>,
    mut screams: MessageReader<SirenScreamShake>,
    health: Res<crate::game::combat::PlayerHealth>,
    player: Query<(&Transform, Option<&crate::player::walk::Walker>), With<FlyCamera>>,
    view_target: Res<crate::engine::view_target::ViewTarget>,
    spatial: SpatialQuery,
) {
    let now = time.elapsed_secs();
    if health.deaths != cam.deaths {
        cam.deaths = health.deaths;
        // KFHumanPawn.Died -> StopHitCamEffects.
        cam.stop("player_died");
        cam.ambient.enabled = false;
    }
    let Ok((t, walker)) = player.single() else {
        hurts.clear();
        screams.clear();
        return;
    };
    // The pawn's Location (the cylinder's centre).
    let centre = walker.map_or(t.translation - Vec3::Y * 44.0 * SCALE, |w| w.center);
    // The pawn's Rotation: the view's yaw only (Pawn.FaceRotation zeroes
    // the pitch while walking; assumed).
    let view = ue_view_matrix(t.rotation);
    let fwd = view.col(0).with_z(0.0).normalize_or(Vec3::X);
    let right = Vec3::new(-fwd.y, fwd.x, 0.0);
    for h in hurts.read() {
        // KFHumanPawn.PlayTakeHit (via Pawn.PlayHit, at most every 0.1 s):
        // direction = Location - HitLocation, flat, in the pawn's axes.
        if cam.last_pain_time.is_none_or(|t| now - t > 0.1) {
            cam.last_pain_time = Some(now);
            let away = h.source.map_or(Vec3::ZERO, |s| {
                let d = centre - s;
                Vec3::new(-d.z, d.x, d.y).with_z(0.0).normalize_or_zero()
            });
            let dir = Vec3::new(away.dot(fwd), away.dot(right), 0.0);
            let jar = (0.1 + h.damage / 10.0).min(1.0);
            cam.do_hit_cam_effects(dir, jar, 2.0, 1.0, &format!("hit_{:?}", h.dam_type));
        }
        // KFPlayerController.NotifyTakeHit -> DamageShake(Damage).
        let d = h.damage.clamp(0.0, 250.0);
        let taken = cam.shake.shake_view(
            Vec3::new(30.0 * d, 0.0, 0.0),
            Vec3::new(120000.0, 0.0, 0.0),
            0.15 + 0.005 * d,
            Vec3::new(0.0, 0.0, 0.03 * d),
            Vec3::ONE,
            0.2,
        );
        runlog::kv("view_shake", &format!("source=damage_shake damage={d} rot_mag=({:.0},0,0) offset_mag=(0,0,{:.2}) taken={taken}", 30.0 * d, 0.03 * d));
        // TakeBileDamage: DoHitCamEffects(BileVect, 0.35, 2.0, 1.0), BileVect
        // three FRand()s.
        if h.bile_tick {
            let v = Vec3::new(cam.frand(), cam.frand(), cam.frand());
            cam.do_hit_cam_effects(v, 0.35, 2.0, 1.0, "bile");
        }
    }
    for s in screams.read() {
        // ZombieSiren.DoShakeEffect: on the view target, if it is within
        // ScreamRadius; reduced behind a wall (FastTrace: level only).
        if view_target.zed().is_some() {
            continue;
        }
        let dist = (centre - s.at).length() / SCALE;
        if dist >= s.radius {
            continue;
        }
        let blocked = Dir3::new(s.at - centre)
            .ok()
            .and_then(|d| spatial.cast_ray(centre, d, (s.at - centre).length(), true, &crate::world::collision::world_filter()))
            .is_some();
        let (scale, blur_scale) = scream_scales(dist, s.radius, blocked, &s.shake);
        cam.ambient = Ambient {
            enabled: true,
            falloff_start: now + s.shake.shake_fade_time,
            falloff_time: s.shake.shake_time,
            offset_mag: s.shake.offset_mag * scale,
            offset_freq: s.shake.offset_rate,
            rot_mag: s.shake.rot_mag * scale,
            rot_freq: s.shake.rot_rate,
        };
        cam.blur.add(s.shake.shake_time, blur_scale * s.shake.blur_scale);
        runlog::kv(
            "view_shake",
            &format!("source=siren_scream distance_unreal={dist:.0} blocked={blocked} scale={scale:.2} blur={:.2} for={}s", blur_scale * s.shake.blur_scale, s.shake.shake_time),
        );
    }
}

/// DoShakeEffect's shake scale and BlurScale at `dist` (inside `radius`):
/// (radius - dist) / radius x ShakeEffectScalar; behind a wall both x 0.25,
/// else the shake is Lerp(scale, MinShakeEffectScale, 1) and the blur keeps
/// the plain scale.
fn scream_scales(dist: f32, radius: f32, blocked: bool, shake: &ScreamShake) -> (f32, f32) {
    let scale = (radius - dist) / radius * shake.effect_scalar;
    if blocked {
        (scale * 0.25, scale * 0.25)
    } else {
        (shake.min_effect_scale + scale * (1.0 - shake.min_effect_scale), scale)
    }
}

/// PlayerController.ViewShake (PlayerTick) and KFHumanPawn.Tick's blur fade.
fn tick_hit_cam(time: Res<Time>, mut cam: ResMut<HitCam>, frames: Res<bevy::diagnostic::FrameCount>) {
    let dt = time.delta_secs();
    cam.shake.view_shake(dt);
    if cam.blur.fade_out > 0.0 {
        cam.blur.fade_out -= dt;
        if cam.blur.fade_out <= 0.0 {
            cam.stop("faded");
        } else {
            cam.blur.amount = cam.blur.fade_out / cam.blur.starting * cam.blur.intensity;
        }
    }
    if frames.0.is_multiple_of(10) && (cam.shake.active() || cam.ambient.enabled || cam.blur.amount > 0.0) {
        runlog::kv(
            "hit_cam",
            &format!(
                "shake_offset=({:.2},{:.2},{:.2}) shake_rot=({},{},{}) drawn_offset=({:.2},{:.2},{:.2}) drawn_rot=({:.0},{:.0},{:.0}) ambient={} blur={:.3}",
                cam.shake.offset.x,
                cam.shake.offset.y,
                cam.shake.offset.z,
                cam.shake.rot[0],
                cam.shake.rot[1],
                cam.shake.rot[2],
                cam.drawn_offset.x,
                cam.drawn_offset.y,
                cam.drawn_offset.z,
                cam.drawn_rot.x,
                cam.drawn_rot.y,
                cam.drawn_rot.z,
                cam.ambient.enabled,
                cam.blur.amount
            ),
        );
    }
}

/// Puts the cameras back to their unshaken pose before gameplay reads them.
#[allow(clippy::type_complexity)]
fn unshake_cameras(mut cams: Query<(&Transform, &mut GlobalTransform), Or<(With<FlyCamera>, With<SkyCamera>)>>) {
    for (t, mut g) in &mut cams {
        *g = GlobalTransform::from(*t);
    }
}

/// CalcFirstPersonView: CameraRotation = Rotation + ShakeRot + AmbShakeRot;
/// CameraLocation += ShakeOffset along the view's axes + AmbShakeOffset.
#[allow(clippy::type_complexity)]
fn shake_cameras(
    time: Res<Time>,
    mut cam: ResMut<HitCam>,
    view: Res<crate::engine::view_target::ViewTarget>,
    mut main: Query<(&Transform, &mut GlobalTransform), (With<FlyCamera>, Without<SkyCamera>)>,
    mut sky: Query<(&Transform, &mut GlobalTransform), (With<SkyCamera>, Without<FlyCamera>)>,
    sky_info: Res<crate::world::map::SkyInfo>,
) {
    let (amb_offset, amb_rot) = cam.ambient.sample(time.elapsed_secs());
    let rot = Vec3::new(cam.shake.rot[0] as f32, cam.shake.rot[1] as f32, cam.shake.rot[2] as f32) + amb_rot;
    if view.zed().is_some() {
        return;
    }
    let Ok((t, mut global)) = main.single_mut() else { return };
    let m = ue_view_matrix(t.rotation);
    let offset_ue = m * cam.shake.offset + amb_offset;
    cam.drawn_offset = offset_ue;
    cam.drawn_rot = rot;
    if offset_ue == Vec3::ZERO && rot == Vec3::ZERO {
        return;
    }
    let r = coords::ue_rotator_of(m) + rot;
    let shaken = ue_rotation_matrix_f(r);
    let c = coords::c();
    let rotation = Quat::from_mat3(&(c * shaken * c.transpose())).normalize();
    *global = GlobalTransform::from(Transform::from_translation(t.translation + coords::pos(offset_ue.to_array())).with_rotation(rotation));
    for (st, mut sg) in &mut sky {
        *sg = GlobalTransform::from(Transform::from_translation(st.translation).with_rotation(sky_info.rotation * rotation));
    }
}

/// `coords::ue_rotation_matrix` for a rotator in (fractional) units.
fn ue_rotation_matrix_f(r: Vec3) -> Mat3 {
    coords::ue_rotation_matrix(Rotator { pitch: r.x.round() as i32, yaw: r.y.round() as i32, roll: r.z.round() as i32 })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DamageShake(10) at 60 fps: the pitch kick (300 units at 120000/s)
    /// reaches its max inside the first frame and ends at once (Time 0.2).
    #[test]
    fn damage_shake_is_one_frame() {
        let mut s = Shake::default();
        s.shake_view(Vec3::new(300.0, 0.0, 0.0), Vec3::new(120000.0, 0.0, 0.0), 0.2, Vec3::new(0.0, 0.0, 0.3), Vec3::ONE, 0.2);
        s.view_shake(1.0 / 60.0);
        assert_eq!(s.rot[0], 0);
        assert_eq!(s.rot_rate.x, 0.0);
    }

    /// A jar (Time 4): each bounce turns back at 3/4 of the last height.
    #[test]
    fn jar_bounces_shrink() {
        let (mut max, mut off, mut rate, mut time) = (100.0f32, 100.0f32, 1000.0f32, 4.0f32);
        check_shake(&mut max, &mut off, &mut rate, &mut time, 0.01);
        assert_eq!(off, 100.0);
        assert!((max + 75.0).abs() < 1e-3 && rate == -1000.0 && (time - 3.99).abs() < 1e-5);
    }

    /// A Siren 350 of 700 away: in sight, shake 0.6 + 0.5 x 0.4 = 0.8 and
    /// blur scale 0.5; behind a wall both 0.125.
    #[test]
    fn scream_scale_wall() {
        let shake = ScreamShake {
            rot_mag: Vec3::splat(150.0),
            rot_rate: 500.0,
            offset_mag: Vec3::new(0.0, 5.0, 1.0),
            offset_rate: 500.0,
            shake_time: 2.0,
            shake_fade_time: 0.25,
            effect_scalar: 1.0,
            min_effect_scale: 0.6,
            blur_scale: 0.85,
        };
        let (s, b) = scream_scales(350.0, 700.0, false, &shake);
        assert!((s - 0.8).abs() < 1e-6 && (b - 0.5).abs() < 1e-6);
        let (s, b) = scream_scales(350.0, 700.0, true, &shake);
        assert!((s - 0.125).abs() < 1e-6 && (b - 0.125).abs() < 1e-6);
    }

    /// The camera rotation survives the Bevy -> Unreal -> Bevy round trip.
    #[test]
    fn view_matrix_round_trip() {
        let q = Quat::from_euler(EulerRot::YXZ, 0.7, -0.3, 0.0);
        let m = ue_view_matrix(q);
        let back = ue_rotation_matrix_f(coords::ue_rotator_of(m));
        let c = coords::c();
        let q2 = Quat::from_mat3(&(c * back * c.transpose()));
        assert!(q.angle_between(q2) < 1e-3);
    }
}
