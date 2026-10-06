//! Weapon sounds (S3): fire and select sounds, animation notify sounds, the full-auto / chainsaw / charge loops, the trader's refusals. See DESIGN.md, "Sound and music".

use super::*;

/// The sound half of PlayFiring. KFFire and KFShotgunFire in first
/// person: StereoFireSound at TransientSoundVolume x 0.85, pitch 1 +-
/// RandomPitchAdjustAmt when bRandomPitchFireSound; other fire classes
/// (WeaponFire, WeldFire, SyringeAltFire, FragFire): FireSound at
/// TransientSoundVolume. Always SLOT_Interact, at the player (we are always
/// in first person; the behind view of view_target.rs keeps it).
pub(super) fn fire_sound(w: &mut Weapons, mode: usize) {
    let s = &w.defs[w.current].modes[mode].sounds;
    let (sound, volume) = match &s.stereo {
        Some(st) => (st.clone(), s.volume * 0.85),
        None => match &s.fire {
            Some(f) => (f.clone(), s.volume),
            None => return,
        },
    };
    let (amount, radius) = (s.random_pitch, s.radius);
    let mut pitch = 1.0;
    if amount > 0.0 {
        pitch += sound_rand(w) * amount * if sound_rand(w) < 0.5 { -1.0 } else { 1.0 };
    }
    let actor = weapon_actor(&w.defs[w.current]);
    w.sounds.push(PlaySound::new(sound, Emitter::Listener).slot(SoundSlot::Interact).volume(volume).radius(radius).pitch(pitch).actor(actor));
}

/// The weapon as a sound "actor": its slots are its own (a hash of the
/// class name, stable when weapons are bought or sold).
pub(super) fn weapon_actor(def: &WeaponDef) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    def.class.hash(&mut h);
    h.finish() | 1
}

/// KFWeaponSoundNotify.Notify: the playing sequence's sound notifies whose
/// time falls in (from, to] (frames): Instigator.PlaySound(Sound, ,
/// Volume, false, Radius, , bAttenuate), on the player: SLOT_None, at the
/// listener.
pub(super) fn anim_sounds(w: &mut Weapons, from: f32, to: f32) {
    let Some(seq) = w.sequence else {
        return;
    };
    let model = &w.defs[w.current].model;
    let length = model.length(seq);
    let hits: Vec<PlaySound> = model
        .notifies(seq)
        .iter()
        .filter(|n| n.time * length > from && n.time * length <= to)
        .filter_map(|n| n.sound.as_ref())
        .map(|n| {
            let radius = if n.radius > 0.0 { n.radius } else { crate::audio::mixer::DEFAULT_RADIUS };
            PlaySound::new(n.sound.clone(), Emitter::Listener).volume(n.volume).radius(radius)
        })
        .collect();
    w.sounds.extend(hits);
}

/// KFBuyMenuSaleList / KFTab_BuyMenu: TraderSoundTooExpensive /
/// TooHeavy, DemoPlaySound(.., SLOT_Interface, 2.0) on the player. KF plays
/// them when an item that cannot be bought is selected in the list; ours,
/// when it is bought (our menu has no separate selection step).
pub(crate) fn trader_refusal(sound: &'static str) -> PlaySound {
    PlaySound::new(sound, Emitter::Listener).slot(SoundSlot::Interface).volume(2.0).actor(4)
}

/// FRand for sounds (xorshift).
pub(super) fn sound_rand(w: &mut Weapons) -> f32 {
    w.sound_rng ^= w.sound_rng << 13;
    w.sound_rng ^= w.sound_rng >> 17;
    w.sound_rng ^= w.sound_rng << 5;
    (w.sound_rng >> 8) as f32 / (1u32 << 24) as f32
}

/// Hands this frame's weapon sounds to the mixer, and has each weapon's
/// sounds loaded the first time it is carried.
pub(super) fn send_weapon_sounds(
    w: Option<ResMut<Weapons>>,
    mut out: MessageWriter<PlaySound>,
    mut preload: MessageWriter<crate::audio::mixer::PreloadSounds>,
    mut done: Local<std::collections::HashSet<String>>,
) {
    let Some(mut w) = w else {
        return;
    };
    out.write_batch(w.sounds.drain(..));
    for def in &w.defs {
        if done.insert(def.class.clone()) {
            let mut sounds: Vec<String> =
                def.select_sound.iter().chain(&def.throw_sound).chain(&def.pickup_sound).cloned().chain(def.idle_ambient.as_ref().map(|a| a.sound.clone())).collect();
            for m in &def.modes {
                let s = &m.sounds;
                sounds.extend([&s.fire, &s.stereo, &s.no_ammo, &s.ambient, &s.end_stereo, &s.fire_start, &s.charge_up, &s.placed].into_iter().flatten().cloned());
                sounds.extend(s.melee_hits.iter().cloned());
            }
            sounds.extend(def.model.all_notify_sounds());
            sounds.sort();
            sounds.dedup();
            preload.write(crate::audio::mixer::PreloadSounds { what: def.class.clone(), sounds });
        }
    }
}

/// The weapon attachment's AmbientSound, which plays at the player:
/// - a fire loop (KFHighROFFire, FlameBurstFire, ChainsawFire state
///   FireLoop) while a full-auto mode is held: AmbientFireSound at
///   AmbientFireVolume and AmbientFireSoundRadius. At its end
///   FireEndStereoSound (else FireEndSound) at AmbientFireVolume / 127.
///   The chainsaw first plays FireStartSound and starts the loop when that
///   has played; after its FireEndSound it is silent until that has
///   played, then idles again (ChainsawFire Timer, GetSoundDuration).
/// - HuskGunFire while charging: AmbientChargeUpSound, then
///   AmbientFireSound once HoldTime reaches MaxChargeTime.
/// - otherwise the attachment's own sound (the chainsaw's idle engine).
#[derive(Default)]
pub(super) struct WeaponAmbient {
    fire_loop: Option<FireSounds>,
    /// Game seconds left of the chainsaw's start or end sound.
    wait: f32,
    /// What is on the weapon camera entity now.
    set: Option<AmbientSound>,
}

pub(super) fn weapon_loop_sound(
    w: Option<ResMut<Weapons>>,
    mut commands: Commands,
    time: Res<Time>,
    mut bank: NonSendMut<crate::audio::mixer::SoundBank>,
    mut st: Local<WeaponAmbient>,
) {
    let Some(mut w) = w else {
        return;
    };
    st.wait = (st.wait - time.delta_secs()).max(0.0);
    let def = &w.defs[w.current];
    let actor = weapon_actor(def);
    let wanted = (0..2).find_map(|m| {
        let fm = &def.modes[m];
        (fm.high_rof && !fm.wait_for_release && w.firing[m] && fm.sounds.ambient.is_some()).then(|| fm.sounds.clone())
    });
    let loop_sound = |s: &FireSounds, sound: &str| AmbientSound { sound: sound.to_string(), volume: s.ambient_volume, radius: s.ambient_radius, pitch: 64, at_listener: true, ..default() };
    match (st.fire_loop.take(), wanted) {
        (None, Some(s)) => {
            st.wait = 0.0;
            if let Some(start) = &s.fire_start {
                w.sounds.push(PlaySound::new(start.clone(), Emitter::Listener).slot(SoundSlot::Interact).volume(s.ambient_volume as f32 / 127.0).radius(s.ambient_radius).actor(actor));
                st.wait = bank.duration(start).unwrap_or(0.0);
            }
            st.fire_loop = Some(s);
        }
        (Some(old), None) => {
            st.wait = 0.0;
            if let Some(end) = &old.end_stereo {
                let slot = if old.chainsaw { SoundSlot::Interact } else { SoundSlot::None };
                w.sounds.push(PlaySound::new(end.clone(), Emitter::Listener).slot(slot).volume(old.ambient_volume as f32 / 127.0).radius(old.ambient_radius).actor(actor));
                if old.chainsaw {
                    st.wait = bank.duration(end).unwrap_or(0.0);
                }
            }
        }
        (old, new) => st.fire_loop = new.or(old),
    }
    let def = &w.defs[w.current];
    // The Husk Gun's hold (mode 0) or the ZED Gun's beam charge.
    let beam_mode = (0..2).find(|&m| def.modes[m].beam.is_some()).unwrap_or(1);
    let charging = w.charge_hold.map(|h| (h, &def.modes[0].sounds)).or_else(|| w.beam.map(|b| (b.charge_up, &def.modes[beam_mode].sounds)));
    let desired = if let Some(s) = &st.fire_loop {
        // The chainsaw's loop waits for its start sound (the idle goes on).
        if st.wait > 0.0 { st.set.clone() } else { s.ambient.as_deref().map(|a| loop_sound(s, a)) }
    } else if let Some((hold, s)) = charging.filter(|(_, s)| s.charge_up.is_some()) {
        let sound = if hold < s.charge_max { s.charge_up.as_deref() } else { s.ambient.as_deref() };
        sound.map(|a| loop_sound(s, a))
    } else if st.wait > 0.0 {
        None
    } else {
        def.idle_ambient.clone()
    };
    if desired != st.set {
        match &desired {
            Some(a) => commands.entity(w.camera).insert(a.clone()),
            None => commands.entity(w.camera).remove::<AmbientSound>(),
        };
        st.set = desired;
    }
}
