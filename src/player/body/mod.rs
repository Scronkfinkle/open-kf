//! The player's third-person body: the character record's mesh and skins,
//! KF's player animations (KFPawn / xPawn, with the animation names of the
//! weapon's third-person attachment), and the attachment model held in the
//! right hand. See DESIGN.md, "The player's third-person body".
//!
//! The body belongs to a pawn, not to the camera: `PawnState` (on the pawn
//! entity) holds what KF replicates to draw any pawn, and the body systems
//! read only that. For the local player, `feed_local_pawn` (here) and the
//! weapon code (`weapons::weapon::publish_pawn_weapon`) fill it in; another
//! player's pawn in a network game is filled from the network
//! (`net::pawns`), with a `PawnCharacter` naming its character.

use bevy::prelude::*;

use crate::engine::camera::FlyCamera;

mod animate;
mod fire_fx;
mod load;
mod preview;

pub use preview::{CharacterPreview, PREVIEW_MODEL_SELECT, PREVIEW_PROFILE, PreviewRequest, PreviewSystems};

/// What is needed to draw a pawn's body (what KF replicates for other
/// players: Location, Velocity, Physics, Rotation and ViewPitch, the
/// weapon attachment's FlashCount / FiringMode, AnimAction,
/// TakeHitLocation, Health). Unreal units are not used here: positions
/// are Bevy space (metres), angles radians (the `FlyCamera` convention:
/// yaw 0 faces -Z, i.e. Unreal +X; pitch up is positive).
#[derive(Component, Debug, Clone, Default)]
pub struct PawnState {
    /// There is a walking pawn to draw (false while flying: no pawn).
    pub active: bool,
    /// The local player's own pawn: hidden in their first-person view.
    pub local: bool,
    /// The collision cylinder's centre (the actor's Location).
    pub location: Vec3,
    /// Metres per second.
    pub velocity: Vec3,
    pub on_ground: bool,
    /// The view's yaw and pitch (Pawn Rotation.Yaw and ViewPitch).
    pub yaw: f32,
    pub pitch: f32,
    /// The weapon in hand (its class path, e.g. `KFMod.Single`); its
    /// AttachmentClass is drawn and gives the animation names.
    pub weapon_class: Option<String>,
    /// Shots so far (WeaponAttachment.FlashCount; counted up per shot).
    pub flash_count: u32,
    /// A fire mode is firing (FlashCount != 0 in KF), and which.
    pub firing: bool,
    pub firing_mode: u8,
    /// Reloads started so far (SetAnimAction(WeaponReloadAnim)).
    pub reloads: u32,
    /// Hits taken so far (PlayTakeHit) and where the last came from.
    pub hits: u32,
    pub hit_from: Option<Vec3>,
    pub dead: bool,
}

/// The character a pawn other than the local player's is drawn with (the
/// name of a character record, e.g. `Mr_Foster`; KF's PRI
/// CharacterName). The local player's pawn uses `CharacterChoice` instead.
/// Changing it gives the pawn a new body.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct PawnCharacter(pub String);

/// The body systems (other code that feeds `PawnState`s runs before).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct BodySystems;

/// KF's FireState (xPawn): what channel 1 (the upper body) is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum FireState {
    #[default]
    None,
    PlayOnce,
    Looping,
    Ready,
}

/// What the lower channels (the whole body) play.
#[derive(Clone, Copy, Debug, PartialEq)]
enum BaseKind {
    Idle,
    Move,
    /// Turning in place: true = left.
    Turn(bool),
    Takeoff,
    Air,
    Land,
    Hit,
    Death,
}

/// One playing sequence: index, frame, and whether it loops.
#[derive(Clone, Copy, Debug)]
struct Play {
    seq: usize,
    frame: f32,
    looping: bool,
    /// Play rate multiplier (1 = the sequence's own rate).
    rate: f32,
}

/// The animation state of one pawn's body and the entities that draw it.
#[derive(Component)]
pub struct PawnBody {
    /// Index into `BodyModels::characters`.
    character: usize,
    /// The drawn root (at the pawn's Location, turned by its yaw).
    root: Entity,
    meshes: Vec<Handle<Mesh>>,
    /// The attachment in hand: weapon class, index into
    /// `BodyModels::attachments`, its part entities and meshes.
    weapon_class: Option<String>,
    attachment: Option<usize>,
    attachment_parts: Vec<Entity>,
    attachment_meshes: Vec<Handle<Mesh>>,
    base_kind: BaseKind,
    /// The base channel's sequence(s). Movement plays up to four at one
    /// phase (0..1) with weights; others play one.
    base: Option<Play>,
    phase: f32,
    move_weights: [f32; 4],
    /// The pose the last base change tweens from (bone locals), and the
    /// tween's time left and length.
    tween_from: Vec<(Quat, Vec3)>,
    tween_left: f32,
    tween_time: f32,
    /// The last base pose (bone locals), for the next tween.
    last_base: Vec<(Quat, Vec3)>,
    /// Channel 1 (from FireRootBone): its sequence, alpha, and the alpha's
    /// blend (target, per second).
    upper: Option<Play>,
    upper_alpha: f32,
    upper_blend: Option<(f32, f32)>,
    /// KFPawn.AnimBlendTime: seconds until channel 1 blends out (weapon switch).
    upper_timer: Option<f32>,
    fire_state: FireState,
    /// Counters seen so far (shots, reloads, hits) and the last yaw.
    seen_flash: u32,
    seen_reloads: u32,
    seen_hits: u32,
    was_on_ground: bool,
    /// Seconds since leaving the ground.
    air_time: f32,
    /// Channel 1 is blending out (its AnimEnd was handled).
    upper_ending: bool,
    last_yaw: f32,
    /// Smoothed turn speed (Unreal units / s) and how long turning lasts.
    turn_rate: f32,
    turn_hold: f32,
    rng: u32,
    /// The last pose (mesh space bone transforms), where a ragdoll starts.
    last_pose: Vec<(Quat, Vec3)>,
    /// The bone locals of the last animated pose (before the aim pitch).
    last_locals: Vec<(Quat, Vec3)>,
    /// The death ragdoll, and the root's transform when it started (the
    /// ragdoll's bodies are in world space; the root stays put).
    ragdoll: Option<crate::zeds::ragdoll::RagdollState>,
    frozen_root: Option<Transform>,
    /// Where the ragdoll's root part is (Bevy), while there is one.
    ragdoll_location: Option<Vec3>,
    /// Last whole second logged (body_state).
    log_second: i64,
    /// Whose body, for the log: "local", else the pawn entity's Name.
    who: String,
    /// The attachment's `tip` bone this frame (Unreal world location and
    /// axes), where the third-person muzzle flash sits.
    tip: Option<(Vec3, Mat3)>,
}

impl PawnBody {
    /// Removes the body from its pawn and despawns what draws it (the pawn
    /// entity itself stays). `spawn_bodies` gives the pawn a new one.
    pub fn despawn(&self, commands: &mut Commands, pawn: Entity) {
        commands.entity(self.root).despawn();
        if let Some(r) = &self.ragdoll {
            for e in r.joints.iter().chain(&r.bodies) {
                commands.entity(*e).despawn();
            }
        }
        if let Ok(mut p) = commands.get_entity(pawn) {
            p.remove::<PawnBody>();
        }
    }

    /// The pawn's Location while it is a ragdoll (the root part's centre;
    /// in KF the actor moves with its ragdoll), for the behind view.
    pub fn ragdoll_location(&self) -> Option<Vec3> {
        self.ragdoll_location
    }
}

pub struct BodyPlugin;

impl Plugin for BodyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<load::BodyModels>()
            .insert_non_send(load::BodyPackages::default())
            .init_resource::<preview::CharacterPreview>()
            .insert_non_send(preview::PreviewState::default())
            .add_systems(Startup, preview::spawn_preview_cameras)
            .add_systems(PostStartup, load::load_body_models)
            .add_systems(
                PostUpdate,
                preview::update_previews.in_set(PreviewSystems).before(bevy::transform::TransformSystems::Propagate),
            )
            .add_systems(
                Update,
                (
                    add_local_pawn_state,
                    feed_local_pawn,
                    load::reload_on_character_change,
                    load::load_pawn_characters,
                    animate::follow_pawn_character,
                    load::load_attachments,
                    animate::spawn_bodies,
                    animate::animate_bodies,
                    fire_fx::remote_fire_effects.in_set(crate::weapons::muzzle_light::MuzzleLightTrigger),
                )
                    .chain()
                    .in_set(BodySystems)
                    .after(crate::player::walk::WalkSystems)
                    .after(crate::weapons::weapon::PublishPawnWeapon),
            );
    }
}

/// The local player's pawn gets a `PawnState` (today the pawn and the
/// camera are one entity: the walker is on the camera).
fn add_local_pawn_state(mut commands: Commands, cams: Query<Entity, (With<FlyCamera>, Without<PawnState>)>) {
    for e in &cams {
        commands.entity(e).insert(PawnState { local: true, ..default() });
    }
}

/// Copies the local player's walker, view angles, health and hits into
/// their pawn's `PawnState` (the weapon part comes from the weapon code).
fn feed_local_pawn(
    mut pawns: Query<(&mut PawnState, &FlyCamera, Option<&crate::player::walk::Walker>)>,
    health: Res<crate::game::combat::PlayerHealth>,
    mut hurts: MessageReader<crate::game::combat::PlayerHurt>,
) {
    let Ok((mut s, cam, walker)) = pawns.single_mut() else {
        hurts.clear();
        return;
    };
    s.active = walker.is_some();
    if let Some(w) = walker {
        s.location = w.center;
        s.velocity = w.velocity;
        s.on_ground = w.on_ground;
    }
    s.yaw = cam.yaw;
    s.pitch = cam.pitch;
    s.dead = health.dead;
    for h in hurts.read() {
        if h.damage > 0.0 {
            s.hits = s.hits.wrapping_add(1);
            s.hit_from = h.source;
        }
    }
}
