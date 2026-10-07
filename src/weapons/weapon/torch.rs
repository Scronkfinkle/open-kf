//! The flashlight on the weapon in hand, the local player's side
//! (DESIGN.md, "Weapon flashlights"): the F key and the light alt fires,
//! KFWeapon.ServerSpawnLight's toggle, the light going off by itself
//! (KFWeapon.WeaponTick), the beam start from the first-person weapon's
//! LightBone, and the TacShine mesh. The state itself is the holder's
//! `Flashlight` component (weapons/flashlight.rs).

use super::*;
use crate::weapons::flashlight::{Beam, Flashlight};

/// KFWeapon.ServerSpawnLight's sound when the light is first made:
/// PlaySound(sound'KF_9MMSnd.NineMM_AltFire1', SLOT_Misc, 100).
const SPAWN_SOUND: &str = "KF_9MMSnd.NineMM_AltFire1";
/// TacLightShineAttachment: Mesh and DrawScale.
const SHINE_MESH: &str = "KFWeaponModels.TacShine";
const SHINE_DRAW_SCALE: f32 = 0.15;
/// Effect_TacLightProjector.Tick: StartTrace = Location + 0.2 x (LightBone - Location) + offset.
const BONE_PULL: f32 = 0.2;

/// A torch weapon (bTorchEnabled).
#[derive(Clone, Debug)]
pub(super) struct TorchDef {
    /// FirstPersonFlashlightOffset, Unreal units along the LightBone's axes.
    offset: Vec3,
    /// The first-person mesh's 'LightBone'.
    light_bone: Option<usize>,
    /// ModeSwitchAnim (lowercase), played by ServerSpawnLight; None if unset.
    switch_anim: Option<String>,
    /// KFPawn.ToggleFlashlight's search order when another weapon is in
    /// hand: 0 Shotgun, 1 BenelliShotgun, 2 Dualies, 3 Single.
    pick: Option<u8>,
}

impl TorchDef {
    /// `is_a` answers the class's IsA for a class name.
    pub(super) fn new(model: &SkinnedModel, offset: [f32; 3], switch_anim: Option<String>, is_a: impl Fn(&str) -> bool) -> Self {
        // FindInventoryType(class'Dualies') also finds the other dual
        // pistols, which ToggleFlashlight then refuses.
        let other_duals = ["DualDeagle", "Dual44Magnum", "DualMK23Pistol", "DualFlareRevolver"].iter().any(|c| is_a(c));
        let pick = if is_a("Shotgun") {
            Some(0)
        } else if is_a("BenelliShotgun") {
            Some(1)
        } else if is_a("Dualies") && !other_duals {
            Some(2)
        } else if is_a("Single") {
            Some(3)
        } else {
            None
        };
        TorchDef {
            offset: Vec3::from_array(offset),
            light_bone: model.find_bone("LightBone"),
            switch_anim: switch_anim.map(|a| a.to_ascii_lowercase()).filter(|a| a.chars().any(char::is_alphanumeric) && a != "none"),
            pick,
        }
    }

    pub(super) fn log(&self, class: &str) {
        runlog::kv(
            "weapon_torch",
            &format!("class={class} offset=({:.0},{:.0},{:.0}) light_bone={:?} switch_anim={:?} pick_order={:?}", self.offset.x, self.offset.y, self.offset.z, self.light_bone, self.switch_anim, self.pick),
        );
    }
}

/// Why the light is asked to toggle (bPendingFlashlight is
/// `TorchState::pending`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum TorchRequest {
    /// The light alt fire's ModeDoFire (middle mouse, or F with a torch
    /// weapon in hand): ToggleTorch -> LightFire.
    AltFire,
}

/// The LightBone of the weapon in hand, as last posed.
#[derive(Clone, Copy, Debug)]
pub(super) struct LightFrame {
    /// Unreal world: the bone, its X, Y, Z axes, and the weapon's location.
    bone: Vec3,
    axes: Mat3,
    weapon: Vec3,
    /// Weapon-camera space (Bevy): the bone and its axes, for the shine.
    local: Vec3,
    local_axes: Mat3,
}

#[derive(Default)]
pub(super) struct TorchState {
    requests: Vec<TorchRequest>,
    /// bPendingFlashlight: the weapon to light once it is up.
    pending: Option<usize>,
    pub(super) frame: Option<LightFrame>,
    /// The first-person TacShine mesh parts (under the weapon camera).
    shine: Vec<Entity>,
    shine_tried: bool,
}

/// What F does (KFPawn.ToggleFlashlight).
pub(super) enum FlashKey {
    /// The weapon in hand has a torch: Weapon.ClientStartFire(1).
    AltFire,
    /// Switch to this torch weapon and light it when it is up.
    Switch(usize),
    Nothing,
}

pub(super) fn flashlight_key(w: &mut Weapons) -> FlashKey {
    if w.defs[w.current].torch.is_some() {
        runlog::kv("flashlight_key", &format!("weapon={} action=alt_fire", w.defs[w.current].item_name));
        return FlashKey::AltFire;
    }
    // Shotgun, else BenelliShotgun, else Dualies (not the other duals),
    // else Single; the first of a kind in the inventory list.
    let found = (0..4u8).find_map(|rank| w.defs.iter().position(|d| !d.gone && d.torch.as_ref().is_some_and(|t| t.pick == Some(rank))));
    match found {
        Some(i) => {
            w.torch.pending = Some(i);
            runlog::kv("flashlight_pending", &format!("from={} to={}", w.defs[w.current].item_name, w.defs[i].item_name));
            FlashKey::Switch(i)
        }
        None => {
            runlog::kv("flashlight_refused", &format!("weapon={} reason=no_torch_weapon", w.defs[w.current].item_name));
            FlashKey::Nothing
        }
    }
}

/// SingleALTFire / ShotgunLightFire / NailGunALTFire while the alt fire is
/// held: ModeDoFire every FireRate when ready (Weapon.ReadyToFire: idle,
/// the other mode not firing) and allowed (not reloading or throwing):
/// ToggleTorch, then KFFire's PlayFiring (FireAnim 'LightOn', FireSound).
pub(super) fn torch_alt_fire(w: &mut Weapons) {
    let cur = w.current;
    let rate = w.defs[cur].modes[1].rate;
    let exclusive = w.defs[cur].modes[1].mode_exclusive || w.defs[cur].modes[0].mode_exclusive;
    if w.action != Action::Idle || (exclusive && (w.firing[0] || w.fire_cooldown[0] > 0.0)) || w.fire_cooldown[1] > 0.0 {
        return;
    }
    if !w.firing[1] {
        w.firing[1] = true;
        w.shots_this_press[1] = 0;
        w.fire_cooldown[1] = 0.0;
    }
    w.fire_cooldown[1] = (w.fire_cooldown[1] + rate).max(0.0);
    w.torch.requests.push(TorchRequest::AltFire);
    play_firing(w, 1, false);
    w.shots_this_press[1] += 1;
}

/// The LightBone frame of the weapon in hand (called by animate_weapon,
/// which has the pose).
pub(super) fn light_frame(def: &WeaponDef, bones: &[(Quat, Vec3)], part_translation: Vec3, main: &Transform) -> Option<LightFrame> {
    let bone = def.torch.as_ref()?.light_bone?;
    let (o, axes) = def.model.bone_frame(bones, bone)?;
    let scale = Vec3::from_array(def.model.mesh.scale);
    let origin = Vec3::from_array(def.model.mesh.origin);
    let local = coords::pos(((o - origin) * scale).to_array()) + part_translation;
    let local_axes = axes.map(|a| coords::dir((a * scale).to_array()).normalize_or_zero());
    let world = local_axes.map(|a| to_ue(main.rotation * a).normalize_or_zero());
    Some(LightFrame {
        bone: to_ue(main.transform_point(local)) / coords::SCALE,
        axes: Mat3::from_cols(world[0], world[1], world[2]),
        weapon: to_ue(main.transform_point(part_translation)) / coords::SCALE,
        local,
        local_axes: Mat3::from_cols(local_axes[0], local_axes[1], local_axes[2]),
    })
}

/// KFWeapon.ServerSpawnLight. `sound`: play its own spawn sound (not when
/// the alt fire's FireSound, the same sound, just played).
fn server_spawn_light(w: &mut Weapons, f: &mut Flashlight, alive: bool, sound: bool, reason: &str) {
    let cur = w.current;
    let (class, name) = (w.defs[cur].class.clone(), w.defs[cur].item_name);
    if w.firing[0] || w.action == Action::Reload {
        runlog::kv("flashlight_refused", &format!("weapon={name} reason={} request={reason}", if w.firing[0] { "firing" } else { "reloading" }));
        return;
    }
    let switch_anim = w.defs[cur].torch.as_ref().and_then(|t| t.switch_anim.clone());
    if f.weapon.as_deref() != Some(class.as_str()) {
        // No light on this weapon yet: spawn it, if there is battery left.
        if f.battery < 1 || !alive {
            runlog::kv("flashlight_refused", &format!("weapon={name} reason={} battery={} request={reason}", if alive { "battery" } else { "dead" }, f.battery));
            return;
        }
        f.weapon = Some(class);
        f.on = true;
        if sound {
            let actor = weapon_actor(&w.defs[cur]);
            w.sounds.push(PlaySound::new(SPAWN_SOUND, Emitter::Listener).slot(SoundSlot::Misc).volume(100.0).actor(actor));
        }
    } else {
        f.on = !f.on;
    }
    // The alt fire's PlayFiring comes right after ToggleTorch in
    // ModeDoFire and replaces this animation with FireAnim.
    if let Some(anim) = switch_anim.filter(|_| reason != "alt_fire")
        && has_anim(w, &anim)
    {
        play(w, &anim, w.defs[cur].modes[0].anim_rate, false);
    }
    runlog::kv("flashlight_toggle", &format!("weapon={name} on={} battery={} percent={} request={reason}", f.on, f.battery, f.percent()));
}

/// Runs after weapon_input: the toggles asked for, the pending light, the
/// light going off by itself.
pub(super) fn torch_update(
    mut commands: Commands,
    weapons: Option<ResMut<Weapons>>,
    health: Res<crate::game::combat::PlayerHealth>,
    mut holder: Query<(Entity, Option<&mut Flashlight>), With<FlyCamera>>,
) {
    let Some(mut w) = weapons else {
        return;
    };
    let Ok((entity, f)) = holder.single_mut() else {
        return;
    };
    // The local player holds a flashlight from the start (KFHumanPawn).
    let Some(mut f) = f else {
        commands.entity(entity).insert(Flashlight::default());
        return;
    };
    let alive = health.health > 0.0;
    let cur = w.current;
    let class = w.defs[cur].class.clone();
    // The projector destroys itself once the pawn's weapon is no longer
    // its weapon (Effect_TacLightProjector.Tick).
    if f.weapon.as_deref().is_some_and(|c| c != class) {
        if f.on {
            runlog::kv("flashlight_off", &format!("weapon={} reason=weapon_changed", f.weapon.as_deref().unwrap_or("")));
        }
        f.weapon = None;
        f.on = false;
    }
    let requests = std::mem::take(&mut w.torch.requests);
    for r in requests {
        match r {
            TorchRequest::AltFire if w.defs[w.current].torch.is_some() => server_spawn_light(&mut w, &mut f, alive, false, "alt_fire"),
            TorchRequest::AltFire => {}
        }
    }
    // KFWeapon.Timer at the end of BringUp: bPendingFlashlight -> LightFire.
    if let Some(p) = w.torch.pending {
        if p == w.current && w.action == Action::Idle {
            w.torch.pending = None;
            if w.defs[p].torch.is_some() {
                server_spawn_light(&mut w, &mut f, alive, true, "pending");
            }
        } else if p != w.current && !matches!(w.action, Action::PutDown { next } if next == p) {
            // Another switch replaced it.
            w.torch.pending = None;
        }
    }
    // KFWeapon.WeaponTick: off when dead, out of battery, or switching.
    if f.on {
        let reason = if !alive {
            Some("dead")
        } else if f.battery <= 0 {
            Some("battery")
        } else if matches!(w.action, Action::PutDown { .. }) {
            Some("switching")
        } else {
            None
        };
        if let Some(reason) = reason {
            f.on = false;
            runlog::kv("flashlight_off", &format!("weapon={} reason={reason} battery={}", w.defs[w.current].item_name, f.battery));
        }
    }
}

/// After the weapon is posed: the beam start and direction from the
/// LightBone (Effect_TacLightProjector.Tick, first person), and the
/// TacShine mesh on the LightBone while the light is on.
#[allow(clippy::type_complexity, clippy::too_many_arguments)] // Bevy system parameters
pub(super) fn torch_beam(
    mut commands: Commands,
    weapons: Option<ResMut<Weapons>>,
    assets: NonSend<WeaponAssets>,
    mut holder: Query<&mut Flashlight, With<FlyCamera>>,
    mut shine: Query<(&mut Transform, &mut Visibility), Without<FlyCamera>>,
    weapon_cam: Query<&Camera, With<WeaponCamera>>,
    (mut meshes, mut images, mut materials): (ResMut<Assets<Mesh>>, ResMut<Assets<Image>>, ResMut<Assets<StandardMaterial>>),
    mut shown: Local<bool>,
) {
    let Some(mut w) = weapons else {
        return;
    };
    let Ok(mut f) = holder.single_mut() else {
        return;
    };
    if !w.torch.shine_tried
        && let Some(set) = assets.0.as_ref()
    {
        w.torch.shine_tried = true;
        w.torch.shine = spawn_shine(&mut commands, set, w.camera, &mut meshes, &mut images, &mut materials);
    }
    let def = &w.defs[w.current];
    let mine = f.weapon.as_deref() == Some(def.class.as_str());
    let frame = w.torch.frame.filter(|_| mine);
    f.beam = frame.map(|lf| {
        let off = def.torch.as_ref().map_or(Vec3::ZERO, |t| t.offset);
        let start = lf.weapon + (lf.bone - lf.weapon) * BONE_PULL + lf.axes.x_axis * off.x + lf.axes.y_axis * off.y + lf.axes.z_axis * off.z;
        Beam { start, dir: lf.axes.x_axis }
    });
    // TacShine: shown while the light is on (AdjustLightGraphic), on the
    // LightBone; hidden with the weapon camera (behind view).
    let show = f.on && frame.is_some() && weapon_cam.single().is_ok_and(|c| c.is_active);
    if show != *shown {
        *shown = show;
        let fmt = |v: Vec3| format!("({:.3},{:.3},{:.3})", v.x, v.y, v.z);
        runlog::kv(
            "flashlight_shine_shown",
            &format!(
                "show={show} parts={} camera_local={} bone_x_local={}",
                w.torch.shine.len(),
                frame.map_or("none".into(), |lf| fmt(lf.local)),
                frame.map_or("none".into(), |lf| fmt(lf.local_axes.x_axis))
            ),
        );
    }
    for &e in &w.torch.shine {
        let Ok((mut t, mut vis)) = shine.get_mut(e) else { continue };
        vis.set_if_neq(if show { Visibility::Visible } else { Visibility::Hidden });
        if let Some(lf) = frame {
            // Unreal mesh axes -> the bone's axes, in weapon-camera space.
            let ue_axes = Mat3::from_cols(coords::dir([1.0, 0.0, 0.0]), coords::dir([0.0, 1.0, 0.0]), coords::dir([0.0, 0.0, 1.0]));
            let mut m = lf.local_axes;
            if m.determinant() < 0.0 {
                m.y_axis = -m.y_axis;
            }
            *t = Transform { translation: lf.local, rotation: Quat::from_mat3(&(m * ue_axes.transpose())), scale: Vec3::splat(SHINE_DRAW_SCALE) };
        }
    }
}

/// TacLightShineAttachment's mesh, hidden, under the weapon camera.
fn spawn_shine(
    commands: &mut Commands,
    set: &PackageSet,
    cam: Entity,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> Vec<Entity> {
    let model = set
        .find_object(SHINE_MESH, Some("SkeletalMesh"))
        .ok_or_else(|| "mesh not found".to_string())
        .and_then(|h| SkinnedModel::load(set, &h, &Skins { refs: Vec::new(), package: None, named: Vec::new() }, false, meshes, images, materials));
    let model = match model {
        Ok(m) => m,
        Err(e) => {
            runlog::kv("flashlight_shine", &format!("mesh={SHINE_MESH} loaded=false error={e}"));
            return Vec::new();
        }
    };
    let skinned = model.skin(&model.bind_pose(), &[]);
    let (scale, origin) = (Vec3::from_array(model.mesh.scale), Vec3::from_array(model.mesh.origin));
    model.upload(&skinned, |p| coords::pos(((p - origin) * scale).to_array()), meshes);
    let (lo, hi) = skinned.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    runlog::kv(
        "flashlight_shine",
        &format!(
            "mesh={SHINE_MESH} loaded=true parts={} points={} bounds_unreal=({:.1},{:.1},{:.1})..({:.1},{:.1},{:.1}) draw_scale={SHINE_DRAW_SCALE}",
            model.parts.len(),
            model.mesh.points.len(),
            lo.x,
            lo.y,
            lo.z,
            hi.x,
            hi.y,
            hi.z
        ),
    );
    model
        .parts
        .iter()
        .map(|part| {
            commands
                .spawn((
                    Mesh3d(part.mesh.clone()),
                    MeshMaterial3d(part.material.clone()),
                    Transform::default(),
                    RenderLayers::layer(WEAPON_LAYER),
                    Visibility::Hidden,
                    ChildOf(cam),
                ))
                .id()
        })
        .collect()
}
