//! Animation and view: playing the weapon's animations, placing the first-person model (bob, iron sights, FOV), the muzzle flash and shell effects.

use super::*;

/// UE2 Pawn.CalcDrawOffset scales PlayerViewOffset by 0.9 / DisplayFOV * 100.
pub(super) fn draw_offset_factor(display_fov: f32) -> f32 {
    0.9 / display_fov * 100.0
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)] // Bevy system parameters
pub(super) fn animate_weapon(
    time: Res<Time>,
    weapons: Option<ResMut<Weapons>>,
    mut meshes: ResMut<Assets<Mesh>>,
    main_cam: Query<&Transform, (With<FlyCamera>, Without<WeaponCamera>)>,
    mut weapon_cam: Query<(&mut Transform, &mut Projection), With<WeaponCamera>>,
    mut visibility: Query<&mut Visibility>,
    mut weapon_parts: Query<&mut Transform, (Without<FlyCamera>, Without<WeaponCamera>)>,
    bob: Res<crate::player::walk::ViewBob>,
    mut view_fov: ResMut<crate::engine::camera::ViewFov>,
    graphics: Res<crate::engine::graphics::GraphicsSettings>,
    mut log_timer: Local<f32>,
    mut scope_request: ResMut<crate::weapons::scope::ScopeRequest>,
    scope_view: Option<Res<crate::weapons::scope::ScopeView>>,
    mut part_materials: Query<&mut MeshMaterial3d<StandardMaterial>>,
) {
    let Some(mut w) = weapons else {
        return;
    };
    let dt = time.delta_secs();

    // Iron sight zoom: blend toward the target over the zoom time. When a
    // zoom finishes while idle, switch between the hip and iron idles
    // (KFWeapon.OnZoomInFinished / OnZoomOutFinished).
    let target = if w.aiming { 1.0 } else { 0.0 };
    if w.zoom != target {
        let step = dt / w.zoom_time.max(1e-3);
        w.zoom = if w.aiming { (w.zoom + step).min(1.0) } else { (w.zoom - step).max(0.0) };
        if w.zoom == target {
            runlog::kv("iron_sights_zoom_done", &format!("aiming={}", w.aiming));
            // Swap the idle for the aimed idle (or back) if idling.
            if w.action == Action::Idle && w.looping && !w.firing.iter().any(|&f| f) {
                play_idle(&mut w);
            }
        }
    }
    let def = &w.defs[w.current];
    // The player's DefaultFOV (`--fov`); iron sights blend to their own.
    let base_fov = graphics.fov;
    let (display_fov, player_fov) = match &def.iron {
        Some(iron) => (
            def.display_fov + (iron.display_fov - def.display_fov) * w.zoom,
            base_fov + (iron.player_fov - base_fov) * w.zoom,
        ),
        None => (def.display_fov, base_fov),
    };
    if view_fov.0 != player_fov {
        view_fov.0 = player_fov;
    }
    // Draw offset: CalcDrawOffset at the current DisplayFOV, blended to zero
    // when aimed (a guess; the native ironsight code is not in the scripts).
    let offset = def.view_offset * draw_offset_factor(display_fov) * (1.0 - w.zoom);

    // Weapon camera follows the main camera and uses this weapon's FOV.
    if let (Ok(main), Ok((mut wt, mut proj))) = (main_cam.single(), weapon_cam.single_mut()) {
        *wt = *main;
        if let Projection::Perspective(p) = proj.as_mut() {
            p.fov = crate::engine::camera::vertical_fov(display_fov);
        }
    }

    // Advance the animation; finished one-shots lead to the next action.
    w.frame += dt * w.play_rate;
    let length = w.sequence.map_or(1.0, |s| w.defs[w.current].model.length(s));
    // Sound notifies passed this frame (reloads, the shotgun's pump), also
    // across a loop's wrap.
    let (from, to) = (w.notify_frame, w.frame.min(length));
    anim_sounds(&mut w, from, to);
    if w.looping && w.frame >= length {
        let wrapped = w.frame % length.max(1e-3);
        anim_sounds(&mut w, -1.0, wrapped);
        w.notify_frame = wrapped;
    } else {
        w.notify_frame = w.frame;
    }
    if w.frame >= length {
        if w.looping {
            w.frame %= length.max(1e-3);
        } else {
            match w.action {
                // Switching runs on timers (weapon_input); hold the last frame.
                Action::PutDown { .. } | Action::Select | Action::Grenade { .. } => w.frame = length,
                // Weapon.AnimEnd: after FireAnim comes FireEndAnim if the
                // weapon has it; otherwise idle unless a mode is firing
                // (then the last frame holds until the next shot).
                Action::Idle | Action::Reload => {
                    let next_end = (0..2).find_map(|m| {
                        let fm = &w.defs[w.current].modes[m];
                        (w.anim == fm.anims[0].to_ascii_lowercase() && has_anim(&w, &fm.end_anim))
                            .then(|| (fm.end_anim.clone(), fm.end_anim_rate))
                    });
                    // PipeBombExplosive.AnimEnd: after the toss, the next bomb
                    // comes out (SelectAnim) if there is one.
                    let def = &w.defs[w.current];
                    let pipe_next = def.modes[0].spawn_delay.is_some()
                        && w.anim == def.modes[0].anims[0].to_ascii_lowercase()
                        && def.ammo.is_some_and(|a| a.mag + a.spare > 0)
                        && !def.gone;
                    if pipe_next {
                        let (anim, rate) = (def.select_anim.clone(), def.select_anim_rate);
                        play(&mut w, &anim, rate, false);
                    } else if let Some((end, rate)) = next_end {
                        play(&mut w, &end, rate, false);
                    } else if w.charge_hold.is_some() {
                        // HuskGun.AnimEnd while charging: ChargeLoop(_Iron).
                        let anim = if w.aiming { "ChargeLoop_Iron" } else { "ChargeLoop" };
                        play(&mut w, anim, 1.0, true);
                    } else if !w.firing.iter().any(|&f| f) {
                        play_idle(&mut w);
                    } else {
                        w.frame = length;
                    }
                }
            }
        }
    }

    // Weapon bob (Pawn.WeaponBob): BobDamping x WalkBob sideways, and
    // (0.45 + 0.55 x BobDamping) x WalkBob vertically, plus LandBob. The
    // camera already moved by twice WalkBob (CalcFirstPersonView), so
    // relative to it the weapon moves by the difference.
    let mut part_translation = offset;
    if let Ok(main) = main_cam.single() {
        let d = w.defs[w.current].bob_damping;
        let world = bob.side * (d - 2.0) + Vec3::Y * (bob.up * (0.45 + 0.55 * d - 2.0) + bob.land);
        let local = main.rotation.inverse() * world;
        part_translation = offset + local;
        for &e in &w.defs[w.current].entities {
            if let Ok(mut t) = weapon_parts.get_mut(e) {
                t.translation = part_translation;
            }
        }
    }

    // Show only the current weapon.
    for (i, def) in w.defs.iter().enumerate() {
        for &e in &def.entities {
            if let Ok(mut v) = visibility.get_mut(e) {
                *v = if i == w.current { Visibility::Visible } else { Visibility::Hidden };
            }
        }
    }

    // 3D scope (Crossbow.RenderOverlays): while aiming, the lens shows the
    // scope view; otherwise its own material (ScriptedTextureFallback).
    let scope_on = w.aiming && w.defs[w.current].scope.is_some();
    match &w.defs[w.current].scope {
        Some(sc) if scope_on => {
            scope_request.active = true;
            scope_request.fov_deg = sc.portal_fov;
            scope_request.reticle = sc.reticle.clone();
        }
        _ => scope_request.active = false,
    }
    if let Some(view) = scope_view.as_deref() {
        for (i, def) in w.defs.iter().enumerate() {
            let Some(sc) = &def.scope else { continue };
            let wanted = if i == w.current && scope_on {
                view.lens_material.clone()
            } else {
                def.model.parts[sc.lens_part].material.clone()
            };
            if let Ok(mut m) = part_materials.get_mut(def.entities[sc.lens_part])
                && m.0 != wanted
            {
                m.0 = wanted;
            }
        }
    }

    // Pose, then mesh space -> drawn: subtract MeshOrigin, scale by MeshScale (UE2).
    let def = &w.defs[w.current];
    let (skinned, bones) = def.model.pose_with_bones(w.sequence, w.frame);
    let scale = Vec3::from_array(def.model.mesh.scale);
    let origin = Vec3::from_array(def.model.mesh.origin);
    def.model.upload(&skinned, |p| coords::pos(((p - origin) * scale).to_array()), &mut meshes);
    // The effect bones in the world. The weapon camera sits where the main
    // camera is, so the weapon's camera-space parts are in world space as KF
    // places its first-person weapon (Instigator.Location + CalcDrawOffset).
    let frames: Vec<_> = match main_cam.single() {
        Ok(main) => {
            let to_world = |bone: Option<usize>| -> Option<(Vec3, Mat3)> {
                let (o, axes) = def.model.bone_frame(&bones, bone?)?;
                let local = coords::pos(((o - origin) * scale).to_array()) + part_translation;
                let pos = to_ue(main.transform_point(local)) / coords::SCALE;
                let axes = axes.map(|a| to_ue(main.rotation * coords::dir((a * scale).to_array())).normalize_or_zero());
                Some((pos, Mat3::from_cols(axes[0], axes[1], axes[2])))
            };
            def.fx.hands.iter().map(|h| (to_world(h.flash_bone), to_world(h.shell_bone))).collect()
        }
        Err(_) => Vec::new(),
    };
    w.hand_frames = frames;
    // The flashlight's LightBone (torch.rs).
    let light = main_cam.single().ok().and_then(|main| light_frame(&w.defs[w.current], &bones, part_translation, main));
    w.torch.frame = light;
    let def = &w.defs[w.current];

    *log_timer += dt;
    if *log_timer >= 1.0 {
        *log_timer = 0.0;
        let (lo, hi) = skinned.iter().fold((Vec3::MAX, Vec3::MIN), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        runlog::kv(
            "weapon_pose",
            &format!(
                "weapon={} action={:?} frame={:.1} aiming={} zoom={:.2} view_fov={:.1} display_fov={:.1} skinned_bounds_unreal=({:.1},{:.1},{:.1})..({:.1},{:.1},{:.1})",
                def.class, w.action, w.frame, w.aiming, w.zoom, view_fov.0, display_fov, lo.x, lo.y, lo.z, hi.x, hi.y, hi.z
            ),
        );
    }
}

/// Bevy direction or position -> Unreal axes (before any SCALE).
pub(super) fn to_ue(v: Vec3) -> Vec3 {
    Vec3::new(-v.z, v.x, v.y)
}

/// KFFire.InitEffects / FlashMuzzleFlash: each weapon's flash and shell
/// ejector are spawned once (drawn on the weapon layer, as KF draws them
/// with the weapon, Canvas.DrawActor at DisplayFOV), kept on their bones,
/// and triggered on every shot.
pub(super) fn weapon_fire_fx(
    mut commands: Commands,
    weapons: Option<ResMut<Weapons>>,
    library: Option<Res<crate::render::particles::EffectLibrary>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut effects: Query<(&mut crate::render::particles::ParticleEffect, &mut Visibility)>,
    mut charge_fx_failed: Local<bool>,
) {
    let Some(mut w) = weapons else {
        return;
    };
    let current = w.current;
    if let Some(lib) = library.as_deref() {
        let options = crate::render::particles::SpawnOptions {
            persistent: true,
            layer: Some(WEAPON_LAYER),
            ..default()
        };
        for def in w.defs.iter_mut().filter(|d| !d.fx.spawn_tried) {
            def.fx.spawn_tried = true;
            let (flash_class, shell_class) = (def.fx.flash_class.clone(), def.fx.shell_class.clone());
            for hand in &mut def.fx.hands {
                for (class, slot) in [(&flash_class, &mut hand.flash), (&shell_class, &mut hand.shell)] {
                    if let (Some(class), None) = (class, &slot) {
                        *slot =
                            crate::render::particles::spawn_effect_with(&mut commands, lib, &mut meshes, class, Vec3::ZERO, Mat3::IDENTITY, 7, options);
                        if slot.is_none() {
                            runlog::kv("weapon_fx_missing", &format!("weapon={} class={class}", def.class));
                        }
                    }
                }
            }
        }
    }
    // HuskGunFire.InitChargeEffect / DestroyChargeEffect: the charge glow
    // on the 'tip' bone while charging. Its growth with the charge (Timer
    // changing emitter sizes) is not done.
    let tip = w.hand_frames.first().and_then(|h| h.0);
    match (w.charge_hold.is_some() || w.beam.is_some(), w.charge_fx) {
        (true, None) => {
            let modes = &w.defs[current].modes;
            let class = modes[0]
                .charge
                .as_ref()
                .and_then(|c| c.effect.clone())
                .or_else(|| modes[1].beam.as_ref().and_then(|b| b.effect.clone()));
            if let (Some(class), Some(lib)) = (class, library.as_deref()) {
                let options = crate::render::particles::SpawnOptions {
                    persistent: true,
                    layer: Some(WEAPON_LAYER),
                    ..default()
                };
                let (at, axes) = tip.unwrap_or((Vec3::ZERO, Mat3::IDENTITY));
                w.charge_fx = crate::render::particles::spawn_effect_with(&mut commands, lib, &mut meshes, &class, at, axes, 11, options);
                if w.charge_fx.is_some() || !*charge_fx_failed {
                    runlog::kv("charge_fx", &format!("class={class} spawned={}", w.charge_fx.is_some()));
                }
                *charge_fx_failed = w.charge_fx.is_none();
            }
        }
        (true, Some(e)) => {
            if let (Ok((mut fx, _)), Some(f)) = (effects.get_mut(e), tip) {
                fx.frame = f;
            }
        }
        (false, Some(e)) => {
            commands.entity(e).despawn();
            w.charge_fx = None;
        }
        (false, None) => {}
    }
    let shots = std::mem::take(&mut w.fx_shots);
    let frames = w.hand_frames.clone();
    for (i, def) in w.defs.iter().enumerate() {
        for (h, hand) in def.fx.hands.iter().enumerate() {
            let (tip, shell) = frames.get(h).copied().unwrap_or((None, None));
            for (entity, frame) in [(hand.flash, tip), (hand.shell, shell)] {
                let Some(Ok((mut fx, mut vis))) = entity.map(|e| effects.get_mut(e)) else {
                    continue;
                };
                // Only the weapon in hand draws its effects.
                *vis = if i == current { Visibility::Inherited } else { Visibility::Hidden };
                if i != current {
                    continue;
                }
                if let Some(f) = frame {
                    fx.frame = f;
                }
                for _ in shots.iter().filter(|&&s| s == h) {
                    fx.trigger();
                }
            }
        }
    }
    if !shots.is_empty() {
        let def = &w.defs[current];
        let fmt = |f: Option<(Vec3, Mat3)>| {
            f.map_or("none".to_string(), |(p, m)| {
                format!("({:.1}, {:.1}, {:.1}) x_axis=({:.2}, {:.2}, {:.2})", p.x, p.y, p.z, m.x_axis.x, m.x_axis.y, m.x_axis.z)
            })
        };
        for &h in &shots {
            let (tip, shell) = frames.get(h).copied().unwrap_or((None, None));
            runlog::kv(
                "weapon_fx",
                &format!(
                    "weapon={} hand={h} flash={} shell={} tip_unreal={} shell_unreal={}",
                    def.class,
                    def.fx.hands.get(h).is_some_and(|x| x.flash.is_some()),
                    def.fx.hands.get(h).is_some_and(|x| x.shell.is_some()),
                    fmt(tip),
                    fmt(shell)
                ),
            );
        }
    }
}
