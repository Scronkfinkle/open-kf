//! The weapon in hand each frame: input, firing (all fire kinds), reloading, switching, aiming, welding, the syringe, grenades.

use super::*;

/// Changes the weapon's state and plays its animation (Select, Idle,
/// Reload, PutDown).
pub(super) fn set_action(w: &mut Weapons, action: Action) {
    w.action = action;
    let def = &w.defs[w.current];
    let (name, rate) = match action {
        Action::Select => {
            w.switch_timer = def.bring_up_time;
            // KFWeapon.BringUp: SelectSound, SLOT_Interact, the weapon's
            // TransientSoundVolume and radius.
            if let Some(sel) = def.select_sound.clone() {
                let actor = weapon_actor(def);
                w.sounds.push(PlaySound::new(sel, Emitter::Listener).slot(SoundSlot::Interact).volume(def.select_volume).actor(actor));
            }
            (def.select_anim.clone(), def.select_anim_rate)
        }
        Action::Idle => return play_idle(w),
        // KFWeapon.ClientReload: PlayAnim(ReloadAnim, .., 0.1).
        Action::Reload => {
            let (anim, rate) = (def.reload_anim.clone(), def.reload_anim_rate);
            play_tween(w, &anim, rate, false, 0.1);
            runlog::kv("weapon_action", &format!("weapon={} action={action:?} anim={anim}", w.defs[w.current].class));
            return;
        }
        Action::Grenade { .. } => return,
        Action::PutDown { .. } => {
            // Weapon.PutDown: wait DownDelay first if a mode fired just now
            // (NextFireTime more than FireRate x (1 - MinReloadPct 0.5) away).
            let delay = (0..2)
                .map(|m| w.fire_cooldown[m] - def.modes[m].rate * 0.5)
                .fold(0.0f32, f32::max);
            if delay > 0.0 {
                w.switch_timer = delay;
                w.down_delayed = true;
                runlog::kv("weapon_action", &format!("weapon={} action={action:?} down_delay={delay:.3}", def.class));
                return;
            }
            w.switch_timer = def.put_down_time;
            (def.put_down_anim.clone(), def.put_down_anim_rate)
        }
    };
    play(w, &name, rate, false);
    runlog::kv("weapon_action", &format!("weapon={} action={action:?} anim={name}", w.defs[w.current].class));
}

/// Whether the weapon in hand has this animation (Actor.HasAnim).
pub(super) fn has_anim(w: &Weapons, name: &str) -> bool {
    w.defs[w.current].model.sequence(name).is_some()
}

/// Plays an animation on the weapon in hand (PlayAnim / LoopAnim) at
/// `rate` x its own rate, with no tween.
pub(super) fn play(w: &mut Weapons, name: &str, rate: f32, looping: bool) {
    play_tween(w, name, rate, looping, 0.0);
}

/// `play` with a tween (PlayAnim's TweenTime): from the pose on screen to
/// the animation's first frame over `tween` seconds, the animation's clock
/// starting after it. Asking for the looping animation that is already
/// looping only changes its rate (as KF).
pub(super) fn play_tween(w: &mut Weapons, name: &str, rate: f32, looping: bool, tween: f32) {
    let def = &w.defs[w.current];
    let seq = def.model.sequence(name);
    if looping && w.looping && seq.is_some() && seq == w.sequence {
        w.play_rate = seq.map_or(30.0, |s| def.model.rate(s)) * rate;
        return;
    }
    w.tween = if w.last_locals.0 == w.current { crate::render::anim::Tween::start(&w.last_locals.1, tween) } else { None };
    w.sequence = seq;
    w.anim = name.to_ascii_lowercase();
    w.frame = 0.0;
    // Notifies count from frame 0, exclusive: one at time 0 never fires (KF).
    w.notify_frame = 0.0;
    w.looping = looping;
    w.play_rate = w.sequence.map_or(30.0, |s| def.model.rate(s)) * rate;
    if w.sequence.is_none() {
        runlog::kv("weapon_anim_missing", &format!("weapon={} anim={name}", def.class));
    } else if !looping || name != "idle" {
        runlog::kv("weapon_anim", &format!("weapon={} anim={} rate={rate} looping={looping} tween={tween}", def.item_name, w.anim));
    }
}

/// KFWeapon.PlayIdle: IdleAimAnim while aiming, else Idle, looping, tween 0.2.
pub(super) fn play_idle(w: &mut Weapons) {
    let name = match &w.defs[w.current].iron {
        Some(iron) if w.aiming => iron.idle_anim.to_ascii_lowercase(),
        _ => w.defs[w.current].idle_anim.clone(),
    };
    play_tween(w, &name, 1.0, true, 0.2);
}

/// KFFire.PlayFiring: the first shot after pressing plays FireAnim (aimed:
/// FireAimedAnim); later shots of the same press FireLoopAnim (aimed:
/// FireLoopAimedAnim, else FireAimedAnim), when the weapon has them.
/// Melee modes cycle through FireAnims.
/// WeldFire / UnWeldFire while the button is held (KFMeleeFire.ModeDoFire
/// with WeldFire.AllowFire). `door` is the door in view (WeldFire.GetDoor
/// traces the fire mode's weaponRange, 70, from the eye).
pub(super) fn weld_fire(w: &mut Weapons, mode: usize, fm: &FireMode, door: Option<crate::world::door::WeldDoor>, now: f32) {
    let cur = w.current;
    let Some(fuel) = w.defs[cur].weld_fuel else {
        return;
    };
    let door = door.filter(|d| d.distance <= fm.combat.range);
    // Weapon.ReadyToFire: ready, the other mode not firing (bModeExclusive),
    // NextFireTime passed. AllowFire runs only then.
    let alt = 1 - mode;
    let exclusive = fm.mode_exclusive || w.defs[cur].modes[alt].mode_exclusive;
    if w.action != Action::Idle || (exclusive && (w.firing[alt] || w.fire_cooldown[alt] > 0.0)) || w.fire_cooldown[mode] > 0.0 {
        return;
    }
    let allow = match (mode, door) {
        // NoWeldTargetMessage at most every 0.5 s (FailTime).
        (0, None) => {
            if now - w.last_weld_fail > 0.5 {
                w.last_weld_fail = now;
                runlog::kv("message", "text=\"You must be near a weldable door to use the welder.\"");
            }
            false
        }
        // CantWeldTargetMessage every AllowFire call in KF; rate-limited
        // like the other here so the log stays readable.
        (0, Some(d)) if d.disallow_weld => {
            if now - w.last_weld_fail > 0.5 {
                w.last_weld_fail = now;
                runlog::kv("message", "text=\"You cannot weld this door.\"");
            }
            false
        }
        // UnWeldFire: a door with weld left; fails silently.
        (1, None) => false,
        (1, Some(d)) if d.weld <= 0.0 => false,
        _ => fuel.amount >= fuel.cost[mode],
    };
    if !allow {
        return;
    }
    if !w.firing[mode] {
        w.firing[mode] = true;
        w.shots_this_press[mode] = 0;
        w.fire_cooldown[mode] = 0.0;
    }
    if let Some(f) = w.defs[cur].weld_fuel.as_mut() {
        f.amount -= fuel.cost[mode];
    }
    w.fire_cooldown[mode] = (w.fire_cooldown[mode] + fm.rate).max(0.0);
    // WeldFire.PlayFiring: FireLoopAnim after the first, if the mesh has it.
    play_firing(w, mode, false);
    w.shots_this_press[mode] += 1;
    w.fire_count += 1;
    // WeldFire.Timer: MyDamage x the perk's GetWeldSpeedModifier (an int;
    // UnWeldFire inherits it).
    let weld_speed = w.vet.weld_speed();
    let weld_damage = (fm.combat.damage_min * weld_speed).trunc();
    w.pending_welds.push((fm.combat.damage_delay, weld_damage, fm.combat.range, mode == 1));
    runlog::kv(
        "weld_fire",
        &format!(
            "mode={} door={} distance={:.0} screen_percent={:.0} fuel={} damage={weld_damage} weld_speed={weld_speed} base_damage={} delay={}",
            if mode == 1 { "unweld" } else { "weld" },
            door.map_or(0, |d| d.index),
            door.map_or(0.0, |d| d.distance),
            // Welder.ScreenWeldPercent, shown on the welder's screen in KF.
            door.map_or(0.0, |d| if d.max_weld > 0.0 { d.weld / d.max_weld * 100.0 } else { 0.0 }),
            fuel.amount - fuel.cost[mode],
            fm.combat.damage_min,
            fm.combat.damage_delay
        ),
    );
}

pub(super) fn play_firing(w: &mut Weapons, mode: usize, last: bool) {
    fire_sound(w, mode);
    let mut m = w.defs[w.current].modes[mode].clone();
    // BoomStick: FireLastAnim / FireLastAimedAnim (fire and reload).
    if last {
        let (name, rate) = if w.aiming && has_anim(w, &m.last_aimed_anim) {
            (m.last_aimed_anim.clone(), m.anim_rate)
        } else {
            (m.last_anim.clone(), m.anim_rate)
        };
        if has_anim(w, &name) {
            return play_tween(w, &name, rate, false, m.tween_time);
        }
    }
    // DualiesFire: the left gun's turn plays FireAnim2 / FireAimedAnim2.
    if w.defs[w.current].dual && mode == 0 {
        if w.dual_left[0] {
            m.anims = vec![m.anim2.clone()];
        }
        if w.dual_left[1] {
            m.aimed_anim = m.aimed_anim2.clone();
        }
    }
    let fire = m.anims[w.fire_count % m.anims.len()].to_ascii_lowercase();
    let later = w.shots_this_press[mode] > 0;
    // FireLoopAnim / FireLoopAimedAnim tween 0, the others TweenTime.
    let (name, rate, tween) = if later && w.aiming && has_anim(w, &m.loop_aimed_anim) {
        (m.loop_aimed_anim, m.loop_anim_rate, 0.0)
    } else if w.aiming && has_anim(w, &m.aimed_anim) {
        (m.aimed_anim, m.anim_rate, m.tween_time)
    } else if later && !w.aiming && has_anim(w, &m.loop_anim) {
        (m.loop_anim, m.loop_anim_rate, 0.0)
    } else {
        (fire, m.anim_rate, m.tween_time)
    };
    play_tween(w, &name, rate, false, tween);
}

/// KFFire.PlayFireEnd (Weapon.StopFire on release): FireEndAimedAnim while
/// aiming if there is one, else FireEndAnim, if the weapon has it.
/// KFHighROFFire plays it only in full auto.
pub(super) fn play_fire_end(w: &mut Weapons, mode: usize) {
    let m = &w.defs[w.current].modes[mode];
    if m.high_rof && m.wait_for_release {
        return;
    }
    let (aimed, end, rate, tween) = (m.end_aimed_anim.clone(), m.end_anim.clone(), m.end_anim_rate, m.tween_time);
    if w.aiming && has_anim(w, &aimed) {
        play_tween(w, &aimed, rate, false, tween);
    } else if has_anim(w, &end) {
        play_tween(w, &end, rate, false, tween);
    }
}

/// The Syringe's two modes while their button is held. SyringeFire (left)
/// heals a teammate in front of you (`healee`: GetHealee, within 80
/// units; healing.rs): AttemptHeal plays the fire and the injection
/// follows InjectDelay later (`pending_inject` with `heal_target`); with
/// no one to heal only "You must be near another player to heal them!"
/// (at most every HealAttemptDelay). SyringeAltFire (alt): with Health
/// under HealthMax and a full charge, ModeDoFire plays AltFire and the
/// heal follows InjectDelay later (`pending_inject`).
pub(super) fn syringe_fire(w: &mut Weapons, mode: usize, pressed: bool, h: HealCharge, health: f32, now: f32, healee: Option<(u64, &str)>) {
    let alt = 1 - mode;
    let fm = &w.defs[w.current].modes[mode];
    let ready = w.action == Action::Idle && w.fire_cooldown[mode] <= 0.0 && w.fire_cooldown[alt] <= 0.0 && !w.firing[alt];
    if mode == 0 {
        // bWaitForRelease: one attempt per click. AllowFire: the charge
        // covers AmmoPerFire; CanFindHealee.
        if !(pressed && ready) {
            return;
        }
        match healee {
            Some((peer, name)) if h.charge >= h.cost[0] => {
                let rate = fm.rate;
                w.firing[mode] = true;
                w.fire_cooldown[mode] = rate;
                w.pending_inject = Some((h.inject_delay[0], w.current, 0));
                w.heal_target = Some(peer);
                play_firing(w, 0, false);
                runlog::kv("syringe_heal_other", &format!("peer={peer} name=\"{name}\" charge={} inject_delay={} fire_rate={rate}", h.charge, h.inject_delay[0]));
            }
            _ if now - w.last_heal_attempt > 0.5 => {
                w.last_heal_attempt = now;
                runlog::kv(
                    "syringe_no_target",
                    &format!("message=\"You must be near another player to heal them!\" charge={} healee={:?}", h.charge, healee.map(|(p, _)| p)),
                );
            }
            _ => {}
        }
        return;
    }
    if !ready {
        return;
    }
    if health >= crate::game::combat::PLAYER_HEALTH_MAX || h.charge < h.cost[1] {
        if pressed {
            runlog::kv("syringe_refused", &format!("health={health:.0} charge={}", h.charge));
        }
        return;
    }
    let rate = fm.rate;
    w.firing[mode] = true;
    w.fire_cooldown[mode] = rate;
    w.pending_inject = Some((h.inject_delay[1], w.current, 1));
    play_firing(w, 1, false);
    runlog::kv("syringe_self_heal", &format!("health={health:.0} charge={} inject_delay={} fire_rate={rate}", h.charge, h.inject_delay[1]));
}

/// KFWeapon.InterruptReload: only one-round-at-a-time reloads stop.
pub(super) fn interrupt_reload(w: &mut Weapons, reason: &str) -> bool {
    if w.action == Action::Reload && w.defs[w.current].hold_to_reload {
        runlog::kv("reload_interrupted", &format!("weapon={} reason={reason}", w.defs[w.current].item_name));
        set_action(w, Action::Idle);
        return true;
    }
    false
}

/// KFWeapon.AllowReload: not while firing, reloading or bringing the weapon
/// up; the magazine not full; spare ammo; the next shot due within 0.1 s.
pub(super) fn allow_reload(w: &Weapons) -> bool {
    let def = &w.defs[w.current];
    // BoomStick.AllowReload: not with one barrel loaded.
    if def.boomstick_reload.is_some() && def.ammo.is_some_and(|a| a.mag == 1) {
        return false;
    }
    def.ammo.is_some_and(|a| a.mag < a.capacity && a.spare > 0)
        && !w.firing.iter().any(|&f| f)
        && w.action == Action::Idle
        && w.fire_cooldown[0] <= 0.1
}

/// KFWeapon.ReloadMeNow.
pub(super) fn start_reload(w: &mut Weapons, reason: &str) {
    zoom_out(w, true, "reload");
    w.reload_timer = 0.0;
    set_action(w, Action::Reload);
    let def = &w.defs[w.current];
    runlog::kv(
        "reload_start",
        &format!(
            "weapon={} reason={reason} reload_rate={} hold_to_reload={} anim={} anim_seconds={:.2}",
            def.item_name,
            def.reload_rate,
            def.hold_to_reload,
            def.reload_anim,
            w.sequence.map_or(0.0, |s| def.model.length(s) / w.play_rate.max(1e-3))
        ),
    );
}

pub(super) fn grabbed_now(cursor: &Query<&bevy::window::CursorOptions, With<bevy::window::PrimaryWindow>>) -> bool {
    cursor
        .single()
        .is_ok_and(|c| c.grab_mode != bevy::window::CursorGrabMode::None)
}

/// Leaves iron sights; `fast` uses FastZoomOutTime (reload, switch).
pub(super) fn zoom_out(w: &mut Weapons, fast: bool, reason: &str) {
    if !w.aiming {
        return;
    }
    let cur = w.current;
    if let Some(iron) = &w.defs[cur].iron {
        w.zoom_time = if fast { iron.fast_zoom_out_time } else { iron.zoom_time };
    }
    w.aiming = false;
    // Dualies.ZoomOut, when animated (toggling off, not reload or switch):
    // GOTO_Hip stretched to the zoom time.
    if !fast && w.defs[cur].dual && has_anim(w, "goto_hip") {
        let def = &w.defs[cur];
        let seq = def.model.sequence("goto_hip").expect("checked");
        let seconds = def.model.length(seq) / def.model.rate(seq).max(1e-3);
        let rate = if w.zoom_time > 0.0 && seconds > 0.0 { seconds / w.zoom_time } else { 1.0 };
        play(w, "goto_hip", rate, false);
    }
    runlog::kv("iron_sights", &format!("weapon={} aiming=false reason={reason}", w.defs[cur].class));
}

/// KFWeapon.ToggleIronSights / IronSightZoomIn: not in the air (Falling),
/// a one-round reload is interrupted, any other reload or a busy weapon
/// (switching, grenade) refuses. True if aiming now. `pressed`: a real
/// press (Hold's retries while held neither interrupt a reload nor log
/// a refusal: they wait for the weapon to be ready).
pub(super) fn try_zoom_in(w: &mut Weapons, in_air: bool, reason: &str, mode: &str, pressed: bool) -> bool {
    let cur = w.current;
    if w.defs[cur].iron.is_some() && !w.defs[cur].gone && !in_air && {
        if pressed {
            interrupt_reload(w, "aim");
        }
        w.action == Action::Idle
    } {
        let iron = w.defs[cur].iron.as_ref().expect("checked");
        w.zoom_time = iron.zoom_time;
        w.aiming = true;
        // Dualies.ZoomIn plays GOTO_Iron.
        if w.defs[cur].dual && has_anim(w, "goto_iron") {
            play(w, "goto_iron", 1.0, false);
        }
        runlog::kv("iron_sights", &format!("weapon={} aiming=true reason={reason} mode={mode}", w.defs[cur].class));
        true
    } else {
        if pressed {
            runlog::kv(
                "iron_sights_refused",
                &format!(
                    "weapon={} has_iron_sights={} action={:?} in_air={in_air} mode={mode}",
                    w.defs[cur].class,
                    w.defs[cur].iron.is_some(),
                    w.action
                ),
            );
        }
        false
    }
}

/// Pawn.PendingWeapon = `next` and the weapon in hand is put down (slot
/// keys, the weapon bar, quick heal, the flashlight key). While putting a
/// weapon down, the new choice replaces the pending one.
fn select_weapon(w: &mut Weapons, next: usize) {
    match w.action {
        Action::PutDown { .. } => w.action = Action::PutDown { next },
        Action::Grenade { .. } => {}
        _ if next != w.current => {
            // KFWeapon.PutDown: a one-round reload is interrupted; any
            // other reload refuses the switch.
            interrupt_reload(w, "switch");
            if w.action == Action::Reload {
                runlog::kv("switch_refused", &format!("weapon={} reason=reloading", w.defs[w.current].item_name));
            } else {
                zoom_out(w, true, "switch");
                w.pending_swings.clear();
                w.firing = [false; 2];
                set_action(w, Action::PutDown { next });
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)] // Bevy system parameters
pub(super) fn weapon_input(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Query<&bevy::window::CursorOptions, With<bevy::window::PrimaryWindow>>,
    weapons: Option<ResMut<Weapons>>,
    mut effects: Option<ResMut<WeaponEffects>>,
    script: Res<ScriptedInput>,
    scroll: Res<bevy::input::mouse::AccumulatedMouseScroll>,
    frames: Res<bevy::diagnostic::FrameCount>,
    main_cam: Query<(&Transform, Option<&crate::player::walk::Walker>), With<FlyCamera>>,
    (mut shots, mut swings, mut pellets, mut kicks): (
        MessageWriter<ShotFired>,
        MessageWriter<MeleeSwing>,
        MessageWriter<crate::weapons::projectile::SpawnPlayerProjectile>,
        MessageWriter<crate::player::walk::PlayerAddVelocity>,
    ),
    mut ammo_display: ResMut<crate::game::combat::AmmoDisplay>,
    mut recoil: ResMut<crate::weapons::firing::Recoil>,
    (health, mut bolt_room, mut bolts_picked, mut heals, mut beam_zaps): (
        Res<crate::game::combat::PlayerHealth>,
        ResMut<crate::weapons::projectile::BoltRoom>,
        MessageReader<crate::weapons::projectile::BoltPickedUp>,
        MessageWriter<crate::game::combat::GiveHealth>,
        MessageWriter<crate::weapons::projectile::BeamZap>,
    ),
    (weld_view, mut weld_hits, match_over, teammates, mut heal_out): (
        Res<crate::world::door::WeldView>,
        MessageWriter<crate::world::door::WeldHit>,
        Option<Res<crate::game::end_game::MatchOver>>,
        Res<crate::game::healing::Teammates>,
        MessageWriter<crate::game::healing::HealTeammate>,
    ),
    (mut scripted_held, aim_in, mut bar): (
        Local<[bool; 2]>,
        (
            Local<(bool, u32)>,
            Res<super::AimSetting>,
            Res<crate::game::menus::MenuState>,
            Res<crate::game::buy_menu::BuyMenu>,
        ),
        ResMut<super::weapon_bar::WeaponBar>,
    ),
) {
    let Some(mut w) = weapons else {
        return;
    };
    // GameEnded: Fire restarts the game (end_game.rs), SwitchWeapon and
    // ThrowWeapon do nothing; the weapon stops (bEndOfRound).
    if match_over.is_some_and(|m| m.active()) {
        return;
    }
    // Dead (a network game goes on without this player until the wave
    // ends): no weapon.
    if health.dead {
        // Ours: the bar closes (KF only stops drawing it without a pawn).
        bar.hide(time.elapsed_secs(), "death");
        w.firing = [false; 2];
        // Pawn.Died: the weapon is gone; the new pawn's starts not aiming.
        zoom_out(&mut w, true, "death");
        return;
    }
    let scripted = |action: &str| script.0.iter().any(|(f, a)| *f == frames.0 && a == action);
    // Time to each mode's NextFireTime; negative = overdue (kept, so the
    // next shot comes FireRate after the last was due, as
    // `NextFireTime += FireRate` does, and the average rate is exact).
    for cd in &mut w.fire_cooldown {
        *cd = (*cd - time.delta_secs()).max(-1.0);
    }
    let now_s = time.elapsed_secs();
    // Syringe.Tick / KFMedicGun.Tick: +10 x the perk's GetSyringeChargeRate
    // (an int) every AmmoRegenRate while under the maximum.
    let charge_step = (10.0 * w.vet.syringe_charge_rate()) as u32;
    for d in &mut w.defs {
        if let Some(h) = d.heal_charge.as_mut()
            && h.charge < HEAL_CHARGE_MAX
            && h.next_regen < now_s
        {
            h.next_regen = now_s + h.regen_rate;
            h.charge = (h.charge + charge_step).min(HEAL_CHARGE_MAX);
        }
    }
    // Welder.Tick: fuel back at AmmoRegenRate while under MaxAmmo.
    for d in &mut w.defs {
        if let Some(f) = d.weld_fuel.as_mut()
            && f.amount < f.max
        {
            f.regen_count += time.delta_secs() * f.regen_rate;
            let whole = f.regen_count.floor();
            f.amount = (f.amount + whole as u32).min(f.max);
            f.regen_count -= whole;
        }
    }
    // SyringeAltFire.Timer (InjectDelay after the shot): use the charge,
    // GiveHealth(HealBoostAmount, 100).
    if let Some((t, wi, mode)) = w.pending_inject {
        let t = t - time.delta_secs();
        if t > 0.0 {
            w.pending_inject = Some((t, wi, mode));
        } else if mode == 0 {
            // SyringeFire.Timer: the cached healee, if still alive:
            // ConsumeAmmo, MedicReward = HealBoostAmount x GetHealPotency
            // (an int), GiveHealth(HealSum, HealthMax) on them.
            w.pending_inject = None;
            let target = w.heal_target.take();
            let potency = w.vet.heal_potency();
            if let (Some(peer), Some(h)) = (target, w.defs[wi].heal_charge.as_mut())
                && teammates.0.iter().any(|t| t.peer == peer && t.alive && t.health > 0.0)
            {
                h.charge = h.charge.saturating_sub(h.cost[0]);
                // Syringe.PostBeginPlay: 50 only with one player (NumPlayers
                // when the weapon is made; here: now).
                let base = if teammates.0.is_empty() { h.boost } else { h.boost_team };
                let amount = (base * potency).trunc();
                heal_out.write(crate::game::healing::HealTeammate { peer, heal_sum: amount, source: "syringe" });
                runlog::kv("syringe_inject_other", &format!("peer={peer} heal={amount} base_heal={base} heal_potency={potency} charge_left={}", h.charge));
            }
        } else {
            w.pending_inject = None;
            let potency = w.vet.heal_potency();
            if let Some(h) = w.defs[wi].heal_charge.as_mut() {
                if h.cost[mode] <= h.charge {
                    h.charge -= h.cost[mode];
                }
                // HealSum = HealBoostAmount x the perk's GetHealPotency;
                // GiveHealth takes an int.
                let base = if teammates.0.is_empty() { h.boost } else { h.boost_team };
                let amount = (base * potency).trunc();
                heals.write(crate::game::combat::GiveHealth { amount, max: crate::game::combat::PLAYER_HEALTH_MAX, source: "syringe" });
                runlog::kv("syringe_inject", &format!("heal={amount} base_heal={base} heal_potency={potency} charge_left={} charge_per_tick={charge_step}", h.charge));
            }
        }
    }
    // Weapon switching. Slot keys: Pawn.SwitchWeapon; mouse wheel:
    // KFHumanPawn.NextWeapon / PrevWeapon. While putting a weapon down, a
    // new choice replaces the pending one (and the wheel steps from it).
    let slot_keys = [
        (KeyCode::Digit1, 1u8, "1"),
        (KeyCode::Digit2, 2, "2"),
        (KeyCode::Digit3, 3, "3"),
        (KeyCode::Digit4, 4, "4"),
        (KeyCode::Digit5, 5, "5"),
    ];
    let mut choice = None;
    for (key, group, name) in slot_keys {
        if keys.just_pressed(key) || scripted(name) {
            choice = switch_group(&slots(&w.defs), w.current, group);
            runlog::kv("weapon_slot_key", &format!("group={group} choice={:?}", choice.map(|i| w.defs[i].item_name)));
        }
    }
    // The wheel (User.ini MouseWheelUp=NextWeapon, MouseWheelDown=
    // PrevWeapon): KFPlayerController sends both to the HUD, which opens
    // the weapon bar and moves its highlight; nothing switches until Fire
    // (weapon_bar.rs).
    bar.items = w.defs.iter().filter(|d| !d.gone && (1..=5).contains(&d.group)).map(|d| (d.class.clone(), d.group)).collect();
    let wheel = scroll.delta.y;
    let in_hand = w.defs[w.current].class.clone();
    if wheel > 0.0 || scripted("next") {
        bar.step(time.elapsed_secs(), &in_hand, true);
    } else if wheel < 0.0 || scripted("prev") {
        bar.step(time.elapsed_secs(), &in_hand, false);
    }
    // KFPawn.QuickHeal (Q): hurt, a Syringe charged to 95% or more: bring
    // it out (or, if it is in hand, inject now).
    let mut force_alt = false;
    let syringe = w.defs.iter().position(|d| d.heal_charge.is_some_and(|h| h.syringe));
    if keys.just_pressed(KeyCode::KeyQ) || scripted("quickheal") {
        let charge = syringe.and_then(|i| w.defs[i].heal_charge).map_or(0, |h| h.charge);
        if health.health >= crate::game::combat::PLAYER_HEALTH_MAX {
            runlog::kv("quick_heal_refused", "reason=full_health");
        } else if let Some(si) = syringe {
            if (charge as f32) < 0.95 * HEAL_CHARGE_MAX as f32 {
                runlog::kv("quick_heal_refused", &format!("reason=charge charge={charge}"));
            } else if w.current == si {
                force_alt = true;
                runlog::kv("quick_heal", "syringe_in_hand=true");
            } else {
                w.quick_heal = QuickHeal::Inject { back: w.current, retry: 0.0 };
                choice = Some(si);
                runlog::kv("quick_heal", &format!("syringe_in_hand=false back={}", w.defs[w.current].item_name));
            }
        }
    }
    // Syringe.Timer while quick healing: once the Syringe is ready, inject
    // (HackClientStartFire); if that fails and you are full or the charge
    // is under 75%, give up, else try again in 0.2 s. After FireRate +
    // 0.5 s, SwitchToLastWeapon. A weapon change ends it.
    let heading_to_syringe = |w: &Weapons, si: usize| w.current == si || matches!(w.action, Action::PutDown { next } if next == si);
    match w.quick_heal {
        QuickHeal::Off => {}
        _ if choice.is_none() && syringe.is_none_or(|si| !heading_to_syringe(&w, si)) => {
            w.quick_heal = QuickHeal::Off;
        }
        QuickHeal::Inject { back, retry } if w.action == Action::Idle && syringe == Some(w.current) => {
            let si = syringe.expect("checked");
            let retry = retry - time.delta_secs();
            if retry <= 0.0 {
                let h = w.defs[si].heal_charge.expect("syringe");
                let can = health.health < crate::game::combat::PLAYER_HEALTH_MAX
                    && h.charge >= h.cost[1]
                    && w.fire_cooldown[0] <= 0.0
                    && w.fire_cooldown[1] <= 0.0;
                if can {
                    force_alt = true;
                    let rate = w.defs[si].modes[1].rate;
                    w.quick_heal = QuickHeal::Back { back, timer: rate + 0.5 };
                } else if health.health >= crate::game::combat::PLAYER_HEALTH_MAX || (h.charge as f32) < 0.75 * HEAL_CHARGE_MAX as f32 {
                    w.quick_heal = QuickHeal::Back { back, timer: 0.2 };
                } else {
                    w.quick_heal = QuickHeal::Inject { back, retry: 0.2 };
                }
            } else {
                w.quick_heal = QuickHeal::Inject { back, retry };
            }
        }
        QuickHeal::Back { back, timer } if w.action == Action::Idle && syringe == Some(w.current) => {
            let timer = timer - time.delta_secs();
            if timer <= 0.0 {
                w.quick_heal = QuickHeal::Off;
                if back < w.defs.len() && !w.defs[back].gone && back != w.current {
                    choice = Some(back);
                }
                runlog::kv("quick_heal_done", &format!("back={}", w.defs[back].item_name));
            } else {
                w.quick_heal = QuickHeal::Back { back, timer };
            }
        }
        _ => {}
    }
    // KFPawn.ToggleFlashlight (F): the light alt fire of a torch weapon in
    // hand, else bring out a torch weapon and light it (torch.rs).
    if keys.just_pressed(KeyCode::KeyF) || scripted("flashlight") {
        match flashlight_key(&mut w) {
            FlashKey::AltFire => force_alt = true,
            FlashKey::Switch(i) if i != w.current => choice = Some(i),
            _ => {}
        }
    }
    if let Some(next) = choice {
        select_weapon(&mut w, next);
    }
    // KFPawn.ThrowGrenade (G): a frag in stock, the weapon's next shot due
    // within 0.1 s, not reloading (a one-round reload is interrupted).
    if keys.just_pressed(KeyCode::KeyG) || scripted("nade") {
        let frag = w.defs.iter().position(|d| d.toss.is_some());
        let frags = frag.and_then(|i| w.defs[i].ammo).map_or(0, |a| a.mag + a.spare);
        interrupt_reload(&mut w, "grenade");
        if let Some(fi) = frag
            && frags > 0
            && w.action == Action::Idle
            && w.fire_cooldown[0] <= 0.1
            && !w.firing.iter().any(|&f| f)
        {
            zoom_out(&mut w, true, "grenade");
            let back = w.current;
            let d = &w.defs[back];
            let (anim, rate, time) = (d.put_down_anim.clone(), d.put_down_anim_rate * d.put_down_time / d.quick_put_down_time, d.quick_put_down_time);
            play(&mut w, &anim, rate, false);
            w.switch_timer = time;
            w.action = Action::Grenade { phase: NadePhase::Down, back_to: back };
            runlog::kv("grenade_throw_start", &format!("weapon={} frags={frags} frag_index={fi}", w.defs[back].item_name));
        } else {
            runlog::kv("grenade_throw_refused", &format!("frags={frags} action={:?} cooldown={:.2}", w.action, w.fire_cooldown[0]));
        }
    }
    // Iron sights (DESIGN.md, "Aim down sights"). Toggle (KF's default,
    // RightMouse=ToggleAiming): a press aims or stops aiming. Hold (KF's
    // `Aiming`: IronSightZoomIn, onrelease IronSightZoomOut): aims while
    // held. Scripted: `aim` = a one-frame press, `aim_down` / `aim_up` =
    // hold / let go.
    let (mut aim_local, aim_setting, menus, buy_menu) = aim_in;
    let (aim_script_held, seen_deaths) = &mut *aim_local;
    if scripted("aim_down") {
        *aim_script_held = true;
    }
    if scripted("aim_up") {
        *aim_script_held = false;
    }
    // A death with an instant respawn (not a waves game): a new pawn,
    // so not aiming (as the death branch above).
    if health.deaths != *seen_deaths {
        *seen_deaths = health.deaths;
        zoom_out(&mut w, true, "death");
    }
    let aim_pressed = (grabbed_now(&cursor) && mouse.just_pressed(MouseButton::Right)) || scripted("aim") || scripted("aim_down");
    let aim_held = (grabbed_now(&cursor) && mouse.pressed(MouseButton::Right)) || scripted("aim") || *aim_script_held;
    let mode = aim_setting.mode();
    let in_air = main_cam.single().is_ok_and(|(_, walker)| walker.is_some_and(|wk| !wk.on_ground));
    // GUIBuyMenu.InitComponent: IronSightZoomOut. The pause menu takes
    // the mouse: Hold counts the button as let go (ours); Toggle keeps
    // the aim (KF's mid-game menu does not zoom out).
    let menu_open = !menus.stack.is_empty();
    if buy_menu.open {
        zoom_out(&mut w, true, "buy_menu");
    } else if menu_open && aim_setting.hold {
        zoom_out(&mut w, true, "menu");
    } else if !menu_open {
        // Aiming with a weapon that has no sights (a forced change kept
        // the flag): out at once.
        let cur = w.current;
        if w.aiming && (w.defs[cur].iron.is_none() || w.defs[cur].gone) {
            zoom_out(&mut w, true, "no_sights");
        }
        if !aim_setting.hold {
            if aim_pressed {
                if w.aiming {
                    zoom_out(&mut w, false, "toggle");
                } else {
                    try_zoom_in(&mut w, in_air, "toggle", mode, true);
                }
            }
        } else if aim_held && !w.aiming {
            // Ours: still held after a refusal or a forced zoom out
            // (reload, switch, grenade, landing): aim as soon as allowed.
            // Refusals are logged on the press only.
            try_zoom_in(&mut w, in_air, if aim_pressed { "hold_press" } else { "hold_retry" }, mode, aim_pressed);
        } else if !aim_held && w.aiming {
            zoom_out(&mut w, false, "hold_release");
        }
    }
    // Firing: left mouse = mode 0, middle mouse = mode 1 (KF's AltFire key).
    let now = time.elapsed_secs();
    let grabbed = grabbed_now(&cursor);
    let buttons = [(MouseButton::Left, "fire"), (MouseButton::Middle, "altfire")];
    // Scripted "fire_down" / "fire_up" (and "altfire_...") hold a button.
    let mut pressed_by_script = [false; 2];
    for (i, (_, name)) in buttons.iter().enumerate() {
        if scripted(&format!("{name}_down")) {
            scripted_held[i] = true;
            pressed_by_script[i] = true;
        }
        if scripted(&format!("{name}_up")) {
            scripted_held[i] = false;
        }
    }
    let held: [bool; 2] =
        std::array::from_fn(|i| (grabbed && mouse.pressed(buttons[i].0)) || scripted(buttons[i].1) || scripted_held[i]);
    let mut pressed: [bool; 2] = std::array::from_fn(|i| {
        (grabbed && mouse.just_pressed(buttons[i].0)) || scripted(buttons[i].1) || pressed_by_script[i]
    });
    let mut held = held;
    // KFPlayerController.Fire while the weapon bar is shown: HUD.SelectWeapon
    // (hide it; the highlighted weapon becomes the pending one, the one in
    // hand is put down) and bFire = 0, so the click does not shoot.
    if pressed[0] && bar.shown {
        bar.hide(now, "fire");
        bar.swallow_fire = true;
        let pick = bar.highlighted.as_deref().and_then(|c| owned_index(&w, c)).filter(|&i| w.defs[i].group > 0);
        runlog::kv("weapon_bar_select", &format!("weapon={} from={}", pick.map_or("none", |i| w.defs[i].item_name), w.defs[w.current].item_name));
        if let Some(next) = pick {
            select_weapon(&mut w, next);
        }
    }
    if bar.swallow_fire {
        if held[0] {
            held[0] = false;
            pressed[0] = false;
        } else {
            bar.swallow_fire = false;
        }
    }
    if force_alt {
        held[1] = true;
        pressed[1] = true;
    }
    // BoomStick.ClientStartFire: both barrels asked for with one loaded
    // fires the single barrel.
    if w.defs[w.current].boomstick_reload.is_some() && w.defs[w.current].ammo.is_some_and(|a| a.mag == 1) && held[1] {
        held[0] = true;
        pressed[0] |= pressed[1];
        held[1] = false;
        pressed[1] = false;
    }
    // A charge ends without a shot if the weapon leaves its ready state.
    if w.charge_hold.is_some() && !matches!(w.action, Action::Idle) {
        runlog::kv("charge_cancelled", &format!("weapon={} action={:?}", w.defs[w.current].item_name, w.action));
        w.charge_hold = None;
        w.firing[0] = false;
    }
    for mode in 0..2 {
        let alt = 1 - mode;
        let cur = w.current;
        // bFireOnRelease (HuskGunFire): letting go fires the charged shot.
        let charge_release = if !held[mode] && mode == 0 { w.charge_hold.take() } else { None };
        // Weapon.StopFire on release: the fire end animation.
        if !held[mode] && charge_release.is_none() {
            // ZEDGunAltFire: letting go ends the beam (StopFiring; the
            // Timer then plays PlayFireEnd: ChargeDown).
            if w.defs[cur].modes[mode].beam.is_some() && w.beam.take().is_some() {
                w.firing[mode] = false;
                play(&mut w, "ChargeDown", 1.0, false);
                // ZEDGunAltFire.PlayFireEnd: the spin-down (StereoFireSound).
                fire_sound(&mut w, mode);
                runlog::kv("beam_stop", &format!("weapon={} reason=released", w.defs[cur].item_name));
                w.press_waiting[mode] = false;
                continue;
            }
            if w.firing[mode] {
                w.firing[mode] = false;
                if matches!(w.action, Action::Idle | Action::Reload) {
                    play_fire_end(&mut w, mode);
                }
            }
            w.press_waiting[mode] = false;
            continue;
        }
        if pressed[mode] {
            w.press_waiting[mode] = true;
        }
        let ready_state = matches!(w.action, Action::Idle | Action::Reload);
        // Rifles: alt fire switches full / semi auto (KFWeapon.DoToggle),
        // if ReadyToFire(0).
        if mode == 1 && let Some(toggle) = w.defs[cur].toggles_on_alt {
            if pressed[mode] {
                let mag_ok = w.defs[cur].ammo.is_none_or(|a| a.mag >= 1);
                if ready_state && w.action != Action::Reload && mag_ok && w.fire_cooldown[0] <= 0.0 && !w.firing[0] {
                    // DoToggle: flip the mode, ReceiveLocalizedMessage
                    // (BullpupSwitchMessage / KSGSwitchMessage, switch 0
                    // = semi auto / wide spread, 1 = full auto / tight).
                    let (class, on, mode) = match toggle {
                        AltToggle::FireMode => {
                            let m = &mut w.defs[cur].modes[0];
                            m.wait_for_release = !m.wait_for_release;
                            let semi = m.wait_for_release;
                            runlog::kv("fire_mode_toggle", &format!("weapon={} semi_auto={semi}", w.defs[cur].item_name));
                            (crate::game::hud::MessageClass::BullpupSwitch, semi, if semi { "single" } else { "auto" })
                        }
                        AltToggle::WideSpread => {
                            w.defs[cur].wide_spread = !w.defs[cur].wide_spread;
                            let wide = w.defs[cur].wide_spread;
                            runlog::kv("fire_mode_toggle", &format!("weapon={} wide_spread={wide}", w.defs[cur].item_name));
                            (crate::game::hud::MessageClass::KsgSwitch, wide, if wide { "wide" } else { "tight" })
                        }
                    };
                    let msg = crate::game::hud::LocalMessage::new(class, if on { 0 } else { 1 });
                    let text = crate::game::hud::message_text(&msg).unwrap_or_default();
                    w.hud_messages.push(msg);
                    // PlayOwnedSound(ToggleSound, SLOT_None, 2.0, .., false):
                    // not attenuated. No stock weapon sets ToggleSound, so
                    // stock KF switches silently.
                    let sound = w.defs[cur].toggle_sound.clone();
                    if let Some(s) = &sound {
                        w.sounds.push(PlaySound::new(s.clone(), Emitter::Listener).volume(2.0));
                    }
                    runlog::kv(
                        "fire_mode_switch",
                        &format!("weapon={} mode={mode} sound={} message=\"{text}\"", w.defs[cur].item_name, sound.as_deref().unwrap_or("none")),
                    );
                } else {
                    runlog::kv("fire_mode_toggle_refused", &format!("weapon={} action={:?}", w.defs[cur].item_name, w.action));
                }
            }
            continue;
        }
        let fm = w.defs[cur].modes[mode].clone();
        if let Some(bf) = fm.beam.clone() {
            // ZEDGunAltFire.AllowFire: not reloading, a round in the magazine.
            let reloading = w.action == Action::Reload;
            let mag_ok = !reloading && w.defs[cur].ammo.is_none_or(|a| a.mag >= 1);
            if w.beam.is_none() {
                let exclusive = fm.mode_exclusive || w.defs[cur].modes[alt].mode_exclusive;
                let alt_busy = exclusive && (w.firing[alt] || w.fire_cooldown[alt] > 0.0);
                if w.action != Action::Idle || alt_busy || w.fire_cooldown[mode] > 0.0 || !mag_ok {
                    continue;
                }
                w.beam = Some(BeamState { charge_up: 0.0, up_time: 0.0, do_hit: false });
                w.firing[mode] = true;
                w.fire_cooldown[mode] = 0.0;
            }
            let dt = time.delta_secs();
            let mut st = w.beam.expect("set above");
            // ModeDoFire every FireRate: DoFireEffect sets bDoHit and UpTime.
            if mag_ok && w.fire_cooldown[mode] <= 0.0 {
                w.fire_cooldown[mode] += fm.rate;
                st.do_hit = true;
                st.up_time = fm.rate + 0.1;
            }
            // ModeTick.
            if mag_ok && st.up_time > 0.0 {
                st.up_time -= dt;
                if st.charge_up == 0.0 {
                    // PlayPreFire.
                    play(&mut w, "Charge", 1.0, false);
                    runlog::kv("beam_start", &format!("weapon={}", w.defs[cur].item_name));
                }
                st.charge_up += dt;
                if st.do_hit {
                    if let Some(a) = w.defs[cur].ammo.as_mut() {
                        a.mag = a.mag.saturating_sub(1);
                    }
                    // ZEDGunAltFire.HandleRecoil: no movement term.
                    let p = crate::weapons::firing::RecoilParams { right_only: false, velocity_scale: 0.0, ..fm.recoil };
                    let r = [w.random(), w.random(), w.random()];
                    let kick = crate::weapons::firing::recoil_kick(p, 0.0, health.health, 100.0, r);
                    recoil.add(kick, fm.recoil.rate, now);
                }
                if let Ok((cam, _)) = main_cam.single() {
                    let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
                    let eye = to_ue(cam.translation) / coords::SCALE;
                    let (x, y, z) = (to_ue(*cam.forward()), to_ue(*cam.right()), to_ue(*cam.up()));
                    let scale = (st.charge_up / bf.sphere_time).min(1.0);
                    beam_zaps.write(crate::weapons::projectile::BeamZap {
                        start: eye + x * bf.offset.x + y * bf.offset.y + z * bf.offset.z,
                        dir: x,
                        range: bf.range,
                        dt,
                        do_hit: st.do_hit,
                        sphere_radius: BEAM_SPHERE_RADIUS * scale,
                        sphere_amount: fm.rate * 0.75,
                    });
                }
                st.do_hit = false;
                w.beam = Some(st);
            } else {
                // StopFiring, then the Timer: PlayFireEnd (ChargeDown), StopFire.
                w.beam = None;
                w.firing[mode] = false;
                play(&mut w, "ChargeDown", 1.0, false);
                fire_sound(&mut w, mode);
                runlog::kv("beam_stop", &format!("weapon={} reason=cannot_fire", w.defs[cur].item_name));
            }
            continue;
        }
        if fm.weld {
            weld_fire(&mut w, mode, &fm, weld_view.door, now);
            continue;
        }
        if let Some(h) = w.defs[cur].heal_charge.filter(|h| h.syringe) {
            // SyringeFire.GetHealee from this game's pawn and view.
            let healee = main_cam.single().ok().and_then(|(t, wk)| {
                let me = wk.map_or(t.translation - Vec3::Y * crate::game::combat::PLAYER_EYE_HEIGHT * coords::SCALE, |wk| wk.center);
                crate::game::healing::syringe_healee(me, *t.forward(), &teammates.0, coords::SCALE, crate::game::healing::PAWN_RADIUS)
            });
            syringe_fire(&mut w, mode, pressed[mode], h, health.health, now, healee.map(|t| (t.peer, t.name.as_str())));
            continue;
        }
        // Torch weapons: the alt fire is the flashlight (torch.rs).
        if mode == 1 && w.defs[cur].torch.is_some() {
            torch_alt_fire(&mut w);
            continue;
        }
        if mode == 1 && !matches!(fm.kind, FireKind::Melee | FireKind::Pellets) {
            // Only melee alt attacks are done; other alt fires come with
            // their weapon family (DESIGN "Weapons").
            if pressed[mode] {
                runlog::kv(
                    "alt_fire_not_implemented",
                    &format!("weapon={} fire_kind={:?} fire_class={}", w.defs[cur].item_name, fm.kind, fm.class),
                );
            }
            continue;
        }
        // KFFire.AllowFire: not while reloading (WinchesterFire and
        // KFShotgunFire: unless 2+ rounds are in), not with an empty magazine.
        let mag = w.defs[cur].ammo.map(|a| a.mag);
        let reloading = w.action == Action::Reload;
        // Rounds a shot needs: AmmoPerFire for pellet fire (BoomStickFire:
        // both barrels, 2); 1 for other primary fire; none for melee alt.
        let needs = match fm.pellets {
            Some(pf) => pf.ammo_per_fire.max(1),
            None => u32::from(mode == 0),
        };
        let total = w.defs[cur].ammo.map(|a| a.mag + a.spare);
        // The alt fire's own pool: the M4 203's grenades, or a medic gun's
        // HealAmmoCharge.
        let medic_charge = w.defs[cur].heal_charge.filter(|h| !h.syringe);
        let alt_pool = if mode == 1 { w.defs[cur].alt_ammo.map(|a| a.0).or(medic_charge.map(|h| h.charge)) } else { None };
        let aimed_in = w.aiming && w.zoom >= 1.0;
        let allow_fire = if let (Some(left), Some(_)) = (alt_pool, medic_charge) {
            // MP7MAltFire.AllowFire: not while reloading; HealAmmoCharge >= AmmoPerFire.
            !reloading && left >= needs
        } else if let Some(left) = alt_pool {
            // M203Fire.AllowFire: AmmoAmount(1) >= AmmoPerFire.
            left >= needs
        } else if fm.total_ammo_only {
            (!fm.requires_aim || aimed_in) && total.is_none_or(|t| t >= needs)
        } else {
            (!reloading || (fm.fire_while_reloading && mag.is_some_and(|m| m >= 2))) && mag.is_none_or(|m| m >= needs)
        };
        if !w.firing[mode] {
            // StartFire. bWaitForRelease modes need a fresh click (a click
            // made while not ready is kept while the button stays down; not
            // checked against KF). Weapon.ReadyToFire: not while the other
            // mode fires, and after both modes' NextFireTime.
            if fm.wait_for_release && !w.press_waiting[mode] {
                continue;
            }
            // KFWeapon.Fire: a click on an empty magazine asks for a reload.
            if mode == 0 && !fm.total_ammo_only && pressed[mode] && mag == Some(0) && !reloading && w.fire_cooldown[0] <= 0.0 {
                runlog::kv("dry_fire", &format!("weapon={} auto_reload={}", w.defs[cur].item_name, w.defs[cur].can_dry_fire));
                // KFWeapon.Fire: NoAmmoSound at 2.0, SLOT_None.
                if w.defs[cur].can_dry_fire
                    && let Some(click) = fm.sounds.no_ammo.clone()
                {
                    w.sounds.push(PlaySound::new(click, Emitter::Listener).volume(2.0));
                }
                if w.defs[cur].can_dry_fire && allow_reload(&w) {
                    start_reload(&mut w, "dry_fire");
                }
                continue;
            }
            // FlameBurstFire.AllowFire: held on an empty tank, NoAmmoSound
            // at most every FireRate.
            if fm.sounds.empty_click && held[mode] && mag == Some(0) && !reloading && now - w.last_click > fm.rate {
                w.last_click = now;
                if let Some(click) = fm.sounds.no_ammo.clone() {
                    let actor = weapon_actor(&w.defs[cur]);
                    w.sounds.push(PlaySound::new(click, Emitter::Listener).slot(SoundSlot::Interact).volume(fm.sounds.volume).actor(actor));
                }
            }
            // Weapon.ReadyToFire: the other mode blocks only if either is
            // bModeExclusive.
            let exclusive = fm.mode_exclusive || w.defs[cur].modes[alt].mode_exclusive;
            let alt_busy = exclusive && (w.firing[alt] || w.fire_cooldown[alt] > 0.0);
            if !ready_state || alt_busy || w.fire_cooldown[mode] > 0.0 || !allow_fire {
                continue;
            }
            w.firing[mode] = true;
            w.press_waiting[mode] = false;
            w.shots_this_press[mode] = 0;
            // HuskGunFire: PlayPreFire (Charge / Charge_Iron), then hold.
            if fm.charge.is_some() && mode == 0 {
                w.charge_hold = Some(0.0);
                let anim = if w.aiming { "Charge_Iron" } else { "Charge" };
                play(&mut w, anim, 1.0, false);
                runlog::kv("charge_start", &format!("weapon={}", w.defs[cur].item_name));
                continue;
            }
            // StartFire: NextFireTime = now (no PreFireTime).
            w.fire_cooldown[mode] = 0.0;
            // KFWeapon.StartFire interrupts a one-round reload.
            interrupt_reload(&mut w, "fire");
            // KFHighROFFire in full auto: state FireLoop loops the animation.
            if fm.high_rof && !fm.wait_for_release {
                let (name, rate) = if w.aiming && has_anim(&w, &fm.loop_aimed_anim) {
                    (fm.loop_aimed_anim.clone(), fm.loop_anim_rate)
                } else {
                    (fm.loop_anim.clone(), fm.loop_anim_rate)
                };
                // KFHighROFFire: LoopAnim(FireLoopAnim, .., TweenTime).
                let tween = fm.tween_time;
                play_tween(&mut w, &name, rate, true, tween);
            }
        }
        // Charging: HoldTime grows until the release.
        if fm.charge.is_some() && charge_release.is_none() {
            if let Some(h) = w.charge_hold.as_mut() {
                *h += time.delta_secs();
            }
            continue;
        }
        // ModeDoFire: once per press in semi auto, every FireRate in full auto.
        if w.fire_cooldown[mode] > 0.0 || !allow_fire || (fm.wait_for_release && w.shots_this_press[mode] > 0) {
            if charge_release.is_some() {
                w.firing[mode] = false;
                runlog::kv("charge_dropped", &format!("weapon={} allow_fire={allow_fire}", w.defs[cur].item_name));
            }
            // KFHighROFFire.ModeTick: an empty magazine ends the loop.
            if fm.high_rof && !fm.wait_for_release && !allow_fire && w.looping && w.firing[mode] {
                w.firing[mode] = false;
                play_fire_end(&mut w, mode);
            }
            continue;
        }
        // BoomStick: bVeryLastShotAnim = AmmoAmount <= AmmoPerFire (before
        // the shot); the gun is emptied when the magazine reaches 0.
        let total_before = w.defs[cur].ammo.map_or(0, |a| a.mag + a.spare);
        // PipeBombFire: the ammo goes when the projectile spawns.
        let needs = if fm.spawn_delay.is_some() { 0 } else { needs };
        let needs = match (&fm.charge, charge_release) {
            (Some(c), Some(h)) => c.ammo(h, total.unwrap_or(0)),
            _ => needs,
        };
        if needs > 0 {
            if alt_pool.is_some() {
                if let Some(h) = w.defs[cur].heal_charge.as_mut().filter(|h| !h.syringe) {
                    h.charge -= needs.min(h.charge);
                } else if let Some(a) = w.defs[cur].alt_ammo.as_mut() {
                    a.0 -= needs.min(a.0);
                }
            } else if let Some(a) = w.defs[cur].ammo.as_mut() {
                // Total-only fire takes from the spare rounds once the
                // magazine is empty (my choice for the HUD; KF counts one total).
                let from_mag = needs.min(a.mag);
                a.mag -= from_mag;
                if fm.total_ammo_only {
                    a.spare -= (needs - from_mag).min(a.spare);
                }
            }
        }
        let emptied = w.defs[cur].ammo.is_some_and(|a| a.mag == 0);
        let very_last = total_before <= needs;
        let last_anim = match fm.last_rule {
            LastShot::WhenEmptied => emptied && !very_last,
            LastShot::WhenVeryLast => very_last,
            LastShot::Never => false,
        };
        // ModeDoFire: NextFireTime = max(NextFireTime + FireRate, now); the
        // BoomStick waits FireLastRate after emptying the gun.
        let rate = match fm.last_rate {
            Some(r) if emptied => r,
            _ => fm.rate,
        };
        w.fire_cooldown[mode] = (w.fire_cooldown[mode] + rate).max(0.0);
        if emptied && let Some(t) = w.defs[cur].boomstick_reload {
            w.boomstick_pending = Some(t);
        }
        w.fire_count += 1;
        // Dual pistols: the hand whose turn it is (DualiesFire.ModeDoFire).
        let dual_side = if w.aiming { 1 } else { 0 };
        let hand = usize::from(w.defs[cur].dual && mode == 0 && w.dual_left[dual_side]);
        if mode == 0 || fm.kind == FireKind::Pellets {
            w.fx_shots.push(hand);
        }
        if !(fm.high_rof && !fm.wait_for_release) {
            play_firing(&mut w, mode, last_anim);
        }
        if w.defs[cur].dual && mode == 0 {
            w.dual_left[dual_side] = !w.dual_left[dual_side];
        }
        w.shots_this_press[mode] += 1;
        let stats = fm.combat;
        let item_name = w.defs[cur].item_name;
        match fm.kind {
            FireKind::Melee => {
                let mut stats = stats;
                if fm.chainsaw {
                    // ChainsawFire.ModeDoFire / DoFireEffect: damage now,
                    // MeleeDamage + Rand(maxAdditionalDamage), traced only.
                    stats.damage_delay = 0.0;
                    stats.damage_min += (w.random() * fm.extra_damage as f32).floor().min(fm.extra_damage.saturating_sub(1) as f32);
                    stats.min_dot = 0.0;
                }
                // KFMeleeFire.ModeDoFire: velocity x ChopSlowRate on the ground.
                let chop = w.defs[cur].chop_slow_rate;
                if chop < 1.0
                    && main_cam.single().is_ok_and(|(_, wk)| wk.is_some_and(|wk| wk.on_ground))
                    && let Some(e) = effects.as_mut()
                {
                    e.fire_velocity_scale = Some(chop);
                }
                runlog::kv(
                    "melee_swing",
                    &format!(
                        "weapon={item_name} mode={mode} class={} damage={} range={} delay={} min_dot={} chop_slow={chop}",
                        fm.class, stats.damage_min, stats.range, stats.damage_delay, stats.min_dot
                    ),
                );
                w.pending_swings.push((stats.damage_delay, stats, item_name, fm.sounds.melee_hits.clone(), fm.sounds.melee_hit_volume));
            }
            FireKind::Pellets if fm.spawn_delay.is_some() => {
                let delay = fm.spawn_delay.unwrap_or(0.0);
                w.pending_spawn = Some((delay, cur, mode));
                runlog::kv("projectile_spawn_delayed", &format!("weapon={item_name} delay={delay}"));
            }
            FireKind::Pellets => {
                let mut pf = fm.pellets.expect("pellet fire has pellet values");
                if let (Some(c), Some(h)) = (&fm.charge, charge_release) {
                    let x = c.projectile(h);
                    runlog::kv(
                        "charge_fired",
                        &format!(
                            "weapon={item_name} hold={h:.2} class={} ammo={needs} damage={:.1} radius={:.0} impact={:.0}",
                            x.class, x.damage, x.radius, x.impact_damage
                        ),
                    );
                    pf.explosive = Some(x);
                    // ModeDoFire with bFireOnRelease: NextFireTime = now + FireRate.
                    w.firing[mode] = false;
                }
                if let Ok((cam, walker)) = main_cam.single() {
                    let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
                    let eye = to_ue(cam.translation) / coords::SCALE;
                    let (x, y, z) = (to_ue(*cam.forward()), to_ue(*cam.right()), to_ue(*cam.up()));
                    // KFShotgunFire.DoFireEffect: StartProj = eye + X x
                    // ProjSpawnOffset.X, plus Y and Z offsets from the hip.
                    let mut start = eye + x * pf.spawn_offset.x;
                    if !w.aiming {
                        start += y * pf.spawn_offset.y + z * pf.spawn_offset.z;
                    }
                    // KFShotgunFire.ModeDoFire: Spread = Default.Spread x the
                    // perk's ModifyRecoilSpread; KSGFire: wide spread x 2.05.
                    let rec = recoil_mod(&w, cur, &fm);
                    let spread = if w.defs[cur].wide_spread { pf.spread * rec * 2.05 } else { pf.spread * rec };
                    // KFShotgunFire: ProjPerFire x Load; MP7MAltFire /
                    // M7A3MAltFire: ProjPerFire only (Load is the 250 charge).
                    let count = if pf.per_load { (pf.per_fire * pf.ammo_per_fire.max(1)).max(1) } else { pf.per_fire.max(1) };
                    let tip = w.hand_frames.get(hand).and_then(|h| h.0).map(|t| t.0);
                    for _ in 0..count {
                        // SS_Random: X >> R, R = Spread x (FRand() - 0.5) each.
                        let r = ue_assets::properties::Rotator {
                            yaw: (spread * (w.random() - 0.5)) as i32,
                            pitch: (spread * (w.random() - 0.5)) as i32,
                            roll: (spread * (w.random() - 0.5)) as i32,
                        };
                        let dir = coords::ue_rotation_matrix(r) * x;
                        pellets.write(crate::weapons::projectile::SpawnPlayerProjectile {
                            origin: start,
                            trace_from: eye,
                            dir,
                            stats: pf.stats,
                            weapon: item_name,
                            tracer_start: if pf.explosive.is_some() || pf.thrown.is_some() || pf.flame.is_some() || pf.dart.is_some() {
                                None
                            } else {
                                tip
                            },
                            explosive: pf.explosive,
                            thrown: pf.thrown,
                            flame: pf.flame,
                            dart: pf.dart,
                            extra_speed: 0.0,
                        });
                    }
                    // AddVelocity(KickMomentum >> view rotation), not when falling.
                    let on_ground = walker.is_some_and(|wk| wk.on_ground);
                    if on_ground && pf.kick != Vec3::ZERO {
                        kicks.write(crate::player::walk::PlayerAddVelocity {
                            velocity: x * pf.kick.x + y * pf.kick.y + z * pf.kick.z,
                        });
                    }
                    if fm.slows_movement && on_ground {
                        let scale = if fm.rate > 0.25 { 0.1 } else { 0.5 };
                        if let Some(e) = effects.as_mut() {
                            e.fire_velocity_scale = Some(scale);
                        }
                    }
                    let speed = walker.map_or(0.0, |wk| wk.velocity.length() / coords::SCALE);
                    let r = [w.random(), w.random(), w.random()];
                    let kick = crate::weapons::firing::recoil_kick(fm.recoil, speed, health.health, 100.0, r);
                    // HandleRecoil(Rec): the kick x Rec.
                    let kick = (kick.0 * rec, kick.1 * rec);
                    recoil.add(kick, recoil_rate(&fm), now);
                    if rec != 1.0 {
                        runlog::kv("perk_mod", &format!("kind=recoil_spread perk={} weapon={item_name} mult={rec} spread={spread:.0} recoil_pitch={:.0}", w.vet.label(), kick.0));
                    }
                    runlog::kv(
                        "pellet_shot",
                        &format!(
                            "weapon={item_name} mode={mode} anim={} pellets={count} damage={} spread={spread:.0} wide={} kick_unreal=({:.0}, {:.0}, {:.0}) recoil_pitch={:.0} mag_left={} emptied={emptied} last_anim={last_anim}",
                            w.anim,
                            pf.stats.damage,
                            w.defs[cur].wide_spread,
                            pf.kick.x,
                            pf.kick.y,
                            pf.kick.z,
                            kick.0,
                            w.defs[cur].ammo.map_or(0, |a| a.mag)
                        ),
                    );
                }
            }
            FireKind::Projectile | FireKind::None => runlog::kv(
                "fire_not_implemented",
                &format!("weapon={item_name} fire_kind={:?} fire_class={}", fm.kind, fm.class),
            ),
            FireKind::Instant => {
                if let Ok((cam, walker)) = main_cam.single() {
                    // KFFire.ModeDoFire: GetSpread, then InstantFire's
                    // direction; KFFire.DoTrace deals DamageMax.
                    let mut spread_state = w.spread_state[mode];
                    // KFFire.ModeDoFire: Spread = GetSpread() x the perk's
                    // ModifyRecoilSpread, and HandleRecoil(Rec).
                    let rec = recoil_mod(&w, cur, &fm);
                    let spread = crate::weapons::firing::kf_spread(fm.spread, &mut spread_state, now, w.aiming, fm.wait_for_release) * rec;
                    w.spread_state[mode] = spread_state;
                    let vrand = loop {
                        let v = Vec3::new(w.random() * 2.0 - 1.0, w.random() * 2.0 - 1.0, w.random() * 2.0 - 1.0);
                        if v.length_squared() > 1e-4 && v.length_squared() <= 1.0 {
                            break v.normalize();
                        }
                    };
                    let frand = w.random();
                    let dir = crate::weapons::firing::spread_dir(*cam.forward(), spread, vrand, frand);
                    // MAC10Fire.DoTrace: DamageType = the perk's
                    // GetMAC10DamageType (the Firebug's burns).
                    let (dam, fire) = match fm.mac10_inc {
                        Some(inc) if w.vet.mac10_incendiary() => (Some(inc), Some(crate::game::combat::FireType::Mac10)),
                        _ => (stats.dam, fm.fire),
                    };
                    shots.write(ShotFired {
                        origin: cam.translation,
                        dir,
                        damage: stats.damage_max,
                        headshot_mult: stats.headshot_mult,
                        weapon: item_name,
                        effect_start: w.hand_frames.get(hand).and_then(|h| h.0).map(|t| t.0),
                        max_penetrations: fm.penetrations,
                        fire,
                        dam,
                    });
                    // HandleRecoil; speed in Unreal units/s.
                    let speed = walker.map_or(0.0, |wk| wk.velocity.length() / coords::SCALE);
                    let r = [w.random(), w.random(), w.random()];
                    let kick = crate::weapons::firing::recoil_kick(fm.recoil, speed, health.health, 100.0, r);
                    let kick = (kick.0 * rec, kick.1 * rec);
                    recoil.add(kick, recoil_rate(&fm), now);
                    if rec != 1.0 {
                        runlog::kv("perk_mod", &format!("kind=recoil_spread perk={} weapon={item_name} mult={rec} spread={spread:.4} recoil_pitch={:.0}", w.vet.label(), kick.0));
                    }
                    // ModeDoFire slows the player unless falling.
                    let on_ground = walker.is_some_and(|wk| wk.on_ground);
                    if fm.slows_movement && on_ground {
                        let scale = if fm.rate > 0.25 { 0.1 } else { 0.5 };
                        if let Some(e) = effects.as_mut() {
                            e.fire_velocity_scale = Some(scale);
                        }
                    }
                    runlog::kv(
                        "gun_shot",
                        &format!(
                            "weapon={item_name} hand={hand} anim={} shot_in_press={} semi_auto={} aiming={} spread={spread:.4} burst={} recoil_pitch={:.0} recoil_yaw={:.0} speed_unreal={speed:.0} mag_left={}",
                            w.anim,
                            w.shots_this_press[mode],
                            fm.wait_for_release,
                            w.aiming,
                            spread_state.shots_in_burst,
                            kick.0,
                            kick.1,
                            w.defs[cur].ammo.map_or(0, |a| a.mag)
                        ),
                    );
                }
            }
        }
    }
    // Melee: damage lands DamagedelayMin seconds into the swing.
    let dt = time.delta_secs();
    let mut landed = Vec::new();
    w.pending_swings.retain_mut(|(t, stats, name, hit_sounds, hit_volume)| {
        *t -= dt;
        if *t <= 0.0 {
            landed.push((*stats, *name, hit_sounds.clone(), *hit_volume));
        }
        *t > 0.0
    });
    for (stats, name, hit_sounds, hit_volume) in landed {
        if let Ok((cam, _)) = main_cam.single() {
            swings.write(MeleeSwing {
                origin: cam.translation,
                dir: *cam.forward(),
                damage: stats.damage_min,
                range: stats.range,
                min_dot: stats.min_dot,
                headshot_mult: stats.headshot_mult,
                weapon: name,
                hit_sounds,
                hit_volume,
                dam: stats.dam,
            });
        }
    }
    // WeldFire.Timer: the weld lands DamagedelayMin after the fire, traced
    // from where the view is then.
    let mut welds_landed = Vec::new();
    w.pending_welds.retain_mut(|(t, damage, range, unweld)| {
        *t -= dt;
        if *t <= 0.0 {
            welds_landed.push((*damage, *range, *unweld));
        }
        *t > 0.0
    });
    for (damage, range, unweld) in welds_landed {
        if let Ok((cam, _)) = main_cam.single() {
            weld_hits.write(crate::world::door::WeldHit {
                origin: cam.translation,
                dir: *cam.forward(),
                range,
                damage,
                unweld,
            });
        }
    }
    // Reloading (KFWeapon.Tick): after ReloadRate the magazine is full and
    // the weapon idles (ClientFinishReloading), even if the reload
    // animation is still playing. One-round reloads add a round every
    // ReloadRate until full.
    // PipeBombFire.Timer: ConsumeAmmo, DoFireEffect; out of ammo: the
    // weapon is gone and the best other one comes up.
    if let Some((t, wi, mode)) = w.pending_spawn {
        let t = t - dt;
        if t > 0.0 {
            w.pending_spawn = Some((t, wi, mode));
        } else {
            w.pending_spawn = None;
            let pf = w.defs[wi].modes[mode].pellets;
            let snd = &w.defs[wi].modes[mode].sounds;
            if let Some(placed) = snd.placed.clone() {
                let (volume, radius, actor) = (snd.volume, snd.radius, weapon_actor(&w.defs[wi]));
                w.sounds.push(PlaySound::new(placed, Emitter::Listener).slot(SoundSlot::Interact).volume(volume).radius(radius).actor(actor));
            }
            if let Some(a) = w.defs[wi].ammo.as_mut() {
                if a.mag > 0 {
                    a.mag -= 1;
                } else {
                    a.spare = a.spare.saturating_sub(1);
                }
            }
            if let (Some(pf), Ok((cam, _))) = (pf, main_cam.single()) {
                let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
                let eye = to_ue(cam.translation) / coords::SCALE;
                let (x, y, z) = (to_ue(*cam.forward()), to_ue(*cam.right()), to_ue(*cam.up()));
                let mut start = eye + x * pf.spawn_offset.x;
                if !w.aiming {
                    start += y * pf.spawn_offset.y + z * pf.spawn_offset.z;
                }
                let r = ue_assets::properties::Rotator {
                    yaw: (pf.spread * (w.random() - 0.5)) as i32,
                    pitch: (pf.spread * (w.random() - 0.5)) as i32,
                    roll: (pf.spread * (w.random() - 0.5)) as i32,
                };
                pellets.write(crate::weapons::projectile::SpawnPlayerProjectile {
                    origin: start,
                    trace_from: eye,
                    dir: coords::ue_rotation_matrix(r) * x,
                    stats: pf.stats,
                    weapon: w.defs[wi].item_name,
                    tracer_start: None,
                    explosive: pf.explosive,
                    thrown: pf.thrown,
                    flame: pf.flame,
                    dart: pf.dart,
                    extra_speed: 0.0,
                });
            }
            let left = w.defs[wi].ammo.map_or(0, |a| a.mag + a.spare);
            if left == 0 {
                w.defs[wi].gone = true;
                runlog::kv("weapon_gone", &format!("weapon={}", w.defs[wi].item_name));
                if w.current == wi
                    && !matches!(w.action, Action::Grenade { .. } | Action::PutDown { .. })
                    && let Some(next) = step_weapon(&slots(&w.defs), wi, false)
                {
                    w.firing = [false; 2];
                    set_action(&mut w, Action::PutDown { next });
                }
            }
        }
    }
    // BoomStick.WeaponTick: ReloadCountDown after the last barrel, both
    // load (MagAmmoRemaining = Min(AmmoAmount, 2)); only while in hand.
    if let Some(t) = w.boomstick_pending
        && w.defs[w.current].boomstick_reload.is_some()
    {
        let t = t - dt;
        if t <= 0.0 {
            w.boomstick_pending = None;
            let cur = w.current;
            if let Some(a) = w.defs[cur].ammo.as_mut() {
                let n = (a.capacity - a.mag).min(a.spare);
                a.mag += n;
                a.spare -= n;
                runlog::kv("boomstick_loaded", &format!("mag={} spare={}", a.mag, a.spare));
            }
        } else {
            w.boomstick_pending = Some(t);
        }
    }
    // Weapon.Timer: the bring-up ends in idle; the put-down plays its
    // animation after any DownDelay, then the next weapon comes up.
    if matches!(w.action, Action::Select | Action::PutDown { .. } | Action::Grenade { .. }) {
        w.switch_timer -= dt;
        if w.switch_timer <= 0.0 {
            match w.action {
                Action::Grenade { phase, back_to } => {
                    let frag = w.defs.iter().position(|d| d.toss.is_some()).unwrap_or(back_to);
                    match phase {
                        NadePhase::Down => {
                            // KFPawn.WeaponDown -> Frag.StartThrow: TossAnim.
                            w.current = frag;
                            let (anim, _, spawn_at) = w.defs[frag].toss.clone().unwrap_or_default();
                            play(&mut w, &anim, 1.0, false);
                            // Frag.StartThrow: PlaySound(FireMode[0].FireSound, SLOT_Interact, 2.0).
                            if let Some(whoosh) = w.defs[frag].modes[0].sounds.fire.clone() {
                                let actor = weapon_actor(&w.defs[frag]);
                                w.sounds.push(PlaySound::new(whoosh, Emitter::Listener).slot(SoundSlot::Interact).volume(2.0).actor(actor));
                            }
                            w.switch_timer = spawn_at;
                            w.action = Action::Grenade { phase: NadePhase::Toss { spawned: false }, back_to };
                        }
                        NadePhase::Toss { spawned: false } => {
                            // Frag.ServerThrow: ConsumeAmmo, FragFire.DoFireEffect,
                            // PlaySound(ThrowSound, SLOT_Interact, 2.0).
                            if let Some(throw) = w.defs[frag].throw_sound.clone() {
                                let actor = weapon_actor(&w.defs[frag]);
                                w.sounds.push(PlaySound::new(throw, Emitter::Listener).slot(SoundSlot::Interact).volume(2.0).actor(actor));
                            }
                            if let Some(a) = w.defs[frag].ammo.as_mut() {
                                if a.mag > 0 {
                                    a.mag -= 1;
                                } else {
                                    a.spare = a.spare.saturating_sub(1);
                                }
                            }
                            let fm = w.defs[frag].modes[0].clone();
                            if let (Some(pf), Ok((cam, walker))) = (fm.pellets, main_cam.single())
                                && let Some(frag_nade) = pf.thrown
                            {
                                // FragFire.GetDesiredProjectileClass: the perk's
                                // GetNadeType.
                                let wanted = w.vet.nade_class();
                                let t = match wanted {
                                    "KFMod.FlameNade" => pf.flame_nade.unwrap_or(frag_nade),
                                    "KFMod.MedicNade" => pf.medic_nade.unwrap_or(frag_nade),
                                    _ => frag_nade,
                                };
                                runlog::kv(
                                    "perk_effect",
                                    &format!("perk={} effect=nade_type wanted={wanted} thrown={} damage={} radius={} fire={:?}", w.vet.label(), t.class, t.damage, t.radius, t.fire),
                                );
                                let to_ue = |v: Vec3| Vec3::new(-v.z, v.x, v.y);
                                let eye = to_ue(cam.translation) / coords::SCALE;
                                let (x, y, z) = (to_ue(*cam.forward()), to_ue(*cam.right()), to_ue(*cam.up()));
                                // StartProj = eye + X x 25, + Hand (right, 1) x Y x -10 + Z x 0.
                                let start = eye + x * pf.spawn_offset.x + y * pf.spawn_offset.y + z * pf.spawn_offset.z;
                                // PostSpawnProjectile: + the player's speed along the view.
                                let pawn_speed = walker.map_or(0.0, |wk| x.dot(to_ue(wk.velocity) / coords::SCALE));
                                pellets.write(crate::weapons::projectile::SpawnPlayerProjectile {
                                    origin: start,
                                    trace_from: eye,
                                    dir: x,
                                    stats: pf.stats,
                                    weapon: w.defs[frag].item_name,
                                    tracer_start: None,
                                    explosive: None,
                                    thrown: Some(t),
                                    flame: None,
                                    dart: None,
                                    extra_speed: pawn_speed,
                                });
                            }
                            let (_, toss_time, spawn_at) = w.defs[frag].toss.clone().unwrap_or_default();
                            w.switch_timer = (toss_time - spawn_at).max(0.0);
                            w.action = Action::Grenade { phase: NadePhase::Toss { spawned: true }, back_to };
                        }
                        NadePhase::Toss { spawned: true } => {
                            // ThrowGrenadeFinished: the weapon comes up quickly.
                            w.current = back_to;
                            let d = &w.defs[back_to];
                            let (anim, rate, time) =
                                (d.select_anim.clone(), d.select_anim_rate * d.bring_up_time / d.quick_bring_up_time, d.quick_bring_up_time);
                            play(&mut w, &anim, rate, false);
                            w.switch_timer = time;
                            w.action = Action::Grenade { phase: NadePhase::Up, back_to };
                        }
                        NadePhase::Up => set_action(&mut w, Action::Idle),
                    }
                }
                Action::Select => set_action(&mut w, Action::Idle),
                Action::PutDown { next } if w.down_delayed => {
                    w.down_delayed = false;
                    let (name, rate, time) = {
                        let d = &w.defs[w.current];
                        (d.put_down_anim.clone(), d.put_down_anim_rate, d.put_down_time)
                    };
                    play(&mut w, &name, rate, false);
                    w.switch_timer = time;
                    w.action = Action::PutDown { next };
                }
                Action::PutDown { next } => {
                    w.current = next;
                    set_action(&mut w, Action::Select);
                }
                _ => {}
            }
        }
    }
    if keys.just_pressed(KeyCode::KeyR) || scripted("reload") {
        if allow_reload(&w) {
            start_reload(&mut w, "key");
        } else {
            runlog::kv("reload_refused", &format!("weapon={} action={:?} firing={:?}", w.defs[w.current].item_name, w.action, w.firing));
        }
    }
    if w.action == Action::Reload {
        w.reload_timer += dt;
        let cur = w.current;
        let (rate, hold, item_name) = (w.defs[cur].reload_rate, w.defs[cur].hold_to_reload, w.defs[cur].item_name);
        if w.reload_timer >= rate
            && let Some(a) = w.defs[cur].ammo.as_mut()
        {
            let n = if hold { 1.min(a.spare) } else { (a.capacity - a.mag).min(a.spare) };
            a.mag += n;
            a.spare -= n;
            let full = a.mag >= a.capacity || a.spare == 0;
            runlog::kv("reloaded", &format!("weapon={item_name} added={n} mag={} spare={} done={}", a.mag, a.spare, !hold || full));
            if hold && !full {
                w.reload_timer = 0.0;
            } else {
                set_action(&mut w, Action::Idle);
            }
        }
    }
    if let Some(e) = effects.as_mut() {
        e.ground_speed_bonus = w.defs[w.current].speed_bonus;
        e.aiming = w.aiming;
    }
    ammo_display.weapon = w.defs[w.current].item_name;
    if ammo_display.class != w.defs[w.current].class {
        ammo_display.class = w.defs[w.current].class.clone();
    }
    ammo_display.capacity = w.defs[w.current].ammo.map_or(0, |a| a.capacity);
    ammo_display.hold_to_reload = w.defs[w.current].hold_to_reload;
    ammo_display.weld_percent = w.defs[w.current].weld_fuel.map(|f| 100 * f.amount / f.max.max(1));
    ammo_display.ammo = w.defs[w.current].ammo.map(|a| (a.mag, a.spare));
    // Only a real second ammo (bHasSecondaryAmmo: a maximum above 0).
    ammo_display.alt_ammo = w.defs[w.current].alt_ammo.filter(|a| a.1 > 0).map(|a| a.0);
    ammo_display.syringe = w.defs.iter().find_map(|d| d.heal_charge.filter(|h| h.syringe)).map(|h| h.charge * 100 / HEAL_CHARGE_MAX);
    ammo_display.heal = w.defs[w.current].heal_charge.filter(|h| !h.syringe).map(|h| h.charge * 100 / HEAL_CHARGE_MAX);
    // CrossbowArrow pickups: room for one more bolt?
    if let Some(d) = w.defs.iter().find(|d| d.class.eq_ignore_ascii_case("KFMod.Crossbow")) {
        bolt_room.0 = d.ammo.is_some_and(|a| a.mag + a.spare < a.max_total);
    }
    for _ in bolts_picked.read() {
        if let Some(a) = w.defs.iter_mut().find(|d| d.class.eq_ignore_ascii_case("KFMod.Crossbow")).and_then(|d| d.ammo.as_mut()) {
            a.spare += 1;
        }
    }
    ammo_display.frags = w.defs.iter().find(|d| d.toss.is_some()).and_then(|d| d.ammo).map(|a| a.mag + a.spare);
    ammo_display.fire_mode = match w.defs[w.current].toggles_on_alt {
        Some(AltToggle::FireMode) => Some(if w.defs[w.current].modes[0].wait_for_release { "SEMI" } else { "AUTO" }),
        Some(AltToggle::WideSpread) => Some(if w.defs[w.current].wide_spread { "WIDE" } else { "NARROW" }),
        None => None,
    };
}

